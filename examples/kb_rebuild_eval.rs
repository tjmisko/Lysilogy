//! Build an isolated canonical state from actual K2 Crossref records, then
//! compare transactional and deleted-database rebuilds. No PDFs or network.
use chrono::{DateTime, Utc};
use clap::Parser;
use lysilogy::{
    citation_graph::Provider,
    kb::{
        AcquisitionState, Authorship, Citation, CitationEvidence, Decision, DecisionAction,
        EntityDecision, Identifier, Observation, ObservationPayload, ObservationSource, Person,
        Work, WorkType,
        store::{Admission, ArtifactSource, EntityProjection, KbStore},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    truth: PathBuf,
    #[arg(long)]
    data: PathBuf,
}
#[derive(Deserialize)]
struct Truth {
    schema_version: u32,
    truth_id: String,
    version: String,
    records: Vec<Record>,
}
#[derive(Deserialize)]
struct Record {
    retrieved_at: DateTime<Utc>,
    crossref: Value,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn doi(value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    let id = value
        .as_str()
        .ok_or("Crossref record lacks DOI")?
        .trim()
        .to_lowercase();
    Identifier::parse(&format!("doi:{id}")).map_err(|error| error.message)?;
    Ok(id)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let truth: Truth = serde_json::from_slice(&fs::read(&args.truth)?)?;
    if truth.schema_version != 1
        || truth.truth_id != "K2"
        || truth.version.trim().is_empty()
        || truth.records.is_empty()
    {
        return Err("requires nonempty actual K2 Crossref records".into());
    }
    let cache = PathBuf::from(std::env::var("HOME")?)
        .join(".cache/lysilogy")
        .canonicalize()?;
    if !args
        .data
        .parent()
        .ok_or("missing data parent")?
        .canonicalize()?
        .starts_with(cache)
    {
        return Err("isolated rebuild data must be beneath ~/.cache/lysilogy".into());
    }
    fs::create_dir(&args.data)?; // Exclusively claim a fresh evaluator-owned root.
    let store = KbStore::open(&args.data)?;
    fs::create_dir(args.data.join("records"))?;
    let mut deposited_pairs = 0;
    let mut first = None;
    for (index, record) in truth.records.iter().enumerate() {
        let source_doi = doi(&record.crossref["DOI"])?;
        let source_id = store.allocate_work(&format!("crossref:doi:{source_doi}"))?;
        let title = record.crossref["title"]
            .as_array()
            .and_then(|titles| titles.first())
            .and_then(Value::as_str)
            .map(str::to_owned);
        let work = Work {
            id: source_id.clone(),
            identifiers: BTreeSet::from([
                Identifier::parse(&format!("doi:{source_doi}")).map_err(|error| error.message)?
            ]),
            title,
            title_key: None,
            year: None,
            venue: None,
            kind: WorkType::Unknown,
            versions: vec![],
            local_copies: vec![],
            acquisition: AcquisitionState::Unresolved,
        };
        let bytes = serde_json::to_vec(&record.crossref)?;
        let relative = format!("records/{index}.json");
        fs::write(args.data.join(&relative), &bytes)?;
        let source = ArtifactSource {
            relative_path: relative,
            sha256: hash(&bytes),
        };
        let observation = Observation {
            id: format!("crossref-work-{source_doi}"),
            source: ObservationSource::Provider {
                provider: Provider::Crossref,
                record_id: source_doi.clone(),
            },
            retrieved_at: record.retrieved_at,
            payload: ObservationPayload::Json(record.crossref.clone()),
        };
        let mut citations = Vec::new();
        for reference in record.crossref["reference"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if reference.get("DOI").is_none() {
                continue;
            }
            let cited_doi = doi(&reference["DOI"])?;
            let cited = store.allocate_work(&format!("crossref:doi:{cited_doi}"))?;
            citations.push(Citation {
                citing: source_id.clone(),
                cited,
                evidence: vec![CitationEvidence::Provider {
                    provider: Provider::Crossref,
                    retrieved_at: record.retrieved_at,
                    provider_edge_id: Some(format!("{source_doi}->{cited_doi}")),
                    passages: vec![],
                    intents: vec![],
                    is_influential: None,
                }],
            });
            deposited_pairs += 1;
        }
        store.admit(
            &Admission {
                observation: observation.clone(),
                entity: EntityProjection::Work(work.clone()),
                source: source.clone(),
                authorships: vec![],
                citations,
            },
            &args.data,
        )?;
        for (position, author) in record.crossref["author"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let given = author["given"].as_str().unwrap_or("");
            let family = author["family"].as_str().unwrap_or("");
            let name = format!("{given} {family}").trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let person_id =
                store.allocate_person(&format!("crossref:{source_doi}:author:{position}"))?;
            let person = Person {
                id: person_id.clone(),
                display_name: name.clone(),
                family_name: None,
                given_names: vec![],
                particles: vec![],
                suffix: None,
                name_variants: vec![],
                identifiers: Default::default(),
            };
            let mut observed = observation.clone();
            observed.id = format!("crossref-person-{source_doi}-{position}");
            store.admit(
                &Admission {
                    observation: observed,
                    entity: EntityProjection::Person(person),
                    source: source.clone(),
                    authorships: vec![Authorship {
                        work_id: source_id.clone(),
                        person_id,
                        position: u32::try_from(position)?,
                        raw_name: name,
                    }],
                    citations: vec![],
                },
                &args.data,
            )?;
        }
        if first.is_none() {
            first = Some((work, observation, source));
        }
    }
    if deposited_pairs == 0 {
        return Err("K2 has no deposited DOI reference pairs".into());
    }
    // Exercise alias replay with a deliberate duplicate of one exact DOI source;
    // this is a store scenario, not a measured automatic entity matcher.
    let (mut duplicate, mut observation, source) = first.ok_or("K2 has no source Works")?;
    let surviving = duplicate.id.clone();
    duplicate.id = store.allocate_work("eval:duplicate-of-first-exact-doi")?;
    let absorbed = duplicate.id.clone();
    observation.id.push_str("-duplicate");
    store.admit(
        &Admission {
            observation,
            entity: EntityProjection::Work(duplicate),
            source,
            authorships: vec![],
            citations: vec![],
        },
        &args.data,
    )?;
    store.record_decision(&Decision {
        id: "eval-exact-doi-merge".into(),
        recorded_at: Utc::now(),
        rationale: "Evaluator deliberately admitted the same exact DOI record twice".into(),
        action: DecisionAction::Work(EntityDecision::Merge {
            surviving,
            absorbed,
        }),
    })?;
    let initial = store.snapshot()?;
    store.rebuild()?;
    let transactional = store.snapshot()?;
    let database = store.database_path();
    drop(store);
    fs::remove_file(database)?;
    let rebuilt = KbStore::open(&args.data)?;
    rebuilt.rebuild()?;
    let deleted = rebuilt.snapshot()?;
    let identical = initial == transactional && initial == deleted;
    let summary = rebuilt.summary()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"schema_version":1,"truth_id":"K2","truth_version":truth.version,"source_records":truth.records.len(),"deposited_pairs":deposited_pairs,"identical":identical,"initial_sha256":hash(&serde_json::to_vec(&initial)?),"transactional_sha256":hash(&serde_json::to_vec(&transactional)?),"deleted_sha256":hash(&serde_json::to_vec(&deleted)?),"summary":summary})
        )?
    );
    if !identical {
        return Err("rebuild determinism failed".into());
    }
    Ok(())
}
