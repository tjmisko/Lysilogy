//! Scores the claim verifier against real citations from the vault, replayed from the
//! responses recorded by `scripts/build-claim-fixtures`. No network is used.
//!
//! Thresholds: no fake is verified or matched; at least 90% of the real set and 80% of the
//! near-miss set resolve to the right Work; none resolves to the wrong Work.

mod claims_support;

use std::path::PathBuf;

use claims_support::{ScratchDatabase, load_fixtures, run_cases};
use lysilogy_ingest::verify_claim::fixtures::ReplayTransport;

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claims")
}

#[tokio::test]
async fn should_meet_claim_thresholds_when_replaying_vault_fixtures() {
    let Some(url) = std::env::var("LYSILOGY_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty())
    else {
        eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
        return;
    };
    let (sets, responses) = load_fixtures(&fixture_directory()).expect("fixtures are committed");
    assert!(!sets.real.is_empty(), "the real set is empty");
    assert!(!sets.fake.is_empty(), "the fake set is empty");
    assert!(!sets.near_miss.is_empty(), "the near-miss set is empty");
    let scratch = ScratchDatabase::create(&url)
        .await
        .expect("scratch database");
    let transport = ReplayTransport::new(responses);
    let report = run_cases(&scratch.pool, transport.clone(), &sets).await;
    scratch
        .drop_database()
        .await
        .expect("drop scratch database");
    let report = report.expect("run cases");
    println!("{}", report.render());
    assert!(
        transport.misses().is_empty(),
        "requests missing from the recording: {:?}",
        transport.misses()
    );
    let failures = report.failures();
    assert!(failures.is_empty(), "{failures:?}\n{}", report.render());
}

#[test]
fn should_cover_every_perturbation_kind_when_fixtures_are_built() {
    let (sets, _) = load_fixtures(&fixture_directory()).expect("fixtures are committed");
    for kind in [
        "fabricated_doi",
        "real_doi_wrong_title",
        "recombined_title",
        "year_off",
        "authors_swapped",
        "invented",
    ] {
        assert!(
            sets.fake.iter().any(|case| case.kind == kind),
            "no fake of kind {kind}"
        );
    }
    for kind in ["doi", "arxiv", "title"] {
        assert!(
            sets.real.iter().any(|case| case.kind == kind),
            "no real case of kind {kind}"
        );
    }
    assert!(sets.fake.iter().all(|case| case.truth.is_none()));
    assert!(sets.real.iter().all(|case| case.truth.is_some()));
}
