//! Tests for the held-out evaluation harness (`tests/claims_holdout`) and for the
//! ablation switch: with nothing ablated the verifier must behave exactly as before.

#[allow(dead_code)]
mod claims_holdout;
#[allow(dead_code)]
mod claims_support;

use std::path::PathBuf;

use claims_holdout::{
    Class, Reconciled, attach_verdicts, load_e2, load_set, read_responses, reconcile, run_items,
    score_set, wilson,
};
use claims_support::{Outcome, ScratchDatabase, load_fixtures};
use lysilogy_core::Ablation;
use lysilogy_db::claim::ClaimRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use lysilogy_ingest::openalex::OpenAlexClient;
use lysilogy_ingest::verify_claim::fixtures::ReplayTransport;
use lysilogy_ingest::verify_claim::{
    ClaimResolution, VerifyContext, verify_claim, verify_claim_with,
};
use serde_json::{Value, json};

/// The scratch database is named by process ID, so tests that use one run one at a time.
static DATABASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn database_url() -> Option<String> {
    let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());
    if url.is_none() {
        eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
    }
    url
}

#[test]
fn should_give_the_known_wilson_interval_when_counting_successes() {
    let (low, high) = wilson(8, 10).expect("interval");
    assert!((low - 0.4902).abs() < 1e-3, "{low}");
    assert!((high - 0.9433).abs() < 1e-3, "{high}");
    assert!(wilson(0, 0).is_none());
    let (low, high) = wilson(0, 12).expect("interval");
    assert!(low.abs() < 1e-12);
    assert!((high - 0.2425).abs() < 1e-3, "{high}");
}

#[test]
fn should_report_the_rule_of_three_bound_only_when_the_count_is_zero() {
    let none = claims_holdout::rate_json(0, 60);
    assert_eq!(none["rule_of_three_upper"], json!(0.05));
    let some = claims_holdout::rate_json(1, 60);
    assert!(some["rule_of_three_upper"].is_null());
    let empty = claims_holdout::rate_json(0, 0);
    assert!(empty["rate"].is_null() && empty["wilson95"].is_null());
    assert!(empty["rule_of_three_upper"].is_null());
}

#[test]
fn should_send_labels_to_the_spot_check_when_labellers_disagree_or_are_unsure() {
    let exists = json!({"label": "exists", "openalex_id": "W1"});
    let flawed = json!({"label": "exists_wrong_details", "openalex_id": "W1"});
    let unsure = json!({"label": "unsure"});
    let nonsense = json!({"label": "maybe"});
    let other_id = json!({"label": "exists", "openalex_id": "w1"});
    let no_id = json!({"label": "exists", "openalex_id": ""});
    assert!(matches!(reconcile(None, None), Reconciled::Unlabelled));
    assert!(matches!(
        reconcile(Some(&exists), Some(&flawed)),
        Reconciled::SpotCheck {
            reason: "label_disagreement"
        }
    ));
    assert!(matches!(
        reconcile(Some(&exists), Some(&unsure)),
        Reconciled::SpotCheck { reason: "unsure" }
    ));
    assert!(matches!(
        reconcile(Some(&exists), None),
        Reconciled::SpotCheck {
            reason: "only_one_labeller"
        }
    ));
    assert!(matches!(
        reconcile(Some(&nonsense), Some(&nonsense)),
        Reconciled::SpotCheck {
            reason: "unrecognised_label"
        }
    ));
    let Reconciled::Agreed { label, truths } = reconcile(Some(&exists), Some(&other_id)) else {
        panic!("agreed labels reconcile");
    };
    assert_eq!((label.as_str(), truths.len()), ("exists", 1));
    let Reconciled::Agreed { truths, .. } = reconcile(Some(&exists), Some(&no_id)) else {
        panic!("agreed labels reconcile");
    };
    assert_eq!(truths, vec!["W1".to_owned()]);
}

/// The base tuning fixtures, run in their own order and graph, with the plain entry point
/// and with the ablation entry point at its default.
async fn tuning_outcomes(url: &str, ablated: bool) -> Vec<(String, String)> {
    let (sets, responses) = load_fixtures(&root().join("tests/fixtures/claims")).expect("fixtures");
    let scratch = ScratchDatabase::create(url).await.expect("scratch");
    let works = WorkRepository::new(scratch.pool.clone());
    let claims = ClaimRepository::new(scratch.pool.clone());
    let records = SourceRecordRepository::new(scratch.pool.clone());
    let transport = ReplayTransport::new(responses);
    let client = OpenAlexClient::new(transport.clone());
    let context = VerifyContext {
        works: &works,
        claims: &claims,
        records: &records,
        client: &client,
    };
    let mut outcomes = Vec::new();
    for (_, case) in sets.in_run_order() {
        let resolution = if ablated {
            verify_claim_with(&context, &case.claim, Ablation::default()).await
        } else {
            verify_claim(&context, &case.claim).await
        }
        .expect("verify");
        let outcome = match resolution {
            ClaimResolution::Work { work, verified, .. } => {
                let ids = works.openalex_ids_for(&[work.id]).await.expect("ids");
                Outcome::Work {
                    openalex_id: ids.into_iter().next().map(|(_, id)| id),
                    verified,
                }
            }
            ClaimResolution::Unresolved { state, .. } => Outcome::Unresolved(state),
        };
        outcomes.push((case.id.clone(), outcome.describe()));
    }
    assert!(transport.misses().is_empty(), "{:?}", transport.misses());
    scratch.drop_database().await.expect("drop");
    outcomes
}

#[tokio::test]
async fn should_reproduce_the_shipped_verdicts_when_no_ablation_is_set_on_the_tuning_fixtures() {
    let Some(url) = database_url() else { return };
    let _guard = DATABASE.lock().await;
    let plain = tuning_outcomes(&url, false).await;
    let ablated = tuning_outcomes(&url, true).await;
    assert_eq!(plain.len(), 67, "the tuning fixtures were not loaded");
    assert_eq!(plain, ablated);
}

#[tokio::test]
async fn should_change_some_verdicts_when_a_check_is_ablated_on_the_tuning_fixtures() {
    let Some(url) = database_url() else { return };
    let _guard = DATABASE.lock().await;
    let (sets, responses) = load_fixtures(&root().join("tests/fixtures/claims")).expect("fixtures");
    let items: Vec<claims_holdout::Item> = sets
        .in_run_order()
        .map(|(_, case)| claims_holdout::Item {
            id: case.id.clone(),
            set: "base",
            group: case.kind.clone(),
            class: Class::Real,
            claim: case.claim.clone(),
            truths: case.truth.clone().into_iter().collect(),
            detail: Value::Null,
        })
        .collect();
    let default = run_items(&url, &responses, &items, Ablation::default())
        .await
        .expect("default run");
    let mut moved = Vec::new();
    for (name, ablation) in claims_holdout::ablations() {
        let run = run_items(&url, &responses, &items, ablation)
            .await
            .expect("ablated run");
        let differs = default
            .rows
            .iter()
            .zip(&run.rows)
            .any(|(a, b)| a.outcome != b.outcome);
        if differs {
            moved.push(name);
        }
    }
    assert!(
        moved.contains(&"surname_check") && moved.contains(&"year_window"),
        "only {moved:?} changed a verdict on the tuning fixtures"
    );
}

#[tokio::test]
async fn should_replay_e3_without_network_when_scoring_with_an_empty_graph() {
    let Some(url) = database_url() else { return };
    let _guard = DATABASE.lock().await;
    let directory = root().join("eval/claims/e3");
    let (items, status) = load_set(&directory, "e3").expect("e3 cases");
    assert_eq!(status["complete"], json!(true));
    let responses = read_responses(&directory).expect("e3 responses");
    let first = run_items(&url, &responses, &items, Ablation::default())
        .await
        .expect("run");
    let second = run_items(&url, &responses, &items, Ablation::default())
        .await
        .expect("run");
    assert_eq!(first.rows.len(), items.len());
    let must_not: Vec<_> = items
        .iter()
        .zip(&first.rows)
        .filter(|(item, _)| item.class == Class::MustNot)
        .collect();
    assert!(!must_not.is_empty());
    assert!(
        must_not
            .iter()
            .all(|(_, row)| !matches!(row.outcome, Outcome::Work { .. })),
        "a must-not-accept case became a Work"
    );
    // The empty graph and cold cache make every claim independent of the others.
    let again: Vec<String> = second.rows.iter().map(|r| r.outcome.describe()).collect();
    let before: Vec<String> = first.rows.iter().map(|r| r.outcome.describe()).collect();
    assert_eq!(before, again);
}

#[tokio::test]
async fn should_score_e2_by_model_and_exclude_spot_checks_when_labels_are_reconciled() {
    let Some(url) = database_url() else { return };
    let _guard = DATABASE.lock().await;
    let e3 = root().join("eval/claims/e3");
    let (items, _) = load_set(&e3, "e3").expect("e3 cases");
    let responses = read_responses(&e3).expect("e3 responses");
    let run = run_items(&url, &responses, &items, Ablation::default())
        .await
        .expect("run");
    // Four real claims the verifier gets right, split between two fake "models".
    let picked: Vec<_> = items
        .iter()
        .zip(&run.rows)
        .filter(|(item, row)| {
            item.class == Class::Real
                && matches!(&row.outcome, Outcome::Work { openalex_id: Some(id), .. }
                    if item.truths.iter().any(|t| t.eq_ignore_ascii_case(id)))
        })
        .take(4)
        .map(|(item, _)| item)
        .collect();
    assert_eq!(picked.len(), 4);
    let directory = tempfile::tempdir().expect("tempdir");
    let rows: Vec<Value> = picked
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({"claim_id": format!("t-{index}"),
                "model": if index < 2 { "alpha" } else { "beta" },
                "title": item.claim.title, "authors": item.claim.authors,
                "year": item.claim.year, "venue": item.claim.venue,
                "doi": item.claim.doi, "arxiv": item.claim.arxiv})
        })
        .collect();
    let entry = |index: usize, label: &str| {
        json!({"claim_id": format!("t-{index}"), "label": label,
            "openalex_id": picked[index].truths[0], "evidence": "test"})
    };
    let labels = json!([{"batch": 0,
        "A": [entry(0, "exists"), entry(1, "exists"), entry(2, "exists"), entry(3, "exists")],
        "B": [entry(0, "exists"), entry(1, "exists"), entry(2, "exists"),
              entry(3, "exists_wrong_details")]}]);
    std::fs::write(
        directory.path().join("claims.json"),
        serde_json::to_vec(&rows).expect("json"),
    )
    .expect("write");
    std::fs::write(
        directory.path().join("labels.json"),
        serde_json::to_vec(&labels).expect("json"),
    )
    .expect("write");
    let mut loaded = load_e2(directory.path()).expect("e2");
    assert_eq!(loaded.spot_check.len(), 1);
    assert_eq!(loaded.spot_check[0]["claim_id"], json!("t-3"));
    assert_eq!(loaded.spot_check[0]["reason"], json!("label_disagreement"));
    assert!(loaded.spot_check[0]["verifier_verdict"].is_null());
    let (scored, rows) = score_set(&url, &responses, &loaded.items, false)
        .await
        .expect("score");
    attach_verdicts(&mut loaded.spot_check, &loaded.items, &rows);
    assert!(loaded.spot_check[0]["verifier_verdict"]["outcome"].is_string());
    assert_eq!(scored["metrics"]["n"], json!(3));
    assert_eq!(scored["metrics"]["real_recall"]["k"], json!(3));
    assert_eq!(scored["excluded"]["n"], json!(1));
    assert_eq!(scored["by_group"]["alpha"]["n"], json!(2));
    assert_eq!(scored["by_group"]["beta"]["n"], json!(1));
}
