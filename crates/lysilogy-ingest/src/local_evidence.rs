//! Persist a paper's local bibliography, resolve each entry to a Work, assert
//! the local citation edges, and store the in-text mentions.
//!
//! Extraction is evidence and is written once (see `EXTRACTOR_VERSION`).
//! Resolution is a ladder: a reused accepted resolution, then identifiers,
//! then an exact title key, then a stub Work. A stub that later coincides with
//! another Work is merged through `WorkRepository::merge_work`.

use std::collections::{BTreeSet, HashMap, HashSet};

use lysilogy_core::{
    Coverage, DocumentVersionId, FieldKind, FieldState, IdentifierScheme, IdentifierStatus,
    ResolutionMethod, ResolutionStatus, SourceRecordId, Visibility, Work, WorkId, WorkKind,
    identifier::normalize, title_key,
};
use lysilogy_db::DbError;
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::evidence::{EvidenceRepository, NewEntry, NewMention, StoredEntry};
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Result;
use crate::bibliography::{BibliographicFields, Bibliography, EXTRACTOR_VERSION, parse_fields};

/// Source record kind for the extraction evidence.
pub const LOCAL_BIBLIOGRAPHY_KIND: &str = "local_bibliography";

/// Actor recorded on automatic merges.
const MERGE_ACTOR: &str = "lit:auto";

/// Confidence recorded for an exact title-key match.
const TITLE_MATCH_CONFIDENCE: f64 = 0.9;

/// Title keys need at least this many words to be trusted for matching.
const MIN_TITLE_KEY_WORDS: usize = 3;

/// The repositories the flow works through.
pub struct LocalEvidenceContext<'a> {
    pub works: &'a WorkRepository,
    pub citations: &'a CitationRepository,
    pub records: &'a SourceRecordRepository,
    pub evidence: &'a EvidenceRepository,
}

/// One extracted bibliography to persist and resolve.
pub struct LocalEvidenceInput<'a> {
    pub citing_work: WorkId,
    pub document_version: DocumentVersionId,
    pub document_sha256: &'a str,
    pub index_schema_version: u16,
    pub index_text_sha256: &'a str,
    pub bibliography: &'a Bibliography,
}

/// How entries were resolved on this run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ResolutionCounts {
    pub identifier: usize,
    pub title_match: usize,
    pub local_stub: usize,
    /// Entries whose earlier accepted resolution was kept.
    pub reused: usize,
}

/// What one local-evidence run did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LocalEvidenceSummary {
    pub entries: usize,
    /// Mentions of resolved entries (stored now or on an earlier run).
    pub mentions: usize,
    /// Markers that reached no unique, resolved entry.
    pub unresolved_mentions: usize,
    pub resolved_entries: usize,
    pub resolutions: ResolutionCounts,
    pub ambiguous_entries: usize,
    pub merged_stubs: usize,
    /// Distinct cited Works asserted from the local bibliography.
    pub local_edges: usize,
    /// Local edges that the provider also asserts.
    pub edges_also_from_provider: usize,
    /// Stable `snake_case` codes, some with `:<entry number>` context.
    pub warnings: Vec<String>,
    /// Filled by the label pass.
    pub labels: Option<Value>,
}

/// How one entry came out.
#[derive(Debug)]
enum EntryOutcome {
    Resolved {
        work: WorkId,
        method: ResolutionMethod,
        created: bool,
        reused: bool,
        merged: bool,
    },
    Ambiguous(Vec<WorkId>),
    SelfReference,
}

/// The result of looking an entry up among existing Works.
#[derive(Debug)]
enum Lookup {
    Found {
        work: WorkId,
        method: ResolutionMethod,
        confidence: f64,
    },
    Ambiguous(Vec<WorkId>),
    SelfReference,
    Nothing,
}

/// Persist, resolve, and connect the local bibliography of one document.
pub async fn run_local_evidence(
    ctx: &LocalEvidenceContext<'_>,
    input: LocalEvidenceInput<'_>,
) -> Result<LocalEvidenceSummary> {
    let bibliography = input.bibliography;
    let mut summary = LocalEvidenceSummary {
        entries: bibliography.entries.len(),
        ..LocalEvidenceSummary::default()
    };
    ctx.works.backfill_title_keys().await?;
    if bibliography.entries.is_empty() {
        summary.unresolved_mentions = bibliography.unresolved.len();
        summary.warnings.push("no_bibliography_entries".to_owned());
        return Ok(summary);
    }

    let (record, drifted) = persist_extraction(ctx, &input).await?;
    let stored = stored_entries(ctx, &input, record).await?;
    let neighborhood = ctx.citations.outgoing(input.citing_work).await?;

    let mut assertions = HashMap::new();
    let mut cited_works = BTreeSet::new();
    for extracted in &bibliography.entries {
        let number = extracted.ordinal + 1;
        let Some(entry) = stored.get(&extracted.ordinal) else {
            summary.warnings.push(format!("entry_not_stored:{number}"));
            continue;
        };
        let fields = serde_json::from_value::<BibliographicFields>(entry.fields.clone())
            .unwrap_or_else(|_| {
                summary.warnings.push(format!("entry_fields_unreadable:{number}"));
                parse_fields(&entry.raw_text, entry.printed_key.clone())
            });
        let outcome = resolve_entry(
            ctx,
            input.citing_work,
            &neighborhood,
            entry,
            &fields,
            record,
            &mut summary.warnings,
        )
        .await?;
        match outcome {
            EntryOutcome::SelfReference => {
                summary.warnings.push(format!("self_reference:{number}"));
            }
            EntryOutcome::Ambiguous(candidates) => {
                summary.ambiguous_entries += 1;
                for candidate in candidates {
                    ctx.evidence
                        .record_resolution(
                            entry.id,
                            candidate,
                            ResolutionStatus::Ambiguous,
                            ResolutionMethod::TitleMatch,
                            Some(TITLE_MATCH_CONFIDENCE),
                        )
                        .await?;
                }
            }
            EntryOutcome::Resolved {
                work,
                method,
                created,
                reused,
                merged,
            } => {
                count_resolution(&mut summary, method, created, reused, merged);
                let assertion = ctx
                    .citations
                    .add_entry_assertion(input.citing_work, work, entry.id)
                    .await?;
                assertions.insert((extracted.ordinal, entry.id), assertion);
                cited_works.insert(work);
            }
        }
    }

    summary.local_edges = cited_works.len();
    let pairs: Vec<(WorkId, WorkId)> = cited_works
        .iter()
        .map(|cited| (input.citing_work, *cited))
        .collect();
    let sources = ctx.citations.edge_sources(&pairs).await?;
    summary.edges_also_from_provider = sources.values().filter(|edge| edge.provider).count();

    summary.unresolved_mentions = bibliography.unresolved.len();
    if drifted {
        summary.warnings.push("index_text_changed".to_owned());
        summary.unresolved_mentions += bibliography.mentions.len();
        return Ok(summary);
    }
    store_mentions(ctx, &input, &stored, &assertions, &mut summary).await?;
    Ok(summary)
}

const fn count_resolution(
    summary: &mut LocalEvidenceSummary,
    method: ResolutionMethod,
    created: bool,
    reused: bool,
    merged: bool,
) {
    summary.resolved_entries += 1;
    if merged {
        summary.merged_stubs += 1;
    }
    if reused {
        summary.resolutions.reused += 1;
        return;
    }
    match method {
        ResolutionMethod::Identifier => summary.resolutions.identifier += 1,
        ResolutionMethod::TitleMatch | ResolutionMethod::Merge => {
            summary.resolutions.title_match += 1;
        }
        ResolutionMethod::LocalStub => {
            if created {
                summary.resolutions.local_stub += 1;
            } else {
                summary.resolutions.reused += 1;
            }
        }
    }
}

/// Upsert the `local_bibliography` record and store the entries once. Returns
/// the record and whether the stored extraction was made from different index
/// text than the one in hand.
async fn persist_extraction(
    ctx: &LocalEvidenceContext<'_>,
    input: &LocalEvidenceInput<'_>,
) -> Result<(SourceRecordId, bool)> {
    let source_key = format!("{}|{EXTRACTOR_VERSION}", input.document_sha256);
    if let Some((record, payload)) = ctx.records.find(LOCAL_BIBLIOGRAPHY_KIND, &source_key).await? {
        let stored_hash = payload.get("index_text_sha256").and_then(Value::as_str);
        return Ok((record, stored_hash != Some(input.index_text_sha256)));
    }
    let payload = json!({
        "extractor": EXTRACTOR_VERSION,
        "index_schema_version": input.index_schema_version,
        "index_text_sha256": input.index_text_sha256,
        "entry_count": input.bibliography.entries.len(),
        "unresolved": input.bibliography.unresolved,
    });
    let record = ctx
        .records
        .upsert(LOCAL_BIBLIOGRAPHY_KIND, &source_key, Coverage::Complete, &payload)
        .await?;
    Ok((record, false))
}

/// The record's entries by ordinal, inserting them when missing.
async fn stored_entries(
    ctx: &LocalEvidenceContext<'_>,
    input: &LocalEvidenceInput<'_>,
    record: SourceRecordId,
) -> Result<HashMap<usize, StoredEntry>> {
    let mut stored = ctx.evidence.entries_for_record(record).await?;
    if stored.len() < input.bibliography.entries.len() {
        let new_entries = new_entries(input.bibliography);
        ctx.evidence
            .insert_entries(input.document_version, record, &new_entries)
            .await?;
        stored = ctx.evidence.entries_for_record(record).await?;
    }
    Ok(stored
        .into_iter()
        .filter_map(|entry| {
            let ordinal = usize::try_from(entry.ordinal?).ok()?;
            Some((ordinal, entry))
        })
        .collect())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn new_entries(bibliography: &Bibliography) -> Vec<NewEntry> {
    bibliography
        .entries
        .iter()
        .map(|entry| NewEntry {
            entry_key: format!("{EXTRACTOR_VERSION}:{}", entry.ordinal + 1),
            ordinal: entry.ordinal as i32,
            raw_text: entry.raw_text.clone(),
            printed_key: entry.fields.printed_key.value.clone(),
            fields: json!(entry.fields),
            page: entry.page as i32,
            text_start: entry.span.start as i32,
            text_end: entry.span.end as i32,
            member_spans: json!(entry.member_spans),
            region: entry.region.map(|region| json!(region)),
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
async fn store_mentions(
    ctx: &LocalEvidenceContext<'_>,
    input: &LocalEvidenceInput<'_>,
    stored: &HashMap<usize, StoredEntry>,
    assertions: &HashMap<(usize, lysilogy_core::EntryId), lysilogy_core::AssertionId>,
    summary: &mut LocalEvidenceSummary,
) -> Result<()> {
    for mention in &input.bibliography.mentions {
        let linked = stored.get(&mention.entry_ordinal).and_then(|entry| {
            assertions
                .get(&(mention.entry_ordinal, entry.id))
                .map(|assertion| (entry.id, *assertion))
        });
        let Some((entry, assertion)) = linked else {
            summary.unresolved_mentions += 1;
            continue;
        };
        ctx.evidence
            .insert_mention(&NewMention {
                assertion,
                version: input.document_version,
                entry,
                style: mention.style,
                marker_text: mention.marker_text.clone(),
                page: mention.page as i32,
                marker_start: mention.marker.start as i32,
                marker_end: mention.marker.end as i32,
                passage_start: mention.passage.start as i32,
                passage_end: mention.passage.end as i32,
                passage: mention.passage_text.clone(),
                rects: Some(json!(mention.rects)),
            })
            .await?;
        summary.mentions += 1;
    }
    Ok(())
}

/// Resolve one stored entry to a Work, creating a stub when nothing matches.
async fn resolve_entry(
    ctx: &LocalEvidenceContext<'_>,
    citing: WorkId,
    neighborhood: &[Work],
    entry: &StoredEntry,
    fields: &BibliographicFields,
    record: SourceRecordId,
    warnings: &mut Vec<String>,
) -> Result<EntryOutcome> {
    let number = entry.ordinal.map_or(0, |ordinal| ordinal + 1);
    let stub = match ctx.evidence.accepted_resolution(entry.id).await? {
        Some((accepted, method)) => {
            let work = ctx.works.resolve_redirect(accepted).await?;
            if method != ResolutionMethod::LocalStub {
                return Ok(EntryOutcome::Resolved {
                    work,
                    method,
                    created: false,
                    reused: true,
                    merged: false,
                });
            }
            Some(work)
        }
        None => None,
    };

    let found = lookup_entry(ctx, citing, neighborhood, fields, stub).await?;
    if let Some(stub) = stub {
        let reused = EntryOutcome::Resolved {
            work: stub,
            method: ResolutionMethod::LocalStub,
            created: false,
            reused: true,
            merged: false,
        };
        let Lookup::Found { work, method, .. } = found else {
            return Ok(reused);
        };
        if !ctx.works.is_auto_mergeable_stub(stub).await? {
            return Ok(reused);
        }
        let reason = format!("local bibliography entry {number} matches an existing Work");
        return match ctx.works.merge_work(stub, work, &reason, MERGE_ACTOR).await {
            Ok(()) => Ok(EntryOutcome::Resolved {
                work,
                method,
                created: false,
                reused: false,
                merged: true,
            }),
            Err(DbError::Conflict(_)) => {
                warnings.push(format!("merge_refused:{number}:conflict"));
                Ok(reused)
            }
            Err(error) => Err(error.into()),
        };
    }

    match found {
        Lookup::SelfReference => Ok(EntryOutcome::SelfReference),
        Lookup::Ambiguous(candidates) => Ok(EntryOutcome::Ambiguous(candidates)),
        Lookup::Found {
            work,
            method,
            confidence,
        } => {
            ctx.evidence
                .record_resolution(entry.id, work, ResolutionStatus::Accepted, method, Some(confidence))
                .await?;
            if method == ResolutionMethod::TitleMatch {
                attach_identifiers(ctx, work, fields, record, number, warnings).await?;
            }
            Ok(EntryOutcome::Resolved {
                work,
                method,
                created: false,
                reused: false,
                merged: false,
            })
        }
        Lookup::Nothing => {
            let work = create_stub(ctx, fields, record).await?;
            ctx.evidence
                .record_resolution(
                    entry.id,
                    work,
                    ResolutionStatus::Accepted,
                    ResolutionMethod::LocalStub,
                    None,
                )
                .await?;
            Ok(EntryOutcome::Resolved {
                work,
                method: ResolutionMethod::LocalStub,
                created: true,
                reused: false,
                merged: false,
            })
        }
    }
}

/// The entry's DOI, then arXiv ID, as normalized `(scheme, value)` pairs.
fn entry_identifiers(fields: &BibliographicFields) -> Vec<(IdentifierScheme, String)> {
    [
        (IdentifierScheme::Doi, fields.doi.value.as_deref()),
        (IdentifierScheme::Arxiv, fields.arxiv_id.value.as_deref()),
    ]
    .into_iter()
    .filter_map(|(scheme, raw)| Some((scheme, normalize(scheme, raw?)?)))
    .collect()
}

/// The leading four-digit year of a printed year such as `2020a`.
fn entry_year(fields: &BibliographicFields) -> Option<i32> {
    let printed = fields.year.value.as_deref()?.trim();
    let digits = printed.get(..4)?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Find an existing Work for the entry by identifier, then by exact title key.
/// `exclude` is a stub being re-examined, which may not match itself.
async fn lookup_entry(
    ctx: &LocalEvidenceContext<'_>,
    citing: WorkId,
    neighborhood: &[Work],
    fields: &BibliographicFields,
    exclude: Option<WorkId>,
) -> Result<Lookup> {
    for (scheme, value) in entry_identifiers(fields) {
        let Some(found) = ctx.works.find_by_identifier(scheme, &value).await? else {
            continue;
        };
        let work = ctx.works.resolve_redirect(found.id).await?;
        if Some(work) == exclude {
            continue;
        }
        if work == citing {
            return Ok(Lookup::SelfReference);
        }
        return Ok(Lookup::Found {
            work,
            method: ResolutionMethod::Identifier,
            confidence: 1.0,
        });
    }

    let Some(title) = fields.title.value.as_deref() else {
        return Ok(Lookup::Nothing);
    };
    let key = title_key(title);
    if key.split_whitespace().count() < MIN_TITLE_KEY_WORDS {
        return Ok(Lookup::Nothing);
    }
    let global = ctx.works.find_by_title_key(&key, exclude).await?;
    let near: Vec<Work> = neighborhood
        .iter()
        .filter(|work| Some(work.id) != exclude)
        .cloned()
        .collect();
    let mut candidates = title_candidates(&near, &global, &key, entry_year(fields));
    let cites_itself = candidates.iter().any(|work| work.id == citing);
    candidates.retain(|work| work.id != citing);
    if candidates.is_empty() {
        return Ok(if cites_itself {
            Lookup::SelfReference
        } else {
            Lookup::Nothing
        });
    }
    let near_ids: HashSet<WorkId> = near.iter().map(|work| work.id).collect();
    let openalex = if candidates.len() > 1 {
        let ids: Vec<WorkId> = candidates.iter().map(|work| work.id).collect();
        ctx.works
            .openalex_ids_for(&ids)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    } else {
        HashSet::new()
    };
    Ok(match select_candidate(&candidates, &near_ids, &openalex) {
        Ok(work) => Lookup::Found {
            work,
            method: ResolutionMethod::TitleMatch,
            confidence: TITLE_MATCH_CONFIDENCE,
        },
        Err(pool) => Lookup::Ambiguous(pool),
    })
}

/// Works whose title key equals `key` and whose year is within one of `year`
/// when both are known. Neighborhood matches come first; duplicates are
/// dropped.
#[must_use]
pub fn title_candidates(
    neighborhood: &[Work],
    global: &[Work],
    key: &str,
    year: Option<i32>,
) -> Vec<Work> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let near = neighborhood.iter().filter(|work| {
        work.display_title
            .as_deref()
            .is_some_and(|title| title_key(title) == key)
    });
    for work in near.chain(global) {
        if !seen.insert(work.id) {
            continue;
        }
        let years_agree = match (year, work.display_year) {
            (Some(entry), Some(known)) => (entry - known).abs() <= 1,
            _ => true,
        };
        if years_agree {
            candidates.push(work.clone());
        }
    }
    candidates
}

/// Pick the one candidate that wins: the only one, else the only neighborhood
/// one, else the only one holding an `OpenAlex` ID. `Err` carries the pool
/// that stayed ambiguous.
fn select_candidate(
    candidates: &[Work],
    neighborhood: &HashSet<WorkId>,
    openalex: &HashSet<WorkId>,
) -> std::result::Result<WorkId, Vec<WorkId>> {
    if let [only] = candidates {
        return Ok(only.id);
    }
    let near: Vec<WorkId> = candidates
        .iter()
        .map(|work| work.id)
        .filter(|id| neighborhood.contains(id))
        .collect();
    if let [only] = near.as_slice() {
        return Ok(*only);
    }
    let pool: Vec<WorkId> = if near.is_empty() {
        candidates.iter().map(|work| work.id).collect()
    } else {
        near
    };
    let with_openalex: Vec<WorkId> = pool
        .iter()
        .copied()
        .filter(|id| openalex.contains(id))
        .collect();
    match with_openalex.as_slice() {
        [only] => Ok(*only),
        _ => Err(pool),
    }
}

/// Attach the entry's identifiers to a matched Work, best effort.
async fn attach_identifiers(
    ctx: &LocalEvidenceContext<'_>,
    work: WorkId,
    fields: &BibliographicFields,
    record: SourceRecordId,
    number: i32,
    warnings: &mut Vec<String>,
) -> Result<()> {
    for (scheme, value) in entry_identifiers(fields) {
        if let Some(owner) = ctx.works.find_by_identifier(scheme, &value).await? {
            if owner.id != work {
                warnings.push(format!("identifier_conflict:{number}"));
            }
            continue;
        }
        let added = ctx
            .works
            .add_identifier(work, scheme, &value, Some(record), IdentifierStatus::Active)
            .await?;
        if added.is_none() {
            warnings.push(format!("identifier_conflict:{number}"));
        }
    }
    Ok(())
}

/// Create a stub Work for the entry, titled from its parsed fields.
async fn create_stub(
    ctx: &LocalEvidenceContext<'_>,
    fields: &BibliographicFields,
    record: SourceRecordId,
) -> Result<WorkId> {
    let work = match entry_identifiers(fields).into_iter().next() {
        Some((scheme, value)) => {
            ctx.works
                .resolve_or_create(scheme, &value, WorkKind::Article, Some(record))
                .await?
                .0
        }
        None => {
            ctx.works
                .create(WorkKind::Article, Visibility::Private)
                .await?
        }
    };
    if let Some(title) = fields
        .title
        .value
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        ctx.works
            .add_claim_once(
                work.id,
                FieldKind::Title,
                &json!(title),
                FieldState::Known,
                Some(record),
                None,
            )
            .await?;
        ctx.works.fill_display_title_if_unknown(work.id, title).await?;
    }
    if let Some(year) = entry_year(fields) {
        ctx.works
            .add_claim_once(
                work.id,
                FieldKind::Year,
                &json!(year),
                FieldState::Known,
                Some(record),
                None,
            )
            .await?;
        ctx.works.fill_display_year_if_unknown(work.id, year).await?;
    }
    Ok(work.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bibliography::{extract, tests::fixture};
        use sqlx::PgPool;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::Mutex;

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
        citations: CitationRepository,
        records: SourceRecordRepository,
        evidence: EvidenceRepository,
        marker: String,
        citing: Work,
        version: DocumentVersionId,
        sha: String,
    }

    impl Harness {
        async fn new() -> Option<Self> {
            let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
                .ok()
                .filter(|url| !url.is_empty())?;
            let pool = lysilogy_db::connect(&url).await.expect("connect");
            lysilogy_db::migrate(&pool).await.expect("migrate");
            let works = WorkRepository::new(pool.clone());
            let marker = unique();
            let citing = works
                .create(WorkKind::Article, Visibility::Private)
                .await
                .expect("citing");
            let sha = format!("sha-{marker}");
            let workspace: uuid::Uuid = sqlx::query_scalar(
                "insert into reader.workspace (name) values ('local-evidence-test') returning id",
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
            .bind(&sha)
            .fetch_one(&pool)
            .await
            .expect("version");
            Some(Self {
                works,
                citations: CitationRepository::new(pool.clone()),
                records: SourceRecordRepository::new(pool.clone()),
                evidence: EvidenceRepository::new(pool.clone()),
                pool,
                marker,
                citing,
                version: DocumentVersionId::from_uuid(version),
                sha,
            })
        }

        async fn run(&self, bibliography: &Bibliography) -> LocalEvidenceSummary {
            let context = LocalEvidenceContext {
                works: &self.works,
                citations: &self.citations,
                records: &self.records,
                evidence: &self.evidence,
            };
            run_local_evidence(
                &context,
                LocalEvidenceInput {
                    citing_work: self.citing.id,
                    document_version: self.version,
                    document_sha256: &self.sha,
                    index_schema_version: 1,
                    index_text_sha256: "index-hash",
                    bibliography,
                },
            )
            .await
            .expect("local evidence")
        }

        /// Title words unique to this test run, so title keys never collide
        /// with other tests or earlier runs.
        fn title(&self, words: &str) -> String {
            format!("{words} {}", self.marker)
        }

        async fn work_with_title(&self, title: &str, year: Option<i32>) -> Work {
            let work = self
                .works
                .create(WorkKind::Article, Visibility::Private)
                .await
                .expect("work");
            self.works.set_display_title(work.id, title).await.expect("title");
            if let Some(year) = year {
                self.works.set_display_year(work.id, year).await.expect("year");
            }
            self.works.get(work.id).await.expect("get").expect("exists")
        }

        async fn count(&self, sql: &str) -> i64 {
            sqlx::query_scalar(sql)
                .bind(self.citing.id.as_uuid())
                .fetch_one(&self.pool)
                .await
                .expect("count")
        }

        async fn outgoing(&self) -> Vec<Work> {
            self.citations.outgoing(self.citing.id).await.expect("outgoing")
        }
    }

    fn numbered(entries: &[String]) -> Bibliography {
        let body = "Prior work [1] is relevant. Later work [2] agrees. See also [1] again.";
        let list = entries.join("\n");
        let index = fixture(&[
            (body, "body", 1),
            ("References", "heading", 2),
            (&list, "body", 2),
        ]);
        extract(&index)
    }

    fn entry(number: usize, text: &str) -> String {
        format!("[{number}] {text}")
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

    #[test]
    fn should_order_neighborhood_first_when_selecting_title_candidates() {
        let make = |title: &str, year: Option<i32>| Work {
            id: WorkId::new(),
            kind: WorkKind::Article,
            visibility: Visibility::Private,
            display_title: Some(title.to_owned()),
            title_state: FieldState::Known,
            display_year: year,
            year_state: FieldState::Known,
            created_at: chrono_now(),
            updated_at: chrono_now(),
        };
        let key = title_key("Deep Residual Learning");
        let near = make("Deep Residual Learning", Some(2016));
        let far = make("Deep Residual Learning", Some(2015));
        let wrong_year = make("Deep Residual Learning", Some(2010));
        let other = make("Something Else Entirely", Some(2016));
        let global = vec![far.clone(), near.clone(), wrong_year];
        let candidates = title_candidates(&[other, near.clone()], &global, &key, Some(2016));
        let ids: Vec<WorkId> = candidates.iter().map(|work| work.id).collect();
        assert_eq!(ids, vec![near.id, far.id]);
    }

    #[test]
    fn should_keep_all_years_when_entry_year_is_unknown() {
        let work = Work {
            id: WorkId::new(),
            kind: WorkKind::Article,
            visibility: Visibility::Private,
            display_title: Some("A B C".to_owned()),
            title_state: FieldState::Known,
            display_year: Some(1990),
            year_state: FieldState::Known,
            created_at: chrono_now(),
            updated_at: chrono_now(),
        };
        let found = title_candidates(&[], std::slice::from_ref(&work), "a b c", None);
        assert_eq!(found.len(), 1);
    }

    fn chrono_now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    #[test]
    fn should_read_leading_year_digits_when_year_has_suffix_or_is_malformed() {
        let fields = |year: &str| {
            let mut fields = parse_fields("Smith. A title here. 2020.", None);
            fields.year.value = Some(year.to_owned());
            fields
        };
        assert_eq!(entry_year(&fields("2020a")), Some(2020));
        assert_eq!(entry_year(&fields("20")), None);
        assert_eq!(entry_year(&fields("abcd")), None);
        assert_eq!(entry_year(&fields("é020")), None);
    }

    #[tokio::test]
    async fn should_create_entries_mentions_and_local_edges_when_paper_has_numbered_bibliography() {
        let (_guard, h) = harness!();
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {}. 2020.", h.title("First numbered result"))),
            entry(2, &format!("Baker. {}. 2021.", h.title("Second numbered result"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.entries, 2, "{summary:?}");
        assert_eq!(summary.mentions, 3, "{summary:?}");
        assert_eq!(summary.unresolved_mentions, 0);
        assert_eq!(summary.resolved_entries, 2);
        assert_eq!(summary.resolutions.local_stub, 2);
        assert_eq!(summary.local_edges, 2);
        assert_eq!(summary.edges_also_from_provider, 0);
        assert_eq!(h.outgoing().await.len(), 2);
        let mentions = h
            .count(
                "select count(*) from graph.citation_mention m \
                 join graph.citation_assertion a on a.id = m.assertion_id \
                 where a.citing_work_id = $1",
            )
            .await;
        assert_eq!(mentions, 3);
        let record = h
            .records
            .get(LOCAL_BIBLIOGRAPHY_KIND, &format!("{}|bib-v1", h.sha))
            .await
            .expect("record")
            .expect("payload");
        assert_eq!(record["entry_count"], 2);
        assert_eq!(record["extractor"], "bib-v1");
    }

    #[tokio::test]
    async fn should_be_idempotent_when_local_evidence_runs_twice() {
        let (_guard, h) = harness!();
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {}. 2020.", h.title("Idempotent first study"))),
            entry(2, &format!("Baker. {}. 2021.", h.title("Idempotent second study"))),
        ]);
        let first = h.run(&bibliography).await;
        let works_before = h.count("select count(*) from graph.work where $1::uuid is not null").await;
        let second = h.run(&bibliography).await;
        let works_after = h.count("select count(*) from graph.work where $1::uuid is not null").await;
        assert_eq!(works_before, works_after, "no new Works");
        assert_eq!(first.entries, second.entries);
        assert_eq!(first.mentions, second.mentions);
        assert_eq!(first.local_edges, second.local_edges);
        assert_eq!(second.resolutions.reused, second.resolved_entries, "{second:?}");
        assert_eq!(second.resolutions.local_stub, 0);
        assert_eq!(second.merged_stubs, 0);
        assert_eq!(
            h.count(
                "select count(*) from graph.citation_assertion \
                 where citing_work_id = $1 and entry_id is not null"
            )
            .await,
            2
        );
        let entries: i64 = sqlx::query_scalar(
            "select count(*) from source.bibliography_entry where citing_version_id = $1",
        )
        .bind(h.version.as_uuid())
        .fetch_one(&h.pool)
        .await
        .expect("entries");
        assert_eq!(entries, 2);
    }

    #[tokio::test]
    async fn should_resolve_by_doi_when_entry_prints_identifier_of_existing_work() {
        let (_guard, h) = harness!();
        let doi = format!("10.5555/{}", h.marker);
        let (existing, _) = h
            .works
            .resolve_or_create(IdentifierScheme::Doi, &doi, WorkKind::Article, None)
            .await
            .expect("existing");
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {}. 2020. doi:{doi}", h.title("Doi keyed study"))),
            entry(2, &format!("Baker. {}. 2021.", h.title("Another keyed study"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.resolutions.identifier, 1, "{summary:?}");
        assert!(h.outgoing().await.iter().any(|work| work.id == existing.id));
    }

    #[tokio::test]
    async fn should_reuse_provider_stub_when_entry_title_matches_neighborhood_work() {
        let (_guard, h) = harness!();
        let title = h.title("Provider stub neighborhood paper");
        let provider = h.work_with_title(&title, Some(2020)).await;
        let record = h
            .records
            .upsert("openalex_work", &format!("prov-{}", h.marker), Coverage::Complete, &json!({}))
            .await
            .expect("record");
        h.citations
            .add_assertion(h.citing.id, provider.id, Some(record))
            .await
            .expect("provider edge");
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {title}. 2020.")),
            entry(2, &format!("Baker. {}. 2021.", h.title("Unrelated second paper"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.resolutions.title_match, 1, "{summary:?}");
        assert_eq!(summary.local_edges, 2);
        assert_eq!(summary.edges_also_from_provider, 1);
        let outgoing = h.outgoing().await;
        assert!(outgoing.iter().any(|work| work.id == provider.id));
        assert_eq!(outgoing.len(), 2, "the provider stub was reused, not duplicated");
    }

    #[tokio::test]
    async fn should_mark_ambiguous_when_two_unrelated_works_share_the_title_key() {
        let (_guard, h) = harness!();
        let title = h.title("Ambiguous shared title paper");
        let first = h.work_with_title(&title, Some(2020)).await;
        let second = h.work_with_title(&title, Some(2020)).await;
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {title}. 2020.")),
            entry(2, &format!("Baker. {}. 2021.", h.title("Plain second paper"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.ambiguous_entries, 1, "{summary:?}");
        assert_eq!(summary.resolved_entries, 1);
        let outgoing = h.outgoing().await;
        assert!(outgoing.iter().all(|work| work.id != first.id && work.id != second.id));
        let ambiguous: i64 = sqlx::query_scalar(
            "select count(*) from graph.entry_resolution \
             where status = 'ambiguous' and candidate_work_id = any($1)",
        )
        .bind(vec![first.id.as_uuid(), second.id.as_uuid()])
        .fetch_one(&h.pool)
        .await
        .expect("count");
        assert_eq!(ambiguous, 2);
        assert_eq!(summary.mentions, 1, "only the resolved entry keeps its mention");
        assert_eq!(summary.unresolved_mentions, 2);
    }

    #[tokio::test]
    async fn should_reject_title_match_when_years_differ_by_more_than_one() {
        let (_guard, h) = harness!();
        let title = h.title("Far apart years paper");
        let old = h.work_with_title(&title, Some(2010)).await;
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {title}. 2020.")),
            entry(2, &format!("Baker. {}. 2021.", h.title("Filler second paper"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.resolutions.title_match, 0, "{summary:?}");
        assert_eq!(summary.resolutions.local_stub, 2);
        assert!(h.outgoing().await.iter().all(|work| work.id != old.id));
    }

    #[tokio::test]
    async fn should_create_titled_stub_when_nothing_matches() {
        let (_guard, h) = harness!();
        let title = h.title("Brand new stub paper");
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {title}. 2020.")),
            entry(2, &format!("Baker. {}. 2021.", h.title("Another new stub paper"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.resolutions.local_stub, 2, "{summary:?}");
        let outgoing = h.outgoing().await;
        let stub = outgoing
            .iter()
            .find(|work| work.display_title.as_deref() == Some(title.as_str()))
            .expect("titled stub");
        assert_eq!(stub.display_year, Some(2020));
        assert_eq!(stub.title_state, FieldState::Known);
        let claims: i64 = sqlx::query_scalar(
            "select count(*) from graph.field_claim where work_id = $1 and field = 'title'",
        )
        .bind(stub.id.as_uuid())
        .fetch_one(&h.pool)
        .await
        .expect("claims");
        assert_eq!(claims, 1);
    }

    #[tokio::test]
    async fn should_merge_local_stub_when_provider_work_with_same_title_appears_later() {
        let (_guard, h) = harness!();
        let title = h.title("Later provider appearance paper");
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {title}. 2020.")),
            entry(2, &format!("Baker. {}. 2021.", h.title("Stable second paper"))),
        ]);
        let first = h.run(&bibliography).await;
        assert_eq!(first.resolutions.local_stub, 2, "{first:?}");
        let stub = h
            .outgoing()
            .await
            .into_iter()
            .find(|work| work.display_title.as_deref() == Some(title.as_str()))
            .expect("stub");

        let provider = h.work_with_title(&title, Some(2020)).await;
        let record = h
            .records
            .upsert("openalex_work", &format!("late-{}", h.marker), Coverage::Complete, &json!({}))
            .await
            .expect("record");
        h.citations
            .add_assertion(h.citing.id, provider.id, Some(record))
            .await
            .expect("provider edge");

        let second = h.run(&bibliography).await;
        assert_eq!(second.merged_stubs, 1, "{second:?}");
        let redirect: Option<uuid::Uuid> = sqlx::query_scalar(
            "select new_work_id from graph.work_redirect where old_work_id = $1",
        )
        .bind(stub.id.as_uuid())
        .fetch_optional(&h.pool)
        .await
        .expect("redirect");
        assert_eq!(redirect, Some(provider.id.as_uuid()));
        let decisions: i64 = sqlx::query_scalar(
            "select count(*) from graph.resolution_decision \
             where kind = 'merge' and subject_ids = $1",
        )
        .bind(vec![stub.id.as_uuid(), provider.id.as_uuid()])
        .fetch_one(&h.pool)
        .await
        .expect("decisions");
        assert_eq!(decisions, 1);
        let outgoing = h.outgoing().await;
        assert!(outgoing.iter().all(|work| work.id != stub.id));
        assert!(outgoing.iter().any(|work| work.id == provider.id));
        let entry_edges = h
            .count(
                "select count(*) from graph.citation_assertion \
                 where citing_work_id = $1 and entry_id is not null",
            )
            .await;
        assert_eq!(entry_edges, 2, "local assertion repointed, not duplicated");
        let third = h.run(&bibliography).await;
        assert_eq!(third.merged_stubs, 0);
        assert_eq!(third.resolutions.reused, third.resolved_entries, "{third:?}");
    }

    #[tokio::test]
    async fn should_skip_self_reference_when_entry_resolves_to_citing_work() {
        let (_guard, h) = harness!();
        let doi = format!("10.5556/{}", h.marker);
        h.works
            .add_identifier(h.citing.id, IdentifierScheme::Doi, &doi, None, IdentifierStatus::Active)
            .await
            .expect("identifier");
        let bibliography = numbered(&[
            entry(1, &format!("Adams. {}. 2020. doi:{doi}", h.title("Citing itself paper"))),
            entry(2, &format!("Baker. {}. 2021.", h.title("Honest second paper"))),
        ]);
        let summary = h.run(&bibliography).await;
        assert!(
            summary.warnings.contains(&"self_reference:1".to_owned()),
            "{summary:?}"
        );
        assert_eq!(summary.resolved_entries, 1);
        assert!(h.outgoing().await.iter().all(|work| work.id != h.citing.id));
    }

    #[tokio::test]
    async fn should_store_no_mentions_when_markers_are_unresolved() {
        let (_guard, h) = harness!();
        let index = fixture(&[
            ("Claims [7] and [8] have no targets.", "body", 1),
            ("References", "heading", 2),
            (
                &format!(
                    "[1] Adams. {}. 2020.\n[2] Baker. {}. 2021.",
                    h.title("Untargeted first paper"),
                    h.title("Untargeted second paper")
                ),
                "body",
                2,
            ),
        ]);
        let bibliography = extract(&index);
        let summary = h.run(&bibliography).await;
        assert_eq!(summary.mentions, 0, "{summary:?}");
        assert_eq!(summary.unresolved_mentions, 2);
        assert_eq!(summary.resolved_entries, 2);
        let stored = h
            .records
            .get(LOCAL_BIBLIOGRAPHY_KIND, &format!("{}|bib-v1", h.sha))
            .await
            .expect("record")
            .expect("payload");
        assert_eq!(stored["unresolved"].as_array().map(Vec::len), Some(2));
    }

    #[tokio::test]
    async fn should_return_warning_when_bibliography_has_no_entries() {
        let (_guard, h) = harness!();
        let summary = h.run(&Bibliography::default()).await;
        assert_eq!(summary.entries, 0);
        assert_eq!(summary.warnings, vec!["no_bibliography_entries".to_owned()]);
    }
}
