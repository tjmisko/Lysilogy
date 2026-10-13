//! Literature endpoints: paper-to-Work lookup, the `lit` job, and local
//! availability of Works.

use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use lysilogy_core::{Work, WorkId, WorkLinkMethod};
use lysilogy_ingest::literature::LiteratureSummary;
use serde::Serialize;
use tokio::sync::Mutex;

use super::{
    AppState, GraphError, fetch_work, graph_or_unavailable, graph_query_failure, parse_id,
};
use crate::domain::PaperId;

/// A paper held locally, as shown next to a Work. Never carries a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct LocalPaper {
    pub paper_id: PaperId,
    pub title: String,
}

/// Local papers keyed by Work id. Works without a local paper have no entry.
pub(super) type LocalPapers = BTreeMap<String, Vec<LocalPaper>>;

#[derive(Clone, Debug)]
pub enum LiteratureJobState {
    Running,
    Succeeded(LiteratureSummary),
    Failed(String),
}

/// The state of each paper's most recent literature job.
#[derive(Clone, Debug, Default)]
pub struct LiteratureJobs(Arc<Mutex<HashMap<PaperId, LiteratureJobState>>>);

impl LiteratureJobs {
    /// Start the flow unless one is already running. Returns the current state.
    pub async fn start(&self, state: AppState, id: PaperId) -> LiteratureJobState {
        let mut jobs = self.0.lock().await;
        if matches!(jobs.get(&id), Some(LiteratureJobState::Running)) {
            return LiteratureJobState::Running;
        }
        jobs.insert(id.clone(), LiteratureJobState::Running);
        drop(jobs);

        let registry = self.clone();
        tokio::spawn(async move {
            let worker_state = state.clone();
            let worker_id = id.clone();
            let outcome =
                tokio::spawn(async move { worker_state.run_literature_flow(&worker_id).await })
                    .await;
            let finished = match outcome {
                Ok(Ok(summary)) => LiteratureJobState::Succeeded(summary),
                Ok(Err(error)) => LiteratureJobState::Failed(error.to_string()),
                Err(error) => LiteratureJobState::Failed(format!("the job stopped: {error}")),
            };
            registry.0.lock().await.insert(id, finished);
        });
        LiteratureJobState::Running
    }

    async fn get(&self, id: &PaperId) -> Option<LiteratureJobState> {
        self.0.lock().await.get(id).cloned()
    }
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/papers/{id}/work", get(get_paper_work))
        .route(
            "/api/papers/{id}/lit",
            get(literature_status).post(start_literature),
        )
}

#[derive(Serialize)]
struct PaperLink {
    method: WorkLinkMethod,
    detail: Option<String>,
    linked_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct PaperWorkResponse {
    paper_id: PaperId,
    work: Option<Work>,
    link: Option<PaperLink>,
}

async fn get_paper_work(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let paper_id = match parse_id(&id) {
        Ok(paper_id) => paper_id,
        Err(error) => return error.into_response(),
    };
    let Some(relative_path) = state
        .catalog
        .read()
        .await
        .get(&paper_id)
        .map(|entry| entry.overview.relative_path.clone())
    else {
        return crate::Error::PaperNotFound(paper_id.to_string()).into_response();
    };
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let linked = async {
        let workspace = graph.documents.ensure_default_workspace().await?;
        let Some(document) = graph
            .documents
            .find_by_path(workspace, &relative_path)
            .await?
        else {
            return Ok(None);
        };
        graph.documents.work_link(document.id).await
    }
    .await
    .map_err(|error: lysilogy_db::DbError| error);
    let link = match linked {
        Ok(link) => link,
        Err(error) => return graph_query_failure(&error),
    };
    let Some(link) = link else {
        return unlinked_response(paper_id);
    };
    let work = match fetch_work(graph, link.work_id).await {
        Ok(work) => work,
        Err(response) => return *response,
    };
    (
        StatusCode::OK,
        Json(PaperWorkResponse {
            paper_id,
            work: Some(work),
            link: Some(PaperLink {
                method: link.method,
                detail: link.detail,
                linked_at: link.linked_at,
            }),
        }),
    )
        .into_response()
}

fn unlinked_response(paper_id: PaperId) -> Response {
    (
        StatusCode::OK,
        Json(PaperWorkResponse {
            paper_id,
            work: None,
            link: None,
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct LiteratureStatusResponse {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<LiteratureSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl From<Option<LiteratureJobState>> for LiteratureStatusResponse {
    fn from(job: Option<LiteratureJobState>) -> Self {
        let (state, summary, message) = match job {
            None => ("idle", None, None),
            Some(LiteratureJobState::Running) => ("running", None, None),
            Some(LiteratureJobState::Succeeded(summary)) => ("succeeded", Some(summary), None),
            Some(LiteratureJobState::Failed(message)) => ("failed", None, Some(message)),
        };
        Self {
            state,
            summary,
            message,
        }
    }
}

async fn require_paper(state: &AppState, id: &str) -> Result<PaperId, Box<Response>> {
    let paper_id = parse_id(id).map_err(|error| Box::new(error.into_response()))?;
    if state.catalog.read().await.get(&paper_id).is_none() {
        return Err(Box::new(
            crate::Error::PaperNotFound(paper_id.to_string()).into_response(),
        ));
    }
    Ok(paper_id)
}

async fn start_literature(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let paper_id = match require_paper(&state, &id).await {
        Ok(paper_id) => paper_id,
        Err(response) => return *response,
    };
    if let Err(response) = graph_or_unavailable(&state) {
        return *response;
    }
    let jobs = state.literature_jobs.clone();
    let current = jobs.start(state, paper_id).await;
    (
        StatusCode::ACCEPTED,
        Json(LiteratureStatusResponse::from(Some(current))),
    )
        .into_response()
}

async fn literature_status(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let paper_id = match require_paper(&state, &id).await {
        Ok(paper_id) => paper_id,
        Err(response) => return *response,
    };
    let job = state.literature_jobs.get(&paper_id).await;
    (StatusCode::OK, Json(LiteratureStatusResponse::from(job))).into_response()
}

/// Present local papers for the given Works, keyed by Work id.
///
/// Documents missing from the catalog are skipped.
pub(super) async fn local_papers_for(
    state: &AppState,
    work_ids: &[WorkId],
) -> Result<LocalPapers, GraphError> {
    let graph = graph_or_unavailable(state)?;
    let documents = async {
        let workspace = graph.documents.ensure_default_workspace().await?;
        graph
            .documents
            .local_documents_for_works(workspace, work_ids)
            .await
    }
    .await
    .map_err(|error| Box::new(graph_query_failure(&error)))?;

    let mut local_papers = LocalPapers::new();
    let catalog = state.catalog.read().await;
    for document in documents {
        let paper_id = PaperId::from_relative_path(Path::new(&document.relative_path));
        let Some(entry) = catalog.get(&paper_id) else {
            continue;
        };
        local_papers
            .entry(document.work_id.to_string())
            .or_default()
            .push(LocalPaper {
                paper_id,
                title: entry.overview.metadata.title.clone(),
            });
    }
    drop(catalog);
    Ok(local_papers)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, time::Duration};

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{DocumentFormat, ScannedFile, Visibility, WorkKind};
    use lysilogy_ingest::openalex::{JsonTransport, ProviderError, ProviderFuture, work_url};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::{api::build_router, graph::GraphState};

    fn marker() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("x{}x{nanos}", std::process::id())
    }

    async fn test_graph() -> Option<GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(GraphState::connect(&url).await.expect("connect"))
    }

    fn test_database_url() -> String {
        std::env::var("LYSILOGY_TEST_DATABASE_URL").expect("database url")
    }

    struct Fixture {
        library_dir: TempDir,
        _data: TempDir,
        state: AppState,
        file_name: String,
        paper_id: PaperId,
    }

    /// A library with one placeholder PDF whose name is unique to this run.
    async fn fixture(
        graph: Option<GraphState>,
        state_edit: impl FnOnce(AppState) -> AppState,
    ) -> Fixture {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let file_name = format!("Autor - 2013 - The Study of Things {}.pdf", marker());
        std::fs::write(library.path().join(&file_name), b"placeholder").expect("write");
        let mut state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        if let Some(graph) = graph {
            state = state.with_graph(graph);
        }
        let state = state_edit(state);
        let paper_id = PaperId::from_relative_path(Path::new(&file_name));
        Fixture {
            library_dir: library,
            _data: data,
            state,
            file_name,
            paper_id,
        }
    }

    async fn request(state: &AppState, method: &str, uri: &str) -> (StatusCode, Value) {
        let response = build_router(state.clone(), None)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Register the fixture's file as a document linked to the Work.
    async fn link_document(
        graph: &GraphState,
        file_name: &str,
        work_id: WorkId,
    ) -> lysilogy_core::DocumentId {
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let file = ScannedFile {
            relative_path: file_name.to_owned(),
            sha256: format!("sha-{file_name}"),
            bytes: 11,
            format: DocumentFormat::Pdf,
        };
        graph
            .documents
            .apply_partial_scan(workspace, &[file])
            .await
            .expect("scan");
        let document = graph
            .documents
            .find_by_path(workspace, file_name)
            .await
            .expect("find")
            .expect("document");
        graph
            .documents
            .set_work_link(document.id, work_id, WorkLinkMethod::Manual, None, "test")
            .await
            .expect("link");
        document.id
    }

    #[tokio::test]
    async fn should_return_503_when_paper_work_requested_without_graph() {
        let fixture = fixture(None, |state| state).await;
        let uri = format!("/api/papers/{}/work", fixture.paper_id);
        let (status, json) = request(&fixture.state, "GET", &uri).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "graph_unavailable");
    }

    #[tokio::test]
    async fn should_return_404_when_paper_work_requested_for_unknown_paper() {
        let fixture = fixture(None, |state| state).await;
        let unknown = PaperId::from_relative_path(Path::new("nowhere.pdf"));
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/papers/{unknown}/work"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "paper_not_found");
    }

    #[tokio::test]
    async fn should_return_null_work_when_paper_has_no_document_row() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph), |state| state).await;
        let uri = format!("/api/papers/{}/work", fixture.paper_id);
        let (status, json) = request(&fixture.state, "GET", &uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["paper_id"], fixture.paper_id.to_string());
        assert!(json["work"].is_null(), "{json}");
        assert!(json["link"].is_null(), "{json}");
    }

    #[tokio::test]
    async fn should_return_linked_work_when_document_is_linked() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work");
        let fixture = fixture(Some(graph.clone()), |state| state).await;
        link_document(&graph, &fixture.file_name, work.id).await;

        let uri = format!("/api/papers/{}/work", fixture.paper_id);
        let (status, json) = request(&fixture.state, "GET", &uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["work"]["id"], work.id.to_string());
        assert_eq!(json["link"]["method"], "manual");
        assert_eq!(json["link"]["detail"], "test");
        assert!(json["link"]["linked_at"].is_string(), "{json}");
    }

    #[tokio::test]
    async fn should_return_404_when_lit_requested_for_unknown_paper() {
        let fixture = fixture(None, |state| state).await;
        let unknown = PaperId::from_relative_path(Path::new("nowhere.pdf"));
        let uri = format!("/api/papers/{unknown}/lit");
        let (status, json) = request(&fixture.state, "POST", &uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "paper_not_found");
        let (status, _) = request(&fixture.state, "GET", &uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn should_return_503_when_lit_started_without_graph() {
        let fixture = fixture(None, |state| state).await;
        let uri = format!("/api/papers/{}/lit", fixture.paper_id);
        let (status, json) = request(&fixture.state, "POST", &uri).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "graph_unavailable");
        let (status, json) = request(&fixture.state, "GET", &uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "idle");
    }

    #[tokio::test]
    async fn should_return_400_when_lit_requested_with_malformed_paper_id() {
        let fixture = fixture(None, |state| state).await;
        let (status, _) = request(&fixture.state, "POST", "/api/papers/not-an-id/lit").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// Serves one `OpenAlex` work for one DOI after a short delay.
    #[derive(Clone)]
    struct SlowTransport {
        doi: String,
        openalex_id: String,
        title: String,
    }

    impl JsonTransport for SlowTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                if url != work_url(&format!("doi:{}", self.doi)) {
                    return Err(ProviderError::Status(404));
                }
                Ok(json!({
                    "id": format!("https://openalex.org/{}", self.openalex_id),
                    "display_name": self.title,
                    "publication_year": 2013,
                    "referenced_works": [],
                }))
            })
        }
    }

    async fn poll_until_finished(state: &AppState, uri: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let (status, json) = request(state, "GET", uri).await;
                assert_eq!(status, StatusCode::OK);
                if json["state"] != "running" {
                    return json;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("job finished before the timeout")
    }

    #[tokio::test]
    async fn should_report_running_then_succeeded_when_lit_job_completes() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = format!("10.5555/{marker}");
        let transport = SlowTransport {
            doi: doi.clone(),
            openalex_id: format!("W9{marker}"),
            title: format!("The Study of Things {marker}"),
        };
        let programs = tempfile::tempdir().expect("programs");
        let pdftotext = programs.path().join("pdftotext");
        std::fs::write(
            &pdftotext,
            format!("#!/bin/sh\necho 'The Study of Things {marker}'\necho 'DOI: {doi}'\n"),
        )
        .expect("script");
        std::fs::set_permissions(&pdftotext, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");

        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let file_name = format!("Autor - 2013 - The Study of Things {marker}.pdf");
        std::fs::write(library.path().join(&file_name), b"placeholder").expect("write");
        let state = AppState::with_services(
            library.path(),
            data.path(),
            crate::extract::PdfExtractor::with_programs(pdftotext, "pdfinfo"),
            crate::analysis::AnalysisService::default(),
        )
        .await
        .expect("state")
        .with_graph(graph)
        .with_literature_client(crate::literature::LiteratureClient::new(transport));
        let paper_id = PaperId::from_relative_path(Path::new(&file_name));
        let uri = format!("/api/papers/{paper_id}/lit");

        let (status, json) = request(&state, "GET", &uri).await;
        assert_eq!(
            (status, json["state"].as_str()),
            (StatusCode::OK, Some("idle"))
        );

        let (status, json) = request(&state, "POST", &uri).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(json["state"], "running");
        let (_, json) = request(&state, "GET", &uri).await;
        assert_eq!(json["state"], "running");
        let (status, json) = request(&state, "POST", &uri).await;
        assert_eq!(
            status,
            StatusCode::ACCEPTED,
            "a second start joins the running job"
        );
        assert_eq!(json["state"], "running");

        let json = poll_until_finished(&state, &uri).await;
        assert_eq!(json["state"], "succeeded", "{json}");
        assert_eq!(json["summary"]["status"]["kind"], "linked", "{json}");
        assert!(json.get("message").is_none(), "{json}");

        let (_, linked) = request(&state, "GET", &format!("/api/papers/{paper_id}/work")).await;
        assert_eq!(linked["link"]["method"], "identifier", "{linked}");
    }

    #[tokio::test]
    async fn should_report_failed_when_lit_job_cannot_read_the_paper() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        // The source file vanishes after the catalog scan, so hashing it fails.
        let fixture = fixture(Some(graph), |state| state).await;
        let uri = format!("/api/papers/{}/lit", fixture.paper_id);
        let source = fixture.library_dir.path().join(&fixture.file_name);
        std::fs::remove_file(source).expect("remove");
        let (status, _) = request(&fixture.state, "POST", &uri).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let json = poll_until_finished(&fixture.state, &uri).await;
        assert_eq!(json["state"], "failed", "{json}");
        assert!(json["message"].is_string(), "{json}");
        assert!(json.get("summary").is_none(), "{json}");
    }

    /// A center Work citing one Work that is held locally as a placeholder PDF.
    async fn center_citing_local_paper(
        graph: &GraphState,
        file_name: &str,
    ) -> (WorkId, WorkId, lysilogy_core::DocumentId) {
        let center = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("center");
        let cited = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("cited");
        graph
            .citations
            .add_assertion(center.id, cited.id, None)
            .await
            .expect("edge");
        let document = link_document(graph, file_name, cited.id).await;
        (center.id, cited.id, document)
    }

    #[tokio::test]
    async fn should_include_local_papers_when_neighbor_is_held_locally() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph.clone()), |state| state).await;
        let (center, cited, _) = center_citing_local_paper(&graph, &fixture.file_name).await;

        for uri in [
            format!("/api/graph/neighbors/{center}?direction=references"),
            format!("/api/works/{center}/citations"),
        ] {
            let (status, json) = request(&fixture.state, "GET", &uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            let held = json["local_papers"][cited.to_string()]
                .as_array()
                .expect("entry");
            assert_eq!(held.len(), 1, "{json}");
            assert_eq!(held[0]["paper_id"], fixture.paper_id.to_string());
            assert!(
                held[0]["title"]
                    .as_str()
                    .is_some_and(|title| title.contains("The Study of Things"))
            );
            assert!(
                json["local_papers"].get(center.to_string()).is_none(),
                "{json}"
            );
            assert!(
                !json.to_string().contains(&fixture.file_name),
                "paths must not leak: {json}"
            );
        }
    }

    #[tokio::test]
    async fn should_omit_local_papers_when_document_tombstoned() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph.clone()), |state| state).await;
        let (center, cited, document) = center_citing_local_paper(&graph, &fixture.file_name).await;

        let pool = sqlx::PgPool::connect(&test_database_url())
            .await
            .expect("pool");
        sqlx::query("update reader.document set relative_path = null where id = $1")
            .bind(document.as_uuid())
            .execute(&pool)
            .await
            .expect("tombstone");

        let uri = format!("/api/graph/neighbors/{center}");
        let (status, json) = request(&fixture.state, "GET", &uri).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            json["local_papers"].get(cited.to_string()).is_none(),
            "{json}"
        );
        assert!(
            json["local_papers"]
                .as_object()
                .is_some_and(serde_json::Map::is_empty),
            "{json}"
        );
    }

    #[tokio::test]
    async fn should_skip_local_papers_when_document_is_missing_from_catalog() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph.clone()), |state| state).await;
        let ghost = format!("Ghost - 2001 - Not In Library {}.pdf", marker());
        let (center, _, _) = center_citing_local_paper(&graph, &ghost).await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{center}/citations"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            json["local_papers"]
                .as_object()
                .is_some_and(serde_json::Map::is_empty),
            "{json}"
        );
    }

    #[test]
    fn should_derive_idle_state_when_no_job_exists() {
        let response = LiteratureStatusResponse::from(None);
        assert_eq!(response.state, "idle");
    }
}
