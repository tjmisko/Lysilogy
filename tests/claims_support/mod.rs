//! Shared by `tests/claims_eval.rs` and `examples/build_claim_fixtures.rs`: the fixture
//! file format, a scratch database, and the runner that scores the verifier.
//!
//! Cases are run in a scratch database created for the run, so local matches, the response
//! cache and the Works created by earlier cases are the same every time.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use lysilogy_core::{Claim, ClaimState};
use lysilogy_db::claim::ClaimRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use lysilogy_ingest::openalex::{JsonTransport, OpenAlexClient};
use lysilogy_ingest::verify_claim::{ClaimResolution, VerifyContext, verify_claim};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// The error type shared with the fixture builder.
pub type Failure = Box<dyn std::error::Error + Send + Sync>;

/// Fraction of the real set that must resolve to the right Work.
pub const REAL_MIN_RATE: f64 = 0.90;
/// Fraction of the near-miss set that must resolve to the right Work.
pub const NEAR_MISS_MIN_RATE: f64 = 0.80;

/// One claim with how it was made and what it should come to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimCase {
    pub id: String,
    /// How the case was made: `doi`, `arxiv`, `title`, a perturbation name, ...
    pub kind: String,
    pub claim: Claim,
    /// The `OpenAlex` ID the claim should resolve to; `None` for fakes.
    pub truth: Option<String>,
    /// The vault paper whose bibliography the entry came from; `None` for fakes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_paper: Option<String>,
}

/// The three sets, as written to `cases.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseSets {
    pub real: Vec<ClaimCase>,
    pub fake: Vec<ClaimCase>,
    pub near_miss: Vec<ClaimCase>,
}

impl CaseSets {
    /// Cases in the order they are run: real first, so fakes built on a real title find
    /// its search already cached.
    pub fn in_run_order(&self) -> impl Iterator<Item = (&'static str, &ClaimCase)> {
        self.real
            .iter()
            .map(|case| ("real", case))
            .chain(self.near_miss.iter().map(|case| ("near_miss", case)))
            .chain(self.fake.iter().map(|case| ("fake", case)))
    }
}

/// How one case came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Became a Work with this `OpenAlex` ID (`None` for a Work with no such ID).
    Work {
        openalex_id: Option<String>,
        verified: bool,
    },
    Unresolved(ClaimState),
}

impl Outcome {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Work {
                openalex_id,
                verified,
            } => format!(
                "{} {}",
                if *verified { "verified" } else { "matched" },
                openalex_id.as_deref().unwrap_or("-")
            ),
            Self::Unresolved(state) => format!("{state:?}").to_lowercase(),
        }
    }

    const fn is_work(&self) -> bool {
        matches!(self, Self::Work { .. })
    }

    fn is_correct(&self, truth: Option<&str>) -> bool {
        let (Self::Work { openalex_id, .. }, Some(truth)) = (self, truth) else {
            return false;
        };
        openalex_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(truth))
    }
}

/// Every case's outcome, with the scores the thresholds read.
#[derive(Debug, Default)]
pub struct Report {
    pub real: Vec<(ClaimCase, Outcome)>,
    pub near_miss: Vec<(ClaimCase, Outcome)>,
    pub fake: Vec<(ClaimCase, Outcome)>,
}

fn correct(results: &[(ClaimCase, Outcome)]) -> usize {
    results
        .iter()
        .filter(|(case, outcome)| outcome.is_correct(case.truth.as_deref()))
        .count()
}

fn wrong(results: &[(ClaimCase, Outcome)]) -> usize {
    results
        .iter()
        .filter(|(case, outcome)| outcome.is_work() && !outcome.is_correct(case.truth.as_deref()))
        .count()
}

#[allow(clippy::cast_precision_loss)]
fn rate(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 / whole as f64
}

impl Report {
    #[must_use]
    pub fn fakes_resolved(&self) -> usize {
        self.fake
            .iter()
            .filter(|(_, outcome)| outcome.is_work())
            .count()
    }

    #[must_use]
    pub fn real_rate(&self) -> f64 {
        rate(correct(&self.real), self.real.len())
    }

    #[must_use]
    pub fn near_miss_rate(&self) -> f64 {
        rate(correct(&self.near_miss), self.near_miss.len())
    }

    /// The threshold failures, empty when the report passes.
    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        let mut failures = Vec::new();
        if self.fakes_resolved() > 0 {
            failures.push(format!(
                "{} fake claims were verified or matched",
                self.fakes_resolved()
            ));
        }
        if self.real_rate() < REAL_MIN_RATE {
            failures.push(format!(
                "real set resolved {:.0}% correctly, need {:.0}%",
                self.real_rate() * 100.0,
                REAL_MIN_RATE * 100.0
            ));
        }
        if wrong(&self.real) + wrong(&self.near_miss) > 0 {
            failures.push(format!(
                "{} claims resolved to the wrong Work",
                wrong(&self.real) + wrong(&self.near_miss)
            ));
        }
        if self.near_miss_rate() < NEAR_MISS_MIN_RATE {
            failures.push(format!(
                "near-miss set resolved {:.0}% correctly, need {:.0}%",
                self.near_miss_rate() * 100.0,
                NEAR_MISS_MIN_RATE * 100.0
            ));
        }
        failures
    }

    /// A printable report: the scores, then every case that did not go as hoped.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "claims_eval: real {}/{} correct ({} wrong Work), near-miss {}/{} correct ({} wrong Work), fakes resolved {}/{}",
            correct(&self.real),
            self.real.len(),
            wrong(&self.real),
            correct(&self.near_miss),
            self.near_miss.len(),
            wrong(&self.near_miss),
            self.fakes_resolved(),
            self.fake.len(),
        );
        let mut by_kind: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        for (case, outcome) in &self.fake {
            let label = outcome
                .describe()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
            *by_kind
                .entry(case.kind.clone())
                .or_default()
                .entry(label)
                .or_default() += 1;
        }
        for (kind, labels) in by_kind {
            let _ = writeln!(out, "  fake {kind}: {labels:?}");
        }
        for (set, results) in [("real", &self.real), ("near_miss", &self.near_miss)] {
            for (case, outcome) in results {
                if outcome.is_correct(case.truth.as_deref()) {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "  MISS {set} {} [{}] {:?} -> {} (truth {})",
                    case.id,
                    case.kind,
                    case.claim.title,
                    outcome.describe(),
                    case.truth.as_deref().unwrap_or("-")
                );
            }
        }
        for (case, outcome) in &self.fake {
            if outcome.is_work() {
                let _ = writeln!(
                    out,
                    "  FAKE RESOLVED {} [{}] {:?} -> {}",
                    case.id,
                    case.kind,
                    case.claim.title,
                    outcome.describe()
                );
            }
        }
        out
    }
}

/// Run every case through the verifier, in a scratch database, with this transport.
pub async fn run_cases<T: JsonTransport>(
    pool: &PgPool,
    transport: T,
    sets: &CaseSets,
) -> Result<Report, Failure> {
    let works = WorkRepository::new(pool.clone());
    let claims = ClaimRepository::new(pool.clone());
    let records = SourceRecordRepository::new(pool.clone());
    let client = OpenAlexClient::new(transport);
    let context = VerifyContext {
        works: &works,
        claims: &claims,
        records: &records,
        client: &client,
    };
    let mut report = Report::default();
    for (set, case) in sets.in_run_order() {
        let resolution = verify_claim(&context, &case.claim).await?;
        let outcome = match resolution {
            ClaimResolution::Work { work, verified, .. } => {
                let ids = works.openalex_ids_for(&[work.id]).await?;
                Outcome::Work {
                    openalex_id: ids.into_iter().next().map(|(_, id)| id),
                    verified,
                }
            }
            ClaimResolution::Unresolved { state, .. } => Outcome::Unresolved(state),
        };
        let slot = match set {
            "real" => &mut report.real,
            "near_miss" => &mut report.near_miss,
            _ => &mut report.fake,
        };
        slot.push((case.clone(), outcome));
    }
    Ok(report)
}

/// A database created for one run and dropped with [`ScratchDatabase::drop_database`].
pub struct ScratchDatabase {
    admin: PgPool,
    name: String,
    pub pool: PgPool,
}

impl ScratchDatabase {
    /// Create and migrate a database beside the one `base_url` names.
    pub async fn create(base_url: &str) -> Result<Self, Failure> {
        let name = format!("lysilogy_claims_{}", std::process::id());
        let admin_url = with_database(base_url, "postgres")?;
        let admin = PgPool::connect(&admin_url).await?;
        sqlx::query(&format!("drop database if exists \"{name}\" with (force)"))
            .execute(&admin)
            .await?;
        sqlx::query(&format!("create database \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = lysilogy_db::connect(&with_database(base_url, &name)?).await?;
        lysilogy_db::migrate(&pool).await?;
        Ok(Self { admin, name, pool })
    }

    pub async fn drop_database(self) -> Result<(), Failure> {
        self.pool.close().await;
        sqlx::query(&format!(
            "drop database if exists \"{}\" with (force)",
            self.name
        ))
        .execute(&self.admin)
        .await?;
        Ok(())
    }
}

fn with_database(url: &str, database: &str) -> Result<String, Failure> {
    let mut parsed = reqwest::Url::parse(url)?;
    parsed.set_path(&format!("/{database}"));
    Ok(parsed.to_string())
}

/// Read `cases.json` and `responses.json` from a fixture directory.
pub fn load_fixtures(
    directory: &Path,
) -> Result<(CaseSets, lysilogy_ingest::verify_claim::fixtures::Responses), Failure> {
    let cases = serde_json::from_slice(&std::fs::read(directory.join("cases.json"))?)?;
    let responses = serde_json::from_slice(&std::fs::read(directory.join("responses.json"))?)?;
    Ok((cases, responses))
}
