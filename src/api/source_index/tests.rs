use super::*;
use crate::source_index::test_support::{cache_fixture, has_pdftotext, native_pdf};
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn cached_http_response_and_conditional_requests_bypass_extraction_lock() {
    let library = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let filename = "Author - Paper.pdf";
    let source = library.path().join(filename);
    tokio::fs::write(&source, b"cached source fixture")
        .await
        .unwrap();
    let id = PaperId::from_relative_path(std::path::Path::new(filename));
    let state = AppState::new(library.path(), data.path()).await.unwrap();
    cache_fixture(&source, &state.store.paper_dir(&id)).await;
    let _guard = state.tools_extract.lock().await;
    let app = crate::build_router(state.clone(), None);
    let uri = format!("/api/papers/{id}/reading-index");
    let request = Request::builder()
        .uri(format!("{uri}?extraQuery=accepted"))
        .header("x-reading-priority", "background")
        .body(Body::empty())
        .unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        app.clone().oneshot(request),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], CACHE_CONTROL);
    assert!(!response.headers().contains_key(header::VARY));
    let etag = response.headers()[header::ETAG]
        .to_str()
        .unwrap()
        .to_owned();
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["text"], "A permanently cached source index.");
    assert!(body.get("etag").is_none());
    let request = Request::builder()
        .uri(&uri)
        .header("x-reading-priority", "interactive")
        .header(header::IF_NONE_MATCH, format!("\"stale\", W/{etag}"))
        .body(Body::empty())
        .unwrap();
    let response =
        tokio::time::timeout(std::time::Duration::from_millis(250), app.oneshot(request))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()[header::ETAG], etag);
    assert_eq!(response.headers()[header::CACHE_CONTROL], CACHE_CONTROL);
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

#[tokio::test]
async fn refresh_and_changed_sources_return_new_http_validators() {
    if !has_pdftotext() {
        return;
    }
    let library = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let filename = "Author - Paper.pdf";
    let source = library.path().join(filename);
    tokio::fs::write(
        &source,
        native_pdf("An original paper has enough text for native reading index extraction."),
    )
    .await
    .unwrap();
    let id = PaperId::from_relative_path(std::path::Path::new(filename));
    let state = AppState::new(library.path(), data.path()).await.unwrap();
    let app = crate::build_router(state.clone(), None);
    let uri = format!("/api/papers/{id}/reading-index");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{uri}?priority=background"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let original = response.headers()[header::ETAG].clone();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{uri}?refresh=true"))
                .header(header::IF_NONE_MATCH, "*")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let refreshed = response.headers()[header::ETAG].clone();
    assert_ne!(original, refreshed);
    tokio::fs::write(&source, native_pdf("The changed paper now contains different text and still enough native words for extraction.")).await.unwrap();
    let changed = state.refresh().await.unwrap().papers[0].id.clone();
    assert_ne!(id, changed);
    let uri = format!("/api/papers/{changed}/reading-index");
    let response = app
        .oneshot(
            Request::builder()
                .uri(&uri)
                .header(header::IF_NONE_MATCH, refreshed.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_ne!(response.headers()[header::ETAG], refreshed);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body["text"].as_str().unwrap().contains("changed paper"));
}

#[tokio::test]
async fn unknown_papers_and_invalid_priorities_do_not_start_builds() {
    let library = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let state = AppState::new(library.path(), data.path()).await.unwrap();
    let app = crate::build_router(state, None);
    let uri = "/api/papers/1234567890abcdef/reading-index";
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("x-reading-priority", "urgent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
