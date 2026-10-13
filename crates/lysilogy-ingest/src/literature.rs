//! The literature flow for one local paper: register the file, link it to a
//! Work, fetch its `OpenAlex` record, ingest its references, and title the
//! stub Works those references created.

use lysilogy_core::{
    DetectedIdentifier, DocumentId, FieldKind, FieldState, IdentifierScheme, IdentifierStatus,
    ScannedFile, SourceRecordId, Work, WorkId, WorkKind, WorkLinkMethod, extract_identifiers,
    title_key,
};
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::document::DocumentRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use serde::Serialize;

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
}

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
    fill_display_fields(context.works, work.id, &flow, input.hints).await?;

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
        warnings: Vec::new(),
    };
    if let Some((record, provider)) = &flow.provider {
        let report = ingest_openalex_references(
            context.works,
            context.citations,
            work.id,
            provider,
            *record,
        )
        .await?;
        summary.references = report.edges;
        summary.new_stub_works = report.new_works;
        let cited = context.citations.outgoing(work.id).await?;
        summary.hydrated_works =
            hydrate_stub_works(context.works, context.records, context.client, &cited).await?;
    }
    summary.warnings = flow.warnings;
    Ok(summary)
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

const fn is_soft(error: &ProviderError) -> bool {
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

/// Step 6: record title and year claims and fill the display fields when the
/// Work has none yet, preferring the provider over the paper's own metadata.
async fn fill_display_fields(
    works: &WorkRepository,
    work_id: WorkId,
    flow: &Flow,
    hints: PaperHints<'_>,
) -> Result<()> {
    let provider = flow.provider.as_ref();
    let provider_record = provider.map(|(record, _)| *record);
    let provider_title = provider.and_then(|(_, work)| work.title.as_deref());
    let provider_year = provider.and_then(|(_, work)| work.year);
    let hint_title = hints.title.map(str::trim).filter(|title| !title.is_empty());

    if let Some(title) = provider_title {
        works
            .add_claim_once(
                work_id,
                FieldKind::Title,
                &serde_json::json!(title),
                FieldState::Known,
                provider_record,
                None,
            )
            .await?;
    }
    if let Some(title) = hint_title {
        works
            .add_claim_once(
                work_id,
                FieldKind::Title,
                &serde_json::json!(title),
                FieldState::Known,
                None,
                None,
            )
            .await?;
    }
    if let Some(title) = provider_title.or(hint_title) {
        works.fill_display_title_if_unknown(work_id, title).await?;
    }
    if let Some(year) = provider_year {
        works
            .add_claim_once(
                work_id,
                FieldKind::Year,
                &serde_json::json!(year),
                FieldState::Known,
                provider_record,
                None,
            )
            .await?;
    }
    if let Some(year) = hints.year {
        works
            .add_claim_once(
                work_id,
                FieldKind::Year,
                &serde_json::json!(year),
                FieldState::Known,
                None,
                None,
            )
            .await?;
    }
    if let Some(year) = provider_year.or(hints.year) {
        works.fill_display_year_if_unknown(work_id, year).await?;
    }
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
               "publication_year": 2011})
    }

    struct Repos {
        works: WorkRepository,
        documents: DocumentRepository,
        citations: CitationRepository,
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
            let client = OpenAlexClient::new(transport.clone());
            let context = LiteratureContext {
                works: &self.works,
                documents: &self.documents,
                citations: &self.citations,
                records: &self.records,
                client: &client,
            };
            run_literature_flow(
                &context,
                LiteratureInput {
                    file,
                    front_matter,
                    hints,
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

        fn routes(&self) -> Vec<(String, Reply)> {
            vec![
                batch_route(&json!([
                    summary_json(&self.refs[0], "First Cited Paper"),
                    summary_json(&self.refs[1], "Second Cited Paper"),
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
        assert_eq!(work.display_year, Some(2013));
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
