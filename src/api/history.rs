//! Reading-history endpoints: event ingest, the timeline, per-paper summaries and reading
//! positions.
//!
//! Check order everywhere: path IDs, body or query, graph availability, existence. The
//! router is exposed as [`history_routes`] and mounted by `build_router`.

// Not mounted yet: the generation task merges `history_routes()` into `build_router` and
// removes this allowance.
#![allow(dead_code)]

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use lysilogy_core::{ListId, WorkId};
use lysilogy_db::history::{HistoryQuery, NewEvent, PAPER_ID_MAX, ReadingPosition};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::{AppState, graph_error, graph_or_unavailable};
use crate::history::{HistoryService, HistoryServiceError, parse_cursor};

type Failure = Box<Response>;

/// The history routes, ready to merge into the application router.
pub fn history_routes() -> Router<AppState> {
    Router::new()
        .route("/api/history/events", post(ingest_events))
        .route("/api/history", get(read_timeline))
        .route("/api/history/inspected", get(read_inspected))
        .route("/api/history/barely-read", get(read_barely_read))
        .route("/api/history/papers/{id}", get(read_paper))
        .route("/api/papers/{id}/position", put(put_position))
}

// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn failure(status: StatusCode, code: &str, message: &str) -> Failure {
    Box::new(graph_error(status, code, message))
}

// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn invalid_body(detail: &str) -> Failure {
    failure(
        StatusCode::BAD_REQUEST,
        "invalid_body",
        &format!("The request body is not valid: {detail}"),
    )
}

/// Parse a body that must be a JSON object with exactly the known fields. The content type
/// is not checked, because `navigator.sendBeacon` sends batches as `text/plain`.
fn parse_object<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, Failure> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_body(&error.to_string()))?;
    if !value.is_object() {
        return Err(invalid_body("expected a JSON object"));
    }
    serde_json::from_value(value).map_err(|error| invalid_body(&error.to_string()))
}

// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn service_failure(error: &HistoryServiceError) -> Failure {
    let status = match error {
        HistoryServiceError::NotFound(_) => StatusCode::NOT_FOUND,
        HistoryServiceError::Invalid(..) => StatusCode::BAD_REQUEST,
        HistoryServiceError::Graph(inner) => {
            tracing::error!(error = %inner, "history request failed");
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "graph_error",
                "The graph query failed.",
            );
        }
    };
    failure(status, error.code(), &error.to_string())
}

fn respond<T: serde::Serialize>(status: StatusCode, value: &T) -> Response {
    (status, Json(value)).into_response()
}

fn finish(result: Result<Response, Failure>) -> Response {
    match result {
        Ok(response) => response,
        Err(response) => *response,
    }
}

async fn service_of(state: &AppState) -> Result<HistoryService, Failure> {
    let graph = graph_or_unavailable(state)?;
    HistoryService::connect(graph)
        .await
        .map_err(|error| service_failure(&error))
}

fn parse_paper_id(text: &str) -> Result<&str, Failure> {
    if text.is_empty() || text.chars().count() > PAPER_ID_MAX || text.contains('\0') {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "invalid_history_paper",
            &format!("A paper identifier is 1 to {PAPER_ID_MAX} characters."),
        ));
    }
    Ok(text)
}

// ---- POST /api/history/events -----------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsBody {
    events: Vec<EventBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventBody {
    occurred_at: DateTime<Utc>,
    session_id: Uuid,
    kind: String,
    paper_id: Option<String>,
    work_id: Option<String>,
    list_id: Option<String>,
    dwell_ms: Option<i32>,
    #[serde(default)]
    detail: Option<Value>,
    client_seq: i32,
}

fn event_from_body(index: usize, body: EventBody) -> Result<NewEvent, Failure> {
    let work_id = body
        .work_id
        .as_deref()
        .map(str::parse::<WorkId>)
        .transpose()
        .map_err(|_| {
            failure(
                StatusCode::BAD_REQUEST,
                "invalid_work_id",
                &format!(
                    "event {index}: work identifiers look like 'w' followed by 22 characters."
                ),
            )
        })?;
    let list_id = body
        .list_id
        .as_deref()
        .map(str::parse::<ListId>)
        .transpose()
        .map_err(|_| {
            failure(
                StatusCode::BAD_REQUEST,
                "invalid_list_id",
                &format!(
                    "event {index}: list identifiers look like 'p' followed by 22 characters."
                ),
            )
        })?;
    Ok(NewEvent {
        occurred_at: body.occurred_at,
        session_id: body.session_id,
        kind: body.kind,
        paper_id: body.paper_id,
        work_id,
        list_id,
        dwell_ms: body.dwell_ms,
        detail: body.detail.unwrap_or_else(|| serde_json::json!({})),
        client_seq: body.client_seq,
    })
}

async fn ingest_events(State(state): State<AppState>, body: Bytes) -> Response {
    finish(ingest_events_inner(&state, &body).await)
}

async fn ingest_events_inner(state: &AppState, body: &Bytes) -> Result<Response, Failure> {
    let parsed: EventsBody = parse_object(body)?;
    let events = parsed
        .events
        .into_iter()
        .enumerate()
        .map(|(index, event)| event_from_body(index, event))
        .collect::<Result<Vec<_>, _>>()?;
    let service = service_of(state).await?;
    let outcome = service
        .ingest(&events)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::ACCEPTED,
        &serde_json::json!({
            "accepted": outcome.accepted,
            "duplicates": outcome.duplicates,
            "nudged": outcome.nudged,
        }),
    ))
}

// ---- GET /api/history -------------------------------------------------------------------

#[derive(Deserialize)]
struct TimelineQuery {
    before: Option<String>,
    /// Comma-separated kinds.
    kinds: Option<String>,
    q: Option<String>,
    limit: Option<String>,
}

async fn read_timeline(
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> Response {
    finish(read_timeline_inner(&state, query).await)
}

async fn read_timeline_inner(state: &AppState, query: TimelineQuery) -> Result<Response, Failure> {
    let before = query
        .before
        .as_deref()
        .map(parse_cursor)
        .transpose()
        .map_err(|error| service_failure(&error))?;
    let kinds: Vec<String> = query
        .kinds
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::to_owned)
        .collect();
    let limit = query
        .limit
        .as_deref()
        .map(|text| {
            text.parse::<i64>()
                .ok()
                .filter(|limit| *limit >= 1)
                .ok_or_else(|| {
                    failure(
                        StatusCode::BAD_REQUEST,
                        "invalid_history_limit",
                        "The limit is a positive whole number.",
                    )
                })
        })
        .transpose()?;
    let service = service_of(state).await?;
    let timeline = service
        .timeline(&HistoryQuery {
            before,
            kinds,
            text: query.q,
            limit,
        })
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &timeline))
}

// ---- derived lists ----------------------------------------------------------------------

async fn read_inspected(State(state): State<AppState>) -> Response {
    finish(read_inspected_inner(&state).await)
}

async fn read_inspected_inner(state: &AppState) -> Result<Response, Failure> {
    let service = service_of(state).await?;
    let works = service
        .inspected_never_opened()
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "works": works }),
    ))
}

async fn read_barely_read(State(state): State<AppState>) -> Response {
    finish(read_barely_read_inner(&state).await)
}

async fn read_barely_read_inner(state: &AppState) -> Result<Response, Failure> {
    let service = service_of(state).await?;
    let papers = service
        .barely_read()
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "papers": papers }),
    ))
}

// ---- GET /api/history/papers/{id} -------------------------------------------------------

async fn read_paper(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    finish(read_paper_inner(&state, &id).await)
}

async fn read_paper_inner(state: &AppState, id: &str) -> Result<Response, Failure> {
    let paper = parse_paper_id(id)?;
    let service = service_of(state).await?;
    let summary = service
        .paper_summary(paper)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &summary))
}

// ---- PUT /api/papers/{id}/position ------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionBody {
    view: String,
    page: Option<i32>,
    offset_utf16: Option<i32>,
    section_id: Option<String>,
    scroll_ratio: Option<f32>,
}

async fn put_position(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(put_position_inner(&state, &id, &body).await)
}

async fn put_position_inner(state: &AppState, id: &str, body: &Bytes) -> Result<Response, Failure> {
    let paper = parse_paper_id(id)?;
    let parsed: PositionBody = parse_object(body)?;
    let service = service_of(state).await?;
    let saved = service
        .save_position(&ReadingPosition {
            paper_id: paper.to_owned(),
            view: parsed.view,
            page: parsed.page,
            offset_utf16: parsed.offset_utf16,
            section_id: parsed.section_id,
            scroll_ratio: parsed.scroll_ratio,
            updated_at: Utc::now(),
        })
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &saved))
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{IdentifierScheme, ReadingStatus};
    use lysilogy_db::reading_state::ReadingStateRepository;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::expansion::fixtures::{
        ScriptedTransport, marker, new_work, plain_state, test_graph,
    };
    use crate::graph::GraphState;

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<(&str, &str)>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some((content_type, _)) = body {
            builder = builder.header("content-type", content_type);
        }
        let request = builder
            .body(body.map_or_else(Body::empty, |(_, text)| Body::from(text.to_owned())))
            .expect("request");
        // The router under test is `history_routes()` alone, as T7 mounts it.
        let response = history_routes()
            .with_state(state.clone())
            .oneshot(request)
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

    async fn post_json(state: &AppState, uri: &str, body: &Value) -> (StatusCode, Value) {
        send(
            state,
            "POST",
            uri,
            Some(("application/json", &body.to_string())),
        )
        .await
    }

    struct Fixture {
        state: AppState,
        graph: GraphState,
        marker: String,
    }

    impl Fixture {
        async fn new() -> Option<Self> {
            let graph = test_graph().await?;
            let state = plain_state(graph.clone(), ScriptedTransport::serving(Vec::new())).await;
            Some(Self {
                state,
                graph,
                marker: marker(),
            })
        }

        async fn work(&self, label: &str) -> WorkId {
            new_work(
                &self.graph,
                IdentifierScheme::Doi,
                &format!("10.9999/{}-{label}", self.marker),
            )
            .await
        }

        fn paper(&self) -> String {
            format!("paper-{}", self.marker)
        }

        fn event(&self, session: Uuid, seq: i32, kind: &str) -> Value {
            json!({
                "occurred_at": Utc::now().to_rfc3339(),
                "session_id": session,
                "kind": kind,
                "paper_id": self.paper(),
                "client_seq": seq,
            })
        }
    }

    #[tokio::test]
    async fn should_answer_503_for_every_route_when_no_database_is_configured() {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        let batch = json!({ "events": [] });
        let (status, body) = post_json(&state, "/api/history/events", &batch).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "graph_unavailable");
        for uri in [
            "/api/history",
            "/api/history/papers/x",
            "/api/history/inspected",
            "/api/history/barely-read",
        ] {
            let (status, _) = send(&state, "GET", uri, None).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        }
    }

    #[tokio::test]
    async fn should_check_the_body_before_the_database_when_ingesting() {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        for body in [
            "",
            "[]",
            "{\"events\": 3}",
            "{\"events\": [], \"extra\": 1}",
        ] {
            let (status, payload) = send(
                &state,
                "POST",
                "/api/history/events",
                Some(("text/plain", body)),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
            assert_eq!(payload["error"], "invalid_body");
        }
        let bad_work = json!({ "events": [{
            "occurred_at": "2026-10-24T10:00:00Z",
            "session_id": Uuid::new_v4(),
            "kind": "paper_opened",
            "work_id": "w-nope",
            "client_seq": 0,
        }]});
        let (status, payload) = post_json(&state, "/api/history/events", &bad_work).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(payload["error"], "invalid_work_id");
    }

    #[tokio::test]
    async fn should_accept_a_batch_and_report_duplicates_when_it_is_sent_twice() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let batch = json!({ "events": [
            f.event(session, 0, "paper_opened"),
            f.event(session, 1, "view_changed"),
        ]});
        let (status, body) = post_json(&f.state, "/api/history/events", &batch).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(
            (body["accepted"].as_u64(), body["duplicates"].as_u64()),
            (Some(2), Some(0))
        );
        let (status, body) = post_json(&f.state, "/api/history/events", &batch).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(
            (body["accepted"].as_u64(), body["duplicates"].as_u64()),
            (Some(0), Some(2))
        );
    }

    #[tokio::test]
    async fn should_accept_a_text_plain_body_when_sent_as_a_beacon() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let batch = json!({ "events": [f.event(Uuid::new_v4(), 0, "paper_opened")] });
        let (status, body) = send(
            &f.state,
            "POST",
            "/api/history/events",
            Some(("text/plain;charset=UTF-8", &batch.to_string())),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["accepted"], 1);
    }

    #[tokio::test]
    async fn should_reject_unknown_kinds_negative_dwell_and_oversized_batches_when_ingesting() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let bad_kind = json!({ "events": [f.event(session, 0, "teleported")] });
        let (status, body) = post_json(&f.state, "/api/history/events", &bad_kind).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_history_kind");

        let mut negative = f.event(session, 1, "paper_left");
        negative["dwell_ms"] = json!(-5);
        let (status, body) = post_json(
            &f.state,
            "/api/history/events",
            &json!({ "events": [negative] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_history_dwell");

        let many: Vec<Value> = (0..201)
            .map(|seq| f.event(session, seq, "paper_opened"))
            .collect();
        let (status, body) =
            post_json(&f.state, "/api/history/events", &json!({ "events": many })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_history_batch");

        let mut huge = f.event(session, 2, "paper_left");
        huge["dwell_ms"] = json!(99_999_999_999_i64);
        let (status, body) = post_json(
            &f.state,
            "/api/history/events",
            &json!({ "events": [huge] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_body");
    }

    #[tokio::test]
    async fn should_nudge_the_work_to_reading_when_a_long_visit_is_ingested() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let work = f.work("nudge").await;
        let mut left = f.event(Uuid::new_v4(), 0, "paper_left");
        left["work_id"] = json!(work.to_string());
        left["dwell_ms"] = json!(crate::history::NUDGE_DWELL_MS);
        let (status, body) = post_json(
            &f.state,
            "/api/history/events",
            &json!({ "events": [left] }),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["nudged"], json!([work.to_string()]));
        let workspace = f
            .graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let status = ReadingStateRepository::new(f.graph.works.pool().clone())
            .get(workspace, work)
            .await
            .expect("status");
        assert_eq!(status, ReadingStatus::Reading);
    }

    #[tokio::test]
    async fn should_return_sessions_filtered_by_kind_and_text_when_reading_the_timeline() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let mut search = f.event(session, 0, "search_run");
        search["detail"] = json!({ "query": format!("rlhf {} ünïcode", f.marker) });
        let batch = json!({ "events": [search, f.event(session, 1, "paper_opened")] });
        post_json(&f.state, "/api/history/events", &batch).await;

        let uri = format!("/api/history?kinds=search_run&q={}", f.marker);
        let (status, body) = send(&f.state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        let sessions = body["sessions"].as_array().expect("sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["session_id"], json!(session));
        let events = sessions[0]["events"].as_array().expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["kind"], "search_run");
        assert!(body["next_before"].is_null());

        let (status, body) = send(
            &f.state,
            "GET",
            "/api/history?kinds=search_run,paper_opened&limit=1",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body["next_before"].is_string(),
            "a full page offers a cursor"
        );
    }

    #[tokio::test]
    async fn should_reject_bad_query_parameters_when_reading_the_timeline() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        for (uri, code) in [
            ("/api/history?kinds=nope", "invalid_history_kind"),
            ("/api/history?before=yesterday", "invalid_history_cursor"),
            ("/api/history?limit=0", "invalid_history_limit"),
            ("/api/history?limit=-3", "invalid_history_limit"),
            ("/api/history?limit=abc", "invalid_history_limit"),
        ] {
            let (status, body) = send(&f.state, "GET", uri, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(body["error"], code, "{uri}");
        }
    }

    #[tokio::test]
    async fn should_save_a_position_and_show_it_with_the_summary_when_reading_a_paper() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let paper = f.paper();
        let uri = format!("/api/history/papers/{paper}");
        let (status, body) = send(&f.state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "paper_not_found");

        let (status, body) = send(
            &f.state,
            "PUT",
            &format!("/api/papers/{paper}/position"),
            Some((
                "application/json",
                &json!({ "view": "pdf", "page": 12, "scroll_ratio": 0.4 }).to_string(),
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["page"], 12);

        let session = Uuid::new_v4();
        let mut left = f.event(session, 1, "paper_left");
        left["dwell_ms"] = json!(30_000);
        let mut focus = f.event(session, 2, "section_focused");
        focus["dwell_ms"] = json!(4_000);
        focus["detail"] = json!({ "section_id": "intro" });
        post_json(
            &f.state,
            "/api/history/events",
            &json!({ "events": [f.event(session, 0, "paper_opened"), left, focus] }),
        )
        .await;
        let (status, body) = send(&f.state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["visits"], 1);
        assert_eq!(body["dwell_ms"], 30_000);
        assert_eq!(body["barely_read"], true);
        assert_eq!(body["sections"][0]["section_id"], "intro");
        assert_eq!(body["position"]["page"], 12);
    }

    #[tokio::test]
    async fn should_reject_bad_positions_when_saving() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let uri = format!("/api/papers/{}/position", f.paper());
        for (body, code) in [
            (json!({ "view": "scroll" }), "invalid_position"),
            (json!({ "view": "pdf", "page": -1 }), "invalid_position"),
            (
                json!({ "view": "pdf", "scroll_ratio": 2.0 }),
                "invalid_position",
            ),
            (json!({ "page": 1 }), "invalid_body"),
            (json!({ "view": "pdf", "bogus": 1 }), "invalid_body"),
        ] {
            let (status, payload) = send(
                &f.state,
                "PUT",
                &uri,
                Some(("application/json", &body.to_string())),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(payload["error"], code, "{body}");
        }
        let long = "x".repeat(PAPER_ID_MAX + 1);
        let (status, payload) = send(
            &f.state,
            "PUT",
            &format!("/api/papers/{long}/position"),
            Some(("application/json", &json!({ "view": "pdf" }).to_string())),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(payload["error"], "invalid_history_paper");
    }

    #[tokio::test]
    async fn should_list_inspected_never_opened_and_barely_read_when_asked() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let peeked = f.work("peeked").await;
        let session = Uuid::new_v4();
        let mut inspect = f.event(session, 0, "work_inspected");
        inspect["work_id"] = json!(peeked.to_string());
        inspect["paper_id"] = Value::Null;
        let mut left = f.event(session, 2, "paper_left");
        left["dwell_ms"] = json!(10_000);
        post_json(
            &f.state,
            "/api/history/events",
            &json!({ "events": [inspect, f.event(session, 1, "paper_opened"), left] }),
        )
        .await;

        let (status, body) = send(&f.state, "GET", "/api/history/inspected", None).await;
        assert_eq!(status, StatusCode::OK);
        let wanted = peeked.to_string();
        assert!(
            body["works"]
                .as_array()
                .expect("works")
                .iter()
                .any(|work| work["work_id"].as_str() == Some(wanted.as_str()))
        );

        let (status, body) = send(&f.state, "GET", "/api/history/barely-read", None).await;
        assert_eq!(status, StatusCode::OK);
        let wanted = f.paper();
        assert!(
            body["papers"]
                .as_array()
                .expect("papers")
                .iter()
                .any(|paper| paper["paper_id"].as_str() == Some(wanted.as_str()))
        );
    }
}
