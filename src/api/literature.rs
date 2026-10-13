//! Literature endpoints: paper-to-Work lookup, the `lit` job, and local
//! availability of Works.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use lysilogy_core::{Work, WorkId, WorkLinkMethod};
use lysilogy_ingest::{
    labels::LabelOptions, literature::LiteratureSummary, local_evidence::LocalEvidenceSummary,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{
    AppState, GraphError, fetch_work, graph_or_unavailable, graph_query_failure, parse_id,
};
use crate::domain::PaperId;

/// A paper held locally, as shown next to a Work. Never carries a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LocalPaper {
    pub paper_id: PaperId,
    pub title: String,
}

/// Local papers keyed by Work id. Works without a local paper have no entry.
pub type LocalPapers = BTreeMap<String, Vec<LocalPaper>>;

/// What a finished job produced.
#[derive(Clone, Debug)]
pub struct JobOutcome {
    summary: LiteratureSummary,
    local: Option<LocalEvidenceSummary>,
}

#[derive(Clone, Debug)]
pub enum LiteratureJobState {
    Running,
    Succeeded(Box<JobOutcome>),
    Failed(String),
}

/// The state of each paper's most recent literature job.
#[derive(Clone, Debug, Default)]
pub struct LiteratureJobs(Arc<Mutex<HashMap<PaperId, LiteratureJobState>>>);

impl LiteratureJobs {
    /// Start the flow unless one is already running. Returns the current state.
    pub async fn start(
        &self,
        state: AppState,
        id: PaperId,
        options: LiteratureOptions,
    ) -> LiteratureJobState {
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
                tokio::spawn(async move { run_job(&worker_state, &worker_id, options).await })
                    .await;
            let finished = match outcome {
                Ok(Ok((summary, local))) => {
                    LiteratureJobState::Succeeded(Box::new(JobOutcome { summary, local }))
                }
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

/// The sidecar that writes edge labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelSidecarChoice {
    Claude,
    Codex,
}

/// What a `lit` job does beyond the provider flow. The default is Ramp A's
/// behavior: provider references only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteratureOptions {
    /// Also resolve the paper's own bibliography and mentions.
    pub local: Option<bool>,
    /// Also label local edges with this sidecar. Implies `local`.
    pub labels: Option<LabelSidecarChoice>,
    /// Most model calls for the label pass.
    pub label_limit: Option<usize>,
    /// Also fetch the publisher (Crossref) record of cited Works with a DOI.
    pub publisher: Option<bool>,
}

/// Run the job, then refresh prominence around the linked Work. A refresh
/// problem is logged and never fails the job.
async fn run_job(
    state: &AppState,
    id: &PaperId,
    options: LiteratureOptions,
) -> crate::Result<(LiteratureSummary, Option<LocalEvidenceSummary>)> {
    let outcome = run_job_steps(state, id, options).await?;
    if let Some(work) = outcome.0.work_id {
        state.refresh_prominence_quietly(work, true).await;
    }
    Ok(outcome)
}

async fn run_job_steps(
    state: &AppState,
    id: &PaperId,
    options: LiteratureOptions,
) -> crate::Result<(LiteratureSummary, Option<LocalEvidenceSummary>)> {
    let summary = state
        .run_literature_flow(id, options.publisher.unwrap_or(false))
        .await?;
    if !options.local.unwrap_or(false) && options.labels.is_none() {
        return Ok((summary, None));
    }
    let Some(mut local) = state.run_local_evidence(id, &summary).await? else {
        return Ok((summary, None));
    };
    let Some(sidecar) = options.labels else {
        return Ok((summary, Some(local)));
    };
    let provider = match sidecar {
        LabelSidecarChoice::Claude => crate::domain::AnalysisProvider::Claude,
        LabelSidecarChoice::Codex => crate::domain::AnalysisProvider::Codex,
    };
    let labeler = crate::labeling::SidecarLabeler::new(
        provider,
        crate::analysis::LocalCliAnalyzer::default(),
        state.store.root().join("labels"),
    )?;
    let defaults = LabelOptions::default();
    let label_options = LabelOptions {
        limit: options.label_limit.unwrap_or(defaults.limit),
        ..defaults
    };
    local.labels = state
        .label_local_edges(id, &summary, &labeler, label_options)
        .await?;
    Ok((summary, Some(local)))
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
    local: Option<LocalEvidenceSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl From<Option<LiteratureJobState>> for LiteratureStatusResponse {
    fn from(job: Option<LiteratureJobState>) -> Self {
        let (state, summary, local, message) = match job {
            None => ("idle", None, None, None),
            Some(LiteratureJobState::Running) => ("running", None, None, None),
            Some(LiteratureJobState::Succeeded(outcome)) => {
                ("succeeded", Some(outcome.summary), outcome.local, None)
            }
            Some(LiteratureJobState::Failed(message)) => ("failed", None, None, Some(message)),
        };
        Self {
            state,
            summary,
            local,
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
    body: Bytes,
) -> Response {
    let paper_id = match require_paper(&state, &id).await {
        Ok(paper_id) => paper_id,
        Err(response) => return *response,
    };
    if let Err(response) = graph_or_unavailable(&state) {
        return *response;
    }
    let options = if body.iter().all(u8::is_ascii_whitespace) {
        LiteratureOptions::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(options) => options,
            Err(error) => {
                return super::graph_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_body",
                    &format!("The request body is not valid: {error}"),
                );
            }
        }
    };
    let jobs = state.literature_jobs.clone();
    let current = jobs.start(state, paper_id, options).await;
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
pub async fn local_papers_for(
    state: &AppState,
    work_ids: &[WorkId],
) -> Result<LocalPapers, GraphError> {
    let graph = graph_or_unavailable(state)?;
    let held = state
        .held_papers_with_titles(graph, work_ids)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;
    let mut local_papers = LocalPapers::new();
    for (work_id, papers) in held {
        local_papers.insert(
            work_id.to_string(),
            papers
                .into_iter()
                .map(|paper| LocalPaper {
                    paper_id: paper.paper_id,
                    title: paper.title,
                })
                .collect(),
        );
    }
    Ok(local_papers)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};

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

    async fn request_with_body(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let body = body.map_or_else(Body::empty, |value| {
            builder = std::mem::take(&mut builder).header("content-type", "application/json");
            Body::from(value.to_string())
        });
        let response = build_router(state.clone(), None)
            .oneshot(builder.body(body).expect("request"))
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
        /// Also cites one stub Work and answers batch and metrics requests.
        with_metrics: bool,
    }

    impl SlowTransport {
        fn stub_id(&self) -> String {
            format!("{}S", self.openalex_id)
        }
    }

    impl JsonTransport for SlowTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let metrics_url = url.contains("cited_by_count");
                if url.contains("filter=openalex:") && (self.with_metrics || !metrics_url) {
                    let ids = url
                        .split("filter=openalex:")
                        .nth(1)
                        .and_then(|rest| rest.split('&').next())
                        .unwrap_or_default();
                    let metrics = url.contains("cited_by_count");
                    let results: Vec<Value> = ids
                        .split('|')
                        .map(|id| {
                            if metrics {
                                json!({"id": format!("https://openalex.org/{id}"), "cited_by_count": 5})
                            } else if id.eq_ignore_ascii_case(&self.openalex_id) {
                                json!({"id": format!("https://openalex.org/{id}"),
                                       "display_name": self.title, "publication_year": 2013,
                                       "authorships": []})
                            } else {
                                json!({"id": format!("https://openalex.org/{id}"),
                                       "display_name": "A Cited Stub", "publication_year": 2001,
                                       "authorships": []})
                            }
                        })
                        .collect();
                    return Ok(json!({ "results": results }));
                }
                if url != work_url(&format!("doi:{}", self.doi)) {
                    return Err(ProviderError::Status(404));
                }
                Ok(json!({
                    "id": format!("https://openalex.org/{}", self.openalex_id),
                    "display_name": self.title,
                    "publication_year": 2013,
                    "referenced_works": if self.with_metrics {
                        vec![format!("https://openalex.org/{}", self.stub_id())]
                    } else {
                        Vec::new()
                    },
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

    /// A state whose fake `pdftotext` prints a title and DOI that the fake
    /// provider knows, so `lit` links the paper to a Work.
    async fn linkable_state(graph: GraphState) -> (AppState, PaperId, std::path::PathBuf, String) {
        linkable_state_with(graph, false).await
    }

    async fn linkable_state_with(
        graph: GraphState,
        with_metrics: bool,
    ) -> (AppState, PaperId, std::path::PathBuf, String) {
        let marker = marker();
        let doi = format!("10.5555/{marker}");
        let transport = SlowTransport {
            doi: doi.clone(),
            openalex_id: format!("W9{marker}"),
            title: format!("The Study of Things {marker}"),
            with_metrics,
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
        let source = library.path().join(&file_name);
        std::fs::write(&source, format!("placeholder {marker}")).expect("write");
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
        // The temp directories live as long as the test process needs them.
        std::mem::forget((programs, library, data));
        let paper_id = PaperId::from_relative_path(Path::new(&file_name));
        (state, paper_id, source, marker)
    }

    async fn linked_work_of(state: &AppState, paper_id: &PaperId) -> WorkId {
        let (_, linked) = request(state, "GET", &format!("/api/papers/{paper_id}/work")).await;
        linked["work"]["id"]
            .as_str()
            .expect("linked work id")
            .parse()
            .expect("work id")
    }

    #[tokio::test]
    async fn should_refresh_prominence_after_lit_job_succeeds() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, paper_id, _, _) = linkable_state_with(graph.clone(), true).await;
        let uri = format!("/api/papers/{paper_id}/lit");
        request(&state, "POST", &uri).await;
        let json = poll_until_finished(&state, &uri).await;
        assert_eq!(json["state"], "succeeded", "{json}");

        let work = linked_work_of(&state, &paper_id).await;
        let stubs = graph.citations.outgoing(work).await.expect("outgoing");
        assert_eq!(stubs.len(), 1);
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let rows = graph
            .prominence
            .cached(workspace, &[work, stubs[0].id])
            .await
            .expect("cached");
        assert_eq!(rows.len(), 2, "linked Work and cited stub get rows");
        for row in &rows {
            assert_eq!(row.signals.cited_by_global_count, Some(5), "{row:?}");
        }
    }

    #[tokio::test]
    async fn should_not_fail_lit_job_when_prominence_refresh_fails() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        // This transport answers 404 to the metrics request.
        let (state, paper_id, _, _) = linkable_state(graph.clone()).await;
        let uri = format!("/api/papers/{paper_id}/lit");
        request(&state, "POST", &uri).await;
        let json = poll_until_finished(&state, &uri).await;
        assert_eq!(json["state"], "succeeded", "{json}");
        assert!(json.get("message").is_none(), "{json}");

        let work = linked_work_of(&state, &paper_id).await;
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let rows = graph
            .prominence
            .cached(workspace, &[work])
            .await
            .expect("cached");
        assert_eq!(rows.len(), 1, "refresh continued without metrics");
        assert_eq!(rows[0].signals.cited_by_global_count, None);
    }

    #[tokio::test]
    async fn should_recompute_all_participating_works_when_scope_is_all() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let citing = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("citing");
        let cited = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("cited");
        graph
            .citations
            .add_assertion(citing.id, cited.id, None)
            .await
            .expect("assertion");
        let fixture = fixture(Some(graph.clone()), |state| state).await;
        let summary = fixture
            .state
            .recompute_prominence(crate::prominence::ProminenceScope::All, false)
            .await
            .expect("recompute");
        assert!(summary.works >= 2, "{summary:?}");
        assert_eq!(summary.metric_requests, 0);
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let rows = graph
            .prominence
            .cached(workspace, &[citing.id, cited.id])
            .await
            .expect("cached");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].signals.citing_works_count + rows[1].signals.citing_works_count,
            1
        );
    }

    #[tokio::test]
    async fn should_keep_ramp_a_behavior_when_lit_body_empty() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, paper_id, _, _) = linkable_state(graph).await;
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
        assert!(
            json.get("local").is_none(),
            "an empty body is Ramp A: {json}"
        );

        let (_, linked) = request(&state, "GET", &format!("/api/papers/{paper_id}/work")).await;
        assert_eq!(linked["link"]["method"], "identifier", "{linked}");
    }

    #[tokio::test]
    async fn should_run_local_evidence_when_lit_body_requests_local() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, paper_id, source, marker) = linkable_state(graph).await;
        crate::source_index::test_support::cache_fixture_with_blocks(
            &source,
            &state.store.paper_dir(&paper_id),
            &[
                (&format!("The Study of Things {marker}"), "heading", 1),
                (&format!("DOI: 10.5555/{marker}"), "body", 1),
                ("Earlier work [1] shows this clearly.", "body", 1),
                ("References", "heading", 2),
                (
                    &format!("[1] Adams. Cited evidence {marker}. 2019."),
                    "body",
                    2,
                ),
            ],
        )
        .await;
        let uri = format!("/api/papers/{paper_id}/lit");
        let (status, _) =
            request_with_body(&state, "POST", &uri, Some(json!({"local": true}))).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let json = poll_until_finished(&state, &uri).await;
        assert_eq!(json["state"], "succeeded", "{json}");
        assert_eq!(json["local"]["entries"], 1, "{json}");
        assert_eq!(json["local"]["mentions"], 1, "{json}");
        assert_eq!(json["local"]["local_edges"], 1, "{json}");
        assert!(json["local"]["labels"].is_null(), "{json}");
    }

    #[tokio::test]
    async fn should_reject_body_when_lit_options_are_unknown() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph), |state| state).await;
        let uri = format!("/api/papers/{}/lit", fixture.paper_id);
        for body in [
            json!({"labels": "gpt"}),
            json!({"label_limit": -1}),
            json!({"publisher": "yes"}),
            json!({"extra": 1}),
        ] {
            let (status, json) = request_with_body(&fixture.state, "POST", &uri, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
            assert_eq!(json["error"], "invalid_body");
        }
    }

    #[test]
    fn should_reject_non_boolean_publisher_when_parsing_lit_options() {
        for body in [
            json!({"publisher": "yes"}),
            json!({"publisher": 1}),
            json!({"publisher": null, "extra": true}),
        ] {
            let parsed = serde_json::from_value::<LiteratureOptions>(body.clone());
            assert!(parsed.is_err(), "{body}");
        }
        let options: LiteratureOptions =
            serde_json::from_value(json!({"publisher": true})).expect("bool");
        assert_eq!(options.publisher, Some(true));
        let unset: LiteratureOptions = serde_json::from_value(json!({})).expect("empty");
        assert_eq!(unset.publisher, None);
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
