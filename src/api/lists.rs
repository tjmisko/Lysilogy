//! Reading-list endpoints: list CRUD, nodes, edges, ordering, per-Work reading state,
//! memberships, and search. Generation and history are served elsewhere.
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
use lysilogy_core::{EdgeKind, ListId, NodeId, WorkId};
use lysilogy_db::list::{ArchiveScope, ListPatch};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{AppState, graph_error, graph_or_unavailable};
use crate::{
    domain::PaperId,
    lists::{
        AddNode, BATCH_MAX, ListServiceError, NodeChange, OrderStrategy, parse_description,
        parse_edge_kind, parse_note, parse_status, parse_strategy, parse_summary, parse_title,
        parse_topic_label,
    },
};

type Failure = Box<Response>;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/lists", get(list_lists).post(create_list))
        .route("/api/lists/search", get(search_lists))
        .route(
            "/api/lists/{list}",
            get(read_list).patch(patch_list).delete(delete_list),
        )
        .route("/api/lists/{list}/nodes", post(add_node))
        .route("/api/lists/{list}/nodes:batch", post(add_nodes))
        .route(
            "/api/lists/{list}/nodes/{node}",
            patch(patch_node).delete(remove_node),
        )
        .route(
            "/api/lists/{list}/edges",
            post(add_edge).delete(remove_edge).patch(accept_edge),
        )
        .route("/api/lists/{list}/order", put(order_list))
        .route("/api/lists/{list}/verify", post(verify_list))
        .route("/api/works/search", get(search_works))
        .route("/api/works/{id}/reading-state", put(put_reading_state))
        .route("/api/works/{id}/lists", get(work_lists))
        .route("/api/papers/{id}/lists", get(paper_lists))
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

fn parse_list_id(text: &str) -> Result<ListId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_list_id",
            "List identifiers look like 'p' followed by 22 characters.",
        )
    })
}

fn parse_node_id(text: &str) -> Result<NodeId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_node_id",
            "Node identifiers look like 'n' followed by 22 characters.",
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

fn parse_paper(text: &str) -> Result<PaperId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_paper_id",
            "Paper identifiers are 16 hexadecimal characters.",
        )
    })
}

/// The HTTP form of a service error.
// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn service_failure(error: &ListServiceError) -> Failure {
    let status = match error {
        ListServiceError::NotFound(_) => StatusCode::NOT_FOUND,
        ListServiceError::Invalid(..) => StatusCode::BAD_REQUEST,
        ListServiceError::Conflict(..) => StatusCode::CONFLICT,
        ListServiceError::EdgeCycle(nodes) => {
            return Box::new(
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": error.code(),
                        "message": error.to_string(),
                        "cycle": nodes,
                    })),
                )
                    .into_response(),
            );
        }
        ListServiceError::Provider(detail) => {
            tracing::warn!(%detail, "list search provider failed");
            StatusCode::BAD_GATEWAY
        }
        ListServiceError::Graph(inner) => {
            tracing::error!(error = %inner, "list request failed");
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

/// Keep `null` distinct from an absent field: `null` becomes `Some(Value::Null)`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// A nullable text field: absent leaves it, `null` or blank clears it.
#[allow(clippy::option_option)]
fn text_change(
    value: Option<&Value>,
    code: &'static str,
    parse: fn(Option<&str>) -> Result<Option<String>, ListServiceError>,
) -> Result<Option<Option<String>>, Failure> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(Some(None)),
        Value::String(text) => parse(Some(text))
            .map(Some)
            .map_err(|error| service_failure(&error)),
        _ => Err(failure(
            StatusCode::BAD_REQUEST,
            code,
            "The field is text, or null to clear it.",
        )),
    }
}

#[derive(Deserialize)]
struct ListsQuery {
    work: Option<String>,
    archived: Option<String>,
}

async fn list_lists(State(state): State<AppState>, Query(query): Query<ListsQuery>) -> Response {
    finish(list_lists_inner(&state, query).await)
}

async fn list_lists_inner(state: &AppState, query: ListsQuery) -> Result<Response, Failure> {
    let work = query.work.as_deref().map(parse_work).transpose()?;
    // `true` lists archived lists with the rest; `only` lists just the archived ones.
    let scope = match query.archived.as_deref() {
        None | Some("" | "false") => ArchiveScope::Active,
        Some("true") => ArchiveScope::All,
        Some("only") => ArchiveScope::Archived,
        Some(_) => {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "invalid_archived",
                "archived is one of true, false, only.",
            ));
        }
    };
    graph_or_unavailable(state)?;
    let lists = state
        .list_summaries(work, scope)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "lists": lists }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateListBody {
    title: String,
    description: Option<String>,
    seed_work_id: Option<String>,
}

async fn create_list(State(state): State<AppState>, body: Bytes) -> Response {
    finish(create_list_inner(&state, &body).await)
}

async fn create_list_inner(state: &AppState, body: &Bytes) -> Result<Response, Failure> {
    let body: CreateListBody = parse_object(body)?;
    let title = parse_title(&body.title).map_err(|error| service_failure(&error))?;
    let description =
        parse_description(body.description.as_deref()).map_err(|error| service_failure(&error))?;
    let seed = body.seed_work_id.as_deref().map(parse_work).transpose()?;
    graph_or_unavailable(state)?;
    let detail = state
        .create_list(&title, description, seed)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::CREATED, &detail))
}

async fn read_list(State(state): State<AppState>, AxumPath(list): AxumPath<String>) -> Response {
    finish(read_list_inner(&state, &list).await)
}

async fn read_list_inner(state: &AppState, list: &str) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    graph_or_unavailable(state)?;
    let detail = state
        .list_detail(list)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &detail))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchListBody {
    title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    description: Option<Value>,
    archived: Option<bool>,
}

async fn patch_list(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(patch_list_inner(&state, &list, &body).await)
}

async fn patch_list_inner(state: &AppState, list: &str, body: &Bytes) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let body: PatchListBody = parse_object(body)?;
    if body.title.is_none() && body.description.is_none() && body.archived.is_none() {
        return Err(invalid_body(
            "at least one of title, description, archived is required",
        ));
    }
    let title = body
        .title
        .as_deref()
        .map(parse_title)
        .transpose()
        .map_err(|error| service_failure(&error))?;
    let description = text_change(body.description.as_ref(), "invalid_description", |text| {
        parse_description(text)
    })?;
    graph_or_unavailable(state)?;
    let summary = state
        .update_list(
            list,
            ListPatch {
                title,
                description,
                archived: body.archived,
            },
        )
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &summary))
}

async fn delete_list(State(state): State<AppState>, AxumPath(list): AxumPath<String>) -> Response {
    finish(delete_list_inner(&state, &list).await)
}

async fn delete_list_inner(state: &AppState, list: &str) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    graph_or_unavailable(state)?;
    state
        .delete_list(list)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicBody {
    label: String,
    summary: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimBody {
    title: String,
    #[serde(default)]
    authors: Vec<String>,
    year: Option<i32>,
    venue: Option<String>,
    doi: Option<String>,
    arxiv: Option<String>,
    raw: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeBody {
    work_id: Option<String>,
    via_work_id: Option<String>,
    topic_ids: Option<Vec<String>>,
    paper_id: Option<String>,
    topic: Option<TopicBody>,
    claim: Option<ClaimBody>,
    note: Option<String>,
}

impl NodeBody {
    fn into_add(self) -> Result<AddNode, Failure> {
        let given = usize::from(self.work_id.is_some())
            + usize::from(self.paper_id.is_some())
            + usize::from(self.topic.is_some())
            + usize::from(self.claim.is_some());
        if given != 1 {
            return Err(invalid_body(
                "give exactly one of work_id, paper_id, topic, claim",
            ));
        }
        if self.via_work_id.is_some() && self.work_id.is_none() {
            return Err(invalid_body("via_work_id goes with work_id"));
        }
        if self.topic_ids.is_some() && self.topic.is_some()
            || self.topic_ids.is_some() && self.claim.is_some()
        {
            return Err(invalid_body("topic_ids goes with work_id or paper_id"));
        }
        let note = parse_note(self.note.as_deref()).map_err(|error| service_failure(&error))?;
        let topics = self
            .topic_ids
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|id| parse_node_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(topic) = self.topic {
            if note.is_some() {
                return Err(invalid_body("a topic takes a summary, not a note"));
            }
            return Ok(AddNode::Topic {
                label: parse_topic_label(&topic.label).map_err(|error| service_failure(&error))?,
                summary: parse_summary(topic.summary.as_deref())
                    .map_err(|error| service_failure(&error))?,
            });
        }
        if let Some(claim) = self.claim {
            return Ok(AddNode::Claim {
                claim: lysilogy_core::Claim {
                    title: claim.title,
                    authors: claim.authors,
                    year: claim.year,
                    venue: claim.venue,
                    doi: claim.doi,
                    arxiv: claim.arxiv,
                    raw: claim.raw,
                },
                note,
            });
        }
        if let Some(paper) = self.paper_id {
            return Ok(AddNode::Paper {
                paper: parse_paper(&paper)?,
                topics,
                note,
            });
        }
        let work = self.work_id.as_deref().map(parse_work).transpose()?;
        let via = self.via_work_id.as_deref().map(parse_work).transpose()?;
        let work = work.ok_or_else(|| invalid_body("work_id is required"))?;
        Ok(AddNode::Work {
            work,
            via,
            topics,
            note,
        })
    }
}

async fn add_node(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(add_node_inner(&state, &list, &body).await)
}

async fn add_node_inner(state: &AppState, list: &str, body: &Bytes) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let add = parse_object::<NodeBody>(body)?.into_add()?;
    graph_or_unavailable(state)?;
    let node = state
        .add_list_node(list, add)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::CREATED, &node))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchBody {
    nodes: Vec<NodeBody>,
}

async fn add_nodes(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(add_nodes_inner(&state, &list, &body).await)
}

async fn add_nodes_inner(state: &AppState, list: &str, body: &Bytes) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let body: BatchBody = parse_object(body)?;
    if body.nodes.is_empty() || body.nodes.len() > BATCH_MAX {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "invalid_batch",
            "A batch adds between 1 and 100 nodes.",
        ));
    }
    let adds = body
        .nodes
        .into_iter()
        .map(NodeBody::into_add)
        .collect::<Result<Vec<_>, _>>()?;
    graph_or_unavailable(state)?;
    let nodes = state
        .add_list_nodes(list, adds)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::CREATED,
        &serde_json::json!({ "nodes": nodes }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchNodeBody {
    #[serde(default, deserialize_with = "present")]
    note: Option<Value>,
    label: Option<String>,
    #[serde(default, deserialize_with = "present")]
    summary: Option<Value>,
    review: Option<String>,
    resolve_to: Option<String>,
    resolve_to_candidate: Option<String>,
}

async fn patch_node(
    State(state): State<AppState>,
    AxumPath((list, node)): AxumPath<(String, String)>,
    body: Bytes,
) -> Response {
    finish(patch_node_inner(&state, &list, &node, &body).await)
}

async fn patch_node_inner(
    state: &AppState,
    list: &str,
    node: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let node = parse_node_id(node)?;
    let body: PatchNodeBody = parse_object(body)?;
    if body.note.is_none()
        && body.label.is_none()
        && body.summary.is_none()
        && body.review.is_none()
        && body.resolve_to.is_none()
        && body.resolve_to_candidate.is_none()
    {
        return Err(invalid_body(
            "at least one of note, label, summary, review, resolve_to, resolve_to_candidate is required",
        ));
    }
    if body
        .resolve_to_candidate
        .as_deref()
        .is_some_and(|provider_id| provider_id.trim().is_empty())
    {
        return Err(invalid_body("resolve_to_candidate must not be empty"));
    }
    let accept = match body.review.as_deref() {
        None => false,
        Some("accepted") => true,
        Some(_) => {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "invalid_review",
                "review can only be set to accepted.",
            ));
        }
    };
    let change = NodeChange {
        note: text_change(body.note.as_ref(), "invalid_note", parse_note)?,
        label: body
            .label
            .as_deref()
            .map(parse_topic_label)
            .transpose()
            .map_err(|error| service_failure(&error))?,
        summary: text_change(body.summary.as_ref(), "invalid_summary", parse_summary)?,
        accept,
        resolve_to: body.resolve_to.as_deref().map(parse_work).transpose()?,
        resolve_to_candidate: body.resolve_to_candidate,
    };
    graph_or_unavailable(state)?;
    let view = state
        .update_list_node(list, node, change)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &view))
}

async fn remove_node(
    State(state): State<AppState>,
    AxumPath((list, node)): AxumPath<(String, String)>,
) -> Response {
    finish(remove_node_inner(&state, &list, &node).await)
}

/// Answers 200 with the deleted node and its edges, so the client can undo.
async fn remove_node_inner(state: &AppState, list: &str, node: &str) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let node = parse_node_id(node)?;
    graph_or_unavailable(state)?;
    let removed = state
        .remove_list_node(list, node)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &removed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeBody {
    from: String,
    to: String,
    kind: String,
}

impl EdgeBody {
    fn parse(&self) -> Result<(NodeId, NodeId, EdgeKind), Failure> {
        Ok((
            parse_node_id(&self.from)?,
            parse_node_id(&self.to)?,
            parse_edge_kind(&self.kind).map_err(|error| service_failure(&error))?,
        ))
    }
}

async fn add_edge(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(add_edge_inner(&state, &list, &body).await)
}

async fn add_edge_inner(state: &AppState, list: &str, body: &Bytes) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let (from, to, kind) = parse_object::<EdgeBody>(body)?.parse()?;
    graph_or_unavailable(state)?;
    let edge = state
        .add_list_edge(list, from, to, kind)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::CREATED, &edge))
}

async fn remove_edge(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(remove_edge_inner(&state, &list, &body).await)
}

async fn remove_edge_inner(
    state: &AppState,
    list: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let (from, to, kind) = parse_object::<EdgeBody>(body)?.parse()?;
    graph_or_unavailable(state)?;
    state
        .remove_list_edge(list, from, to, kind)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptEdgeBody {
    from: String,
    to: String,
    kind: String,
    review: String,
}

async fn accept_edge(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(accept_edge_inner(&state, &list, &body).await)
}

async fn accept_edge_inner(
    state: &AppState,
    list: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let body: AcceptEdgeBody = parse_object(body)?;
    if body.review != "accepted" {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "invalid_review",
            "review can only be set to accepted.",
        ));
    }
    let (from, to, kind) = EdgeBody {
        from: body.from,
        to: body.to,
        kind: body.kind,
    }
    .parse()?;
    graph_or_unavailable(state)?;
    state
        .accept_list_edge(list, from, to, kind)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderBody {
    node_ids: Option<Vec<String>>,
    strategy: Option<String>,
}

async fn order_list(
    State(state): State<AppState>,
    AxumPath(list): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(order_list_inner(&state, &list, &body).await)
}

async fn order_list_inner(state: &AppState, list: &str, body: &Bytes) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    let body: OrderBody = parse_object(body)?;
    let invalid_order = || {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_order",
            "The order must list every item exactly once.",
        )
    };
    let manual: Result<Vec<NodeId>, OrderStrategy> = match (&body.node_ids, &body.strategy) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(invalid_body("give exactly one of node_ids or strategy"));
        }
        (None, Some(strategy)) => {
            Err(parse_strategy(strategy).map_err(|error| service_failure(&error))?)
        }
        (Some(ids), None) => Ok(ids
            .iter()
            .map(|id| id.parse::<NodeId>().map_err(|_| invalid_order()))
            .collect::<Result<_, _>>()?),
    };
    graph_or_unavailable(state)?;
    let detail = match manual {
        Ok(order) => state.reorder_list(list, &order).await,
        Err(strategy) => state.order_list_by(list, strategy).await,
    }
    .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &detail))
}

async fn verify_list(State(state): State<AppState>, AxumPath(list): AxumPath<String>) -> Response {
    finish(verify_list_inner(&state, &list).await)
}

async fn verify_list_inner(state: &AppState, list: &str) -> Result<Response, Failure> {
    let list = parse_list_id(list)?;
    graph_or_unavailable(state)?;
    let report = state
        .verify_list(list)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &report))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadingStateBody {
    status: String,
}

async fn put_reading_state(
    State(state): State<AppState>,
    AxumPath(work): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(put_reading_state_inner(&state, &work, &body).await)
}

async fn put_reading_state_inner(
    state: &AppState,
    work: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let work = parse_work(work)?;
    let body: ReadingStateBody = parse_object(body)?;
    let status = parse_status(&body.status).map_err(|error| service_failure(&error))?;
    graph_or_unavailable(state)?;
    let stored = state
        .set_work_reading_state(work, status)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(StatusCode::OK, &stored))
}

async fn work_lists(State(state): State<AppState>, AxumPath(work): AxumPath<String>) -> Response {
    finish(work_lists_inner(&state, &work).await)
}

async fn work_lists_inner(state: &AppState, work: &str) -> Result<Response, Failure> {
    let work = parse_work(work)?;
    graph_or_unavailable(state)?;
    let (work, memberships) = state
        .work_memberships(work)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "work_id": work, "memberships": memberships }),
    ))
}

async fn paper_lists(State(state): State<AppState>, AxumPath(paper): AxumPath<String>) -> Response {
    finish(paper_lists_inner(&state, &paper).await)
}

async fn paper_lists_inner(state: &AppState, paper: &str) -> Result<Response, Failure> {
    let paper = parse_paper(paper)?;
    graph_or_unavailable(state)?;
    let (work, memberships) = state
        .paper_memberships(&paper)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "paper_id": paper, "work_id": work, "memberships": memberships }),
    ))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    provider: Option<String>,
}

async fn search_lists(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Response {
    finish(search_lists_inner(&state, &query).await)
}

async fn search_lists_inner(state: &AppState, query: &SearchQuery) -> Result<Response, Failure> {
    graph_or_unavailable(state)?;
    let lists = state
        .search_lists(query.q.as_deref().unwrap_or_default())
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "lists": lists }),
    ))
}

async fn search_works(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Response {
    finish(search_works_inner(&state, &query).await)
}

async fn search_works_inner(state: &AppState, query: &SearchQuery) -> Result<Response, Failure> {
    let provider = match query.provider.as_deref() {
        None | Some("" | "false") => false,
        Some("true") => true,
        Some(_) => {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "invalid_provider",
                "provider is true or false.",
            ));
        }
    };
    graph_or_unavailable(state)?;
    let results = state
        .search_works(query.q.as_deref().unwrap_or_default(), provider)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(
        StatusCode::OK,
        &serde_json::json!({ "results": results }),
    ))
}

#[cfg(test)]
#[allow(clippy::too_many_lines)]
mod tests {
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{
        ClaimState, DocumentFormat, IdentifierScheme, ListReview, ScannedFile, WorkLinkMethod,
    };
    use lysilogy_db::list::{NewEdge, NewNode};
    use lysilogy_ingest::openalex::ProviderError;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::build_router,
        expansion::fixtures::{
            ScriptedTransport, batch_reply, marker, new_work, plain_state, test_graph,
        },
        graph::GraphState,
        literature::LiteratureClient,
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

    /// Answers title searches for records whose title carries a needle, batch lookups with
    /// generic titles, and Crossref with an empty record; fails everything with 503 while
    /// `down` is set.
    fn provider(found: Vec<(String, Value)>, down: &Arc<AtomicBool>) -> ScriptedTransport {
        let down = Arc::clone(down);
        ScriptedTransport::new(move |url| {
            if down.load(Ordering::SeqCst) {
                return Err(ProviderError::Status(503));
            }
            if url.contains("title.search:") {
                for (needle, record) in &found {
                    if url.contains(needle.as_str()) {
                        return Ok(json!({"results": [record]}));
                    }
                }
                return Ok(json!({"results": []}));
            }
            if url.contains("filter=openalex:") {
                return Ok(batch_reply(url));
            }
            if url.contains("api.crossref.org/works/") {
                return Ok(json!({"message": {}}));
            }
            Err(ProviderError::Status(404))
        })
    }

    fn record(openalex_id: &str, title: &str) -> Value {
        json!({
            "id": format!("https://openalex.org/{openalex_id}"),
            "display_name": title,
            "publication_year": 2021,
            "authorships": [{"author": {"display_name": "Ada Lovelace"}}],
            "referenced_works_count": 3
        })
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

        async fn with_transport(transport: ScriptedTransport) -> Option<Self> {
            let graph = test_graph().await?;
            let state = plain_state(graph.clone(), transport).await;
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

        async fn create(&self, label: &str) -> String {
            let (status, json) = call(
                &self.state,
                "POST",
                "/api/lists",
                &json!({"title": format!("{label} {}", self.marker)}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{json}");
            json["list"]["id"].as_str().expect("list id").to_owned()
        }

        async fn add_work(&self, list: &str, work: WorkId) -> (StatusCode, Value) {
            call(
                &self.state,
                "POST",
                &format!("/api/lists/{list}/nodes"),
                &json!({"work_id": work.to_string()}),
            )
            .await
        }

        /// The node ID of a Work added to the list.
        async fn node_of(&self, list: &str, work: WorkId) -> String {
            let (status, json) = self.add_work(list, work).await;
            assert_eq!(status, StatusCode::CREATED, "{json}");
            json["id"].as_str().expect("node id").to_owned()
        }

        async fn topic(&self, list: &str, label: &str) -> String {
            let (status, json) = call(
                &self.state,
                "POST",
                &format!("/api/lists/{list}/nodes"),
                &json!({"topic": {"label": label}}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{json}");
            json["id"].as_str().expect("node id").to_owned()
        }

        async fn detail(&self, list: &str) -> Value {
            let (status, json) =
                request(&self.state, "GET", &format!("/api/lists/{list}"), None).await;
            assert_eq!(status, StatusCode::OK, "{json}");
            json
        }

        async fn order(&self, list: &str) -> Vec<String> {
            self.detail(list).await["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .map(|node| node["id"].as_str().expect("id").to_owned())
                .collect()
        }

        fn list_id(text: &str) -> ListId {
            text.parse().expect("list id")
        }

        fn node_id(text: &str) -> NodeId {
            text.parse().expect("node id")
        }
    }

    #[tokio::test]
    async fn should_return_503_when_lists_requested_without_graph() {
        let state = bare_state().await;
        let list = ListId::new();
        let node = NodeId::new();
        let work = WorkId::new();
        let paper = "0123456789abcdef";
        for (method, uri, body) in [
            ("POST", "/api/lists".to_owned(), json!({"title": "x"})),
            ("PATCH", format!("/api/lists/{list}"), json!({"title": "x"})),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"work_id": work.to_string()}),
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes:batch"),
                json!({"nodes": [{"work_id": work.to_string()}]}),
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/nodes/{node}"),
                json!({"note": "x"}),
            ),
            (
                "POST",
                format!("/api/lists/{list}/edges"),
                json!({"from": node.to_string(), "to": node.to_string(), "kind": "prerequisite"}),
            ),
            (
                "DELETE",
                format!("/api/lists/{list}/edges"),
                json!({"from": node.to_string(), "to": node.to_string(), "kind": "prerequisite"}),
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/edges"),
                json!({"from": node.to_string(), "to": node.to_string(), "kind": "prerequisite", "review": "accepted"}),
            ),
            (
                "PUT",
                format!("/api/lists/{list}/order"),
                json!({"strategy": "topological"}),
            ),
            (
                "PUT",
                format!("/api/works/{work}/reading-state"),
                json!({"status": "read"}),
            ),
            ("POST", format!("/api/lists/{list}/verify"), json!({})),
        ] {
            let (status, json) = call(&state, method, &uri, &body).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
            assert_eq!(json["error"], "graph_unavailable", "{method} {uri}");
        }
        for (method, uri) in [
            ("GET", "/api/lists".to_owned()),
            ("GET", "/api/lists/search?q=x".to_owned()),
            ("GET", format!("/api/lists/{list}")),
            ("DELETE", format!("/api/lists/{list}")),
            ("DELETE", format!("/api/lists/{list}/nodes/{node}")),
            ("GET", "/api/works/search?q=x".to_owned()),
            ("GET", format!("/api/works/{work}/lists")),
            ("GET", format!("/api/papers/{paper}/lists")),
        ] {
            let (status, json) = request(&state, method, &uri, None).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
            assert_eq!(json["error"], "graph_unavailable", "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn should_return_404_unknown_api_when_ramp_routes_requested() {
        let state = bare_state().await;
        let ramp = ListId::new();
        for (method, uri) in [
            ("GET", "/api/ramps".to_owned()),
            ("POST", "/api/ramps".to_owned()),
            ("GET", format!("/api/ramps/{ramp}")),
            ("DELETE", format!("/api/ramps/{ramp}/items/w1")),
            ("PUT", format!("/api/ramps/{ramp}/order")),
        ] {
            let (status, json) = request(&state, method, &uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
            assert_eq!(json["error"], "unknown_api_route", "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn should_return_400_when_ids_are_malformed() {
        let state = bare_state().await;
        let work = WorkId::new();
        let list = ListId::new();
        let node = NodeId::new();
        for (method, uri, body, code) in [
            ("GET", format!("/api/lists/{work}"), None, "invalid_list_id"),
            (
                "DELETE",
                "/api/lists/nonsense".to_owned(),
                None,
                "invalid_list_id",
            ),
            (
                "GET",
                format!("/api/lists?work={list}"),
                None,
                "invalid_work_id",
            ),
            (
                "POST",
                "/api/lists".to_owned(),
                Some(json!({"title": "x", "seed_work_id": list.to_string()})),
                "invalid_work_id",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                Some(json!({"work_id": "nope"})),
                "invalid_work_id",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                Some(json!({"work_id": work.to_string(), "via_work_id": list.to_string()})),
                "invalid_work_id",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                Some(json!({"work_id": work.to_string(), "topic_ids": [work.to_string()]})),
                "invalid_node_id",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                Some(json!({"paper_id": "short"})),
                "invalid_paper_id",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/nodes/{work}"),
                Some(json!({"note": "x"})),
                "invalid_node_id",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/nodes/{node}"),
                Some(json!({"resolve_to": node.to_string()})),
                "invalid_work_id",
            ),
            (
                "DELETE",
                format!("/api/lists/{list}/nodes/bad"),
                None,
                "invalid_node_id",
            ),
            (
                "POST",
                format!("/api/lists/{list}/edges"),
                Some(json!({"from": work.to_string(), "to": node.to_string(), "kind": "covers"})),
                "invalid_node_id",
            ),
            (
                "POST",
                format!("/api/lists/{list}/edges"),
                Some(json!({"from": node.to_string(), "to": node.to_string(), "kind": "cites"})),
                "invalid_edge_kind",
            ),
            (
                "PUT",
                format!("/api/lists/{work}/order"),
                Some(json!({"strategy": "topological"})),
                "invalid_list_id",
            ),
            (
                "PUT",
                format!("/api/lists/{list}/order"),
                Some(json!({"node_ids": [work.to_string()]})),
                "invalid_order",
            ),
            (
                "PUT",
                format!("/api/lists/{list}/order"),
                Some(json!({"strategy": "alphabetical"})),
                "invalid_strategy",
            ),
            (
                "PUT",
                format!("/api/works/{list}/reading-state"),
                Some(json!({"status": "read"})),
                "invalid_work_id",
            ),
            (
                "PUT",
                format!("/api/works/{work}/reading-state"),
                Some(json!({"status": "done"})),
                "invalid_status",
            ),
            (
                "GET",
                format!("/api/works/{list}/lists"),
                None,
                "invalid_work_id",
            ),
            (
                "GET",
                format!("/api/papers/{work}/lists"),
                None,
                "invalid_paper_id",
            ),
            (
                "POST",
                format!("/api/lists/{work}/verify"),
                None,
                "invalid_list_id",
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
    async fn should_return_400_when_body_is_not_an_object_or_is_inconsistent() {
        let state = bare_state().await;
        let list = ListId::new();
        let work = WorkId::new().to_string();
        for body in ["[1]", "null", "\"x\"", "{", ""] {
            let (status, json) = request(&state, "POST", "/api/lists", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error"], "invalid_body", "{body}");
        }
        for (method, uri, body, code) in [
            (
                "POST",
                "/api/lists".to_owned(),
                json!({"title": "x", "color": "red"}),
                "invalid_body",
            ),
            (
                "POST",
                "/api/lists".to_owned(),
                json!({"title": "   "}),
                "invalid_list_title",
            ),
            (
                "POST",
                "/api/lists".to_owned(),
                json!({"title": "x".repeat(121)}),
                "invalid_list_title",
            ),
            (
                "POST",
                "/api/lists".to_owned(),
                json!({"title": "x", "description": "d".repeat(4001)}),
                "invalid_description",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}"),
                json!({}),
                "invalid_body",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}"),
                json!({"description": 3}),
                "invalid_description",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({}),
                "invalid_body",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"work_id": work, "paper_id": "0123456789abcdef"}),
                "invalid_body",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"topic": {"label": "T"}, "via_work_id": work}),
                "invalid_body",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"topic": {"label": "T"}, "note": "n"}),
                "invalid_body",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"topic": {"label": " "}}),
                "invalid_topic_label",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"topic": {"label": "T", "summary": "s".repeat(2001)}}),
                "invalid_summary",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"work_id": work, "note": "n".repeat(2001)}),
                "invalid_note",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes"),
                json!({"claim": {"title": "t", "bogus": 1}}),
                "invalid_body",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes:batch"),
                json!({"nodes": []}),
                "invalid_batch",
            ),
            (
                "POST",
                format!("/api/lists/{list}/nodes:batch"),
                json!({"nodes": vec![json!({"work_id": work}); 101]}),
                "invalid_batch",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/nodes/{}", NodeId::new()),
                json!({}),
                "invalid_body",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/nodes/{}", NodeId::new()),
                json!({"review": "proposed"}),
                "invalid_review",
            ),
            (
                "PATCH",
                format!("/api/lists/{list}/edges"),
                json!({"from": NodeId::new().to_string(), "to": NodeId::new().to_string(),
                       "kind": "covers", "review": "rejected"}),
                "invalid_review",
            ),
            (
                "PUT",
                format!("/api/lists/{list}/order"),
                json!({}),
                "invalid_body",
            ),
            (
                "PUT",
                format!("/api/lists/{list}/order"),
                json!({"node_ids": [], "strategy": "year"}),
                "invalid_body",
            ),
            (
                "PUT",
                format!("/api/works/{work}/reading-state"),
                json!({}),
                "invalid_body",
            ),
        ] {
            let (status, json) = call(&state, method, &uri, &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}: {json}");
            assert_eq!(json["error"], code, "{method} {uri} {body}");
        }
        let (status, json) = request(&state, "GET", "/api/lists?archived=sometimes", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_archived");
        let (status, json) =
            request(&state, "GET", "/api/works/search?q=x&provider=maybe", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_provider");
    }

    #[tokio::test]
    async fn should_create_walk_list_with_the_seed_as_first_node_when_seeded() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            "/api/lists",
            &json!({"title": "  Seeded   list  ", "description": " about it ",
                    "seed_work_id": seed.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["list"]["title"], "Seeded list");
        assert_eq!(json["list"]["description"], "about it");
        assert_eq!(json["list"]["origin"], "walk");
        assert_eq!(json["list"]["seed_work_id"], seed.to_string());
        assert_eq!(json["list"]["item_count"], 1);
        assert_eq!(json["nodes"][0]["work_id"], seed.to_string());
        assert_eq!(json["nodes"][0]["ordinal"], 1);
        assert_eq!(json["reading_states"][seed.to_string()]["status"], "unread");
        assert!(json["citations"].as_array().expect("citations").is_empty());
    }

    #[tokio::test]
    async fn should_return_404_when_seed_work_is_unknown() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (status, json) = call(
            &fixture.state,
            "POST",
            "/api/lists",
            &json!({"title": "x", "seed_work_id": WorkId::new().to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "work_not_found");
    }

    #[tokio::test]
    async fn should_filter_summaries_when_work_and_archive_filters_are_given() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("member").await;
        let holder = fixture.create("Holder").await;
        let other = fixture.create("Other").await;
        fixture.node_of(&holder, work).await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/lists?work={work}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let lists = json["lists"].as_array().expect("lists");
        assert_eq!(lists.len(), 1, "{json}");
        assert_eq!(lists[0]["id"], holder);
        assert_eq!(lists[0]["item_count"], 1);
        assert_eq!(lists[0]["status_counts"]["unread"], 1);
        assert_eq!(lists[0]["proposed_count"], 0);
        assert_eq!(lists[0]["claim_count"], 0);

        let (status, _) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{other}"),
            &json!({"archived": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let ids = |json: &Value| -> Vec<String> {
            json["lists"]
                .as_array()
                .expect("lists")
                .iter()
                .map(|list| list["id"].as_str().expect("id").to_owned())
                .collect()
        };
        for (query, expect_other) in [
            ("", false),
            ("?archived=false", false),
            ("?archived=true", true),
        ] {
            let (_, json) =
                request(&fixture.state, "GET", &format!("/api/lists{query}"), None).await;
            assert_eq!(ids(&json).contains(&other), expect_other, "{query}");
            assert_eq!(ids(&json).contains(&holder), query != "?archived=only");
        }
        let (_, json) = request(&fixture.state, "GET", "/api/lists?archived=only", None).await;
        assert!(ids(&json).contains(&other));
        assert!(!ids(&json).contains(&holder));
    }

    #[tokio::test]
    async fn should_update_list_fields_when_title_description_and_archive_are_patched() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Before").await;
        let uri = format!("/api/lists/{list}");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &uri,
            &json!({"title": "After", "description": "Why"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["title"], "After");
        assert_eq!(json["description"], "Why");
        assert!(json["archived_at"].is_null());
        let (_, json) = call(&fixture.state, "PATCH", &uri, &json!({"archived": true})).await;
        assert!(json["archived_at"].is_string(), "{json}");
        assert_eq!(json["title"], "After");
        let (_, json) = call(
            &fixture.state,
            "PATCH",
            &uri,
            &json!({"description": null, "archived": false}),
        )
        .await;
        assert!(json["description"].is_null(), "{json}");
        assert!(json["archived_at"].is_null(), "{json}");
        let (status, json) = call(&fixture.state, "PATCH", &uri, &json!({"title": ""})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_list_title");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{}", ListId::new()),
            &json!({"title": "x"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "list_not_found");
    }

    #[tokio::test]
    async fn should_report_list_not_found_when_reading_a_deleted_list() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Doomed").await;
        let work = fixture.work("w").await;
        fixture.node_of(&list, work).await;
        let uri = format!("/api/lists/{list}");
        let (status, _) = request(&fixture.state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        for method in ["GET", "DELETE"] {
            let (status, json) = request(&fixture.state, method, &uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
            assert_eq!(json["error"], "list_not_found");
        }
        let (status, json) = fixture.add_work(&list, work).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "list_not_found");
    }

    #[tokio::test]
    async fn should_report_errors_when_adding_duplicate_unknown_or_bad_via_work_nodes() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Adds").await;
        let (first, second, third) = (
            fixture.work("a").await,
            fixture.work("b").await,
            fixture.work("c").await,
        );
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": first.to_string(), "note": " why "}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["kind"], "work");
        assert_eq!(json["note"], "why");
        assert_eq!(json["review"], "accepted");
        assert_eq!(json["source"], "user");
        assert_eq!(json["work"]["id"], first.to_string());
        assert_eq!(json["ordinal"], 1);
        let (status, json) = fixture.add_work(&list, first).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], "item_exists");
        let (status, json) = fixture.add_work(&list, WorkId::new()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "work_not_found");
        // `via` must be a member, and it must cite the Work.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": second.to_string(), "via_work_id": third.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "via_not_in_list");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": second.to_string(), "via_work_id": first.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "no_citation_edge");
        fixture.cite(first, second).await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": second.to_string(), "via_work_id": first.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["via_work_id"], first.to_string());
        assert_eq!(json["depth"], 1);
        assert_eq!(json["path"].as_array().expect("path").len(), 2);
        assert_eq!(
            json["path_steps"][0]["title"],
            format!("Title a {}", fixture.marker)
        );
        assert_eq!(
            fixture.detail(&list).await["citations"][0][0],
            first.to_string()
        );
    }

    #[tokio::test]
    async fn should_follow_redirect_when_adding_a_merged_work() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (keep, gone) = (fixture.work("keep").await, fixture.work("gone").await);
        fixture
            .graph
            .works
            .merge_work(gone, keep, "test", "test")
            .await
            .expect("merge");
        let list = fixture.create("Redirects").await;
        let (status, json) = fixture.add_work(&list, gone).await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["work_id"], keep.to_string());
        let (status, json) = fixture.add_work(&list, keep).await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "item_exists");
    }

    #[tokio::test]
    async fn should_report_list_full_when_five_hundred_items_exist() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Full").await;
        for index in 0..500 {
            fixture
                .graph
                .lists
                .add_node(
                    Fixture::list_id(&list),
                    NewNode::claim(
                        json!({"title": format!("Claim {index}")}),
                        ClaimState::Unverified,
                    ),
                )
                .await
                .expect("claim");
        }
        let work = fixture.work("overflow").await;
        let (status, json) = fixture.add_work(&list, work).await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "list_full");
        // Topics have their own limit.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"topic": {"label": "Still fine"}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
    }

    #[tokio::test]
    async fn should_reject_duplicate_topic_when_labels_differ_only_by_case() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Topics").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"topic": {"label": "  Reward   modelling ", "summary": " about rewards "}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["kind"], "topic");
        assert_eq!(json["label"], "Reward modelling");
        assert_eq!(json["summary"], "about rewards");
        for label in ["reward MODELLING", "Reward modelling"] {
            let (status, json) = call(
                &fixture.state,
                "POST",
                &format!("/api/lists/{list}/nodes"),
                &json!({"topic": {"label": label}}),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{label}");
            assert_eq!(json["error"], "item_exists");
        }
        // Unicode labels are legitimate.
        let (status, _) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"topic": {"label": "Übersetzung 翻訳"}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn should_file_work_under_topics_when_topic_ids_given() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Filing").await;
        let topic = fixture.topic(&list, "Alignment").await;
        let work = fixture.work("filed").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": work.to_string(), "topic_ids": [topic]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        let detail = fixture.detail(&list).await;
        let edges = detail["edges"].as_array().expect("edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["kind"], "covers");
        assert_eq!(edges[0]["from"], topic);
        // An unknown topic refuses the whole add, and a Work is not a topic.
        let other = fixture.work("other").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": other.to_string(), "topic_ids": [NodeId::new().to_string()]}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "node_not_found");
        let member = json!(detail["nodes"][1]["id"]);
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": other.to_string(), "topic_ids": [member]}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "invalid_edge_kind");
        assert_eq!(fixture.order(&list).await.len(), 2, "nothing was added");
    }

    /// A state whose library holds one placeholder PDF.
    async fn paper_state(graph: GraphState) -> (AppState, String, String) {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let file_name = format!("Autor - 2013 - The Study of Things {}.pdf", marker());
        std::fs::write(library.path().join(&file_name), b"placeholder").expect("write");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state")
            .with_graph(graph)
            .with_literature_client(LiteratureClient::new(ScriptedTransport::new(|_| {
                Err(ProviderError::Status(404))
            })));
        std::mem::forget((library, data));
        let paper = PaperId::from_relative_path(Path::new(&file_name)).to_string();
        (state, paper, file_name)
    }

    async fn link_document(graph: &GraphState, file_name: &str, work: WorkId) {
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
            .set_work_link(document.id, work, WorkLinkMethod::Manual, None, "test")
            .await
            .expect("link");
    }

    #[tokio::test]
    async fn should_refuse_paper_node_when_the_paper_is_unlinked_or_unknown() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, paper, file_name) = paper_state(fixture.graph.clone()).await;
        let list = fixture.create("Papers").await;
        let uri = format!("/api/lists/{list}/nodes");
        // Unlinked: the lit flow cannot link a placeholder, so the add is refused.
        let (status, json) = call(&state, "POST", &uri, &json!({"paper_id": paper})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "paper_not_linked");
        assert!(fixture.order(&list).await.is_empty());
        // Unknown paper.
        let (status, json) = call(
            &state,
            "POST",
            &uri,
            &json!({"paper_id": "0123456789abcdef"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "paper_not_found");
        // Linked: the Work is added, and the paper sees the list.
        let work = fixture.work("held").await;
        link_document(&fixture.graph, &file_name, work).await;
        let (status, json) = call(&state, "POST", &uri, &json!({"paper_id": paper})).await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["work_id"], work.to_string());
        let (status, json) = call(&state, "POST", &uri, &json!({"paper_id": paper})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "item_exists");
        let (status, json) =
            request(&state, "GET", &format!("/api/papers/{paper}/lists"), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["work_id"], work.to_string());
        assert_eq!(json["memberships"][0]["list_id"], list);
        let detail = fixture_detail(&state, &list).await;
        assert_eq!(
            detail["local_papers"][work.to_string()][0]["paper_id"],
            paper
        );
    }

    async fn fixture_detail(state: &AppState, list: &str) -> Value {
        let (status, json) = request(state, "GET", &format!("/api/lists/{list}"), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        json
    }

    #[tokio::test]
    async fn should_report_empty_memberships_when_paper_has_no_work() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, paper, _) = paper_state(fixture.graph.clone()).await;
        let (status, json) =
            request(&state, "GET", &format!("/api/papers/{paper}/lists"), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert!(json["work_id"].is_null());
        assert!(
            json["memberships"]
                .as_array()
                .expect("memberships")
                .is_empty()
        );
        let (status, json) =
            request(&state, "GET", "/api/papers/0123456789abcdef/lists", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "paper_not_found");
    }

    #[tokio::test]
    async fn should_add_claim_as_work_when_it_verifies_and_as_claim_node_otherwise() {
        let down = Arc::new(AtomicBool::new(false));
        let marker_word = marker();
        let title = format!("Verified claims join lists {marker_word}");
        let transport = provider(
            vec![(
                marker_word.clone(),
                record(&format!("W9{marker_word}"), &title),
            )],
            &down,
        );
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Claims").await;
        let uri = format!("/api/lists/{list}/nodes");
        // A claim the provider knows becomes a Work node.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"claim": {"title": title, "year": 2021, "authors": ["Ada Lovelace"]},
                    "note": "from memory"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["kind"], "work", "{json}");
        assert_eq!(json["note"], "from memory");
        // The same claim again matches that Work, which is already a member.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"claim": {"title": title, "year": 2021, "authors": ["Ada Lovelace"]}}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "item_exists");
        // A claim nothing supports stays a claim, with the typed text kept.
        let invented = format!("An invented paper about nothing qq{}", fixture.marker);
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"claim": {"title": invented, "raw": "see 10.9999/invented.1"}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["kind"], "claim", "{json}");
        assert!(json["work_id"].is_null());
        assert_eq!(json["claim"]["title"], invented);
        assert_eq!(json["claim"]["doi"], "10.9999/invented.1");
        assert!(
            ["unverified", "contradicted", "pending"]
                .contains(&json["claim_state"].as_str().expect("state")),
            "{json}"
        );
        // While the provider is down a claim is pending, never unverified.
        down.store(true, Ordering::SeqCst);
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"claim": {"title": format!("Offline claim zz{}", fixture.marker)}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["claim_state"], "pending", "{json}");
        let detail = fixture.detail(&list).await;
        assert_eq!(detail["list"]["claim_count"], 2);
        assert_eq!(detail["list"]["item_count"], 3);
    }

    #[tokio::test]
    async fn should_return_400_when_claim_title_is_blank() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Blank claims").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"claim": {"title": "   "}}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_claim");
    }

    #[tokio::test]
    async fn should_add_nothing_when_any_batch_item_is_refused() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Batch").await;
        let (first, second, third) = (
            fixture.work("a").await,
            fixture.work("b").await,
            fixture.work("c").await,
        );
        let batch = format!("/api/lists/{list}/nodes:batch");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &batch,
            &json!({"nodes": [
                {"work_id": first.to_string()},
                {"topic": {"label": "Together"}},
                {"work_id": second.to_string(), "note": "n"}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        let nodes = json["nodes"].as_array().expect("nodes");
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0]["work_id"], first.to_string());
        assert_eq!(nodes[1]["label"], "Together");
        assert_eq!(nodes[2]["note"], "n");
        // The duplicate in the middle refuses the batch and the new Work before it is undone.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &batch,
            &json!({"nodes": [
                {"work_id": third.to_string()},
                {"work_id": first.to_string()}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "item_exists");
        assert_eq!(fixture.order(&list).await.len(), 3, "nothing was added");
        // The same Work twice inside one batch is also refused.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &batch,
            &json!({"nodes": [
                {"work_id": third.to_string()},
                {"work_id": third.to_string()}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(fixture.order(&list).await.len(), 3);
        // An unknown Work refuses the batch before any write.
        let (status, json) = call(
            &fixture.state,
            "POST",
            &batch,
            &json!({"nodes": [
                {"work_id": third.to_string()},
                {"work_id": WorkId::new().to_string()}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(fixture.order(&list).await.len(), 3);
    }

    #[tokio::test]
    async fn should_update_node_fields_when_notes_topic_review_or_claim_are_patched() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Patches").await;
        let list_id = Fixture::list_id(&list);
        let work = fixture.work("noted").await;
        let node = fixture.node_of(&list, work).await;
        let uri = format!("/api/lists/{list}/nodes/{node}");
        let (status, json) = call(&fixture.state, "PATCH", &uri, &json!({"note": "  keep "})).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["note"], "keep");
        let (_, json) = call(&fixture.state, "PATCH", &uri, &json!({"note": null})).await;
        assert!(json["note"].is_null(), "{json}");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &uri,
            &json!({"note": "n".repeat(2001)}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_note");
        let (status, json) = call(&fixture.state, "PATCH", &uri, &json!({"note": 5})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_note");
        let (status, json) = call(&fixture.state, "PATCH", &uri, &json!({"label": "Nope"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_node_kind");

        let topic = fixture.topic(&list, "First").await;
        let second = fixture.topic(&list, "Second").await;
        let topic_uri = format!("/api/lists/{list}/nodes/{topic}");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &topic_uri,
            &json!({"label": "Renamed", "summary": "text"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["label"], "Renamed");
        assert_eq!(json["summary"], "text");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{list}/nodes/{second}"),
            &json!({"label": "renamed"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "item_exists");
        let (_, json) = call(
            &fixture.state,
            "PATCH",
            &topic_uri,
            &json!({"summary": null}),
        )
        .await;
        assert!(json["summary"].is_null(), "{json}");
        let (status, json) =
            call(&fixture.state, "PATCH", &topic_uri, &json!({"label": " "})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_topic_label");

        // A proposed node is accepted.
        let mut proposed =
            NewNode::claim(json!({"title": "Proposed claim"}), ClaimState::Unverified);
        proposed.review = ListReview::Proposed;
        let proposed = fixture
            .graph
            .lists
            .add_node(list_id, proposed)
            .await
            .expect("proposed");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{list}/nodes/{}", proposed.id),
            &json!({"review": "accepted"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["review"], "accepted");

        // A claim resolves to a Work once; a Work node is not a claim; a member Work is taken.
        let target = fixture.work("target").await;
        let claim_uri = format!("/api/lists/{list}/nodes/{}", proposed.id);
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &claim_uri,
            &json!({"resolve_to": WorkId::new().to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "work_not_found");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &claim_uri,
            &json!({"resolve_to": work.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "item_exists");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &claim_uri,
            &json!({"resolve_to": target.to_string(), "note": "resolved"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["kind"], "work");
        assert_eq!(json["work_id"], target.to_string());
        assert_eq!(json["note"], "resolved");
        assert!(json["claim"].is_null());
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &claim_uri,
            &json!({"resolve_to": fixture.work("again").await.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "not_a_claim");

        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{list}/nodes/{}", NodeId::new()),
            &json!({"note": "x"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "node_not_found");
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{}/nodes/{node}", ListId::new()),
            &json!({"note": "x"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "list_not_found");
    }

    #[tokio::test]
    async fn should_return_removed_node_and_edges_when_deleting_a_node() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            "/api/lists",
            &json!({"title": format!("Removal {}", fixture.marker), "seed_work_id": seed.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let list = json["list"]["id"].as_str().expect("id").to_owned();
        let seed_node = json["nodes"][0]["id"].as_str().expect("node").to_owned();
        let topic = fixture.topic(&list, "Basics").await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/edges"),
            &json!({"from": topic, "to": seed_node, "kind": "covers"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        let (status, json) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/lists/{list}/nodes/{seed_node}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["node"]["id"], seed_node);
        assert_eq!(json["node"]["work_id"], seed.to_string());
        assert_eq!(json["edges"].as_array().expect("edges").len(), 1);
        assert_eq!(json["edges"][0]["kind"], "covers");
        assert_eq!(json["was_seed"], true);
        let detail = fixture.detail(&list).await;
        assert!(detail["list"]["seed_work_id"].is_null(), "{detail}");
        assert!(detail["edges"].as_array().expect("edges").is_empty());
        // Undo is a re-add: the same Work is accepted again.
        let (status, _) = fixture.add_work(&list, seed).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, json) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/lists/{list}/nodes/{seed_node}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "node_not_found");
        let (status, json) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/lists/{}/nodes/{seed_node}", ListId::new()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "list_not_found");
    }

    #[tokio::test]
    async fn should_report_edge_errors_when_edges_cycle_misfit_or_miss_nodes() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Edges").await;
        let (first, second) = (fixture.work("a").await, fixture.work("b").await);
        let (a, b) = (
            fixture.node_of(&list, first).await,
            fixture.node_of(&list, second).await,
        );
        let topic = fixture.topic(&list, "T").await;
        let uri = format!("/api/lists/{list}/edges");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": a, "to": b, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["kind"], "prerequisite");
        assert_eq!(json["source"], "user");
        assert_eq!(json["review"], "accepted");
        // Storing it again is a no-op.
        let (status, _) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": a, "to": b, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            fixture.detail(&list).await["edges"]
                .as_array()
                .expect("edges")
                .len(),
            1
        );
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": b, "to": a, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "edge_cycle");
        let cycle = json["cycle"].as_array().expect("cycle");
        assert!(
            cycle.contains(&json!(a)) && cycle.contains(&json!(b)),
            "{json}"
        );
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": a, "to": a, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "edge_cycle");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": a, "to": b, "kind": "covers"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "invalid_edge_kind");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": topic, "to": NodeId::new().to_string(), "kind": "covers"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "node_not_found");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{}/edges", ListId::new()),
            &json!({"from": a, "to": b, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "list_not_found");

        let (status, _) = call(
            &fixture.state,
            "DELETE",
            &uri,
            &json!({"from": a, "to": b, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, json) = call(
            &fixture.state,
            "DELETE",
            &uri,
            &json!({"from": a, "to": b, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "edge_not_found");
        // With the edge gone the reverse direction is allowed.
        let (status, _) = call(
            &fixture.state,
            "POST",
            &uri,
            &json!({"from": b, "to": a, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn should_accept_proposed_edge_when_it_exists_and_report_missing_otherwise() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Proposals").await;
        let (first, second) = (fixture.work("a").await, fixture.work("b").await);
        let (a, b) = (
            fixture.node_of(&list, first).await,
            fixture.node_of(&list, second).await,
        );
        fixture
            .graph
            .lists
            .add_edge(
                Fixture::list_id(&list),
                NewEdge {
                    from: Fixture::node_id(&a),
                    to: Fixture::node_id(&b),
                    kind: EdgeKind::Prerequisite,
                    source: lysilogy_core::EdgeSource::Agent,
                    review: ListReview::Proposed,
                },
            )
            .await
            .expect("add")
            .expect("accepted by the graph");
        let uri = format!("/api/lists/{list}/edges");
        let body = json!({"from": a, "to": b, "kind": "prerequisite", "review": "accepted"});
        let (status, json) = call(&fixture.state, "PATCH", &uri, &body).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{json}");
        assert_eq!(
            fixture.detail(&list).await["edges"][0]["review"],
            "accepted"
        );
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &uri,
            &json!({"from": b, "to": a, "kind": "prerequisite", "review": "accepted"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "edge_not_found");
    }

    #[tokio::test]
    async fn should_order_list_when_given_a_permutation_or_a_strategy() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Ordering").await;
        let (a, b, c) = (
            fixture.work("a").await,
            fixture.work("b").await,
            fixture.work("c").await,
        );
        // c cites b cites a, and the list starts in the opposite order.
        fixture.cite(c, b).await;
        fixture.cite(b, a).await;
        let node_c = fixture.node_of(&list, c).await;
        let node_b = fixture.node_of(&list, b).await;
        let node_a = fixture.node_of(&list, a).await;
        let topic = fixture.topic(&list, "Basics").await;
        let uri = format!("/api/lists/{list}/order");
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &uri,
            &json!({"strategy": "foundational_first"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let ids: Vec<String> = json["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .map(|node| node["id"].as_str().expect("id").to_owned())
            .collect();
        assert_eq!(
            ids,
            vec![
                node_a.clone(),
                node_b.clone(),
                node_c.clone(),
                topic.clone()
            ],
            "cited first; the empty topic last"
        );
        assert_eq!(json["nodes"][0]["ordinal"], 1);

        // Filing b and c under the topic puts the topic just before its first Work.
        for node in [&node_b, &node_c] {
            let (status, _) = call(
                &fixture.state,
                "POST",
                &format!("/api/lists/{list}/edges"),
                &json!({"from": topic, "to": node, "kind": "covers"}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }
        // A prerequisite beats the citations: c before a.
        let (status, _) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/edges"),
            &json!({"from": node_c, "to": node_a, "kind": "prerequisite"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, _) = call(
            &fixture.state,
            "PUT",
            &uri,
            &json!({"strategy": "topological"}),
        )
        .await;
        let order = fixture.order(&list).await;
        let position = |id: &String| order.iter().position(|item| item == id).expect("present");
        assert!(position(&node_c) < position(&node_a), "{order:?}");
        assert!(
            position(&topic) < position(&node_b).min(position(&node_c)),
            "{order:?}"
        );

        // Years sort older first.
        fixture
            .graph
            .works
            .set_display_year(a, 2020)
            .await
            .expect("year");
        fixture
            .graph
            .works
            .set_display_year(b, 2010)
            .await
            .expect("year");
        fixture
            .graph
            .works
            .set_display_year(c, 2015)
            .await
            .expect("year");
        let (status, _) = call(&fixture.state, "PUT", &uri, &json!({"strategy": "year"})).await;
        assert_eq!(status, StatusCode::OK);
        let order = fixture.order(&list).await;
        let position = |id: &String| order.iter().position(|item| item == id).expect("present");
        assert!(position(&node_b) < position(&node_c) && position(&node_c) < position(&node_a));

        // A manual permutation is taken as given.
        let reversed: Vec<String> = order.iter().rev().cloned().collect();
        let (status, json) =
            call(&fixture.state, "PUT", &uri, &json!({"node_ids": reversed})).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(fixture.order(&list).await, reversed);
        for bad in [
            json!({"node_ids": [reversed[0]]}),
            json!({"node_ids": []}),
            json!({"node_ids": [reversed[0], reversed[0], reversed[1], reversed[2]]}),
            json!({"node_ids": [reversed[0], reversed[1], reversed[2], NodeId::new().to_string()]}),
            json!({"node_ids": ["junk"]}),
        ] {
            let (status, json) = call(&fixture.state, "PUT", &uri, &bad).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {json}");
            assert_eq!(json["error"], "invalid_order", "{bad}");
        }
        assert_eq!(
            fixture.order(&list).await,
            reversed,
            "refusals changed nothing"
        );
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &format!("/api/lists/{}/order", ListId::new()),
            &json!({"strategy": "year"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "list_not_found");
    }

    #[tokio::test]
    async fn should_keep_seed_last_when_ordering_a_walk_list_by_strategy() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seed = fixture.work("seed").await;
        let reference = fixture.work("reference").await;
        fixture.cite(seed, reference).await;
        let (_, json) = call(
            &fixture.state,
            "POST",
            "/api/lists",
            &json!({"title": format!("Walk {}", fixture.marker), "seed_work_id": seed.to_string()}),
        )
        .await;
        let list = json["list"]["id"].as_str().expect("id").to_owned();
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"work_id": reference.to_string(), "via_work_id": seed.to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &format!("/api/lists/{list}/order"),
            &json!({"strategy": "foundational_first"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["nodes"][0]["work_id"], reference.to_string());
        assert_eq!(json["nodes"][1]["work_id"], seed.to_string());
    }

    #[tokio::test]
    async fn should_show_one_reading_state_in_every_list_when_a_work_is_set() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("reading").await;
        let (first, second) = (fixture.create("One").await, fixture.create("Two").await);
        fixture.node_of(&first, work).await;
        fixture.node_of(&second, work).await;
        let uri = format!("/api/works/{work}/reading-state");
        let (status, json) = call(&fixture.state, "PUT", &uri, &json!({"status": "reading"})).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["status"], "reading");
        assert_eq!(json["work_id"], work.to_string());
        for list in [&first, &second] {
            let detail = fixture.detail(list).await;
            assert_eq!(
                detail["reading_states"][work.to_string()]["status"],
                "reading"
            );
            assert_eq!(detail["list"]["status_counts"]["reading"], 1);
        }
        let (_, json) = call(&fixture.state, "PUT", &uri, &json!({"status": "read"})).await;
        assert_eq!(json["status"], "read");
        let (status, json) = call(&fixture.state, "PUT", &uri, &json!({"status": "Read"})).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "status names are lowercase"
        );
        assert_eq!(json["error"], "invalid_status");
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &format!("/api/works/{}/reading-state", WorkId::new()),
            &json!({"status": "read"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "work_not_found");
    }

    #[tokio::test]
    async fn should_omit_archived_lists_when_listing_memberships_of_a_work() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("member").await;
        let (first, second) = (
            fixture.create("Active").await,
            fixture.create("Archived").await,
        );
        let topic = fixture.topic(&first, "Basics").await;
        let (status, _) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{first}/nodes"),
            &json!({"work_id": work.to_string(), "topic_ids": [topic], "note": "start here"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        fixture.node_of(&second, work).await;
        call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{second}"),
            &json!({"archived": true}),
        )
        .await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/lists"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["work_id"], work.to_string());
        let memberships = json["memberships"].as_array().expect("memberships");
        assert_eq!(memberships.len(), 1, "{json}");
        assert_eq!(memberships[0]["list_id"], first);
        assert_eq!(memberships[0]["topics"][0], "Basics");
        assert_eq!(memberships[0]["note"], "start here");
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{}/lists", WorkId::new()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "work_not_found");
    }

    #[tokio::test]
    async fn should_find_lists_when_searching_by_title_words() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let wanted = fixture.create("Reward modelling").await;
        let hidden = fixture.create("Reward archive").await;
        call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{hidden}"),
            &json!({"archived": true}),
        )
        .await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/lists/search?q=reward%20{}", fixture.marker),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let lists = json["lists"].as_array().expect("lists");
        assert_eq!(lists.len(), 1, "{json}");
        assert_eq!(lists[0]["id"], wanted);
        let (status, json) =
            request(&fixture.state, "GET", "/api/lists/search?q=%25_%5C", None).await;
        assert_eq!(status, StatusCode::OK, "metacharacters match literally");
        assert!(json["lists"].as_array().expect("lists").is_empty());
        let (status, json) = request(&fixture.state, "GET", "/api/lists/search", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["lists"].is_array());
    }

    #[tokio::test]
    async fn should_include_provider_hits_when_searching_works_with_provider_set() {
        let down = Arc::new(AtomicBool::new(false));
        let needle = marker();
        let title = format!("Provider only paper {needle}");
        let known = format!("W8{needle}");
        let transport = provider(
            vec![(needle.clone(), record(&format!("W9{needle}"), &title))],
            &down,
        );
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&fixture.graph, IdentifierScheme::Openalex, &known).await;
        let graph_title = format!("Graph resident paper {}", fixture.marker);
        fixture
            .graph
            .works
            .set_display_title(work, &graph_title)
            .await
            .expect("title");
        fixture
            .graph
            .works
            .backfill_title_keys()
            .await
            .expect("keys");
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/search?q={}", graph_title.replace(' ', "%20")),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let results = json["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "{json}");
        assert_eq!(results[0]["work_id"], work.to_string());
        assert_eq!(results[0]["source"], "graph");
        assert_eq!(results[0]["in_graph"], true);
        // Without provider=true no provider request is made.
        let (_, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/search?q=provider%20only%20paper%20{needle}"),
            None,
        )
        .await;
        assert!(
            json["results"].as_array().expect("results").is_empty(),
            "{json}"
        );
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/search?q=provider%20only%20paper%20{needle}&provider=true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let results = json["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "{json}");
        assert_eq!(results[0]["source"], "provider");
        assert_eq!(results[0]["in_graph"], false);
        assert!(results[0]["work_id"].is_null());
        assert_eq!(results[0]["openalex_id"], format!("W9{needle}"));
        assert_eq!(results[0]["year"], 2021);
        // A blank query is refused, and a provider outage is a 502, not an empty answer.
        let (status, json) =
            request(&fixture.state, "GET", "/api/works/search?q=%20%20", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_query");
        let (status, _) = request(&fixture.state, "GET", "/api/works/search", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        down.store(true, Ordering::SeqCst);
        let (status, json) = request(
            &fixture.state,
            "GET",
            "/api/works/search?q=something%20else%20entirely&provider=true",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{json}");
        assert_eq!(json["error"], "provider_error");
    }

    #[tokio::test]
    async fn should_match_held_papers_when_searching_by_title_words() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, paper, file_name) = paper_state(fixture.graph.clone()).await;
        let word = file_name
            .rsplit(' ')
            .next()
            .expect("marker")
            .trim_end_matches(".pdf")
            .to_owned();
        let (status, json) =
            request(&state, "GET", &format!("/api/works/search?q={word}"), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let results = json["results"].as_array().expect("results");
        assert_eq!(results.len(), 1, "{json}");
        assert_eq!(results[0]["source"], "library");
        assert_eq!(results[0]["paper_id"], paper);
        assert_eq!(results[0]["in_graph"], false);
        let linked = fixture.work("linked-paper").await;
        link_document(&fixture.graph, &file_name, linked).await;
        let (_, json) = request(&state, "GET", &format!("/api/works/search?q={word}"), None).await;
        assert_eq!(json["results"][0]["work_id"], linked.to_string());
        assert_eq!(json["results"][0]["in_graph"], true);
    }

    #[tokio::test]
    async fn should_reclassify_pending_claim_when_the_list_is_full() {
        let down = Arc::new(AtomicBool::new(false));
        let Some(fixture) = Fixture::with_transport(provider(Vec::new(), &down)).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Full verify").await;
        let list_id = Fixture::list_id(&list);
        let pending = fixture
            .graph
            .lists
            .add_node(
                list_id,
                NewNode::claim(
                    json!({"title": format!("Nothing like this exists ww{}", fixture.marker)}),
                    ClaimState::Pending,
                ),
            )
            .await
            .expect("claim");
        sqlx::query(
            "insert into reader.list_node (list_id, kind, claim, claim_state, position, source) \
             select $1, 'claim', '{}'::jsonb, 'unverified', g, 'user' \
             from generate_series(1, 499) g",
        )
        .bind(list_id.as_uuid())
        .execute(&test_pool().await)
        .await
        .expect("bulk claims");
        let uri = format!("/api/lists/{list}/verify");
        let (status, json) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["reclassified"], 1, "{json}");
        let nodes = fixture.graph.lists.nodes(list_id).await.expect("nodes");
        assert_eq!(nodes.len(), 500);
        assert_eq!(
            nodes[0]
                .claim_state
                .map(|state| state != ClaimState::Pending),
            Some(true)
        );
        assert!(nodes.iter().all(|node| node.id != pending.id));
    }

    #[tokio::test]
    async fn should_retry_pending_claims_when_verifying_a_list() {
        let down = Arc::new(AtomicBool::new(false));
        let needle = marker();
        let title = format!("Pending claims get verified {needle}");
        let transport = provider(
            vec![(needle.clone(), record(&format!("W7{needle}"), &title))],
            &down,
        );
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Verify").await;
        let list_id = Fixture::list_id(&list);
        let claim = json!({"title": title, "year": 2021, "authors": ["Ada Lovelace"]});
        let pending = fixture
            .graph
            .lists
            .add_node(list_id, NewNode::claim(claim, ClaimState::Pending))
            .await
            .expect("claim");
        let unknown = fixture
            .graph
            .lists
            .add_node(
                list_id,
                NewNode::claim(
                    json!({"title": format!("Nothing like this exists yy{}", fixture.marker)}),
                    ClaimState::Pending,
                ),
            )
            .await
            .expect("claim");
        let after = fixture.work("after").await;
        fixture.node_of(&list, after).await;
        let uri = format!("/api/lists/{list}/verify");
        // Still down: both stay pending.
        down.store(true, Ordering::SeqCst);
        let (status, json) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["checked"], 2);
        assert_eq!(json["still_pending"], 2);
        assert_eq!(json["resolved"], 0);
        // Back up: one resolves, the other is reclassified in place.
        down.store(false, Ordering::SeqCst);
        let before = fixture.order(&list).await;
        let (status, json) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["checked"], 2);
        assert_eq!(json["resolved"], 1, "{json}");
        assert_eq!(json["reclassified"], 1, "{json}");
        let detail = fixture.detail(&list).await;
        let nodes = detail["nodes"].as_array().expect("nodes");
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0]["id"], pending.id.to_string());
        assert_eq!(nodes[0]["kind"], "work");
        assert_eq!(nodes[1]["kind"], "claim");
        assert_ne!(nodes[1]["claim_state"], "pending", "{detail}");
        assert_ne!(
            nodes[1]["id"],
            unknown.id.to_string(),
            "the claim node was replaced"
        );
        assert_eq!(nodes[2]["work_id"], after.to_string());
        assert_eq!(before.len(), nodes.len());
        // Nothing is left to retry.
        let (_, json) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(json["checked"], 0);
        let (status, json) = request(
            &fixture.state,
            "POST",
            &format!("/api/lists/{}/verify", ListId::new()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "list_not_found");
    }

    #[tokio::test]
    async fn should_show_detail_with_defaults_when_the_list_has_citations_and_expansions() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Detail").await;
        let (citing, cited, loner) = (
            fixture.work("citing").await,
            fixture.work("cited").await,
            fixture.work("loner").await,
        );
        fixture.cite(citing, cited).await;
        for work in [citing, cited, loner] {
            fixture.node_of(&list, work).await;
        }
        let detail = fixture.detail(&list).await;
        assert_eq!(detail["citations"].as_array().expect("citations").len(), 1);
        assert_eq!(
            detail["citations"][0],
            json!([citing.to_string(), cited.to_string()])
        );
        assert_eq!(detail["list"]["item_count"], 3);
        assert_eq!(detail["list"]["status_counts"]["unread"], 3);
        assert!(detail["reading_states"][loner.to_string()]["changed_at"].is_null());
        assert_eq!(
            detail["expansions"][citing.to_string()]["references_count"],
            1
        );
        assert!(detail["local_papers"].is_object());
        assert!(detail["acquisitions"].is_object());
        for (index, node) in detail["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .enumerate()
        {
            assert_eq!(node["ordinal"], index + 1);
        }
    }

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL").expect("test database url");
        sqlx::PgPool::connect(&url).await.expect("pool")
    }

    #[tokio::test]
    async fn should_report_topics_full_when_list_holds_two_hundred_topics() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Many topics").await;
        sqlx::query(
            "insert into reader.list_node (list_id, kind, label, position, source) \
             select $1, 'topic', 'topic ' || g, g, 'user' from generate_series(0, 199) g",
        )
        .bind(Fixture::list_id(&list).as_uuid())
        .execute(&test_pool().await)
        .await
        .expect("topics");
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"topic": {"label": "One too many"}}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "topics_full");
    }

    #[tokio::test]
    async fn should_report_edges_full_when_list_holds_four_thousand_edges() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Many edges").await;
        let list_id = Fixture::list_id(&list);
        for index in 0..100 {
            fixture
                .graph
                .lists
                .add_node(
                    list_id,
                    NewNode::claim(
                        json!({"title": format!("Claim {index}")}),
                        ClaimState::Unverified,
                    ),
                )
                .await
                .expect("claim");
        }
        sqlx::query(
            "insert into reader.list_edge (list_id, from_node, to_node, kind, source) \
             select $1, a.id, b.id, 'prerequisite', 'user' \
             from reader.list_node a, reader.list_node b \
             where a.list_id = $1 and b.list_id = $1 and a.id <> b.id limit 4000",
        )
        .bind(list_id.as_uuid())
        .execute(&test_pool().await)
        .await
        .expect("edges");
        let topic = fixture.topic(&list, "Topic").await;
        let work = fixture.work("edge-limit").await;
        let node = fixture.node_of(&list, work).await;
        let (status, json) = call(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/edges"),
            &json!({"from": topic, "to": node, "kind": "covers"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "edges_full");
    }

    #[tokio::test]
    async fn should_keep_claim_proposed_when_a_proposed_pending_claim_verifies() {
        let down = Arc::new(AtomicBool::new(false));
        let needle = marker();
        let title = format!("Proposed claims stay proposed {needle}");
        let transport = provider(
            vec![(needle.clone(), record(&format!("W8{needle}"), &title))],
            &down,
        );
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Proposed verify").await;
        let list_id = Fixture::list_id(&list);
        let mut proposed = NewNode::claim(
            json!({"title": title, "year": 2021, "authors": ["Ada Lovelace"]}),
            ClaimState::Pending,
        );
        proposed.review = ListReview::Proposed;
        let pending = fixture
            .graph
            .lists
            .add_node(list_id, proposed)
            .await
            .expect("claim");
        let after = fixture.work("after-proposed").await;
        fixture.node_of(&list, after).await;
        let (status, json) = request(
            &fixture.state,
            "POST",
            &format!("/api/lists/{list}/verify"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["resolved"], 1, "{json}");
        assert_eq!(json["replaced"][0]["from"], pending.id.to_string());
        let detail = fixture.detail(&list).await;
        let nodes = detail["nodes"].as_array().expect("nodes");
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0]["kind"], "work", "{detail}");
        assert_eq!(nodes[0]["review"], "proposed", "{detail}");
        assert_eq!(nodes[0]["id"], json["replaced"][0]["to"]);
        assert_eq!(nodes[1]["work_id"], after.to_string(), "order is kept");
    }

    #[tokio::test]
    async fn should_remove_claim_and_count_duplicate_when_pending_claim_verifies_to_a_member() {
        let down = Arc::new(AtomicBool::new(false));
        let needle = marker();
        let title = format!("Duplicate claims collapse {needle}");
        let transport = provider(
            vec![(needle.clone(), record(&format!("W9{needle}"), &title))],
            &down,
        );
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Duplicate verify").await;
        let list_id = Fixture::list_id(&list);
        let claim = json!({"title": title, "year": 2021, "authors": ["Ada Lovelace"]});
        for _ in 0..2 {
            fixture
                .graph
                .lists
                .add_node(list_id, NewNode::claim(claim.clone(), ClaimState::Pending))
                .await
                .expect("claim");
        }
        let uri = format!("/api/lists/{list}/verify");
        let (status, json) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["checked"], 2);
        assert_eq!(json["resolved"], 1, "{json}");
        assert_eq!(json["duplicates"], 1, "{json}");
        assert_eq!(json["still_pending"], 0, "{json}");
        assert_eq!(fixture.order(&list).await.len(), 1);
        let (_, json) = request(&fixture.state, "POST", &uri, None).await;
        assert_eq!(json["checked"], 0, "nothing is left to retry");
    }

    #[tokio::test]
    async fn should_leave_claim_untouched_when_resolve_to_comes_with_a_label() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Resolve and label").await;
        let claim = fixture
            .graph
            .lists
            .add_node(
                Fixture::list_id(&list),
                NewNode::claim(json!({"title": "Some claim"}), ClaimState::Unverified),
            )
            .await
            .expect("claim");
        let work = fixture.work("resolve-label").await;
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{list}/nodes/{}", claim.id),
            &json!({"resolve_to": work.to_string(), "label": "Nope"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_node_kind");
        let detail = fixture.detail(&list).await;
        assert_eq!(detail["nodes"][0]["kind"], "claim", "{detail}");
    }

    /// A claim node holding one provider candidate, with a note and rationale to keep.
    async fn candidate_claim(
        fixture: &Fixture,
        list: ListId,
        candidate: &Value,
    ) -> lysilogy_db::list::ListNode {
        let mut claim = NewNode::claim(
            json!({"title": candidate["title"], "year": 2021, "authors": ["Ada Lovelace"]}),
            ClaimState::Ambiguous,
        );
        claim.content = lysilogy_db::list::NewNodeContent::Claim {
            claim: json!({"title": candidate["title"], "year": 2021, "authors": ["Ada Lovelace"]}),
            state: ClaimState::Ambiguous,
            candidates: json!([candidate]),
        };
        claim.note = Some("why this matters".to_owned());
        claim.rationale = Some("the model's reason".to_owned());
        fixture
            .graph
            .lists
            .add_node(list, claim)
            .await
            .expect("claim")
    }

    #[tokio::test]
    async fn should_resolve_claim_in_place_when_given_a_stored_candidate_not_yet_in_the_graph() {
        let down = Arc::new(AtomicBool::new(false));
        let needle = marker();
        let openalex_id = format!("W7{needle}");
        let title = format!("Candidate only {needle}");
        let transport = provider(vec![(needle.clone(), record(&openalex_id, &title))], &down);
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = fixture.create("Resolve candidate").await;
        let list_id = Fixture::list_id(&list);
        let candidate = json!({
            "provider_id": openalex_id, "title": title, "year": 2021,
            "authors": ["Ada Lovelace"], "doi": null
        });
        let claim = candidate_claim(&fixture, list_id, &candidate).await;
        let node_uri = format!("/api/lists/{list}/nodes/{}", claim.id);

        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &node_uri,
            &json!({"resolve_to_candidate": "W-not-stored"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "candidate_not_found");

        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &node_uri,
            &json!({"resolve_to_candidate": openalex_id, "resolve_to": WorkId::new().to_string()}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_body");

        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &node_uri,
            &json!({"resolve_to_candidate": "  "}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
        assert_eq!(json["error"], "invalid_body");

        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &node_uri,
            &json!({"resolve_to_candidate": openalex_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["id"], claim.id.to_string(), "resolved in place");
        assert_eq!(json["kind"], "work");
        assert_eq!(json["note"], "why this matters");
        assert_eq!(json["rationale"], "the model's reason");
        let work = fixture
            .graph
            .works
            .find_by_identifier(IdentifierScheme::Openalex, &openalex_id)
            .await
            .expect("lookup")
            .expect("the candidate became a Work");
        assert_eq!(json["work_id"], work.id.to_string());

        // A second claim naming the same candidate reuses that Work instead of duplicating it.
        let other = fixture.create("Resolve candidate again").await;
        let again = candidate_claim(&fixture, Fixture::list_id(&other), &candidate).await;
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &format!("/api/lists/{other}/nodes/{}", again.id),
            &json!({"resolve_to_candidate": openalex_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["work_id"], work.id.to_string());

        // Resolving the now-Work node again is a conflict, not a second Work.
        let (status, json) = call(
            &fixture.state,
            "PATCH",
            &node_uri,
            &json!({"resolve_to_candidate": openalex_id}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "not_a_claim");
    }

    #[tokio::test]
    async fn should_keep_provider_hits_when_local_hits_fill_the_search_limit() {
        let needle = marker();
        let down = Arc::new(AtomicBool::new(false));
        let transport = provider(
            vec![(
                needle.clone(),
                record(&format!("W6{needle}"), &format!("Provider only {needle}")),
            )],
            &down,
        );
        let Some(fixture) = Fixture::with_transport(transport).await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/search?q=Provider%20only%20{needle}&provider=true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert!(
            json.to_string().contains("\"provider\""),
            "provider hit present: {json}"
        );
    }

    #[tokio::test]
    async fn should_link_held_paper_through_the_literature_flow_when_adding_it() {
        let needle = marker();
        let title = format!("The Study of Things {needle}");
        let mut found = record(&format!("W5{needle}"), &title);
        found["publication_year"] = json!(2013);
        found["authorships"] = json!([{"author": {"display_name": "Anna Autor"}}]);
        let work_url = format!("works/W5{needle}");
        let transport = ScriptedTransport::new(move |url| {
            if url.contains(work_url.as_str()) {
                return Ok(found.clone());
            }
            if url.contains("title.search:") {
                return Ok(json!({"results": [found.clone()]}));
            }
            if url.contains("filter=openalex:") {
                return Ok(batch_reply(url));
            }
            if url.contains("api.crossref.org/works/") {
                return Ok(json!({"message": {}}));
            }
            Err(ProviderError::Status(404))
        });
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let file_name = format!("Autor - 2013 - {title}.pdf");
        std::fs::write(
            library.path().join(&file_name),
            crate::source_index::test_support::native_pdf(&title),
        )
        .expect("write");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state")
            .with_graph(fixture.graph.clone())
            .with_literature_client(LiteratureClient::new(transport));
        std::mem::forget((library, data));
        let paper = PaperId::from_relative_path(Path::new(&file_name)).to_string();
        let list = fixture.create("Link first").await;
        let (status, json) = call(
            &state,
            "POST",
            &format!("/api/lists/{list}/nodes"),
            &json!({"paper_id": paper}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["kind"], "work", "{json}");
        let (status, json) =
            request(&state, "GET", &format!("/api/papers/{paper}/work"), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert!(json["work"]["id"].is_string(), "the paper is now linked");
    }
}
