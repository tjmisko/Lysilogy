//! Grounded relationship labels for local citation edges.
//!
//! A model reads the passages in which a paper cites a work and answers with a
//! relation, how much the paper engages with the work, a reason, and a
//! verbatim quote. The answer is only kept when the quote is an exact
//! substring of a passage the model was shown ("grounding"). Raw model output
//! is cached in `source.source_record` so reruns cost nothing, including
//! reruns of answers that failed grounding.
//!
//! Labels come from two paths that share one store: the core reading's
//! admitted relations seed a cheap first label ([`seed`]), and this labeler
//! fills the edges that have citing passages but no label. [`merge_decision`]
//! is the one rule for what happens when a new label meets an existing one.
//! The contract is in `docs/edge-labels.md`.

pub mod seed;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lysilogy_core::{
    AssertionId, Coverage, DocumentVersionId, MentionId, RelationshipOrigin, ReviewState,
    SourceRecordId, WorkId,
};
use lysilogy_db::evidence::{EvidenceRepository, StoredMention};
use lysilogy_db::relationship::{LabelPath, NewAiClaim, RelationshipRepository, StoredClaim};
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::Result;
use crate::bibliography::BibliographicFields;
use crate::concurrency::{BoxFuture, join_all};

/// Bump when the prompt or the request shape changes.
///
/// Old cache entries are then not reused. Version 2 briefs the model on the role the work plays, shows
/// the citing paper's thesis, the cited work's abstract, wider passages and
/// the citation markers, and asks for `engagement`.
pub const LABEL_PROMPT_VERSION: u32 = 2;

/// Source record kind for cached raw model output.
pub const LABEL_RECORD_KIND: &str = "relationship_label";

/// The brief the labeler model reads before the edge's data.
pub const LABEL_BRIEF: &str = include_str!("../../../prompts/edge-label.md");

/// The relation vocabulary the model must choose from.
///
/// Unchanged since version 1: stored labels, the prominence counts (`uses_method`,
/// `uses_data` and `extends` count as method citations, `background` as
/// background) and the core reading's schema all use it. How much the paper
/// does with a work is the separate `engagement`, not a new relation.
pub const RELATIONS: &[&str] = &[
    "background",
    "uses_method",
    "uses_data",
    "extends",
    "supports",
    "contrasts",
    "critiques",
    "compares",
    "other",
];

/// How much the citing paper does with the cited work.
pub const ENGAGEMENTS: &[&str] = &["substantive", "perfunctory", "not_shown"];

/// Strict output schema: every property is required and no others are allowed.
///
/// The properties are in the order the model should decide them:
/// find the evidence, judge engagement, choose the relation, then write the
/// reason.
pub const LABEL_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["passage_index", "quote", "engagement", "relation", "reason"],
  "properties": {
    "passage_index": { "type": "integer" },
    "quote": { "type": "string" },
    "engagement": {
      "type": "string",
      "enum": ["substantive", "perfunctory", "not_shown"]
    },
    "relation": {
      "type": "string",
      "enum": ["background", "uses_method", "uses_data", "extends", "supports", "contrasts", "critiques", "compares", "other"]
    },
    "reason": { "type": "string" }
  }
}"#;

/// Most passages shown to the model for one edge.
const MAX_PASSAGES: usize = 6;

/// Longest accepted reason, in characters.
const MAX_REASON_CHARS: usize = 600;

/// Fewest words a quote must contain, so a bare marker like `[12]` is rejected.
const MIN_QUOTE_WORDS: usize = 3;

/// Longest bibliography entry text shown to the model, in characters.
const MAX_ENTRY_TEXT_CHARS: usize = 400;

/// Longest cited abstract shown to the model, in characters.
const MAX_ABSTRACT_CHARS: usize = 1_200;

/// Longest citing thesis shown to the model, in characters.
const MAX_THESIS_CHARS: usize = 400;

/// Most cited authors shown to the model.
const MAX_AUTHORS: usize = 3;

/// The future returned by a labeler.
pub type LabelFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<LabelDraft, LabelerError>> + Send + 'a>>;

/// A model-backed source of relationship labels.
pub trait RelationshipLabeler: Send + Sync {
    /// Identifies the model in cache keys and stored claims.
    fn model_key(&self) -> &str;

    /// Label one edge from the passages in `request`.
    fn label<'a>(&'a self, request: &'a LabelRequest) -> LabelFuture<'a>;
}

/// What the model is told about the cited work.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CitedWorkBrief {
    pub title: Option<String>,
    pub year: Option<i32>,
    pub authors: Vec<String>,
    pub entry_text: Option<String>,
    /// The authors' abstract, when the library holds and has analysed the work.
    pub abstract_text: Option<String>,
}

/// What the model is told about one passage besides its text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PassageContext {
    /// The citation marker as printed, e.g. `[3, 7-9]` or `(Smith, 2001)`.
    pub marker: Option<String>,
    /// How many works that marker cites at once; 1 when it cites only this work.
    pub cited_together: usize,
    pub page: Option<i32>,
    /// The sentence carrying the citation, when the passage shows text around it.
    pub citing_sentence: Option<String>,
}

/// One edge to label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LabelRequest {
    pub citing_title: Option<String>,
    /// The citing paper's thesis, so the model can see what the argument is.
    pub citing_thesis: Option<String>,
    pub cited: CitedWorkBrief,
    /// Distinct passages, in text order, whitespace-collapsed. A quote must
    /// come from one of these.
    pub passages: Vec<String>,
    /// One entry per passage, in the same order; may be shorter when nothing
    /// is known about the later passages.
    pub contexts: Vec<PassageContext>,
}

/// The model's answer, before grounding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDraft {
    pub relation: String,
    pub reason: String,
    pub quote: String,
    pub passage_index: usize,
    /// `substantive`, `perfunctory` or `not_shown`; absent in version 1 answers.
    #[serde(default)]
    pub engagement: Option<String>,
}

/// Why a labeler produced no draft.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LabelerError {
    /// The model program is not installed or not reachable.
    #[error("labeler unavailable: {0}")]
    Unavailable(String),
    /// The model program ran and failed.
    #[error("labeler failed: {0}")]
    Failed(String),
    /// The model answered with something that is not a label.
    #[error("labeler returned a malformed answer: {0}")]
    Malformed(String),
    /// The label budget is spent; the edge is left for a later run.
    #[error("label budget spent: {0}")]
    Budget(String),
}

/// Where a grounded quote sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundedQuote {
    pub passage_index: usize,
    /// Byte offsets of the quote inside `passages[passage_index]`.
    pub start: usize,
    pub end: usize,
}

/// Why a draft was not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundingRejection {
    EmptyQuote,
    QuoteTooShort,
    QuoteNotInPassage,
    InvalidRelation,
    InvalidEngagement,
    EmptyReason,
    ReasonTooLong,
}

impl GroundingRejection {
    /// The stable code used in summaries and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyQuote => "empty_quote",
            Self::QuoteTooShort => "quote_too_short",
            Self::QuoteNotInPassage => "quote_not_in_passage",
            Self::InvalidRelation => "invalid_relation",
            Self::InvalidEngagement => "invalid_engagement",
            Self::EmptyReason => "empty_reason",
            Self::ReasonTooLong => "reason_too_long",
        }
    }
}

/// How many works one citation marker cites at once.
///
/// Numeric markers count their numbers, with ranges expanded (`[3, 7-9]` is
/// four); author-year markers count their semicolon-separated parts. Anything
/// unreadable counts as one.
#[must_use]
pub fn cited_together(marker: &str) -> usize {
    let inner = marker
        .trim()
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')'])
        .trim();
    let numeric = inner.chars().any(|character| character.is_ascii_digit())
        && inner.chars().all(|character| {
            character.is_ascii_digit() || matches!(character, ',' | ';' | '-' | '\u{2013}' | ' ')
        });
    if numeric {
        let mut count = 0;
        for part in inner.split([',', ';']).map(str::trim) {
            if part.is_empty() {
                continue;
            }
            if let Some((first, last)) = part.split_once(['-', '\u{2013}'])
                && let (Ok(first), Ok(last)) =
                    (first.trim().parse::<usize>(), last.trim().parse::<usize>())
                && last >= first
                && last - first < 200
            {
                count += last - first + 1;
                continue;
            }
            count += 1;
        }
        return count.max(1);
    }
    inner
        .split(';')
        .filter(|part| !part.trim().is_empty())
        .count()
        .max(1)
}

fn clip_chars(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= limit {
        return compact;
    }
    let mut clipped: String = compact.chars().take(limit).collect();
    clipped.push('\u{2026}');
    clipped
}

/// The prompt for one edge: the brief, then the edge's data.
#[must_use]
pub fn label_prompt(request: &LabelRequest) -> String {
    let mut prompt = String::from(LABEL_BRIEF.trim_end());
    prompt.push_str("\n\n---\n\nThe arrow to label.\n\n");
    let unknown = "(unknown title)";
    let cited = &request.cited;
    let _ = writeln!(
        prompt,
        "Citing paper: {}",
        request.citing_title.as_deref().unwrap_or(unknown)
    );
    if let Some(thesis) = request
        .citing_thesis
        .as_deref()
        .filter(|thesis| !thesis.trim().is_empty())
    {
        let _ = writeln!(
            prompt,
            "Its thesis: {}",
            clip_chars(thesis, MAX_THESIS_CHARS)
        );
    }
    let _ = writeln!(prompt, "\nCited work:");
    let _ = writeln!(
        prompt,
        "  Title: {}",
        cited.title.as_deref().unwrap_or(unknown)
    );
    if let Some(year) = cited.year {
        let _ = writeln!(prompt, "  Year: {year}");
    }
    if !cited.authors.is_empty() {
        let _ = writeln!(prompt, "  Authors: {}", cited.authors.join("; "));
    }
    if let Some(entry_text) = &cited.entry_text {
        let _ = writeln!(prompt, "  Bibliography entry: {entry_text}");
    }
    match cited
        .abstract_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        Some(text) => {
            let _ = writeln!(
                prompt,
                "  Abstract: {}",
                clip_chars(text, MAX_ABSTRACT_CHARS)
            );
        }
        None => prompt.push_str("  Abstract: not held by Lysilogy\n"),
    }
    let _ = writeln!(prompt, "\nPassages ({}):", request.passages.len());
    for (index, passage) in request.passages.iter().enumerate() {
        let _ = writeln!(prompt, "[{index}] {passage}");
        let Some(context) = request.contexts.get(index) else {
            continue;
        };
        if let Some(sentence) = &context.citing_sentence {
            let _ = writeln!(prompt, "    Citing sentence: {sentence}");
        }
        if let Some(marker) = &context.marker {
            let together = match context.cited_together {
                0 | 1 => "cites only this work".to_owned(),
                count => format!("cites {count} works at once"),
            };
            let _ = writeln!(prompt, "    Citation marker: {marker} ({together})");
        }
        if let Some(page) = context.page {
            let _ = writeln!(prompt, "    Page: {page}");
        }
    }
    prompt
}

/// SHA-256 (hex) of the canonical request, which keys the label cache.
#[must_use]
pub fn label_cache_key(request: &LabelRequest) -> String {
    cache_key_for_version(request, LABEL_PROMPT_VERSION)
}

fn cache_key_for_version(request: &LabelRequest, version: u32) -> String {
    let canonical = json!({
        "prompt_version": version,
        "request": request,
    });
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

/// Check a draft against the passages it was written from.
///
/// The quote must be an exact substring of `passages[passage_index]` once its
/// outer whitespace is trimmed. When the index is wrong, a unique hit in the
/// other passages is accepted and re-indexed.
///
/// # Errors
/// The first [`GroundingRejection`] that applies.
pub fn ground_quote(
    passages: &[String],
    draft: &LabelDraft,
) -> std::result::Result<GroundedQuote, GroundingRejection> {
    let quote = draft.quote.trim();
    if quote.is_empty() {
        return Err(GroundingRejection::EmptyQuote);
    }
    if !quote_has_enough_words(quote) {
        return Err(GroundingRejection::QuoteTooShort);
    }
    let grounded = locate_quote(passages, draft.passage_index, quote)
        .ok_or(GroundingRejection::QuoteNotInPassage)?;
    check_label_text(&draft.relation, draft.engagement.as_deref(), &draft.reason)?;
    Ok(grounded)
}

/// Whether a quote has at least [`MIN_QUOTE_WORDS`] words with letters.
#[must_use]
pub fn quote_has_enough_words(quote: &str) -> bool {
    quote
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count()
        >= MIN_QUOTE_WORDS
}

/// The relation, engagement and reason checks every label path shares.
///
/// # Errors
/// The first [`GroundingRejection`] that applies.
pub fn check_label_text(
    relation: &str,
    engagement: Option<&str>,
    reason: &str,
) -> std::result::Result<(), GroundingRejection> {
    if !RELATIONS.contains(&relation) {
        return Err(GroundingRejection::InvalidRelation);
    }
    if engagement.is_some_and(|engagement| !ENGAGEMENTS.contains(&engagement)) {
        return Err(GroundingRejection::InvalidEngagement);
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(GroundingRejection::EmptyReason);
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(GroundingRejection::ReasonTooLong);
    }
    Ok(())
}

fn locate_quote(passages: &[String], named: usize, quote: &str) -> Option<GroundedQuote> {
    if let Some(start) = passages.get(named).and_then(|passage| passage.find(quote)) {
        return Some(GroundedQuote {
            passage_index: named,
            start,
            end: start + quote.len(),
        });
    }
    let mut hits = passages
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != named)
        .filter_map(|(index, passage)| passage.find(quote).map(|start| (index, start)));
    let (passage_index, start) = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(GroundedQuote {
        passage_index,
        start,
        end: start + quote.len(),
    })
}

/// A label already on an edge, as the merge rule sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExistingLabel<'a> {
    pub path: LabelPath,
    pub relation: &'a str,
    /// A person accepted, rejected or wrote it.
    pub reviewed: bool,
}

impl<'a> ExistingLabel<'a> {
    /// The merge view of a stored claim.
    #[must_use]
    pub fn of(claim: &'a StoredClaim) -> Self {
        Self {
            path: claim.path(),
            relation: &claim.relation,
            reviewed: claim.origin == RelationshipOrigin::User
                || claim.review_state != ReviewState::Unreviewed,
        }
    }
}

/// What to do with a new label on an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeDecision {
    /// The edge has no current label: write it.
    Insert,
    /// Replace the current label, which is unreviewed and from a path this
    /// one outranks or a staler run of the same path.
    Replace { disagrees: bool },
    /// Keep the current label; `disagrees` when the new one names another relation.
    Keep { disagrees: bool },
}

/// The one rule for combining label paths into one current label per edge.
///
/// - A person's label, or any label a person accepted or rejected, is never
///   replaced by automation.
/// - The core reading's relation is the cheap first label. A newer core
///   reading replaces an older unreviewed core label (same relation or not:
///   the newer run is the current reading of the paper).
/// - The edge labeler read the edge's own passages with a brief written for
///   this question, so its unreviewed label outranks a core label: a core
///   relation never replaces it, and it replaces a core label when it is
///   asked to relabel.
/// - A relabel by the edge labeler (after a prompt change) replaces its own
///   older unreviewed label.
#[must_use]
pub fn merge_decision(
    existing: Option<ExistingLabel<'_>>,
    candidate: LabelPath,
    relation: &str,
) -> MergeDecision {
    let Some(existing) = existing else {
        return MergeDecision::Insert;
    };
    let disagrees = existing.relation != relation;
    if existing.reviewed || existing.path == LabelPath::User {
        return MergeDecision::Keep { disagrees };
    }
    match (existing.path, candidate) {
        (LabelPath::EdgeLabeler, LabelPath::CoreRelation) | (_, LabelPath::User) => {
            MergeDecision::Keep { disagrees }
        }
        _ => MergeDecision::Replace { disagrees },
    }
}

/// How a label pass runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelOptions {
    /// Most model calls; cache hits do not count.
    pub limit: usize,
    /// Model calls in flight at once.
    pub concurrency: usize,
    /// Also label edges whose current label is unreviewed AI output, and
    /// replace it under [`merge_decision`]. Off by default: one label per
    /// edge, and the labeler fills only edges without one.
    pub relabel_unreviewed: bool,
}

impl Default for LabelOptions {
    fn default() -> Self {
        Self {
            limit: 25,
            concurrency: 2,
            relabel_unreviewed: false,
        }
    }
}

/// What one label pass did.
///
/// Every edge it saw lands in exactly one of
/// `already_labeled`, `edges_without_passages`, `over_limit`, `over_budget`,
/// `failed`, `rejected_for_grounding`, `labeled`, `replaced` or `kept`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelSummary {
    pub prompt_version: u32,
    /// Local edges of the paper.
    pub edges_considered: usize,
    /// Edges with neither a stored mention nor a citing sentence from the
    /// core reading: nothing to show the model.
    pub edges_without_passages: usize,
    /// Edges whose only passages came from the core reading's citations.
    pub edges_from_core_citations: usize,
    pub model_calls: usize,
    pub cache_hits: usize,
    /// New labels on edges that had none.
    pub labeled: usize,
    /// Labels that replaced an unreviewed AI label (relabel only).
    pub replaced: usize,
    /// Answers kept out because the edge's label was reviewed meanwhile.
    pub kept: usize,
    pub rejected_for_grounding: usize,
    pub rejections: BTreeMap<String, usize>,
    pub failed: usize,
    /// Edges left unlabeled because the call limit was reached.
    pub over_limit: usize,
    /// Edges left unlabeled because the label budget was spent.
    pub over_budget: usize,
    pub already_labeled: usize,
    /// Engagement of the labels written, by value (`unstated` for none).
    pub engagement: BTreeMap<String, usize>,
    /// Labels written over a core label that named another relation.
    pub disagreements_with_core: usize,
}

/// What the caller knows that the graph does not.
#[derive(Default)]
pub struct LabelBrief {
    /// The citing paper's thesis from its analysis.
    pub citing_thesis: Option<String>,
    /// Abstracts of cited works the library holds, by Work.
    pub cited_abstracts: HashMap<WorkId, String>,
    /// Wider text around a stored mention, shown in place of its passage.
    /// It must contain the stored passage; otherwise the passage is shown.
    pub surround: Option<Box<Surround>>,
    /// Citing sentences for edges without stored mentions, by cited Work:
    /// the core reading's admitted citations.
    pub extra_passages: HashMap<WorkId, Vec<String>>,
}

/// Wider text around a stored mention.
pub type Surround = dyn Fn(&StoredMention) -> Option<String> + Send + Sync;

/// The repositories the label pass works through.
pub struct LabelContext<'a> {
    pub works: &'a WorkRepository,
    pub records: &'a SourceRecordRepository,
    pub evidence: &'a EvidenceRepository,
    pub relationships: &'a RelationshipRepository,
}

/// One edge ready to label.
struct EdgeJob {
    assertion: AssertionId,
    /// The stored mention behind each passage, `None` for a core citation.
    mentions: Vec<Option<MentionId>>,
    request: LabelRequest,
    cache_source_key: String,
    existing: Option<StoredClaim>,
}

/// A raw draft ready to ground.
struct Drafted {
    job: EdgeJob,
    record: SourceRecordId,
    draft: LabelDraft,
}

/// Label the local edges of `citing_work` that have no current claim (or,
/// with [`LabelOptions::relabel_unreviewed`], an unreviewed AI claim).
///
/// Edges are taken by mention count (most first), then by text position.
/// Labeler errors count as `failed` (or `over_budget`) and never stop the pass.
///
/// # Errors
/// Database failures.
#[allow(clippy::too_many_lines)] // One pass: select edges, call, then record, with every outcome counted.
pub async fn label_edges(
    ctx: &LabelContext<'_>,
    citing_work: WorkId,
    document_version: DocumentVersionId,
    labeler: &dyn RelationshipLabeler,
    brief: &LabelBrief,
    options: LabelOptions,
) -> Result<LabelSummary> {
    let mut summary = LabelSummary {
        prompt_version: LABEL_PROMPT_VERSION,
        ..LabelSummary::default()
    };
    let citing_title = ctx
        .works
        .get(citing_work)
        .await?
        .and_then(|work| work.display_title);
    let mut edges = ctx
        .evidence
        .local_edges(citing_work, document_version)
        .await?;
    summary.edges_considered = edges.len();
    edges.sort_by_key(|(_, _, mentions)| {
        (
            std::cmp::Reverse(mentions.len()),
            mentions.first().and_then(|mention| mention.marker_start),
        )
    });
    let assertions: Vec<AssertionId> = edges.iter().map(|(_, assertion, _)| *assertion).collect();
    let mut current = ctx
        .relationships
        .current_claims_by_assertion(&assertions)
        .await?;

    let model_key = labeler.model_key();
    let mut cached = Vec::new();
    let mut pending = Vec::new();
    for (cited, assertion, mentions) in edges {
        let existing = current.remove(&assertion);
        if let Some(claim) = &existing {
            let relabel = options.relabel_unreviewed && !ExistingLabel::of(claim).reviewed;
            if !relabel {
                summary.already_labeled += 1;
                continue;
            }
        }
        let extra = brief
            .extra_passages
            .get(&cited)
            .map_or(&[][..], Vec::as_slice);
        if mentions
            .iter()
            .all(|mention| mention.passage.trim().is_empty())
            && extra.is_empty()
        {
            summary.edges_without_passages += 1;
            continue;
        }
        if mentions.is_empty() {
            summary.edges_from_core_citations += 1;
        }
        let (request, mention_ids) =
            build_request(ctx, brief, citing_title.as_deref(), cited, &mentions, extra).await?;
        let cache_source_key = format!("{model_key}|{}", label_cache_key(&request));
        let job = EdgeJob {
            assertion,
            mentions: mention_ids,
            request,
            cache_source_key,
            existing,
        };
        let hit = ctx
            .records
            .find(LABEL_RECORD_KIND, &job.cache_source_key)
            .await?;
        let cached_draft = hit.and_then(|(record, payload)| {
            serde_json::from_value::<LabelDraft>(payload)
                .ok()
                .map(|draft| (record, draft))
        });
        if let Some((record, draft)) = cached_draft {
            summary.cache_hits += 1;
            cached.push(Drafted { job, record, draft });
            continue;
        }
        if pending.len() >= options.limit {
            summary.over_limit += 1;
            continue;
        }
        pending.push(job);
    }

    let mut drafted = cached;
    summary.model_calls = pending.len();
    let semaphore = Arc::new(Semaphore::new(options.concurrency.max(1)));
    let calls = pending
        .iter()
        .map(|job| {
            let future: BoxFuture<'_, Result<CallOutcome>> =
                Box::pin(run_call(ctx, labeler, &semaphore, job));
            future
        })
        .collect();
    let outcomes = join_all(calls).await;
    for (job, outcome) in pending.into_iter().zip(outcomes) {
        match outcome? {
            CallOutcome::Drafted(record, draft) => drafted.push(Drafted { job, record, draft }),
            CallOutcome::Failed(LabelerError::Budget(reason)) => {
                tracing::info!(%reason, "relationship label left for a later run");
                summary.over_budget += 1;
            }
            CallOutcome::Failed(error) => {
                tracing::warn!(%error, "relationship labeler failed");
                summary.failed += 1;
            }
        }
    }

    for item in drafted {
        record_draft(ctx, model_key, item, &mut summary).await?;
    }
    Ok(summary)
}

enum CallOutcome {
    Drafted(SourceRecordId, LabelDraft),
    Failed(LabelerError),
}

/// Call the labeler under the semaphore and cache its raw draft immediately,
/// so a crash after the call does not lose a paid answer.
async fn run_call(
    ctx: &LabelContext<'_>,
    labeler: &dyn RelationshipLabeler,
    semaphore: &Semaphore,
    job: &EdgeJob,
) -> Result<CallOutcome> {
    let Ok(_permit) = semaphore.acquire().await else {
        return Ok(CallOutcome::Failed(LabelerError::Failed(
            "label semaphore closed".to_owned(),
        )));
    };
    let draft = match labeler.label(&job.request).await {
        Ok(draft) => draft,
        Err(error) => return Ok(CallOutcome::Failed(error)),
    };
    let payload = serde_json::to_value(&draft).unwrap_or(Value::Null);
    let record = ctx
        .records
        .upsert(
            LABEL_RECORD_KIND,
            &job.cache_source_key,
            Coverage::Complete,
            &payload,
        )
        .await?;
    Ok(CallOutcome::Drafted(record, draft))
}

async fn record_draft(
    ctx: &LabelContext<'_>,
    model_key: &str,
    item: Drafted,
    summary: &mut LabelSummary,
) -> Result<()> {
    let Drafted { job, record, draft } = item;
    let grounded = match ground_quote(&job.request.passages, &draft) {
        Ok(grounded) => grounded,
        Err(rejection) => {
            summary.rejected_for_grounding += 1;
            *summary
                .rejections
                .entry(rejection.as_str().to_owned())
                .or_default() += 1;
            return Ok(());
        }
    };
    let mention = job.mentions.get(grounded.passage_index).copied().flatten();
    let quote = draft.quote.trim();
    let claim = NewAiClaim {
        assertion: job.assertion,
        mention,
        relation: &draft.relation,
        reason: draft.reason.trim(),
        quote,
        model: model_key,
        record,
        path: LabelPath::EdgeLabeler,
        engagement: draft.engagement.as_deref(),
        prompt_version: i32::try_from(LABEL_PROMPT_VERSION).ok(),
    };
    let engagement = draft
        .engagement
        .clone()
        .unwrap_or_else(|| "unstated".to_owned());
    let decision = merge_decision(
        job.existing.as_ref().map(ExistingLabel::of),
        LabelPath::EdgeLabeler,
        &draft.relation,
    );
    let written = match (decision, &job.existing) {
        (MergeDecision::Replace { disagrees }, Some(existing)) => {
            match ctx
                .relationships
                .replace_ai_claim(existing.id, &claim)
                .await
            {
                Ok(Some(_)) => {
                    summary.replaced += 1;
                    if disagrees && existing.path() == LabelPath::CoreRelation {
                        summary.disagreements_with_core += 1;
                    }
                    true
                }
                Ok(None) => false,
                Err(lysilogy_db::DbError::Conflict(_)) => {
                    summary.kept += 1;
                    false
                }
                Err(error) => return Err(error.into()),
            }
        }
        (MergeDecision::Keep { .. }, _) => {
            summary.kept += 1;
            false
        }
        _ => {
            let inserted = ctx.relationships.insert_ai_claim(&claim).await?;
            if inserted.is_some() {
                summary.labeled += 1;
            }
            inserted.is_some()
        }
    };
    if written {
        *summary.engagement.entry(engagement).or_default() += 1;
    }
    Ok(())
}

/// The model-facing description of one edge, and the stored mention behind
/// each passage.
async fn build_request(
    ctx: &LabelContext<'_>,
    brief: &LabelBrief,
    citing_title: Option<&str>,
    cited: WorkId,
    mentions: &[StoredMention],
    extra: &[String],
) -> Result<(LabelRequest, Vec<Option<MentionId>>)> {
    let work = ctx.works.get(cited).await?;
    let entry = match mentions.iter().find_map(|mention| mention.entry) {
        Some(entry_id) => ctx.evidence.entry(entry_id).await?,
        None => None,
    };
    let fields = entry
        .as_ref()
        .and_then(|entry| serde_json::from_value::<BibliographicFields>(entry.fields.clone()).ok());
    let title = work
        .as_ref()
        .and_then(|work| work.display_title.clone())
        .or_else(|| {
            fields
                .as_ref()
                .and_then(|fields| fields.title.value.clone())
        });
    let year = work
        .as_ref()
        .and_then(|work| work.display_year)
        .or_else(|| {
            let printed = fields.as_ref()?.year.value.as_deref()?;
            let digits: String = printed.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        });
    let authors = fields
        .as_ref()
        .and_then(|fields| fields.authors.value.clone())
        .or_else(|| work.as_ref().and_then(|work| work.display_authors.clone()))
        .unwrap_or_default()
        .into_iter()
        .take(MAX_AUTHORS)
        .collect();
    let entry_text = entry
        .as_ref()
        .map(|entry| entry.raw_text.chars().take(MAX_ENTRY_TEXT_CHARS).collect());
    let (passages, contexts, mention_ids) = edge_passages(brief, mentions, extra);
    Ok((
        LabelRequest {
            citing_title: citing_title.map(str::to_owned),
            citing_thesis: brief.citing_thesis.clone(),
            cited: CitedWorkBrief {
                title,
                year,
                authors,
                entry_text,
                abstract_text: brief.cited_abstracts.get(&cited).cloned(),
            },
            passages,
            contexts,
        },
        mention_ids,
    ))
}

/// Distinct passages in text order, at most [`MAX_PASSAGES`]: each stored
/// mention's passage (widened by [`LabelBrief::surround`] when it can be),
/// then the core reading's citing sentences that no passage already holds.
fn edge_passages(
    brief: &LabelBrief,
    mentions: &[StoredMention],
    extra: &[String],
) -> (Vec<String>, Vec<PassageContext>, Vec<Option<MentionId>>) {
    let mut passages: Vec<String> = Vec::new();
    let mut contexts = Vec::new();
    let mut ids = Vec::new();
    for mention in mentions {
        if passages.len() == MAX_PASSAGES {
            break;
        }
        let stored = mention.passage.trim();
        if stored.is_empty() || passages.iter().any(|passage| passage.contains(stored)) {
            continue;
        }
        let wide = brief
            .surround
            .as_ref()
            .and_then(|surround| surround(mention))
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| text.contains(stored) && text.len() > stored.len());
        let citing_sentence = wide.as_ref().map(|_| stored.to_owned());
        passages.push(wide.unwrap_or_else(|| stored.to_owned()));
        contexts.push(PassageContext {
            marker: mention.marker_text.clone(),
            cited_together: mention.marker_text.as_deref().map_or(1, cited_together),
            page: mention.page,
            citing_sentence,
        });
        ids.push(Some(mention.id));
    }
    for sentence in extra {
        if passages.len() == MAX_PASSAGES {
            break;
        }
        let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
        if sentence.is_empty() || passages.iter().any(|passage| passage.contains(&sentence)) {
            continue;
        }
        passages.push(sentence);
        contexts.push(PassageContext::default());
        ids.push(None);
    }
    (passages, contexts, ids)
}

type ScriptedFn =
    dyn Fn(&LabelRequest) -> std::result::Result<LabelDraft, LabelerError> + Send + Sync;

/// A deterministic labeler for tests: answers from a closure and counts calls.
pub struct ScriptedLabeler {
    model_key: String,
    script: Box<ScriptedFn>,
    calls: AtomicUsize,
}

impl ScriptedLabeler {
    /// A labeler that answers every request with `script`.
    #[must_use]
    pub fn new(
        model_key: &str,
        script: impl Fn(&LabelRequest) -> std::result::Result<LabelDraft, LabelerError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            model_key: model_key.to_owned(),
            script: Box::new(script),
            calls: AtomicUsize::new(0),
        }
    }

    /// How many times `label` was called.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RelationshipLabeler for ScriptedLabeler {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn label<'a>(&'a self, request: &'a LabelRequest) -> LabelFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = (self.script)(request);
        Box::pin(async move { answer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lysilogy_core::{CitationStyle, Visibility, WorkKind};
    use lysilogy_db::citation::CitationRepository;
    use lysilogy_db::evidence::{NewEntry, NewMention};
    use sqlx::PgPool;
    use std::sync::atomic::AtomicU64;

    fn passages(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| (*text).to_owned()).collect()
    }

    fn draft(quote: &str, passage_index: usize) -> LabelDraft {
        LabelDraft {
            relation: "extends".to_owned(),
            reason: "It builds on the cited method.".to_owned(),
            quote: quote.to_owned(),
            passage_index,
            engagement: Some("substantive".to_owned()),
        }
    }

    fn sample_request() -> LabelRequest {
        LabelRequest {
            citing_title: Some("A citing paper".to_owned()),
            cited: CitedWorkBrief {
                title: Some("The cited work".to_owned()),
                year: Some(2019),
                authors: vec!["Ada Adams".to_owned()],
                entry_text: Some("[1] Adams. The cited work. 2019.".to_owned()),
                abstract_text: None,
            },
            passages: passages(&[
                "We extend the cited method [1].",
                "See also [1] for details.",
            ]),
            citing_thesis: None,
            contexts: vec![],
        }
    }

    #[test]
    fn should_accept_quote_when_exact_substring_of_named_passage() {
        let texts = passages(&[
            "First sentence here.",
            "We extend the cited method [1] to new tasks.",
        ]);
        let grounded = ground_quote(&texts, &draft("extend the cited method", 1)).unwrap();
        assert_eq!(grounded.passage_index, 1);
        assert_eq!(
            &texts[1][grounded.start..grounded.end],
            "extend the cited method"
        );
        let padded = ground_quote(&texts, &draft("  extend the cited method\n", 1)).unwrap();
        assert_eq!(padded, grounded, "outer whitespace is trimmed");
    }

    #[test]
    fn should_reindex_quote_when_found_uniquely_in_another_passage() {
        let texts = passages(&[
            "Nothing relevant lives here.",
            "We extend the cited method [1] to new tasks.",
        ]);
        let grounded = ground_quote(&texts, &draft("extend the cited method", 0)).unwrap();
        assert_eq!(grounded.passage_index, 1);
        let out_of_range = ground_quote(&texts, &draft("extend the cited method", 9)).unwrap();
        assert_eq!(out_of_range.passage_index, 1);

        let twice = passages(&[
            "We extend the cited method here.",
            "Again we extend the cited method there.",
        ]);
        assert_eq!(
            ground_quote(&twice, &draft("extend the cited method", 5)),
            Err(GroundingRejection::QuoteNotInPassage),
            "an ambiguous hit elsewhere is not guessed"
        );
        assert!(ground_quote(&twice, &draft("extend the cited method", 1)).is_ok());
    }

    #[test]
    fn should_reject_quote_when_whitespace_or_case_differs() {
        let texts = passages(&["We extend the cited method [1] to new tasks."]);
        for quote in [
            "Extend the cited method",
            "extend  the cited method",
            "extend the\ncited method",
            "extend the cited method to old tasks",
        ] {
            assert_eq!(
                ground_quote(&texts, &draft(quote, 0)),
                Err(GroundingRejection::QuoteNotInPassage),
                "{quote:?}"
            );
        }
    }

    #[test]
    fn should_reject_quote_when_only_a_citation_marker() {
        let texts = passages(&["We extend the cited method [1] to new tasks."]);
        assert_eq!(
            ground_quote(&texts, &draft("[1]", 0)),
            Err(GroundingRejection::QuoteTooShort)
        );
        assert_eq!(
            ground_quote(&texts, &draft("method [1]", 0)),
            Err(GroundingRejection::QuoteTooShort)
        );
        assert_eq!(
            ground_quote(&texts, &draft("   ", 0)),
            Err(GroundingRejection::EmptyQuote)
        );
        assert_eq!(GroundingRejection::EmptyQuote.as_str(), "empty_quote");
        assert_eq!(
            GroundingRejection::QuoteTooShort.as_str(),
            "quote_too_short"
        );
    }

    #[test]
    fn should_reject_label_when_relation_not_in_vocabulary() {
        let texts = passages(&["We extend the cited method [1] to new tasks."]);
        for relation in ["Extends", "", "refutes", "extends "] {
            let mut bad = draft("extend the cited method", 0);
            bad.relation = relation.to_owned();
            assert_eq!(
                ground_quote(&texts, &bad),
                Err(GroundingRejection::InvalidRelation),
                "{relation:?}"
            );
        }
        for relation in RELATIONS {
            let mut good = draft("extend the cited method", 0);
            good.relation = (*relation).to_owned();
            assert!(ground_quote(&texts, &good).is_ok(), "{relation}");
        }
    }

    #[test]
    fn should_reject_label_when_reason_empty_or_too_long() {
        let texts = passages(&["We extend the cited method [1] to new tasks."]);
        let mut label = draft("extend the cited method", 0);
        label.reason = "  \n".to_owned();
        assert_eq!(
            ground_quote(&texts, &label),
            Err(GroundingRejection::EmptyReason)
        );
        label.reason = "r".repeat(MAX_REASON_CHARS + 1);
        assert_eq!(
            ground_quote(&texts, &label),
            Err(GroundingRejection::ReasonTooLong)
        );
        label.reason = "r".repeat(MAX_REASON_CHARS);
        assert!(ground_quote(&texts, &label).is_ok());
        label.reason = "\u{e9}".repeat(MAX_REASON_CHARS);
        assert!(
            ground_quote(&texts, &label).is_ok(),
            "limit counts characters, not bytes"
        );
    }

    #[test]
    fn should_reject_quote_when_passage_index_out_of_range_and_not_found() {
        let texts = passages(&["We extend the cited method [1] to new tasks."]);
        assert_eq!(
            ground_quote(&texts, &draft("an invented sentence entirely", 4)),
            Err(GroundingRejection::QuoteNotInPassage)
        );
        assert_eq!(
            ground_quote(&[], &draft("an invented sentence entirely", 0)),
            Err(GroundingRejection::QuoteNotInPassage)
        );
    }

    #[test]
    fn should_ground_quote_when_passage_has_multibyte_text() {
        let texts = passages(&["Caf\u{e9} models \u{2014} as in [1] \u{2014} extend prior work."]);
        let grounded = ground_quote(&texts, &draft("as in [1] \u{2014} extend prior", 0)).unwrap();
        assert_eq!(
            &texts[0][grounded.start..grounded.end],
            "as in [1] \u{2014} extend prior"
        );
    }

    #[test]
    fn should_keep_cache_key_stable_when_request_is_identical() {
        let first = label_cache_key(&sample_request());
        assert_eq!(first, label_cache_key(&sample_request()));
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn should_change_cache_key_when_passages_change() {
        let base = label_cache_key(&sample_request());
        let mut changed = sample_request();
        changed
            .passages
            .push("A third passage cites [1].".to_owned());
        assert_ne!(base, label_cache_key(&changed));
        let mut reordered = sample_request();
        reordered.passages.reverse();
        assert_ne!(base, label_cache_key(&reordered));
        let mut retitled = sample_request();
        retitled.cited.title = Some("Another title".to_owned());
        assert_ne!(base, label_cache_key(&retitled));
    }

    #[test]
    fn should_number_passages_when_building_prompt() {
        let prompt = label_prompt(&sample_request());
        assert!(
            prompt.contains("[0] We extend the cited method [1]."),
            "{prompt}"
        );
        assert!(prompt.contains("[1] See also [1] for details."), "{prompt}");
        assert!(prompt.contains("Title: The cited work"), "{prompt}");
        assert!(prompt.contains("Year: 2019"), "{prompt}");
        assert!(prompt.contains("character for character"), "{prompt}");
        for relation in RELATIONS {
            assert!(prompt.contains(relation), "{relation}");
        }
        assert_eq!(
            prompt,
            label_prompt(&sample_request()),
            "prompt is deterministic"
        );
    }

    #[test]
    fn should_parse_schema_when_embedded() {
        let schema: Value = serde_json::from_str(LABEL_SCHEMA).expect("valid JSON");
        assert_eq!(schema["additionalProperties"], false);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(required.len(), properties.len());
        for name in &required {
            assert!(properties.contains_key(*name), "{name}");
        }
        let relations: Vec<&str> = properties["relation"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        assert_eq!(relations, RELATIONS);
        let parsed: LabelDraft = serde_json::from_value(json!({
            "relation": "other", "reason": "r", "quote": "q", "passage_index": 0
        }))
        .unwrap();
        assert_eq!(parsed.passage_index, 0);
    }

    #[test]
    fn should_dedupe_and_cap_passages_when_building_request() {
        let mention = |passage: &str| StoredMention {
            id: lysilogy_core::MentionId::new(),
            assertion: AssertionId::new(),
            entry: None,
            style: None,
            marker_text: None,
            page: None,
            marker_start: None,
            marker_end: None,
            passage_start: None,
            passage_end: None,
            passage: passage.to_owned(),
            rects: None,
        };
        let mentions: Vec<StoredMention> = ["a", "a", " ", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|passage| mention(passage))
            .collect();
        let (texts, contexts, ids) = edge_passages(&LabelBrief::default(), &mentions, &[]);
        assert_eq!(texts, vec!["a", "b", "c", "d", "e", "f"]);
        assert_eq!((contexts.len(), ids.len()), (6, 6));
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct Harness {
        pool: PgPool,
        works: WorkRepository,
        records: SourceRecordRepository,
        evidence: EvidenceRepository,
        relationships: RelationshipRepository,
        citations: CitationRepository,
        marker: String,
        citing: WorkId,
        version: DocumentVersionId,
        assertions: Vec<AssertionId>,
        cited: Vec<WorkId>,
    }

    impl Harness {
        async fn new(edges: usize) -> Option<Self> {
            let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
                .ok()
                .filter(|url| !url.is_empty())?;
            let pool = lysilogy_db::connect(&url).await.expect("connect");
            lysilogy_db::migrate(&pool).await.expect("migrate");
            let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
            let marker = format!(
                "lbl{}x{sequence}x{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let works = WorkRepository::new(pool.clone());
            let citing = works
                .create(WorkKind::Article, Visibility::Private)
                .await
                .expect("citing")
                .id;
            let workspace: uuid::Uuid = sqlx::query_scalar(
                "insert into reader.workspace (name) values ('labels-test') returning id",
            )
            .fetch_one(&pool)
            .await
            .expect("workspace");
            let document: uuid::Uuid = sqlx::query_scalar(
                "insert into reader.document (workspace_id) values ($1) returning id",
            )
            .bind(workspace)
            .fetch_one(&pool)
            .await
            .expect("document");
            let version: uuid::Uuid = sqlx::query_scalar(
                "insert into reader.document_version (document_id, sha256) values ($1, $2) returning id",
            )
            .bind(document)
            .bind(format!("sha-{marker}"))
            .fetch_one(&pool)
            .await
            .expect("version");
            let mut harness = Self {
                works,
                records: SourceRecordRepository::new(pool.clone()),
                evidence: EvidenceRepository::new(pool.clone()),
                relationships: RelationshipRepository::new(pool.clone()),
                citations: CitationRepository::new(pool.clone()),
                pool,
                marker,
                citing,
                version: DocumentVersionId::from_uuid(version),
                assertions: Vec::new(),
                cited: Vec::new(),
            };
            harness.add_edges(edges).await;
            Some(harness)
        }

        async fn add_edges(&mut self, count: usize) {
            let record = self
                .records
                .upsert(
                    "local_bibliography",
                    &format!("bib-{}", self.marker),
                    Coverage::Complete,
                    &json!({}),
                )
                .await
                .expect("bibliography record");
            let entries: Vec<NewEntry> = (0..count)
                .map(|index| NewEntry {
                    entry_key: format!("bib-v1:{}", index + 1),
                    ordinal: i32::try_from(index).unwrap_or(0),
                    raw_text: format!("[{}] Adams. Cited work number {index}. 2019.", index + 1),
                    printed_key: Some(format!("{}", index + 1)),
                    fields: json!({"title": {"value": format!("Cited work number {index}"), "confidence": "high"}}),
                    page: 1,
                    text_start: 0,
                    text_end: 10,
                    member_spans: json!([]),
                    region: None,
                })
                .collect();
            let entry_ids = self
                .evidence
                .insert_entries(self.version, record, &entries)
                .await
                .expect("entries");
            for (index, entry) in entry_ids.into_iter().enumerate() {
                let cited = self
                    .works
                    .create(WorkKind::Article, Visibility::Private)
                    .await
                    .expect("cited")
                    .id;
                let assertion = self
                    .citations
                    .add_entry_assertion(self.citing, cited, entry)
                    .await
                    .expect("assertion");
                self.evidence
                    .record_resolution(
                        entry,
                        cited,
                        lysilogy_core::ResolutionStatus::Accepted,
                        lysilogy_core::ResolutionMethod::Identifier,
                        Some(1.0),
                    )
                    .await
                    .expect("resolution");
                self.cited.push(cited);
                let start = i32::try_from(100 * (index + 1)).unwrap_or(0);
                self.evidence
                    .insert_mention(&NewMention {
                        assertion,
                        version: self.version,
                        entry,
                        style: CitationStyle::Numeric,
                        marker_text: "[1]".to_owned(),
                        page: 1,
                        marker_start: start,
                        marker_end: start + 3,
                        passage_start: start - 50,
                        passage_end: start + 50,
                        passage: format!(
                            "Edge {index} extends the reward modelling approach [1] in {}.",
                            self.marker
                        ),
                        rects: None,
                    })
                    .await
                    .expect("mention")
                    .expect("new mention");
                self.assertions.push(assertion);
            }
        }

        fn context(&self) -> LabelContext<'_> {
            LabelContext {
                works: &self.works,
                records: &self.records,
                evidence: &self.evidence,
                relationships: &self.relationships,
            }
        }

        async fn run(&self, labeler: &ScriptedLabeler, limit: usize) -> LabelSummary {
            self.run_with(labeler, &LabelBrief::default(), limit, false)
                .await
        }

        async fn run_with(
            &self,
            labeler: &ScriptedLabeler,
            brief: &LabelBrief,
            limit: usize,
            relabel_unreviewed: bool,
        ) -> LabelSummary {
            label_edges(
                &self.context(),
                self.citing,
                self.version,
                labeler,
                brief,
                LabelOptions {
                    limit,
                    concurrency: 2,
                    relabel_unreviewed,
                },
            )
            .await
            .expect("label edges")
        }

        async fn seed(&self, reading: &seed::CoreReading) -> seed::CoreSeedOutcome {
            seed::seed_core_labels(&self.context(), self.citing, self.version, reading, 0)
                .await
                .expect("seed")
        }

        async fn current(&self) -> BTreeMap<AssertionId, StoredClaim> {
            self.relationships
                .current_claims_by_assertion(&self.assertions)
                .await
                .expect("current")
        }

        async fn claim_count(&self) -> i64 {
            sqlx::query_scalar(
                "select count(*) from graph.relationship_claim where assertion_id = any($1)",
            )
            .bind(
                self.assertions
                    .iter()
                    .map(|id| id.as_uuid())
                    .collect::<Vec<_>>(),
            )
            .fetch_one(&self.pool)
            .await
            .expect("count")
        }
    }

    fn quoting_labeler() -> ScriptedLabeler {
        ScriptedLabeler::new("fake:model", |request| {
            Ok(LabelDraft {
                relation: "extends".to_owned(),
                reason: "The passage extends the approach.".to_owned(),
                quote: "extends the reward modelling approach".to_owned(),
                passage_index: request.passages.len().saturating_sub(1),
                engagement: Some("substantive".to_owned()),
            })
        })
    }

    #[tokio::test]
    async fn should_write_unreviewed_ai_claim_with_quote_and_mention_when_label_grounded() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(1).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let labeler = quoting_labeler();
        let summary = harness.run(&labeler, 25).await;
        assert_eq!(summary.edges_considered, 1);
        assert_eq!(
            (summary.model_calls, summary.labeled, summary.failed),
            (1, 1, 0)
        );

        let claims = harness
            .relationships
            .current_claims_for_edges(&[(
                harness.citing,
                harness
                    .evidence
                    .local_edges(harness.citing, harness.version)
                    .await
                    .unwrap()[0]
                    .0,
            )])
            .await
            .unwrap();
        assert_eq!(claims.len(), 1);
        let claim = &claims[0];
        assert_eq!(claim.claim.relation, "extends");
        assert_eq!(
            claim.claim.quote.as_deref(),
            Some("extends the reward modelling approach")
        );
        assert_eq!(claim.claim.model.as_deref(), Some("fake:model"));
        assert_eq!(
            claim.claim.review_state,
            lysilogy_core::ReviewState::Unreviewed
        );
        assert_eq!(claim.claim.origin, lysilogy_core::RelationshipOrigin::Ai);
        assert!(claim.claim.mention.is_some());
        assert!(claim.claim.source_record.is_some());
        assert!(
            claim
                .passage
                .as_deref()
                .unwrap()
                .contains("extends the reward modelling approach")
        );
    }

    #[tokio::test]
    async fn should_not_call_labeler_when_cached_output_exists() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(2).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let labeler = quoting_labeler();
        let first = harness.run(&labeler, 25).await;
        assert_eq!((first.model_calls, first.labeled), (2, 2));
        assert_eq!(labeler.calls(), 2);

        let again = harness.run(&labeler, 25).await;
        assert_eq!(
            (again.model_calls, again.already_labeled, again.labeled),
            (0, 2, 0)
        );

        sqlx::query("delete from graph.relationship_claim where assertion_id = any($1)")
            .bind(
                harness
                    .assertions
                    .iter()
                    .map(|id| id.as_uuid())
                    .collect::<Vec<_>>(),
            )
            .execute(&harness.pool)
            .await
            .unwrap();
        let rebuilt = harness.run(&labeler, 25).await;
        assert_eq!(labeler.calls(), 2, "cached drafts are reused");
        assert_eq!(
            (rebuilt.model_calls, rebuilt.cache_hits, rebuilt.labeled),
            (0, 2, 2)
        );
        assert_eq!(harness.claim_count().await, 2);

        let third = harness.run(&labeler, 25).await;
        assert_eq!(
            (third.model_calls, third.cache_hits, third.labeled),
            (0, 0, 0)
        );
        assert_eq!(harness.claim_count().await, 2, "no duplicate claims");
    }

    #[tokio::test]
    async fn should_count_rejection_and_write_no_claim_when_quote_not_in_passage() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(1).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let labeler = ScriptedLabeler::new("fake:model", |_| {
            Ok(LabelDraft {
                relation: "supports".to_owned(),
                reason: "Because.".to_owned(),
                quote: "a sentence the model invented".to_owned(),
                passage_index: 0,
                engagement: Some("substantive".to_owned()),
            })
        });
        let summary = harness.run(&labeler, 25).await;
        assert_eq!((summary.model_calls, summary.labeled), (1, 0));
        assert_eq!(summary.rejected_for_grounding, 1);
        assert_eq!(summary.rejections.get("quote_not_in_passage"), Some(&1));
        assert_eq!(harness.claim_count().await, 0);

        let again = harness.run(&labeler, 25).await;
        assert_eq!(labeler.calls(), 1, "a rejected answer stays cached");
        assert_eq!((again.model_calls, again.cache_hits), (0, 1));
        assert_eq!(again.rejections.get("quote_not_in_passage"), Some(&1));
    }

    #[tokio::test]
    async fn should_continue_when_labeler_fails_for_one_edge() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(3).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let labeler = ScriptedLabeler::new("fake:model", |request| {
            if request.passages[0].starts_with("Edge 1 ") {
                return Err(LabelerError::Failed("boom".to_owned()));
            }
            if request.passages[0].starts_with("Edge 2 ") {
                return Err(LabelerError::Malformed("not json".to_owned()));
            }
            Ok(LabelDraft {
                relation: "extends".to_owned(),
                reason: "Fine.".to_owned(),
                quote: "extends the reward modelling approach".to_owned(),
                passage_index: 0,
                engagement: Some("substantive".to_owned()),
            })
        });
        let summary = harness.run(&labeler, 25).await;
        assert_eq!(summary.model_calls, 3);
        assert_eq!((summary.labeled, summary.failed), (1, 2));
        assert_eq!(harness.claim_count().await, 1);

        let again = harness.run(&labeler, 25).await;
        assert_eq!(
            again.model_calls, 2,
            "failures are not cached, so they are retried"
        );
        assert_eq!(again.already_labeled, 1);
    }

    #[tokio::test]
    async fn should_stop_calling_when_label_limit_reached() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(4).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let labeler = quoting_labeler();
        let summary = harness.run(&labeler, 2).await;
        assert_eq!(summary.edges_considered, 4);
        assert_eq!(
            (summary.model_calls, summary.labeled, summary.over_limit),
            (2, 2, 2)
        );
        assert_eq!(labeler.calls(), 2);

        let rest = harness.run(&labeler, 2).await;
        assert_eq!(
            (rest.model_calls, rest.labeled, rest.already_labeled),
            (2, 2, 2)
        );
        assert_eq!(rest.over_limit, 0);

        let none = harness.run(&labeler, 0).await;
        assert_eq!((none.model_calls, none.already_labeled), (0, 4));
    }

    fn core_relation(key: &str, relation: &str, confidence: &str) -> seed::CoreRelation {
        seed::CoreRelation {
            reference_key: key.to_owned(),
            relation: relation.to_owned(),
            why: "Takes the reward modelling approach as its starting point.".to_owned(),
            quote: "extends the reward modelling approach".to_owned(),
            page: Some(1),
            confidence: Some(confidence.to_owned()),
        }
    }

    fn reading(run: &str, relations: Vec<seed::CoreRelation>) -> seed::CoreReading {
        seed::CoreReading {
            relations,
            run_key: run.to_owned(),
            model: "openrouter:core".to_owned(),
            ..seed::CoreReading::default()
        }
    }

    #[test]
    fn should_keep_reviewed_labels_and_rank_the_labeler_over_core_relations() {
        let core = |relation, reviewed| ExistingLabel {
            path: LabelPath::CoreRelation,
            relation,
            reviewed,
        };
        let labeler = |relation, reviewed| ExistingLabel {
            path: LabelPath::EdgeLabeler,
            relation,
            reviewed,
        };
        assert_eq!(
            merge_decision(None, LabelPath::CoreRelation, "extends"),
            MergeDecision::Insert
        );
        assert_eq!(
            merge_decision(
                Some(labeler("uses_method", false)),
                LabelPath::CoreRelation,
                "extends"
            ),
            MergeDecision::Keep { disagrees: true },
            "a core relation never replaces the labeler's reading of the passages"
        );
        assert_eq!(
            merge_decision(
                Some(core("extends", false)),
                LabelPath::EdgeLabeler,
                "background"
            ),
            MergeDecision::Replace { disagrees: true }
        );
        assert_eq!(
            merge_decision(
                Some(core("extends", false)),
                LabelPath::CoreRelation,
                "extends"
            ),
            MergeDecision::Replace { disagrees: false },
            "a newer core reading replaces an older one"
        );
        for path in [LabelPath::CoreRelation, LabelPath::EdgeLabeler] {
            assert_eq!(
                merge_decision(Some(core("extends", true)), path, "critiques"),
                MergeDecision::Keep { disagrees: true },
                "reviewed labels are never replaced"
            );
        }
        let user = ExistingLabel {
            path: LabelPath::User,
            relation: "critiques",
            reviewed: false,
        };
        assert_eq!(
            merge_decision(Some(user), LabelPath::EdgeLabeler, "critiques"),
            MergeDecision::Keep { disagrees: false }
        );
    }

    #[test]
    fn should_count_works_cited_together_in_one_marker() {
        assert_eq!(cited_together("[3]"), 1);
        assert_eq!(cited_together("[3, 7-9]"), 4);
        assert_eq!(cited_together("[2\u{2013}5; 11]"), 5);
        assert_eq!(
            cited_together("(Smith, 2001; Jones and Lee, 2003; Okafor, 2010)"),
            3
        );
        assert_eq!(cited_together("Smith (2001)"), 1);
        assert_eq!(cited_together(""), 1);
    }

    #[test]
    fn should_show_thesis_abstract_markers_and_citing_sentence_in_the_prompt() {
        let mut request = sample_request();
        request.citing_thesis = Some("Reefs recover faster when grazers return.".to_owned());
        request.cited.abstract_text =
            Some("We build a bleaching index from satellites.".to_owned());
        request.passages[0] =
            "Earlier text. We extend the cited method [1]. Later text.".to_owned();
        request.contexts = vec![PassageContext {
            marker: Some("[1, 4-6]".to_owned()),
            cited_together: 4,
            page: Some(3),
            citing_sentence: Some("We extend the cited method [1].".to_owned()),
        }];
        let prompt = label_prompt(&request);
        assert!(
            prompt.starts_with(LABEL_BRIEF.trim_end()),
            "the brief leads"
        );
        for expected in [
            "Its thesis: Reefs recover faster when grazers return.",
            "Abstract: We build a bleaching index from satellites.",
            "[0] Earlier text. We extend the cited method [1]. Later text.",
            "Citing sentence: We extend the cited method [1].",
            "Citation marker: [1, 4-6] (cites 4 works at once)",
            "Page: 3",
            "[1] See also [1] for details.",
        ] {
            assert!(prompt.contains(expected), "{expected}\n{prompt}");
        }
        assert!(label_prompt(&sample_request()).contains("Abstract: not held by Lysilogy"));
        for engagement in ENGAGEMENTS {
            assert!(prompt.contains(engagement), "{engagement}");
        }
    }

    #[test]
    fn should_reject_an_engagement_outside_the_vocabulary() {
        let texts = passages(&["We extend the cited method [1] to new tasks."]);
        let mut label = draft("extend the cited method", 0);
        label.engagement = Some("central".to_owned());
        assert_eq!(
            ground_quote(&texts, &label),
            Err(GroundingRejection::InvalidEngagement)
        );
        label.engagement = None;
        assert!(
            ground_quote(&texts, &label).is_ok(),
            "version 1 answers had none"
        );
        let schema: Value = serde_json::from_str(LABEL_SCHEMA).unwrap();
        assert_eq!(
            schema["properties"]["engagement"]["enum"],
            json!(ENGAGEMENTS)
        );
        // Property order reaches the provider where serde_json keeps it, so
        // check the text, which is what is sent.
        let mut order = ["passage_index", "quote", "engagement", "relation", "reason"]
            .map(|key| (LABEL_SCHEMA.find(&format!("\"{key}\": {{")).unwrap(), key))
            .to_vec();
        order.sort_unstable();
        assert_eq!(
            order.iter().map(|(_, key)| *key).collect::<Vec<_>>(),
            ["passage_index", "quote", "engagement", "relation", "reason"],
            "evidence first, the reason last"
        );
    }

    #[test]
    fn should_change_the_cache_key_when_the_prompt_version_changes() {
        let request = sample_request();
        assert_ne!(
            cache_key_for_version(&request, LABEL_PROMPT_VERSION - 1),
            label_cache_key(&request)
        );
    }

    #[test]
    fn should_widen_passages_and_add_core_citations_only_when_new() {
        let mention = |passage: &str, marker: &str| StoredMention {
            id: lysilogy_core::MentionId::new(),
            assertion: AssertionId::new(),
            entry: None,
            style: None,
            marker_text: Some(marker.to_owned()),
            page: Some(2),
            marker_start: None,
            marker_end: None,
            passage_start: None,
            passage_end: None,
            passage: passage.to_owned(),
            rects: None,
        };
        let mentions = vec![
            mention("We adopt their index [2].", "[2]"),
            mention("Reefs vary [2, 5, 9].", "[2, 5, 9]"),
        ];
        let brief = LabelBrief {
            surround: Some(Box::new(|mention: &StoredMention| {
                (mention.marker_text.as_deref() == Some("[2]")).then(|| {
                    "Heat stress matters.  We adopt their index [2]. It is daily.".to_owned()
                })
            })),
            ..LabelBrief::default()
        };
        let extra = vec![
            "We adopt their index [2].".to_owned(),
            "A sentence the core reading found [2].".to_owned(),
        ];
        let (texts, contexts, ids) = edge_passages(&brief, &mentions, &extra);
        assert_eq!(
            texts,
            [
                "Heat stress matters. We adopt their index [2]. It is daily.",
                "Reefs vary [2, 5, 9].",
                "A sentence the core reading found [2].",
            ]
        );
        assert_eq!(
            contexts[0].citing_sentence.as_deref(),
            Some("We adopt their index [2].")
        );
        assert_eq!(contexts[1].cited_together, 3);
        assert_eq!(contexts[1].citing_sentence, None);
        assert!(ids[0].is_some() && ids[1].is_some() && ids[2].is_none());
    }

    #[tokio::test]
    async fn should_not_reuse_a_cached_answer_from_an_older_prompt_version_but_resume_from_this_one()
     {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(2).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        // Cache an answer for every edge under the previous prompt version,
        // as a run before the bump would have.
        let context = harness.context();
        for (cited, _, mentions) in harness
            .evidence
            .local_edges(harness.citing, harness.version)
            .await
            .unwrap()
        {
            let (request, _) = build_request(
                &context,
                &LabelBrief::default(),
                None,
                cited,
                &mentions,
                &[],
            )
            .await
            .unwrap();
            let old_key = format!(
                "fake:model|{}",
                cache_key_for_version(&request, LABEL_PROMPT_VERSION - 1)
            );
            let stale = serde_json::to_value(LabelDraft {
                relation: "background".to_owned(),
                reason: "Old prompt.".to_owned(),
                quote: "extends the reward modelling approach".to_owned(),
                passage_index: 0,
                engagement: None,
            })
            .unwrap();
            harness
                .records
                .upsert(LABEL_RECORD_KIND, &old_key, Coverage::Complete, &stale)
                .await
                .unwrap();
        }
        let labeler = quoting_labeler();
        let first = harness.run(&labeler, 1).await;
        assert_eq!(
            (first.cache_hits, first.model_calls, first.over_limit),
            (0, 1, 1),
            "old-version answers are not reused"
        );
        let resumed = harness.run(&labeler, 1).await;
        assert_eq!((resumed.already_labeled, resumed.model_calls), (1, 1));
        assert_eq!(labeler.calls(), 2);
        let claims = harness.current().await;
        assert_eq!(claims.len(), 2);
        assert!(claims.values().all(|claim| claim.relation == "extends"
            && claim.prompt_version == i32::try_from(LABEL_PROMPT_VERSION).ok()
            && claim.path() == LabelPath::EdgeLabeler
            && claim.engagement.as_deref() == Some("substantive")));
    }

    #[tokio::test]
    async fn should_seed_core_relations_and_report_unresolvable_keys() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(3).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let mut invalid = core_relation("3", "refutes", "high");
        invalid.why = "Invented relation.".to_owned();
        let core = reading(
            "run-1",
            vec![
                core_relation("1", "uses_method", "high"),
                core_relation("[1]", "extends", "high"),
                core_relation("2", "supports", "low"),
                core_relation("Nobody (1999)", "critiques", "high"),
                core_relation("", "background", "medium"),
                invalid,
            ],
        );
        let outcome = harness.seed(&core).await;
        let summary = &outcome.summary;
        assert_eq!(summary.relations, 6);
        assert_eq!(summary.inserted, 1);
        assert_eq!(summary.duplicate_edge, 1);
        assert_eq!(summary.low_confidence, 1);
        assert_eq!(summary.unresolved.get("no_matching_entry"), Some(&1));
        assert_eq!(summary.unresolved.get("empty_key"), Some(&1));
        assert_eq!(summary.invalid.get("invalid_relation"), Some(&1));
        assert_eq!(
            summary
                .unresolved_keys
                .iter()
                .map(|key| key.key.as_str())
                .collect::<Vec<_>>(),
            ["Nobody (1999)", ""]
        );
        let counted = summary.inserted
            + summary.duplicate_edge
            + summary.low_confidence
            + summary.unresolved.values().sum::<usize>()
            + summary.invalid.values().sum::<usize>();
        assert_eq!(
            counted, summary.relations,
            "every relation is accounted for"
        );

        let claims = harness.current().await;
        let seeded = &claims[&harness.assertions[0]];
        assert_eq!(seeded.path(), LabelPath::CoreRelation);
        assert_eq!(seeded.relation, "uses_method");
        assert_eq!(seeded.model.as_deref(), Some("openrouter:core"));
        assert!(
            seeded.mention.is_some(),
            "the quote sits in a stored passage"
        );

        let again = harness.seed(&core).await;
        assert_eq!(
            (again.summary.inserted, again.summary.unchanged),
            (0, 1),
            "seeding is idempotent"
        );

        // The labeler fills the edges the core reading did not label, the
        // low-confidence one included.
        let labeler = quoting_labeler();
        let labelled = harness.run(&labeler, 25).await;
        assert_eq!((labelled.already_labeled, labelled.labeled), (1, 2));
    }

    #[tokio::test]
    async fn should_record_disagreement_between_core_relation_and_labeler() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(2).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        harness
            .seed(&reading(
                "run-1",
                vec![core_relation("1", "uses_method", "high")],
            ))
            .await;
        let background = ScriptedLabeler::new("fake:model", |request| {
            Ok(LabelDraft {
                relation: "background".to_owned(),
                reason: "Cited in passing for the approach.".to_owned(),
                quote: "extends the reward modelling approach".to_owned(),
                passage_index: request.passages.len().saturating_sub(1),
                engagement: Some("perfunctory".to_owned()),
            })
        });
        let plain = harness.run(&background, 25).await;
        assert_eq!(
            (plain.already_labeled, plain.labeled, plain.replaced),
            (1, 1, 0),
            "by default the labeler fills only unlabelled edges"
        );
        let relabel = harness
            .run_with(&background, &LabelBrief::default(), 25, true)
            .await;
        assert_eq!(relabel.replaced, 1, "the core label is replaced on relabel");
        assert_eq!(relabel.disagreements_with_core, 1);
        assert_eq!(relabel.engagement.get("perfunctory"), Some(&1));
        let claims = harness.current().await;
        assert_eq!(claims.len(), 2, "one current label per edge");
        let first = &claims[&harness.assertions[0]];
        assert_eq!(
            (first.path(), first.relation.as_str()),
            (LabelPath::EdgeLabeler, "background")
        );

        // A later core reading meets the labeler's label: it is kept, and the
        // disagreement is listed for the run report.
        let later = harness
            .seed(&reading(
                "run-2",
                vec![core_relation("1", "uses_method", "high")],
            ))
            .await;
        assert_eq!(
            (later.summary.kept_existing, later.summary.inserted),
            (1, 0)
        );
        let disagreement = &later.summary.disagreements[0];
        assert_eq!(disagreement.existing_relation, "background");
        assert_eq!(disagreement.existing_path, LabelPath::EdgeLabeler);
        assert!(!disagreement.replaced);
        assert_eq!(disagreement.cited_work, harness.cited[0].to_string());
    }

    #[tokio::test]
    async fn should_label_an_edge_from_core_citations_when_it_has_no_stored_mention() {
        let _guard = LOCK.lock().await;
        let Some(harness) = Harness::new(1).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        sqlx::query("delete from graph.citation_mention where assertion_id = any($1)")
            .bind(
                harness
                    .assertions
                    .iter()
                    .map(|id| id.as_uuid())
                    .collect::<Vec<_>>(),
            )
            .execute(&harness.pool)
            .await
            .unwrap();
        let labeler = quoting_labeler();
        let none = harness.run(&labeler, 25).await;
        assert_eq!(
            (none.edges_without_passages, none.model_calls),
            (1, 0),
            "counted, not hidden"
        );

        let mut core = reading("run-1", Vec::new());
        core.citations = vec![seed::CoreCitation {
            reference_key: "1".to_owned(),
            context: "Our design extends the reward modelling approach of [1].".to_owned(),
            page: Some(2),
        }];
        let outcome = harness.seed(&core).await;
        assert_eq!(outcome.summary.citation_passages, 1);
        let brief = LabelBrief {
            extra_passages: outcome.extra_passages,
            ..LabelBrief::default()
        };
        let labelled = harness.run_with(&labeler, &brief, 25, false).await;
        assert_eq!(
            (labelled.edges_from_core_citations, labelled.labeled),
            (1, 1)
        );
        let claim = &harness.current().await[&harness.assertions[0]];
        assert!(
            claim.mention.is_none(),
            "a core citing sentence has no stored mention"
        );
    }
}
