//! `lysilogy extract`: resources, pass 1, optional self-check, admission.
//!
//! Every outcome is written to one JSON record per paper, including partial
//! output and the reason a stage failed or was skipped, so a benchmark run
//! never silently drops a paper.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use futures_util::{StreamExt as _, stream};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ABSTRACT_SCHEMA, ANALYSIS_SCHEMA, CARD_SCHEMA, CHECK_SCHEMA, CITATIONS_SCHEMA, CORE_SCHEMA,
    GLOSSARY_SCHEMA, IDENTITY_SCHEMA, OBJECTS_SCHEMA, QUESTION_SCHEMA, REFERENCES_SCHEMA,
    RELATIONS_SCHEMA, REPAIR_SCHEMA, SECTIONS_SCHEMA, SUPERCUT_SCHEMA, TYPE_SCHEMA,
    admission::{
        CheckCounts, LOCAL_REPAIR_SIMILARITY, QuoteSite, SiteKind, SourceText, Verdict,
        apply_removals, check, quote_sites,
    },
    config::{
        Input, InputFormat, Repair as RepairMode, RunConfig, SelfCheck as SelfCheckMode,
        scope_pages,
    },
    core_fields::CORE_FIELDS,
    corrections::{self, CheckResponse, Correction, CorrectionOutcome},
    features::Features,
    objects::{self, ChunkObjects, MergeStats},
    resources::{self, PageResources},
};
use crate::{
    Error, Result,
    analysis::{
        openrouter::{self, ChatRequest, ContentPart, CostSource, ReasoningEffort},
        spend::SpendLedger,
    },
    extract::PdfExtractor,
    store::write_atomic,
};

const CORE_PROMPT: &str = include_str!("../../prompts/paper-core.md");
const FIELDS_PROMPT: &str = include_str!("../../prompts/paper-fields.md");
const SECTIONS_POLICY: &str = include_str!("../../prompts/sectioning.md");
const OBJECTS_PROMPT: &str = include_str!("../../prompts/paper-objects.md");
const SUPERCUT_PROMPT: &str = include_str!("../../prompts/paper-supercut.md");
const ANALYSIS_PROMPT: &str = include_str!("../../prompts/paper-analysis.md");
const GLOSSARY_PROMPT: &str = include_str!("../../prompts/paper-glossary.md");
const CHECK_PROMPT: &str = include_str!("../../prompts/paper-artifacts-check.md");
const REPAIR_PROMPT: &str = include_str!("../../prompts/paper-artifacts-repair.md");

/// Bump when the record shape changes.
const RECORD_VERSION: u16 = 4;

/// Object chunks in flight at once.
const CHUNK_CONCURRENCY: usize = 3;

// Output ceilings per call, reasoning included, so a looping answer fails
// fast instead of running to the provider limit (131k tokens, 16 cents).
const CORE_MAX_TOKENS: u32 = 65_536;
const CHECK_MAX_TOKENS: u32 = 49_152;
const OBJECTS_MAX_TOKENS: u32 = 32_768;
const REPAIR_MAX_TOKENS: u32 = 32_768;
const FEATURE_MAX_TOKENS: u32 = 24_576;

/// The most pages sent with a repair request.
const REPAIR_PAGE_LIMIT: usize = 10;

/// How much unparseable model output a failure keeps.
const RAW_PREVIEW_CHARS: usize = 2_000;

#[derive(Clone, Debug)]
pub struct ExtractOptions {
    pub pdf: PathBuf,
    pub model: String,
    /// 1 or 2; pass 2 is the self-check.
    pub passes: u8,
    pub dpi: u32,
    /// Where page images and text are written.
    pub work_directory: PathBuf,
    /// The JSON record.
    pub output: PathBuf,
    /// No further model call starts once this paper has spent this much.
    /// Calls already in flight finish, so concurrent object chunks can pass
    /// it by up to `CHUNK_CONCURRENCY - 1` calls.
    pub paper_budget_usd: f64,
    pub timeout: Duration,
    /// The optional feature passes to run after the core pass.
    pub features: Features,
    /// Pages per chunk for the chunked object pass.
    pub object_pages: usize,
    /// How much reasoning every model call may spend.
    pub reasoning: ReasoningEffort,
    /// How the core fields are requested: one combined call or one per field.
    pub core_mode: CoreMode,
    /// Providers to try in order; empty lets `OpenRouter` choose.
    pub providers: Vec<String>,
    /// A configured call plan; when set it replaces the core/feature paths.
    pub plan: Option<RunConfig>,
}

/// How the core record's fields are asked for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CoreMode {
    /// One call fills every core field at once.
    #[default]
    Combined,
    /// One call per field, all sharing the same paper prefix so a provider
    /// cache can serve it repeatedly.
    Fanout,
}

impl CoreMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Combined => "combined",
            Self::Fanout => "fanout",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkRecord {
    pub first_page: u32,
    pub last_page: u32,
    /// Objects returned before merging; `None` when the call failed.
    pub objects: Option<usize>,
    /// Equation numbers the text-layer checklist offered.
    pub checklist: usize,
    /// The first answer failed or listed under half the checklist.
    pub retried: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallRecord {
    pub stage: String,
    pub provider: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: u64,
    /// Prompt tokens served from the provider's prefix cache.
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    pub cost_source: CostSource,
    pub latency_s: f64,
}

/// One call's reasoning text, for reading back how the model worked.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasoningTrace {
    pub stage: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub stage: String,
    /// The object chunk, as `p7-12`, when the failure belongs to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<String>,
    pub error: String,
    /// The start of the model output when it could not be parsed.
    pub raw: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteStatus {
    Verbatim,
    RepairedLocal,
    RepairedModel,
    Withdrawn,
    Dropped,
}

#[derive(Clone, Debug, Serialize)]
pub struct SiteOutcome {
    pub kind: SiteKind,
    pub pointer: String,
    pub status: SiteStatus,
    pub original: String,
    pub admitted: Option<String>,
    pub similarity: Option<f64>,
    pub cited_page: Option<u32>,
    pub matched_page: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Totals {
    pub calls: usize,
    pub prompt_tokens: u64,
    /// Prompt tokens served from the provider's prefix cache.
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
    pub model_latency_s: f64,
    pub wall_s: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExtractRecord {
    pub record_version: u16,
    pub started_at: DateTime<Utc>,
    pub pdf: PathBuf,
    pub model: String,
    pub passes_requested: u8,
    /// The optional feature passes that ran, in a stable order.
    pub features: String,
    /// How the core fields were requested: `combined` or `fanout`.
    pub core_mode: String,
    /// The reasoning effort every call was asked to use.
    pub reasoning: String,
    pub dpi: u32,
    pub pages: usize,
    pub calls: Vec<CallRecord>,
    pub totals: Totals,
    /// Why a later stage did not run, when the paper budget stopped it.
    pub budget_stopped: Option<String>,
    pub failures: Vec<Failure>,
    /// Object-chunk failures a retry made good; not counted as failures.
    pub recovered_failures: Vec<Failure>,
    pub object_chunks: Vec<ChunkRecord>,
    pub object_merge: Option<MergeStats>,
    /// Where the model's reasoning traces were written, when it returned any.
    pub reasoning_trace: Option<String>,
    /// Pass 1 with the merged object inventory, before the self-check.
    pub pass1: Option<Value>,
    pub pass1_check: Option<CheckCounts>,
    pub corrections: Vec<Correction>,
    pub correction_outcomes: Vec<CorrectionOutcome>,
    pub pass2_check: Option<CheckCounts>,
    pub admission: Vec<SiteOutcome>,
    pub admission_counts: BTreeMap<String, usize>,
    pub artifacts: Option<Value>,
}

impl ExtractRecord {
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        self.artifacts.is_some() && self.failures.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct RepairResponse {
    repairs: Vec<Repair>,
}

#[derive(Debug, Deserialize)]
struct Repair {
    id: String,
    action: RepairAction,
    quote: Option<String>,
    page: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum RepairAction {
    ReplaceQuote,
    Withdraw,
}

/// The model calls for one paper, with its own spend cap. Calls take `&self`
/// so independent calls can run concurrently.
struct Session<'a> {
    client: reqwest::Client,
    ledger: &'a SpendLedger,
    model: &'a str,
    paper_budget_usd: f64,
    reasoning: ReasoningEffort,
    /// Providers to try in order, so a shared prefix can hit one cache.
    providers: Vec<String>,
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    spent_usd: f64,
    calls: Vec<CallRecord>,
    reasoning: Vec<ReasoningTrace>,
    failures: Vec<Failure>,
    recovered: Vec<Failure>,
    budget_stopped: Option<String>,
}

impl<'a> Session<'a> {
    fn new(
        client: reqwest::Client,
        ledger: &'a SpendLedger,
        model: &'a str,
        paper_budget_usd: f64,
        reasoning: ReasoningEffort,
        providers: Vec<String>,
    ) -> Self {
        Self {
            client,
            ledger,
            model,
            paper_budget_usd,
            reasoning,
            providers,
            state: Mutex::new(SessionState::default()),
        }
    }

    fn state(&self) -> MutexGuard<'_, SessionState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn into_state(self) -> SessionState {
        self.state
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One call parsed as JSON; `None` after recording why there is no value.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        stage: &str,
        parts: &[ContentPart],
        schema: &str,
        max_tokens: u32,
    ) -> Option<T> {
        self.call_for(stage, None, parts, schema, max_tokens).await
    }

    /// A call whose failure belongs to `chunk`, so a later retry can recover it.
    async fn call_for<T: serde::de::DeserializeOwned>(
        &self,
        stage: &str,
        chunk: Option<&str>,
        parts: &[ContentPart],
        schema: &str,
        max_tokens: u32,
    ) -> Option<T> {
        self.call_with(
            stage,
            chunk,
            parts,
            schema,
            max_tokens,
            self.model,
            self.reasoning,
        )
        .await
    }

    /// One call with an explicit model and effort, for a configured plan.
    #[allow(clippy::too_many_arguments)]
    async fn call_with<T: serde::de::DeserializeOwned>(
        &self,
        stage: &str,
        chunk: Option<&str>,
        parts: &[ContentPart],
        schema: &str,
        max_tokens: u32,
        model: &str,
        reasoning: ReasoningEffort,
    ) -> Option<T> {
        if !self.within_budget(stage) {
            return None;
        }
        let schema: Value = match serde_json::from_str(schema) {
            Ok(schema) => schema,
            Err(error) => {
                self.fail(stage, chunk, format!("schema is not JSON: {error}"), None);
                return None;
            }
        };
        let request = ChatRequest {
            model,
            parts,
            schema: &schema,
            stage,
            max_tokens: Some(max_tokens),
            reasoning: Some(reasoning),
            provider_order: &self.providers,
        };
        let completion = match openrouter::complete(&self.client, self.ledger, &request).await {
            Ok(completion) => completion,
            Err(error) => {
                self.fail(stage, chunk, error.to_string(), None);
                return None;
            }
        };
        let parsed = completion.parsed;
        self.record_call(stage, &parsed, completion.latency);
        if parsed.finish_reason.as_deref() == Some("length") {
            self.fail(
                stage,
                chunk,
                "output was cut off at the length limit".to_owned(),
                Some(&parsed.content),
            );
            return None;
        }
        match serde_json::from_str(&parsed.content) {
            Ok(value) => Some(value),
            Err(error) => {
                self.fail(
                    stage,
                    chunk,
                    format!("output does not match the schema: {error}"),
                    Some(&parsed.content),
                );
                None
            }
        }
    }

    /// Whether another call may start; records why not when it may not.
    fn within_budget(&self, stage: &str) -> bool {
        let mut guard = self.state();
        if guard.spent_usd < self.paper_budget_usd {
            return true;
        }
        let reason = format!(
            "{stage} skipped: ${:.4} spent of the ${:.2} paper budget",
            guard.spent_usd, self.paper_budget_usd
        );
        guard.budget_stopped.get_or_insert(reason);
        drop(guard);
        false
    }

    fn record_call(&self, stage: &str, parsed: &openrouter::ParsedCompletion, latency: Duration) {
        let mut guard = self.state();
        guard.spent_usd += parsed.cost_usd;
        guard.calls.push(CallRecord {
            stage: stage.to_owned(),
            provider: parsed.provider.clone(),
            finish_reason: parsed.finish_reason.clone(),
            prompt_tokens: parsed.prompt_tokens,
            cached_tokens: parsed.cached_tokens,
            completion_tokens: parsed.completion_tokens,
            reasoning_tokens: parsed.reasoning_tokens,
            cost_usd: parsed.cost_usd,
            cost_source: parsed.cost_source,
            latency_s: latency.as_secs_f64(),
        });
        if let Some(text) = &parsed.reasoning {
            guard.reasoning.push(ReasoningTrace {
                stage: stage.to_owned(),
                text: text.clone(),
            });
        }
    }

    /// Move the failures of a chunk whose retry succeeded out of the failures.
    fn recover(&self, chunk: &str) {
        let mut guard = self.state();
        let (recovered, failures) = std::mem::take(&mut guard.failures)
            .into_iter()
            .partition(|failure| failure.chunk.as_deref() == Some(chunk));
        guard.failures = failures;
        guard.recovered.extend::<Vec<Failure>>(recovered);
    }

    fn fail(&self, stage: &str, chunk: Option<&str>, error: String, raw: Option<&str>) {
        self.state().failures.push(Failure {
            stage: stage.to_owned(),
            chunk: chunk.map(str::to_owned),
            error,
            raw: raw.map(|raw| raw.chars().take(RAW_PREVIEW_CHARS).collect()),
        });
    }
}

#[derive(Deserialize)]
struct ObjectsResponse {
    objects: Vec<Value>,
}

/// Extract one paper and write its record to `options.output`.
///
/// # Errors
/// Fails only when the options are invalid, the PDF cannot be turned into
/// page resources, or the record cannot be written. Model failures are
/// recorded in the returned record instead.
#[allow(clippy::too_many_lines)]
pub async fn extract(options: &ExtractOptions, ledger: &SpendLedger) -> Result<ExtractRecord> {
    if !(1..=2).contains(&options.passes) {
        return Err(Error::InvalidRequest(format!(
            "passes must be 1 or 2, not {}",
            options.passes
        )));
    }
    let started = Instant::now();
    let started_at = Utc::now();
    let resources = resources::build(
        &PdfExtractor::default(),
        &options.pdf,
        &options.work_directory,
        options.dpi,
    )
    .await?;
    let text = text_parts(&resources);
    let source = SourceText::new(&resources.paper.pages);
    let session = Session::new(
        openrouter::client(options.timeout)?,
        ledger,
        &options.model,
        options.paper_budget_usd,
        options.reasoning,
        options.providers.clone(),
    );

    let (core, object_chunks, object_merge) = if let Some(plan) = &options.plan {
        run_plan(&session, &resources, plan, options.object_pages).await
    } else {
        let core = match options.core_mode {
            CoreMode::Combined => core_pass(&session, &resources, &text).await,
            CoreMode::Fanout => fanout_core(&session, &resources, &text).await,
        };
        let mut draft = core.clone();
        let (chunks, merge) = feature_passes(
            &mut draft,
            &session,
            &resources,
            &text,
            options.features,
            options.object_pages,
        )
        .await;
        (draft, chunks, merge)
    };
    let pass1_check = core.as_ref().map(|draft| check(&source, draft));

    let mut draft = core.clone();
    let self_check_enabled = options.plan.as_ref().map_or(options.passes == 2, |plan| {
        plan.verify.self_check == SelfCheckMode::Full
    });
    let repair_model = options
        .plan
        .as_ref()
        .is_none_or(|plan| plan.verify.repair == RepairMode::Model);
    let hide_objects = options.plan.as_ref().map_or_else(
        || options.features.any_objects(),
        |plan| {
            plan.calls
                .iter()
                .any(|call| call.fields.iter().any(|field| field == "objects"))
        },
    );
    let schema = options
        .plan
        .as_ref()
        .map_or_else(|| draft_schema(options.features), plan_schema);
    let check_pass = match draft.as_mut() {
        Some(current) if self_check_enabled => {
            self_check(current, &text, &source, &session, hide_objects, &schema).await?
        }
        _ => SelfCheck::default(),
    };
    let admission = match draft.as_mut() {
        Some(current) => admit(current, &source, &resources, &session, repair_model).await,
        None => Vec::new(),
    };
    let mut admission_counts = BTreeMap::new();
    for outcome in &admission {
        let key = serde_json::to_value(outcome.status)?
            .as_str()
            .unwrap_or_default()
            .to_owned();
        *admission_counts.entry(key).or_insert(0) += 1;
    }

    let session = session.into_state();
    let reasoning_trace = write_reasoning_traces(&options.output, &session.reasoning).await?;
    let totals = Totals {
        calls: session.calls.len(),
        prompt_tokens: session.calls.iter().map(|call| call.prompt_tokens).sum(),
        cached_tokens: session.calls.iter().map(|call| call.cached_tokens).sum(),
        completion_tokens: session
            .calls
            .iter()
            .map(|call| call.completion_tokens)
            .sum(),
        cost_usd: session.spent_usd,
        model_latency_s: session.calls.iter().map(|call| call.latency_s).sum(),
        wall_s: started.elapsed().as_secs_f64(),
    };
    let record = ExtractRecord {
        record_version: RECORD_VERSION,
        started_at,
        pdf: options.pdf.clone(),
        model: options.model.clone(),
        passes_requested: options.passes,
        features: options.features.label(),
        core_mode: options.plan.as_ref().map_or_else(
            || options.core_mode.label().to_owned(),
            |_| "plan".to_owned(),
        ),
        reasoning: options.reasoning.label().to_owned(),
        dpi: resources.dpi,
        pages: resources.paper.pages.len(),
        calls: session.calls,
        totals,
        budget_stopped: session.budget_stopped,
        failures: session.failures,
        recovered_failures: session.recovered,
        object_chunks,
        object_merge,
        reasoning_trace,
        pass1: core,
        pass1_check,
        corrections: check_pass.corrections,
        correction_outcomes: check_pass.outcomes,
        pass2_check: check_pass.counts,
        admission,
        admission_counts,
        artifacts: draft,
    };
    write_record(&options.output, &record).await?;
    Ok(record)
}

/// Write one JSON line per call's reasoning beside the result, for later reading.
async fn write_reasoning_traces(
    output: &Path,
    traces: &[ReasoningTrace],
) -> Result<Option<String>> {
    if traces.is_empty() {
        return Ok(None);
    }
    let path = output.with_extension("reasoning.jsonl");
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| Error::io(parent, error))?;
    }
    let mut text = String::new();
    for trace in traces {
        text.push_str(&serde_json::to_string(trace)?);
        text.push('\n');
    }
    tokio::fs::write(&path, text)
        .await
        .map_err(|error| Error::io(&path, error))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

async fn write_record(output: &Path, record: &ExtractRecord) -> Result<()> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| Error::io(parent, error))?;
    }
    write_atomic(output, &serde_json::to_vec_pretty(record)?).await
}

#[derive(Default)]
struct SelfCheck {
    corrections: Vec<Correction>,
    outcomes: Vec<CorrectionOutcome>,
    counts: Option<CheckCounts>,
}

/// Pass 2: the model may only correct or flag the draft.
async fn self_check(
    draft: &mut Value,
    text: &[ContentPart],
    source: &SourceText<'_>,
    session: &Session<'_>,
    hide_objects: bool,
    schema: &Value,
) -> Result<SelfCheck> {
    // Objects are verified against the page images elsewhere; the text-only
    // self-check spends nothing on them.
    let mut view = draft.clone();
    if hide_objects && let Some(fields) = view.as_object_mut() {
        fields.remove("objects");
    }
    let parts = vec![
        ContentPart::Text(CHECK_PROMPT.to_owned()),
        ContentPart::Text(format!(
            "Draft:\n```json\n{}\n```",
            serde_json::to_string(&view)?
        )),
        ContentPart::Text("Paper:".to_owned()),
    ];
    let mut parts = parts;
    parts.extend(text.iter().cloned());
    let Some(response) = session
        .call::<CheckResponse>("extract.pass2", &parts, CHECK_SCHEMA, CHECK_MAX_TOKENS)
        .await
    else {
        return Ok(SelfCheck::default());
    };
    let outcomes =
        corrections::apply_with_schema(draft, &response.corrections, source, Some(schema));
    Ok(SelfCheck {
        corrections: response.corrections,
        outcomes,
        counts: Some(check(source, draft)),
    })
}

/// The core pass: every field every extraction needs, from text alone.
async fn core_pass(
    session: &Session<'_>,
    resources: &PageResources,
    text: &[ContentPart],
) -> Option<Value> {
    let prompt = CORE_PROMPT.replace(
        "{{sections}}",
        &sections_guidance(resources.paper.pages.len()),
    );
    let mut parts = vec![ContentPart::Text(prompt)];
    parts.extend(text.iter().cloned());
    session
        .call::<Value>("extract.pass1", &parts, CORE_SCHEMA, CORE_MAX_TOKENS)
        .await
}

/// The core fields, one call each over the same paper prefix. The fields run
/// in order so the first call can populate a provider's prefix cache and the
/// rest can read it back.
async fn fanout_core(
    session: &Session<'_>,
    resources: &PageResources,
    text: &[ContentPart],
) -> Option<Value> {
    let mut draft = serde_json::Map::new();
    let mut abstract_answered = false;
    for field in CORE_FIELDS {
        let shared = shared_prefix(FIELDS_PROMPT, text, field.page_limit);
        let instruction = field.instruction.replace(
            "{{sections}}",
            &sections_guidance(resources.paper.pages.len()),
        );
        let parts = [ContentPart::Text(shared), ContentPart::Text(instruction)];
        let Some(value) = session
            .call::<Value>(field.stage, &parts, field.schema, FEATURE_MAX_TOKENS)
            .await
        else {
            continue;
        };
        if field.name == "abstract" {
            abstract_answered = true;
        }
        merge_fields(&mut draft, &value);
    }
    if abstract_answered
        && draft
            .get("abstract")
            .and_then(|value| value.get("text"))
            .and_then(Value::as_str)
            .is_none()
    {
        session.fail(
            "extract.abstract",
            None,
            "no abstract found in the first three pages".to_owned(),
            None,
        );
    }
    (!draft.is_empty()).then_some(Value::Object(draft))
}

/// The shared prefix every field call carries, so a provider cache can hit it.
/// `limit` keeps only the first pages, for the abstract shortcut.
fn shared_prefix(preamble: &str, text: &[ContentPart], limit: Option<usize>) -> String {
    let mut out = String::from(preamble);
    for part in text.iter().take(limit.unwrap_or(usize::MAX)) {
        if let ContentPart::Text(page) = part {
            out.push('\n');
            out.push_str(page);
        }
    }
    out
}

/// Whether a config field name is one a call can fill.
#[must_use]
pub fn is_unit(name: &str) -> bool {
    unit_schema(name).is_some()
}

/// Whether a config field name is one of the nine core fields.
#[must_use]
fn is_core_field(name: &str) -> bool {
    super::core_fields::CORE_FIELDS
        .iter()
        .any(|field| field.name == name)
}

/// The output ceiling for a plan call: explicit, else by the fields it carries.
fn plan_max_tokens(call: &super::config::Call) -> u32 {
    call.max_tokens.unwrap_or_else(|| {
        if call.fields.iter().any(|field| is_core_field(field)) {
            CORE_MAX_TOKENS
        } else {
            FEATURE_MAX_TOKENS
        }
    })
}

fn unit_schema(name: &str) -> Option<&'static str> {
    Some(match name {
        "identity" => IDENTITY_SCHEMA,
        "type" => TYPE_SCHEMA,
        "abstract" => ABSTRACT_SCHEMA,
        "card" => CARD_SCHEMA,
        "question" => QUESTION_SCHEMA,
        "sections" => SECTIONS_SCHEMA,
        "references" => REFERENCES_SCHEMA,
        "citations" => CITATIONS_SCHEMA,
        "relations" => RELATIONS_SCHEMA,
        "objects" => OBJECTS_SCHEMA,
        "supercut" => SUPERCUT_SCHEMA,
        "analysis" => ANALYSIS_SCHEMA,
        "glossary" => GLOSSARY_SCHEMA,
        _ => return None,
    })
}

fn unit_instruction(name: &str, page_count: usize) -> String {
    let instruction = CORE_FIELDS
        .iter()
        .find(|field| field.name == name)
        .map_or_else(
            || match name {
                "objects" => "Return the `objects` field.",
                "supercut" => "Return the `supercut` field: the argument as ordered source and connector segments.",
                "analysis" => "Return the `analysis` fields: claims, method, key numbers, limitations, open questions, headline results, and the read signal.",
                "glossary" => "Return the `notation` and `glossary` fields.",
                _ => "",
            },
            |field| field.instruction,
        );
    instruction.replace("{{sections}}", &sections_guidance(page_count))
}

/// The union schema for the fields one call asks for.
fn call_schema(call: &super::config::Call) -> Value {
    let mut schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [],
        "properties": {}
    });
    for field in &call.fields {
        if let Some(text) = unit_schema(field) {
            merge_properties(&mut schema, text);
        }
    }
    schema
}

/// The union schema for every field a plan asks for.
fn plan_schema(plan: &RunConfig) -> Value {
    let mut schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [],
        "properties": {}
    });
    for call in &plan.calls {
        for field in &call.fields {
            if let Some(text) = unit_schema(field) {
                merge_properties(&mut schema, text);
            }
        }
    }
    schema
}

/// Run a configured list of calls and merge their answers into one draft.
#[allow(clippy::too_many_lines)]
async fn run_plan(
    session: &Session<'_>,
    resources: &PageResources,
    plan: &RunConfig,
    object_pages: usize,
) -> (Option<Value>, Vec<ChunkRecord>, Option<MergeStats>) {
    let mut draft = serde_json::Map::new();
    let mut chunks = Vec::new();
    let mut merge = None;
    for call in &plan.calls {
        let scope = scope_pages(call.scope.as_deref());
        if call.fields.iter().any(|field| field == "objects") {
            let kinds = ["figure", "table", "algorithm", "equation"];
            let page_parts = match page_parts(resources).await {
                Ok(parts) => parts,
                Err(error) => {
                    session.fail(&call.name, None, error.to_string(), None);
                    continue;
                }
            };
            let (inventory, call_merge, call_chunks) =
                object_pass(session, resources, &page_parts, &kinds, object_pages).await;
            draft.insert("objects".to_owned(), Value::Array(inventory));
            chunks.extend(call_chunks);
            merge = Some(call_merge);
            continue;
        }
        let schema = call_schema(call);
        let Ok(schema) = serde_json::to_string(&schema) else {
            continue;
        };
        let instruction = call
            .fields
            .iter()
            .map(|field| unit_instruction(field, resources.paper.pages.len()))
            .collect::<Vec<_>>()
            .join("\n");
        let mut parts = vec![ContentPart::Text(FIELDS_PROMPT.to_owned())];
        match rendered_parts(resources, &plan.input, scope).await {
            Ok(mut paper) => parts.append(&mut paper),
            Err(error) => {
                session.fail(&call.name, None, error.to_string(), None);
                continue;
            }
        }
        parts.push(ContentPart::Text(instruction));
        let model = plan.model_for(call);
        let max_tokens = plan_max_tokens(call);
        let value = session
            .call_with::<Value>(
                &call.name,
                None,
                &parts,
                &schema,
                max_tokens,
                &model,
                call.effort.reasoning(),
            )
            .await;
        if let Some(value) = value {
            merge_fields(&mut draft, &value);
        }
    }
    let draft = (!draft.is_empty()).then_some(Value::Object(draft));
    (draft, chunks, merge)
}

/// The paper as the model sees it: raw page-marked text or markdown, first
/// pages only when scoped, page images optional, with a metadata block first.
async fn rendered_parts(
    resources: &PageResources,
    input: &Input,
    scope: Option<usize>,
) -> Result<Vec<ContentPart>> {
    let limit = scope.unwrap_or(usize::MAX);
    let mut parts = Vec::new();
    if input.priors {
        parts.push(ContentPart::Text(front_matter(resources)));
    }
    match input.format {
        InputFormat::Raw => {
            for (index, page) in resources.paper.pages.iter().enumerate() {
                if index >= limit {
                    break;
                }
                parts.push(ContentPart::Text(format!(
                    "<<page {}>>\n{}",
                    page.number,
                    page.text.trim()
                )));
                if input.images {
                    let image = &resources.images[index];
                    let bytes = tokio::fs::read(image)
                        .await
                        .map_err(|error| Error::io(image, error))?;
                    parts.push(ContentPart::Png(bytes));
                }
            }
        }
        InputFormat::Markdown => {
            let mut paper = resources.paper.clone();
            if let Some(limit) = scope {
                paper.pages.truncate(limit);
            }
            parts.push(ContentPart::Text(crate::markdown::render_source(&paper)));
        }
    }
    Ok(parts)
}

/// A short deterministic front-matter block, when the PDF metadata has any.
fn front_matter(resources: &PageResources) -> String {
    use std::fmt::Write as _;
    let metadata = &resources.paper.metadata;
    let mut text =
        String::from("Deterministic front matter from the PDF (may be incomplete or wrong):");
    if !metadata.title.is_empty() {
        let _ = write!(text, "\n- title: {}", metadata.title);
    }
    if !metadata.authors.is_empty() {
        let _ = write!(text, "\n- authors: {}", metadata.authors.join(", "));
    }
    if let Some(year) = metadata.year {
        let _ = write!(text, "\n- year: {year}");
    }
    text.push_str("\nEnd of front matter.\n");
    text
}

/// Run the optional feature passes that were asked for and merge them in.
/// Objects go through the chunked pass; the rest are one text call each.
async fn feature_passes(
    draft: &mut Option<Value>,
    session: &Session<'_>,
    resources: &PageResources,
    text: &[ContentPart],
    features: Features,
    object_pages: usize,
) -> (Vec<ChunkRecord>, Option<MergeStats>) {
    if draft.is_none() {
        return (Vec::new(), None);
    }
    let supercut = async {
        if features.supercut {
            text_feature(
                session,
                text,
                "extract.supercut",
                SUPERCUT_PROMPT,
                SUPERCUT_SCHEMA,
            )
            .await
        } else {
            None
        }
    };
    let analysis = async {
        if features.analysis {
            text_feature(
                session,
                text,
                "extract.analysis",
                ANALYSIS_PROMPT,
                ANALYSIS_SCHEMA,
            )
            .await
        } else {
            None
        }
    };
    let glossary = async {
        if features.glossary {
            text_feature(
                session,
                text,
                "extract.glossary",
                GLOSSARY_PROMPT,
                GLOSSARY_SCHEMA,
            )
            .await
        } else {
            None
        }
    };
    let objects = async {
        if features.any_objects() {
            let kinds = features.object_kinds();
            let page_parts = match page_parts(resources).await {
                Ok(parts) => parts,
                Err(error) => {
                    session.fail("extract.objects", None, error.to_string(), None);
                    return (Vec::new(), None, Vec::new());
                }
            };
            let (inventory, merge, chunks) =
                object_pass(session, resources, &page_parts, &kinds, object_pages).await;
            (chunks, Some(merge), inventory)
        } else {
            (Vec::new(), None, Vec::new())
        }
    };
    let (supercut, analysis, glossary, (chunks, merge, inventory)) =
        tokio::join!(supercut, analysis, glossary, objects);
    if let Some(draft) = draft.as_mut() {
        let Some(fields) = draft.as_object_mut() else {
            return (chunks, merge);
        };
        for value in [supercut, analysis, glossary].into_iter().flatten() {
            merge_fields(fields, &value);
        }
        if features.any_objects() {
            fields.insert("objects".to_owned(), Value::Array(inventory));
        }
    }
    (chunks, merge)
}

/// One text-only feature call that returns the fields named by its schema.
async fn text_feature(
    session: &Session<'_>,
    text: &[ContentPart],
    stage: &str,
    prompt: &str,
    schema: &str,
) -> Option<Value> {
    let mut parts = vec![ContentPart::Text(prompt.to_owned())];
    parts.extend(text.iter().cloned());
    session
        .call::<Value>(stage, &parts, schema, FEATURE_MAX_TOKENS)
        .await
}

/// Merge a feature answer into the draft. Headline results and the read
/// signal belong under `card`.
fn merge_fields(draft: &mut serde_json::Map<String, Value>, fields: &Value) {
    let Some(fields) = fields.as_object() else {
        return;
    };
    for (key, value) in fields {
        if matches!(key.as_str(), "headline_results" | "read_signal") {
            if let Some(card) = draft.get_mut("card").and_then(Value::as_object_mut) {
                card.insert(key.clone(), value.clone());
            }
        } else {
            draft.insert(key.clone(), value.clone());
        }
    }
}

/// The schema for a draft, so a self-check correction can be validated against
/// the shape the app expects. Feature fields are added when the feature ran.
fn draft_schema(features: Features) -> Value {
    let mut base: Value = serde_json::from_str(CORE_SCHEMA).unwrap_or(Value::Null);
    if features.any_objects() {
        merge_properties(&mut base, OBJECTS_SCHEMA);
    }
    if features.supercut {
        merge_properties(&mut base, SUPERCUT_SCHEMA);
    }
    if features.analysis {
        merge_properties(&mut base, ANALYSIS_SCHEMA);
    }
    if features.glossary {
        merge_properties(&mut base, GLOSSARY_SCHEMA);
    }
    base
}

/// Add the properties and required keys of `extra` to `base`.
fn merge_properties(base: &mut Value, extra: &str) {
    let Ok(extra) = serde_json::from_str::<Value>(extra) else {
        return;
    };
    let Some(extra_properties) = extra.get("properties").and_then(Value::as_object) else {
        return;
    };
    if let Some(base_properties) = base.get_mut("properties").and_then(Value::as_object_mut) {
        for (key, value) in extra_properties {
            base_properties.insert(key.clone(), value.clone());
        }
    }
    let extra_required = extra
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(base_required) = base.get_mut("required").and_then(Value::as_array_mut) {
        for key in extra_required {
            if !base_required.contains(&key) {
                base_required.push(key);
            }
        }
    }
}

/// The sectioning policy plus a page-count budget, for `{{sections}}`.
fn sections_guidance(page_count: usize) -> String {
    let pages = page_count.max(1);
    let lower = pages.div_ceil(2);
    let upper = pages.saturating_mul(2).div_ceil(3).max(lower);
    format!(
        "{SECTIONS_POLICY}\nThis PDF contains {pages} pages. Start by considering roughly {lower}\u{2013}{upper} substantive reading units, then adjust for actual topic boundaries and subtract references, appendices, or adjacent articles from the main-text budget. This is a planning guide, not a quota or a reason to drop content."
    )
}

/// The object inventory, read a chunk of pages at a time and merged.
async fn object_pass(
    session: &Session<'_>,
    resources: &PageResources,
    pages: &[ContentPart],
    kinds: &[&str],
    pages_per_chunk: usize,
) -> (Vec<Value>, MergeStats, Vec<ChunkRecord>) {
    let loads = resources
        .paper
        .pages
        .iter()
        .map(|page| (page.number, objects::equation_candidates(&page.text).len()))
        .collect::<Vec<_>>();
    let chunks = objects::page_chunks(&loads, pages_per_chunk, objects::CHUNK_EQUATION_LOAD);
    let prompt = OBJECTS_PROMPT.replace("{{kinds}}", &kinds.join(", "));
    let mut start = 0;
    let requests = chunks
        .into_iter()
        .map(|chunk| {
            // `pages` holds a text part and an image part per page, in order.
            let chunk_pages = &resources.paper.pages[start..start + chunk.len()];
            let mut parts = vec![ContentPart::Text(prompt.clone())];
            parts.extend(pages[start * 2..(start + chunk.len()) * 2].iter().cloned());
            parts.push(ContentPart::Text(checklist(chunk_pages)));
            let candidates = chunk_pages
                .iter()
                .map(|page| objects::equation_candidates(&page.text).len())
                .sum::<usize>();
            start += chunk.len();
            (chunk, parts, candidates)
        })
        .collect::<Vec<_>>();
    let answers = stream::iter(
        requests
            .into_iter()
            .map(|(chunk, parts, candidates)| async move {
                let tag = format!(
                    "p{}-{}",
                    chunk.first().copied().unwrap_or_default(),
                    chunk.last().copied().unwrap_or_default()
                );
                let ask = |stage| {
                    let (parts, tag) = (&parts, &tag);
                    async move {
                        session
                            .call_for::<ObjectsResponse>(
                                stage,
                                Some(tag),
                                parts,
                                OBJECTS_SCHEMA,
                                OBJECTS_MAX_TOKENS,
                            )
                            .await
                            .map(|response| response.objects)
                    }
                };
                let first = ask("extract.objects").await;
                let retried = objects::needs_retry(first.as_deref(), candidates);
                let objects = if retried {
                    let objects = objects::better(first, ask("extract.objects.retry").await);
                    if objects.is_some() {
                        session.recover(&tag);
                    }
                    objects
                } else {
                    first
                };
                // Keep only the kinds that were requested, whatever the model returned.
                let objects = objects.map(|objects| keep_requested_kinds(objects, kinds));
                (
                    ChunkObjects {
                        pages: chunk,
                        objects,
                    },
                    candidates,
                    retried,
                )
            }),
    )
    .buffered(CHUNK_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let records = answers
        .iter()
        .map(|(chunk, candidates, retried)| ChunkRecord {
            first_page: chunk.pages.first().copied().unwrap_or_default(),
            last_page: chunk.pages.last().copied().unwrap_or_default(),
            objects: chunk.objects.as_ref().map(Vec::len),
            checklist: *candidates,
            retried: *retried,
        })
        .collect();
    let results = answers.into_iter().map(|(chunk, _, _)| chunk).collect();
    let (inventory, merge) = objects::merge(results);
    (inventory, merge, records)
}

/// The objects whose kind was actually requested.
fn keep_requested_kinds(objects: Vec<Value>, kinds: &[&str]) -> Vec<Value> {
    objects
        .into_iter()
        .filter(|object| {
            object
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kinds.contains(&kind))
        })
        .collect()
}

/// The equation numbers the text layer shows on each page of a chunk.
fn checklist(pages: &[crate::domain::ExtractedPage]) -> String {
    use std::fmt::Write as _;
    let mut text = "Checklist of equation numbers in the text layer:".to_owned();
    let mut any = false;
    for page in pages {
        let candidates = objects::equation_candidates(&page.text);
        if candidates.is_empty() {
            continue;
        }
        any = true;
        let labels = candidates
            .iter()
            .map(|label| format!("({label})"))
            .collect::<Vec<_>>()
            .join(" ");
        // Writing to a String cannot fail.
        let _ = write!(text, "\n- page {}: {labels}", page.number);
    }
    if !any {
        text.push_str(" none found.");
    }
    text
}

/// Each page's text part alone, for the text-only core and feature passes.
fn text_parts(resources: &PageResources) -> Vec<ContentPart> {
    resources
        .paper
        .pages
        .iter()
        .map(|page| ContentPart::Text(format!("<<page {}>>\n{}", page.number, page.text.trim())))
        .collect()
}

/// Each page's text followed by its image.
async fn page_parts(resources: &PageResources) -> Result<Vec<ContentPart>> {
    let mut parts = Vec::with_capacity(resources.images.len() * 2);
    for (page, image) in resources.paper.pages.iter().zip(&resources.images) {
        parts.push(ContentPart::Text(format!(
            "<<page {}>>\n{}",
            page.number,
            page.text.trim()
        )));
        let bytes = tokio::fs::read(image)
            .await
            .map_err(|error| Error::io(image, error))?;
        parts.push(ContentPart::Png(bytes));
    }
    Ok(parts)
}

/// Admit every quote: verbatim, repaired locally, repaired once by the
/// model, or removed.
async fn admit(
    draft: &mut Value,
    source: &SourceText<'_>,
    resources: &PageResources,
    session: &Session<'_>,
    repair_model: bool,
) -> Vec<SiteOutcome> {
    let mut outcomes = Vec::new();
    let mut failing = Vec::<(usize, QuoteSite)>::new();
    for site in quote_sites(draft) {
        let original = site.quote(draft).unwrap_or_default().to_owned();
        let cited = site.page(draft);
        let mut outcome = SiteOutcome {
            kind: site.kind,
            pointer: site.quote_pointer.clone(),
            status: SiteStatus::Dropped,
            original: original.clone(),
            admitted: None,
            similarity: None,
            cited_page: cited,
            matched_page: None,
        };
        match source.verdict(&original, cited) {
            Verdict::Verbatim { page } => {
                set_page(draft, &site, page);
                outcome.status = SiteStatus::Verbatim;
                outcome.admitted = Some(original);
                outcome.matched_page = Some(page);
            }
            Verdict::Near(found) if found.similarity() >= LOCAL_REPAIR_SIMILARITY => {
                set_quote(draft, &site, &found.text, found.page);
                outcome.status = SiteStatus::RepairedLocal;
                outcome.similarity = Some(found.similarity());
                outcome.admitted = Some(found.text);
                outcome.matched_page = Some(found.page);
            }
            Verdict::Near(found) => {
                outcome.similarity = Some(found.similarity());
                failing.push((outcomes.len(), site));
            }
            Verdict::Missing => failing.push((outcomes.len(), site)),
        }
        outcomes.push(outcome);
    }
    if !failing.is_empty() && repair_model {
        let repairs = request_repairs(&failing, &outcomes, resources, session).await;
        for (index, site) in &failing {
            let outcome = &mut outcomes[*index];
            let repair = repairs.get(&format!("q{index}"));
            match repair {
                Some(Repair {
                    action: RepairAction::Withdraw,
                    ..
                }) => outcome.status = SiteStatus::Withdrawn,
                Some(Repair {
                    action: RepairAction::ReplaceQuote,
                    quote: Some(quote),
                    page,
                    ..
                }) => match source.verdict(quote, page.or(outcome.cited_page)) {
                    Verdict::Verbatim { page } => {
                        set_quote(draft, site, quote, page);
                        outcome.status = SiteStatus::RepairedModel;
                        outcome.admitted = Some(quote.clone());
                        outcome.matched_page = Some(page);
                    }
                    Verdict::Near(found) if found.similarity() >= LOCAL_REPAIR_SIMILARITY => {
                        set_quote(draft, site, &found.text, found.page);
                        outcome.status = SiteStatus::RepairedModel;
                        outcome.admitted = Some(found.text);
                        outcome.matched_page = Some(found.page);
                    }
                    Verdict::Near(_) | Verdict::Missing => {}
                },
                _ => {}
            }
        }
    }
    let removals = failing
        .iter()
        .filter(|(index, _)| {
            matches!(
                outcomes[*index].status,
                SiteStatus::Withdrawn | SiteStatus::Dropped
            )
        })
        .map(|(_, site)| site.removal.clone())
        .collect::<Vec<_>>();
    apply_removals(draft, &removals);
    outcomes
}

async fn request_repairs(
    failing: &[(usize, QuoteSite)],
    outcomes: &[SiteOutcome],
    resources: &PageResources,
    session: &Session<'_>,
) -> BTreeMap<String, Repair> {
    let items = failing
        .iter()
        .map(|(index, site)| {
            serde_json::json!({
                "id": format!("q{index}"),
                "for": site.kind,
                "quote": outcomes[*index].original,
                "page": outcomes[*index].cited_page,
            })
        })
        .collect::<Vec<_>>();
    let mut wanted = BTreeSet::new();
    for (index, _) in failing {
        if let Some(page) = outcomes[*index].cited_page {
            wanted.extend([page.saturating_sub(1), page, page + 1]);
        }
    }
    let mut parts = vec![
        ContentPart::Text(REPAIR_PROMPT.to_owned()),
        ContentPart::Text(format!(
            "Quotes:\n```json\n{}\n```",
            serde_json::to_string_pretty(&items).unwrap_or_default()
        )),
    ];
    parts.extend(
        resources
            .paper
            .pages
            .iter()
            .filter(|page| wanted.is_empty() || wanted.contains(&page.number))
            .take(REPAIR_PAGE_LIMIT)
            .map(|page| {
                ContentPart::Text(format!("<<page {}>>\n{}", page.number, page.text.trim()))
            }),
    );
    session
        .call::<RepairResponse>("extract.repair", &parts, REPAIR_SCHEMA, REPAIR_MAX_TOKENS)
        .await
        .map(|response| {
            response
                .repairs
                .into_iter()
                .map(|repair| (repair.id.clone(), repair))
                .collect()
        })
        .unwrap_or_default()
}

fn set_quote(draft: &mut Value, site: &QuoteSite, quote: &str, page: u32) {
    if let Some(slot) = draft.pointer_mut(&site.quote_pointer) {
        *slot = Value::String(quote.to_owned());
    }
    set_page(draft, site, page);
}

fn set_page(draft: &mut Value, site: &QuoteSite, page: u32) {
    if let Some(pointer) = &site.page_pointer
        && let Some(slot) = draft.pointer_mut(pointer)
    {
        *slot = Value::from(page);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{DocumentLayout, ExtractedPage, ExtractedPaper, PaperMetadata};

    fn resources() -> PageResources {
        PageResources {
            paper: ExtractedPaper {
                metadata: PaperMetadata::default(),
                pages: vec![
                    ExtractedPage {
                        number: 1,
                        text: "Sparse Models Under Noise\nWe show that pruning preserves accuracy.".to_owned(),
                    },
                    ExtractedPage {
                        number: 2,
                        text: "Table 1: Accuracy after pruning.\nPruning half the weights costs one point.".to_owned(),
                    },
                ],
                layout: DocumentLayout::default(),
            },
            images: Vec::new(),
            dpi: resources::DEFAULT_DPI,
        }
    }

    fn session(ledger: &SpendLedger, paper_budget_usd: f64) -> Session<'_> {
        Session::new(
            reqwest::Client::new(),
            ledger,
            openrouter::DEFAULT_MODEL,
            paper_budget_usd,
            ReasoningEffort::Low,
            Vec::new(),
        )
    }

    fn draft() -> Value {
        json!({
            "abstract": {"text": "We show that pruning preserves accuracy.", "page": 2},
            "claims": [{"id": "c1", "evidence": [
                {"quote": "Pruning half of the weights costs one point.", "page": 2},
                {"quote": "Pruning always improves robustness to adversaries.", "page": 2}
            ]}],
            "supercut": [
                {"kind": "connector", "text": "The paper asks whether pruning hurts.", "page": null},
                {"kind": "source", "text": "An invented sentence the paper never says at all.", "page": 1}
            ],
            "objects": [{"caption": "Table 1: Accuracy after pruning.", "page": 2}]
        })
    }

    #[tokio::test]
    async fn should_drop_failing_quotes_when_the_budget_forbids_a_repair_call() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(directory.path().join("spend.jsonl"), 1.0);
        let resources = resources();
        let source = SourceText::new(&resources.paper.pages);
        let session = session(&ledger, 0.0);
        let mut value = draft();
        let outcomes = admit(&mut value, &source, &resources, &session, true).await;
        let statuses = outcomes
            .iter()
            .map(|outcome| (outcome.pointer.as_str(), outcome.status))
            .collect::<Vec<_>>();
        assert_eq!(
            statuses,
            [
                ("/abstract/text", SiteStatus::Verbatim),
                ("/claims/0/evidence/0/quote", SiteStatus::RepairedLocal),
                ("/claims/0/evidence/1/quote", SiteStatus::Dropped),
                ("/supercut/1/text", SiteStatus::Dropped),
                ("/objects/0/caption", SiteStatus::Verbatim),
            ]
        );
        assert!(
            session.state().budget_stopped.is_some(),
            "the repair call must be skipped"
        );
        assert!(session.state().calls.is_empty());
        // The verbatim abstract's page is corrected to where it actually is.
        assert_eq!(value["abstract"]["page"], 1);
        assert_eq!(
            value["claims"][0]["evidence"],
            json!([{"quote": "Pruning half the weights costs one point.", "page": 2}])
        );
        assert_eq!(value["supercut"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["supercut"][0]["kind"], "connector");
    }

    #[tokio::test]
    async fn should_make_no_call_when_every_quote_is_admitted_locally() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(directory.path().join("spend.jsonl"), 1.0);
        let resources = resources();
        let source = SourceText::new(&resources.paper.pages);
        let session = session(&ledger, 1.0);
        let mut value =
            json!({"objects": [{"caption": "Table 1: Accuracy after pruning.", "page": 2}]});
        let outcomes = admit(&mut value, &source, &resources, &session, true).await;
        assert_eq!(outcomes.len(), 1);
        assert!(session.state().calls.is_empty() && session.state().failures.is_empty());
        assert!(session.state().budget_stopped.is_none());
    }

    #[tokio::test]
    async fn should_record_why_a_call_was_skipped_when_the_paper_budget_is_spent() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(directory.path().join("spend.jsonl"), 1.0);
        let session = session(&ledger, 0.25);
        session.state().spent_usd = 0.30;
        let value: Option<Value> = session
            .call(
                "extract.pass2",
                &[ContentPart::Text("x".to_owned())],
                CHECK_SCHEMA,
                CHECK_MAX_TOKENS,
            )
            .await;
        assert!(value.is_none());
        let reason = session.state().budget_stopped.clone().expect("reason");
        assert!(reason.starts_with("extract.pass2 skipped"), "{reason}");
        assert!(
            session.state().failures.is_empty(),
            "a budget stop is not a failure"
        );
    }

    #[test]
    fn should_list_candidates_by_page_and_say_when_there_are_none() {
        let resources = resources();
        assert_eq!(
            checklist(&resources.paper.pages),
            "Checklist of equation numbers in the text layer: none found."
        );
        let pages = [crate::domain::ExtractedPage {
            number: 4,
            text: "a = b (3)\nc = d (4)".to_owned(),
        }];
        assert_eq!(
            checklist(&pages),
            "Checklist of equation numbers in the text layer:\n- page 4: (3) (4)"
        );
    }

    #[tokio::test]
    async fn should_start_no_object_chunk_once_the_paper_budget_is_spent() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(directory.path().join("spend.jsonl"), 1.0);
        let resources = resources();
        let pages = resources
            .paper
            .pages
            .iter()
            .flat_map(|page| {
                [
                    ContentPart::Text(page.text.clone()),
                    ContentPart::Png(Vec::new()),
                ]
            })
            .collect::<Vec<_>>();
        let session = session(&ledger, 0.15);
        session.state().spent_usd = 0.15;
        let (inventory, merge, chunks) =
            object_pass(&session, &resources, &pages, &["equation"], 1).await;
        assert!(inventory.is_empty());
        assert_eq!(merge.chunks, 2);
        assert_eq!(merge.failed_chunks, 2);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (
                    chunk.first_page,
                    chunk.last_page,
                    chunk.objects,
                    chunk.retried
                ))
                .collect::<Vec<_>>(),
            [(1, 1, None, true), (2, 2, None, true)]
        );
        let state = session.into_state();
        assert!(state.calls.is_empty());
        assert!(
            state
                .budget_stopped
                .is_some_and(|reason| reason.starts_with("extract.objects skipped"))
        );
    }

    #[test]
    fn should_move_only_the_recovered_chunks_failures() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(directory.path().join("spend.jsonl"), 1.0);
        let session = session(&ledger, 1.0);
        session.fail("extract.objects", Some("p7-12"), "cut off".to_owned(), None);
        session.fail(
            "extract.objects",
            Some("p13-18"),
            "cut off".to_owned(),
            None,
        );
        session.fail("extract.pass1", None, "bad json".to_owned(), None);
        session.recover("p7-12");
        let state = session.into_state();
        let stages = |failures: &[Failure]| {
            failures
                .iter()
                .map(|failure| (failure.stage.clone(), failure.chunk.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            stages(&state.failures),
            [
                ("extract.objects".to_owned(), Some("p13-18".to_owned())),
                ("extract.pass1".to_owned(), None)
            ]
        );
        assert_eq!(
            stages(&state.recovered),
            [("extract.objects".to_owned(), Some("p7-12".to_owned()))]
        );
    }

    #[test]
    fn should_put_headline_results_and_read_signal_under_card() {
        let draft = serde_json::json!({"card": {"thesis": {"text": "t"}}});
        let mut fields = draft.as_object().expect("object").clone();
        merge_fields(
            &mut fields,
            &serde_json::json!({
                "claims": [{"id": "c1"}],
                "headline_results": [{"statement": "s"}],
                "read_signal": {"judgment": "skim"}
            }),
        );
        let merged = serde_json::Value::Object(fields);
        assert_eq!(merged["claims"][0]["id"], "c1");
        assert_eq!(merged["card"]["headline_results"][0]["statement"], "s");
        assert_eq!(merged["card"]["read_signal"]["judgment"], "skim");
        assert!(merged.get("headline_results").is_none());
    }

    #[test]
    fn should_build_one_text_part_per_page_for_the_text_passes() {
        let resources = resources();
        let parts = text_parts(&resources);
        assert_eq!(parts.len(), 2);
        for (part, page) in parts.iter().zip(&resources.paper.pages) {
            match part {
                ContentPart::Text(text) => {
                    assert!(text.starts_with(&format!("<<page {}>>", page.number)));
                }
                ContentPart::Png(_) => panic!("text passes must not send images"),
            }
        }
    }

    #[test]
    fn should_send_only_the_leading_pages_for_the_abstract_shortcut() {
        let resources = resources();
        let text = text_parts(&resources);
        let all = shared_prefix(FIELDS_PROMPT, &text, None);
        let first = shared_prefix(FIELDS_PROMPT, &text, Some(1));
        assert!(all.contains("<<page 2>>"));
        assert!(first.contains("<<page 1>>") && !first.contains("<<page 2>>"));
        assert!(first.starts_with(FIELDS_PROMPT));
        assert!(all.starts_with(FIELDS_PROMPT));
    }

    #[test]
    fn should_name_every_core_field_once() {
        let names = CORE_FIELDS
            .iter()
            .map(|field| field.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "identity",
                "type",
                "abstract",
                "card",
                "question",
                "sections",
                "references",
                "citations",
                "relations"
            ]
        );
        let stages = CORE_FIELDS
            .iter()
            .map(|field| field.stage)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(stages.len(), CORE_FIELDS.len());
    }

    #[test]
    fn should_keep_only_the_requested_object_kinds() {
        let objects = vec![
            serde_json::json!({"kind": "figure", "number": "1"}),
            serde_json::json!({"kind": "equation", "number": "2"}),
            serde_json::json!({"kind": "table", "number": "3"}),
        ];
        let kept = keep_requested_kinds(objects, &["figure", "equation"]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1]["kind"], "equation");
    }

    #[test]
    fn should_state_a_page_budget_in_the_section_guidance() {
        let guidance = sections_guidance(2);
        assert!(guidance.contains("This PDF contains 2 pages"));
        assert!(guidance.contains("1\u{2013}2 substantive reading units"));
        assert!(guidance.starts_with(SECTIONS_POLICY));
    }

    #[test]
    fn should_recognize_the_fillable_units() {
        for name in [
            "identity",
            "type",
            "abstract",
            "card",
            "question",
            "sections",
            "references",
            "citations",
            "relations",
            "objects",
            "supercut",
            "analysis",
            "glossary",
        ] {
            assert!(is_unit(name), "{name}");
        }
        assert!(!is_unit("vibes"));
    }

    #[test]
    fn should_size_a_plan_call_by_the_fields_it_carries() {
        let make = |name: &str, field: &str, max_tokens: Option<u32>| {
            crate::paper_artifacts::config::Call {
                name: name.to_owned(),
                fields: vec![field.to_owned()],
                effort: crate::paper_artifacts::config::Effort::Low,
                model: None,
                max_tokens,
                scope: None,
            }
        };
        assert_eq!(
            plan_max_tokens(&make("core", "sections", None)),
            CORE_MAX_TOKENS
        );
        assert_eq!(
            plan_max_tokens(&make("feature", "analysis", None)),
            FEATURE_MAX_TOKENS
        );
        assert_eq!(
            plan_max_tokens(&make("short", "identity", Some(4_096))),
            4_096
        );
    }

    #[test]
    fn should_union_the_schemas_of_a_call() {
        let call = crate::paper_artifacts::config::Call {
            name: "graph".to_owned(),
            fields: vec!["references".to_owned(), "citations".to_owned()],
            effort: crate::paper_artifacts::config::Effort::Low,
            model: None,
            max_tokens: None,
            scope: None,
        };
        let schema = call_schema(&call);
        assert!(schema["properties"].get("references").is_some());
        assert!(schema["properties"].get("citations").is_some());
        assert!(schema["properties"].get("sections").is_none());
        let required = schema["required"].as_array().expect("required");
        assert!(required.contains(&serde_json::json!("references")));
    }

    #[tokio::test]
    async fn should_scope_and_mark_raw_pages() {
        let resources = resources();
        let input = Input {
            format: InputFormat::Raw,
            images: false,
            priors: false,
        };
        let parts = rendered_parts(&resources, &input, Some(1))
            .await
            .expect("parts");
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            ContentPart::Text(text) => {
                assert!(text.contains("<<page 1>>"));
                assert!(!text.contains("<<page 2>>"));
            }
            ContentPart::Png(_) => panic!("no images were asked for"),
        }
    }

    #[tokio::test]
    async fn should_prepend_front_matter_when_priors_are_on() {
        let mut resources = resources();
        resources.paper.metadata.title = "Sparse Models Under Noise".to_owned();
        let input = Input {
            format: InputFormat::Raw,
            images: false,
            priors: true,
        };
        let parts = rendered_parts(&resources, &input, None)
            .await
            .expect("parts");
        match &parts[0] {
            ContentPart::Text(text) => assert!(text.contains("Sparse Models Under Noise")),
            ContentPart::Png(_) => panic!("front matter is text"),
        }
        assert_eq!(parts.len(), 3, "front matter plus two pages");
    }

    #[tokio::test]
    async fn should_reject_a_pass_count_other_than_one_or_two() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(directory.path().join("spend.jsonl"), 1.0);
        for passes in [0, 3] {
            let options = ExtractOptions {
                pdf: directory.path().join("missing.pdf"),
                model: openrouter::DEFAULT_MODEL.to_owned(),
                passes,
                dpi: resources::DEFAULT_DPI,
                work_directory: directory.path().join("work"),
                output: directory.path().join("out.json"),
                paper_budget_usd: 0.25,
                timeout: Duration::from_secs(1),
                features: Features::default(),
                object_pages: 6,
                reasoning: ReasoningEffort::Low,
                core_mode: CoreMode::Combined,
                providers: Vec::new(),
                plan: None,
            };
            assert!(matches!(
                extract(&options, &ledger).await,
                Err(Error::InvalidRequest(_))
            ));
        }
    }
}
