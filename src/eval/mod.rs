//! Offline evaluation, complete scorecards, and monotonic baseline ratchets.
pub mod measurement;
pub mod registry;
mod scorecard;
#[cfg(test)]
mod tests;

use crate::{Error, Result};
use chrono::Utc;
use clap::{Args, ValueEnum};
use measurement::{EvidenceFile, Input};
use registry::{Definition, Kind, definitions};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Suite {
    Objects,
    Bibliography,
    Resolution,
    Persons,
    Acquisition,
    Citations,
    Lists,
    ReadNext,
    Scale,
    Tests,
    #[default]
    All,
}
impl Suite {
    const fn name(self) -> &'static str {
        match self {
            Self::Objects => "objects",
            Self::Bibliography => "bibliography",
            Self::Resolution => "resolution",
            Self::Persons => "persons",
            Self::Acquisition => "acquisition",
            Self::Citations => "citations",
            Self::Lists => "lists",
            Self::ReadNext => "read-next",
            Self::Scale => "scale",
            Self::Tests => "tests",
            Self::All => "all",
        }
    }
    fn includes(self, definition: Definition) -> bool {
        self == Self::All || self.name() == definition.suite
    }
}

#[derive(Debug, Args)]
pub struct EvalArgs {
    #[arg(value_enum, default_value = "all")]
    pub suite: Suite,
    /// Fail on measured hard-gate failures or unjustified objective regressions.
    #[arg(long)]
    pub check: bool,
    /// Also require every gate and at least 80% of all objectives at target (all suite only).
    #[arg(long, requires = "check")]
    pub require_complete: bool,
    /// Repository containing eval/inputs, eval/baselines.json, and docs/.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// JSON metric-to-justification map for explicit objective baseline resets.
    #[arg(long)]
    pub justification: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricResult {
    pub value: Option<f64>,
    pub reason: Option<String>,
    pub cases: u64,
    pub truth_sets: BTreeMap<String, EvidenceFile>,
    pub evidence: Vec<EvidenceFile>,
}
impl MetricResult {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            reason: Some(reason.into()),
            cases: 0,
            truth_sets: BTreeMap::new(),
            evidence: vec![],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Run {
    pub schema_version: u32,
    pub suite: String,
    pub timestamp: String,
    pub commit: String,
    pub dirty: bool,
    pub metrics: BTreeMap<String, MetricResult>,
    pub costs_usd: BTreeMap<String, Option<f64>>,
    pub wall_seconds: BTreeMap<String, f64>,
    pub failures: Vec<String>,
    pub baseline_before: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Baselines {
    pub schema_version: u32,
    pub metrics: BTreeMap<String, f64>,
    pub adjustments: Vec<Adjustment>,
}
impl Default for Baselines {
    fn default() -> Self {
        Self {
            schema_version: 1,
            metrics: BTreeMap::new(),
            adjustments: vec![],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Adjustment {
    pub metric: String,
    pub previous: f64,
    pub value: f64,
    pub justification: Justification,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Justification {
    pub reason: String,
    pub evidence: EvidenceFile,
}

pub fn run(args: &EvalArgs) -> Result<()> {
    if args.require_complete && args.suite != Suite::All {
        return Err(Error::InvalidRequest(
            "--require-complete requires the all suite".into(),
        ));
    }
    let root = args
        .root
        .canonicalize()
        .map_err(|error| Error::io(&args.root, error))?;
    let commit = git(&root, &["rev-parse", "HEAD"])?;
    let dirty = !git(
        &root,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty();
    let mut baselines: Baselines =
        read_optional(&root.join("eval/baselines.json"))?.unwrap_or_default();
    validate_baselines(&baselines)?;
    let justifications = load_justifications(&root, args.justification.as_deref())?;
    verify_committed_baselines(&root, &baselines)?;
    let mut run = collect(&root, args.suite)?;
    run.commit = commit;
    run.dirty = dirty;
    run.baseline_before.clone_from(&baselines.metrics);
    run.failures = check(&run.metrics, &baselines, &justifications);
    if args.require_complete {
        run.failures
            .extend(scorecard::acceptance_failures(&run.metrics));
    }
    if run.failures.is_empty() {
        ratchet(&run.metrics, &mut baselines, &justifications);
    }
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.9fZ");
    let result_path = root.join(format!(
        "eval/results/{}/{stamp}-{}.json",
        args.suite.name(),
        run.commit
    ));
    write_json(&result_path, &run)?;
    if run.failures.is_empty() {
        write_json(&root.join("eval/baselines.json"), &baselines)?;
    }
    // Scorecard combines latest persisted suite measurements; a newer unavailable result
    // replaces an older measured one instead of making stale evidence appear current.
    let latest = latest_metrics(&root)?;
    let report = scorecard::render(&latest, &baselines);
    write_atomic(&root.join("docs/kb-scorecard.md"), report.as_bytes())?;
    print!("{report}");
    println!("Result: {}", result_path.display());
    if args.check && !run.failures.is_empty() {
        return Err(Error::InvalidRequest(run.failures.join("; ")));
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| Error::io(root, error))?;
    if !output.status.success() {
        return Err(Error::InvalidRequest(
            "evaluation root must be a Git repository".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn collect(root: &Path, suite: Suite) -> Result<Run> {
    let mut run = Run {
        schema_version: 1,
        suite: suite.name().into(),
        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        commit: String::new(),
        dirty: false,
        metrics: BTreeMap::new(),
        costs_usd: BTreeMap::new(),
        wall_seconds: BTreeMap::new(),
        failures: vec![],
        baseline_before: BTreeMap::new(),
    };
    let mut inputs = BTreeMap::<String, Option<Input>>::new();
    for definition in definitions().into_iter().filter(|d| suite.includes(*d)) {
        if definition.id == "G5" {
            let (metric, seconds) = isolated_tests(root)?;
            run.wall_seconds.insert("tests".into(), seconds);
            run.costs_usd.insert("tests".into(), Some(0.0));
            run.metrics.insert(definition.id.into(), metric);
            continue;
        }
        if !inputs.contains_key(definition.suite) {
            let input: Option<Input> =
                read_optional(&root.join(format!("eval/inputs/{}.json", definition.suite)))?;
            if let Some(input) = &input {
                validate_input(input, definition.suite)?;
                run.costs_usd
                    .insert(definition.suite.into(), input.cost_usd);
                run.wall_seconds
                    .insert(definition.suite.into(), input.wall_seconds);
            }
            inputs.insert(definition.suite.into(), input);
        }
        let metric = evaluate(
            root,
            definition,
            inputs.get(definition.suite).and_then(Option::as_ref),
        )?;
        run.metrics.insert(definition.id.into(), metric);
    }
    Ok(run)
}

fn validate_input(input: &Input, suite: &str) -> Result<()> {
    if input.schema_version != 1
        || input.suite != suite
        || input.collector.trim().is_empty()
        || !input.wall_seconds.is_finite()
        || input.wall_seconds < 0.0
        || input
            .cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
    {
        return Err(Error::InvalidRequest(format!(
            "invalid {suite} evaluation input metadata"
        )));
    }
    for id in input.metrics.keys() {
        if !definitions()
            .iter()
            .any(|d| d.id == id && d.suite == suite && d.id != "G5")
        {
            return Err(Error::InvalidRequest(format!(
                "unknown or misplaced metric {id} in {suite}"
            )));
        }
    }
    Ok(())
}

fn evaluate(root: &Path, definition: Definition, input: Option<&Input>) -> Result<MetricResult> {
    let Some(input) = input else {
        return Ok(MetricResult::unavailable(format!(
            "missing eval/inputs/{}.json; truth/collector not built",
            definition.suite
        )));
    };
    let Some(observation) = input.metrics.get(definition.id) else {
        return Ok(MetricResult::unavailable(
            "collector has no observation for this component",
        ));
    };
    if input.implementation.is_empty()
        || input.truth_sets.is_empty()
        || observation.evidence.is_empty()
        || observation.cases == 0
    {
        return Ok(MetricResult::unavailable(
            "nonempty implementation, truth, case count, and observation evidence required",
        ));
    }
    for evidence in input
        .implementation
        .iter()
        .chain(input.truth_sets.values())
        .chain(&observation.evidence)
    {
        if !measurement::verify(root, evidence)? {
            return Ok(MetricResult::unavailable(format!(
                "missing or stale evidence: {}",
                evidence.path
            )));
        }
    }
    // K identifiers are required whenever specified by the scorecard. Scale may use
    // synthetic evidence before K0 exists; the final scale report must identify its tier.
    for truth in definition
        .truth
        .split(',')
        .filter(|part| part.starts_with('K'))
    {
        if !(input.truth_sets.contains_key(truth)
            || definition.suite == "scale" && input.truth_sets.contains_key("synthetic"))
        {
            return Ok(MetricResult::unavailable(format!(
                "missing truth set {truth}"
            )));
        }
    }
    let Some(value) = observation.sample.calculate() else {
        return Ok(MetricResult::unavailable(
            "empty or invalid measurement sample",
        ));
    };
    if !definition.valid_value(value) {
        return Err(Error::InvalidRequest(format!(
            "invalid {} value {value} ({})",
            definition.id, definition.unit
        )));
    }
    Ok(MetricResult {
        value: Some(value),
        reason: None,
        cases: observation.cases,
        truth_sets: input.truth_sets.clone(),
        evidence: input
            .implementation
            .iter()
            .chain(&observation.evidence)
            .cloned()
            .collect(),
    })
}

fn isolated_tests(root: &Path) -> Result<(MetricResult, f64)> {
    if std::env::var_os("LYSILOGY_EVAL_ISOLATED").is_some() {
        return Ok((
            MetricResult::unavailable("nested G5 invocation is forbidden"),
            0.0,
        ));
    }
    let output_dir = root
        .join("target/eval-g5")
        .join(Utc::now().format("%Y%m%dT%H%M%S%.9fZ").to_string());
    fs::create_dir_all(&output_dir).map_err(|error| Error::io(&output_dir, error))?;
    let script = root.join("src/eval/g5.py");
    let started = std::time::Instant::now();
    let status = Command::new("python3")
        .arg(&script)
        .arg(root)
        .arg(&output_dir)
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|error| Error::io(&script, error))?;
    let elapsed = started.elapsed().as_secs_f64();
    let path = output_dir.join("evidence.json");
    let mut evidence = vec![];
    if path.is_file() {
        evidence.push(EvidenceFile {
            path: path
                .strip_prefix(root)
                .map_err(|error| Error::InvalidRequest(error.to_string()))?
                .to_string_lossy()
                .into_owned(),
            sha256: measurement::digest(&path)?,
            version: "g5-network-namespace-v1".into(),
        });
    }
    let passed = status.success() && !evidence.is_empty();
    Ok((
        MetricResult {
            value: Some(if passed { 1.0 } else { 0.0 }),
            reason: (!passed).then(|| {
                format!(
                    "isolated runner failed: {status}; logs {}",
                    output_dir.display()
                )
            }),
            cases: 1,
            truth_sets: BTreeMap::new(),
            evidence,
        },
        elapsed,
    ))
}

fn validate_baselines(baselines: &Baselines) -> Result<()> {
    if baselines.schema_version != 1 {
        return Err(Error::InvalidRequest("unsupported baseline schema".into()));
    }
    for (id, value) in &baselines.metrics {
        let definition = definitions()
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| Error::InvalidRequest(format!("unknown baseline metric {id}")))?;
        if !definition.valid_value(*value) {
            return Err(Error::InvalidRequest(format!("invalid baseline for {id}")));
        }
    }
    Ok(())
}

fn verify_committed_baselines(root: &Path, current: &Baselines) -> Result<()> {
    let output = Command::new("git")
        .args(["show", "HEAD:eval/baselines.json"])
        .current_dir(root)
        .output()
        .map_err(|error| Error::io(root, error))?;
    if !output.status.success() {
        // First introduction of the harness or an isolated fixture repository.
        return Ok(());
    }
    let committed: Baselines = serde_json::from_slice(&output.stdout)?;
    verify_baseline_changes(root, &committed, current)
}

fn verify_baseline_changes(root: &Path, previous: &Baselines, current: &Baselines) -> Result<()> {
    for (id, old) in &previous.metrics {
        let definition = definitions()
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| Error::InvalidRequest(format!("unknown committed baseline {id}")))?;
        let Some(value) = current.metrics.get(id) else {
            return Err(Error::InvalidRequest(format!(
                "committed baseline {id} was removed"
            )));
        };
        if value.to_bits() == old.to_bits() || definition.improves(*value, *old) {
            continue;
        }
        let adjustment = current
            .adjustments
            .iter()
            .rev()
            .find(|change| change.metric == *id && change.value.to_bits() == value.to_bits());
        if definition.kind == Kind::Gate
            || !matches!(adjustment, Some(change)
            if change.justification.reason.trim().len() >= 20
                && measurement::verify(root, &change.justification.evidence)?)
        {
            return Err(Error::InvalidRequest(format!(
                "committed baseline {id} weakened without a recorded justification"
            )));
        }
    }
    Ok(())
}

fn load_justifications(
    root: &Path,
    path: Option<&Path>,
) -> Result<BTreeMap<String, Justification>> {
    let Some(path) = path else {
        return Ok(BTreeMap::new());
    };
    let path = measurement::safe_file(root, &path.to_string_lossy())?;
    let values: BTreeMap<String, Justification> = read_optional(&path)?
        .ok_or_else(|| Error::InvalidRequest("missing justification file".into()))?;
    for (id, justification) in &values {
        if !definitions()
            .iter()
            .any(|d| d.id == id && d.kind == Kind::Objective)
            || justification.reason.trim().len() < 20
            || !measurement::verify(root, &justification.evidence)?
        {
            return Err(Error::InvalidRequest(format!(
                "invalid objective justification for {id}"
            )));
        }
    }
    Ok(values)
}

#[must_use]
pub fn check(
    metrics: &BTreeMap<String, MetricResult>,
    baselines: &Baselines,
    justifications: &BTreeMap<String, Justification>,
) -> Vec<String> {
    let mut failures = vec![];
    for definition in definitions() {
        let Some(value) = metrics.get(definition.id).and_then(|metric| metric.value) else {
            continue;
        };
        if definition.kind == Kind::Gate && !definition.at_target(value) {
            failures.push(format!("{} hard gate failed: {value}", definition.id));
        }
        if definition.kind == Kind::Objective
            && let Some(baseline) = baselines.metrics.get(definition.id)
            && definition.regresses(value, *baseline)
            && !justifications.contains_key(definition.id)
        {
            failures.push(format!(
                "{} regressed: {baseline} -> {value}",
                definition.id
            ));
        }
    }
    failures
}

pub fn ratchet(
    metrics: &BTreeMap<String, MetricResult>,
    baselines: &mut Baselines,
    justifications: &BTreeMap<String, Justification>,
) {
    for definition in definitions()
        .into_iter()
        .filter(|d| d.kind != Kind::Reported)
    {
        let Some(value) = metrics.get(definition.id).and_then(|metric| metric.value) else {
            continue;
        };
        let previous = baselines.metrics.get(definition.id).copied();
        if let Some(previous) = previous
            && !definition.improves(value, previous)
        {
            if value.to_bits() == previous.to_bits() {
                continue;
            }
            let Some(justification) = justifications.get(definition.id) else {
                continue;
            };
            if definition.kind == Kind::Gate {
                continue;
            }
            baselines.adjustments.push(Adjustment {
                metric: definition.id.into(),
                previous,
                value,
                justification: justification.clone(),
            });
        }
        baselines.metrics.insert(definition.id.into(), value);
    }
}

fn read_optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(path, error)),
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidRequest("missing output parent".into()))?;
    fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| Error::io(&temporary, error))?;
    fs::rename(&temporary, path).map_err(|error| Error::io(path, error))
}
fn latest_metrics(root: &Path) -> Result<BTreeMap<String, MetricResult>> {
    let mut runs = vec![];
    for suite in [
        "all",
        "objects",
        "bibliography",
        "resolution",
        "persons",
        "acquisition",
        "citations",
        "lists",
        "read-next",
        "scale",
        "tests",
    ] {
        let folder = root.join(format!("eval/results/{suite}"));
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io(&folder, error)),
        };
        for entry in entries {
            let path = entry.map_err(|error| Error::io(&folder, error))?.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Some(run) = read_optional::<Run>(&path)?
            {
                runs.push((run.timestamp, run.suite, run.metrics));
            }
        }
    }
    runs.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let mut metrics = BTreeMap::new();
    for (_, _, values) in runs {
        metrics.extend(values);
    }
    Ok(metrics)
}
