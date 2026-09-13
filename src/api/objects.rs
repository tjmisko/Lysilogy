use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::header,
    routing::get,
};

use super::{AppState, parse_id};
use crate::{
    Result,
    domain::PaperId,
    objects::{ObjectsArtifact, load_or_build},
    source_index::BuildPriority,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/papers/{id}/objects", get(paper_objects))
}

impl AppState {
    pub async fn paper_objects(&self, id: &PaperId) -> Result<ObjectsArtifact> {
        let document = self
            .reading_index_document(id, false, BuildPriority::Interactive)
            .await?;
        load_or_build(&self.store.paper_dir(id), id, &document).await
    }
}

async fn paper_objects(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<(
    [(header::HeaderName, &'static str); 1],
    Json<ObjectsArtifact>,
)> {
    let artifact = state.paper_objects(&parse_id(&id)?).await?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-cache")],
        Json(artifact),
    ))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{objects::OBJECTS_FILE, source_index::test_support::cache_fixture};

    #[tokio::test]
    async fn should_serve_objects_when_reading_index_is_cached_without_analysis() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let filename = "Author - Paper.pdf";
        let source = library.path().join(filename);
        tokio::fs::write(&source, b"cached source fixture")
            .await
            .unwrap();
        let id = PaperId::from_relative_path(std::path::Path::new(filename));
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let directory = state.store.paper_dir(&id);
        cache_fixture(&source, &directory).await;
        let path = directory.join("reading-index.json");
        let mut cache: Value =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        cache["index"]["figures"] = json!([{
            "id": "figure-3", "kind": "figure", "label": "Figure 3", "page": 1,
            "caption": "A cached caption.", "start": 0, "end": 17, "spans": [],
            "rect": null, "confidence": "candidate", "references": []
        }]);
        cache["index"]["text"] = json!("Figure 3. A cached caption.");
        cache["index"]["pages"] = json!([{"number":1,"width":600.0,"height":800.0,"start":0,"end":27,"provenance":"native","confidence":null}]);
        cache["index"]["tokens"] = json!([
            {"start":0,"end":6,"page":1,"text":"Figure","rects":[{"x_min":50.0,"x_max":85.0,"y_min":200.0,"y_max":210.0}],"provenance":"native"},
            {"start":7,"end":9,"page":1,"text":"3.","rects":[{"x_min":90.0,"x_max":100.0,"y_min":200.0,"y_max":210.0}],"provenance":"native"},
            {"start":10,"end":27,"page":1,"text":"A cached caption.","rects":[{"x_min":105.0,"x_max":190.0,"y_min":200.0,"y_max":210.0}],"provenance":"native"}
        ]);
        cache["index"]["objects"]["paragraph"] = json!([{"start":0,"end":27,"kind":"caption"}]);
        tokio::fs::write(&path, serde_json::to_vec(&cache).unwrap())
            .await
            .unwrap();
        let extraction_guard = state.tools_extract.lock().await;
        let app = crate::build_router(state.clone(), None);
        let uri = format!("/api/papers/{id}/objects");
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            app.clone()
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-cache"
        );
        let first: ObjectsArtifact =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(first.objects[0].id, "fig-3");
        assert_eq!(first.objects[0].text, "Figure 3. A cached caption.");
        assert!(directory.join(OBJECTS_FILE).is_file());
        assert!(!directory.join("analysis.json").exists());
        cache["generation"] = json!("new-generation");
        cache["index"]["text"] = json!("Figure 3. A newer! caption.");
        cache["index"]["tokens"][2]["text"] = json!("A newer! caption.");
        tokio::fs::write(&path, serde_json::to_vec(&cache).unwrap())
            .await
            .unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            app.oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let second: ObjectsArtifact =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_ne!(
            first.reading_index_generation,
            second.reading_index_generation
        );
        assert_eq!(second.objects[0].id, first.objects[0].id);
        assert_eq!(second.objects[0].text, "Figure 3. A newer! caption.");
        drop(extraction_guard);
    }

    #[tokio::test]
    async fn should_reject_objects_request_when_paper_id_is_invalid_or_unknown() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let app = crate::build_router(state, None);
        for (id, status) in [
            ("invalid", StatusCode::BAD_REQUEST),
            ("0000000000000000", StatusCode::NOT_FOUND),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/papers/{id}/objects"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
    }
}
