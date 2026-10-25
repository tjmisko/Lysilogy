//! Generation endpoints: start an agent run that builds or extends a list, and read its
//! progress. Retrying a list's pending claims is `POST /api/lists/{list}/verify`, served
//! with the other list routes.
//!
//! Check order everywhere: path IDs, body, graph availability, existence.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use lysilogy_core::{GenerationId, ListId, WorkId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppState, graph_error, graph_or_unavailable};
use crate::reading_list_planner::{
    DEFAULT_BUDGET_USD, GenerateRequest, GenerationError, Level, SIZE_DEFAULT, SIZE_MAX, SIZE_MIN,
};

type Failure = Box<Response>;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/lists/generate", post(start_generation))
        .route("/api/generations/{id}", get(read_generation))
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

/// The HTTP form of a service error.
// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn service_failure(error: &GenerationError) -> Failure {
    let status = match error {
        GenerationError::NotFound(_) => StatusCode::NOT_FOUND,
        GenerationError::Invalid(..) => StatusCode::BAD_REQUEST,
        GenerationError::Conflict(_, _, existing) => {
            return Box::new(
                (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": error.code(),
                        "message": error.to_string(),
                        "generation_id": existing,
                    })),
                )
                    .into_response(),
            );
        }
        GenerationError::Graph(inner) => {
            tracing::error!(error = %inner, "generation request failed");
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "graph_error",
                "The graph query failed.",
            );
        }
    };
    failure(status, error.code(), &error.to_string())
}

fn finish(result: Result<Response, Failure>) -> Response {
    match result {
        Ok(response) => response,
        Err(response) => *response,
    }
}

/// Body of `POST /api/lists/generate`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateBody {
    goal: String,
    size: Option<i64>,
    level: Option<String>,
    anchors: Option<Vec<String>>,
    into_list: Option<String>,
    budget_usd: Option<f64>,
}

impl GenerateBody {
    fn request(self) -> Result<GenerateRequest, Failure> {
        let size = match self.size {
            None => SIZE_DEFAULT,
            Some(size) => usize::try_from(size)
                .ok()
                .filter(|size| (SIZE_MIN..=SIZE_MAX).contains(size))
                .ok_or_else(|| {
                    failure(
                        StatusCode::BAD_REQUEST,
                        "invalid_size",
                        &format!("Size is between {SIZE_MIN} and {SIZE_MAX}."),
                    )
                })?,
        };
        let level = match self.level.as_deref() {
            None => Level::Survey,
            Some(text) => text.parse().map_err(|()| {
                failure(
                    StatusCode::BAD_REQUEST,
                    "invalid_level",
                    "Level is one of intro, survey, deep.",
                )
            })?,
        };
        let anchors = self
            .anchors
            .unwrap_or_default()
            .iter()
            .map(|text| {
                text.parse::<WorkId>().map_err(|_| {
                    failure(
                        StatusCode::BAD_REQUEST,
                        "invalid_work_id",
                        "Work identifiers look like 'w' followed by 22 characters.",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let into_list = match self.into_list {
            None => None,
            Some(text) => Some(text.parse::<ListId>().map_err(|_| {
                failure(
                    StatusCode::BAD_REQUEST,
                    "invalid_list_id",
                    "List identifiers look like 'p' followed by 22 characters.",
                )
            })?),
        };
        Ok(GenerateRequest {
            goal: self.goal,
            size,
            level,
            anchors,
            into_list,
            budget_usd: self.budget_usd.unwrap_or(DEFAULT_BUDGET_USD),
        })
    }
}

async fn start_generation(State(state): State<AppState>, body: Bytes) -> Response {
    finish(start_generation_inner(&state, &body).await)
}

async fn start_generation_inner(state: &AppState, body: &Bytes) -> Result<Response, Failure> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_body(&error.to_string()))?;
    if !value.is_object() {
        return Err(invalid_body("expected a JSON object"));
    }
    let body: GenerateBody =
        serde_json::from_value(value).map_err(|error| invalid_body(&error.to_string()))?;
    let request = body.request()?;
    request
        .validate()
        .map_err(|error| service_failure(&error))?;
    graph_or_unavailable(state)?;
    let id = state
        .start_generation(request)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "generation_id": id }))).into_response())
}

async fn read_generation(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    finish(read_generation_inner(&state, &id).await)
}

async fn read_generation_inner(state: &AppState, id: &str) -> Result<Response, Failure> {
    let id: GenerationId = id.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_generation_id",
            "Generation identifiers look like 'g' followed by 22 characters.",
        )
    })?;
    graph_or_unavailable(state)?;
    let view = state
        .get_generation(id)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok((StatusCode::OK, Json(view)).into_response())
}
