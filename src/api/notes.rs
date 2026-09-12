use super::{AppState, parse_id};
use crate::{
    Error,
    notes::{MAX_NOTES_BYTES, SaveNoteRequest},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    response::{IntoResponse, Response},
    routing::get,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/papers/{id}/notes", get(read).put(save))
        .layer(DefaultBodyLimit::max(MAX_NOTES_BYTES * 6 + 4_096))
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
}
