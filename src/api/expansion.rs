//! Expansion endpoints: start a bounded walk from a Work and report its
//! progress and the Work's expansion state.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use lysilogy_core::{ExpansionDirection, WalkBounds, WorkId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

use super::{
    AppState, fetch_work, graph_error, graph_or_unavailable, graph_query_failure, parse_work_id,
};
use crate::expansion::{ExpandOptions, ExpansionView, WalkReport};

/// One walk at a time: the provider is polite-use, and a walk is a single job.
const MAX_CONCURRENT_JOBS: usize = 1;

/// Body of `POST /api/works/{id}/expand`; every field is optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpandBody {
    pub depth: Option<i64>,
    pub fanout: Option<i64>,
    pub refresh: Option<bool>,
    pub local: Option<bool>,
    /// `references` (the default) or `cited_by`; cited-by is a single level.
    pub direction: Option<ExpansionDirection>,
}

impl ExpandBody {
    fn options(self) -> Result<(ExpandOptions, ExpansionDirection), Box<Response>> {
        let bounds = WalkBounds::parse(self.depth, self.fanout).map_err(|error| {
            Box::new(graph_error(
                StatusCode::BAD_REQUEST,
                error.code(),
                &error.to_string(),
            ))
        })?;
        let direction = self.direction.unwrap_or(ExpansionDirection::References);
        if direction == ExpansionDirection::CitedBy && bounds.depth != 1 {
            return Err(Box::new(graph_error(
                StatusCode::BAD_REQUEST,
                "invalid_depth",
                "cited_by expansion is a single level; depth must be 1",
            )));
        }
        Ok((
            ExpandOptions {
                bounds,
                refresh: self.refresh.unwrap_or(false),
                local: self.local.unwrap_or(true),
            },
            direction,
        ))
    }
}

/// An empty or whitespace body means defaults; anything else must be a JSON
/// object of known fields.
fn parse_options(body: &Bytes) -> Result<(ExpandOptions, ExpansionDirection), Box<Response>> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok((ExpandOptions::default(), ExpansionDirection::References));
    }
    let invalid = |detail: String| {
        Box::new(graph_error(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            &format!("The request body is not valid: {detail}"),
        ))
    };
    let value: Value = serde_json::from_slice(body).map_err(|error| invalid(error.to_string()))?;
    if !value.is_object() {
        return Err(invalid("expected a JSON object".to_owned()));
    }
    serde_json::from_value::<ExpandBody>(value)
        .map_err(|error| invalid(error.to_string()))?
        .options()
}

#[derive(Clone, Debug)]
pub enum ExpansionJobState {
    Running,
    Succeeded(Box<WalkReport>),
    Failed(String),
}

impl ExpansionJobState {
    fn finished(result: crate::Result<WalkReport>) -> Self {
        match result {
            Ok(report) => Self::Succeeded(Box::new(report)),
            Err(error) => Self::Failed(error.to_string()),
        }
    }
}

/// The state of each Work's most recent expansion job, plus the limit on jobs
/// that run at once.
#[derive(Clone, Debug)]
pub struct ExpansionJobs {
    jobs: Arc<Mutex<HashMap<(WorkId, ExpansionDirection), ExpansionJobState>>>,
    permits: Arc<Semaphore>,
}

impl Default for ExpansionJobs {
    fn default() -> Self {
        Self {
            jobs: Arc::default(),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS)),
        }
    }
}

impl ExpansionJobs {
    /// Start a job unless one is already running for the Work. Returns the
    /// current state. A request the cache answers finishes before this returns,
    /// without waiting for a permit.
    #[cfg(test)]
    pub async fn start(
        &self,
        state: AppState,
        work: WorkId,
        options: ExpandOptions,
    ) -> ExpansionJobState {
        self.start_directed(state, work, ExpansionDirection::References, options)
            .await
    }

    /// Like [`Self::start`]. Each direction of a Work has its own job, so a cited-by
    /// job never blocks or replaces a references job.
    pub async fn start_directed(
        &self,
        state: AppState,
        work: WorkId,
        direction: ExpansionDirection,
        options: ExpandOptions,
    ) -> ExpansionJobState {
        let key = (work, direction);
        let mut jobs = self.jobs.lock().await;
        if matches!(jobs.get(&key), Some(ExpansionJobState::Running)) {
            return ExpansionJobState::Running;
        }
        jobs.insert(key, ExpansionJobState::Running);
        drop(jobs);

        let single_level = options.bounds.depth == 1 && !options.refresh;
        if single_level
            && state
                .expansion_is_cached_directed(work, direction)
                .await
                .unwrap_or(false)
        {
            let finished = ExpansionJobState::finished(
                state.walk_from_directed(work, direction, options).await,
            );
            self.jobs.lock().await.insert(key, finished.clone());
            return finished;
        }

        let registry = self.clone();
        tokio::spawn(async move {
            let permits = registry.permits.clone();
            let worker = tokio::spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|error| crate::Error::Literature(error.to_string()))?;
                state.walk_from_directed(work, direction, options).await
            })
            .await;
            let finished = match worker {
                Ok(result) => ExpansionJobState::finished(result),
                Err(error) => ExpansionJobState::Failed(format!("the job stopped: {error}")),
            };
            registry.jobs.lock().await.insert(key, finished);
        });
        ExpansionJobState::Running
    }

    #[cfg(test)]
    async fn get(&self, work: WorkId) -> Option<ExpansionJobState> {
        self.get_directed(work, ExpansionDirection::References)
            .await
    }

    async fn get_directed(
        &self,
        work: WorkId,
        direction: ExpansionDirection,
    ) -> Option<ExpansionJobState> {
        self.jobs.lock().await.get(&(work, direction)).cloned()
    }
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/works/{id}/expand", post(start_expand))
        .route("/api/works/{id}/expansion", get(expansion_status))
}

#[derive(Serialize)]
struct JobView {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<WalkReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl From<Option<ExpansionJobState>> for JobView {
    fn from(job: Option<ExpansionJobState>) -> Self {
        let (state, report, message) = match job {
            None => ("idle", None, None),
            Some(ExpansionJobState::Running) => ("running", None, None),
            Some(ExpansionJobState::Succeeded(report)) => ("succeeded", Some(*report), None),
            Some(ExpansionJobState::Failed(message)) => ("failed", None, Some(message)),
        };
        Self {
            state,
            report,
            message,
        }
    }
}

#[derive(Serialize)]
struct ExpansionStatusResponse {
    work_id: WorkId,
    job: JobView,
    expansion: Option<ExpansionView>,
}

async fn start_expand(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> Response {
    let work_id = match parse_work_id(&id) {
        Ok(work_id) => work_id,
        Err(response) => return *response,
    };
    let (options, direction) = match parse_options(&body) {
        Ok(parsed) => parsed,
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
    let jobs = state.expansion_jobs.clone();
    let current = jobs
        .start_directed(state, work.id, direction, options)
        .await;
    (StatusCode::ACCEPTED, Json(JobView::from(Some(current)))).into_response()
}

/// Query of `GET /api/works/{id}/expansion`.
#[derive(Debug, Default, Deserialize)]
struct StatusQuery {
    direction: Option<ExpansionDirection>,
}

async fn expansion_status(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    query: Result<Query<StatusQuery>, QueryRejection>,
) -> Response {
    let direction = match query {
        Ok(Query(query)) => query.direction.unwrap_or(ExpansionDirection::References),
        Err(rejection) => {
            return graph_error(
                StatusCode::BAD_REQUEST,
                "invalid_direction",
                &format!("The query is not valid: {rejection}"),
            );
        }
    };
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
    let expansion = if direction == ExpansionDirection::CitedBy {
        match state.cited_by_view_raw(graph, work.id).await {
            Ok(view) => view,
            Err(error) => return graph_query_failure(&error),
        }
    } else {
        match state.expansion_views_raw(graph, &[work.id]).await {
            Ok(mut views) => views.remove(&work.id.to_string()),
            Err(error) => return graph_query_failure(&error),
        }
    };
    let job = state.expansion_jobs.get_directed(work.id, direction).await;
    (
        StatusCode::OK,
        Json(ExpansionStatusResponse {
            work_id: work.id,
            job: JobView::from(job),
            expansion,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{IdentifierScheme, Visibility, WorkKind};
    use lysilogy_ingest::openalex::ProviderError;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::build_router,
        expansion::fixtures::{
            ScriptedTransport, marker, new_work, plain_state, record_json, test_graph,
        },
    };

    async fn request(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let response = build_router(state.clone(), None)
            .oneshot(
                builder
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

    async fn bare_state() -> AppState {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        std::mem::forget((library, data));
        state
    }

    async fn poll_until_finished(state: &AppState, work: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let (status, json) =
                    request(state, "GET", &format!("/api/works/{work}/expansion"), None).await;
                assert_eq!(status, StatusCode::OK, "{json}");
                if json["job"]["state"] != "running" {
                    return json;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("job finished before the timeout")
    }

    #[tokio::test]
    async fn should_return_400_when_expand_bounds_out_of_range() {
        let state = bare_state().await;
        let work = WorkId::new();
        let uri = format!("/api/works/{work}/expand");
        let cases = [
            (r#"{"depth":0}"#, "invalid_depth"),
            (r#"{"depth":4}"#, "invalid_depth"),
            (r#"{"depth":-1}"#, "invalid_depth"),
            (r#"{"depth":9223372036854775807}"#, "invalid_depth"),
            (r#"{"depth":"2"}"#, "invalid_body"),
            (r#"{"depth":2.5}"#, "invalid_body"),
            (r#"{"depth":null,"fanout":0}"#, "invalid_fanout"),
            (r#"{"fanout":11}"#, "invalid_fanout"),
            (r#"{"fanout":"5"}"#, "invalid_body"),
            (r#"{"refresh":"yes"}"#, "invalid_body"),
            (r#"{"local":1}"#, "invalid_body"),
            (r#"{"extra":1}"#, "invalid_body"),
            ("[1]", "invalid_body"),
            ("[]", "invalid_body"),
            ("null", "invalid_body"),
            ("\"depth\"", "invalid_body"),
            ("{", "invalid_body"),
            ("not json", "invalid_body"),
        ];
        for (body, code) in cases {
            let (status, json) = request(&state, "POST", &uri, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], code, "{body}");
        }
        let (status, json) = request(
            &state,
            "POST",
            "/api/works/not-a-work/expand",
            Some(r#"{"depth":99}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json["error"], "invalid_work_id",
            "the path is checked first"
        );
        let (status, json) = request(&state, "GET", "/api/works/not-a-work/expansion", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_work_id");
    }

    #[tokio::test]
    async fn should_return_503_when_expanding_without_graph() {
        let state = bare_state().await;
        let work = WorkId::new();
        for body in [
            None,
            Some(""),
            Some("  \n"),
            Some(r#"{"depth":2,"fanout":3}"#),
        ] {
            let (status, json) =
                request(&state, "POST", &format!("/api/works/{work}/expand"), body).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body:?}: {json}");
            assert_eq!(json["error"], "graph_unavailable");
        }
        let (status, json) =
            request(&state, "GET", &format!("/api/works/{work}/expansion"), None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "graph_unavailable");
    }

    #[tokio::test]
    async fn should_return_404_when_expanding_unknown_work() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(503)));
        let state = plain_state(graph, transport.clone()).await;
        let work = WorkId::new();
        let (status, json) =
            request(&state, "POST", &format!("/api/works/{work}/expand"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "work_not_found");
        let (status, json) =
            request(&state, "GET", &format!("/api/works/{work}/expansion"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "work_not_found");
        assert!(transport.urls().is_empty());
    }

    #[tokio::test]
    async fn should_report_succeeded_with_walk_report_when_expand_job_finishes() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let own = format!("W1{marker}");
        let record = record_json(&own, "A Stub Paper", &[format!("W2{marker}")]);
        let transport = ScriptedTransport::serving(vec![(own.clone(), record)]);
        let state = plain_state(graph.clone(), transport.clone()).await;
        let work = new_work(&graph, IdentifierScheme::Openalex, &own).await;

        let (status, json) =
            request(&state, "GET", &format!("/api/works/{work}/expansion"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["job"]["state"], "idle");
        assert!(json["expansion"].is_null(), "{json}");

        let (status, started) = request(
            &state,
            "POST",
            &format!("/api/works/{work}/expand"),
            Some(r#"{"depth":1,"fanout":2}"#),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{started}");
        let json = poll_until_finished(&state, &work.to_string()).await;
        assert_eq!(json["work_id"], work.to_string());
        assert_eq!(json["job"]["state"], "succeeded", "{json}");
        let report = &json["job"]["report"];
        assert_eq!(report["seed"], work.to_string());
        assert_eq!(report["levels"], 1);
        assert_eq!(report["truncated"], false);
        let expansion = &report["expansions"][0];
        assert_eq!(expansion["method"], "provider", "{json}");
        assert_eq!(expansion["state"], "expanded");
        assert_eq!(expansion["references"], 1);
        assert_eq!(expansion["cached"], false);
        assert_eq!(json["expansion"]["state"], "expanded");
        assert_eq!(json["expansion"]["references_count"], 1);

        // Asking again is a cache hit that finishes before the response.
        let before = transport.urls().len();
        let (status, again) = request(
            &state,
            "POST",
            &format!("/api/works/{work}/expand"),
            Some("   "),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(again["state"], "succeeded", "{again}");
        assert_eq!(again["report"]["expansions"][0]["cached"], true);
        assert_eq!(transport.urls().len(), before, "no network on a cache hit");
    }

    #[tokio::test]
    async fn should_report_failed_with_message_when_provider_errors() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(503)));
        let state = plain_state(graph.clone(), transport).await;
        let work = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W1{}", marker()),
        )
        .await;
        let (status, _) = request(&state, "POST", &format!("/api/works/{work}/expand"), None).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let json = poll_until_finished(&state, &work.to_string()).await;
        assert_eq!(json["job"]["state"], "failed", "{json}");
        assert!(json["job"]["message"].is_string(), "{json}");
        assert!(json["job"].get("report").is_none());
        assert!(json["expansion"].is_null());
    }

    #[tokio::test]
    async fn should_follow_redirect_when_expanding_merged_work() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let own = format!("W1{marker}");
        let record = record_json(&own, "Survivor", &[format!("W2{marker}")]);
        let transport = ScriptedTransport::serving(vec![(own.clone(), record)]);
        let state = plain_state(graph.clone(), transport).await;
        let survivor = new_work(&graph, IdentifierScheme::Openalex, &own).await;
        let merged = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("merged")
            .id;
        graph
            .works
            .merge_work(merged, survivor, "test", "test")
            .await
            .expect("merge");

        let (status, _) =
            request(&state, "POST", &format!("/api/works/{merged}/expand"), None).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let json = poll_until_finished(&state, &merged.to_string()).await;
        assert_eq!(json["work_id"], survivor.to_string(), "{json}");
        assert_eq!(json["job"]["state"], "succeeded", "{json}");
        assert_eq!(
            json["job"]["report"]["seed"],
            survivor.to_string(),
            "the job is keyed by the surviving Work"
        );
        let direct = poll_until_finished(&state, &survivor.to_string()).await;
        assert_eq!(direct["job"]["state"], "succeeded");
    }

    #[tokio::test]
    async fn should_finish_cache_hit_without_waiting_for_permit_when_permit_is_held() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(503)));
        let state = plain_state(graph.clone(), transport).await;
        let cached = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W1{}", marker()),
        )
        .await;
        let child = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W2{}", marker()),
        )
        .await;
        graph
            .citations
            .add_assertion(cached, child, None)
            .await
            .expect("edge");
        let uncached = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W3{}", marker()),
        )
        .await;

        let jobs = ExpansionJobs::default();
        let _held = jobs.permits.clone().acquire_owned().await.expect("permit");
        let finished = tokio::time::timeout(
            Duration::from_secs(5),
            jobs.start(state.clone(), cached, ExpandOptions::default()),
        )
        .await
        .expect("a cache hit does not wait for the permit");
        assert!(
            matches!(finished, ExpansionJobState::Succeeded(_)),
            "{finished:?}"
        );

        let queued = jobs.start(state, uncached, ExpandOptions::default()).await;
        assert!(matches!(queued, ExpansionJobState::Running));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            matches!(jobs.get(uncached).await, Some(ExpansionJobState::Running)),
            "a job that needs the provider waits for the permit"
        );
        let repeat = jobs
            .start(
                plain_state(
                    graph,
                    ScriptedTransport::new(|_| Err(ProviderError::Status(503))),
                )
                .await,
                uncached,
                ExpandOptions::default(),
            )
            .await;
        assert!(
            matches!(repeat, ExpansionJobState::Running),
            "one running job per Work"
        );
    }

    #[test]
    fn should_derive_idle_state_when_no_expansion_job_exists() {
        let view = JobView::from(None);
        assert_eq!(view.state, "idle");
        assert!(view.report.is_none() && view.message.is_none());
    }

    #[tokio::test]
    async fn should_reject_unknown_direction_and_deep_cited_by_when_parsing_body() {
        let state = bare_state().await;
        let uri = format!("/api/works/{}/expand", WorkId::new());
        let cases = [
            (r#"{"direction":"sideways"}"#, "invalid_body"),
            (r#"{"direction":1}"#, "invalid_body"),
            (r#"{"direction":"CITED_BY"}"#, "invalid_body"),
            (r#"{"direction":"cited_by","depth":2}"#, "invalid_depth"),
            (r#"{"direction":"cited_by","depth":0}"#, "invalid_depth"),
        ];
        for (body, code) in cases {
            let (status, json) = request(&state, "POST", &uri, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], code, "{body}");
        }
    }

    #[tokio::test]
    async fn should_return_json_error_when_status_query_has_bad_direction() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let state = plain_state(graph, ScriptedTransport::new(|_| Ok(serde_json::json!({})))).await;
        let base = format!("/api/works/{}/expansion", WorkId::new());
        let (status, json) =
            request(&state, "GET", &format!("{base}?direction=sideways"), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_direction");
        let (status, json) = request(&state, "GET", &format!("{base}?cache=1"), None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "extra parameters are ignored: {json}"
        );
    }

    #[tokio::test]
    async fn should_run_cited_by_job_and_report_its_status_when_direction_is_cited_by() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let own = format!("W1{marker}");
        let citer = format!("W2{marker}");
        let record = record_json(&own, "Own", &[]);
        let citing = serde_json::json!({"results": [{
            "id": format!("https://openalex.org/{citer}"),
            "display_name": "Citer", "publication_year": 2022, "authorships": []}]});
        let transport = ScriptedTransport::new(move |url| {
            if url.contains("filter=cites:") {
                return Ok(citing.clone());
            }
            Ok(record.clone())
        });
        let state = plain_state(graph.clone(), transport).await;
        let work = new_work(&graph, IdentifierScheme::Openalex, &own).await;

        let uri = format!("/api/works/{work}/expand");
        let (status, _) = request(&state, "POST", &uri, Some(r#"{"direction":"cited_by"}"#)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let done = {
            let status_uri = format!("/api/works/{work}/expansion?direction=cited_by");
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let (code, json) = request(&state, "GET", &status_uri, None).await;
                    assert_eq!(code, StatusCode::OK, "{json}");
                    if json["job"]["state"] != "running" {
                        return json;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("finished")
        };
        assert_eq!(done["job"]["state"], "succeeded", "{done}");
        assert_eq!(done["job"]["report"]["expansions"][0]["references"], 1);
        assert_eq!(done["expansion"]["state"], "expanded");

        let (_, references) =
            request(&state, "GET", &format!("/api/works/{work}/expansion"), None).await;
        assert_eq!(references["job"]["state"], "idle", "{references}");
        assert!(references["expansion"].is_null(), "{references}");
    }
}
