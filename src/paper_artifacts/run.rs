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
    CHECK_SCHEMA, CORE_SCHEMA, OBJECTS_SCHEMA, PAPER_ARTIFACTS_SCHEMA, REPAIR_SCHEMA,
    admission::{
        CheckCounts, LOCAL_REPAIR_SIMILARITY, QuoteSite, SiteKind, SourceText, Verdict,
        apply_removals, check, quote_sites,
    },
    corrections::{self, CheckResponse, Correction, CorrectionOutcome},
    objects::{self, ChunkObjects, MergeStats},
    resources::{self, PageResources},
};
use crate::{
    Error, Result,
    analysis::{
        openrouter::{self, ChatRequest, ContentPart, CostSource},
        spend::SpendLedger,
    },
    extract::PdfExtractor,
    store::write_atomic,
};

const EXTRACT_PROMPT: &str = include_str!("../../prompts/paper-artifacts-extract.md");
const OBJECTS_INLINE: &str = include_str!("../../prompts/paper-artifacts-objects-inline.md");
const OBJECTS_SEPARATE: &str = include_str!("../../prompts/paper-artifacts-objects-separate.md");
const OBJECTS_PROMPT: &str = include_str!("../../prompts/paper-objects.md");
const CHECK_PROMPT: &str = include_str!("../../prompts/paper-artifacts-check.md");
const REPAIR_PROMPT: &str = include_str!("../../prompts/paper-artifacts-repair.md");

/// Bump when the record shape changes.
const RECORD_VERSION: u16 = 2;

/// Object chunks in flight at once.
const CHUNK_CONCURRENCY: usize = 3;

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
    pub objects: ObjectMode,
}

/// Where the object inventory comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectMode {
    /// Inside the whole-paper pass 1 answer.
    Inline,
    /// A separate pass over page chunks of this size, merged.
    Chunked { pages_per_chunk: usize },
}

impl ObjectMode {
    fn label(self) -> String {
        match self {
            Self::Inline => "inline".to_owned(),
            Self::Chunked { pages_per_chunk } => format!("chunked:{pages_per_chunk}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkRecord {
    pub first_page: u32,
    pub last_page: u32,
    /// Objects returned before merging; `None` when the call failed.
    pub objects: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallRecord {
    pub stage: String,
    pub provider: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    pub cost_source: CostSource,
    pub latency_s: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub stage: String,
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
    pub object_mode: String,
    pub dpi: u32,
    pub pages: usize,
    pub calls: Vec<CallRecord>,
    pub totals: Totals,
    /// Why a later stage did not run, when the paper budget stopped it.
    pub budget_stopped: Option<String>,
    pub failures: Vec<Failure>,
    pub object_chunks: Vec<ChunkRecord>,
    pub object_merge: Option<MergeStats>,
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
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    spent_usd: f64,
    calls: Vec<CallRecord>,
    failures: Vec<Failure>,
    budget_stopped: Option<String>,
}

impl<'a> Session<'a> {
    fn new(
        client: reqwest::Client,
        ledger: &'a SpendLedger,
        model: &'a str,
        paper_budget_usd: f64,
    ) -> Self {
        Self {
            client,
            ledger,
            model,
            paper_budget_usd,
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
    ) -> Option<T> {
        if !self.within_budget(stage) {
            return None;
        }
        let schema: Value = match serde_json::from_str(schema) {
            Ok(schema) => schema,
            Err(error) => {
                self.fail(stage, format!("schema is not JSON: {error}"), None);
                return None;
            }
        };
        let request = ChatRequest {
            model: self.model,
            parts,
            schema: &schema,
            stage,
        };
        let completion = match openrouter::complete(&self.client, self.ledger, &request).await {
            Ok(completion) => completion,
            Err(error) => {
                self.fail(stage, error.to_string(), None);
                return None;
            }
        };
        let parsed = completion.parsed;
        self.record_call(stage, &parsed, completion.latency);
        if parsed.finish_reason.as_deref() == Some("length") {
            self.fail(
                stage,
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
            completion_tokens: parsed.completion_tokens,
            reasoning_tokens: parsed.reasoning_tokens,
            cost_usd: parsed.cost_usd,
            cost_source: parsed.cost_source,
            latency_s: latency.as_secs_f64(),
        });
    }

    fn fail(&self, stage: &str, error: String, raw: Option<&str>) {
        self.state().failures.push(Failure {
            stage: stage.to_owned(),
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
    let pages = page_parts(&resources).await?;
    let source = SourceText::new(&resources.paper.pages);
    let session = Session::new(
        openrouter::client(options.timeout)?,
        ledger,
        &options.model,
        options.paper_budget_usd,
    );

    let (pass1, object_chunks, object_merge) =
        first_pass(&session, options.objects, &resources, &pages).await;
    let pass1_check = pass1.as_ref().map(|draft| check(&source, draft));

    let mut draft = pass1.clone();
    let check_pass = match draft.as_mut() {
        Some(current) if options.passes == 2 => {
            let hide_objects = options.objects != ObjectMode::Inline;
            self_check(current, &pages, &source, &session, hide_objects).await?
        }
        _ => SelfCheck::default(),
    };
    let admission = match draft.as_mut() {
        Some(current) => admit(current, &source, &resources, &session).await,
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
    let totals = Totals {
        calls: session.calls.len(),
        prompt_tokens: session.calls.iter().map(|call| call.prompt_tokens).sum(),
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
        object_mode: options.objects.label(),
        dpi: resources.dpi,
        pages: resources.paper.pages.len(),
        calls: session.calls,
        totals,
        budget_stopped: session.budget_stopped,
        failures: session.failures,
        object_chunks,
        object_merge,
        pass1,
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
    pages: &[ContentPart],
    source: &SourceText<'_>,
    session: &Session<'_>,
    hide_objects: bool,
) -> Result<SelfCheck> {
    // With a chunked inventory the self-check reviews everything else; it
    // cannot reach the objects, so it cannot spend on them.
    let mut view = draft.clone();
    if hide_objects && let Some(fields) = view.as_object_mut() {
        fields.remove("objects");
    }
    let mut parts = vec![
        ContentPart::Text(CHECK_PROMPT.to_owned()),
        ContentPart::Text(format!(
            "Draft:\n```json\n{}\n```",
            serde_json::to_string(&view)?
        )),
    ];
    parts.extend(pages.iter().cloned());
    let Some(response) = session
        .call::<CheckResponse>("extract.pass2", &parts, CHECK_SCHEMA)
        .await
    else {
        return Ok(SelfCheck::default());
    };
    let outcomes = corrections::apply(draft, &response.corrections, source);
    Ok(SelfCheck {
        corrections: response.corrections,
        outcomes,
        counts: Some(check(source, draft)),
    })
}

/// Pass 1, with the object inventory inline or from the chunked pass.
async fn first_pass(
    session: &Session<'_>,
    mode: ObjectMode,
    resources: &PageResources,
    pages: &[ContentPart],
) -> (Option<Value>, Vec<ChunkRecord>, Option<MergeStats>) {
    match mode {
        ObjectMode::Inline => {
            let mut parts = vec![ContentPart::Text(
                EXTRACT_PROMPT.replace("{{objects}}", OBJECTS_INLINE.trim_end()),
            )];
            parts.extend(pages.iter().cloned());
            let pass1 = session
                .call::<Value>("extract.pass1", &parts, PAPER_ARTIFACTS_SCHEMA)
                .await;
            (pass1, Vec::new(), None)
        }
        ObjectMode::Chunked { pages_per_chunk } => {
            let mut parts = vec![ContentPart::Text(
                EXTRACT_PROMPT.replace("{{objects}}", OBJECTS_SEPARATE.trim_end()),
            )];
            parts.extend(pages.iter().cloned());
            let (core, (inventory, merge, chunks)) = tokio::join!(
                session.call::<Value>("extract.pass1", &parts, CORE_SCHEMA),
                object_pass(session, resources, pages, pages_per_chunk),
            );
            let pass1 = core.map(|mut core| {
                if let Some(fields) = core.as_object_mut() {
                    fields.insert("objects".to_owned(), Value::Array(inventory));
                }
                core
            });
            (pass1, chunks, Some(merge))
        }
    }
}

/// The object inventory, read a chunk of pages at a time and merged.
async fn object_pass(
    session: &Session<'_>,
    resources: &PageResources,
    pages: &[ContentPart],
    pages_per_chunk: usize,
) -> (Vec<Value>, MergeStats, Vec<ChunkRecord>) {
    let numbers = resources
        .paper
        .pages
        .iter()
        .map(|page| page.number)
        .collect::<Vec<_>>();
    let chunks = objects::page_chunks(&numbers, pages_per_chunk);
    let mut start = 0;
    let requests = chunks
        .into_iter()
        .map(|chunk| {
            // `pages` holds a text part and an image part per page, in order.
            let mut parts = vec![ContentPart::Text(OBJECTS_PROMPT.to_owned())];
            parts.extend(pages[start * 2..(start + chunk.len()) * 2].iter().cloned());
            start += chunk.len();
            (chunk, parts)
        })
        .collect::<Vec<_>>();
    let results = stream::iter(requests.into_iter().map(|(chunk, parts)| async move {
        let response = session
            .call::<ObjectsResponse>("extract.objects", &parts, OBJECTS_SCHEMA)
            .await;
        ChunkObjects {
            pages: chunk,
            objects: response.map(|response| response.objects),
        }
    }))
    .buffered(CHUNK_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let records = results
        .iter()
        .map(|chunk| ChunkRecord {
            first_page: chunk.pages.first().copied().unwrap_or_default(),
            last_page: chunk.pages.last().copied().unwrap_or_default(),
            objects: chunk.objects.as_ref().map(Vec::len),
        })
        .collect();
    let (inventory, merge) = objects::merge(results);
    (inventory, merge, records)
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
    if !failing.is_empty() {
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
        .call::<RepairResponse>("extract.repair", &parts, REPAIR_SCHEMA)
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
        let outcomes = admit(&mut value, &source, &resources, &session).await;
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
        let outcomes = admit(&mut value, &source, &resources, &session).await;
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
        let (inventory, merge, chunks) = object_pass(&session, &resources, &pages, 1).await;
        assert!(inventory.is_empty());
        assert_eq!(merge.chunks, 2);
        assert_eq!(merge.failed_chunks, 2);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (chunk.first_page, chunk.last_page, chunk.objects))
                .collect::<Vec<_>>(),
            [(1, 1, None), (2, 2, None)]
        );
        let state = session.into_state();
        assert!(state.calls.is_empty());
        assert!(
            state
                .budget_stopped
                .is_some_and(|reason| reason.starts_with("extract.objects skipped"))
        );
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
                objects: ObjectMode::Chunked { pages_per_chunk: 6 },
            };
            assert!(matches!(
                extract(&options, &ledger).await,
                Err(Error::InvalidRequest(_))
            ));
        }
    }
}
