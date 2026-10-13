//! Acquisition endpoints: start a fetch of an open-access copy of a Work and
//! report its progress.

use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use lysilogy_core::{AcquisitionState, WorkId};
use lysilogy_db::acquisition::AcquisitionRow;
use lysilogy_ingest::acquire::AcquisitionSummary;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};

use super::{
    AppState, fetch_work, graph_error, graph_or_unavailable, graph_query_failure, parse_work_id,
};
use crate::{
    acquisition::AcquireOptions,
    domain::{AnalysisProvider, PaperId},
};

/// Most acquisition jobs running at once.
const MAX_CONCURRENT_JOBS: usize = 2;

/// The sidecar that may pick a PDF link on a landing page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFallbackChoice {
    Claude,
    Codex,
}

/// Body of `POST /api/works/{id}/fetch`; every field is optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchOptions {
    pub retry_unavailable: Option<bool>,
    pub model_fallback: Option<ModelFallbackChoice>,
}

impl FetchOptions {
    const fn acquire_options(self) -> AcquireOptions {
        AcquireOptions {
            retry_unavailable: matches!(self.retry_unavailable, Some(true)),
            model_fallback: match self.model_fallback {
                None => None,
                Some(ModelFallbackChoice::Claude) => Some(AnalysisProvider::Claude),
                Some(ModelFallbackChoice::Codex) => Some(AnalysisProvider::Codex),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub enum AcquisitionJobState {
    Running,
    Succeeded(Box<AcquisitionSummary>),
    Failed(String),
}

/// The state of each Work's most recent acquisition job, plus the limit on
/// jobs that run at once.
#[derive(Clone, Debug)]
pub struct AcquisitionJobs {
    jobs: Arc<Mutex<HashMap<WorkId, AcquisitionJobState>>>,
    permits: Arc<Semaphore>,
}

impl Default for AcquisitionJobs {
    fn default() -> Self {
        Self {
            jobs: Arc::default(),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS)),
        }
    }
}

impl AcquisitionJobs {
    /// Start a job unless one is already running for the Work. Returns the
    /// current state.
    pub async fn start(
        &self,
        state: AppState,
        work: WorkId,
        options: FetchOptions,
    ) -> AcquisitionJobState {
        let mut jobs = self.jobs.lock().await;
        if matches!(jobs.get(&work), Some(AcquisitionJobState::Running)) {
            return AcquisitionJobState::Running;
        }
        jobs.insert(work, AcquisitionJobState::Running);
        drop(jobs);

        let registry = self.clone();
        tokio::spawn(async move {
            let permits = registry.permits.clone();
            let worker = tokio::spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|error| crate::Error::Acquisition(error.to_string()))?;
                state.acquire_work(work, options.acquire_options()).await
            })
            .await;
            let finished = match worker {
                Ok(Ok(report)) => AcquisitionJobState::Succeeded(Box::new(report.summary)),
                Ok(Err(error)) => AcquisitionJobState::Failed(error.to_string()),
                Err(error) => AcquisitionJobState::Failed(format!("the job stopped: {error}")),
            };
            registry.jobs.lock().await.insert(work, finished);
        });
        AcquisitionJobState::Running
    }

    async fn get(&self, work: WorkId) -> Option<AcquisitionJobState> {
        self.jobs.lock().await.get(&work).cloned()
    }
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/works/{id}/fetch", post(start_fetch))
        .route("/api/works/{id}/acquisition", get(acquisition_status))
}

#[derive(Serialize)]
struct JobView {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<AcquisitionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl From<Option<AcquisitionJobState>> for JobView {
    fn from(job: Option<AcquisitionJobState>) -> Self {
        let (state, summary, message) = match job {
            None => ("idle", None, None),
            Some(AcquisitionJobState::Running) => ("running", None, None),
            Some(AcquisitionJobState::Succeeded(summary)) => ("succeeded", Some(*summary), None),
            Some(AcquisitionJobState::Failed(message)) => ("failed", None, Some(message)),
        };
        Self {
            state,
            summary,
            message,
        }
    }
}

/// What the client may see of an acquisition row. No paths, no blob locations.
#[derive(Serialize)]
struct AcquisitionView {
    state: AcquisitionState,
    unavailable_reason: Option<String>,
    checked_at: Option<DateTime<Utc>>,
    attempts: i32,
    last_error: Option<String>,
    source: Option<String>,
    pdf_url: Option<String>,
    bytes: Option<i64>,
    updated_at: DateTime<Utc>,
}

impl From<&AcquisitionRow> for AcquisitionView {
    fn from(row: &AcquisitionRow) -> Self {
        Self {
            state: row.state,
            unavailable_reason: row.unavailable_reason.map(|reason| reason.to_string()),
            checked_at: row.checked_at,
            attempts: row.attempts,
            last_error: row.last_error.clone(),
            source: row.source.map(|source| source.to_string()),
            pdf_url: row.pdf_url.clone(),
            bytes: row.bytes,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Serialize)]
struct AcquisitionStatusResponse {
    work_id: WorkId,
    job: JobView,
    acquisition: Option<AcquisitionView>,
    paper_id: Option<PaperId>,
}

fn parse_options(body: &Bytes) -> Result<FetchOptions, Box<Response>> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(FetchOptions::default());
    }
    serde_json::from_slice(body).map_err(|error| {
        Box::new(graph_error(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            &format!("The request body is not valid: {error}"),
        ))
    })
}

async fn start_fetch(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> Response {
    let work_id = match parse_work_id(&id) {
        Ok(work_id) => work_id,
        Err(response) => return *response,
    };
    let options = match parse_options(&body) {
        Ok(options) => options,
        Err(response) => return *response,
    };
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let work = match fetch_work(graph, work_id).await {
        Ok(work) => work,
        Err(response) => return *response,
    };
    let jobs = state.acquisition_jobs.clone();
    let current = jobs.start(state, work.id, options).await;
    (StatusCode::ACCEPTED, Json(JobView::from(Some(current)))).into_response()
}

async fn acquisition_status(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let work_id = match parse_work_id(&id) {
        Ok(work_id) => work_id,
        Err(response) => return *response,
    };
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let work = match fetch_work(graph, work_id).await {
        Ok(work) => work,
        Err(response) => return *response,
    };
    let row = match graph.acquisitions.get(work.id).await {
        Ok(row) => row,
        Err(error) => return graph_query_failure(&error),
    };
    let paper_id = match mapped_paper_id(&state, row.as_ref()).await {
        Ok(paper_id) => paper_id,
        Err(response) => return *response,
    };
    let job = state.acquisition_jobs.get(work.id).await;
    (
        StatusCode::OK,
        Json(AcquisitionStatusResponse {
            work_id: work.id,
            job: JobView::from(job),
            acquisition: row.as_ref().map(AcquisitionView::from),
            paper_id,
        }),
    )
        .into_response()
}

/// The catalog paper holding a mapped copy, if the document is present and the
/// catalog has it.
async fn mapped_paper_id(
    state: &AppState,
    row: Option<&AcquisitionRow>,
) -> Result<Option<PaperId>, Box<Response>> {
    let Some(row) = row else {
        return Ok(None);
    };
    if row.state != AcquisitionState::Mapped {
        return Ok(None);
    }
    let Some(document) = row.document_id else {
        return Ok(None);
    };
    let graph = graph_or_unavailable(state)?;
    let presence = graph
        .documents
        .document_presence(document)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;
    let Some(Some(relative_path)) = presence else {
        return Ok(None);
    };
    let paper_id = PaperId::from_relative_path(Path::new(&relative_path));
    let in_catalog = state.catalog.read().await.get(&paper_id).is_some();
    Ok(in_catalog.then_some(paper_id))
}

/// Acquisition state of a Work as shown on graph nodes.
#[derive(Debug, Serialize)]
pub(super) struct AcquisitionBrief {
    state: AcquisitionState,
    unavailable_reason: Option<String>,
}

/// Brief acquisition states keyed by Work id. Works without a row have no entry.
pub(super) type AcquisitionStates = BTreeMap<String, AcquisitionBrief>;

/// Acquisition states for the given Works.
pub(super) async fn states_for(
    state: &AppState,
    work_ids: &[WorkId],
) -> Result<AcquisitionStates, Box<Response>> {
    let graph = graph_or_unavailable(state)?;
    let rows = graph
        .acquisitions
        .states_for(work_ids)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;
    Ok(rows
        .into_iter()
        .map(|(work, state, reason)| {
            (
                work.to_string(),
                AcquisitionBrief {
                    state,
                    unavailable_reason: reason.map(|reason| reason.to_string()),
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{
        DocumentFormat, IdentifierScheme, IdentifierStatus, ScannedFile, Visibility, WorkKind,
    };
    use lysilogy_ingest::{
        acquire::{
            FetchError, FetchFuture, FetchRequest, HostThrottle, PdfFetcher, ScriptedFetcher,
            ScriptedResponse,
        },
        openalex::{JsonTransport, ProviderError, ProviderFuture},
    };
    use serde_json::Value;
    use tempfile::TempDir;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use super::*;
    use crate::{api::build_router, graph::GraphState};

    fn marker() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    async fn test_graph() -> Option<GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(GraphState::connect(&url).await.expect("connect"))
    }

    /// Answers every provider request with 404, so only identifier-derived
    /// candidates remain.
    struct NotFoundTransport;

    impl JsonTransport for NotFoundTransport {
        fn get_json<'a>(
            &'a self,
            _url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async { Err(ProviderError::Status(404)) })
        }
    }

    /// Holds every fetch until released, then fails it permanently.
    #[derive(Default)]
    struct GatedFetcher {
        gate: Notify,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl PdfFetcher for GatedFetcher {
        fn fetch<'a>(&'a self, _request: FetchRequest<'a>) -> FetchFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.gate.notified().await;
                Err(FetchError::Permanent("http_404".to_owned()))
            })
        }
    }

    struct Fixture {
        library: TempDir,
        _data: TempDir,
        state: AppState,
    }

    async fn fixture(graph: Option<GraphState>, fetcher: Arc<dyn PdfFetcher>) -> Fixture {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let mut state = AppState::new(library.path(), data.path())
            .await
            .expect("state")
            .with_literature_client(crate::literature::LiteratureClient::new(NotFoundTransport))
            .with_acquisition_throttle(HostThrottle::disabled())
            .with_acquisition_fetcher(fetcher);
        if let Some(graph) = graph {
            state = state.with_graph(graph);
        }
        Fixture {
            library,
            _data: data,
            state,
        }
    }

    async fn request(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value) {
        let response = build_router(state.clone(), None)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
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

    /// A Work with a fresh arXiv identifier and a title.
    async fn arxiv_work(graph: &GraphState) -> (WorkId, String) {
        let token = uuid::Uuid::new_v4().as_u128();
        let arxiv = format!("2{:03}.{:05}", token % 1000, (token / 1000) % 100_000);
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        graph
            .works
            .add_identifier(
                work,
                IdentifierScheme::Arxiv,
                &arxiv,
                None,
                IdentifierStatus::Active,
            )
            .await
            .expect("identifier");
        graph
            .works
            .set_display_title(work, "A Scripted Study of Fetching")
            .await
            .expect("title");
        (work, arxiv)
    }

    async fn poll_until_finished(state: &AppState, work: WorkId) -> Value {
        let uri = format!("/api/works/{work}/acquisition");
        for _ in 0..200 {
            let (status, json) = request(state, "GET", &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{json}");
            if json["job"]["state"] != "running" {
                return json;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the job did not finish");
    }

    #[tokio::test]
    async fn should_return_202_running_when_posting_fetch() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (work, arxiv) = arxiv_work(&graph).await;
        let url = format!("https://arxiv.org/pdf/{arxiv}");
        let fetcher = Arc::new(ScriptedFetcher::new().with(
            &url,
            ScriptedResponse::Error(FetchError::Permanent("http_404".to_owned())),
        ));
        let fixture = fixture(Some(graph), fetcher).await;
        let (status, json) = request(
            &fixture.state,
            "POST",
            &format!("/api/works/{work}/fetch"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(json["state"], "running", "{json}");
        let done = poll_until_finished(&fixture.state, work).await;
        assert_eq!(done["job"]["state"], "succeeded", "{done}");
        assert_eq!(done["acquisition"]["state"], "unavailable", "{done}");
        assert_eq!(
            done["acquisition"]["unavailable_reason"],
            "all_links_failed"
        );
        assert!(done["paper_id"].is_null(), "{done}");
    }

    #[tokio::test]
    async fn should_return_current_state_when_fetch_already_running() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (work, _) = arxiv_work(&graph).await;
        let fetcher = Arc::new(GatedFetcher::default());
        let fixture = fixture(Some(graph), fetcher.clone()).await;
        let uri = format!("/api/works/{work}/fetch");
        let (first, _) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(first, StatusCode::ACCEPTED);
        for _ in 0..200 {
            if fetcher.calls.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let (second, json) = request(&fixture.state, "POST", &uri, Some("{}")).await;
        assert_eq!(second, StatusCode::ACCEPTED);
        assert_eq!(json["state"], "running", "{json}");
        let (_, status) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/acquisition"),
            None,
        )
        .await;
        assert_eq!(status["job"]["state"], "running", "{status}");
        fetcher.gate.notify_waiters();
        fetcher.gate.notify_one();
        let done = poll_until_finished(&fixture.state, work).await;
        assert_eq!(done["job"]["state"], "succeeded", "{done}");
        assert_eq!(
            fetcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the second post must not start another run"
        );
    }

    #[tokio::test]
    async fn should_return_400_when_work_id_malformed() {
        let fixture = fixture(None, Arc::new(ScriptedFetcher::new())).await;
        for (method, uri) in [
            ("POST", "/api/works/not-a-work/fetch"),
            ("GET", "/api/works/not-a-work/acquisition"),
        ] {
            let (status, json) = request(&fixture.state, method, uri, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(json["error"], "invalid_work_id");
        }
    }

    #[tokio::test]
    async fn should_return_400_when_body_has_unknown_field() {
        let work = WorkId::from_uuid(uuid::Uuid::new_v4());
        let fixture = fixture(None, Arc::new(ScriptedFetcher::new())).await;
        let uri = format!("/api/works/{work}/fetch");
        for body in [
            r#"{"retry":true}"#,
            r#"{"model_fallback":"gpt"}"#,
            r#"{"retry_unavailable":"yes"}"#,
            "not json",
            "[]",
        ] {
            let (status, json) = request(&fixture.state, "POST", &uri, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error"], "invalid_body", "{body}");
        }
    }

    #[tokio::test]
    async fn should_return_404_when_work_unknown() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = WorkId::from_uuid(uuid::Uuid::new_v4());
        let fixture = fixture(Some(graph), Arc::new(ScriptedFetcher::new())).await;
        for (method, uri) in [
            ("POST", format!("/api/works/{work}/fetch")),
            ("GET", format!("/api/works/{work}/acquisition")),
        ] {
            let (status, json) = request(&fixture.state, method, &uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(json["error"], "work_not_found");
        }
    }

    #[tokio::test]
    async fn should_return_503_when_graph_unavailable() {
        let work = WorkId::from_uuid(uuid::Uuid::new_v4());
        let fixture = fixture(None, Arc::new(ScriptedFetcher::new())).await;
        for (method, uri) in [
            ("POST", format!("/api/works/{work}/fetch")),
            ("GET", format!("/api/works/{work}/acquisition")),
        ] {
            let (status, json) = request(&fixture.state, method, &uri, None).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
            assert_eq!(json["error"], "graph_unavailable");
        }
    }

    #[tokio::test]
    async fn should_report_idle_and_no_row_when_never_fetched() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (work, _) = arxiv_work(&graph).await;
        let fixture = fixture(Some(graph), Arc::new(ScriptedFetcher::new())).await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/acquisition"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["work_id"], work.to_string());
        assert_eq!(json["job"]["state"], "idle", "{json}");
        assert!(json["acquisition"].is_null(), "{json}");
        assert!(json["paper_id"].is_null(), "{json}");
    }

    #[tokio::test]
    async fn should_report_mapped_acquisition_and_paper_id_when_job_finished() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (work, arxiv) = arxiv_work(&graph).await;
        let mut body = b"%PDF-1.4\n".to_vec();
        body.extend_from_slice(marker().as_bytes());
        let url = format!("https://arxiv.org/pdf/{arxiv}");
        let fetcher = Arc::new(ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(body)));
        let fixture = fixture(Some(graph), fetcher).await;
        let (status, _) = request(
            &fixture.state,
            "POST",
            &format!("/api/works/{work}/fetch"),
            Some(r#"{"retry_unavailable":false}"#),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let done = poll_until_finished(&fixture.state, work).await;
        assert_eq!(done["job"]["state"], "succeeded", "{done}");
        assert_eq!(
            done["job"]["summary"]["outcome"]["kind"], "mapped",
            "{done}"
        );
        assert_eq!(done["acquisition"]["state"], "mapped", "{done}");
        assert_eq!(done["acquisition"]["source"], "arxiv", "{done}");
        let paper_id = done["paper_id"].as_str().expect("paper id");
        let (paper_status, _) = request(
            &fixture.state,
            "GET",
            &format!("/api/papers/{paper_id}"),
            None,
        )
        .await;
        assert_eq!(paper_status, StatusCode::OK);
        let text = done["acquisition"].to_string();
        assert!(!text.contains("Fetched/"), "paths must not leak: {text}");
        assert!(done["acquisition"].get("final_url").is_none(), "{done}");
        assert!(done["acquisition"].get("blob_id").is_none(), "{done}");
        assert!(fixture.library.path().join("Fetched").is_dir());
    }

    #[tokio::test]
    async fn should_include_acquisitions_in_neighbors_when_rows_exist() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let center = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("center")
            .id;
        let (cited, _) = arxiv_work(&graph).await;
        let untouched = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("untouched")
            .id;
        for target in [cited, untouched] {
            graph
                .citations
                .add_assertion(center, target, None)
                .await
                .expect("edge");
        }
        graph.acquisitions.claim(cited).await.expect("claim");
        graph.acquisitions.release(cited).await.expect("release");
        graph
            .acquisitions
            .mark_unavailable(cited, lysilogy_core::UnavailableReason::NoOaLocation)
            .await
            .expect("unavailable");
        let fixture = fixture(Some(graph), Arc::new(ScriptedFetcher::new())).await;

        for uri in [
            format!("/api/graph/neighbors/{center}?direction=references"),
            format!("/api/works/{center}/citations"),
        ] {
            let (status, json) = request(&fixture.state, "GET", &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            let entry = &json["acquisitions"][cited.to_string()];
            assert_eq!(entry["state"], "unavailable", "{json}");
            assert_eq!(entry["unavailable_reason"], "no_oa_location", "{json}");
            assert!(
                json["acquisitions"].get(untouched.to_string()).is_none(),
                "{json}"
            );
        }
    }

    #[tokio::test]
    async fn should_omit_paper_id_when_document_not_in_catalog() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (work, _) = arxiv_work(&graph).await;
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let path = format!("Fetched/{}.pdf", marker());
        graph
            .documents
            .apply_partial_scan(
                workspace,
                &[ScannedFile {
                    relative_path: path.clone(),
                    sha256: format!("sha-{path}"),
                    bytes: 4,
                    format: DocumentFormat::Pdf,
                }],
            )
            .await
            .expect("scan");
        let document = graph
            .documents
            .find_by_path(workspace, &path)
            .await
            .expect("find")
            .expect("document");
        graph.acquisitions.claim(work).await.expect("claim");
        graph
            .acquisitions
            .mark_mapped(work, document.id, None)
            .await
            .expect("mapped");
        let fixture = fixture(Some(graph), Arc::new(ScriptedFetcher::new())).await;

        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/acquisition"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["acquisition"]["state"], "mapped", "{json}");
        assert!(json["paper_id"].is_null(), "{json}");
    }
}
