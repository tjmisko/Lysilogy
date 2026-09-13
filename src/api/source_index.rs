use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

use super::{AppState, parse_id};
use crate::{
    Error, Result,
    domain::PaperId,
    source_index::{BuildPriority, IndexDocument, ReadingIndex},
};

mod jobs;
pub(super) use jobs::IndexJobs;

const CACHE_CONTROL: &str = "private, max-age=2592000, must-revalidate";

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/papers/{id}/reading-index", get(reading_index))
}

impl AppState {
    /// Uses a separate local extraction index; no model or analysis is necessary.
    pub async fn reading_index(&self, id: &PaperId, refresh: bool) -> Result<ReadingIndex> {
        Ok(self
            .reading_index_document(id, refresh, BuildPriority::Interactive)
            .await?
            .index
            .clone())
    }

    pub(super) async fn reading_index_document(
        &self,
        id: &PaperId,
        refresh: bool,
        priority: BuildPriority,
    ) -> Result<std::sync::Arc<IndexDocument>> {
        let source = self.source_path(id).await?;
        self.reading_indexes
            .get_or_build(
                source,
                self.store.paper_dir(id),
                refresh,
                priority,
                self.tools_extract.clone(),
            )
            .await
    }
}

#[derive(Default, Deserialize)]
struct Options {
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    priority: BuildPriority,
}

async fn reading_index(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(options): Query<Options>,
    headers: HeaderMap,
) -> Result<Response> {
    let priority = match headers.get("x-reading-priority") {
        Some(value) => match value.to_str().unwrap_or("") {
            "background" => BuildPriority::Background,
            "interactive" => BuildPriority::Interactive,
            _ => {
                return Err(Error::InvalidRequest(
                    "X-Reading-Priority must be background or interactive.".into(),
                ));
            }
        },
        None => options.priority,
    };
    let document = state
        .reading_index_document(&parse_id(&id)?, options.refresh, priority)
        .await?;
    let unchanged = !options.refresh
        && headers.get_all(header::IF_NONE_MATCH).iter().any(|value| {
            value.to_str().is_ok_and(|value| {
                value.split(',').any(|candidate| {
                    let candidate = candidate.trim();
                    candidate == "*"
                        || candidate.strip_prefix("W/").unwrap_or(candidate) == document.etag
                })
            })
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Json(&document.index).into_response()
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&document.etag).map_err(|error| Error::Task(error.to_string()))?,
    );
    Ok(response)
}

#[cfg(test)]
mod tests;
