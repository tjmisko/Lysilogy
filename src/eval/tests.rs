use super::*;
use measurement::{Observation, Sample};

fn measured(value: f64) -> MetricResult {
    MetricResult {
        value: Some(value),
        reason: None,
        cases: 10,
        truth_sets: BTreeMap::new(),
        evidence: vec![],
    }
}
fn metrics(id: &str, value: f64) -> BTreeMap<String, MetricResult> {
    BTreeMap::from([(id.into(), measured(value))])
}
fn baseline(id: &str, value: f64) -> Baselines {
    Baselines {
        metrics: BTreeMap::from([(id.into(), value)]),
        ..Baselines::default()
    }
}

#[test]
fn should_exit_nonzero_when_a_hard_gate_fails() {
    assert!(
        !check(
            &metrics("G1.works", 0.989),
            &Baselines::default(),
            &BTreeMap::new()
        )
        .is_empty()
    );
}
#[test]
fn should_exit_nonzero_when_an_objective_regresses_beyond_tolerance() {
    assert!(
        !check(
            &metrics("O1", 0.879),
            &baseline("O1", 0.90),
            &BTreeMap::new()
        )
        .is_empty()
    );
}
#[test]
fn should_report_suite_unavailable_when_truth_set_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let result = collect(dir.path(), Suite::Objects).unwrap();
    assert!(result.metrics.values().all(|m| m.value.is_none()));
    assert!(check(&result.metrics, &Baselines::default(), &BTreeMap::new()).is_empty());
}
#[test]
fn should_ratchet_baseline_when_a_metric_improves() {
    let mut base = baseline("O1", 0.80);
    ratchet(&metrics("O1", 0.85), &mut base, &BTreeMap::new());
    assert_eq!(base.metrics["O1"].to_bits(), 0.85_f64.to_bits());
}
#[test]
fn should_ratchet_downward_when_latency_improves() {
    let mut base = baseline("O25", 3.0);
    ratchet(&metrics("O25", 2.0), &mut base, &BTreeMap::new());
    assert_eq!(base.metrics["O25"].to_bits(), 2.0_f64.to_bits());
}
#[test]
fn should_keep_baseline_when_a_decline_is_within_tolerance() {
    let mut base = baseline("O1", 0.90);
    let values = metrics("O1", 0.89);
    assert!(check(&values, &base, &BTreeMap::new()).is_empty());
    ratchet(&values, &mut base, &BTreeMap::new());
    assert_eq!(base.metrics["O1"].to_bits(), 0.90_f64.to_bits());
}
#[test]
fn should_use_relative_tolerance_when_latency_regresses() {
    assert!(
        check(
            &metrics("O26.search", 110.0),
            &baseline("O26.search", 100.0),
            &BTreeMap::new()
        )
        .is_empty()
    );
    assert!(
        !check(
            &metrics("O26.search", 110.01),
            &baseline("O26.search", 100.0),
            &BTreeMap::new()
        )
        .is_empty()
    );
}
#[test]
fn should_reject_any_budget_violation_when_zero_is_the_baseline() {
    assert!(
        !check(
            &metrics("O30", 1.0),
            &baseline("O30", 0.0),
            &BTreeMap::new()
        )
        .is_empty()
    );
}
#[test]
fn should_preserve_separate_components_when_an_objective_has_multiple_targets() {
    let values = metrics("O22.resolved", 0.95);
    let report = scorecard::render(&values, &Baselines::default());
    assert!(report.contains("0/30 at target"));
    assert!(report.contains("O22.fabricated"));
}
#[test]
fn should_include_all_scorecard_ids_when_rendering() {
    let defs = definitions();
    for prefix in ['G', 'O', 'R'] {
        let max = match prefix {
            'G' => 5,
            'O' => 30,
            _ => 4,
        };
        for id in 1..=max {
            assert!(
                defs.iter()
                    .any(|d| d.id.split('.').next() == Some(format!("{prefix}{id}").as_str()))
            );
        }
    }
}
#[test]
fn should_render_identically_when_results_are_identical() {
    let values = metrics("O1", 0.90);
    assert_eq!(
        scorecard::render(&values, &Baselines::default()),
        scorecard::render(&values, &Baselines::default())
    );
}
#[test]
fn should_require_all_gates_when_system_acceptance_is_requested() {
    assert_eq!(scorecard::acceptance_failures(&BTreeMap::new()).len(), 6);
}
#[test]
fn should_calculate_from_raw_samples_when_observations_are_present() {
    for (sample, expected) in [
        (
            Sample::Ratio {
                numerator: 8.0,
                denominator: 10.0,
            },
            0.8,
        ),
        (
            Sample::F1 {
                true_positive: 8.0,
                false_positive: 1.0,
                false_negative: 3.0,
            },
            0.8,
        ),
        (
            Sample::Median {
                values: vec![1.0, 4.0, 2.0, 3.0],
            },
            2.5,
        ),
        (
            Sample::P95 {
                values: (1..=100).map(f64::from).collect(),
            },
            95.0,
        ),
        (
            Sample::Mean {
                values: vec![2.0, 4.0],
            },
            3.0,
        ),
    ] {
        assert!((sample.calculate().unwrap() - expected).abs() < 1e-12);
    }
}
#[test]
fn should_report_unavailable_when_samples_are_empty_or_nonfinite() {
    assert!(
        Sample::Ratio {
            numerator: 0.0,
            denominator: 0.0
        }
        .calculate()
        .is_none()
    );
    assert!(Sample::Value { value: f64::NAN }.calculate().is_none());
    assert!(Sample::Median { values: vec![] }.calculate().is_none());
}

fn evidence(dir: &Path, name: &str) -> EvidenceFile {
    fs::write(dir.join(name), b"fixture-v1").unwrap();
    EvidenceFile {
        path: name.into(),
        sha256: measurement::digest(&dir.join(name)).unwrap(),
        version: "fixture-v1".into(),
    }
}
fn input(dir: &Path) -> Input {
    let truth = evidence(dir, "truth.json");
    let implementation = evidence(dir, "collector.rs");
    let result = evidence(dir, "observations.json");
    Input {
        schema_version: 1,
        suite: "objects".into(),
        collector: "fixture".into(),
        implementation: vec![implementation],
        truth_sets: BTreeMap::from([("K1".into(), truth)]),
        metrics: BTreeMap::from([(
            "O1".into(),
            Observation {
                sample: Sample::F1 {
                    true_positive: 9.0,
                    false_positive: 1.0,
                    false_negative: 1.0,
                },
                cases: 10,
                evidence: vec![result],
            },
        )]),
        cost_usd: Some(0.0),
        wall_seconds: 0.1,
    }
}
#[test]
fn should_compute_metric_when_truth_and_implementation_hashes_match() {
    let dir = tempfile::tempdir().unwrap();
    let source = input(dir.path());
    let result = evaluate(
        dir.path(),
        definitions().into_iter().find(|d| d.id == "O1").unwrap(),
        Some(&source),
    )
    .unwrap();
    assert_eq!(result.value.unwrap().to_bits(), 0.90_f64.to_bits());
}
#[test]
fn should_report_unavailable_when_implementation_evidence_becomes_stale() {
    let dir = tempfile::tempdir().unwrap();
    let source = input(dir.path());
    fs::write(dir.path().join("collector.rs"), b"new code").unwrap();
    let result = evaluate(
        dir.path(),
        definitions().into_iter().find(|d| d.id == "O1").unwrap(),
        Some(&source),
    )
    .unwrap();
    assert!(result.value.is_none());
    assert!(result.reason.unwrap().contains("stale"));
}
#[test]
fn should_reject_evidence_when_a_path_escapes_or_is_prohibited() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "../outside",
        "/etc/shadow",
        ".env",
        "nested/.env.local",
        "nested/.secrets/data",
    ] {
        assert!(measurement::safe_file(dir.path(), path).is_err());
    }
}
#[test]
fn should_not_waive_hard_gates_when_justification_is_present() {
    let justification = Justification {
        reason: "a reason cannot waive a gate".into(),
        evidence: EvidenceFile {
            path: "x".into(),
            sha256: "x".into(),
            version: "v1".into(),
        },
    };
    assert!(
        !check(
            &metrics("G2", 1.0),
            &Baselines::default(),
            &BTreeMap::from([("G2".into(), justification)])
        )
        .is_empty()
    );
}
#[test]
fn should_record_adjustment_when_an_objective_reset_is_justified() {
    let dir = tempfile::tempdir().unwrap();
    let justification = Justification {
        reason: "Documented changed truth population after audit".into(),
        evidence: evidence(dir.path(), "report.md"),
    };
    let mut base = baseline("O1", 0.95);
    ratchet(
        &metrics("O1", 0.90),
        &mut base,
        &BTreeMap::from([("O1".into(), justification)]),
    );
    assert_eq!(base.adjustments.len(), 1);
    assert_eq!(base.metrics["O1"].to_bits(), 0.90_f64.to_bits());
}
#[test]
fn should_replace_stale_pass_when_a_later_result_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let mut run = collect(dir.path(), Suite::Objects).unwrap();
    run.timestamp = "2026-01-01T00:00:00Z".into();
    run.metrics = metrics("O1", 0.95);
    write_json(&dir.path().join("eval/results/objects/a.json"), &run).unwrap();
    run.timestamp = "2026-01-02T00:00:00Z".into();
    run.metrics = BTreeMap::from([("O1".into(), MetricResult::unavailable("missing truth"))]);
    write_json(&dir.path().join("eval/results/all/b.json"), &run).unwrap();
    assert!(latest_metrics(dir.path()).unwrap()["O1"].value.is_none());
}

#[test]
fn should_reject_baseline_edits_when_committed_values_are_weakened_without_evidence() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        verify_baseline_changes(dir.path(), &baseline("O1", 0.90), &baseline("O1", 0.89)).is_err()
    );
    assert!(
        verify_baseline_changes(dir.path(), &baseline("O25", 2.0), &baseline("O25", 2.1)).is_err()
    );
    assert!(
        verify_baseline_changes(dir.path(), &baseline("O1", 0.90), &Baselines::default()).is_err()
    );
}

#[test]
fn should_accept_baseline_edits_when_measurements_improved() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        verify_baseline_changes(dir.path(), &baseline("O1", 0.90), &baseline("O1", 0.95)).is_ok()
    );
    assert!(
        verify_baseline_changes(dir.path(), &baseline("O25", 2.0), &baseline("O25", 1.8)).is_ok()
    );
}

#[test]
fn should_accept_stored_adjustment_when_justified_baseline_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut current = baseline("O1", 0.90);
    let original = current.clone();
    let justification = Justification {
        reason: "Truth population audited and expanded with harder cases".into(),
        evidence: evidence(dir.path(), "audit.md"),
    };
    ratchet(
        &metrics("O1", 0.80),
        &mut current,
        &BTreeMap::from([("O1".into(), justification)]),
    );
    assert!(verify_baseline_changes(dir.path(), &original, &current).is_ok());
}

fn scale_input(root: &Path, collector: &str, id: &str, value: f64) -> Input {
    Input {
        schema_version: 1,
        suite: "scale".into(),
        collector: format!("{collector}-v1"),
        implementation: vec![evidence(root, &format!("{collector}-implementation.rs"))],
        truth_sets: BTreeMap::from([(
            "synthetic".into(),
            evidence(root, &format!("{collector}-truth.json")),
        )]),
        metrics: BTreeMap::from([(
            id.into(),
            Observation {
                sample: Sample::Value { value },
                cases: 10,
                evidence: vec![evidence(root, &format!("{collector}-observations.json"))],
            },
        )]),
        cost_usd: Some(0.0),
        wall_seconds: 1.5,
    }
}

#[test]
fn should_compose_disjoint_collectors_when_they_own_different_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let mut scan = scale_input(dir.path(), "scan", "O25", 1.9);
    scan.cost_usd = None;
    scan.wall_seconds = 4.0;
    write_json(&dir.path().join("eval/inputs/scale/scan.json"), &scan).unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale/provider-budgets.json"),
        &scale_input(dir.path(), "budgets", "O30", 0.0),
    )
    .unwrap();
    let run = collect(dir.path(), Suite::Scale).unwrap();
    assert_eq!(
        run.metrics["O25"].value.unwrap().to_bits(),
        1.9_f64.to_bits()
    );
    assert_eq!(
        run.metrics["O30"].value.unwrap().to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(run.costs_usd.len(), 2);
    assert_eq!(run.costs_usd["scale/scan"], None);
    assert_eq!(
        run.costs_usd["scale/provider-budgets"].unwrap().to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(run.wall_seconds["scale/scan"].to_bits(), 4.0_f64.to_bits());
    assert_eq!(
        run.wall_seconds["scale/provider-budgets"].to_bits(),
        1.5_f64.to_bits()
    );
    assert_eq!(run.collectors["scale/scan"].metrics, ["O25"]);
    assert_eq!(run.collectors["scale/scan"].input.version, "scan-v1");
    assert!(measurement::verify(dir.path(), &run.collectors["scale/scan"].input).unwrap());
}

#[test]
fn should_preserve_unrelated_metrics_when_one_collectors_dependencies_are_stale() {
    let dir = tempfile::tempdir().unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale/scan.json"),
        &scale_input(dir.path(), "scan", "O25", 1.9),
    )
    .unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale/provider-budgets.json"),
        &scale_input(dir.path(), "budgets", "O30", 0.0),
    )
    .unwrap();
    fs::write(
        dir.path().join("budgets-implementation.rs"),
        b"changed budget code",
    )
    .unwrap();
    let run = collect(dir.path(), Suite::Scale).unwrap();
    assert_eq!(
        run.metrics["O25"].value.unwrap().to_bits(),
        1.9_f64.to_bits()
    );
    assert!(run.metrics["O30"].value.is_none());
    assert!(
        run.metrics["O30"]
            .reason
            .as_ref()
            .unwrap()
            .contains("budgets-implementation.rs")
    );
    assert_eq!(run.collectors.len(), 2);
}

#[test]
fn should_reject_duplicate_ownership_when_two_collectors_claim_the_same_metric() {
    let dir = tempfile::tempdir().unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale/first.json"),
        &scale_input(dir.path(), "first", "O30", 0.0),
    )
    .unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale/second.json"),
        &scale_input(dir.path(), "second", "O30", 0.0),
    )
    .unwrap();
    fs::write(
        dir.path().join("first-implementation.rs"),
        b"stale owner still owns the metric",
    )
    .unwrap();
    assert!(
        collect(dir.path(), Suite::Scale)
            .unwrap_err()
            .to_string()
            .contains("duplicate metric ownership for O30")
    );
}

#[test]
fn should_compose_legacy_and_named_inputs_when_their_metric_ownership_is_disjoint() {
    let dir = tempfile::tempdir().unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale.json"),
        &scale_input(dir.path(), "scan", "O25", 1.9),
    )
    .unwrap();
    write_json(
        &dir.path().join("eval/inputs/scale/provider-budgets.json"),
        &scale_input(dir.path(), "budgets", "O30", 0.0),
    )
    .unwrap();
    let run = collect(dir.path(), Suite::Scale).unwrap();
    assert!(run.metrics["O25"].value.is_some());
    assert!(run.metrics["O30"].value.is_some());
    assert!(run.costs_usd.contains_key("scale"));
    assert!(run.costs_usd.contains_key("scale/provider-budgets"));
    write_json(
        &dir.path().join("eval/inputs/scale/duplicate.json"),
        &scale_input(dir.path(), "duplicate", "O25", 1.8),
    )
    .unwrap();
    assert!(
        collect(dir.path(), Suite::Scale)
            .unwrap_err()
            .to_string()
            .contains("duplicate metric ownership for O25")
    );
}

#[test]
fn should_reject_unbounded_discovery_when_a_suite_has_too_many_collectors() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = dir.path().join("eval/inputs/scale");
    fs::create_dir_all(&inputs).unwrap();
    for index in 0..=MAX_INPUTS_PER_SUITE {
        fs::write(inputs.join(format!("collector-{index}.json")), b"{}").unwrap();
    }
    assert!(
        collect(dir.path(), Suite::Scale)
            .unwrap_err()
            .to_string()
            .contains("32-input limit")
    );
}

#[test]
fn should_reject_oversized_input_when_a_collector_exceeds_the_byte_limit() {
    // The sparse size fixture belongs in the worktree target, not RAM-backed /tmp.
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/eval-fixtures");
    fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::tempdir_in(scratch).unwrap();
    let inputs = dir.path().join("eval/inputs");
    fs::create_dir_all(&inputs).unwrap();
    fs::File::create(inputs.join("scale.json"))
        .unwrap()
        .set_len(MAX_INPUT_BYTES + 1)
        .unwrap();
    assert!(
        collect(dir.path(), Suite::Scale)
            .unwrap_err()
            .to_string()
            .contains("byte input limit")
    );
}
