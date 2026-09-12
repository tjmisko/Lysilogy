use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    routing::get,
};
use serde::Deserialize;

use super::{AppState, parse_id};
use crate::{Result, domain::PaperId, source_index::ReadingIndex};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/papers/{id}/reading-index", get(reading_index))
}

impl AppState {
    /// Uses a separate local extraction index; no model or analysis is necessary.
    pub async fn reading_index(&self, id: &PaperId, refresh: bool) -> Result<ReadingIndex> {
        let source = self.source_path(id).await?;
        // Existing extraction lock bounds total Poppler/OCR resource use across readers.
        let _guard = self.tools_extract.lock().await;
        crate::source_index::load_or_build(&source, &self.store.paper_dir(id), refresh).await
    }
}

#[derive(Default, Deserialize)]
struct Options {
    #[serde(default)]
    refresh: bool,
}

async fn reading_index(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(options): Query<Options>,
) -> Result<Json<ReadingIndex>> {
    state
        .reading_index(&parse_id(&id)?, options.refresh)
        .await
        .map(Json)
}
