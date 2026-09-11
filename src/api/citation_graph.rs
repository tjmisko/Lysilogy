use super::{AppState, parse_id};
use crate::{
    Result,
    citation_graph::{GraphRequest, GraphSnapshot, SNAPSHOT_FILE},
    domain::PaperId,
    store::write_atomic,
};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    routing::get,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/papers/{id}/citation-graph", get(snapshot).post(fetch))
}
impl AppState {
    pub async fn fetch_citation_graph(
        &self,
        id: &PaperId,
        request: &GraphRequest,
    ) -> Result<GraphSnapshot> {
        // Validate membership before external requests. Explicit IDs avoid speculative title matching.
        self.source_path(id).await?;
        let snapshot =
            crate::citation_graph::fetch(id.as_str(), request, &self.citation_http).await?;
        let directory = self.store.paper_dir(id);
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| crate::Error::io(&directory, error))?;
        write_atomic(
            &directory.join(SNAPSHOT_FILE),
            &serde_json::to_vec_pretty(&snapshot)?,
        )
        .await?;
        Ok(snapshot)
    }
    pub async fn citation_graph(&self, id: &PaperId) -> Result<Option<GraphSnapshot>> {
        self.source_path(id).await?;
        let path = self.store.paper_dir(id).join(SNAPSHOT_FILE);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(crate::Error::io(path, error)),
        }
    }
}
async fn fetch(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<GraphRequest>,
) -> Result<Json<GraphSnapshot>> {
    state
        .fetch_citation_graph(&parse_id(&id)?, &request)
        .await
        .map(Json)
}
async fn snapshot(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Option<GraphSnapshot>>> {
    state.citation_graph(&parse_id(&id)?).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn api_persists_provider_failures_honestly_and_get_never_fetches() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let filename = "Ada - 1843 - Notes.pdf";
        tokio::fs::write(library.path().join(filename), b"discovery fixture")
            .await
            .unwrap();
        let id = PaperId::from_relative_path(std::path::Path::new(filename));
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let app = crate::build_router(state, None);
        let uri = format!("/api/papers/{id}/citation-graph");
        let empty = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::OK);
        assert_eq!(
            empty
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .as_ref(),
            b"null"
        );
        let response = app.clone().oneshot(Request::builder().uri(&uri).method("POST").header("content-type","application/json")
            .body(Body::from(r#"{"identifier":"doi:10.1234/paper","providers":["crossref"],"directions":["citations"]}"#)).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let fetched: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(fetched["reports"][0]["failure"]["kind"], "unsupported");
        assert_eq!(fetched["reports"][0]["coverage"], "unavailable");
        assert_eq!(fetched["paper_id"], id.as_str());
        let response = app
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(saved, fetched);
    }

    #[tokio::test]
    async fn api_rejects_unknown_paper_before_fetching() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let app = crate::build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/papers/1234567890abcdef/citation-graph")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"identifier":"doi:10.1234/paper"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
