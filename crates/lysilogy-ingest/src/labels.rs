//! Grounded relationship labels for local citation edges.
//!
//! A model reads the passages in which a paper cites a work and answers with a
//! relation, a reason, and a verbatim quote. The answer is only kept when the
//! quote is an exact substring of a passage the model was shown ("grounding").
//! Raw model output is cached in `source.source_record` so reruns cost nothing,
//! including reruns of answers that failed grounding.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lysilogy_core::{AssertionId, Coverage, DocumentVersionId, SourceRecordId, WorkId};
use lysilogy_db::evidence::{EvidenceRepository, StoredMention};
use lysilogy_db::relationship::{NewAiClaim, RelationshipRepository};
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::Result;
use crate::bibliography::BibliographicFields;
use crate::concurrency::{BoxFuture, join_all};

/// Bump when the prompt or the request shape changes, so old cache entries are not reused.
pub const LABEL_PROMPT_VERSION: u32 = 1;

/// Source record kind for cached raw model output.
pub const LABEL_RECORD_KIND: &str = "relationship_label";

/// The relation vocabulary the model must choose from.
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

/// Strict output schema: every property is required and no others are allowed.
pub const LABEL_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["relation", "reason", "quote", "passage_index"],
  "properties": {
    "relation": {
      "type": "string",
      "enum": ["background", "uses_method", "uses_data", "extends", "supports", "contrasts", "critiques", "compares", "other"]
    },
    "reason": { "type": "string" },
    "quote": { "type": "string" },
    "passage_index": { "type": "integer" }
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
}

/// One edge to label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LabelRequest {
    pub citing_title: Option<String>,
    pub cited: CitedWorkBrief,
    /// Distinct passages, in text order, whitespace-collapsed.
    pub passages: Vec<String>,
}

/// The model's answer, before grounding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDraft {
    pub relation: String,
    pub reason: String,
    pub quote: String,
    pub passage_index: usize,
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
            Self::EmptyReason => "empty_reason",
            Self::ReasonTooLong => "reason_too_long",
        }
    }
}

/// The fixed-format prompt for one edge.
#[must_use]
pub fn label_prompt(request: &LabelRequest) -> String {
    let mut prompt = String::from(
        "You are labeling why one paper cites another. Read the numbered passages from the \
         citing paper, each of which cites the work described below, and decide how the \
         citing paper relates to the cited work.\n\n",
    );
    let unknown = "(unknown title)";
    let cited = &request.cited;
    let _ = writeln!(
        prompt,
        "Citing paper: {}",
        request.citing_title.as_deref().unwrap_or(unknown)
    );
    let _ = writeln!(prompt, "Cited work:");
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
    let _ = writeln!(prompt, "\nPassages:");
    for (index, passage) in request.passages.iter().enumerate() {
        let _ = writeln!(prompt, "[{index}] {passage}");
    }
    let _ = write!(
        prompt,
        "\nRelations: {}.\n\n\
         Answer with JSON: relation (one of the relations above), reason (one or two \
         sentences, at most {MAX_REASON_CHARS} characters, saying why the citing paper cites \
         the work), quote (text copied VERBATIM, character for character, from exactly one \
         numbered passage, at least {MIN_QUOTE_WORDS} words, never just a citation marker), \
         and passage_index (the number of the passage the quote comes from).\n",
        RELATIONS.join(", ")
    );
    prompt
}

/// SHA-256 (hex) of the canonical request, which keys the label cache.
#[must_use]
pub fn label_cache_key(request: &LabelRequest) -> String {
    let canonical = json!({
        "prompt_version": LABEL_PROMPT_VERSION,
        "citing_title": request.citing_title,
        "cited": request.cited,
        "passages": request.passages,
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
    let words = quote
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count();
    if words < MIN_QUOTE_WORDS {
        return Err(GroundingRejection::QuoteTooShort);
    }
    let grounded = locate_quote(passages, draft.passage_index, quote)
        .ok_or(GroundingRejection::QuoteNotInPassage)?;
    if !RELATIONS.contains(&draft.relation.as_str()) {
        return Err(GroundingRejection::InvalidRelation);
    }
    let reason = draft.reason.trim();
    if reason.is_empty() {
        return Err(GroundingRejection::EmptyReason);
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(GroundingRejection::ReasonTooLong);
    }
    Ok(grounded)
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

/// How a label pass runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelOptions {
    /// Most model calls; cache hits do not count.
    pub limit: usize,
    /// Model calls in flight at once.
    pub concurrency: usize,
}

impl Default for LabelOptions {
    fn default() -> Self {
        Self {
            limit: 25,
            concurrency: 2,
        }
    }
}

/// What one label pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LabelSummary {
    /// Local edges that have at least one mention.
    pub edges_considered: usize,
    pub model_calls: usize,
    pub cache_hits: usize,
    pub labeled: usize,
    pub rejected_for_grounding: usize,
    pub rejections: BTreeMap<String, usize>,
    pub failed: usize,
    /// Edges left unlabeled because the call limit was reached.
    pub over_limit: usize,
    pub already_labeled: usize,
}

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
    mentions: Vec<StoredMention>,
    request: LabelRequest,
    cache_source_key: String,
}

/// A model call that has to be made.
struct PendingCall {
    job: EdgeJob,
}

/// A raw draft ready to ground.
struct Drafted {
    job: EdgeJob,
    record: SourceRecordId,
    draft: LabelDraft,
}

/// Label the local edges of `citing_work` that have no current claim.
///
/// Edges are taken by mention count (most first), then by text position.
/// Labeler errors count as `failed` and never stop the pass.
///
/// # Errors
/// Database failures.
pub async fn label_edges(
    ctx: &LabelContext<'_>,
    citing_work: WorkId,
    document_version: DocumentVersionId,
    labeler: &dyn RelationshipLabeler,
    options: LabelOptions,
) -> Result<LabelSummary> {
    let mut summary = LabelSummary::default();
    let citing_title = ctx
        .works
        .get(citing_work)
        .await?
        .and_then(|work| work.display_title);
    let mut edges = ctx
        .evidence
        .local_edges(citing_work, document_version)
        .await?;
    edges.retain(|(_, _, mentions)| !mentions.is_empty());
    edges.sort_by_key(|(_, _, mentions)| {
        (
            std::cmp::Reverse(mentions.len()),
            mentions.first().and_then(|mention| mention.marker_start),
        )
    });
    summary.edges_considered = edges.len();

    let model_key = labeler.model_key();
    let mut cached = Vec::new();
    let mut pending = Vec::new();
    for (cited, assertion, mentions) in edges {
        if ctx.relationships.has_current_claim(assertion).await? {
            summary.already_labeled += 1;
            continue;
        }
        let request = build_request(ctx, citing_title.as_deref(), cited, &mentions).await?;
        let cache_source_key = format!("{model_key}|{}", label_cache_key(&request));
        let job = EdgeJob {
            assertion,
            mentions,
            request,
            cache_source_key,
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
        pending.push(PendingCall { job });
    }

    let mut drafted = cached;
    summary.model_calls = pending.len();
    let semaphore = Arc::new(Semaphore::new(options.concurrency.max(1)));
    let calls = pending
        .iter()
        .map(|call| {
            let future: BoxFuture<'_, Result<CallOutcome>> =
                Box::pin(run_call(ctx, labeler, &semaphore, &call.job));
            future
        })
        .collect();
    let outcomes = join_all(calls).await;
    for (call, outcome) in pending.into_iter().zip(outcomes) {
        match outcome? {
            CallOutcome::Drafted(record, draft) => drafted.push(Drafted {
                job: call.job,
                record,
                draft,
            }),
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
    let passage = &job.request.passages[grounded.passage_index];
    let mention = job
        .mentions
        .iter()
        .find(|mention| &mention.passage == passage)
        .map(|mention| mention.id);
    let quote = draft.quote.trim();
    let inserted = ctx
        .relationships
        .insert_ai_claim(&NewAiClaim {
            assertion: job.assertion,
            mention,
            relation: &draft.relation,
            reason: draft.reason.trim(),
            quote,
            model: model_key,
            record,
        })
        .await?;
    if inserted.is_some() {
        summary.labeled += 1;
    }
    Ok(())
}

/// The model-facing description of one edge.
async fn build_request(
    ctx: &LabelContext<'_>,
    citing_title: Option<&str>,
    cited: WorkId,
    mentions: &[StoredMention],
) -> Result<LabelRequest> {
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
        .unwrap_or_default()
        .into_iter()
        .take(MAX_AUTHORS)
        .collect();
    let entry_text = entry
        .as_ref()
        .map(|entry| entry.raw_text.chars().take(MAX_ENTRY_TEXT_CHARS).collect());
    Ok(LabelRequest {
        citing_title: citing_title.map(str::to_owned),
        cited: CitedWorkBrief {
            title,
            year,
            authors,
            entry_text,
        },
        passages: distinct_passages(mentions),
    })
}

/// Distinct passages in text order, at most [`MAX_PASSAGES`].
fn distinct_passages(mentions: &[StoredMention]) -> Vec<String> {
    let mut passages: Vec<String> = Vec::new();
    for mention in mentions {
        if mention.passage.trim().is_empty() || passages.contains(&mention.passage) {
            continue;
        }
        passages.push(mention.passage.clone());
        if passages.len() == MAX_PASSAGES {
            break;
        }
    }
    passages
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
            },
            passages: passages(&[
                "We extend the cited method [1].",
                "See also [1] for details.",
            ]),
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
        assert!(prompt.contains("VERBATIM"), "{prompt}");
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
        assert_eq!(
            distinct_passages(&mentions),
            vec!["a", "b", "c", "d", "e", "f"]
        );
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

        async fn run(&self, labeler: &ScriptedLabeler, limit: usize) -> LabelSummary {
            let context = LabelContext {
                works: &self.works,
                records: &self.records,
                evidence: &self.evidence,
                relationships: &self.relationships,
            };
            label_edges(
                &context,
                self.citing,
                self.version,
                labeler,
                LabelOptions {
                    limit,
                    concurrency: 2,
                },
            )
            .await
            .expect("label edges")
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
}
