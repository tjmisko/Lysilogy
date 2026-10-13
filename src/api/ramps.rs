//! Reading-ramp endpoints: ramp CRUD, items, and ordering.
//!
//! Check order everywhere: path IDs, body, graph availability, existence.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
};
use lysilogy_core::{RampError, RampId, RampItemStatus, WorkId};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{AppState, graph_error, graph_or_unavailable};
use crate::ramps::{RampServiceError, parse_note, parse_status, parse_title};

type Failure = Box<Response>;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/ramps", get(list_ramps).post(create_ramp))
        .route(
            "/api/ramps/{ramp}",
            get(read_ramp).patch(rename_ramp).delete(delete_ramp),
        )
        .route("/api/ramps/{ramp}/items", post(add_item))
        .route(
            "/api/ramps/{ramp}/items/{work}",
            patch(patch_item).delete(remove_item),
        )
        .route("/api/ramps/{ramp}/order", put(order_ramp))
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

/// Parse a body that must be a JSON object with exactly the known fields.
fn parse_object<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, Failure> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_body(&error.to_string()))?;
    if !value.is_object() {
        return Err(invalid_body("expected a JSON object"));
    }
    serde_json::from_value(value).map_err(|error| invalid_body(&error.to_string()))
}

fn parse_ramp_id(text: &str) -> Result<RampId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_ramp_id",
            "Ramp identifiers look like 'p' followed by 22 characters.",
        )
    })
}

fn parse_work(text: &str) -> Result<WorkId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_work_id",
            "Work identifiers look like 'w' followed by 22 characters.",
        )
    })
}

/// The HTTP form of a service error.
// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn service_failure(error: &RampServiceError) -> Failure {
    let status = match error {
        RampServiceError::NotFound(_) => StatusCode::NOT_FOUND,
        RampServiceError::Invalid(..) => StatusCode::BAD_REQUEST,
        RampServiceError::Conflict(_) => StatusCode::CONFLICT,
        RampServiceError::Graph(inner) => {
            tracing::error!(error = %inner, "ramp request failed");
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

#[derive(Deserialize)]
struct ListQuery {
    seed: Option<String>,
}

async fn list_ramps(State(state): State<AppState>, Query(query): Query<ListQuery>) -> Response {
    finish(list_ramps_inner(&state, query).await)
}

async fn list_ramps_inner(state: &AppState, query: ListQuery) -> Result<Response, Failure> {
    let seed = query.seed.as_deref().map(parse_work).transpose()?;
    graph_or_unavailable(state)?;
    let ramps = state
        .list_ramps(seed)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "ramps": ramps }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRampBody {
    seed_work_id: String,
    title: Option<String>,
}

async fn create_ramp(State(state): State<AppState>, body: Bytes) -> Response {
    finish(create_ramp_inner(&state, &body).await)
}

async fn create_ramp_inner(state: &AppState, body: &Bytes) -> Result<Response, Failure> {
    let body: CreateRampBody = parse_object(body)?;
    let seed = parse_work(&body.seed_work_id)?;
    let title = body
        .title
        .as_deref()
        .map(parse_title)
        .transpose()
        .map_err(|error| service_failure(&error))?;
    graph_or_unavailable(state)?;
    let detail = state
        .create_ramp(seed, title)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::CREATED, &detail))
}

async fn read_ramp(State(state): State<AppState>, AxumPath(ramp): AxumPath<String>) -> Response {
    finish(read_ramp_inner(&state, &ramp).await)
}

async fn read_ramp_inner(state: &AppState, ramp: &str) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    graph_or_unavailable(state)?;
    let detail = state
        .ramp_detail(ramp)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &detail))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameRampBody {
    title: String,
}

async fn rename_ramp(
    State(state): State<AppState>,
    AxumPath(ramp): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(rename_ramp_inner(&state, &ramp, &body).await)
}

async fn rename_ramp_inner(
    state: &AppState,
    ramp: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    let body: RenameRampBody = parse_object(body)?;
    let title = parse_title(&body.title).map_err(|error| service_failure(&error))?;
    graph_or_unavailable(state)?;
    let summary = state
        .rename_ramp(ramp, &title)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &summary))
}

async fn delete_ramp(State(state): State<AppState>, AxumPath(ramp): AxumPath<String>) -> Response {
    finish(delete_ramp_inner(&state, &ramp).await)
}

async fn delete_ramp_inner(state: &AppState, ramp: &str) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    graph_or_unavailable(state)?;
    state
        .delete_ramp(ramp)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddItemBody {
    work_id: String,
    via_work_id: Option<String>,
    note: Option<String>,
}

async fn add_item(
    State(state): State<AppState>,
    AxumPath(ramp): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(add_item_inner(&state, &ramp, &body).await)
}

async fn add_item_inner(state: &AppState, ramp: &str, body: &Bytes) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    let body: AddItemBody = parse_object(body)?;
    let work = parse_work(&body.work_id)?;
    let via = body.via_work_id.as_deref().map(parse_work).transpose()?;
    let note = parse_note(body.note.as_deref()).map_err(|error| service_failure(&error))?;
    graph_or_unavailable(state)?;
    let item = state
        .add_ramp_item(ramp, work, via, note)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::CREATED, &item))
}

/// Keep `null` distinct from an absent field: `null` becomes `Some(Value::Null)`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchItemBody {
    #[serde(default, deserialize_with = "present")]
    status: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    note: Option<Value>,
}

impl PatchItemBody {
    /// The status to set, if given.
    fn status(&self) -> Result<Option<RampItemStatus>, Failure> {
        let Some(value) = &self.status else {
            return Ok(None);
        };
        let Some(text) = value.as_str() else {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "invalid_status",
                "Status is one of unread, reading, read.",
            ));
        };
        parse_status(text)
            .map(Some)
            .map_err(|error| service_failure(&error))
    }

    /// The note change: absent leaves it, `null` or blank clears it. The shape matches
    /// the repository patch.
    #[allow(clippy::option_option)]
    fn note(&self) -> Result<Option<Option<String>>, Failure> {
        let Some(value) = &self.note else {
            return Ok(None);
        };
        match value {
            Value::Null => Ok(Some(None)),
            Value::String(text) => parse_note(Some(text))
                .map(Some)
                .map_err(|error| service_failure(&error)),
            _ => Err(failure(
                StatusCode::BAD_REQUEST,
                "invalid_note",
                "A note is text, or null to clear it.",
            )),
        }
    }
}

async fn patch_item(
    State(state): State<AppState>,
    AxumPath((ramp, work)): AxumPath<(String, String)>,
    body: Bytes,
) -> Response {
    finish(patch_item_inner(&state, &ramp, &work, &body).await)
}

async fn patch_item_inner(
    state: &AppState,
    ramp: &str,
    work: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    let work = parse_work(work)?;
    let body: PatchItemBody = parse_object(body)?;
    if body.status.is_none() && body.note.is_none() {
        return Err(invalid_body("at least one of status, note is required"));
    }
    let status = body.status()?;
    let note = body.note()?;
    graph_or_unavailable(state)?;
    let item = state
        .update_ramp_item(ramp, work, status, note)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &item))
}

async fn remove_item(
    State(state): State<AppState>,
    AxumPath((ramp, work)): AxumPath<(String, String)>,
) -> Response {
    finish(remove_item_inner(&state, &ramp, &work).await)
}

async fn remove_item_inner(state: &AppState, ramp: &str, work: &str) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    let work = parse_work(work)?;
    graph_or_unavailable(state)?;
    state
        .remove_ramp_item(ramp, work)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderBody {
    work_ids: Option<Vec<String>>,
    strategy: Option<String>,
}

/// The only strategy the API offers.
const STRATEGY_FOUNDATIONAL_FIRST: &str = "foundational_first";

async fn order_ramp(
    State(state): State<AppState>,
    AxumPath(ramp): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(order_ramp_inner(&state, &ramp, &body).await)
}

async fn order_ramp_inner(state: &AppState, ramp: &str, body: &Bytes) -> Result<Response, Failure> {
    let ramp = parse_ramp_id(ramp)?;
    let body: OrderBody = parse_object(body)?;
    let invalid_order = || {
        failure(
            StatusCode::BAD_REQUEST,
            RampError::InvalidOrder.code(),
            &RampError::InvalidOrder.to_string(),
        )
    };
    let manual: Option<Vec<WorkId>> = match (&body.work_ids, &body.strategy) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(invalid_body("give exactly one of work_ids or strategy"));
        }
        (None, Some(strategy)) => {
            if strategy != STRATEGY_FOUNDATIONAL_FIRST {
                return Err(failure(
                    StatusCode::BAD_REQUEST,
                    "invalid_strategy",
                    "The only strategy is foundational_first.",
                ));
            }
            None
        }
        (Some(ids), None) => Some(
            ids.iter()
                .map(|id| id.parse::<WorkId>().map_err(|_| invalid_order()))
                .collect::<Result<_, _>>()?,
        ),
    };
    graph_or_unavailable(state)?;
    let detail = match manual {
        Some(order) => state.reorder_ramp(ramp, &order).await,
        None => state.suggest_ramp_order(ramp).await,
    }
    .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &detail))
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::IdentifierScheme;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::build_router,
        expansion::fixtures::{ScriptedTransport, marker, new_work, plain_state, test_graph},
        graph::GraphState,
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

    async fn call(state: &AppState, method: &str, uri: &str, body: &Value) -> (StatusCode, Value) {
        request(state, method, uri, Some(&body.to_string())).await
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

        /// A titled Work; `label` keeps identifiers unique within the test.
        async fn work(&self, label: &str) -> WorkId {
            let id = new_work(
                &self.graph,
                IdentifierScheme::Doi,
                &format!("10.9999/{}-{label}", self.marker),
            )
            .await;
            self.graph
                .works
                .set_display_title(id, &format!("Title {label} {}", self.marker))
                .await
                .expect("title");
            id
        }

        async fn cite(&self, citing: WorkId, cited: WorkId) {
            self.graph
                .citations
                .add_assertion(citing, cited, None)
                .await
                .expect("assertion");
        }

        async fn create(&self, seed: WorkId) -> String {
            let (status, json) = call(
                &self.state,
                "POST",
                "/api/ramps",
                &json!({"seed_work_id": seed.to_string()}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{json}");
            json["ramp"]["id"].as_str().expect("ramp id").to_owned()
        }

        async fn add(&self, ramp: &str, work: WorkId, via: Option<WorkId>) -> (StatusCode, Value) {
            let mut body = json!({"work_id": work.to_string()});
            if let Some(via) = via {
                body["via_work_id"] = json!(via.to_string());
            }
            call(
                &self.state,
                "POST",
                &format!("/api/ramps/{ramp}/items"),
                &body,
            )
            .await
        }

        async fn delete(&self, ramp: &str) {
            let (status, _) =
                request(&self.state, "DELETE", &format!("/api/ramps/{ramp}"), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }

        async fn order(&self, ramp: &str) -> Vec<String> {
            let (status, json) =
                request(&self.state, "GET", &format!("/api/ramps/{ramp}"), None).await;
            assert_eq!(status, StatusCode::OK, "{json}");
            json["items"]
                .as_array()
                .expect("items")
                .iter()
                .map(|item| item["work"]["id"].as_str().expect("id").to_owned())
                .collect()
        }
    }

    #[tokio::test]
    async fn should_return_503_when_ramps_requested_without_graph() {
        let state = bare_state().await;
        let ramp = RampId::new();
        let work = WorkId::new();
        let (status, json) = request(&state, "GET", "/api/ramps", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "graph_unavailable");
        for (method, uri, body) in [
            (
                "POST",
                "/api/ramps".to_owned(),
                json!({"seed_work_id": work.to_string()}),
            ),
            ("PATCH", format!("/api/ramps/{ramp}"), json!({"title": "x"})),
            (
                "POST",
                format!("/api/ramps/{ramp}/items"),
                json!({"work_id": work.to_string()}),
            ),
            (
                "PATCH",
                format!("/api/ramps/{ramp}/items/{work}"),
                json!({"status": "read"}),
            ),
            (
                "PUT",
                format!("/api/ramps/{ramp}/order"),
                json!({"strategy": "foundational_first"}),
            ),
        ] {
            let (status, json) = call(&state, method, &uri, &body).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
            assert_eq!(json["error"], "graph_unavailable");
        }
        for (method, uri) in [
            ("GET", format!("/api/ramps/{ramp}")),
            ("DELETE", format!("/api/ramps/{ramp}")),
            ("DELETE", format!("/api/ramps/{ramp}/items/{work}")),
        ] {
            let (status, _) = request(&state, method, &uri, None).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn should_return_400_when_ramp_or_work_ids_are_malformed() {
        let state = bare_state().await;
        let work = WorkId::new();
        let ramp = RampId::new();
        // A Work ID in the ramp slot, a ramp ID in the Work slot, garbage.
        for (method, uri, body, code) in [
            ("GET", format!("/api/ramps/{work}"), None, "invalid_ramp_id"),
            (
                "DELETE",
                "/api/ramps/nonsense".to_owned(),
                None,
                "invalid_ramp_id",
            ),
            (
                "GET",
                format!("/api/ramps?seed={ramp}"),
                None,
                "invalid_work_id",
            ),
            (
                "POST",
                "/api/ramps".to_owned(),
                Some(json!({"seed_work_id": ramp.to_string()})),
                "invalid_work_id",
            ),
            (
                "POST",
                format!("/api/ramps/{ramp}/items"),
                Some(json!({"work_id": "nope"})),
                "invalid_work_id",
            ),
            (
                "POST",
                format!("/api/ramps/{ramp}/items"),
                Some(json!({"work_id": work.to_string(), "via_work_id": ramp.to_string()})),
                "invalid_work_id",
            ),
            (
                "PATCH",
                format!("/api/ramps/{ramp}/items/{ramp}"),
                Some(json!({"status": "read"})),
                "invalid_work_id",
            ),
            (
                "DELETE",
                format!("/api/ramps/{ramp}/items/bad"),
                None,
                "invalid_work_id",
            ),
            (
                "PUT",
                format!("/api/ramps/{work}/order"),
                Some(json!({"strategy": "foundational_first"})),
                "invalid_ramp_id",
            ),
        ] {
            let (status, json) = match &body {
                Some(body) => call(&state, method, &uri, body).await,
                None => request(&state, method, &uri, None).await,
            };
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}: {json}");
            assert_eq!(json["error"], code, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn should_return_400_when_body_is_not_an_object_or_has_unknown_fields() {
        let state = bare_state().await;
        let work = WorkId::new();
        for body in ["[1]", "null", "\"x\"", "{", ""] {
            let (status, json) = request(&state, "POST", "/api/ramps", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error"], "invalid_body", "{body}");
        }
        let (status, json) = call(
            &state,
            "POST",
            "/api/ramps",
            &json!({"seed_work_id": work.to_string(), "color": "red"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_body");
    }

    #[tokio::test]
    async fn should_create_ramp_with_seed_item_and_default_title() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            "/api/ramps",
            &json!({"seed_work_id": seed.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(
            json["ramp"]["title"],
            format!("Title seed {}", fixture.marker)
        );
        assert_eq!(json["ramp"]["seed_work_id"], seed.to_string());
        assert_eq!(json["ramp"]["item_count"], 1);
        assert_eq!(json["ramp"]["status_counts"]["unread"], 1);
        let items = json["items"].as_array().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["ordinal"], 1);
        assert_eq!(items[0]["depth"], 0);
        assert_eq!(items[0]["via_work_id"], Value::Null);
        assert_eq!(items[0]["status"], "unread");
        assert_eq!(items[0]["path"][0]["work_id"], seed.to_string());

        let (status, titled) = call(
            &fixture.state,
            "POST",
            "/api/ramps",
            &json!({"seed_work_id": seed.to_string(), "title": "  My   ramp "}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(titled["ramp"]["title"], "My ramp");

        let (status, json) = call(
            &fixture.state,
            "POST",
            "/api/ramps",
            &json!({"seed_work_id": WorkId::new().to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "work_not_found");

        fixture.delete(json_id(&titled)).await;
        let created = fixture.state.list_ramps(Some(seed)).await.expect("list");
        for summary in created {
            fixture.delete(&summary.id.to_string()).await;
        }
    }

    fn json_id(detail: &Value) -> &str {
        detail["ramp"]["id"].as_str().expect("ramp id")
    }

    #[tokio::test]
    async fn should_return_400_when_ramp_title_invalid() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let ramp = fixture.create(seed).await;
        let too_long = "x".repeat(121);
        for title in [
            json!(""),
            json!("   "),
            json!(too_long),
            json!("bad\u{7}title"),
            json!(5),
            json!(["a"]),
        ] {
            let (status, json) = call(
                &fixture.state,
                "POST",
                "/api/ramps",
                &json!({"seed_work_id": seed.to_string(), "title": title}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{title}: {json}");
            let (status, _) = call(
                &fixture.state,
                "PATCH",
                &format!("/api/ramps/{ramp}"),
                &json!({"title": title}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{title}");
        }
        let (_, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/ramps/{ramp}"),
            &json!({"title": ""}),
        )
        .await;
        assert_eq!(json["error"], "invalid_ramp_title");

        let (status, renamed) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/ramps/{ramp}"),
            &json!({"title": " Renamed   ramp "}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{renamed}");
        assert_eq!(renamed["title"], "Renamed ramp");
        assert_eq!(renamed["item_count"], 1);
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_list_only_ramps_for_seed_when_seed_given() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let first_seed = fixture.work("first").await;
        let second_seed = fixture.work("second").await;
        let first_ramp = fixture.create(first_seed).await;
        let second_ramp = fixture.create(second_seed).await;

        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/ramps?seed={first_seed}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let ids: Vec<&str> = json["ramps"]
            .as_array()
            .expect("ramps")
            .iter()
            .map(|ramp| ramp["id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, vec![first_ramp.as_str()]);
        assert_eq!(
            json["ramps"][0]["seed_title"],
            format!("Title first {}", fixture.marker)
        );

        let (_, all) = request(&fixture.state, "GET", "/api/ramps", None).await;
        let all_ids: Vec<&str> = all["ramps"]
            .as_array()
            .expect("ramps")
            .iter()
            .map(|ramp| ramp["id"].as_str().expect("id"))
            .collect();
        assert!(all_ids.contains(&first_ramp.as_str()));
        assert!(all_ids.contains(&second_ramp.as_str()));
        let first_position = all_ids.iter().position(|id| *id == first_ramp);
        let second_position = all_ids.iter().position(|id| *id == second_ramp);
        assert!(
            second_position < first_position,
            "most recently updated first"
        );
        fixture.delete(&first_ramp).await;
        fixture.delete(&second_ramp).await;
    }

    #[tokio::test]
    async fn should_add_item_with_path_and_titles_when_via_cites_work() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let first = fixture.work("a").await;
        let second = fixture.work("b").await;
        fixture.cite(seed, first).await;
        fixture.cite(first, second).await;
        let ramp = fixture.create(seed).await;

        let (status, item) = fixture.add(&ramp, first, Some(seed)).await;
        assert_eq!(status, StatusCode::CREATED, "{item}");
        assert_eq!(item["depth"], 1);
        assert_eq!(item["ordinal"], 2);
        assert_eq!(item["via_work_id"], seed.to_string());
        let (status, deep) = call(
            &fixture.state,
            "POST",
            &format!("/api/ramps/{ramp}/items"),
            &json!({"work_id": second.to_string(), "via_work_id": first.to_string(), "note": "  read me "}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{deep}");
        assert_eq!(deep["depth"], 2);
        assert_eq!(deep["ordinal"], 3);
        assert_eq!(deep["note"], "read me");
        assert_eq!(deep["status"], "unread");
        let path: Vec<&str> = deep["path"]
            .as_array()
            .expect("path")
            .iter()
            .map(|step| step["work_id"].as_str().expect("id"))
            .collect();
        assert_eq!(
            path,
            vec![seed.to_string(), first.to_string(), second.to_string()]
        );
        assert_eq!(
            deep["path"][1]["title"],
            format!("Title a {}", fixture.marker)
        );
        assert_eq!(deep["work"]["id"], second.to_string());

        let (_, detail) = request(&fixture.state, "GET", &format!("/api/ramps/{ramp}"), None).await;
        assert_eq!(detail["ramp"]["item_count"], 3);
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_return_409_codes_when_item_add_conflicts() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let cited = fixture.work("cited").await;
        let stranger = fixture.work("stranger").await;
        fixture.cite(seed, cited).await;
        let ramp = fixture.create(seed).await;

        let (status, json) = fixture.add(&ramp, cited, Some(seed)).await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        let (status, json) = fixture.add(&ramp, cited, Some(seed)).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("item_exists"))
        );
        let (status, json) = fixture.add(&ramp, seed, None).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("item_exists"))
        );
        let (status, json) = fixture.add(&ramp, stranger, Some(stranger)).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("via_not_in_ramp"))
        );
        let (status, json) = fixture.add(&ramp, stranger, Some(cited)).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("no_citation_edge"))
        );

        // A chain deep enough that the ninth step exceeds the path limit.
        let mut previous = cited;
        for step in 0..7 {
            let next = fixture.work(&format!("chain{step}")).await;
            fixture.cite(previous, next).await;
            let (status, json) = fixture.add(&ramp, next, Some(previous)).await;
            assert_eq!(status, StatusCode::CREATED, "{step}: {json}");
            previous = next;
        }
        let too_deep = fixture.work("toodeep").await;
        fixture.cite(previous, too_deep).await;
        let (status, json) = fixture.add(&ramp, too_deep, Some(previous)).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("ramp_too_deep"))
        );
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_return_404_when_adding_unknown_work_or_to_unknown_ramp() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let ramp = fixture.create(seed).await;
        let (status, json) = fixture.add(&ramp, WorkId::new(), None).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("work_not_found"))
        );
        let (status, json) = fixture.add(&RampId::new().to_string(), seed, None).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("ramp_not_found"))
        );
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/ramps/{}", RampId::new()),
            None,
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("ramp_not_found"))
        );
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/ramps/{}/items/{seed}", RampId::new()),
            &json!({"status": "read"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("ramp_not_found"))
        );
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/ramps/{ramp}/items/{}", WorkId::new()),
            &json!({"status": "read"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("item_not_found"))
        );
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_update_status_and_note_and_clear_note_with_null() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let ramp = fixture.create(seed).await;
        let uri = format!("/api/ramps/{ramp}/items/{seed}");

        let (status, item) = call(
            &fixture.state,
            "PATCH",
            &uri,
            &json!({"status": "reading", "note": " start here "}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{item}");
        assert_eq!(item["status"], "reading");
        assert_eq!(item["note"], "start here");
        assert!(item["status_changed_at"].is_string());

        // Only the note: the status stays.
        let (status, item) = call(&fixture.state, "PATCH", &uri, &json!({"note": "second"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item["status"], "reading");
        assert_eq!(item["note"], "second");

        let (status, item) = call(&fixture.state, "PATCH", &uri, &json!({"note": null})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item["note"], Value::Null);
        assert_eq!(item["status"], "reading");

        call(&fixture.state, "PATCH", &uri, &json!({"note": "again"})).await;
        let (_, item) = call(&fixture.state, "PATCH", &uri, &json!({"note": ""})).await;
        assert_eq!(item["note"], Value::Null, "an empty note clears it");

        let (status, item) = call(&fixture.state, "PATCH", &uri, &json!({"status": "read"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item["status"], "read");
        let (_, detail) = request(&fixture.state, "GET", &format!("/api/ramps/{ramp}"), None).await;
        assert_eq!(detail["ramp"]["status_counts"]["read"], 1);
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_return_400_when_status_unknown_or_patch_empty() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let ramp = fixture.create(seed).await;
        let uri = format!("/api/ramps/{ramp}/items/{seed}");
        for (body, code) in [
            (json!({}), "invalid_body"),
            (json!({"extra": 1}), "invalid_body"),
            (json!({"status": "done"}), "invalid_status"),
            (json!({"status": null}), "invalid_status"),
            (json!({"status": 3}), "invalid_status"),
            (json!({"note": 3}), "invalid_note"),
            (json!({"note": "x".repeat(2001)}), "invalid_note"),
            (json!({"note": ["a"]}), "invalid_note"),
        ] {
            let (status, json) = call(&fixture.state, "PATCH", &uri, &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], code, "{body}");
        }
        let (status, json) = request(&fixture.state, "PATCH", &uri, Some("[]")).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_body"))
        );
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_return_409_when_removing_seed_item() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let cited = fixture.work("cited").await;
        fixture.cite(seed, cited).await;
        let ramp = fixture.create(seed).await;
        let (status, json) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/ramps/{ramp}/items/{seed}"),
            None,
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("seed_item"))
        );

        let (status, _) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/ramps/{ramp}/items/{cited}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "not an item yet");
        fixture.add(&ramp, cited, Some(seed)).await;
        let (status, _) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/ramps/{ramp}/items/{cited}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(fixture.order(&ramp).await, vec![seed.to_string()]);
        fixture.delete(&ramp).await;
    }

    /// A ramp over seed, A, B with `seed -> A -> B`; returns the Work IDs.
    async fn chain(fixture: &Fixture) -> (String, [WorkId; 3]) {
        let seed = fixture.work("seed").await;
        let first = fixture.work("a").await;
        let second = fixture.work("b").await;
        fixture.cite(seed, first).await;
        fixture.cite(first, second).await;
        let ramp = fixture.create(seed).await;
        assert_eq!(
            fixture.add(&ramp, first, Some(seed)).await.0,
            StatusCode::CREATED
        );
        assert_eq!(
            fixture.add(&ramp, second, Some(first)).await.0,
            StatusCode::CREATED
        );
        (ramp, [seed, first, second])
    }

    #[tokio::test]
    async fn should_reorder_when_work_ids_are_a_permutation() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (ramp, [seed, first, second]) = chain(&fixture).await;
        let wanted = [second, seed, first];
        let (status, detail) = call(
            &fixture.state,
            "PUT",
            &format!("/api/ramps/{ramp}/order"),
            &json!({"work_ids": wanted.iter().map(ToString::to_string).collect::<Vec<_>>()}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        let ids: Vec<String> = detail["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["work"]["id"].as_str().expect("id").to_owned())
            .collect();
        assert_eq!(
            ids,
            wanted.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
        let ordinals: Vec<u64> = detail["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["ordinal"].as_u64().expect("ordinal"))
            .collect();
        assert_eq!(ordinals, vec![1, 2, 3]);
        assert_eq!(fixture.order(&ramp).await, ids, "the order persists");
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_return_400_when_order_is_not_a_permutation() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (ramp, [seed, first, second]) = chain(&fixture).await;
        let before = fixture.order(&ramp).await;
        let uri = format!("/api/ramps/{ramp}/order");
        let (seed, first, second) = (seed.to_string(), first.to_string(), second.to_string());
        let stranger = WorkId::new().to_string();
        for (body, code) in [
            (json!({"work_ids": [seed, first]}), "invalid_order"),
            (
                json!({"work_ids": [seed, first, second, stranger]}),
                "invalid_order",
            ),
            (
                json!({"work_ids": [seed, first, stranger]}),
                "invalid_order",
            ),
            (json!({"work_ids": [seed, first, first]}), "invalid_order"),
            (json!({"work_ids": [seed, first, "nope"]}), "invalid_order"),
            (json!({"work_ids": []}), "invalid_order"),
            (
                json!({"work_ids": [seed, first, second], "strategy": "foundational_first"}),
                "invalid_body",
            ),
            (json!({}), "invalid_body"),
            (json!({"strategy": "alphabetical"}), "invalid_strategy"),
            (json!({"strategy": 3}), "invalid_body"),
            (json!({"work_ids": "w"}), "invalid_body"),
        ] {
            let (status, json) = call(&fixture.state, "PUT", &uri, &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], code, "{body}");
        }
        assert_eq!(
            fixture.order(&ramp).await,
            before,
            "refused orders change nothing"
        );
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_apply_foundational_first_when_strategy_given() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (ramp, [seed, first, second]) = chain(&fixture).await;
        fixture.cite(seed, second).await;
        let (status, detail) = call(
            &fixture.state,
            "PUT",
            &format!("/api/ramps/{ramp}/order"),
            &json!({"strategy": "foundational_first"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        let ids: Vec<String> = detail["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["work"]["id"].as_str().expect("id").to_owned())
            .collect();
        assert_eq!(
            ids,
            vec![second.to_string(), first.to_string(), seed.to_string()]
        );
        assert_eq!(fixture.order(&ramp).await, ids);
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_include_local_papers_acquisitions_and_expansions_when_reading_ramp() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (ramp, [seed, first, _]) = chain(&fixture).await;
        fixture
            .graph
            .acquisitions
            .claim(first)
            .await
            .expect("claim");
        let (status, detail) =
            request(&fixture.state, "GET", &format!("/api/ramps/{ramp}"), None).await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert!(detail["local_papers"].is_object());
        assert!(
            detail["acquisitions"][first.to_string()]["state"].is_string(),
            "{detail}"
        );
        assert_eq!(detail["expansions"][seed.to_string()]["state"], "known");
        assert_eq!(
            detail["expansions"][seed.to_string()]["references_count"],
            1
        );
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_follow_redirects_when_adding_merged_work_and_reading_ramp() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let survivor = fixture.work("survivor").await;
        let merged = fixture.work("merged").await;
        let other = fixture.work("other").await;
        fixture.cite(seed, survivor).await;
        fixture.cite(seed, other).await;
        let ramp = fixture.create(seed).await;
        fixture
            .graph
            .works
            .merge_work(merged, survivor, "duplicate", "test")
            .await
            .expect("merge");

        let (status, item) = fixture.add(&ramp, merged, Some(seed)).await;
        assert_eq!(status, StatusCode::CREATED, "{item}");
        assert_eq!(
            item["work"]["id"],
            survivor.to_string(),
            "responds with the resolved ID"
        );
        let (status, json) = fixture.add(&ramp, merged, Some(seed)).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("item_exists"))
        );

        let (status, patched) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/ramps/{ramp}/items/{merged}"),
            &json!({"status": "read"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{patched}");
        assert_eq!(patched["work"]["id"], survivor.to_string());

        assert_eq!(
            fixture.add(&ramp, other, Some(seed)).await.0,
            StatusCode::CREATED
        );
        let (status, detail) = call(
            &fixture.state,
            "PUT",
            &format!("/api/ramps/{ramp}/order"),
            &json!({"work_ids": [other.to_string(), merged.to_string(), seed.to_string()]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["items"][1]["work"]["id"], survivor.to_string());

        let (status, _) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/ramps/{ramp}/items/{merged}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        fixture.delete(&ramp).await;
    }

    #[tokio::test]
    async fn should_return_404_when_ramp_deleted() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let ramp = fixture.create(seed).await;
        fixture.delete(&ramp).await;
        for (method, uri) in [
            ("GET", format!("/api/ramps/{ramp}")),
            ("DELETE", format!("/api/ramps/{ramp}")),
            ("DELETE", format!("/api/ramps/{ramp}/items/{seed}")),
        ] {
            let (status, json) = request(&fixture.state, method, &uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
            assert_eq!(json["error"], "ramp_not_found");
        }
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &format!("/api/ramps/{ramp}/order"),
            &json!({"strategy": "foundational_first"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("ramp_not_found"))
        );
    }
}
