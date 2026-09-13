use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "test: initialize eval fixture",
        ],
    ] {
        let output = Command::new("git")
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    dir
}
fn evaluate(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lysilogy"))
        .args(["eval", "objects", "--check", "--root"])
        .arg(root)
        .current_dir(root)
        .output()
        .unwrap()
}
fn observations(root: &Path, id: &str, value: f64) {
    fs::create_dir_all(root.join("eval/inputs")).unwrap();
    fs::write(root.join("fixture.json"), b"derived evaluation fixture").unwrap();
    let file = json!({"path":"fixture.json","sha256":format!("{:x}",Sha256::digest(b"derived evaluation fixture")),"version":"v1"});
    let input = json!({"schema_version":1,"suite":"objects","collector":"fixture-v1","implementation":[file],"truth_sets":{"K1":file},"metrics":{id:{"sample":{"method":"value","value":value},"cases":10,"evidence":[file]}},"cost_usd":0,"wall_seconds":0.1});
    fs::write(
        root.join("eval/inputs/objects.json"),
        serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
}

#[test]
fn should_exit_nonzero_when_cli_observes_a_failed_hard_gate() {
    let dir = repository();
    observations(dir.path(), "G3", 1.0);
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("G3 hard gate failed"));
    assert!(!dir.path().join("eval/baselines.json").exists());
}
#[test]
fn should_exit_nonzero_when_cli_observes_an_objective_regression() {
    let dir = repository();
    observations(dir.path(), "O1", 0.8);
    fs::write(
        dir.path().join("eval/baselines.json"),
        br#"{"schema_version":1,"metrics":{"O1":0.9},"adjustments":[]}"#,
    )
    .unwrap();
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("O1 regressed"));
    let baseline: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("eval/baselines.json")).unwrap()).unwrap();
    assert_eq!(baseline["metrics"]["O1"], json!(0.9));
}
#[test]
fn should_report_unavailable_without_initializing_library_when_cli_has_no_truth() {
    let dir = repository();
    let output = evaluate(dir.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("unavailable"));
    for name in ["local-articles", ".lysilogy", "Notes"] {
        assert!(!dir.path().join(name).exists());
    }
    assert!(dir.path().join("docs/kb-scorecard.md").exists());
}

#[test]
fn should_mark_run_dirty_when_an_implementation_file_is_untracked() {
    let dir = repository();
    observations(dir.path(), "O1", 0.90);
    assert!(evaluate(dir.path()).status.success());
    let record = fs::read_dir(dir.path().join("eval/results/objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let run: serde_json::Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    assert_eq!(run["dirty"], json!(true));
}

fn fixture_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_files(root: &Path, paths: &[&str]) {
    let mut add = vec!["add", "--"];
    add.extend_from_slice(paths);
    fixture_git(root, &add);
    fixture_git(
        root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "test: record evaluation fixture",
        ],
    );
}

fn baseline_file(root: &Path, value: f64, adjustments: &serde_json::Value) {
    fs::create_dir_all(root.join("eval")).unwrap();
    fs::write(
        root.join("eval/baselines.json"),
        serde_json::to_vec(
            &json!({"schema_version":1,"metrics":{"O1":value},"adjustments":adjustments}),
        )
        .unwrap(),
    )
    .unwrap();
}

fn adjustment(root: &Path, previous: f64, value: f64) -> serde_json::Value {
    let report = b"A reproducible audit justifying the objective baseline change.";
    fs::write(root.join("audit.md"), report).unwrap();
    json!({"metric":"O1","previous":previous,"value":value,"justification":{"reason":"A reproducible audit justifies the changed truth population.","evidence":{"path":"audit.md","version":"audit-v1","sha256":format!("{:x}",Sha256::digest(report))}}})
}

#[test]
fn should_reject_history_when_an_unjustified_weaker_baseline_is_committed() {
    let dir = repository();
    observations(dir.path(), "O1", 0.8);
    baseline_file(dir.path(), 0.9, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    baseline_file(dir.path(), 0.8, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("baseline history"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("O1 weakened"));
}

#[test]
fn should_reject_history_when_a_baseline_removal_is_committed() {
    let dir = repository();
    baseline_file(dir.path(), 0.9, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    fs::remove_file(dir.path().join("eval/baselines.json")).unwrap();
    commit_files(dir.path(), &["eval/baselines.json"]);
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("O1 was removed"));
}

#[test]
fn should_accept_history_when_justification_was_valid_at_the_weakening_commit() {
    let dir = repository();
    observations(dir.path(), "O1", 0.8);
    baseline_file(dir.path(), 0.9, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    let change = adjustment(dir.path(), 0.9, 0.8);
    baseline_file(dir.path(), 0.8, &json!([change]));
    commit_files(dir.path(), &["eval/baselines.json", "audit.md"]);
    fs::write(
        dir.path().join("audit.md"),
        b"A later revision of the audit.",
    )
    .unwrap();
    commit_files(dir.path(), &["audit.md"]);
    let output = evaluate(dir.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn should_reject_history_when_justification_evidence_was_added_only_after_weakening() {
    let dir = repository();
    observations(dir.path(), "O1", 0.8);
    baseline_file(dir.path(), 0.9, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    let change = adjustment(dir.path(), 0.9, 0.8);
    baseline_file(dir.path(), 0.8, &json!([change]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    commit_files(dir.path(), &["audit.md"]);
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid recorded justification"));
}

#[test]
fn should_reject_history_when_an_adjustment_does_not_match_the_previous_baseline() {
    let dir = repository();
    observations(dir.path(), "O1", 0.8);
    baseline_file(dir.path(), 0.9, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    let change = adjustment(dir.path(), 0.85, 0.8);
    baseline_file(dir.path(), 0.8, &json!([change]));
    commit_files(dir.path(), &["eval/baselines.json", "audit.md"]);
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match its previous value"));
}

#[test]
fn should_reject_history_when_an_old_adjustment_is_reused_for_a_new_weakening() {
    let dir = repository();
    observations(dir.path(), "O1", 0.8);
    baseline_file(dir.path(), 0.9, &json!([]));
    commit_files(dir.path(), &["eval/baselines.json"]);
    let changes = json!([adjustment(dir.path(), 0.9, 0.8)]);
    baseline_file(dir.path(), 0.8, &changes);
    commit_files(dir.path(), &["eval/baselines.json", "audit.md"]);
    baseline_file(dir.path(), 0.9, &changes);
    commit_files(dir.path(), &["eval/baselines.json"]);
    baseline_file(dir.path(), 0.8, &changes);
    commit_files(dir.path(), &["eval/baselines.json"]);
    let output = evaluate(dir.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("O1 weakened"));
}
