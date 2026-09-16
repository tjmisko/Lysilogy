use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as AxumPath, State, rejection::JsonRejection},
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;

use super::{AppState, parse_id};
use crate::{
    Error, Result,
    domain::PaperId,
    objects::{
        ObjectsArtifact,
        grades::{self, GradingStatus, MAX_GRADES_BYTES, ObjectGrades, SaveOutcome},
        load_or_build_from_source,
    },
    source_index::BuildPriority,
};

const DEFAULT_QUEUE_LIMIT: usize = 200;
const MAX_QUEUE_LIMIT: usize = 5000;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/papers/{id}/objects", get(paper_objects))
        .route(
            "/api/papers/{id}/objects/grades",
            get(read_grades).put(save_grades),
        )
        .route("/api/grading/queue", get(grading_queue))
        .route("/api/grading/next", get(grading_next))
        .layer(DefaultBodyLimit::max(MAX_GRADES_BYTES + 4_096))
}

impl AppState {
    pub async fn paper_objects(&self, id: &PaperId) -> Result<ObjectsArtifact> {
        let document = self
            .reading_index_document(id, false, BuildPriority::Interactive)
            .await?;
        let source = self.source_path(id).await?;
        load_or_build_from_source(&source, &self.store.paper_dir(id), id, &document).await
    }

    async fn known_paper(&self, id: &str) -> Result<PaperId> {
        let id = parse_id(id)?;
        if self.catalog.read().await.get(&id).is_none() {
            return Err(Error::PaperNotFound(id.to_string()));
        }
        Ok(id)
    }

    /// Papers in grading order, before any status is read.
    ///
    /// The queue file, when present, fixes the order; papers it names that are
    /// not in the catalog are skipped. Without it, the catalog's own order applies.
    /// Corpus papers without extracted metadata are named by their file name.
    async fn grading_entries(&self) -> Result<Vec<(PaperId, String, Option<String>)>> {
        let overviews = self.catalog.read().await.overviews();
        let name = |paper: &crate::domain::PaperOverview| {
            if paper.metadata.title.trim().is_empty() {
                paper.relative_path.clone()
            } else {
                paper.metadata.title.clone()
            }
        };
        let queue = grades::load_queue(self.store.root()).await?;
        Ok(match queue {
            Some(queue) => {
                let titles: std::collections::HashMap<&str, String> = overviews
                    .iter()
                    .map(|paper| (paper.id.as_str(), name(paper)))
                    .collect();
                queue
                    .papers
                    .into_iter()
                    .filter_map(|entry| {
                        let title = titles.get(entry.paper_id.as_str())?;
                        let id = parse_id(&entry.paper_id).ok()?;
                        Some((id, title.clone(), entry.stratum))
                    })
                    .collect()
            }
            None => overviews
                .iter()
                .map(|paper| (paper.id.clone(), name(paper), None))
                .collect(),
        })
    }

    /// Every paper with its status; reads one grades file per paper.
    async fn grading_order(&self) -> Result<Vec<QueueRow>> {
        let entries = self.grading_entries().await?;
        let mut rows = Vec::with_capacity(entries.len());
        for (position, (id, title, stratum)) in entries.into_iter().enumerate() {
            let status = grades::status(&self.store.paper_dir(&id)).await?;
            rows.push(QueueRow {
                paper_id: id.to_string(),
                title,
                stratum,
                status,
                position,
            });
        }
        Ok(rows)
    }

    /// First paper after `after` (wrapping) that is not complete, reading as
    /// few grades files as possible so `N` in the reader stays quick.
    async fn grading_next(&self, after: Option<&str>) -> Result<Option<(PaperId, String)>> {
        let entries = self.grading_entries().await?;
        let start = after
            .and_then(|id| {
                entries
                    .iter()
                    .position(|(entry, _, _)| entry.as_str() == id)
            })
            .map_or(0, |position| position + 1);
        for (id, title, _) in entries.iter().skip(start).chain(entries.iter().take(start)) {
            if grades::status(&self.store.paper_dir(id)).await? != GradingStatus::Complete {
                return Ok(Some((id.clone(), title.clone())));
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Debug, Serialize)]
struct QueueRow {
    paper_id: String,
    title: String,
    stratum: Option<String>,
    status: GradingStatus,
    position: usize,
}

#[derive(Debug, Serialize)]
struct QueueSummary {
    complete: usize,
    partial: usize,
    ungraded: usize,
}

#[derive(Debug, Serialize)]
struct QueueResponse {
    summary: QueueSummary,
    papers: Vec<QueueRow>,
}

#[derive(Debug, Serialize)]
struct NextResponse {
    paper_id: String,
    title: String,
}

fn reply(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({"error": kind, "message": message.into()})),
    )
        .into_response()
}

fn query_param<'a>(uri: &'a Uri, name: &str) -> Option<&'a str> {
    uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then_some(value)
    })
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

async fn read_grades(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let id = match state.known_paper(&id).await {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    match grades::load(&state.store.paper_dir(&id)).await {
        Ok(Some(grades)) => {
            ([(header::CACHE_CONTROL, "private, no-cache")], Json(grades)).into_response()
        }
        Ok(None) => reply(
            StatusCode::NOT_FOUND,
            "grades_not_found",
            "This paper has not been graded yet.",
        ),
        Err(error) => error.into_response(),
    }
}

async fn save_grades(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    request: std::result::Result<Json<ObjectGrades>, JsonRejection>,
) -> Response {
    let Json(grades) = match request {
        Ok(request) => request,
        Err(error) => return reply(StatusCode::BAD_REQUEST, "invalid_grades", error.body_text()),
    };
    let id = match state.known_paper(&id).await {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    if let Err(error) = grades.validate(id.as_str()) {
        return error.into_response();
    }
    // Spans and object ids only mean something against the exact index the
    // grader looked at; a rebuilt index invalidates them rather than drifting.
    let document = match state
        .reading_index_document(&id, false, BuildPriority::Interactive)
        .await
    {
        Ok(document) => document,
        Err(error) => return error.into_response(),
    };
    if document.etag != format!("\"{}\"", grades.index_sha256) {
        return reply(
            StatusCode::CONFLICT,
            "grades_stale_index",
            "The reading index changed since these grades were made. Reload the paper and grade again.",
        );
    }
    let expected = grades.revision.clone();
    match grades::save(&state.store.paper_dir(&id), grades, expected.as_deref()).await {
        Ok(SaveOutcome::Saved(saved)) => Json(saved).into_response(),
        Ok(SaveOutcome::Conflict(current)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "grades_conflict",
                "message": "These grades were saved elsewhere since you loaded them.",
                "current": current,
            })),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}

async fn grading_queue(State(state): State<AppState>, uri: Uri) -> Response {
    let limit = query_param(&uri, "limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_QUEUE_LIMIT)
        .clamp(1, MAX_QUEUE_LIMIT);
    let rows = match state.grading_order().await {
        Ok(rows) => rows,
        Err(error) => return error.into_response(),
    };
    let count = |status: GradingStatus| rows.iter().filter(|row| row.status == status).count();
    let summary = QueueSummary {
        complete: count(GradingStatus::Complete),
        partial: count(GradingStatus::Partial),
        ungraded: count(GradingStatus::Ungraded),
    };
    Json(QueueResponse {
        summary,
        papers: rows.into_iter().take(limit).collect(),
    })
    .into_response()
}

async fn grading_next(State(state): State<AppState>, uri: Uri) -> Response {
    // An unknown `after` starts from the top instead of failing; the grader
    // may have opened a paper outside the queue.
    let after = query_param(&uri, "after").filter(|value| !value.is_empty());
    match state.grading_next(after).await {
        Ok(Some((id, title))) => Json(NextResponse {
            paper_id: id.to_string(),
            title,
        })
        .into_response(),
        Ok(None) => reply(
            StatusCode::NOT_FOUND,
            "grading_queue_exhausted",
            "Every paper in the grading queue is complete.",
        ),
        Err(error) => error.into_response(),
    }
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

    async fn json_body(response: axum::response::Response) -> Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

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

    /// Two cached papers, no analysis, ready for grading requests.
    async fn grading_fixture() -> (tempfile::TempDir, tempfile::TempDir, AppState, Vec<PaperId>) {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let mut ids = Vec::new();
        for filename in ["Alpha - First.pdf", "Beta - Second.pdf"] {
            let source = library.path().join(filename);
            tokio::fs::write(&source, format!("cached source {filename}"))
                .await
                .unwrap();
            ids.push(PaperId::from_relative_path(std::path::Path::new(filename)));
        }
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        for (id, filename) in ids.iter().zip(["Alpha - First.pdf", "Beta - Second.pdf"]) {
            cache_fixture(&library.path().join(filename), &state.store.paper_dir(id)).await;
        }
        (library, data, state, ids)
    }

    async fn index_sha256(app: &Router, id: &PaperId) -> String {
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
        assert_eq!(response.status(), StatusCode::OK);
        let artifact = json_body(response).await;
        artifact["reading_index_generation"]
            .as_str()
            .unwrap()
            .trim_matches('"')
            .to_owned()
    }

    fn grades_json(
        id: &PaperId,
        index_sha256: &str,
        revision: Option<&str>,
        complete: bool,
    ) -> Value {
        json!({
            "schema_version": 1,
            "paper_id": id.as_str(),
            "index_sha256": index_sha256,
            "objects_generation": "detector-generation",
            "grader": "",
            "revision": revision,
            "complete": complete,
            "verdicts": {"fig-1": {"verdict": "correct", "region": null, "note": ""}},
            "additions": [{"id": "add-1", "kind": "table", "printed_label": "II", "page": 1,
                "region": {"x_min": 10.0, "y_min": 10.0, "x_max": 200.0, "y_max": 120.0},
                "caption": {"start": 0, "end": 12}, "note": ""}]
        })
    }

    fn put(id: &PaperId, body: &Value) -> Request<Body> {
        Request::builder()
            .method("PUT")
            .uri(format!("/api/papers/{id}/objects/grades"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap()
    }

    fn get_request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn should_store_grades_with_revision_when_index_matches_and_reject_stale_saves() {
        let (_library, _data, state, ids) = grading_fixture().await;
        let guard = state.tools_extract.lock().await;
        let app = crate::build_router(state.clone(), None);
        let id = &ids[0];
        let missing = app
            .clone()
            .oneshot(get_request(&format!("/api/papers/{id}/objects/grades")))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(missing).await["error"], "grades_not_found");

        let sha = index_sha256(&app, id).await;
        let stale_index = app
            .clone()
            .oneshot(put(id, &grades_json(id, &"0".repeat(64), None, false)))
            .await
            .unwrap();
        assert_eq!(stale_index.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(stale_index).await["error"], "grades_stale_index");

        let saved = app
            .clone()
            .oneshot(put(id, &grades_json(id, &sha, None, false)))
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let saved = json_body(saved).await;
        let revision = saved["revision"].as_str().unwrap().to_owned();
        assert_eq!(revision.len(), 64);
        assert!(saved["updated_at"].is_string());
        assert!(!saved["grader"].as_str().unwrap().is_empty());
        assert_eq!(saved["additions"][0]["printed_label"], "II");

        let read = app
            .clone()
            .oneshot(get_request(&format!("/api/papers/{id}/objects/grades")))
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK);
        assert_eq!(json_body(read).await["revision"], revision);

        let conflict = app
            .clone()
            .oneshot(put(id, &grades_json(id, &sha, None, true)))
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let conflict = json_body(conflict).await;
        assert_eq!(conflict["error"], "grades_conflict");
        assert_eq!(conflict["current"]["revision"], revision);

        let malformed = app
            .clone()
            .oneshot(put(
                id,
                &json!({"schema_version": 1, "paper_id": id.as_str(), "index_sha256": sha,
                "verdicts": {"fig-1": {"verdict": "region", "region": null}}, "additions": []}),
            ))
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

        let updated = app
            .clone()
            .oneshot(put(id, &grades_json(id, &sha, Some(&revision), true)))
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::OK);
        assert_eq!(json_body(updated).await["complete"], true);
        drop(guard);
    }

    #[tokio::test]
    async fn should_walk_the_grading_queue_when_papers_are_graded_in_turn() {
        let (_library, _data, state, ids) = grading_fixture().await;
        let guard = state.tools_extract.lock().await;
        let app = crate::build_router(state.clone(), None);
        let queue = json_body(
            app.clone()
                .oneshot(get_request("/api/grading/queue?limit=1"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(queue["summary"]["ungraded"], 2);
        assert_eq!(queue["papers"].as_array().unwrap().len(), 1);
        assert_eq!(queue["papers"][0]["status"], "ungraded");
        assert!(queue["papers"][0]["stratum"].is_null());

        // A queue file fixes the order and carries strata.
        let order = json!({"schema_version": 1, "papers": [
            {"paper_id": ids[1].as_str(), "arxiv_id": "2001.00002", "stratum": "cs"},
            {"paper_id": "ffffffffffffffff", "stratum": "math"},
            {"paper_id": ids[0].as_str(), "arxiv_id": "2001.00001", "stratum": "math"}]});
        tokio::fs::write(
            state.store.root().join(grades::QUEUE_FILE),
            serde_json::to_vec(&order).unwrap(),
        )
        .await
        .unwrap();
        let queue = json_body(
            app.clone()
                .oneshot(get_request("/api/grading/queue"))
                .await
                .unwrap(),
        )
        .await;
        let papers = queue["papers"].as_array().unwrap();
        assert_eq!(papers.len(), 2, "unknown queue papers are skipped");
        assert_eq!(papers[0]["paper_id"], ids[1].as_str());
        assert_eq!(papers[0]["stratum"], "cs");
        assert_eq!(papers[1]["position"], 1);

        let next = json_body(
            app.clone()
                .oneshot(get_request("/api/grading/next"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(next["paper_id"], ids[1].as_str());

        let sha = index_sha256(&app, &ids[1]).await;
        let saved = app
            .clone()
            .oneshot(put(&ids[1], &grades_json(&ids[1], &sha, None, true)))
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let next = json_body(
            app.clone()
                .oneshot(get_request(&format!("/api/grading/next?after={}", ids[1])))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(next["paper_id"], ids[0].as_str());
        let queue = json_body(
            app.clone()
                .oneshot(get_request("/api/grading/queue"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(queue["summary"]["complete"], 1);
        assert_eq!(queue["summary"]["ungraded"], 1);

        let sha = index_sha256(&app, &ids[0]).await;
        let saved = app
            .clone()
            .oneshot(put(&ids[0], &grades_json(&ids[0], &sha, None, true)))
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let exhausted = app
            .clone()
            .oneshot(get_request("/api/grading/next"))
            .await
            .unwrap();
        assert_eq!(exhausted.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            json_body(exhausted).await["error"],
            "grading_queue_exhausted"
        );
        drop(guard);
    }
}
