//! E2 verification: runs the frozen claim verifier over `e2/claims.json` against the live
//! `OpenAlex` and Crossref APIs, records every provider response into `responses.json` so
//! the run replays offline, and writes `verdicts.json`.
//!
//! Each model's claims run in their own scratch database, in file order, so one model's
//! Works never decide another's claims. After the live run the whole run is replayed from
//! the recording and the verdicts must be identical.
//!
//! `cargo run --example e2_verify -- --dir eval/claims/e2 [--max-searches N] [--scratch FILE]`

#[allow(dead_code)]
#[path = "../tests/claims_support/mod.rs"]
mod claims_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use claims_support::{Failure, ScratchDatabase};
use lysilogy_core::Claim;
use lysilogy_db::claim::ClaimRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use lysilogy_ingest::openalex::{JsonTransport, OpenAlexClient, ProviderFuture, ReqwestTransport};
use lysilogy_ingest::verify_claim::fixtures::{CapturingTransport, ReplayTransport, Responses};
use lysilogy_ingest::verify_claim::{ClaimResolution, VerifyContext, verify_claim};
use serde_json::{Value, json};

/// Lets the runner own a transport the caller can still read afterwards.
struct Shared<T>(Arc<T>);

impl<T: JsonTransport> JsonTransport for Shared<T> {
    fn get_json<'a>(
        &'a self,
        url: &'a str,
    ) -> ProviderFuture<'a, Result<Value, lysilogy_ingest::openalex::ProviderError>> {
        self.0.get_json(url)
    }
}

#[allow(clippy::cast_possible_truncation)] // a publication year
fn claim_of(row: &Value) -> Claim {
    Claim {
        title: row["title"].as_str().unwrap_or_default().to_owned(),
        authors: row["authors"]
            .as_array()
            .map(|authors| {
                authors
                    .iter()
                    .filter_map(|author| author.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        year: row["year"].as_i64().map(|year| year as i32),
        venue: row["venue"].as_str().map(str::to_owned),
        doi: row["doi"].as_str().map(str::to_owned),
        arxiv: row["arxiv"].as_str().map(str::to_owned),
        raw: None,
    }
}

#[allow(clippy::too_many_lines)] // one pass over the claims, read top to bottom
async fn run<T: JsonTransport>(
    transport: T,
    claims: &[Value],
    test_url: &str,
) -> Result<BTreeMap<String, Value>, Failure> {
    let mut verdicts = BTreeMap::new();
    let mut models: Vec<&str> = claims
        .iter()
        .filter_map(|row| row["model"].as_str())
        .collect();
    models.dedup();
    let client = OpenAlexClient::new(transport);
    for model in models {
        let scratch = ScratchDatabase::create(test_url).await?;
        let works = WorkRepository::new(scratch.pool.clone());
        let claim_repository = ClaimRepository::new(scratch.pool.clone());
        let records = SourceRecordRepository::new(scratch.pool.clone());
        let context = VerifyContext {
            works: &works,
            claims: &claim_repository,
            records: &records,
            client: &client,
        };
        for row in claims.iter().filter(|row| row["model"] == model) {
            let id = row["claim_id"].as_str().unwrap_or_default().to_owned();
            let resolution = verify_claim(&context, &claim_of(row)).await;
            let verdict = match resolution {
                Ok(ClaimResolution::Work {
                    work,
                    verified,
                    created,
                    identifier_dropped,
                }) => {
                    let ids = works.openalex_ids_for(&[work.id]).await?;
                    json!({
                        "outcome": if verified { "verified" } else { "matched" },
                        "state": null,
                        "work_id": ids.into_iter().next().map(|(_, id)| id),
                        "created": created,
                        "identifier_dropped": identifier_dropped,
                        "candidates": [],
                    })
                }
                Ok(ClaimResolution::Unresolved {
                    state,
                    candidates,
                    identifier_dropped,
                }) => json!({
                    "outcome": "unresolved",
                    "state": format!("{state:?}").to_lowercase(),
                    "work_id": null,
                    "identifier_dropped": identifier_dropped,
                    "candidates": candidates.iter().map(|candidate| json!({
                        "work_id": candidate.provider_id,
                        "title": candidate.title,
                        "year": candidate.year,
                        "authors": candidate.authors,
                        "doi": candidate.doi,
                    })).collect::<Vec<_>>(),
                }),
                Err(error) => json!({"outcome": "error", "state": null, "work_id": null,
                    "error": error.to_string(), "candidates": []}),
            };
            verdicts.insert(id, verdict);
        }
        scratch.drop_database().await?;
    }
    Ok(verdicts)
}

fn read_responses(path: &Path) -> Responses {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_json<V: serde::Serialize>(path: &Path, value: &V) -> Result<(), Failure> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let home = std::env::var("HOME")?;
    let mut dir = PathBuf::from("eval/claims/e2");
    let mut max_searches = 600_usize;
    let mut scratch_file = PathBuf::from(&home).join(".cache/lysilogy-scratch/e2-responses.json");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or("flag needs a value");
        match flag.as_str() {
            "--dir" => dir = PathBuf::from(value()?),
            "--max-searches" => max_searches = value()?.parse()?,
            "--scratch" => scratch_file = PathBuf::from(value()?),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let test_url = std::env::var("LYSILOGY_TEST_DATABASE_URL").unwrap_or_else(|_| {
        format!(
            "postgresql://{}@127.0.0.1:5433/lysilogy_test",
            std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned())
        )
    });
    let claims: Vec<Value> = serde_json::from_slice(&std::fs::read(dir.join("claims.json"))?)?;
    eprintln!("{} claims", claims.len());

    let seed = read_responses(&scratch_file);
    eprintln!("resuming from {} recorded responses", seed.len());
    let capture = Arc::new(CapturingTransport::new(
        ReqwestTransport::from_environment()?,
        seed,
        max_searches,
    ));
    let live = run(Shared(Arc::clone(&capture)), &claims, &test_url).await;
    write_json(&scratch_file, &capture.captured())?;
    let live = live?;
    eprintln!(
        "{} live requests, {} live title searches",
        capture.live_requests(),
        capture.paid_requests()
    );
    if capture.exhausted() {
        return Err(format!(
            "the request budget ran out; {} responses kept in {}, rerun to continue",
            capture.captured().len(),
            scratch_file.display()
        )
        .into());
    }

    let replay = ReplayTransport::new(capture.captured());
    let replayed = run(replay.clone(), &claims, &test_url).await?;
    if !replay.misses().is_empty() {
        return Err(format!("replay missed {:?}", replay.misses()).into());
    }
    if replayed != live {
        return Err("the replay's verdicts differ from the live run's".into());
    }
    write_json(&dir.join("responses.json"), &capture.captured())?;
    write_json(&dir.join("verdicts.json"), &live)?;
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    for verdict in live.values() {
        let label = verdict["state"]
            .as_str()
            .unwrap_or_else(|| verdict["outcome"].as_str().unwrap_or("?"));
        *tally.entry(label.to_owned()).or_default() += 1;
    }
    eprintln!("verdicts: {tally:?}");
    Ok(())
}
