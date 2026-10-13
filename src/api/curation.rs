//! Private curation endpoints: tags, tag assignments, and marks on Works.
//!
//! Everything lives in the default workspace. Check order: path ids, body,
//! graph availability, existence.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use lysilogy_core::{
    TagColor, TagId, TagKind, TagName, WorkId, WorkMark, WorkspaceId, parse_mark_note,
};
use lysilogy_db::{
    DbError,
    curation::{StoredMark, Tag, TagWithCount},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    AppState, fetch_work, graph_error, graph_or_unavailable, graph_query_failure, parse_work_id,
};
use crate::graph::GraphState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/tags", get(list_tags).post(create_tag))
        .route(
            "/api/tags/{tag}",
            axum::routing::patch(update_tag).delete(delete_tag),
        )
        .route("/api/works/{id}/tags", get(work_tags))
        .route(
            "/api/works/{id}/tags/{tag}",
            put(assign_tag).delete(unassign_tag),
        )
        .route(
            "/api/works/{id}/mark",
            get(get_mark).put(set_mark).delete(clear_mark),
        )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTagBody {
    name: String,
    color: Option<String>,
    kind: Option<TagKind>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateTagBody {
    name: Option<String>,
    color: Option<String>,
    kind: Option<TagKind>,
}

/// `mark` stays a raw value so an unknown mark is `invalid_mark`, not a generic body error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetMarkBody {
    mark: Value,
    note: Option<String>,
}

#[derive(Serialize)]
struct TagView {
    id: TagId,
    name: String,
    color: String,
    kind: TagKind,
    work_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TagView {
    fn new(tag: Tag, work_count: i64) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            color: tag.color,
            kind: tag.kind,
            work_count,
            created_at: tag.created_at,
            updated_at: tag.updated_at,
        }
    }
}

impl From<TagWithCount> for TagView {
    fn from(row: TagWithCount) -> Self {
        Self::new(row.tag, row.work_count)
    }
}

#[derive(Serialize)]
struct MarkView {
    mark: WorkMark,
    note: Option<String>,
    updated_at: DateTime<Utc>,
}

impl From<StoredMark> for MarkView {
    fn from(stored: StoredMark) -> Self {
        Self {
            mark: stored.mark,
            note: stored.note,
            updated_at: stored.updated_at,
        }
    }
}

#[derive(Serialize)]
struct WorkTagsResponse {
    work_id: WorkId,
    tags: Vec<TagView>,
}

#[derive(Serialize)]
struct WorkMarkResponse {
    work_id: WorkId,
    mark: Option<MarkView>,
}

/// A mark as shown next to graph nodes.
#[derive(Debug, Serialize)]
pub(super) struct MarkBrief {
    mark: WorkMark,
    note: Option<String>,
}

/// Marks keyed by Work id; unmarked Works have no entry.
pub(super) type Marks = BTreeMap<String, MarkBrief>;
/// Tag ids keyed by Work id; untagged Works have no entry.
pub(super) type WorkTagIds = BTreeMap<String, Vec<TagId>>;

type Failure = Box<Response>;

fn failure(status: StatusCode, code: &str, message: &str) -> Response {
    graph_error(status, code, message)
}

fn db_failure(error: &DbError) -> Response {
    if matches!(error, DbError::Conflict(code) if code == "tag_exists") {
        return failure(
            StatusCode::CONFLICT,
            "tag_exists",
            "A tag with that name already exists.",
        );
    }
    graph_query_failure(error)
}

fn parse_tag_id(text: &str) -> Result<TagId, Failure> {
    text.parse().map_err(|_| {
        Box::new(failure(
            StatusCode::BAD_REQUEST,
            "invalid_tag_id",
            "Tag identifiers look like 't' followed by 22 characters.",
        ))
    })
}

fn tag_not_found() -> Response {
    failure(StatusCode::NOT_FOUND, "tag_not_found", "No such tag.")
}

fn invalid_body(detail: &str) -> Response {
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
        return Err(Box::new(invalid_body("expected a JSON object")));
    }
    serde_json::from_value(value).map_err(|error| Box::new(invalid_body(&error.to_string())))
}

fn curation_failure(error: lysilogy_core::CurationError) -> Response {
    failure(StatusCode::BAD_REQUEST, error.code(), &error.to_string())
}

async fn default_workspace(graph: &GraphState) -> Result<WorkspaceId, Failure> {
    graph
        .documents
        .ensure_default_workspace()
        .await
        .map_err(|error| Box::new(db_failure(&error)))
}

async fn list_tags(State(state): State<AppState>) -> Response {
    match list_tags_inner(&state).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn list_tags_inner(state: &AppState) -> Result<Failure, Failure> {
    let graph = graph_or_unavailable(state)?;
    let workspace = default_workspace(graph).await?;
    let rows = graph
        .curation
        .list_tags(workspace)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    let tags: Vec<TagView> = rows.into_iter().map(TagView::from).collect();
    Ok(Box::new(
        (StatusCode::OK, Json(serde_json::json!({ "tags": tags }))).into_response(),
    ))
}

async fn create_tag(State(state): State<AppState>, body: Bytes) -> Response {
    match create_tag_inner(&state, &body).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn create_tag_inner(state: &AppState, body: &Bytes) -> Result<Failure, Failure> {
    let body: CreateTagBody = parse_object(body)?;
    let name = TagName::parse(&body.name).map_err(|error| Box::new(curation_failure(error)))?;
    let color = body
        .color
        .as_deref()
        .map(TagColor::parse)
        .transpose()
        .map_err(|error| Box::new(curation_failure(error)))?;
    let graph = graph_or_unavailable(state)?;
    let workspace = default_workspace(graph).await?;
    let color = if let Some(color) = color {
        color
    } else {
        let count = graph
            .curation
            .count_tags(workspace)
            .await
            .map_err(|error| Box::new(db_failure(&error)))?;
        // A tag count is non-negative and far below usize::MAX.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let index = count as usize;
        TagColor::from_palette(index)
    };
    let tag = graph
        .curation
        .create_tag(
            workspace,
            &name,
            &color,
            body.kind.unwrap_or(TagKind::Topic),
        )
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    Ok(Box::new(
        (StatusCode::CREATED, Json(TagView::new(tag, 0))).into_response(),
    ))
}

async fn update_tag(
    State(state): State<AppState>,
    AxumPath(tag): AxumPath<String>,
    body: Bytes,
) -> Response {
    match update_tag_inner(&state, &tag, &body).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn update_tag_inner(state: &AppState, tag: &str, body: &Bytes) -> Result<Failure, Failure> {
    let tag_id = parse_tag_id(tag)?;
    let body: UpdateTagBody = parse_object(body)?;
    if body.name.is_none() && body.color.is_none() && body.kind.is_none() {
        return Err(Box::new(invalid_body(
            "at least one of name, color, kind is required",
        )));
    }
    let name = body
        .name
        .as_deref()
        .map(TagName::parse)
        .transpose()
        .map_err(|error| Box::new(curation_failure(error)))?;
    let color = body
        .color
        .as_deref()
        .map(TagColor::parse)
        .transpose()
        .map_err(|error| Box::new(curation_failure(error)))?;
    let graph = graph_or_unavailable(state)?;
    let workspace = default_workspace(graph).await?;
    let updated = graph
        .curation
        .update_tag(workspace, tag_id, name.as_ref(), color.as_ref(), body.kind)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?
        .ok_or_else(|| Box::new(tag_not_found()))?;
    let work_count = tag_work_count(graph, workspace, updated.id).await?;
    Ok(Box::new(
        (StatusCode::OK, Json(TagView::new(updated, work_count))).into_response(),
    ))
}

async fn tag_work_count(
    graph: &GraphState,
    workspace: WorkspaceId,
    tag: TagId,
) -> Result<i64, Failure> {
    let tags = graph
        .curation
        .list_tags(workspace)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    Ok(tags
        .into_iter()
        .find(|row| row.tag.id == tag)
        .map_or(0, |row| row.work_count))
}

async fn delete_tag(State(state): State<AppState>, AxumPath(tag): AxumPath<String>) -> Response {
    match delete_tag_inner(&state, &tag).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn delete_tag_inner(state: &AppState, tag: &str) -> Result<Failure, Failure> {
    let tag_id = parse_tag_id(tag)?;
    let graph = graph_or_unavailable(state)?;
    let workspace = default_workspace(graph).await?;
    let removed = graph
        .curation
        .delete_tag(workspace, tag_id)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    if !removed {
        return Err(Box::new(tag_not_found()));
    }
    Ok(Box::new(StatusCode::NO_CONTENT.into_response()))
}

async fn work_tags(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    match work_tags_inner(&state, &id).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn work_tags_inner(state: &AppState, id: &str) -> Result<Failure, Failure> {
    let work_id = parse_work_id(id)?;
    let graph = graph_or_unavailable(state)?;
    let work = fetch_work(graph, work_id).await?;
    let workspace = default_workspace(graph).await?;
    let rows = graph
        .curation
        .tags_for_works(workspace, &[work.id])
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    let counts: BTreeMap<TagId, i64> = graph
        .curation
        .list_tags(workspace)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?
        .into_iter()
        .map(|row| (row.tag.id, row.work_count))
        .collect();
    let tags = rows
        .into_iter()
        .map(|(_, tag)| {
            let count = counts.get(&tag.id).copied().unwrap_or(0);
            TagView::new(tag, count)
        })
        .collect();
    Ok(Box::new(
        (
            StatusCode::OK,
            Json(WorkTagsResponse {
                work_id: work.id,
                tags,
            }),
        )
            .into_response(),
    ))
}

async fn assign_tag(
    State(state): State<AppState>,
    AxumPath((id, tag)): AxumPath<(String, String)>,
) -> Response {
    match assign_tag_inner(&state, &id, &tag).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn assign_tag_inner(state: &AppState, id: &str, tag: &str) -> Result<Failure, Failure> {
    let work_id = parse_work_id(id)?;
    let tag_id = parse_tag_id(tag)?;
    let graph = graph_or_unavailable(state)?;
    let work = fetch_work(graph, work_id).await?;
    let workspace = default_workspace(graph).await?;
    graph
        .curation
        .get_tag(workspace, tag_id)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?
        .ok_or_else(|| Box::new(tag_not_found()))?;
    graph
        .curation
        .assign_tag(workspace, work.id, tag_id)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    Ok(Box::new(StatusCode::NO_CONTENT.into_response()))
}

async fn unassign_tag(
    State(state): State<AppState>,
    AxumPath((id, tag)): AxumPath<(String, String)>,
) -> Response {
    match unassign_tag_inner(&state, &id, &tag).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn unassign_tag_inner(state: &AppState, id: &str, tag: &str) -> Result<Failure, Failure> {
    let work_id = parse_work_id(id)?;
    let tag_id = parse_tag_id(tag)?;
    let graph = graph_or_unavailable(state)?;
    let work = fetch_work(graph, work_id).await?;
    let workspace = default_workspace(graph).await?;
    graph
        .curation
        .unassign_tag(workspace, work.id, tag_id)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    Ok(Box::new(StatusCode::NO_CONTENT.into_response()))
}

async fn get_mark(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    match get_mark_inner(&state, &id).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn get_mark_inner(state: &AppState, id: &str) -> Result<Failure, Failure> {
    let work_id = parse_work_id(id)?;
    let graph = graph_or_unavailable(state)?;
    let work = fetch_work(graph, work_id).await?;
    let workspace = default_workspace(graph).await?;
    let mark = graph
        .curation
        .marks_for_works(workspace, &[work.id])
        .await
        .map_err(|error| Box::new(db_failure(&error)))?
        .into_iter()
        .next()
        .map(|(_, stored)| MarkView::from(stored));
    Ok(Box::new(
        (
            StatusCode::OK,
            Json(WorkMarkResponse {
                work_id: work.id,
                mark,
            }),
        )
            .into_response(),
    ))
}

async fn set_mark(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> Response {
    match set_mark_inner(&state, &id, &body).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn set_mark_inner(state: &AppState, id: &str, body: &Bytes) -> Result<Failure, Failure> {
    let work_id = parse_work_id(id)?;
    let body: SetMarkBody = parse_object(body)?;
    let mark = body
        .mark
        .as_str()
        .and_then(|text| text.parse::<WorkMark>().ok())
        .ok_or_else(|| {
            Box::new(failure(
                StatusCode::BAD_REQUEST,
                "invalid_mark",
                "The mark must be one of pinned, downplayed, hidden.",
            ))
        })?;
    let note =
        parse_mark_note(body.note.as_deref()).map_err(|error| Box::new(curation_failure(error)))?;
    let graph = graph_or_unavailable(state)?;
    let work = fetch_work(graph, work_id).await?;
    let workspace = default_workspace(graph).await?;
    let stored = graph
        .curation
        .set_mark(workspace, work.id, mark, note.as_deref())
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    Ok(Box::new(
        (
            StatusCode::OK,
            Json(WorkMarkResponse {
                work_id: work.id,
                mark: Some(MarkView::from(stored)),
            }),
        )
            .into_response(),
    ))
}

async fn clear_mark(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    match clear_mark_inner(&state, &id).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn clear_mark_inner(state: &AppState, id: &str) -> Result<Failure, Failure> {
    let work_id = parse_work_id(id)?;
    let graph = graph_or_unavailable(state)?;
    let work = fetch_work(graph, work_id).await?;
    let workspace = default_workspace(graph).await?;
    graph
        .curation
        .clear_mark(workspace, work.id)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?;
    Ok(Box::new(StatusCode::NO_CONTENT.into_response()))
}

/// Marks and tag ids for the given Works, as carried by graph responses.
pub(super) async fn curation_for(
    state: &AppState,
    work_ids: &[WorkId],
) -> Result<(Marks, WorkTagIds), Failure> {
    let graph = graph_or_unavailable(state)?;
    let workspace = default_workspace(graph).await?;
    let marks = graph
        .curation
        .marks_for_works(workspace, work_ids)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?
        .into_iter()
        .map(|(work, stored)| {
            (
                work.to_string(),
                MarkBrief {
                    mark: stored.mark,
                    note: stored.note,
                },
            )
        })
        .collect();
    let mut work_tags = WorkTagIds::new();
    for (work, tag) in graph
        .curation
        .tags_for_works(workspace, work_ids)
        .await
        .map_err(|error| Box::new(db_failure(&error)))?
    {
        work_tags.entry(work.to_string()).or_default().push(tag.id);
    }
    Ok((marks, work_tags))
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{Visibility, WorkKind};
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

    struct Fixture {
        _library: tempfile::TempDir,
        _data: tempfile::TempDir,
        state: AppState,
    }

    async fn fixture(graph: Option<GraphState>) -> Fixture {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let mut state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        if let Some(graph) = graph {
            state = state.with_graph(graph);
        }
        Fixture {
            _library: library,
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

    async fn new_work(graph: &GraphState) -> WorkId {
        graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id
    }

    /// Create a tag through the API and return its id.
    async fn new_tag(state: &AppState, name: &str) -> String {
        let (status, json) = request(
            state,
            "POST",
            "/api/tags",
            Some(&serde_json::json!({ "name": name }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        json["id"].as_str().expect("tag id").to_owned()
    }

    #[tokio::test]
    async fn should_return_503_when_tags_requested_without_graph() {
        let fixture = fixture(None).await;
        let work = WorkId::new();
        let tag = TagId::new();
        let cases = [
            ("GET", "/api/tags".to_owned(), None),
            ("POST", "/api/tags".to_owned(), Some(r#"{"name":"x"}"#)),
            ("PATCH", format!("/api/tags/{tag}"), Some(r#"{"name":"x"}"#)),
            ("DELETE", format!("/api/tags/{tag}"), None),
            ("GET", format!("/api/works/{work}/tags"), None),
            ("PUT", format!("/api/works/{work}/tags/{tag}"), None),
            ("DELETE", format!("/api/works/{work}/tags/{tag}"), None),
            ("GET", format!("/api/works/{work}/mark"), None),
            (
                "PUT",
                format!("/api/works/{work}/mark"),
                Some(r#"{"mark":"pinned"}"#),
            ),
            ("DELETE", format!("/api/works/{work}/mark"), None),
        ];
        for (method, uri, body) in cases {
            let (status, json) = request(&fixture.state, method, &uri, body).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{method} {uri}");
            assert_eq!(json["error"], "graph_unavailable", "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn should_check_path_ids_and_body_before_graph_when_graph_unavailable() {
        let fixture = fixture(None).await;
        let (status, json) = request(&fixture.state, "GET", "/api/works/nope/mark", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_work_id");
        let (status, json) = request(&fixture.state, "POST", "/api/tags", Some("{}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_body");
        let work = WorkId::new();
        let (status, json) = request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{work}/mark"),
            Some(r#"{"mark":"loud"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_mark");
    }

    #[tokio::test]
    async fn should_create_tag_with_palette_color_when_color_omitted() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("ws");
        let fixture = fixture(Some(graph)).await;
        let before = fixture
            .state
            .graph
            .as_ref()
            .expect("graph")
            .curation
            .count_tags(workspace)
            .await
            .expect("count");
        let name = format!("palette-{}", marker());
        let (status, json) = request(
            &fixture.state,
            "POST",
            "/api/tags",
            Some(&serde_json::json!({ "name": name }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["name"], name);
        assert_eq!(json["kind"], "topic");
        assert_eq!(json["work_count"], 0);
        let color = json["color"].as_str().expect("color");
        let palette = lysilogy_core::TAG_PALETTE;
        let candidates = [before, before + 1, before + 2];
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let matches_palette = candidates
            .iter()
            .any(|count| palette[*count as usize % palette.len()] == color);
        assert!(matches_palette, "{color}");
        let id = json["id"].as_str().expect("id");
        request(&fixture.state, "DELETE", &format!("/api/tags/{id}"), None).await;
    }

    #[tokio::test]
    async fn should_normalize_color_and_name_when_creating_tag() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph)).await;
        let token = marker();
        let body = serde_json::json!({
            "name": format!("  Spaced   {token}  Out "),
            "color": "#ABC",
            "kind": "status",
        });
        let (status, json) =
            request(&fixture.state, "POST", "/api/tags", Some(&body.to_string())).await;
        assert_eq!(status, StatusCode::CREATED, "{json}");
        assert_eq!(json["name"], format!("Spaced {token} Out"));
        assert_eq!(json["color"], "#aabbcc");
        assert_eq!(json["kind"], "status");
        let id = json["id"].as_str().expect("id");
        request(&fixture.state, "DELETE", &format!("/api/tags/{id}"), None).await;
    }

    #[tokio::test]
    async fn should_return_409_when_creating_tag_with_existing_name_in_other_case() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph)).await;
        let name = format!("Dup-{}", marker());
        let id = new_tag(&fixture.state, &name).await;
        let (status, json) = request(
            &fixture.state,
            "POST",
            "/api/tags",
            Some(&serde_json::json!({ "name": name.to_uppercase() }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "tag_exists");

        let other = new_tag(&fixture.state, &format!("Other-{}", marker())).await;
        let (status, json) = request(
            &fixture.state,
            "PATCH",
            &format!("/api/tags/{other}"),
            Some(&serde_json::json!({ "name": name.to_lowercase() }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "tag_exists");
        for tag in [id, other] {
            request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
        }
    }

    #[tokio::test]
    async fn should_return_400_when_tag_body_has_unknown_field_or_wrong_type() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph)).await;
        let too_long = format!(r#"{{"name":"{}"}}"#, "n".repeat(49));
        let cases: Vec<(String, &str)> = vec![
            (r#"{"name":"x","extra":1}"#.to_owned(), "invalid_body"),
            (r#"{"name":5}"#.to_owned(), "invalid_body"),
            (r"{}".to_owned(), "invalid_body"),
            (r#"["x"]"#.to_owned(), "invalid_body"),
            (r#""x""#.to_owned(), "invalid_body"),
            (r"null".to_owned(), "invalid_body"),
            (String::new(), "invalid_body"),
            ("{not json".to_owned(), "invalid_body"),
            (r#"{"name":"x","kind":"public"}"#.to_owned(), "invalid_body"),
            (r#"{"name":"x","kind":7}"#.to_owned(), "invalid_body"),
            (r#"{"name":""}"#.to_owned(), "invalid_tag_name"),
            (r#"{"name":"   "}"#.to_owned(), "invalid_tag_name"),
            (r#"{"name":"a\u0007b"}"#.to_owned(), "invalid_tag_name"),
            (too_long, "invalid_tag_name"),
            (r#"{"name":"x","color":"red"}"#.to_owned(), "invalid_color"),
            (
                r##"{"name":"x","color":"#12345"}"##.to_owned(),
                "invalid_color",
            ),
        ];
        for (body, code) in cases {
            let (status, json) = request(&fixture.state, "POST", "/api/tags", Some(&body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], code, "{body}: {json}");
        }
    }

    #[tokio::test]
    async fn should_return_400_when_patch_body_is_empty_object() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("patch-{}", marker())).await;
        let uri = format!("/api/tags/{tag}");
        for body in ["{}", "[]", "", r#"{"nope":1}"#] {
            let (status, json) = request(&fixture.state, "PATCH", &uri, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], "invalid_body", "{body}");
        }
        let (status, json) = request(
            &fixture.state,
            "PATCH",
            &uri,
            Some(r##"{"color":"#FF0000","kind":"status"}"##),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["color"], "#ff0000");
        assert_eq!(json["kind"], "status");
        request(&fixture.state, "DELETE", &uri, None).await;
    }

    #[tokio::test]
    async fn should_return_404_when_patching_or_deleting_unknown_tag() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(Some(graph)).await;
        let uri = format!("/api/tags/{}", TagId::new());
        let (status, json) =
            request(&fixture.state, "PATCH", &uri, Some(r#"{"name":"ghost"}"#)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "tag_not_found");
        let (status, json) = request(&fixture.state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "tag_not_found");
    }

    #[tokio::test]
    async fn should_return_400_when_tag_id_is_malformed() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let cases = [
            ("PATCH", "/api/tags/nonsense".to_owned()),
            ("DELETE", format!("/api/tags/{work}")),
            ("PUT", format!("/api/works/{work}/tags/{work}")),
            ("DELETE", format!("/api/works/{work}/tags/{work}")),
        ];
        for (method, uri) in cases {
            let body = (method == "PATCH").then_some(r#"{"name":"x"}"#);
            let (status, json) = request(&fixture.state, method, &uri, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}");
            assert_eq!(json["error"], "invalid_tag_id", "{method} {uri}");
        }
        let tag = TagId::new();
        let (status, json) = request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{tag}/tags/{tag}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_work_id");
    }

    #[tokio::test]
    async fn should_assign_idempotently_and_list_tags_when_putting_work_tag() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("assign-{}", marker())).await;
        let uri = format!("/api/works/{work}/tags/{tag}");
        for _ in 0..2 {
            let (status, _) = request(&fixture.state, "PUT", &uri, None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/tags"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["work_id"], work.to_string());
        let tags = json["tags"].as_array().expect("tags");
        assert_eq!(tags.len(), 1, "{json}");
        assert_eq!(tags[0]["id"], tag);
        assert_eq!(tags[0]["work_count"], 1);
        request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
    }

    #[tokio::test]
    async fn should_return_404_when_assigning_unknown_tag_or_work() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("nf-{}", marker())).await;
        let (status, json) = request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{work}/tags/{}", TagId::new()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "tag_not_found");
        let (status, json) = request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{}/tags/{tag}", WorkId::new()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert_eq!(json["error"], "work_not_found");
        for uri in [
            format!("/api/works/{}/tags", WorkId::new()),
            format!("/api/works/{}/mark", WorkId::new()),
        ] {
            let (status, json) = request(&fixture.state, "GET", &uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(json["error"], "work_not_found");
        }
        request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
    }

    #[tokio::test]
    async fn should_succeed_when_unassigning_absent_tag() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("absent-{}", marker())).await;
        let uri = format!("/api/works/{work}/tags/{tag}");
        let (status, _) = request(&fixture.state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        request(&fixture.state, "PUT", &uri, None).await;
        for _ in 0..2 {
            let (status, _) = request(&fixture.state, "DELETE", &uri, None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let (_, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/tags"),
            None,
        )
        .await;
        assert!(json["tags"].as_array().expect("tags").is_empty(), "{json}");
        request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
    }

    #[tokio::test]
    async fn should_follow_redirect_when_tagging_merged_work() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let merged = new_work(&graph).await;
        let survivor = new_work(&graph).await;
        graph
            .works
            .merge_work(merged, survivor, "test", "test")
            .await
            .expect("merge");
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("redir-{}", marker())).await;
        let (status, _) = request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{merged}/tags/{tag}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{merged}/tags"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["work_id"], survivor.to_string());
        assert_eq!(json["tags"][0]["id"], tag);
        let (_, json) = request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{merged}/mark"),
            Some(r#"{"mark":"pinned"}"#),
        )
        .await;
        assert_eq!(json["work_id"], survivor.to_string());
        request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
        request(
            &fixture.state,
            "DELETE",
            &format!("/api/works/{survivor}/mark"),
            None,
        )
        .await;
    }

    #[tokio::test]
    async fn should_set_replace_and_clear_mark_when_using_mark_routes() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let uri = format!("/api/works/{work}/mark");
        let (status, json) = request(
            &fixture.state,
            "PUT",
            &uri,
            Some(r#"{"mark":"pinned","note":"  read this  "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["work_id"], work.to_string());
        assert_eq!(json["mark"]["mark"], "pinned");
        assert_eq!(json["mark"]["note"], "read this");
        assert!(json["mark"]["updated_at"].is_string());

        let (status, json) = request(
            &fixture.state,
            "PUT",
            &uri,
            Some(r#"{"mark":"hidden","note":""}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let (_, json) = request(&fixture.state, "GET", &uri, None).await;
        assert_eq!(json["mark"]["mark"], "hidden", "{json}");
        assert!(json["mark"]["note"].is_null(), "{json}");

        for _ in 0..2 {
            let (status, _) = request(&fixture.state, "DELETE", &uri, None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let (_, json) = request(&fixture.state, "GET", &uri, None).await;
        assert!(json["mark"].is_null(), "{json}");
    }

    #[tokio::test]
    async fn should_return_400_when_mark_is_unknown_or_note_too_long() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let uri = format!("/api/works/{work}/mark");
        let long_note = format!(r#"{{"mark":"pinned","note":"{}"}}"#, "n".repeat(2001));
        let cases: Vec<(String, &str)> = vec![
            (r#"{"mark":"loud"}"#.to_owned(), "invalid_mark"),
            (r#"{"mark":"Pinned"}"#.to_owned(), "invalid_mark"),
            (r#"{"mark":5}"#.to_owned(), "invalid_mark"),
            (r#"{"mark":null}"#.to_owned(), "invalid_mark"),
            (long_note, "invalid_note"),
            (r"{}".to_owned(), "invalid_body"),
            (r#"{"note":"x"}"#.to_owned(), "invalid_body"),
            (r#"{"mark":"pinned","extra":1}"#.to_owned(), "invalid_body"),
            (r#"{"mark":"pinned","note":3}"#.to_owned(), "invalid_body"),
            (r#"["pinned"]"#.to_owned(), "invalid_body"),
            (String::new(), "invalid_body"),
        ];
        for (body, code) in cases {
            let (status, json) = request(&fixture.state, "PUT", &uri, Some(&body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
            assert_eq!(json["error"], code, "{body}: {json}");
        }
        let (_, json) = request(&fixture.state, "GET", &uri, None).await;
        assert!(
            json["mark"].is_null(),
            "rejected bodies store nothing: {json}"
        );
    }

    #[tokio::test]
    async fn should_return_null_mark_when_work_has_none() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/mark"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["work_id"], work.to_string());
        assert!(json["mark"].is_null(), "{json}");
    }

    #[tokio::test]
    async fn should_drop_assignments_when_tag_deleted() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = new_work(&graph).await;
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("drop-{}", marker())).await;
        request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{work}/tags/{tag}"),
            None,
        )
        .await;
        let (status, _) =
            request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{work}/tags"),
            None,
        )
        .await;
        assert!(json["tags"].as_array().expect("tags").is_empty(), "{json}");
        let (_, json) = request(&fixture.state, "GET", "/api/tags", None).await;
        let listed = json["tags"].as_array().expect("tags");
        assert!(listed.iter().all(|row| row["id"] != tag.as_str()), "{json}");
    }

    #[tokio::test]
    async fn should_include_marks_and_work_tags_when_reading_citations() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let center = new_work(&graph).await;
        let cited = new_work(&graph).await;
        let citing = new_work(&graph).await;
        let plain = new_work(&graph).await;
        graph
            .citations
            .add_assertion(center, cited, None)
            .await
            .expect("edge");
        graph
            .citations
            .add_assertion(citing, center, None)
            .await
            .expect("edge");
        graph
            .citations
            .add_assertion(center, plain, None)
            .await
            .expect("edge");
        let fixture = fixture(Some(graph)).await;
        let tag = new_tag(&fixture.state, &format!("cit-{}", marker())).await;
        request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{cited}/tags/{tag}"),
            None,
        )
        .await;
        request(
            &fixture.state,
            "PUT",
            &format!("/api/works/{citing}/mark"),
            Some(r#"{"mark":"downplayed","note":"meh"}"#),
        )
        .await;

        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{center}/citations"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["marks"][citing.to_string()]["mark"], "downplayed");
        assert_eq!(json["marks"][citing.to_string()]["note"], "meh");
        assert_eq!(json["work_tags"][cited.to_string()][0], tag);
        assert!(json["marks"].get(plain.to_string()).is_none(), "{json}");
        assert!(json["work_tags"].get(plain.to_string()).is_none(), "{json}");
        request(&fixture.state, "DELETE", &format!("/api/tags/{tag}"), None).await;
    }
}
