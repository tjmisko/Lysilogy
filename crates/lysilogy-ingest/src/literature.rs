//! The literature flow for one local paper: register the file, link it to a
//! Work, fetch its `OpenAlex` record, ingest its references, and title the
//! stub Works those references created.

use lysilogy_core::provenance::FieldValue;
use lysilogy_core::{
    ClaimConfidence, ClaimOrigin, Coverage, DetectedIdentifier, DocumentId, IdentifierScheme,
    IdentifierStatus, ScannedFile, SourceRecordId, Work, WorkId, WorkKind, WorkLinkMethod,
    extract_identifiers, identifier::normalize, title_key,
};
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::claim::{ClaimRepository, NewClaim};
use lysilogy_db::document::DocumentRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::crossref::{
    CrossrefOutcome, PUBLISHER_FETCH_MAX, PublisherFetch, PublisherOutcome, fetch_crossref_cached,
    is_crossref_doi, record_publisher_claims,
};
use crate::front_matter::front_matter_fields;
use crate::identify::{PaperHints, primary_identifier};
use crate::ingest::{hydrate_stub_works, ingest_openalex_references};
use crate::openalex::{
    JsonTransport, OpenAlexClient, OpenAlexWork, ProviderError, fetch_work_cached_record,
    search_title_cached,
};
use crate::title_match::{
    TitleMatch, TitleQuery, TitleRejection, select_title_match, titles_disagree,
};
use crate::{IngestError, Result};

/// The repositories and provider client the flow works through.
pub struct LiteratureContext<'a, T: JsonTransport> {
    pub works: &'a WorkRepository,
    pub documents: &'a DocumentRepository,
    pub citations: &'a CitationRepository,
    pub claims: &'a ClaimRepository,
    pub records: &'a SourceRecordRepository,
    pub client: &'a OpenAlexClient<T>,
}

/// One paper to process. `front_matter` must already be cut at the references
/// heading (see `lysilogy_core::front_matter`).
#[derive(Clone, Debug)]
pub struct LiteratureInput<'a> {
    pub file: ScannedFile,
    pub front_matter: &'a str,
    pub hints: PaperHints<'a>,
    /// Also fetch the publisher record of the cited Works with a DOI
    /// (at most [`PUBLISHER_FETCH_MAX`] requests).
    pub publisher_cited: bool,
}

/// Source record kind for the front-matter evidence of a paper.
pub const PAPER_FRONT_MATTER_KIND: &str = "paper_front_matter";

/// Extractor recorded on claims read from a paper's own front matter.
const FRONT_MATTER_EXTRACTOR: &str = "frontmatter-v1";

/// Whether the paper ended up attached to a Work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiteratureStatus {
    Linked,
    Unmatched { reason: String },
}

/// What one flow run did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LiteratureSummary {
    pub status: LiteratureStatus,
    pub document_id: DocumentId,
    pub work_id: Option<WorkId>,
    pub link_method: Option<WorkLinkMethod>,
    pub link_detail: Option<String>,
    pub openalex_id: Option<String>,
    /// Citation edges asserted from the provider record.
    pub references: usize,
    /// Works created by this run's reference ingest.
    pub new_stub_works: usize,
    /// Stub Works given a title by this run.
    pub hydrated_works: usize,
    /// Publisher (Crossref) lookups, the paper's own DOI first.
    pub publisher: Vec<PublisherFetch>,
    /// Stable `snake_case` codes with short context.
    pub warnings: Vec<String>,
}

/// The provider lookup key for a detected identifier. arXiv papers are looked
/// up through their `DataCite` DOI.
#[must_use]
pub fn provider_key(identifier: &DetectedIdentifier) -> String {
    match identifier.scheme {
        IdentifierScheme::Doi => format!("doi:{}", identifier.value),
        IdentifierScheme::Arxiv => format!("doi:10.48550/arxiv.{}", identifier.value),
        IdentifierScheme::Openalex => identifier.value.clone(),
        scheme => format!("{scheme}:{}", identifier.value),
    }
}

/// Find the Work for a paper: by `OpenAlex` ID, then by the primary identifier
/// (DOI or arXiv). A Work is created only when none exists and there is
/// something to create it from.
pub async fn resolve_paper_work(
    works: &WorkRepository,
    openalex_id: Option<&str>,
    primary: Option<&DetectedIdentifier>,
) -> Result<Option<Work>> {
    if let Some(openalex_id) = openalex_id
        && let Some(work) = works
            .find_by_identifier(IdentifierScheme::Openalex, openalex_id)
            .await?
    {
        return Ok(Some(work));
    }
    if let Some(primary) = primary
        && let Some(work) = works
            .find_by_identifier(primary.scheme, &primary.value)
            .await?
    {
        return Ok(Some(work));
    }
    let (scheme, value) = match (primary, openalex_id) {
        (Some(primary), _) => (primary.scheme, primary.value.as_str()),
        (None, Some(openalex_id)) => (IdentifierScheme::Openalex, openalex_id),
        (None, None) => return Ok(None),
    };
    let (work, _created) = works
        .resolve_or_create(scheme, value, WorkKind::Article, None)
        .await?;
    Ok(Some(work))
}

/// Mutable state threaded through the flow steps.
struct Flow {
    work: Option<Work>,
    provider: Option<(SourceRecordId, OpenAlexWork)>,
    link: Option<(WorkLinkMethod, Option<SourceRecordId>, String)>,
    rejection: Option<String>,
    warnings: Vec<String>,
}

/// Run the literature flow for one paper.
///
/// Registers only this file (never tombstoning other documents), links it to a
/// Work from front-matter identifiers or, failing that, an exact title match,
/// then ingests references and titles the stubs. Soft provider failures
/// (not found, malformed) become warnings; other provider errors fail the run.
pub async fn run_literature_flow<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    input: LiteratureInput<'_>,
) -> Result<LiteratureSummary> {
    let workspace = context.documents.ensure_default_workspace().await?;
    context
        .documents
        .apply_partial_scan(workspace, std::slice::from_ref(&input.file))
        .await?;
    let document = context
        .documents
        .find_by_path(workspace, &input.file.relative_path)
        .await?
        .ok_or_else(|| {
            IngestError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("document was not registered: {}", input.file.relative_path),
            ))
        })?;

    let mut flow = Flow {
        work: None,
        provider: None,
        link: None,
        rejection: None,
        warnings: Vec::new(),
    };
    let detected = extract_identifiers(input.front_matter);
    if let Some(primary) = primary_identifier(&detected) {
        link_by_identifier(context, &mut flow, &detected, &primary, input.hints).await?;
    }
    if flow.provider.is_none() {
        link_by_title(context, &mut flow, input.hints).await?;
    }

    let Some(work) = flow.work.clone() else {
        let reason = flow
            .rejection
            .clone()
            .unwrap_or_else(|| "no_identifier_or_title".to_owned());
        return Ok(LiteratureSummary {
            status: LiteratureStatus::Unmatched { reason },
            document_id: document.id,
            work_id: None,
            link_method: None,
            link_detail: None,
            openalex_id: None,
            references: 0,
            new_stub_works: 0,
            hydrated_works: 0,
            publisher: Vec::new(),
            warnings: flow.warnings,
        });
    };
    if let Some((_, provider)) = &flow.provider {
        attach_provider_identifiers(context.works, work.id, provider, &mut flow.warnings).await?;
    }
    if let Some((method, record, detail)) = &flow.link {
        context
            .documents
            .set_work_link(document.id, work.id, *method, *record, detail)
            .await?;
    }
    fill_display_fields(context, work.id, &flow, &input).await?;

    let mut summary = LiteratureSummary {
        status: LiteratureStatus::Linked,
        document_id: document.id,
        work_id: Some(work.id),
        link_method: flow.link.as_ref().map(|(method, _, _)| *method),
        link_detail: flow.link.as_ref().map(|(_, _, detail)| detail.clone()),
        openalex_id: flow
            .provider
            .as_ref()
            .map(|(_, provider)| provider.openalex_id.clone()),
        references: 0,
        new_stub_works: 0,
        hydrated_works: 0,
        publisher: Vec::new(),
        warnings: Vec::new(),
    };
    if let Some(doi) = own_publisher_doi(&flow, &detected)
        && let Some(fetch) =
            fetch_publisher(context, work.id, &doi, false, &mut flow.warnings).await?
    {
        summary.publisher.push(fetch);
    }
    if let Some((record, provider)) = &flow.provider {
        ingest_provider_references(
            context,
            &work,
            (*record, provider),
            input.publisher_cited,
            &mut summary,
            &mut flow.warnings,
        )
        .await?;
    }
    summary.warnings = flow.warnings;
    Ok(summary)
}

/// Steps 7-8: ingest the provider record's references, then record summaries
/// (titles, years, authors, DOIs) for the cited Works and the paper itself, and
/// optionally the cited Works' publisher records.
async fn ingest_provider_references<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    work: &Work,
    (record, provider): (SourceRecordId, &OpenAlexWork),
    publisher_cited: bool,
    summary: &mut LiteratureSummary,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let report =
        ingest_openalex_references(context.works, context.citations, work.id, provider, record)
            .await?;
    summary.references = report.edges;
    summary.new_stub_works = report.new_works;
    let cited = context.citations.outgoing(work.id).await?;
    let hydrated: Vec<Work> = cited
        .iter()
        .cloned()
        .chain(std::iter::once(work.clone()))
        .collect();
    summary.hydrated_works = hydrate_stub_works(
        context.works,
        context.claims,
        context.records,
        context.client,
        &hydrated,
    )
    .await?;
    if publisher_cited {
        fetch_cited_publishers(context, &cited, &mut summary.publisher, warnings).await?;
    }
    Ok(())
}

/// Steps 2-3: resolve the paper's own Work from its front-matter identifier and
/// fetch the provider record, unless the record belongs to a different paper.
async fn link_by_identifier<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    flow: &mut Flow,
    detected: &[DetectedIdentifier],
    primary: &DetectedIdentifier,
    hints: PaperHints<'_>,
) -> Result<()> {
    flow.provider = fetch_provider(context, &provider_key(primary), &mut flow.warnings).await?;
    if let Some((_, provider)) = &flow.provider
        && titles_disagree(hints.title, provider.title.as_deref())
    {
        flow.warnings.push(format!(
            "provider_title_mismatch: {} is titled {:?}",
            provider.openalex_id,
            provider.title.as_deref().unwrap_or_default()
        ));
        flow.provider = None;
    }
    let openalex_id = flow
        .provider
        .as_ref()
        .map(|(_, provider)| provider.openalex_id.clone());
    let Some(work) =
        resolve_paper_work(context.works, openalex_id.as_deref(), Some(primary)).await?
    else {
        return Ok(());
    };
    for identifier in detected {
        attach_identifier(
            context.works,
            work.id,
            identifier.scheme,
            &identifier.value,
            &mut flow.warnings,
        )
        .await?;
    }
    let record = flow.provider.as_ref().map(|(record, _)| *record);
    flow.link = Some((
        WorkLinkMethod::Identifier,
        record,
        format!("{}:{}", primary.scheme, primary.value),
    ));
    flow.work = Some(work);
    Ok(())
}

/// Step 4: with no trustworthy provider record, search by exact title.
async fn link_by_title<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    flow: &mut Flow,
    hints: PaperHints<'_>,
) -> Result<()> {
    let Some(title) = hints.title.map(str::trim).filter(|title| !title.is_empty()) else {
        return Ok(());
    };
    let query = TitleQuery {
        title,
        year: hints.year,
        first_author: hints.first_author,
    };
    // A generic title is rejected before any request is made.
    if select_title_match(&query, &[]) == TitleMatch::Rejected(TitleRejection::TitleTooGeneric) {
        flow.warnings
            .push(TitleRejection::TitleTooGeneric.as_str().to_owned());
        flow.rejection = Some(TitleRejection::TitleTooGeneric.as_str().to_owned());
        return Ok(());
    }
    let searched =
        match search_title_cached(context.records, context.client, title, hints.year).await {
            Ok(searched) => searched,
            Err(error) if is_soft(&error) => {
                flow.warnings.push("title_search_failed".to_owned());
                flow.rejection = Some("title_search_failed".to_owned());
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
    let Some((search_record, candidates)) = searched else {
        flow.rejection = Some("title_has_no_searchable_words".to_owned());
        return Ok(());
    };
    let (candidate, considered) = match select_title_match(&query, &candidates) {
        TitleMatch::Accepted {
            candidate,
            considered,
        } => (candidate, considered),
        TitleMatch::Rejected(reason) => {
            flow.warnings.push(reason.as_str().to_owned());
            flow.rejection = Some(reason.as_str().to_owned());
            return Ok(());
        }
    };
    let Some(provider) =
        fetch_provider(context, &candidate.openalex_id, &mut flow.warnings).await?
    else {
        flow.rejection = Some("provider_not_found".to_owned());
        return Ok(());
    };
    if flow.work.is_none() {
        flow.work = resolve_paper_work(context.works, Some(&candidate.openalex_id), None).await?;
        flow.link = Some((
            WorkLinkMethod::TitleMatch,
            Some(search_record),
            format!("title_key={};candidates={considered}", title_key(title)),
        ));
    }
    flow.provider = Some(provider);
    Ok(())
}

async fn fetch_provider<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    key: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<(SourceRecordId, OpenAlexWork)>> {
    match fetch_work_cached_record(context.records, context.client, key).await {
        Ok(found) => Ok(Some(found)),
        Err(error) if is_soft(&error) => {
            warnings.push(format!("provider_not_found: {key}"));
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) const fn is_soft(error: &ProviderError) -> bool {
    error.is_not_found() || matches!(error, ProviderError::Malformed)
}

async fn attach_provider_identifiers(
    works: &WorkRepository,
    work_id: WorkId,
    provider: &OpenAlexWork,
    warnings: &mut Vec<String>,
) -> Result<()> {
    attach_identifier(
        works,
        work_id,
        IdentifierScheme::Openalex,
        &provider.openalex_id,
        warnings,
    )
    .await?;
    if let Some(doi) = &provider.doi {
        attach_identifier(works, work_id, IdentifierScheme::Doi, doi, warnings).await?;
    }
    Ok(())
}

/// Attach an identifier best-effort. An identifier another Work already owns
/// is reported as a warning, never an error.
async fn attach_identifier(
    works: &WorkRepository,
    work_id: WorkId,
    scheme: IdentifierScheme,
    value: &str,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let attached = works
        .add_identifier(work_id, scheme, value, None, IdentifierStatus::Active)
        .await?;
    if attached.is_some() {
        return Ok(());
    }
    let owner = works.find_by_identifier(scheme, value).await?;
    if owner.is_some_and(|owner| owner.id != work_id) {
        warnings.push(format!("identifier_conflict: {scheme}:{value}"));
    }
    Ok(())
}

/// The paper's own DOI for the publisher lookup: the provider's, else the
/// first detected one that Crossref can hold.
fn own_publisher_doi(flow: &Flow, detected: &[DetectedIdentifier]) -> Option<String> {
    let provider = flow
        .provider
        .as_ref()
        .and_then(|(_, provider)| provider.doi.clone());
    provider
        .into_iter()
        .chain(
            detected
                .iter()
                .filter(|identifier| identifier.scheme == IdentifierScheme::Doi)
                .map(|identifier| identifier.value.clone()),
        )
        .filter_map(|doi| normalize(IdentifierScheme::Doi, &doi))
        .find(|doi| is_crossref_doi(doi))
}

/// Look one DOI up at Crossref and record its publisher claims on `work`.
///
/// Not-found and malformed answers add `publisher_not_found:<doi>`; transport
/// and server failures add `publisher_fetch_failed:<doi>`. Neither fails the
/// flow. `None` when Crossref does not hold the DOI at all.
async fn fetch_publisher<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    work: WorkId,
    doi: &str,
    cited: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<PublisherFetch>> {
    let outcome = match fetch_crossref_cached(context.records, context.client, doi).await {
        Ok(CrossrefOutcome::Skipped) => return Ok(None),
        Ok(CrossrefOutcome::Fetched(record, crossref)) => {
            record_publisher_claims(context.claims, work, record, &crossref).await?;
            PublisherOutcome::Fetched
        }
        Ok(CrossrefOutcome::Cached(record, crossref)) => {
            record_publisher_claims(context.claims, work, record, &crossref).await?;
            PublisherOutcome::Cached
        }
        Ok(CrossrefOutcome::NotFound { .. }) => PublisherOutcome::NotFound,
        Err(error) if is_soft(&error) => PublisherOutcome::NotFound,
        Err(ProviderError::Db(error)) => return Err(error.into()),
        Err(_) => PublisherOutcome::Failed,
    };
    match outcome {
        PublisherOutcome::NotFound => warnings.push(format!("publisher_not_found:{doi}")),
        PublisherOutcome::Failed => warnings.push(format!("publisher_fetch_failed:{doi}")),
        PublisherOutcome::Fetched | PublisherOutcome::Cached => {}
    }
    Ok(Some(PublisherFetch {
        doi: doi.to_owned(),
        outcome,
        cited,
    }))
}

/// Publisher lookups for the cited Works' active DOIs, sequentially. Stops
/// after [`PUBLISHER_FETCH_MAX`] requests; cached answers cost none.
async fn fetch_cited_publishers<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    cited: &[Work],
    fetches: &mut Vec<PublisherFetch>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut requests = 0;
    for work in cited {
        if requests >= PUBLISHER_FETCH_MAX {
            warnings.push("publisher_fetch_limit_reached".to_owned());
            return Ok(());
        }
        let identifiers = context.works.active_identifiers(work.id).await?;
        let Some(doi) = identifiers
            .iter()
            .find(|identifier| identifier.scheme == IdentifierScheme::Doi)
            .map(|identifier| identifier.value_normalized.as_str())
            .filter(|doi| is_crossref_doi(doi))
        else {
            continue;
        };
        let Some(fetch) = fetch_publisher(context, work.id, doi, true, warnings).await? else {
            continue;
        };
        if fetch.outcome != PublisherOutcome::Cached {
            requests += 1;
        }
        fetches.push(fetch);
    }
    Ok(())
}

/// Step 6: record claims for the paper's own Work: the provider record
/// (`openalex`), the catalog metadata (`library_metadata`), and the title and
/// authors its front matter confirms (`paper`). Selection ranks them.
async fn fill_display_fields<T: JsonTransport>(
    context: &LiteratureContext<'_, T>,
    work_id: WorkId,
    flow: &Flow,
    input: &LiteratureInput<'_>,
) -> Result<()> {
    let hints = input.hints;
    let provider = flow.provider.as_ref();
    let provider_record = provider.map(|(record, _)| *record);
    let provider_title = provider.and_then(|(_, work)| work.title.clone());
    let provider_year = provider.and_then(|(_, work)| work.year);
    let hint_title = hints
        .title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);

    let provider_values: Vec<FieldValue> = provider_title
        .map(FieldValue::Title)
        .into_iter()
        .chain(provider_year.map(FieldValue::Year))
        .collect();
    let hint_values: Vec<FieldValue> = hint_title
        .map(FieldValue::Title)
        .into_iter()
        .chain(hints.year.map(FieldValue::Year))
        .collect();
    let provider_claims = provider_values.iter().map(|value| NewClaim {
        work_id,
        field: value.field(),
        value,
        origin: ClaimOrigin::Openalex,
        extractor: "openalex",
        confidence: ClaimConfidence::Explicit,
        source_record_id: provider_record,
        entry_id: None,
        locator: None,
    });
    let hint_claims = hint_values.iter().map(|value| NewClaim {
        work_id,
        field: value.field(),
        value,
        origin: ClaimOrigin::LibraryMetadata,
        extractor: "catalog-metadata",
        confidence: ClaimConfidence::Heuristic,
        source_record_id: None,
        entry_id: None,
        locator: None,
    });

    let front = front_matter_fields(input.front_matter, hints);
    let front_record = if front.title.is_some() || !front.authors.is_empty() {
        let payload = json!({
            "extractor": FRONT_MATTER_EXTRACTOR,
            "front_matter_sha256": format!("{:x}", Sha256::digest(input.front_matter.as_bytes())),
            "title": front.title,
            "authors": front.authors,
        });
        let key = format!("{}|{FRONT_MATTER_EXTRACTOR}", input.file.sha256);
        Some(
            context
                .records
                .upsert(PAPER_FRONT_MATTER_KIND, &key, Coverage::Complete, &payload)
                .await?,
        )
    } else {
        None
    };
    let front_values: Vec<FieldValue> = front
        .title
        .map(FieldValue::Title)
        .into_iter()
        .chain((!front.authors.is_empty()).then_some(FieldValue::Authors(front.authors)))
        .collect();
    let front_claims = front_values.iter().map(|value| NewClaim {
        work_id,
        field: value.field(),
        value,
        origin: ClaimOrigin::Paper,
        extractor: FRONT_MATTER_EXTRACTOR,
        confidence: ClaimConfidence::Heuristic,
        source_record_id: front_record,
        entry_id: None,
        locator: Some("front_matter"),
    });

    let new_claims: Vec<NewClaim<'_>> = provider_claims
        .chain(hint_claims)
        .chain(front_claims)
        .collect();
    context.claims.record(&new_claims).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lysilogy_core::{DocumentFormat, front_matter};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::openalex::{ProviderFuture, work_url};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Alphanumeric only, so it survives URL percent-encoding unchanged.
    fn marker() -> String {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        format!("x{}x{sequence}x{nanos}", std::process::id())
    }

    #[derive(Clone)]
    enum Reply {
        Json(Value),
        Status(u16),
    }

    /// Replies by the first route whose key the URL contains; 404 otherwise.
    #[derive(Clone)]
    struct RoutingTransport {
        routes: Arc<Vec<(String, Reply)>>,
        urls: Arc<Mutex<Vec<String>>>,
    }

    impl RoutingTransport {
        fn new(routes: Vec<(String, Reply)>) -> Self {
            Self {
                routes: Arc::new(routes),
                urls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.lock().expect("lock").clone()
        }
    }

    impl JsonTransport for RoutingTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async move {
                self.urls.lock().expect("lock").push(url.to_owned());
                let reply = self
                    .routes
                    .iter()
                    .find(|(key, _)| url.contains(key.as_str()))
                    .map(|(_, reply)| reply.clone());
                match reply {
                    Some(Reply::Json(value)) => Ok(value),
                    Some(Reply::Status(status)) => Err(ProviderError::Status(status)),
                    None => Err(ProviderError::Status(404)),
                }
            })
        }
    }

    fn batch_route(results: &Value) -> (String, Reply) {
        (
            "filter=openalex:".to_owned(),
            Reply::Json(json!({ "results": results })),
        )
    }

    fn search_route(results: &Value) -> (String, Reply) {
        (
            "filter=title.search".to_owned(),
            Reply::Json(json!({ "results": results })),
        )
    }

    fn work_route(url: &str, openalex_id: &str, title: &str, refs: &[String]) -> (String, Reply) {
        let referenced: Vec<String> = refs
            .iter()
            .map(|id| format!("https://openalex.org/{id}"))
            .collect();
        (
            url.to_owned(),
            Reply::Json(json!({
                "id": format!("https://openalex.org/{openalex_id}"),
                "display_name": title,
                "publication_year": 2013,
                "referenced_works": referenced,
            })),
        )
    }

    fn summary_json(id: &str, title: &str) -> Value {
        json!({"id": format!("https://openalex.org/{id}"), "display_name": title,
               "publication_year": 2011, "authorships": []})
    }

    fn summary_json_with_doi(id: &str, title: &str, doi: &str) -> Value {
        let mut value = summary_json(id, title);
        value["doi"] = json!(format!("https://doi.org/{doi}"));
        value
    }

    fn crossref_route(doi: &str, title: &str, author: &str) -> (String, Reply) {
        (
            format!("api.crossref.org/works/{doi}"),
            Reply::Json(json!({"message": {
                "DOI": doi,
                "title": [title],
                "author": [{"given": author, "family": "Publisher"}],
                "container-title": ["Journal of Tests"],
                "issued": {"date-parts": [[2014]]}
            }})),
        )
    }

    struct Repos {
        works: WorkRepository,
        documents: DocumentRepository,
        citations: CitationRepository,
        claims: ClaimRepository,
        records: SourceRecordRepository,
    }

    impl Repos {
        async fn connect() -> Option<Self> {
            let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
                .ok()
                .filter(|url| !url.is_empty())?;
            let pool = lysilogy_db::connect(&url).await.expect("connect");
            lysilogy_db::migrate(&pool).await.expect("migrate");
            Some(Self {
                works: WorkRepository::new(pool.clone()),
                documents: DocumentRepository::new(pool.clone()),
                citations: CitationRepository::new(pool.clone()),
                claims: ClaimRepository::new(pool.clone()),
                records: SourceRecordRepository::new(pool),
            })
        }

        async fn run(
            &self,
            transport: &RoutingTransport,
            file: ScannedFile,
            front_matter: &str,
            hints: PaperHints<'_>,
        ) -> LiteratureSummary {
            self.run_with(transport, file, front_matter, hints, false)
                .await
        }

        async fn run_with(
            &self,
            transport: &RoutingTransport,
            file: ScannedFile,
            front_matter: &str,
            hints: PaperHints<'_>,
            publisher_cited: bool,
        ) -> LiteratureSummary {
            let client = OpenAlexClient::new(transport.clone());
            let context = LiteratureContext {
                works: &self.works,
                documents: &self.documents,
                citations: &self.citations,
                claims: &self.claims,
                records: &self.records,
                client: &client,
            };
            run_literature_flow(
                &context,
                LiteratureInput {
                    file,
                    front_matter,
                    hints,
                    publisher_cited,
                },
            )
            .await
            .expect("flow")
        }
    }

    fn file(marker: &str) -> ScannedFile {
        ScannedFile {
            relative_path: format!("lit-tests/{marker}.pdf"),
            sha256: format!("sha-{marker}"),
            bytes: 1,
            format: DocumentFormat::Pdf,
        }
    }

    /// The fixture shared by most tests: a DOI paper citing two stubs.
    struct DoiPaper {
        marker: String,
        doi: String,
        openalex_id: String,
        refs: [String; 2],
        title: String,
    }

    impl DoiPaper {
        fn new() -> Self {
            let marker = marker();
            Self {
                doi: format!("10.5555/{marker}"),
                openalex_id: format!("W9{marker}"),
                refs: [format!("W81{marker}"), format!("W82{marker}")],
                title: format!("The Study of Things {marker}"),
                marker,
            }
        }

        fn front_matter(&self) -> String {
            format!("{}\nDOI: {}\nAbstract text", self.title, self.doi)
        }

        fn cited_doi(&self, index: usize) -> String {
            format!("10.6666/ref{index}{}", self.marker)
        }

        fn routes(&self) -> Vec<(String, Reply)> {
            vec![
                crossref_route(&self.doi, &self.title, "Pat"),
                batch_route(&json!([
                    summary_json_with_doi(&self.refs[0], "First Cited Paper", &self.cited_doi(0)),
                    summary_json_with_doi(&self.refs[1], "Second Cited Paper", &self.cited_doi(1)),
                ])),
                work_route(
                    &work_url(&format!("doi:{}", self.doi)),
                    &self.openalex_id,
                    &self.title,
                    &self.refs,
                ),
            ]
        }

        fn hints(&self) -> PaperHints<'_> {
            PaperHints {
                title: Some(self.title.as_str()),
                year: Some(2013),
                first_author: Some("Autor"),
                authors: &[],
            }
        }
    }

    #[tokio::test]
    async fn should_link_and_ingest_references_when_front_matter_has_doi() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let transport = RoutingTransport::new(paper.routes());
        let summary = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;

        assert_eq!(summary.status, LiteratureStatus::Linked);
        assert_eq!(summary.link_method, Some(WorkLinkMethod::Identifier));
        assert_eq!(summary.link_detail, Some(format!("doi:{}", paper.doi)));
        assert_eq!(
            summary.openalex_id.as_deref(),
            Some(paper.openalex_id.as_str())
        );
        assert_eq!(summary.references, 2);
        assert_eq!(summary.new_stub_works, 2);
        assert_eq!(summary.hydrated_works, 2);
        assert!(summary.warnings.is_empty(), "{:?}", summary.warnings);

        let work_id = summary.work_id.expect("work");
        let work = repos.works.get(work_id).await.expect("get").expect("work");
        assert_eq!(work.display_title.as_deref(), Some(paper.title.as_str()));
        assert_eq!(
            work.display_year,
            Some(2014),
            "the publisher year outranks OpenAlex"
        );
        let by_openalex = repos
            .works
            .find_by_identifier(IdentifierScheme::Openalex, &paper.openalex_id)
            .await
            .expect("find")
            .expect("openalex identifier");
        assert_eq!(by_openalex.id, work_id);
        let link = repos
            .documents
            .work_link(summary.document_id)
            .await
            .expect("link")
            .expect("recorded");
        assert_eq!(link.work_id, work_id);
        assert_eq!(link.method, WorkLinkMethod::Identifier);

        let mut cited: Vec<String> = repos
            .citations
            .outgoing(work_id)
            .await
            .expect("outgoing")
            .into_iter()
            .filter_map(|work| work.display_title)
            .collect();
        cited.sort();
        assert_eq!(cited, ["First Cited Paper", "Second Cited Paper"]);
    }

    #[tokio::test]
    async fn should_be_idempotent_when_run_twice() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let transport = RoutingTransport::new(paper.routes());
        let first = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;
        let edges_after_first = repos
            .citations
            .outgoing(first.work_id.expect("work"))
            .await
            .expect("outgoing")
            .len();
        let second = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;

        assert_eq!(second.work_id, first.work_id);
        assert_eq!(second.document_id, first.document_id);
        assert_eq!(second.new_stub_works, 0);
        assert_eq!(second.hydrated_works, 0);
        assert_eq!(second.references, first.references);
        let edges_after_second = repos
            .citations
            .outgoing(second.work_id.expect("work"))
            .await
            .expect("outgoing")
            .len();
        assert_eq!(edges_after_second, edges_after_first);
    }

    #[tokio::test]
    async fn should_fall_back_to_title_search_when_provider_title_disagrees() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let right_id = format!("W7{}", paper.marker);
        let wrong_id = format!("W6{}", paper.marker);
        let title = format!("Concrete Problems in AI Safety {}", paper.marker);
        let title = title.as_str();
        let transport = RoutingTransport::new(vec![
            batch_route(&json!([summary_json(&paper.refs[0], "Cited One")])),
            work_route(
                &work_url(&format!("doi:{}", paper.doi)),
                &wrong_id,
                "Agnostic Learning with Unknown Utilities",
                &[],
            ),
            search_route(&json!([{
                "id": format!("https://openalex.org/{right_id}"),
                "display_name": title,
                "publication_year": 2016,
                "referenced_works_count": 1,
                "authorships": [{"author": {"display_name": "Dario Amodei"}}],
            }])),
            work_route(&work_url(&right_id), &right_id, title, &paper.refs[..1]),
        ]);
        let summary = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                PaperHints {
                    title: Some(title),
                    year: Some(2016),
                    first_author: Some("Amodei"),
                    authors: &[],
                },
            )
            .await;

        assert!(
            summary
                .warnings
                .iter()
                .any(|warning| warning.starts_with("provider_title_mismatch")),
            "{:?}",
            summary.warnings
        );
        assert_eq!(summary.openalex_id.as_deref(), Some(right_id.as_str()));
        assert_eq!(summary.references, 1);
        let wrong = repos
            .works
            .find_by_identifier(IdentifierScheme::Openalex, &wrong_id)
            .await
            .expect("find");
        assert!(wrong.is_none(), "the mis-merged record must not be linked");
    }

    #[tokio::test]
    async fn should_fall_back_to_title_search_when_doi_returns_not_found() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let found_id = format!("W7{}", paper.marker);
        let transport = RoutingTransport::new(vec![
            batch_route(&json!([summary_json(&paper.refs[0], "Cited One")])),
            search_route(&json!([{
                "id": format!("https://openalex.org/{found_id}"),
                "display_name": &paper.title,
                "publication_year": 2013,
                "referenced_works_count": 1,
                "authorships": [{"author": {"display_name": "David Autor"}}],
            }])),
            work_route(
                &work_url(&found_id),
                &found_id,
                &paper.title,
                &paper.refs[..1],
            ),
            (paper.marker.clone(), Reply::Status(404)),
        ]);
        let summary = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;

        assert!(
            summary
                .warnings
                .iter()
                .any(|warning| warning.starts_with("provider_not_found")),
            "{:?}",
            summary.warnings
        );
        assert_eq!(summary.openalex_id.as_deref(), Some(found_id.as_str()));
        assert_eq!(summary.references, 1);
        assert_eq!(summary.status, LiteratureStatus::Linked);
    }

    #[tokio::test]
    async fn should_link_by_title_match_when_no_identifier_and_title_matches() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let found_id = format!("W7{}", paper.marker);
        let transport = RoutingTransport::new(vec![
            batch_route(&json!([summary_json(&paper.refs[0], "Cited One")])),
            search_route(&json!([{
                "id": format!("https://openalex.org/{found_id}"),
                "display_name": &paper.title,
                "publication_year": 2013,
                "referenced_works_count": 1,
                "authorships": [{"author": {"display_name": "David Autor"}}],
            }])),
            work_route(
                &work_url(&found_id),
                &found_id,
                &paper.title,
                &paper.refs[..1],
            ),
        ]);
        let summary = repos
            .run(
                &transport,
                file(&paper.marker),
                "A paper with no identifiers on its first page.",
                paper.hints(),
            )
            .await;

        assert_eq!(summary.status, LiteratureStatus::Linked);
        assert_eq!(summary.link_method, Some(WorkLinkMethod::TitleMatch));
        assert_eq!(
            summary.link_detail.as_deref(),
            Some(format!("title_key={};candidates=1", title_key(&paper.title)).as_str())
        );
        assert_eq!(summary.openalex_id.as_deref(), Some(found_id.as_str()));
        assert_eq!(summary.references, 1);
    }

    #[tokio::test]
    async fn should_return_unmatched_when_no_identifier_and_title_generic() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = RoutingTransport::new(Vec::new());
        let summary = repos
            .run(
                &transport,
                file(&marker()),
                "Nothing identifiable here.",
                PaperHints {
                    title: Some("Introduction"),
                    year: None,
                    first_author: None,
                    authors: &[],
                },
            )
            .await;

        assert_eq!(
            summary.status,
            LiteratureStatus::Unmatched {
                reason: "title_too_generic".to_owned()
            }
        );
        assert_eq!(summary.work_id, None);
        assert_eq!(summary.link_method, None);
        assert!(
            transport.urls().is_empty(),
            "a generic title is never searched"
        );
    }

    #[tokio::test]
    async fn should_return_unmatched_when_no_identifier_and_no_title() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = RoutingTransport::new(Vec::new());
        let summary = repos
            .run(&transport, file(&marker()), "", PaperHints::default())
            .await;
        assert_eq!(
            summary.status,
            LiteratureStatus::Unmatched {
                reason: "no_identifier_or_title".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn should_reuse_existing_stub_work_when_paper_was_already_cited() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let (stub, _) = repos
            .works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                &paper.openalex_id,
                WorkKind::Article,
                None,
            )
            .await
            .expect("stub");
        let transport = RoutingTransport::new(paper.routes());
        let summary = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;
        assert_eq!(summary.work_id, Some(stub.id));
        let by_doi = repos
            .works
            .find_by_identifier(IdentifierScheme::Doi, &paper.doi)
            .await
            .expect("find")
            .expect("doi attached to the stub");
        assert_eq!(by_doi.id, stub.id);
    }

    #[tokio::test]
    async fn should_not_tombstone_other_documents_when_running_lit() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let workspace = repos
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let others = [file(&marker()), file(&marker())];
        repos
            .documents
            .apply_partial_scan(workspace, &others)
            .await
            .expect("register others");

        let paper = DoiPaper::new();
        let transport = RoutingTransport::new(paper.routes());
        repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;

        for other in &others {
            let document = repos
                .documents
                .find_by_path(workspace, &other.relative_path)
                .await
                .expect("find");
            assert!(document.is_some(), "{} was tombstoned", other.relative_path);
        }
    }

    #[tokio::test]
    async fn should_ignore_reference_dois_when_they_appear_after_references_heading() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let reference_doi = format!("10.1111/{}ref", paper.marker);
        let pages = [
            paper.front_matter(),
            format!("More text.\nReferences\n[1] Someone. doi:{reference_doi}"),
        ];
        let page_refs: Vec<&str> = pages.iter().map(String::as_str).collect();
        let cut = front_matter(&page_refs);
        let transport = RoutingTransport::new(paper.routes());
        let summary = repos
            .run(&transport, file(&paper.marker), &cut, paper.hints())
            .await;

        assert_eq!(summary.link_detail, Some(format!("doi:{}", paper.doi)));
        let reference_owner = repos
            .works
            .find_by_identifier(IdentifierScheme::Doi, &reference_doi)
            .await
            .expect("find");
        assert!(reference_owner.is_none());
        assert!(
            transport
                .urls()
                .iter()
                .all(|url| !url.contains(&format!("{}ref", paper.marker))),
            "{:?}",
            transport.urls()
        );
    }

    #[tokio::test]
    async fn should_prefer_openalex_then_doi_then_arxiv_when_resolving_paper_work() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = DetectedIdentifier {
            scheme: IdentifierScheme::Doi,
            value: format!("10.5555/{marker}"),
        };
        let openalex = format!("W5{marker}");

        let none = resolve_paper_work(&repos.works, None, None)
            .await
            .expect("none");
        assert!(none.is_none());

        let created = resolve_paper_work(&repos.works, None, Some(&doi))
            .await
            .expect("create")
            .expect("work");
        let again = resolve_paper_work(&repos.works, None, Some(&doi))
            .await
            .expect("again")
            .expect("work");
        assert_eq!(again.id, created.id);

        let (owner, _) = repos
            .works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                &openalex,
                WorkKind::Article,
                None,
            )
            .await
            .expect("owner");
        let by_openalex = resolve_paper_work(&repos.works, Some(&openalex), Some(&doi))
            .await
            .expect("by openalex")
            .expect("work");
        assert_eq!(by_openalex.id, owner.id, "OpenAlex ID wins over the DOI");
    }

    #[tokio::test]
    async fn should_attach_doi_and_arxiv_to_one_work_when_both_are_on_the_front_page() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        let arxiv = format!("{:04}.{:05}", (nanos / 100_000) % 10_000, nanos % 100_000);
        let text = format!("{}\narXiv:{arxiv}v2", paper.front_matter());
        let transport = RoutingTransport::new(paper.routes());
        let summary = repos
            .run(&transport, file(&paper.marker), &text, paper.hints())
            .await;
        let work_id = summary.work_id.expect("work");
        for (scheme, value) in [
            (IdentifierScheme::Doi, paper.doi.as_str()),
            (IdentifierScheme::Arxiv, arxiv.as_str()),
        ] {
            let owner = repos
                .works
                .find_by_identifier(scheme, value)
                .await
                .expect("find")
                .expect("attached");
            assert_eq!(owner.id, work_id);
        }
    }

    #[tokio::test]
    async fn should_write_paper_claims_that_outrank_openalex_when_front_matter_matches() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let authors = vec!["Ada Lovelace".to_owned(), "Grace Hopper".to_owned()];
        let front = format!(
            "{}\nAda Lovelace   Grace Hopper\nDOI: {}\nAbstract text",
            paper.title, paper.doi
        );
        let mut routes = paper.routes();
        // The provider title differs only in case, so it is not a mismatch.
        routes[0] = work_route(
            &work_url(&format!("doi:{}", paper.doi)),
            &paper.openalex_id,
            &paper.title.to_uppercase(),
            &paper.refs,
        );
        routes.remove(1);
        routes.push(batch_route(&json!([])));
        let transport = RoutingTransport::new(routes);
        let hints = PaperHints {
            authors: &authors,
            ..paper.hints()
        };
        let summary = repos
            .run(&transport, file(&paper.marker), &front, hints)
            .await;
        let work_id = summary.work_id.expect("work");

        let work = repos.works.get(work_id).await.expect("get").expect("work");
        assert_eq!(work.display_title.as_deref(), Some(paper.title.as_str()));
        assert_eq!(work.display_authors, Some(authors.clone()));
        let claims = repos.claims.claims_for(work_id).await.expect("claims");
        let openalex_title = claims
            .iter()
            .find(|claim| claim.origin == ClaimOrigin::Openalex && claim.field.as_str() == "title")
            .expect("openalex title claim");
        assert_eq!(openalex_title.value, json!(paper.title.to_uppercase()));
        let paper_claims: Vec<_> = claims
            .iter()
            .filter(|claim| claim.origin == ClaimOrigin::Paper)
            .collect();
        assert_eq!(paper_claims.len(), 2, "title and authors, no year");
        for claim in paper_claims {
            assert_eq!(claim.extractor, "frontmatter-v1");
            assert_eq!(claim.confidence, ClaimConfidence::Heuristic);
            assert_eq!(claim.locator.as_deref(), Some("front_matter"));
            assert!(claim.source_record_id.is_some());
        }
        let payload = repos
            .records
            .get(
                PAPER_FRONT_MATTER_KIND,
                &format!("sha-{}|frontmatter-v1", paper.marker),
            )
            .await
            .expect("record")
            .expect("payload");
        assert_eq!(payload["extractor"], "frontmatter-v1");
        assert_eq!(payload["title"], json!(paper.title));
        assert_eq!(payload["authors"], json!(authors));
        assert!(payload["front_matter_sha256"].is_string());
    }

    #[tokio::test]
    async fn should_fetch_publisher_record_for_own_work_when_doi_known() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let transport = RoutingTransport::new(paper.routes());
        let first = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;
        assert_eq!(
            first.publisher,
            [PublisherFetch {
                doi: paper.doi.clone(),
                outcome: PublisherOutcome::Fetched,
                cited: false,
            }]
        );
        let work_id = first.work_id.expect("work");
        let claims = repos.claims.claims_for(work_id).await.expect("claims");
        let publisher: Vec<_> = claims
            .iter()
            .filter(|claim| claim.origin == ClaimOrigin::Publisher)
            .collect();
        assert_eq!(publisher.len(), 5, "title, year, authors, venue, doi");
        assert!(
            publisher
                .iter()
                .all(|claim| claim.extractor == "crossref-v1")
        );
        let work = repos.works.get(work_id).await.expect("get").expect("work");
        assert_eq!(work.display_year, Some(2014), "publisher outranks OpenAlex");
        assert_eq!(work.display_authors, Some(vec!["Pat Publisher".to_owned()]));

        let second = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;
        assert_eq!(second.publisher.len(), 1);
        assert_eq!(second.publisher[0].outcome, PublisherOutcome::Cached);
        let crossref_requests = transport
            .urls()
            .iter()
            .filter(|url| url.contains("api.crossref.org"))
            .count();
        assert_eq!(crossref_requests, 1);
    }

    #[tokio::test]
    async fn should_fetch_publisher_for_cited_works_only_when_requested() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let mut routes = paper.routes();
        routes.insert(
            0,
            crossref_route(&paper.cited_doi(0), "Cited Publisher Title", "Cy"),
        );
        let transport = RoutingTransport::new(routes);
        let without = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;
        assert!(without.publisher.iter().all(|fetch| !fetch.cited));
        assert!(
            transport
                .urls()
                .iter()
                .all(|url| !url.contains(&format!("ref0{}", paper.marker))),
            "{:?}",
            transport.urls()
        );

        let with = repos
            .run_with(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
                true,
            )
            .await;
        let cited: Vec<&PublisherFetch> =
            with.publisher.iter().filter(|fetch| fetch.cited).collect();
        assert_eq!(cited.len(), 2, "{:?}", with.publisher);
        let outcome_of = |doi: &str| {
            cited
                .iter()
                .find(|fetch| fetch.doi == doi)
                .map(|fetch| fetch.outcome)
        };
        assert_eq!(
            outcome_of(&paper.cited_doi(0)),
            Some(PublisherOutcome::Fetched)
        );
        assert_eq!(
            outcome_of(&paper.cited_doi(1)),
            Some(PublisherOutcome::NotFound)
        );
        assert!(
            with.warnings
                .contains(&format!("publisher_not_found:{}", paper.cited_doi(1)))
        );
        let first_cited = repos
            .works
            .find_by_identifier(IdentifierScheme::Doi, &paper.cited_doi(0))
            .await
            .expect("find")
            .expect("cited work");
        let cited_work = repos
            .works
            .get(first_cited.id)
            .await
            .expect("get")
            .expect("work");
        assert_eq!(
            cited_work.display_title.as_deref(),
            Some("Cited Publisher Title")
        );
    }

    #[tokio::test]
    async fn should_warn_and_continue_when_publisher_fetch_fails_hard() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let paper = DoiPaper::new();
        let mut routes = paper.routes();
        routes[0] = (
            format!("api.crossref.org/works/{}", paper.doi),
            Reply::Status(503),
        );
        let transport = RoutingTransport::new(routes);
        let summary = repos
            .run(
                &transport,
                file(&paper.marker),
                &paper.front_matter(),
                paper.hints(),
            )
            .await;
        assert_eq!(summary.status, LiteratureStatus::Linked);
        assert_eq!(summary.references, 2);
        assert_eq!(summary.publisher.len(), 1);
        assert_eq!(summary.publisher[0].outcome, PublisherOutcome::Failed);
        assert_eq!(
            summary.warnings,
            [format!("publisher_fetch_failed:{}", paper.doi)]
        );
    }

    #[test]
    fn should_derive_datacite_doi_when_identifier_is_arxiv() {
        let arxiv = DetectedIdentifier {
            scheme: IdentifierScheme::Arxiv,
            value: "1606.06565".to_owned(),
        };
        assert_eq!(provider_key(&arxiv), "doi:10.48550/arxiv.1606.06565");
        let doi = DetectedIdentifier {
            scheme: IdentifierScheme::Doi,
            value: "10.1257/aer.103.6.2121".to_owned(),
        };
        assert_eq!(provider_key(&doi), "doi:10.1257/aer.103.6.2121");
    }
}
