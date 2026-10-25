//! Claim verification: gather evidence for a claimed citation, decide it with
//! [`lysilogy_core::decide_claim`], and turn a verified claim into a Work.
//!
//! Gathering is the only I/O. Local Works come from the title key, identifiers from
//! `OpenAlex` then Crossref, and the title search from `OpenAlex`. Every provider response
//! goes through the Postgres response cache, so a repeat of the same claim makes no
//! request. A timeout or 5xx anywhere is evidence of nothing; it surfaces as `Pending`.

use std::collections::HashSet;

use lysilogy_core::identifier::normalize;
use lysilogy_core::names::citation_family;
use lysilogy_core::{
    CLAIM_CANDIDATES_MAX, Claim, ClaimEvidence, ClaimOutcome, ClaimState, ClaimVerdict,
    IdentifierLookup, IdentifierScheme, IdentifierStatus, LocalWork, ProviderRecord, TitleSearch,
    Work, WorkKind, decide_claim, title_key, title_similarity,
};
use lysilogy_db::claim::ClaimRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;

use crate::Result;
use crate::crossref::{
    CrossrefOutcome, fetch_crossref_cached, is_crossref_doi, record_publisher_claims,
};
use crate::ingest::hydrate_stub_works;
use crate::openalex::{
    JsonTransport, OpenAlexClient, OpenAlexSummary, ProviderError, fetch_work_cached_record,
    search_title_cached,
};

/// A title search with no result is retried on each half of the title's words, so one
/// misspelled word cannot hide the paper. Titles shorter than this are not split.
const SPLIT_SEARCH_MIN_WORDS: usize = 4;
/// A title cut off at a subtitle needs this many words to count as a duplicate of the longer.
const DUPLICATE_PREFIX_MIN_WORDS: usize = 3;
/// The `provider_id` prefix of a record that has no `OpenAlex` ID (Crossref only).
const DOI_RECORD_PREFIX: &str = "doi:";

/// The repositories and provider client verification works through.
pub struct VerifyContext<'a, T: JsonTransport> {
    pub works: &'a WorkRepository,
    pub claims: &'a ClaimRepository,
    pub records: &'a SourceRecordRepository,
    pub client: &'a OpenAlexClient<T>,
}

/// What a verified claim became.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimResolution {
    /// The claim is a Work: an existing one (`Matched`) or one found or created for a
    /// provider record (`Verified`).
    Work {
        work: Work,
        /// A new Work row was created for this claim.
        created: bool,
        /// False when the claim matched a Work already in the graph.
        verified: bool,
        identifier_dropped: bool,
    },
    /// The claim stays a `claim` node in this state.
    Unresolved {
        state: ClaimState,
        candidates: Vec<ProviderRecord>,
        identifier_dropped: bool,
    },
}

/// Whether a provider failure says nothing about the claim (timeout, 5xx, rate limit).
/// Other 4xx statuses and malformed bodies are answers: the provider does not have it.
const fn is_transport_error(error: &ProviderError) -> bool {
    match error {
        ProviderError::Request(_) => true,
        ProviderError::Status(status) => *status == 429 || *status >= 500,
        _ => false,
    }
}

fn record_from_summary(summary: &OpenAlexSummary) -> Option<ProviderRecord> {
    Some(ProviderRecord {
        provider_id: summary.openalex_id.clone(),
        title: summary.title.clone()?,
        year: summary.year,
        authors: summary.authors.clone(),
        doi: summary.doi.clone(),
    })
}

/// Families (title-keyed surnames) of a record's authors.
fn author_families(record: &ProviderRecord) -> HashSet<String> {
    record
        .authors
        .iter()
        .filter_map(|author| citation_family(author))
        .map(|family| title_key(&family))
        .filter(|family| !family.is_empty())
        .collect()
}

/// Whether one title key is the other cut off at a word boundary (a dropped subtitle).
fn is_subtitle_of(left: &str, right: &str) -> bool {
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    short.split_whitespace().count() >= DUPLICATE_PREFIX_MIN_WORDS
        && long.starts_with(short)
        && long[short.len()..].starts_with(' ')
}

/// Whether two records are the same paper entered twice: an equal title, or one cut off at a
/// subtitle, in years within one, with an author surname in common.
fn same_paper(left: &ProviderRecord, right: &ProviderRecord) -> bool {
    let years_near = match (left.year, right.year) {
        (Some(a), Some(b)) => a.abs_diff(b) <= 1,
        _ => true,
    };
    if !years_near {
        return false;
    }
    let (left_key, right_key) = (title_key(&left.title), title_key(&right.title));
    if left_key != right_key && !is_subtitle_of(&left_key, &right_key) {
        return false;
    }
    let right_families = author_families(right);
    author_families(left)
        .iter()
        .any(|family| right_families.contains(family))
}

/// Collapse `OpenAlex` duplicates of one paper to the most-referenced record.
///
/// A preprint and its published version, or the same paper under two spellings of its
/// authors, are one paper. The tie-break is the one `select_title_match` uses, so a claim is
/// not ambiguous with itself. Order of first appearance is kept.
#[must_use]
pub fn collapse_duplicate_records(
    summaries: &[OpenAlexSummary],
) -> Vec<(ProviderRecord, Option<u32>)> {
    let mut kept: Vec<(ProviderRecord, Option<u32>)> = Vec::new();
    for summary in summaries {
        let Some(record) = record_from_summary(summary) else {
            continue;
        };
        let referenced = summary.referenced_works_count;
        let Some(slot) = kept
            .iter()
            .position(|(other, _)| same_paper(other, &record))
        else {
            kept.push((record, referenced));
            continue;
        };
        let (current, current_referenced) = &kept[slot];
        let better = referenced.unwrap_or(0) > current_referenced.unwrap_or(0)
            || (referenced.unwrap_or(0) == current_referenced.unwrap_or(0)
                && record.provider_id < current.provider_id);
        if better {
            kept[slot] = (record, referenced);
        }
    }
    kept
}

/// The two queries tried when a title search finds nothing: the first and the second half
/// of the title's words. `None` when the title is too short to split.
fn title_halves(title: &str) -> Option<[String; 2]> {
    let key = title_key(title);
    let words: Vec<&str> = key.split_whitespace().collect();
    if words.len() < SPLIT_SEARCH_MIN_WORDS {
        return None;
    }
    let middle = words.len().div_ceil(2);
    Some([words[..middle].join(" "), words[middle..].join(" ")])
}

/// Whether any record is close enough to the claimed title to stop searching.
fn has_close_title(title: &str, records: &[(ProviderRecord, Option<u32>)]) -> bool {
    records.iter().any(|(record, _)| {
        title_similarity(title, &record.title) >= lysilogy_core::claim::TITLE_PLAUSIBLE_MIN
    })
}

async fn local_works(works: &WorkRepository, claim: &Claim) -> Result<Vec<LocalWork>> {
    let key = title_key(&claim.title);
    let found = works.find_by_title_key(&key, None).await?;
    Ok(found
        .into_iter()
        .filter_map(|work| {
            Some(LocalWork {
                work_id: work.id,
                title: work.display_title?,
                year: work.display_year,
                authors: work.display_authors.unwrap_or_default(),
            })
        })
        .collect())
}

/// Look one DOI up: `OpenAlex`, then Crossref when `OpenAlex` does not hold it.
async fn lookup_doi<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
    doi: &str,
) -> Result<IdentifierLookup> {
    let mut transport_failed = false;
    match fetch_work_cached_record(context.records, context.client, &format!("doi:{doi}")).await {
        Ok((_, work)) => {
            return Ok(resolved_or_not_found(
                claim,
                ProviderRecord {
                    provider_id: work.openalex_id,
                    title: work.title.unwrap_or_default(),
                    year: work.year,
                    authors: work.authors,
                    doi: work.doi.or_else(|| Some(doi.to_owned())),
                },
            ));
        }
        Err(ProviderError::Db(error)) => return Err(error.into()),
        Err(error) => transport_failed |= is_transport_error(&error),
    }
    if !is_crossref_doi(doi) {
        return Ok(not_found_or_transport(transport_failed));
    }
    match fetch_crossref_cached(context.records, context.client, doi).await {
        Ok(CrossrefOutcome::Fetched(_, work) | CrossrefOutcome::Cached(_, work)) => {
            Ok(resolved_or_not_found(
                claim,
                ProviderRecord {
                    provider_id: format!("{DOI_RECORD_PREFIX}{doi}"),
                    title: work.title.unwrap_or_default(),
                    year: work.year,
                    authors: work.authors,
                    doi: Some(doi.to_owned()),
                },
            ))
        }
        Ok(CrossrefOutcome::NotFound { .. } | CrossrefOutcome::Skipped) => {
            Ok(not_found_or_transport(transport_failed))
        }
        Err(ProviderError::Db(error)) => Err(error.into()),
        Err(error) => Ok(not_found_or_transport(
            transport_failed || is_transport_error(&error),
        )),
    }
}

/// A record with no title cannot vouch for a claim that has one.
fn resolved_or_not_found(claim: &Claim, record: ProviderRecord) -> IdentifierLookup {
    if record.title.trim().is_empty() && !claim.title.trim().is_empty() {
        return IdentifierLookup::NotFound;
    }
    IdentifierLookup::Resolved(record)
}

const fn not_found_or_transport(transport_failed: bool) -> IdentifierLookup {
    if transport_failed {
        IdentifierLookup::TransportError
    } else {
        IdentifierLookup::NotFound
    }
}

/// Look an arXiv ID up through its `DataCite` DOI, which only `OpenAlex` holds.
async fn lookup_arxiv<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
    arxiv: &str,
) -> Result<IdentifierLookup> {
    let key = format!("doi:10.48550/arxiv.{arxiv}");
    match fetch_work_cached_record(context.records, context.client, &key).await {
        Ok((_, work)) => Ok(resolved_or_not_found(
            claim,
            ProviderRecord {
                provider_id: work.openalex_id,
                title: work.title.unwrap_or_default(),
                year: work.year,
                authors: work.authors,
                doi: work.doi,
            },
        )),
        Err(ProviderError::Db(error)) => Err(error.into()),
        Err(error) => Ok(not_found_or_transport(is_transport_error(&error))),
    }
}

/// One cached title search, folded into records. A transport failure is `Err(())`.
async fn search_once<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    title: &str,
    year: Option<i32>,
) -> Result<std::result::Result<Vec<(ProviderRecord, Option<u32>)>, ()>> {
    match search_title_cached(context.records, context.client, title, year).await {
        Ok(Some((_, summaries))) => Ok(Ok(collapse_duplicate_records(&summaries))),
        Err(ProviderError::Db(error)) => Err(error.into()),
        Err(error) if is_transport_error(&error) => Ok(Err(())),
        Ok(None) | Err(_) => Ok(Ok(Vec::new())),
    }
}

async fn search_title<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
) -> Result<TitleSearch> {
    let Ok(mut found) = search_once(context, &claim.title, claim.year).await? else {
        return Ok(TitleSearch::TransportError);
    };
    if found.is_empty()
        && let Some(halves) = title_halves(&claim.title)
    {
        for half in &halves {
            let Ok(more) = search_once(context, half, claim.year).await? else {
                return Ok(TitleSearch::TransportError);
            };
            found.extend(more);
            if has_close_title(&claim.title, &found) {
                break;
            }
        }
    }
    let mut seen = HashSet::new();
    Ok(TitleSearch::Results(
        found
            .into_iter()
            .map(|(record, _)| record)
            .filter(|record| seen.insert(record.provider_id.clone()))
            .collect(),
    ))
}

async fn identifier_lookups<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
) -> Result<Vec<IdentifierLookup>> {
    let mut lookups = Vec::new();
    if let Some(raw) = claim.doi.as_deref().filter(|raw| !raw.trim().is_empty()) {
        lookups.push(match normalize(IdentifierScheme::Doi, raw) {
            Some(doi) => lookup_doi(context, claim, &doi).await?,
            // A DOI that is not shaped like one names nothing.
            None => IdentifierLookup::NotFound,
        });
    }
    if let Some(raw) = claim.arxiv.as_deref().filter(|raw| !raw.trim().is_empty()) {
        lookups.push(match normalize(IdentifierScheme::Arxiv, raw) {
            Some(arxiv) => lookup_arxiv(context, claim, &arxiv).await?,
            None => IdentifierLookup::NotFound,
        });
    }
    Ok(lookups)
}

/// Gather what is known about a claim and decide it. The title search runs only when the
/// local graph and the identifiers did not already decide.
pub async fn verdict_for_claim<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
) -> Result<ClaimVerdict> {
    let searchable = {
        let key = title_key(&claim.title);
        !claim.title.trim().is_empty() && key != "untitled"
    };
    let local = if searchable {
        local_works(context.works, claim).await?
    } else {
        Vec::new()
    };
    let identifiers = identifier_lookups(context, claim).await?;
    let mut evidence = ClaimEvidence {
        local,
        identifiers,
        title_search: TitleSearch::NotRun,
    };
    let early = decide_claim(claim, &evidence);
    if matches!(
        early.outcome,
        ClaimOutcome::Matched(_) | ClaimOutcome::Verified(_)
    ) || !searchable
    {
        return Ok(early);
    }
    evidence.title_search = search_title(context, claim).await?;
    Ok(decide_claim(claim, &evidence))
}

/// A Work already holding one of the identifiers that name this record.
async fn existing_work_for_record<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
    identifier_dropped: bool,
    record: &ProviderRecord,
) -> Result<Option<Work>> {
    let mut keys: Vec<(IdentifierScheme, String)> = Vec::new();
    if !identifier_dropped {
        if let Some(doi) = claim
            .doi
            .as_deref()
            .and_then(|raw| normalize(IdentifierScheme::Doi, raw))
        {
            keys.push((IdentifierScheme::Doi, doi));
        }
        if let Some(arxiv) = claim
            .arxiv
            .as_deref()
            .and_then(|raw| normalize(IdentifierScheme::Arxiv, raw))
        {
            keys.push((IdentifierScheme::Arxiv, arxiv));
        }
    }
    if let Some(doi) = record
        .doi
        .as_deref()
        .and_then(|raw| normalize(IdentifierScheme::Doi, raw))
    {
        keys.push((IdentifierScheme::Doi, doi));
    }
    for (scheme, value) in keys {
        let Some(work) = context.works.find_by_identifier(scheme, &value).await? else {
            continue;
        };
        let work = context.works.resolve_redirect(work.id).await?;
        let Some(work) = context.works.get(work).await? else {
            continue;
        };
        if !record.provider_id.starts_with(DOI_RECORD_PREFIX) {
            context
                .works
                .add_identifier(
                    work.id,
                    IdentifierScheme::Openalex,
                    &record.provider_id,
                    None,
                    IdentifierStatus::Active,
                )
                .await?;
        }
        return Ok(Some(work));
    }
    Ok(None)
}

/// Find or create the Work for a provider record and give it its fields.
///
/// A Work that already holds the record's DOI (or the claim's verified DOI or arXiv ID) is
/// the paper, even when it has no `OpenAlex` ID or a different title yet; it gets the
/// `OpenAlex` ID instead of a duplicate being created.
async fn work_for_record<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
    identifier_dropped: bool,
    record: &ProviderRecord,
) -> Result<(Work, bool)> {
    let (work, created) = if let Some(existing) =
        existing_work_for_record(context, claim, identifier_dropped, record).await?
    {
        (existing, false)
    } else if let Some(doi) = record.provider_id.strip_prefix(DOI_RECORD_PREFIX) {
        context
            .works
            .resolve_or_create(IdentifierScheme::Doi, doi, WorkKind::Article, None)
            .await?
    } else {
        context
            .works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                &record.provider_id,
                WorkKind::Article,
                None,
            )
            .await?
    };
    if let Some(doi) = record.provider_id.strip_prefix(DOI_RECORD_PREFIX) {
        if let Ok(
            CrossrefOutcome::Fetched(source, fields) | CrossrefOutcome::Cached(source, fields),
        ) = fetch_crossref_cached(context.records, context.client, doi).await
        {
            record_publisher_claims(context.claims, work.id, source, &fields).await?;
        }
    } else if let Err(error) = hydrate_stub_works(
        context.works,
        context.claims,
        context.records,
        context.client,
        std::slice::from_ref(&work),
    )
    .await
    {
        // The Work exists and keeps its identifier; its title fills in on a later hydrate.
        tracing::warn!(%error, work = %work.id, "could not hydrate a verified claim's Work");
    }
    let refreshed = context.works.get(work.id).await?.unwrap_or(work);
    Ok((refreshed, created))
}

/// Verify a claim. A `Matched` or `Verified` claim becomes a Work (created and hydrated
/// when new); every other outcome is returned for the caller to store as a `claim` node.
pub async fn verify_claim<T: JsonTransport>(
    context: &VerifyContext<'_, T>,
    claim: &Claim,
) -> Result<ClaimResolution> {
    let verdict = verdict_for_claim(context, claim).await?;
    let identifier_dropped = verdict.identifier_dropped;
    match verdict.outcome {
        ClaimOutcome::Matched(work_id) => {
            let Some(work) = context.works.get(work_id).await? else {
                return Ok(ClaimResolution::Unresolved {
                    state: ClaimState::Unverified,
                    candidates: Vec::new(),
                    identifier_dropped,
                });
            };
            Ok(ClaimResolution::Work {
                work,
                created: false,
                verified: false,
                identifier_dropped,
            })
        }
        ClaimOutcome::Verified(record) => {
            let (work, created) =
                work_for_record(context, claim, identifier_dropped, &record).await?;
            Ok(ClaimResolution::Work {
                work,
                created,
                verified: true,
                identifier_dropped,
            })
        }
        other => Ok(ClaimResolution::Unresolved {
            state: other.claim_state().unwrap_or(ClaimState::Unverified),
            candidates: verdict
                .candidates
                .into_iter()
                .take(CLAIM_CANDIDATES_MAX)
                .collect(),
            identifier_dropped,
        }),
    }
}

/// Recorded provider responses: the transports that make verification testable offline.
pub mod fixtures {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use crate::openalex::{JsonTransport, ProviderError, ProviderFuture};

    /// Most attempts for one request that fails in transit while recording.
    const RECORD_ATTEMPTS: u32 = 3;
    /// Pause between live requests while recording.
    const RECORD_PAUSE: Duration = Duration::from_millis(120);

    /// One recorded provider answer: a 200 body, or a bare 404.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RecordedResponse {
        pub status: u16,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        pub body: Value,
    }

    /// Responses keyed by request URL, as written to `responses.json`.
    pub type Responses = BTreeMap<String, RecordedResponse>;

    fn answer(response: &RecordedResponse) -> Result<Value, ProviderError> {
        if response.status == 200 {
            return Ok(response.body.clone());
        }
        Err(ProviderError::Status(response.status))
    }

    /// Replays recorded responses. A URL that was never recorded fails like a network
    /// error and is remembered, so a test can assert that nothing was missing.
    #[derive(Clone)]
    pub struct ReplayTransport {
        responses: Arc<Responses>,
        misses: Arc<Mutex<BTreeSet<String>>>,
    }

    impl ReplayTransport {
        #[must_use]
        pub fn new(responses: Responses) -> Self {
            Self {
                responses: Arc::new(responses),
                misses: Arc::new(Mutex::new(BTreeSet::new())),
            }
        }

        /// URLs requested that were not recorded.
        #[must_use]
        pub fn misses(&self) -> Vec<String> {
            self.misses.lock().expect("lock").iter().cloned().collect()
        }
    }

    impl JsonTransport for ReplayTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, Result<Value, ProviderError>> {
            Box::pin(async move {
                let Some(response) = self.responses.get(url) else {
                    self.misses.lock().expect("lock").insert(url.to_owned());
                    return Err(ProviderError::Request("response was not recorded".into()));
                };
                answer(response)
            })
        }
    }

    /// Wraps a live transport and records what it returns.
    ///
    /// Responses already in `seed` are served without a request, which makes a recording
    /// run resumable. After a 429 or after `max_paid_requests` live title searches no more
    /// are made, so a free-tier budget is never overrun.
    pub struct CapturingTransport<T> {
        inner: T,
        seed: Responses,
        captured: Mutex<Responses>,
        live_requests: AtomicUsize,
        paid_requests: AtomicUsize,
        max_paid_requests: usize,
        exhausted: AtomicBool,
    }

    impl<T: JsonTransport> CapturingTransport<T> {
        #[must_use]
        pub const fn new(inner: T, seed: Responses, max_paid_requests: usize) -> Self {
            Self {
                inner,
                seed,
                captured: Mutex::new(Responses::new()),
                live_requests: AtomicUsize::new(0),
                paid_requests: AtomicUsize::new(0),
                max_paid_requests,
                exhausted: AtomicBool::new(false),
            }
        }

        /// Everything served so far, seeded or live.
        #[must_use]
        pub fn captured(&self) -> Responses {
            self.captured.lock().expect("lock").clone()
        }

        /// Live requests made so far.
        #[must_use]
        pub fn live_requests(&self) -> usize {
            self.live_requests.load(Ordering::SeqCst)
        }

        /// Live title searches made so far.
        #[must_use]
        pub fn paid_requests(&self) -> usize {
            self.paid_requests.load(Ordering::SeqCst)
        }

        /// True once a request was refused for budget or rate-limit reasons.
        #[must_use]
        pub fn exhausted(&self) -> bool {
            self.exhausted.load(Ordering::SeqCst)
        }

        fn remember(&self, url: &str, response: RecordedResponse) {
            self.captured
                .lock()
                .expect("lock")
                .insert(url.to_owned(), response);
        }
    }

    impl<T: JsonTransport> JsonTransport for CapturingTransport<T> {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, Result<Value, ProviderError>> {
            Box::pin(async move {
                if let Some(response) = self.seed.get(url) {
                    self.remember(url, response.clone());
                    return answer(response);
                }
                if self.exhausted() {
                    return Err(ProviderError::Request("recording budget exhausted".into()));
                }
                let mut last = ProviderError::Malformed;
                for attempt in 0..RECORD_ATTEMPTS {
                    self.live_requests.fetch_add(1, Ordering::SeqCst);
                    // Title searches are the only requests the free tier charges for.
                    if url.contains("title.search")
                        && self.paid_requests.fetch_add(1, Ordering::SeqCst)
                            >= self.max_paid_requests
                    {
                        self.exhausted.store(true, Ordering::SeqCst);
                        return Err(ProviderError::Request("paid request cap reached".into()));
                    }
                    tokio::time::sleep(RECORD_PAUSE * (attempt + 1)).await;
                    match self.inner.get_json(url).await {
                        Ok(body) => {
                            self.remember(
                                url,
                                RecordedResponse {
                                    status: 200,
                                    body: body.clone(),
                                },
                            );
                            return Ok(body);
                        }
                        Err(ProviderError::Status(404)) => {
                            self.remember(
                                url,
                                RecordedResponse {
                                    status: 404,
                                    body: Value::Null,
                                },
                            );
                            return Err(ProviderError::Status(404));
                        }
                        Err(ProviderError::Status(429)) => {
                            self.exhausted.store(true, Ordering::SeqCst);
                            return Err(ProviderError::Status(429));
                        }
                        Err(error) => last = error,
                    }
                }
                Err(last)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{RecordedResponse, ReplayTransport, Responses};
    use super::*;
    use crate::openalex::{title_search_url, work_url, works_batch_url};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique() -> String {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}x{sequence}", std::process::id())
    }

    async fn pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = lysilogy_db::connect(&url).await.expect("connect");
        lysilogy_db::migrate(&pool).await.expect("migrate");
        Some(pool)
    }

    fn ok(body: Value) -> RecordedResponse {
        RecordedResponse { status: 200, body }
    }

    fn missing() -> RecordedResponse {
        RecordedResponse {
            status: 404,
            body: Value::Null,
        }
    }

    fn summary(id: &str, title: &str, year: i32, author: &str, referenced: u32) -> Value {
        json!({
            "id": format!("https://openalex.org/{id}"),
            "display_name": title,
            "publication_year": year,
            "referenced_works_count": referenced,
            "authorships": [{"author": {"display_name": author}}],
        })
    }

    struct Rig {
        works: WorkRepository,
        claims: ClaimRepository,
        records: SourceRecordRepository,
    }

    async fn rig() -> Option<Rig> {
        let pool = pool().await?;
        Some(Rig {
            works: WorkRepository::new(pool.clone()),
            claims: ClaimRepository::new(pool.clone()),
            records: SourceRecordRepository::new(pool),
        })
    }

    macro_rules! context {
        ($rig:expr, $client:expr) => {
            VerifyContext {
                works: &$rig.works,
                claims: &$rig.claims,
                records: &$rig.records,
                client: &$client,
            }
        };
    }

    #[allow(clippy::needless_pass_by_value)]
    fn search(responses: &mut Responses, title: &str, year: Option<i32>, results: Vec<Value>) {
        let url = title_search_url(title, year).expect("searchable");
        responses.insert(url, ok(json!({ "results": results })));
    }

    fn claim(title: &str, year: i32, author: &str) -> Claim {
        Claim {
            title: title.to_owned(),
            authors: vec![author.to_owned()],
            year: Some(year),
            ..Claim::default()
        }
    }

    #[test]
    fn should_keep_most_referenced_record_when_openalex_duplicates_a_paper() {
        let parse = |value: &Value| crate::openalex::parse_summary(value).expect("summary");
        let summaries = vec![
            parse(&summary("W2", "Same Paper Title", 2019, "Ada Lovelace", 10)),
            parse(&summary(
                "W1",
                "Same Paper Title.",
                2020,
                "Ada Lovelace",
                90,
            )),
            parse(&summary("W3", "Same Paper Title", 2020, "Grace Hopper", 5)),
            parse(&json!({"id": "https://openalex.org/W4"})),
        ];
        let kept = collapse_duplicate_records(&summaries);
        let ids: Vec<&str> = kept
            .iter()
            .map(|(record, _)| record.provider_id.as_str())
            .collect();
        assert_eq!(ids, vec!["W1", "W3"]);
    }

    fn ids(kept: &[(ProviderRecord, Option<u32>)]) -> Vec<&str> {
        kept.iter()
            .map(|(record, _)| record.provider_id.as_str())
            .collect()
    }

    #[test]
    fn should_collapse_records_when_authors_are_spelled_differently_but_share_a_surname() {
        let parse = |value: &Value| crate::openalex::parse_summary(value).expect("summary");
        let mut first = summary(
            "W1",
            "Breeding Mario Behaviors",
            2016,
            "Patrikk Sorensen",
            21,
        );
        first["authorships"] = json!([
            {"author": {"display_name": "Patrikk D. Sorensen"}},
            {"author": {"display_name": "Sebastian Risi"}},
        ]);
        let mut second = summary(
            "W2",
            "Breeding Mario Behaviors",
            2016,
            "D Sorensen Patrikk",
            0,
        );
        second["authorships"] = json!([
            {"author": {"display_name": "D Sorensen Patrikk"}},
            {"author": {"display_name": "Sebastian Risi"}},
        ]);
        let kept = collapse_duplicate_records(&[parse(&first), parse(&second)]);
        assert_eq!(ids(&kept), vec!["W1"]);
    }

    #[test]
    fn should_collapse_a_subtitle_trimmed_record_into_the_full_title_when_authors_and_year_agree() {
        let parse = |value: &Value| crate::openalex::parse_summary(value).expect("summary");
        let summaries = vec![
            parse(&summary(
                "W1",
                "Rank Analysis of Incomplete Block Designs",
                1952,
                "Ralph Bradley",
                0,
            )),
            parse(&summary(
                "W2",
                "Rank Analysis of Incomplete Block Designs: I. The Method of Paired Comparisons",
                1952,
                "Ralph Bradley",
                3,
            )),
        ];
        assert_eq!(ids(&collapse_duplicate_records(&summaries)), vec!["W2"]);
    }

    #[test]
    fn should_keep_records_apart_when_they_extend_a_shared_head_differently() {
        let parse = |value: &Value| crate::openalex::parse_summary(value).expect("summary");
        let summaries = vec![
            parse(&summary(
                "W1",
                "Stimulus and Response Generalization: A Stochastic Model",
                1957,
                "Roger Shepard",
                28,
            )),
            parse(&summary(
                "W2",
                "Stimulus and Response Generalization: Tests of a Model",
                1958,
                "Roger Shepard",
                47,
            )),
        ];
        assert_eq!(
            ids(&collapse_duplicate_records(&summaries)),
            vec!["W1", "W2"]
        );
    }

    #[test]
    fn should_keep_records_apart_when_years_or_authors_differ() {
        let parse = |value: &Value| crate::openalex::parse_summary(value).expect("summary");
        let summaries = vec![
            parse(&summary("W1", "On Compact Spaces", 1990, "Ada Lovelace", 1)),
            parse(&summary("W2", "On Compact Spaces", 2000, "Ada Lovelace", 1)),
            parse(&summary("W3", "On Compact Spaces", 1990, "Grace Hopper", 1)),
            parse(&json!({"id": "https://openalex.org/W4", "display_name": "On Compact Spaces"})),
        ];
        assert_eq!(
            ids(&collapse_duplicate_records(&summaries)),
            vec!["W1", "W2", "W3", "W4"]
        );
    }

    #[test]
    fn should_split_title_in_halves_only_when_it_has_enough_words() {
        assert_eq!(title_halves("Too short title"), None);
        assert_eq!(
            title_halves("Deep Batch Active Learning by Diverse Gradients"),
            Some([
                "deep batch active learning".to_owned(),
                "by diverse gradients".to_owned()
            ])
        );
        assert_eq!(title_halves(""), None);
    }

    #[test]
    fn should_treat_timeouts_rate_limits_and_server_errors_as_transport_failures_when_a_provider_fails()
     {
        assert!(is_transport_error(&ProviderError::Request("x".into())));
        assert!(is_transport_error(&ProviderError::Status(503)));
        assert!(is_transport_error(&ProviderError::Status(429)));
        assert!(!is_transport_error(&ProviderError::Status(404)));
        assert!(!is_transport_error(&ProviderError::Status(400)));
        assert!(!is_transport_error(&ProviderError::Status(403)));
        assert!(!is_transport_error(&ProviderError::Status(410)));
        assert!(!is_transport_error(&ProviderError::Malformed));
    }

    #[tokio::test]
    async fn should_create_hydrated_work_when_title_search_finds_the_paper() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let title = format!("Verified claims create stub works {marker}");
        let id = format!("W9{marker}");
        let mut responses = Responses::new();
        let record = summary(&id, &title, 2021, "Ada Lovelace", 4);
        search(&mut responses, &title, Some(2021), vec![record.clone()]);
        responses.insert(
            works_batch_url(&[&id.to_ascii_uppercase()]),
            ok(json!({"results": [record]})),
        );
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let context = context!(rig, client);
        let resolution = verify_claim(&context, &claim(&title, 2021, "Ada Lovelace"))
            .await
            .expect("verify");
        let ClaimResolution::Work {
            work,
            created,
            verified,
            ..
        } = resolution
        else {
            panic!("expected a Work, got {resolution:?}");
        };
        assert!(created && verified);
        assert_eq!(work.display_title.as_deref(), Some(title.as_str()));
        let again = verify_claim(&context, &claim(&title, 2021, "Ada Lovelace"))
            .await
            .expect("verify again");
        let ClaimResolution::Work {
            work: same,
            created,
            verified,
            ..
        } = again
        else {
            panic!("expected a Work");
        };
        assert_eq!(same.id, work.id);
        assert!(!created && !verified, "second run matches the local Work");
    }

    #[tokio::test]
    async fn should_be_pending_when_the_title_search_times_out() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let title = format!("A claim whose search never answers {}", unique());
        // Nothing is recorded, so every request fails like a timeout.
        let client = OpenAlexClient::new(ReplayTransport::new(Responses::new()));
        let resolution = verify_claim(&context!(rig, client), &claim(&title, 2020, "Ada Lovelace"))
            .await
            .expect("verify");
        assert_eq!(
            resolution,
            ClaimResolution::Unresolved {
                state: ClaimState::Pending,
                candidates: Vec::new(),
                identifier_dropped: false
            }
        );
    }

    #[tokio::test]
    async fn should_be_pending_when_the_doi_lookup_returns_a_server_error() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let doi = format!("10.5555/pending.{marker}");
        let mut responses = Responses::new();
        responses.insert(
            work_url(&format!("doi:{doi}")),
            RecordedResponse {
                status: 503,
                body: Value::Null,
            },
        );
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let asked = Claim {
            doi: Some(doi),
            ..Claim::default()
        };
        let resolution = verify_claim(&context!(rig, client), &asked)
            .await
            .expect("verify");
        assert!(matches!(
            resolution,
            ClaimResolution::Unresolved {
                state: ClaimState::Pending,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn should_verify_through_crossref_when_openalex_lacks_the_doi() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let doi = format!("10.5555/crossref.{marker}");
        let title = format!("A journal article only crossref knows {marker}");
        let mut responses = Responses::new();
        responses.insert(work_url(&format!("doi:{doi}")), missing());
        responses.insert(
            crate::crossref::crossref_work_url(&doi),
            ok(json!({"message": {
                "DOI": doi,
                "title": [title],
                "author": [{"given": "Ada", "family": "Lovelace"}],
                "issued": {"date-parts": [[2018]]},
            }})),
        );
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let asked = Claim {
            doi: Some(format!("https://doi.org/{}", doi.to_ascii_uppercase())),
            ..claim(&title, 2018, "A. Lovelace")
        };
        let resolution = verify_claim(&context!(rig, client), &asked)
            .await
            .expect("verify");
        let ClaimResolution::Work { work, created, .. } = resolution else {
            panic!("expected a Work, got {resolution:?}");
        };
        assert!(created);
        assert_eq!(work.display_title.as_deref(), Some(title.as_str()));
    }

    #[tokio::test]
    async fn should_be_ambiguous_when_the_doi_is_invented_but_the_title_is_real() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let title = format!("A real title wearing a made up doi {marker}");
        let doi = format!("10.5555/madeup.{marker}");
        let mut responses = Responses::new();
        responses.insert(work_url(&format!("doi:{doi}")), missing());
        responses.insert(crate::crossref::crossref_work_url(&doi), missing());
        search(
            &mut responses,
            &title,
            Some(2020),
            vec![summary(
                &format!("W8{marker}"),
                &title,
                2020,
                "Ada Lovelace",
                3,
            )],
        );
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let asked = Claim {
            doi: Some(doi),
            ..claim(&title, 2020, "Ada Lovelace")
        };
        let resolution = verify_claim(&context!(rig, client), &asked)
            .await
            .expect("verify");
        let ClaimResolution::Unresolved {
            state,
            candidates,
            identifier_dropped,
        } = resolution
        else {
            panic!("expected an unresolved claim, got {resolution:?}");
        };
        assert_eq!(state, ClaimState::Ambiguous);
        assert_eq!(candidates.len(), 1);
        assert!(identifier_dropped);
    }

    #[tokio::test]
    async fn should_be_unverified_when_the_doi_is_not_shaped_like_a_doi() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let client = OpenAlexClient::new(ReplayTransport::new(Responses::new()));
        let asked = Claim {
            doi: Some("not a doi at all".into()),
            ..Claim::default()
        };
        let resolution = verify_claim(&context!(rig, client), &asked)
            .await
            .expect("verify");
        assert!(matches!(
            resolution,
            ClaimResolution::Unresolved {
                state: ClaimState::Unverified,
                identifier_dropped: true,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn should_find_a_misspelled_title_when_a_half_of_it_still_searches() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let real = format!("Gradient{marker} methods for noisy quantum devices");
        let typo = format!("Gradient{marker} methods for noisy quantum divices");
        let id = format!("W7{marker}");
        let mut responses = Responses::new();
        search(&mut responses, &typo, Some(2022), vec![]);
        let halves = title_halves(&typo).expect("splits");
        search(
            &mut responses,
            &halves[0],
            Some(2022),
            vec![summary(&id, &real, 2022, "Ada Lovelace", 3)],
        );
        search(&mut responses, &halves[1], Some(2022), vec![]);
        responses.insert(
            works_batch_url(&[&id.to_ascii_uppercase()]),
            ok(json!({"results": [summary(&id, &real, 2022, "Ada Lovelace", 3)]})),
        );
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let resolution = verify_claim(&context!(rig, client), &claim(&typo, 2022, "Ada Lovelace"))
            .await
            .expect("verify");
        assert!(
            matches!(resolution, ClaimResolution::Work { verified: true, .. }),
            "{resolution:?}"
        );
    }

    #[tokio::test]
    async fn should_reuse_the_doi_keyed_work_when_a_claim_with_a_different_title_verifies() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let doi = format!("10.5555/held.{marker}");
        let (existing, _) = rig
            .works
            .resolve_or_create(IdentifierScheme::Doi, &doi, WorkKind::Article, None)
            .await
            .expect("existing");
        let real = format!("Held paper about reuse of stored works {marker}");
        let claimed = format!("Held paper about reuse of stored works {marker} revisited");
        let id = format!("W6{marker}");
        let mut record = summary(&id, &real, 2019, "Ada Lovelace", 4);
        record["doi"] = json!(format!("https://doi.org/{doi}"));
        let mut responses = Responses::new();
        search(&mut responses, &claimed, Some(2019), vec![record.clone()]);
        responses.insert(
            works_batch_url(&[&id.to_ascii_uppercase()]),
            ok(json!({"results": [record]})),
        );
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let resolution = verify_claim(
            &context!(rig, client),
            &claim(&claimed, 2019, "Ada Lovelace"),
        )
        .await
        .expect("verify");
        let ClaimResolution::Work { work, created, .. } = resolution else {
            panic!("expected a Work, got {resolution:?}");
        };
        assert_eq!(work.id, existing.id);
        assert!(!created);
        let found = rig
            .works
            .find_by_identifier(IdentifierScheme::Openalex, &id)
            .await
            .expect("find");
        assert_eq!(found.map(|work| work.id), Some(existing.id));
    }

    #[tokio::test]
    async fn should_not_verify_when_the_doi_resolves_to_an_untitled_record() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let doi = format!("10.5555/untitled.{marker}");
        let mut responses = Responses::new();
        responses.insert(
            work_url(&format!("doi:{doi}")),
            ok(json!({"id": format!("https://openalex.org/W5{marker}")})),
        );
        responses.insert(crate::crossref::crossref_work_url(&doi), missing());
        let client = OpenAlexClient::new(ReplayTransport::new(responses));
        let asked = Claim {
            doi: Some(doi),
            ..claim(&format!("Some titled claim {marker}"), 2020, "Ada Lovelace")
        };
        let resolution = verify_claim(&context!(rig, client), &asked)
            .await
            .expect("verify");
        assert!(
            matches!(resolution, ClaimResolution::Unresolved { .. }),
            "{resolution:?}"
        );
    }

    #[tokio::test]
    async fn should_not_search_when_the_title_is_blank() {
        let Some(rig) = rig().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let replay = ReplayTransport::new(Responses::new());
        let client = OpenAlexClient::new(replay.clone());
        let resolution = verify_claim(&context!(rig, client), &Claim::default())
            .await
            .expect("verify");
        assert!(matches!(
            resolution,
            ClaimResolution::Unresolved {
                state: ClaimState::Unverified,
                ..
            }
        ));
        assert!(replay.misses().is_empty());
    }
}
