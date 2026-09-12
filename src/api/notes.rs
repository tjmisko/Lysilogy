use super::{AppState, parse_id};
use crate::{
    Error,
    notes::{MAX_NOTES_BYTES, SaveNoteRequest},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as AxumPath, State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/papers/{id}/notes", get(read).put(save))
        .route("/api/papers/{id}/notes/open", post(open))
        .layer(DefaultBodyLimit::max(MAX_NOTES_BYTES * 6 + 4_096))
}

#[derive(Default, Deserialize)]
struct OpenNoteRequest {
    #[serde(default)]
    created_at: Option<DateTime<FixedOffset>>,
}

async fn open(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    request: Result<Json<OpenNoteRequest>, JsonRejection>,
) -> Response {
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"invalid_notes_request", "message":error.body_text()})),
        )
            .into_response(),
    };
    let path = match paper_path(&state, &id).await {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };
    let source = match parse_id(&id) {
        Ok(id) => match state.source_path(&id).await {
            Ok(source) => source,
            Err(error) => return error.into_response(),
        },
        Err(error) => return error.into_response(),
    };
    match state.notes.open(&path, &source, request.created_at).await {
        Ok(note) => Json(note).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn paper_path(state: &AppState, id: &str) -> crate::Result<String> {
    let id = parse_id(id)?;
    state
        .catalog
        .read()
        .await
        .get(&id)
        .map(|entry| entry.overview.relative_path.clone())
        .ok_or_else(|| Error::PaperNotFound(id.to_string()))
}

async fn read(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let path = match paper_path(&state, &id).await {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };
    match state.notes.read(&path).await {
        Ok(note) => Json(note).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn save(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<SaveNoteRequest>,
) -> Response {
    let path = match paper_path(&state, &id).await {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };
    match state
        .notes
        .save(&path, request.text, request.revision)
        .await
    {
        Ok(note) => Json(note).into_response(),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PaperId;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn notes_api_reads_writes_and_detects_external_changes() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let notes_parent = tempfile::tempdir().unwrap();
        let notes_root = notes_parent.path().join("notes");
        let filename = "Author - Paper.pdf";
        tokio::fs::write(library.path().join(filename), b"discovery fixture")
            .await
            .unwrap();
        let id = PaperId::from_relative_path(std::path::Path::new(filename));
        let state = AppState::new(library.path(), data.path())
            .await
            .unwrap()
            .with_notes_root(&notes_root);
        let app = crate::build_router(state, None);
        let uri = format!("/api/papers/{id}/notes");
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let empty: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(empty["text"], "");
        assert!(empty["revision"].is_null());
        assert!(!notes_root.exists());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(r##"{"text":"# My notes\n","revision":null}"##))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(notes_root.join("Author - Paper.md"))
                .await
                .unwrap(),
            "# My notes\n"
        );
        tokio::fs::write(notes_root.join("Author - Paper.md"), "An external edit")
            .await
            .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"text":"Stale draft","revision":saved["revision"]})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            tokio::fs::read_to_string(notes_root.join("Author - Paper.md"))
                .await
                .unwrap(),
            "An external edit"
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/papers/1234567890abcdef/notes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn notes_open_api_initializes_once_with_local_time_and_returns_json_errors() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let notes = tempfile::tempdir().unwrap();
        let filename = "Author [v2] - 日本語 #%.pdf";
        tokio::fs::write(library.path().join(filename), b"discovery fixture")
            .await
            .unwrap();
        let id = PaperId::from_relative_path(std::path::Path::new(filename));
        let state = AppState::new(library.path(), data.path())
            .await
            .unwrap()
            .with_notes_root(notes.path());
        let app = crate::build_router(state, None);
        let uri = format!("/api/papers/{id}/notes/open");
        let request = || {
            Request::builder()
                .uri(&uri)
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"created_at":"2026-09-09T23:34:00-07:00"}"#))
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let first: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let text = first["text"].as_str().unwrap();
        assert!(text.starts_with("---\ndate: 2026-09-09\ntime: 23:34\ntags:\n  - paper\n---"));
        assert!(text.contains("[Author \\[v2\\] - 日本語 #%](<file:///"));
        assert!(text.contains("%E6%97%A5%E6%9C%AC%E8%AA%9E%20%23%25.pdf>"));
        assert!(first["revision"].is_string());
        let response = app.clone().oneshot(request()).await.unwrap();
        let again: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(first, again);
        let note_path = notes
            .path()
            .join(std::path::Path::new(filename).with_extension("md"));
        tokio::fs::write(&note_path, "").await.unwrap();
        let response = app.clone().oneshot(request()).await.unwrap();
        let empty: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(empty["text"], "");
        assert!(empty["revision"].is_string());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"created_at":"invalid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["content-type"], "application/json");
        let error: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(error["error"], "invalid_notes_request");
    }
}
