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
