//! Grounded model extraction of bibliographic fields.
//!
//! bib-v1 leaves authors and year `Missing` or `Heuristic`. A cheap model reads
//! the printed bibliography entries (or the paper's own front matter) and
//! answers with a title, an author list and a year. An answer is only kept when
//! each value is a verbatim substring of the text the model was shown
//! ("grounding"). Raw answers are cached per entry in `source.source_record`,
//! so reruns cost nothing, including entries the model skipped.
//!
//! Kept values become `model`-confidence claims, which rank below every
//! deterministic claim of the same origin.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use lysilogy_core::provenance::{FieldValue, selection_rank};
use lysilogy_core::{
    ClaimConfidence, ClaimOrigin, Coverage, DocumentVersionId, EntryId, ProvenanceField,
    ReviewState, SourceRecordId, WorkId,
};
use lysilogy_db::claim::{ClaimRepository, NewClaim};
use lysilogy_db::evidence::{EvidenceRepository, StoredEntry};
use lysilogy_db::source_record::SourceRecordRepository;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::Result;
use crate::bibliography::{BibliographicFields, FieldConfidence, parse_fields};
use crate::concurrency::{BoxFuture, join_all};

/// Bump when the prompt or the schema changes, so old cache entries are not reused.
pub const FIELD_PROMPT_VERSION: u32 = 1;

/// Source record kind for cached per-entry model answers.
pub const BIBLIOGRAPHY_FIELDS_KIND: &str = "bibliography_fields_model";

/// Source record kind for the cached answer about the paper's own front matter.
pub const PAPER_FIELDS_KIND: &str = "paper_fields_model";

/// Extractor recorded on claims grounded from a bibliography entry.
pub const BIBLIOGRAPHY_MODEL_EXTRACTOR: &str = "bib-model-v1";

/// Extractor recorded on claims grounded from the paper's front matter.
pub const FRONT_MATTER_MODEL_EXTRACTOR: &str = "frontmatter-model-v1";

/// Locator recorded on claims grounded from the front matter.
const FRONT_MATTER_LOCATOR: &str = "front_matter";

/// Entries per model call.
pub const FIELD_BATCH: usize = 20;

/// Longest entry text shown to the model, in characters.
pub const ENTRY_SHOWN_CHARS: usize = 600;

/// Longest front matter shown to the model, in characters.
pub const FRONT_MATTER_SHOWN_CHARS: usize = 4000;

/// Longest accepted title, in characters.
const MAX_TITLE_CHARS: usize = 500;

/// Most author names accepted for one item.
const MAX_AUTHORS: usize = 100;

/// Accepted publication years.
const YEAR_RANGE: std::ops::RangeInclusive<i32> = 1400..=2200;

/// Strict output schema: every property is required and no others are allowed.
/// A value the model cannot find is an empty string or an empty array.
pub const FIELD_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["items"],
  "properties": {
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["index", "title", "authors", "year"],
        "properties": {
          "index": { "type": "integer" },
          "title": { "type": "string" },
          "authors": { "type": "array", "items": { "type": "string" } },
          "year": { "type": "string" }
        }
      }
    }
  }
}"#;

/// The future returned by a field extractor.
pub type FieldFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<FieldDraft, FieldExtractorError>> + Send + 'a>>;

/// A model-backed source of bibliographic fields.
pub trait FieldExtractor: Send + Sync {
    /// Identifies the model in cache keys.
    fn model_key(&self) -> &str;

    /// Read the items in `request` and answer for each.
    fn extract<'a>(&'a self, request: &'a FieldRequest) -> FieldFuture<'a>;
}

/// Why an extractor produced no draft.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FieldExtractorError {
    /// The model program is not installed or not reachable.
    #[error("field extractor unavailable: {0}")]
    Unavailable(String),
    /// The model program ran and failed.
    #[error("field extractor failed: {0}")]
    Failed(String),
    /// The model answered with something that is not a field draft.
    #[error("field extractor returned a malformed answer: {0}")]
    Malformed(String),
}

/// What the items are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldRequestKind {
    /// Printed bibliography entries, one per item.
    BibliographyEntries,
    /// The first pages of one paper, as a single item.
    FrontMatter,
}

/// Text for the model to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRequest {
    pub kind: FieldRequestKind,
    /// The text shown for each item, whitespace-collapsed and already cut.
    pub items: Vec<String>,
}

/// The model's answer for one item, before grounding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDraftItem {
    pub index: i64,
    pub title: String,
    pub authors: Vec<String>,
    pub year: String,
}

/// The model's answer, before grounding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDraft {
    pub items: Vec<FieldDraftItem>,
}

/// Why a value was not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldDrop {
    TitleNotInEntry,
    YearNotInEntry,
    YearMalformed,
    AuthorNotInEntry,
    TooManyAuthors,
    UnknownIndex,
}

impl FieldDrop {
    /// The stable code used in summaries and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TitleNotInEntry => "title_not_in_entry",
            Self::YearNotInEntry => "year_not_in_entry",
            Self::YearMalformed => "year_malformed",
            Self::AuthorNotInEntry => "author_not_in_entry",
            Self::TooManyAuthors => "too_many_authors",
            Self::UnknownIndex => "unknown_index",
        }
    }
}

/// The values of a draft item that survived grounding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroundedFields {
    pub title: Option<String>,
    /// The leading four digits of the printed year.
    pub year: Option<i32>,
    pub authors: Option<Vec<String>>,
    /// One code per value that was offered and rejected.
    pub dropped: Vec<FieldDrop>,
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whitespace-collapsed `text`, cut at `limit` characters on a char boundary.
fn shown_text(text: &str, limit: usize) -> String {
    let collapsed = collapse_whitespace(text);
    match collapsed.char_indices().nth(limit) {
        Some((cut, _)) => collapsed[..cut].trim_end().to_owned(),
        None => collapsed,
    }
}

/// The fixed-format prompt for one request.
#[must_use]
pub fn field_prompt(request: &FieldRequest) -> String {
    let mut prompt = match request.kind {
        FieldRequestKind::BibliographyEntries => String::from(
            "Below are numbered entries copied from the reference list of a paper. For each \
             entry give the cited work's title, its authors in printed order, and its year of \
             publication.\n\n",
        ),
        FieldRequestKind::FrontMatter => String::from(
            "Below is the front matter of one paper. Give this paper's own title, its authors \
             in printed order, and its year of publication if one is printed.\n\n",
        ),
    };
    for (index, item) in request.items.iter().enumerate() {
        let _ = writeln!(prompt, "[{index}] {item}");
    }
    prompt.push_str(
        "\nAnswer with JSON: items, one object per numbered text, with index (the number in \
         brackets), title, authors (a list with one string per author) and year. Copy every \
         value character for character from the text, exactly as printed; never correct, \
         expand, translate or reorder anything. Leave title and year as an empty string and \
         authors as an empty list when the text does not print them.\n",
    );
    prompt
}

/// The cache key for one shown text, which names a `source_record`.
#[must_use]
pub fn field_cache_key(model_key: &str, shown: &str) -> String {
    format!(
        "{model_key}|v{FIELD_PROMPT_VERSION}|{:x}",
        Sha256::digest(shown.as_bytes())
    )
}

fn leading_year(printed: &str) -> Option<i32> {
    let bytes = printed.as_bytes();
    let well_formed = match bytes.len() {
        4 => bytes.iter().all(u8::is_ascii_digit),
        5 => bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4].is_ascii_lowercase(),
        _ => false,
    };
    if !well_formed {
        return None;
    }
    let year: i32 = printed[..4].parse().ok()?;
    YEAR_RANGE.contains(&year).then_some(year)
}

/// Keep only the values of `item` that occur verbatim in `shown`.
///
/// Values are trimmed and whitespace-collapsed; `shown` is expected to be
/// collapsed already. An empty value means the model found nothing and is not
/// a drop. Any author missing from `shown` drops the whole list, because a
/// partial ordered list is wrong.
#[must_use]
pub fn ground_field_draft(shown: &str, item: &FieldDraftItem) -> GroundedFields {
    let mut grounded = GroundedFields::default();

    let title = collapse_whitespace(&item.title);
    if !title.is_empty() {
        if title.chars().count() <= MAX_TITLE_CHARS && shown.contains(&title) {
            grounded.title = Some(title);
        } else {
            grounded.dropped.push(FieldDrop::TitleNotInEntry);
        }
    }

    let year = item.year.trim();
    if !year.is_empty() {
        match leading_year(year) {
            None => grounded.dropped.push(FieldDrop::YearMalformed),
            Some(_) if !shown.contains(year) => grounded.dropped.push(FieldDrop::YearNotInEntry),
            Some(value) => grounded.year = Some(value),
        }
    }

    let authors: Vec<String> = item
        .authors
        .iter()
        .map(|name| collapse_whitespace(name))
        .filter(|name| !name.is_empty())
        .collect();
    if authors.len() > MAX_AUTHORS {
        grounded.dropped.push(FieldDrop::TooManyAuthors);
    } else if !authors.is_empty() {
        if authors.iter().all(|name| shown.contains(name.as_str())) {
            grounded.authors = Some(authors);
        } else {
            grounded.dropped.push(FieldDrop::AuthorNotInEntry);
        }
    }
    grounded
}

/// Pair each of `count` shown items with the draft item that names its index.
/// Items naming an index outside `0..count` are counted as `unknown_index`;
/// the first item wins when an index repeats.
#[must_use]
pub fn assign_draft_items(
    count: usize,
    draft: &FieldDraft,
) -> (Vec<Option<FieldDraftItem>>, usize) {
    let mut slots: Vec<Option<FieldDraftItem>> = vec![None; count];
    let mut unknown = 0;
    for item in &draft.items {
        let Some(slot) = usize::try_from(item.index)
            .ok()
            .and_then(|index| slots.get_mut(index))
        else {
            unknown += 1;
            continue;
        };
        if slot.is_none() {
            *slot = Some(item.clone());
        }
    }
    (slots, unknown)
}

/// How a bibliography field pass runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldOptions {
    /// Most model calls; cache hits do not count.
    pub limit: usize,
    /// Model calls in flight at once.
    pub concurrency: usize,
}

impl Default for FieldOptions {
    fn default() -> Self {
        Self {
            limit: 10,
            concurrency: 2,
        }
    }
}

/// What one field pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FieldHarvestSummary {
    /// Resolved entries examined (or `1` for the front matter).
    pub entries_considered: usize,
    /// Entries skipped because their Work already has higher-tier authors and year.
    pub skipped_higher_tier: usize,
    pub model_calls: usize,
    /// Entries answered from the cache.
    pub cache_hits: usize,
    /// Claim rows inserted (already-recorded ones do not count).
    pub claims_written: usize,
    /// Dropped values by [`FieldDrop`] code.
    pub dropped: BTreeMap<String, usize>,
    /// Model calls that failed.
    pub failed: usize,
    /// Entries left unasked because the call limit was reached.
    pub over_limit: usize,
}

impl FieldHarvestSummary {
    fn count_drops(&mut self, drops: &[FieldDrop]) {
        for drop in drops {
            *self.dropped.entry(drop.as_str().to_owned()).or_default() += 1;
        }
    }

    /// Total values dropped across codes.
    #[must_use]
    pub fn dropped_total(&self) -> usize {
        self.dropped.values().sum()
    }
}

/// The repositories the field pass works through.
pub struct FieldContext<'a> {
    pub claims: &'a ClaimRepository,
    pub records: &'a SourceRecordRepository,
    pub evidence: &'a EvidenceRepository,
}

/// An entry the model may be asked about.
struct Target {
    entry: StoredEntry,
    work: WorkId,
    shown: String,
    cache_key: String,
}

/// A target with its answer, ready to ground.
struct Answered {
    target: Target,
    record: SourceRecordId,
    item: Option<FieldDraftItem>,
}

/// The outcome of one model call.
enum CallOutcome {
    Answered(Vec<(SourceRecordId, Option<FieldDraftItem>)>, usize),
    Failed(FieldExtractorError),
}

/// Whether `value` needs a model: bib-v1 left authors or year weak.
fn needs_model(fields: &BibliographicFields) -> bool {
    let weak = |confidence: FieldConfidence| confidence != FieldConfidence::Explicit;
    weak(fields.authors.confidence) || weak(fields.year.confidence)
}

/// Whether `work` already has eligible author and year claims that outrank
/// `citing_bibliography`, so a model claim could never be selected.
async fn has_higher_tier_authors_and_year(claims: &ClaimRepository, work: WorkId) -> Result<bool> {
    let ceiling = selection_rank(ClaimOrigin::CitingBibliography, ClaimConfidence::Explicit).origin;
    let mut has_authors = false;
    let mut has_year = false;
    for claim in claims.claims_for(work).await? {
        let eligible = claim.retracted_at.is_none()
            && claim.review_state != ReviewState::Rejected
            && selection_rank(claim.origin, claim.confidence).origin < ceiling
            && FieldValue::parse(claim.field, &claim.value).is_some();
        if !eligible {
            continue;
        }
        match claim.field {
            ProvenanceField::Authors => has_authors = true,
            ProvenanceField::Year => has_year = true,
            _ => {}
        }
    }
    Ok(has_authors && has_year)
}

/// The cached answer for a key: `Some(None)` is a cached omission.
async fn cached_answer(
    records: &SourceRecordRepository,
    kind: &str,
    key: &str,
) -> Result<Option<(SourceRecordId, Option<FieldDraftItem>)>> {
    let Some((record, payload)) = records.find(kind, key).await? else {
        return Ok(None);
    };
    if payload.get("omitted").and_then(Value::as_bool) == Some(true) {
        return Ok(Some((record, None)));
    }
    Ok(serde_json::from_value::<FieldDraftItem>(payload)
        .ok()
        .map(|item| (record, Some(item))))
}

fn answer_payload(item: Option<&FieldDraftItem>) -> Value {
    item.map_or_else(
        || json!({ "omitted": true }),
        |item| serde_json::to_value(item).unwrap_or(Value::Null),
    )
}

/// Ask for one batch and cache each entry's raw answer immediately, so a
/// crash after the call does not lose a paid answer.
async fn run_batch(
    ctx: &FieldContext<'_>,
    extractor: &dyn FieldExtractor,
    semaphore: &Semaphore,
    kind: FieldRequestKind,
    record_kind: &str,
    batch: &[&Target],
) -> Result<CallOutcome> {
    let Ok(_permit) = semaphore.acquire().await else {
        return Ok(CallOutcome::Failed(FieldExtractorError::Failed(
            "field semaphore closed".to_owned(),
        )));
    };
    let request = FieldRequest {
        kind,
        items: batch.iter().map(|target| target.shown.clone()).collect(),
    };
    let draft = match extractor.extract(&request).await {
        Ok(draft) => draft,
        Err(error) => return Ok(CallOutcome::Failed(error)),
    };
    let (slots, unknown) = assign_draft_items(batch.len(), &draft);
    let mut answers = Vec::with_capacity(batch.len());
    for (target, item) in batch.iter().zip(slots) {
        let record = ctx
            .records
            .upsert(
                record_kind,
                &target.cache_key,
                Coverage::Complete,
                &answer_payload(item.as_ref()),
            )
            .await?;
        answers.push((record, item));
    }
    Ok(CallOutcome::Answered(answers, unknown))
}

/// Entries the model should read, split into those already answered in the
/// cache and those that need a call.
async fn collect_targets(
    ctx: &FieldContext<'_>,
    citing_work: WorkId,
    document_version: DocumentVersionId,
    model_key: &str,
    summary: &mut FieldHarvestSummary,
) -> Result<(Vec<Answered>, Vec<Target>)> {
    let resolved = ctx.evidence.resolved_entries(document_version).await?;
    let mut higher_tier: HashMap<WorkId, bool> = HashMap::new();
    let mut answered = Vec::new();
    let mut misses = Vec::new();
    for (entry, work) in resolved {
        if work == citing_work {
            continue;
        }
        summary.entries_considered += 1;
        let fields = serde_json::from_value::<BibliographicFields>(entry.fields.clone())
            .unwrap_or_else(|_| parse_fields(&entry.raw_text, entry.printed_key.clone()));
        if !needs_model(&fields) {
            continue;
        }
        let skip = if let Some(known) = higher_tier.get(&work) {
            *known
        } else {
            let known = has_higher_tier_authors_and_year(ctx.claims, work).await?;
            higher_tier.insert(work, known);
            known
        };
        if skip {
            summary.skipped_higher_tier += 1;
            continue;
        }
        let shown = shown_text(&entry.raw_text, ENTRY_SHOWN_CHARS);
        if shown.is_empty() {
            continue;
        }
        let cache_key = field_cache_key(model_key, &shown);
        let target = Target {
            entry,
            work,
            shown,
            cache_key,
        };
        match cached_answer(ctx.records, BIBLIOGRAPHY_FIELDS_KIND, &target.cache_key).await? {
            Some((record, item)) => {
                summary.cache_hits += 1;
                answered.push(Answered {
                    target,
                    record,
                    item,
                });
            }
            None => misses.push(target),
        }
    }
    Ok((answered, misses))
}

/// Ask for the batches of `misses` the limit allows, in ordinal order, and
/// return the answered targets. Failed batches count and are skipped.
async fn call_batches(
    ctx: &FieldContext<'_>,
    extractor: &dyn FieldExtractor,
    options: FieldOptions,
    misses: Vec<Target>,
    summary: &mut FieldHarvestSummary,
) -> Result<Vec<Answered>> {
    let batch_lens: Vec<usize> = misses.chunks(FIELD_BATCH).map(<[Target]>::len).collect();
    let allowed = batch_lens.len().min(options.limit);
    summary.over_limit = batch_lens[allowed..].iter().sum();
    summary.model_calls = allowed;
    let semaphore = Semaphore::new(options.concurrency.max(1));
    let semaphore = &semaphore;
    let results = {
        let calls: Vec<BoxFuture<'_, Result<CallOutcome>>> = misses
            .chunks(FIELD_BATCH)
            .take(allowed)
            .map(|batch| {
                let members: Vec<&Target> = batch.iter().collect();
                let future: BoxFuture<'_, Result<CallOutcome>> = Box::pin(async move {
                    run_batch(
                        ctx,
                        extractor,
                        semaphore,
                        FieldRequestKind::BibliographyEntries,
                        BIBLIOGRAPHY_FIELDS_KIND,
                        &members,
                    )
                    .await
                });
                future
            })
            .collect();
        join_all(calls).await
    };
    let mut answered = Vec::new();
    let mut remaining = misses.into_iter();
    for (batch_len, outcome) in batch_lens.into_iter().zip(results) {
        let members: Vec<Target> = remaining.by_ref().take(batch_len).collect();
        match outcome? {
            CallOutcome::Answered(answers, unknown) => {
                if unknown > 0 {
                    *summary
                        .dropped
                        .entry(FieldDrop::UnknownIndex.as_str().to_owned())
                        .or_default() += unknown;
                }
                for (target, (record, item)) in members.into_iter().zip(answers) {
                    answered.push(Answered {
                        target,
                        record,
                        item,
                    });
                }
            }
            CallOutcome::Failed(error) => {
                tracing::warn!(%error, "field extractor failed");
                summary.failed += 1;
            }
        }
    }
    Ok(answered)
}

/// Read the resolved entries of `document_version` whose authors or year are
/// weak, and record the grounded answers on the cited Works.
///
/// Claims are `citing_bibliography` / `bib-model-v1` / `model`. Extractor
/// errors count as `failed` and never stop the pass.
///
/// # Errors
/// Database failures.
pub async fn harvest_bibliography_fields(
    ctx: &FieldContext<'_>,
    citing_work: WorkId,
    document_version: DocumentVersionId,
    extractor: &dyn FieldExtractor,
    options: FieldOptions,
) -> Result<FieldHarvestSummary> {
    let mut summary = FieldHarvestSummary::default();
    let (mut answered, misses) = collect_targets(
        ctx,
        citing_work,
        document_version,
        extractor.model_key(),
        &mut summary,
    )
    .await?;
    answered.extend(call_batches(ctx, extractor, options, misses, &mut summary).await?);
    for answer in answered {
        let Some(item) = &answer.item else {
            continue;
        };
        let grounded = ground_field_draft(&answer.target.shown, item);
        summary.count_drops(&grounded.dropped);
        summary.claims_written += record_grounded(
            ctx.claims,
            answer.target.work,
            &grounded,
            ClaimOrigin::CitingBibliography,
            BIBLIOGRAPHY_MODEL_EXTRACTOR,
            answer.record,
            Some(answer.target.entry.id),
            &answer.target.entry.entry_key,
        )
        .await?;
    }
    Ok(summary)
}

/// Record the grounded values as `model` claims. Returns rows inserted.
#[allow(clippy::too_many_arguments)] // one flat claim description
async fn record_grounded(
    claims: &ClaimRepository,
    work: WorkId,
    grounded: &GroundedFields,
    origin: ClaimOrigin,
    extractor: &str,
    record: SourceRecordId,
    entry: Option<EntryId>,
    locator: &str,
) -> Result<usize> {
    let values: Vec<FieldValue> = [
        grounded.title.clone().map(FieldValue::Title),
        grounded.year.map(FieldValue::Year),
        grounded.authors.clone().map(FieldValue::Authors),
    ]
    .into_iter()
    .flatten()
    .collect();
    if values.is_empty() {
        return Ok(0);
    }
    let new_claims: Vec<NewClaim<'_>> = values
        .iter()
        .map(|value| NewClaim {
            work_id: work,
            field: value.field(),
            value,
            origin,
            extractor,
            confidence: ClaimConfidence::Model,
            source_record_id: Some(record),
            entry_id: entry,
            locator: Some(locator),
        })
        .collect();
    Ok(claims.record(&new_claims).await?.inserted)
}

/// Read the paper's own front matter and record the grounded title, authors
/// and year.
///
/// Runs when `work` lacks a non-model `paper` claim for any of them, and
/// writes `paper` / `frontmatter-model-v1` / `model` claims for the fields
/// that lack one.
///
/// # Errors
/// Database failures.
pub async fn harvest_front_matter_fields(
    ctx: &FieldContext<'_>,
    work: WorkId,
    front_matter: &str,
    extractor: &dyn FieldExtractor,
) -> Result<FieldHarvestSummary> {
    let mut summary = FieldHarvestSummary::default();
    let mut covered = [false; 3];
    for claim in ctx.claims.claims_for(work).await? {
        if claim.origin != ClaimOrigin::Paper
            || claim.confidence == ClaimConfidence::Model
            || claim.retracted_at.is_some()
        {
            continue;
        }
        match claim.field {
            ProvenanceField::Title => covered[0] = true,
            ProvenanceField::Authors => covered[1] = true,
            ProvenanceField::Year => covered[2] = true,
            _ => {}
        }
    }
    if covered.iter().all(|covered| *covered) {
        return Ok(summary);
    }
    let shown = shown_text(front_matter, FRONT_MATTER_SHOWN_CHARS);
    if shown.is_empty() {
        return Ok(summary);
    }
    summary.entries_considered = 1;
    let key = field_cache_key(extractor.model_key(), &shown);
    let (record, item) =
        if let Some(hit) = cached_answer(ctx.records, PAPER_FIELDS_KIND, &key).await? {
            summary.cache_hits = 1;
            hit
        } else {
            summary.model_calls = 1;
            let request = FieldRequest {
                kind: FieldRequestKind::FrontMatter,
                items: vec![shown.clone()],
            };
            let draft = match extractor.extract(&request).await {
                Ok(draft) => draft,
                Err(error) => {
                    tracing::warn!(%error, "field extractor failed on front matter");
                    summary.failed = 1;
                    return Ok(summary);
                }
            };
            let (mut slots, unknown) = assign_draft_items(1, &draft);
            if unknown > 0 {
                summary
                    .dropped
                    .insert(FieldDrop::UnknownIndex.as_str().to_owned(), unknown);
            }
            let item = slots.pop().flatten();
            let record = ctx
                .records
                .upsert(
                    PAPER_FIELDS_KIND,
                    &key,
                    Coverage::Complete,
                    &answer_payload(item.as_ref()),
                )
                .await?;
            (record, item)
        };
    let Some(item) = item else {
        return Ok(summary);
    };
    let mut grounded = ground_field_draft(&shown, &item);
    summary.count_drops(&grounded.dropped);
    if covered[0] {
        grounded.title = None;
    }
    if covered[1] {
        grounded.authors = None;
    }
    if covered[2] {
        grounded.year = None;
    }
    summary.claims_written = record_grounded(
        ctx.claims,
        work,
        &grounded,
        ClaimOrigin::Paper,
        FRONT_MATTER_MODEL_EXTRACTOR,
        record,
        None,
        FRONT_MATTER_LOCATOR,
    )
    .await?;
    Ok(summary)
}

type ScriptedFn =
    dyn Fn(&FieldRequest) -> std::result::Result<FieldDraft, FieldExtractorError> + Send + Sync;

/// A deterministic extractor for tests: answers from a closure and counts calls.
pub struct ScriptedFieldExtractor {
    model_key: String,
    script: Box<ScriptedFn>,
    calls: AtomicUsize,
}

impl ScriptedFieldExtractor {
    /// An extractor that answers every request with `script`.
    #[must_use]
    pub fn new(
        model_key: &str,
        script: impl Fn(&FieldRequest) -> std::result::Result<FieldDraft, FieldExtractorError>
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

    /// How many times `extract` was called.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl FieldExtractor for ScriptedFieldExtractor {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn extract<'a>(&'a self, request: &'a FieldRequest) -> FieldFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = (self.script)(request);
        Box::pin(async move { answer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lysilogy_core::{Visibility, WorkKind};
    use lysilogy_db::evidence::NewEntry;
    use lysilogy_db::work::WorkRepository;
    use sqlx::PgPool;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use tokio::sync::Mutex;

    fn item(title: &str, authors: &[&str], year: &str) -> FieldDraftItem {
        FieldDraftItem {
            index: 0,
            title: title.to_owned(),
            authors: authors.iter().map(|name| (*name).to_owned()).collect(),
            year: year.to_owned(),
        }
    }

    const SHOWN: &str = "P. Christiano, J. Leike, T. Brown. Deep reinforcement learning from human preferences. In NeurIPS, 2017.";

    #[test]
    fn should_drop_model_value_when_not_verbatim_in_entry_text() {
        let grounded = ground_field_draft(
            SHOWN,
            &item(
                "Deep RL from human preferences",
                &["P. Christiano", "J. Leike", "T. Browne"],
                "2018",
            ),
        );
        assert_eq!(grounded.title, None);
        assert_eq!(grounded.authors, None);
        assert_eq!(grounded.year, None);
        assert_eq!(
            grounded.dropped,
            vec![
                FieldDrop::TitleNotInEntry,
                FieldDrop::YearNotInEntry,
                FieldDrop::AuthorNotInEntry
            ]
        );

        let kept = ground_field_draft(
            SHOWN,
            &item(
                "Deep reinforcement learning from human preferences",
                &["P. Christiano", "J. Leike", "T. Brown"],
                "2017",
            ),
        );
        assert_eq!(
            kept.title.as_deref(),
            Some("Deep reinforcement learning from human preferences")
        );
        assert_eq!(kept.year, Some(2017));
        assert_eq!(kept.authors.map(|names| names.len()), Some(3));
        assert!(kept.dropped.is_empty());
    }

    #[test]
    fn should_keep_verbatim_values_when_one_other_value_is_dropped() {
        let grounded = ground_field_draft(
            SHOWN,
            &item("Invented title", &["P. Christiano", "J. Leike"], "2017"),
        );
        assert_eq!(grounded.title, None);
        assert_eq!(grounded.year, Some(2017));
        assert_eq!(grounded.authors.map(|names| names.len()), Some(2));
        assert_eq!(grounded.dropped, vec![FieldDrop::TitleNotInEntry]);
    }

    #[test]
    fn should_treat_empty_values_as_absent_when_grounding() {
        let grounded = ground_field_draft(SHOWN, &item("", &[], ""));
        assert_eq!(grounded, GroundedFields::default());
        let blank = ground_field_draft(SHOWN, &item("  ", &["", "  "], " "));
        assert_eq!(blank, GroundedFields::default());
    }

    #[test]
    fn should_drop_year_when_malformed_or_out_of_range() {
        let shown = "A. Author. Title words here. 1066 and 2300 and 20177 and 2017a and 99.";
        for printed in ["1066", "2300", "20177", "99", "2O17", "17a"] {
            let grounded = ground_field_draft(shown, &item("", &[], printed));
            assert_eq!(grounded.year, None, "{printed}");
            assert_eq!(
                grounded.dropped,
                vec![FieldDrop::YearMalformed],
                "{printed}"
            );
        }
        let suffixed = ground_field_draft(shown, &item("", &[], "2017a"));
        assert_eq!(suffixed.year, Some(2017));
    }

    #[test]
    fn should_ground_against_whitespace_collapsed_text_when_entry_has_line_breaks() {
        let raw = "P. Christiano,\n  J. Leike. Deep reinforcement\nlearning from human preferences.\n2017.";
        let shown = shown_text(raw, ENTRY_SHOWN_CHARS);
        assert!(!shown.contains('\n'));
        let grounded = ground_field_draft(
            &shown,
            &item(
                "Deep reinforcement\nlearning from human   preferences",
                &["P. Christiano", "J. Leike"],
                "2017",
            ),
        );
        assert_eq!(
            grounded.title.as_deref(),
            Some("Deep reinforcement learning from human preferences")
        );
        assert!(grounded.dropped.is_empty(), "{:?}", grounded.dropped);
    }

    #[test]
    fn should_cut_shown_text_on_a_char_boundary_when_entry_is_long() {
        let raw = "é".repeat(700);
        let shown = shown_text(&raw, ENTRY_SHOWN_CHARS);
        assert_eq!(shown.chars().count(), ENTRY_SHOWN_CHARS);
    }

    #[test]
    fn should_drop_unknown_index_when_model_answers_for_missing_entry() {
        let draft = FieldDraft {
            items: vec![
                FieldDraftItem {
                    index: 1,
                    ..item("A", &[], "")
                },
                FieldDraftItem {
                    index: 7,
                    ..item("B", &[], "")
                },
                FieldDraftItem {
                    index: -1,
                    ..item("C", &[], "")
                },
                FieldDraftItem {
                    index: 1,
                    ..item("D", &[], "")
                },
            ],
        };
        let (slots, unknown) = assign_draft_items(2, &draft);
        assert_eq!(unknown, 2);
        assert!(slots[0].is_none(), "the model skipped entry 0");
        assert_eq!(slots[1].as_ref().map(|slot| slot.title.as_str()), Some("A"));
    }

    #[test]
    fn should_drop_author_list_when_more_than_one_hundred_names() {
        let names: Vec<String> = (0..101).map(|number| format!("Author{number}")).collect();
        let shown = names.join(", ");
        let draft = FieldDraftItem {
            index: 0,
            title: String::new(),
            authors: names.clone(),
            year: String::new(),
        };
        let grounded = ground_field_draft(&shown, &draft);
        assert_eq!(grounded.authors, None);
        assert_eq!(grounded.dropped, vec![FieldDrop::TooManyAuthors]);
        let hundred = FieldDraftItem {
            authors: names[..100].to_vec(),
            ..draft
        };
        assert_eq!(
            ground_field_draft(&shown, &hundred)
                .authors
                .map(|kept| kept.len()),
            Some(100)
        );
    }

    #[test]
    fn should_change_cache_key_when_prompt_version_or_model_changes() {
        let base = field_cache_key("claude:haiku", SHOWN);
        assert_eq!(base, field_cache_key("claude:haiku", SHOWN));
        assert_ne!(base, field_cache_key("claude:default", SHOWN));
        assert_ne!(base, field_cache_key("claude:haiku", "other text"));
        assert!(
            base.starts_with(&format!("claude:haiku|v{FIELD_PROMPT_VERSION}|")),
            "{base}"
        );
        assert_eq!(base.rsplit('|').next().map(str::len), Some(64));
    }

    #[test]
    fn should_reject_extra_properties_when_schema_is_strict() {
        let schema: Value = serde_json::from_str(FIELD_SCHEMA).expect("schema is JSON");
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["required"], json!(["items"]));
        let element = &schema["properties"]["items"]["items"];
        assert_eq!(element["additionalProperties"], json!(false));
        assert_eq!(
            element["required"],
            json!(["index", "title", "authors", "year"])
        );
        let properties = element["properties"].as_object().expect("properties");
        assert_eq!(properties.len(), 4);
        for name in element["required"].as_array().expect("required") {
            assert!(properties.contains_key(name.as_str().expect("name")));
        }
    }

    #[test]
    fn should_ask_for_verbatim_copies_when_building_prompt() {
        let prompt = field_prompt(&FieldRequest {
            kind: FieldRequestKind::BibliographyEntries,
            items: vec!["first entry".to_owned(), "second entry".to_owned()],
        });
        assert!(prompt.contains("[0] first entry"));
        assert!(prompt.contains("[1] second entry"));
        assert!(prompt.contains("character for character"));
        let front = field_prompt(&FieldRequest {
            kind: FieldRequestKind::FrontMatter,
            items: vec!["front".to_owned()],
        });
        assert!(front.contains("this paper's own title"));
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static LOCK: Mutex<()> = Mutex::const_new(());

    fn unique() -> String {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        format!("x{}x{sequence}x{nanos}", std::process::id())
    }

    struct Harness {
        pool: PgPool,
        works: WorkRepository,
        claims: ClaimRepository,
        records: SourceRecordRepository,
        evidence: EvidenceRepository,
        marker: String,
        citing: WorkId,
        version: DocumentVersionId,
        record: SourceRecordId,
    }

    impl Harness {
        async fn new() -> Option<Self> {
            let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
                .ok()
                .filter(|url| !url.is_empty())?;
            let pool = lysilogy_db::connect(&url).await.expect("connect");
            lysilogy_db::migrate(&pool).await.expect("migrate");
            let works = WorkRepository::new(pool.clone());
            let records = SourceRecordRepository::new(pool.clone());
            let marker = unique();
            let citing = works
                .create(WorkKind::Article, Visibility::Private)
                .await
                .expect("citing")
                .id;
            let workspace: uuid::Uuid = sqlx::query_scalar(
                "insert into reader.workspace (name) values ('field-model-test') returning id",
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
                "insert into reader.document_version (document_id, sha256) \
                 values ($1, $2) returning id",
            )
            .bind(document)
            .bind(format!("sha-{marker}"))
            .fetch_one(&pool)
            .await
            .expect("version");
            let record = records
                .upsert(
                    "local_bibliography",
                    &format!("field-test|{marker}"),
                    Coverage::Complete,
                    &json!({}),
                )
                .await
                .expect("record");
            Some(Self {
                claims: ClaimRepository::new(pool.clone()),
                evidence: EvidenceRepository::new(pool.clone()),
                works,
                records,
                pool,
                marker,
                citing,
                version: DocumentVersionId::from_uuid(version),
                record,
            })
        }

        fn context(&self) -> FieldContext<'_> {
            FieldContext {
                claims: &self.claims,
                records: &self.records,
                evidence: &self.evidence,
            }
        }

        /// Entries "Author<n> <marker>. Title<n> <marker>. 2017." each resolved
        /// to its own fresh Work; returns the Works in ordinal order.
        async fn entries(&self, count: usize) -> Vec<WorkId> {
            let mut works = Vec::new();
            let mut new_entries = Vec::new();
            for number in 0..count {
                let work = self
                    .works
                    .create(WorkKind::Article, Visibility::Private)
                    .await
                    .expect("work")
                    .id;
                works.push(work);
                new_entries.push(NewEntry {
                    entry_key: format!("bib-v1:{number}"),
                    ordinal: i32::try_from(number).expect("small"),
                    raw_text: self.raw(number),
                    printed_key: None,
                    fields: json!({}),
                    page: 1,
                    text_start: 0,
                    text_end: 1,
                    member_spans: json!([]),
                    region: None,
                });
            }
            let ids = self
                .evidence
                .insert_entries(self.version, self.record, &new_entries)
                .await
                .expect("entries");
            for (id, work) in ids.into_iter().zip(&works) {
                self.evidence
                    .record_resolution(
                        id,
                        *work,
                        lysilogy_core::ResolutionStatus::Accepted,
                        lysilogy_core::ResolutionMethod::LocalStub,
                        None,
                    )
                    .await
                    .expect("resolution");
            }
            works
        }

        fn raw(&self, number: usize) -> String {
            format!(
                "Ada{number} Author{number}, Bo Writer. Title{number} of a study {}. Venue, 2017.",
                self.marker
            )
        }

        fn answer(&self, number: usize) -> FieldDraftItem {
            FieldDraftItem {
                index: i64::try_from(number).expect("small"),
                title: format!("Title{number} of a study {}", self.marker),
                authors: vec![
                    format!("Ada{number} Author{number}"),
                    "Bo Writer".to_owned(),
                ],
                year: "2017".to_owned(),
            }
        }

        async fn harvest(
            &self,
            extractor: &dyn FieldExtractor,
            options: FieldOptions,
        ) -> FieldHarvestSummary {
            harvest_bibliography_fields(
                &self.context(),
                self.citing,
                self.version,
                extractor,
                options,
            )
            .await
            .expect("harvest")
        }

        async fn claim_count(&self, works: &[WorkId]) -> usize {
            let mut total = 0;
            for work in works {
                total += self
                    .claims
                    .claims_for(*work)
                    .await
                    .expect("claims")
                    .iter()
                    .filter(|claim| claim.extractor == BIBLIOGRAPHY_MODEL_EXTRACTOR)
                    .count();
            }
            total
        }
    }

    /// Answers every numbered text of the request, in the model's own numbering.
    fn answer_all(
        marker: String,
    ) -> impl Fn(&FieldRequest) -> std::result::Result<FieldDraft, FieldExtractorError> {
        move |request| {
            let items = request
                .items
                .iter()
                .enumerate()
                .map(|(index, shown)| {
                    // The entry number is the digits after "Title" in the shown text.
                    let number: usize = shown
                        .split("Title")
                        .nth(1)
                        .and_then(|rest| rest.split(' ').next())
                        .and_then(|digits| digits.parse().ok())
                        .unwrap_or(0);
                    FieldDraftItem {
                        index: i64::try_from(index).expect("small"),
                        title: format!("Title{number} of a study {marker}"),
                        authors: vec![
                            format!("Ada{number} Author{number}"),
                            "Bo Writer".to_owned(),
                        ],
                        year: "2017".to_owned(),
                    }
                })
                .collect();
            Ok(FieldDraft { items })
        }
    }

    macro_rules! harness {
        () => {{
            let guard = LOCK.lock().await;
            let Some(harness) = Harness::new().await else {
                eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
                return;
            };
            (guard, harness)
        }};
    }

    #[tokio::test]
    async fn should_make_no_model_calls_when_rerun_hits_cache() {
        let (_guard, harness) = harness!();
        let works = harness.entries(3).await;
        let extractor =
            ScriptedFieldExtractor::new("test:model", answer_all(harness.marker.clone()));
        let first = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(first.model_calls, 1, "{first:?}");
        assert_eq!(first.cache_hits, 0);
        assert_eq!(
            first.claims_written, 9,
            "title, year, authors for 3 entries"
        );
        let claims = harness.claim_count(&works).await;
        assert_eq!(claims, 9);

        let second = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(second.model_calls, 0, "{second:?}");
        assert_eq!(second.cache_hits, 3);
        assert_eq!(second.claims_written, 0);
        assert_eq!(extractor.calls(), 1);
        assert_eq!(harness.claim_count(&works).await, claims);
    }

    #[tokio::test]
    async fn should_cache_omitted_entries_when_batch_answer_skips_them() {
        let (_guard, harness) = harness!();
        let works = harness.entries(3).await;
        let answer = harness.answer(1);
        let extractor = ScriptedFieldExtractor::new("test:model", move |_| {
            Ok(FieldDraft {
                items: vec![answer.clone()],
            })
        });
        let first = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(first.model_calls, 1);
        assert_eq!(first.claims_written, 3, "only entry 1 was answered");
        assert_eq!(harness.claim_count(&works).await, 3);
        let omitted = harness
            .records
            .get(
                BIBLIOGRAPHY_FIELDS_KIND,
                &field_cache_key(
                    "test:model",
                    &shown_text(&harness.raw(0), ENTRY_SHOWN_CHARS),
                ),
            )
            .await
            .expect("record");
        assert_eq!(omitted, Some(json!({ "omitted": true })));

        let second = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(second.model_calls, 0, "omissions are cached: {second:?}");
        assert_eq!(second.cache_hits, 3);
        assert_eq!(extractor.calls(), 1);
    }

    #[tokio::test]
    async fn should_batch_twenty_entries_per_call_and_stop_at_limit() {
        let (_guard, harness) = harness!();
        harness.entries(45).await;
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&sizes);
        let inner = answer_all(harness.marker.clone());
        let extractor = ScriptedFieldExtractor::new("test:model", move |request| {
            seen.lock().expect("sizes").push(request.items.len());
            inner(request)
        });
        let summary = harness
            .harvest(
                &extractor,
                FieldOptions {
                    limit: 2,
                    concurrency: 2,
                },
            )
            .await;
        assert_eq!(summary.model_calls, 2, "{summary:?}");
        assert_eq!(summary.over_limit, 5, "the third batch holds 5 entries");
        assert_eq!(summary.claims_written, 40 * 3);
        let mut sizes = sizes.lock().expect("sizes").clone();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![20, 20]);

        let rest = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(rest.model_calls, 1, "{rest:?}");
        assert_eq!(rest.cache_hits, 40);
        assert_eq!(rest.over_limit, 0);
    }

    #[tokio::test]
    async fn should_skip_entries_whose_work_has_higher_tier_authors_and_year() {
        let (_guard, harness) = harness!();
        let works = harness.entries(2).await;
        let names = FieldValue::Authors(vec!["Ada Author".to_owned()]);
        let year = FieldValue::Year(2017);
        let claims: Vec<NewClaim<'_>> = [&names, &year]
            .into_iter()
            .map(|value| NewClaim {
                work_id: works[0],
                field: value.field(),
                value,
                origin: ClaimOrigin::Openalex,
                extractor: "openalex",
                confidence: ClaimConfidence::Explicit,
                source_record_id: None,
                entry_id: None,
                locator: None,
            })
            .collect();
        harness.claims.record(&claims).await.expect("claims");
        // Authors alone (no year) at a higher tier does not skip entry 1.
        harness
            .claims
            .record(&[NewClaim {
                work_id: works[1],
                field: ProvenanceField::Authors,
                value: &names,
                origin: ClaimOrigin::Openalex,
                extractor: "openalex",
                confidence: ClaimConfidence::Explicit,
                source_record_id: None,
                entry_id: None,
                locator: None,
            }])
            .await
            .expect("authors only");
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked);
        let extractor = ScriptedFieldExtractor::new("test:model", move |request| {
            seen.lock().expect("asked").extend(request.items.clone());
            Ok(FieldDraft::default())
        });
        let summary = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(summary.skipped_higher_tier, 1, "{summary:?}");
        let asked = asked.lock().expect("asked").clone();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].contains("Title1"), "{asked:?}");
    }

    #[tokio::test]
    async fn should_write_model_claims_below_bib_v1_when_both_exist() {
        let (_guard, harness) = harness!();
        let works = harness.entries(1).await;
        let work = works[0];
        let heuristic =
            FieldValue::Authors(vec!["Ada0 Author0".to_owned(), "Bo Writer".to_owned()]);
        let entry = harness
            .evidence
            .resolved_entries(harness.version)
            .await
            .expect("entries")
            .remove(0)
            .0;
        harness
            .claims
            .record(&[NewClaim {
                work_id: work,
                field: ProvenanceField::Authors,
                value: &heuristic,
                origin: ClaimOrigin::CitingBibliography,
                extractor: "bib-v1",
                confidence: ClaimConfidence::Heuristic,
                source_record_id: Some(harness.record),
                entry_id: Some(entry.id),
                locator: Some("bib-v1:0"),
            }])
            .await
            .expect("bib-v1 claim");
        // The model reads a different (still verbatim) author list.
        let extractor = ScriptedFieldExtractor::new("test:model", |request| {
            let shown = &request.items[0];
            assert!(shown.contains("Bo Writer"));
            Ok(FieldDraft {
                items: vec![FieldDraftItem {
                    index: 0,
                    title: String::new(),
                    authors: vec!["Bo Writer".to_owned()],
                    year: String::new(),
                }],
            })
        });
        let summary = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(summary.claims_written, 1, "{summary:?}");
        let stored = harness.claims.claims_for(work).await.expect("claims");
        let model = stored
            .iter()
            .find(|claim| claim.extractor == BIBLIOGRAPHY_MODEL_EXTRACTOR)
            .expect("model claim");
        assert_eq!(model.confidence, ClaimConfidence::Model);
        assert_eq!(model.origin, ClaimOrigin::CitingBibliography);
        assert_eq!(model.entry_id, Some(entry.id));
        assert_eq!(model.locator.as_deref(), Some("bib-v1:0"));
        assert!(model.source_record_id.is_some());
        let work_row = harness.works.get(work).await.expect("get").expect("work");
        assert_eq!(
            work_row.display_authors,
            Some(vec!["Ada0 Author0".to_owned(), "Bo Writer".to_owned()]),
            "the heuristic claim wins"
        );
        assert_eq!(work_row.authors_state, lysilogy_core::FieldState::Known);
    }

    #[tokio::test]
    async fn should_count_failures_without_stopping_when_extractor_errors() {
        let (_guard, harness) = harness!();
        let works = harness.entries(45).await;
        let inner = answer_all(harness.marker.clone());
        let extractor = ScriptedFieldExtractor::new("test:model", move |request| {
            if request.items.len() == 20 && request.items[0].contains("Title0 ") {
                return Err(FieldExtractorError::Failed("boom".to_owned()));
            }
            inner(request)
        });
        let summary = harness
            .harvest(
                &extractor,
                FieldOptions {
                    limit: 10,
                    concurrency: 1,
                },
            )
            .await;
        assert_eq!(summary.model_calls, 3);
        assert_eq!(summary.failed, 1, "{summary:?}");
        assert_eq!(summary.claims_written, 25 * 3);
        assert_eq!(harness.claim_count(&works).await, 25 * 3);
        let again = harness.harvest(&extractor, FieldOptions::default()).await;
        assert_eq!(again.model_calls, 1, "only the failed batch is retried");
        assert_eq!(again.failed, 1);
    }

    #[tokio::test]
    async fn should_write_paper_model_claims_when_front_matter_lacks_deterministic_fields() {
        let (_guard, harness) = harness!();
        let paper = harness.citing;
        let front = format!(
            "Learning {m} Preferences\nAda Lovelace and Bo Writer\nUniversity of Nowhere\nPublished 2019.",
            m = harness.marker
        );
        let title = format!("Learning {} Preferences", harness.marker);
        let expected = title.clone();
        let extractor = ScriptedFieldExtractor::new("test:model", move |request| {
            assert_eq!(request.kind, FieldRequestKind::FrontMatter);
            assert_eq!(request.items.len(), 1);
            Ok(FieldDraft {
                items: vec![FieldDraftItem {
                    index: 0,
                    title: title.clone(),
                    authors: vec!["Ada Lovelace".to_owned(), "Bo Writer".to_owned()],
                    year: "2019".to_owned(),
                }],
            })
        });
        let first = harvest_front_matter_fields(&harness.context(), paper, &front, &extractor)
            .await
            .expect("front matter");
        assert_eq!(first.model_calls, 1, "{first:?}");
        assert_eq!(first.claims_written, 3);
        let claims = harness.claims.claims_for(paper).await.expect("claims");
        assert_eq!(claims.len(), 3);
        for claim in &claims {
            assert_eq!(claim.origin, ClaimOrigin::Paper);
            assert_eq!(claim.extractor, FRONT_MATTER_MODEL_EXTRACTOR);
            assert_eq!(claim.confidence, ClaimConfidence::Model);
            assert_eq!(claim.locator.as_deref(), Some("front_matter"));
        }
        let row = harness.works.get(paper).await.expect("get").expect("work");
        assert_eq!(row.display_title.as_deref(), Some(expected.as_str()));
        assert_eq!(row.display_year, Some(2019));

        let second = harvest_front_matter_fields(&harness.context(), paper, &front, &extractor)
            .await
            .expect("rerun");
        assert_eq!(second.model_calls, 0);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.claims_written, 0);
        assert_eq!(extractor.calls(), 1);
        let _ = &harness.pool;
    }
}
