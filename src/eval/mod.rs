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
    #[serde(default)]
    pub collectors: BTreeMap<String, CollectorResult>,
    pub failures: Vec<String>,
    pub baseline_before: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CollectorResult {
    pub input: EvidenceFile,
    pub metrics: Vec<String>,
}

const MAX_INPUTS_PER_SUITE: usize = 32;
const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;

struct LoadedInput {
    input: Input,
    source: EvidenceFile,
}

#[derive(Default)]
struct SuiteInputs {
    inputs: BTreeMap<String, LoadedInput>,
    owners: BTreeMap<String, String>,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Adjustment {
    pub metric: String,
    pub previous: f64,
    pub value: f64,
    pub justification: Justification,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
        collectors: BTreeMap::new(),
        failures: vec![],
        baseline_before: BTreeMap::new(),
    };
    let mut inputs = BTreeMap::<String, SuiteInputs>::new();
    for definition in definitions().into_iter().filter(|d| suite.includes(*d)) {
        if definition.id == "G5" {
            let (metric, seconds) = isolated_tests(root)?;
            run.wall_seconds.insert("tests".into(), seconds);
            run.costs_usd.insert("tests".into(), Some(0.0));
            run.metrics.insert(definition.id.into(), metric);
            continue;
        }
        if !inputs.contains_key(definition.suite) {
            let collectors = load_suite_inputs(root, definition.suite)?;
            for (key, loaded) in &collectors.inputs {
                run.costs_usd.insert(key.clone(), loaded.input.cost_usd);
                run.wall_seconds
                    .insert(key.clone(), loaded.input.wall_seconds);
                run.collectors.insert(
                    key.clone(),
                    CollectorResult {
                        input: loaded.source.clone(),
                        metrics: loaded.input.metrics.keys().cloned().collect(),
                    },
                );
            }
            inputs.insert(definition.suite.into(), collectors);
        }
        let owner = inputs.get(definition.suite).and_then(|suite| {
            suite
                .owners
                .get(definition.id)
                .and_then(|owner| suite.inputs.get(owner))
        });
        let metric = evaluate(root, definition, owner.map(|loaded| &loaded.input))?;
        run.metrics.insert(definition.id.into(), metric);
    }
    Ok(run)
}

fn load_suite_inputs(root: &Path, suite: &str) -> Result<SuiteInputs> {
    let mut sources = BTreeMap::new();
    let legacy = format!("eval/inputs/{suite}.json");
    if let Some(input) = load_input(root, &legacy, suite)? {
        sources.insert(suite.to_owned(), input);
    }
    let directory = format!("eval/inputs/{suite}");
    let folder = match measurement::safe_file(root, &directory) {
        Ok(path) => Some(path),
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(folder) = folder {
        let entries = fs::read_dir(&folder).map_err(|error| Error::io(&folder, error))?;
        let mut paths = BTreeMap::new();
        for entry in entries {
            let path = entry.map_err(|error| Error::io(&folder, error))?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if id.is_empty()
                || id.len() > 64
                || !id.as_bytes()[0].is_ascii_alphanumeric()
                || !id.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
                })
            {
                return Err(Error::InvalidRequest("collector filenames must use 1–64 lowercase letters, digits, dots, underscores, or hyphens, starting with a letter or digit".into()));
            }
            paths.insert(format!("{suite}/{id}"), format!("{directory}/{id}.json"));
            if sources.len() + paths.len() > MAX_INPUTS_PER_SUITE {
                return Err(Error::InvalidRequest(format!(
                    "{suite} exceeds the {MAX_INPUTS_PER_SUITE}-input limit"
                )));
            }
        }
        for (key, path) in paths {
            let input = load_input(root, &path, suite)?.ok_or_else(|| {
                Error::InvalidRequest(format!("collector input disappeared: {path}"))
            })?;
            sources.insert(key, input);
        }
    }
    let mut owners = BTreeMap::new();
    for (key, loaded) in &sources {
        for id in loaded.input.metrics.keys() {
            if let Some(previous) = owners.insert(id.clone(), key.clone()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate metric ownership for {id}: {previous} and {key}"
                )));
            }
        }
    }
    Ok(SuiteInputs {
        inputs: sources,
        owners,
    })
}

fn load_input(root: &Path, relative: &str, suite: &str) -> Result<Option<LoadedInput>> {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;
    let path = match measurement::safe_file(root, relative) {
        Ok(path) => path,
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if !fs::metadata(&path)
        .map_err(|error| Error::io(&path, error))?
        .is_file()
    {
        return Err(Error::InvalidRequest(format!(
            "collector input is not a regular file: {relative}"
        )));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|error| Error::io(&path, error))?
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(&path, error))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_INPUT_BYTES {
        return Err(Error::InvalidRequest(format!(
            "{relative} exceeds the {MAX_INPUT_BYTES}-byte input limit"
        )));
    }
    let input: Input = serde_json::from_slice(&bytes)?;
    validate_input(&input, suite)?;
    let source = EvidenceFile {
        path: relative.into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        version: input.collector.clone(),
    };
    Ok(Some(LoadedInput { input, source }))
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
            "no owning collector in eval/inputs/{}.json or eval/inputs/{}/; truth/collector not built",
            definition.suite, definition.suite
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
    if git(root, &["rev-parse", "--is-shallow-repository"])? == "true" {
        return Err(Error::InvalidRequest(
            "baseline validation requires complete first-parent Git history".into(),
        ));
    }
    // Audit only commits that change the baseline on the integration history. A
    // committed weakening cannot become trusted merely by becoming HEAD.
    let history = git(
        root,
        &[
            "log",
            "--first-parent",
            "--full-history",
            "--reverse",
            "--format=%H",
            "HEAD",
            "--",
            "eval/baselines.json",
        ],
    )?;
    let mut previous = Baselines::default();
    for commit in history.lines() {
        let baseline = historical_file(root, commit, "eval/baselines.json")?
            .map(|bytes| serde_json::from_slice::<Baselines>(&bytes))
            .transpose()?
            .unwrap_or_default();
        validate_baselines(&baseline)?;
        verify_baseline_transition(&previous, &baseline, |evidence| {
            verify_historical_evidence(root, commit, evidence)
        })
        .map_err(|error| Error::InvalidRequest(format!("baseline history at {commit}: {error}")))?;
        previous = baseline;
    }
    verify_baseline_changes(root, &previous, current)
}

fn historical_file(root: &Path, commit: &str, path: &str) -> Result<Option<Vec<u8>>> {
    measurement::validate_relative_path(path)?;
    let entry = git(
        root,
        &[
            "--literal-pathspecs",
            "ls-tree",
            "--format=%(objectmode) %(objectname)",
            commit,
            "--",
            path,
        ],
    )?;
    if entry.is_empty() {
        return Ok(None);
    }
    let Some((mode, object)) = entry.split_once(' ') else {
        return Err(Error::InvalidRequest(format!(
            "invalid historical evidence entry: {path}"
        )));
    };
    if !matches!(mode, "100644" | "100755") {
        return Err(Error::InvalidRequest(format!(
            "historical evidence is not a regular file: {path}"
        )));
    }
    let output = Command::new("git")
        .args(["cat-file", "blob", object])
        .current_dir(root)
        .output()
        .map_err(|error| Error::io(root, error))?;
    if !output.status.success() {
        return Err(Error::InvalidRequest(format!(
            "cannot read historical evidence: {path}"
        )));
    }
    Ok(Some(output.stdout))
}

fn verify_historical_evidence(root: &Path, commit: &str, evidence: &EvidenceFile) -> Result<bool> {
    use sha2::{Digest, Sha256};
    if evidence.version.trim().is_empty() {
        return Ok(false);
    }
    Ok(historical_file(root, commit, &evidence.path)?
        .is_some_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == evidence.sha256))
}

fn verify_baseline_changes(root: &Path, previous: &Baselines, current: &Baselines) -> Result<()> {
    verify_baseline_transition(previous, current, |evidence| {
        measurement::verify(root, evidence)
    })
}

fn verify_baseline_transition(
    previous: &Baselines,
    current: &Baselines,
    mut verify_evidence: impl FnMut(&EvidenceFile) -> Result<bool>,
) -> Result<()> {
    if !current.adjustments.starts_with(&previous.adjustments) {
        return Err(Error::InvalidRequest(
            "baseline adjustment history was removed or modified".into(),
        ));
    }
    let additions = &current.adjustments[previous.adjustments.len()..];
    for change in additions {
        let definition = definitions()
            .into_iter()
            .find(|d| d.id == change.metric)
            .ok_or_else(|| {
                Error::InvalidRequest(format!("unknown baseline adjustment {}", change.metric))
            })?;
        if definition.kind != Kind::Objective
            || !definition.valid_value(change.previous)
            || !definition.valid_value(change.value)
            || !definition.improves(change.previous, change.value)
            || change.justification.reason.trim().len() < 20
            || !verify_evidence(&change.justification.evidence)?
        {
            return Err(Error::InvalidRequest(format!(
                "invalid recorded justification for {}",
                change.metric
            )));
        }
    }
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
        let mut justified = *old;
        // Several resets may occur before one commit. Each must start at the
        // preceding recorded value; an improvement must be committed before a
        // reset from that improved baseline. Old adjustments cannot be reused.
        for change in additions.iter().filter(|change| change.metric == *id) {
            if change.previous.to_bits() != justified.to_bits() {
                return Err(Error::InvalidRequest(format!(
                    "baseline adjustment for {id} does not match its previous value"
                )));
            }
            justified = change.value;
        }
        if value.to_bits() != justified.to_bits() && !definition.improves(*value, justified) {
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
