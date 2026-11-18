//! Review routes: the rubric, the evaluation set and its progress, one
//! paper's review of its current output, and aggregates over every review.

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use serde::Serialize;

use super::{AppState, parse_id};
use crate::{
    Error, Result,
    domain::PaperId,
    review::{
        MANIFEST_JSON, Manifest, RUBRIC_JSON, Review, ReviewInput, ReviewSnapshot, ReviewSubject,
        Rubric, load_all_reviews, load_analysis_json, load_paper_reviews, load_review,
        manifest_match, pin_edge_labels, progress, save_review, snapshot_from_analysis, summarize,
    },
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/review/rubric", get(rubric))
        .route("/api/review/manifest", get(manifest))
        .route("/api/review/progress", get(review_progress))
        .route("/api/reviews", get(list))
        .route("/api/reviews/summary", get(summary))
        .route("/api/papers/{id}/review", get(read).put(save))
}

/// A paper's review state: the output it shows now, the review of that
/// output if one exists, and earlier reviews of output since replaced.
#[derive(Serialize)]
struct PaperReview {
    paper_id: PaperId,
    paper_title: String,
    manifest_id: Option<String>,
    /// `None` until the paper has been analysed.
    snapshot: Option<ReviewSnapshot>,
    review: Option<Review>,
    earlier: Vec<EarlierReview>,
}

#[derive(Serialize)]
struct EarlierReview {
    snapshot_id: String,
    generated_at: Option<String>,
    pipeline_version: Option<String>,
    updated_at: chrono::DateTime<Utc>,
    overall: Option<u32>,
}

fn raw_json(json: &'static str) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}

async fn rubric() -> Response {
    raw_json(RUBRIC_JSON)
}

async fn manifest() -> Response {
    raw_json(MANIFEST_JSON)
}

async fn review_progress(State(state): State<AppState>) -> Result<Response> {
    let papers = state.catalog.read().await.overviews();
    let rows = progress(&state.store, &Manifest::bundled()?, &papers).await?;
    Ok(Json(rows).into_response())
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<Review>>> {
    Ok(Json(load_all_reviews(&state.store).await?))
}

async fn summary(State(state): State<AppState>) -> Result<Response> {
    let rubric = Rubric::bundled()?;
    let reviews = load_all_reviews(&state.store).await?;
    let mut summary = summarize(&rubric, &reviews);
    if let Some(graph) = &state.graph {
        summary.label_actions = crate::review::label_actions(graph).await;
    }
    Ok(Json(summary).into_response())
}

async fn subject(state: &AppState, id: &str) -> Result<ReviewSubject> {
    let id = parse_id(id)?;
    let overview = state
        .catalog
        .read()
        .await
        .get(&id)
        .map(|entry| entry.overview.clone())
        .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
    let manifest = Manifest::bundled()?;
    Ok(ReviewSubject {
        manifest_id: manifest_match(&manifest, &overview).map(|entry| entry.id.clone()),
        paper_id: id,
        paper_title: overview.metadata.title,
    })
}

/// The output the paper shows now, with a sample of its citation labels
/// pinned beside the analysis fields (not part of the snapshot id).
async fn current_snapshot(state: &AppState, id: &PaperId) -> Result<Option<ReviewSnapshot>> {
    let Some(mut snapshot) = load_analysis_json(&state.store, id)
        .await?
        .as_ref()
        .map(snapshot_from_analysis)
    else {
        return Ok(None);
    };
    let run = state.load_edge_label_run(id).await?;
    pin_edge_labels(&mut snapshot, run.as_ref());
    Ok(Some(snapshot))
}

async fn read(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Result<Response> {
    let subject = subject(&state, &id).await?;
    let snapshot = current_snapshot(&state, &subject.paper_id).await?;
    let mut review = None;
    let mut earlier = Vec::new();
    for stored in load_paper_reviews(&state.store, &subject.paper_id).await? {
        if snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.snapshot_id == stored.snapshot.snapshot_id)
        {
            review = Some(stored);
        } else {
            earlier.push(EarlierReview {
                snapshot_id: stored.snapshot.snapshot_id,
                generated_at: stored.snapshot.generated_at,
                pipeline_version: stored.snapshot.pipeline_version,
                updated_at: stored.updated_at,
                overall: stored.overall,
            });
        }
    }
    Ok(Json(PaperReview {
        paper_id: subject.paper_id,
        paper_title: subject.paper_title,
        manifest_id: subject.manifest_id,
        snapshot,
        review,
        earlier,
    })
    .into_response())
}

async fn save(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(input): Json<ReviewInput>,
) -> Result<Response> {
    let subject = subject(&state, &id).await?;
    let rubric = Rubric::bundled()?;
    rubric.validate(&input)?;
    let Some(snapshot) = current_snapshot(&state, &subject.paper_id).await? else {
        return Err(Error::InvalidRequest(
            "analyze this paper before reviewing it".to_owned(),
        ));
    };
    if snapshot.snapshot_id != input.snapshot_id {
        return Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "stale_snapshot",
                "message": "This paper was analysed again since the review opened. Reopen the review to grade the new output; the earlier review is kept.",
                "snapshot_id": snapshot.snapshot_id,
            })),
        )
            .into_response());
    }
    let previous = load_review(&state.store, &subject.paper_id, &snapshot.snapshot_id).await?;
    // The labels graded are the ones pinned at the first save, even if a
    // labelling run or a review in the graph has changed them since.
    let mut snapshot = snapshot;
    if let Some(pinned) = previous
        .as_ref()
        .and_then(|review| review.snapshot.fields.get(crate::review::EDGE_LABELS_FIELD))
    {
        snapshot
            .fields
            .insert(crate::review::EDGE_LABELS_FIELD.to_owned(), pinned.clone());
    }
    let review = Review::from_input(
        &rubric,
        subject,
        snapshot,
        input,
        previous.as_ref(),
        Utc::now(),
    );
    save_review(&state.store, &review).await?;
    Ok(Json(review).into_response())
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

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().uri(uri).method(method);
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn should_save_a_review_against_the_current_output_and_summarise_it() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let filename = "autor2013-china-syndrome.pdf";
        tokio::fs::write(library.path().join(filename), b"discovery fixture")
            .await
            .unwrap();
        let id = PaperId::from_relative_path(std::path::Path::new(filename));
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let app = crate::build_router(state.clone(), None);
        let uri = format!("/api/papers/{id}/review");

        let (status, rubric) = call(&app, "GET", "/api/review/rubric", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(rubric["components"][2]["id"], "context");
        assert_eq!(rubric["components"][5]["id"], "edge_labels");

        let (status, before) = call(&app, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(before["snapshot"].is_null());
        assert_eq!(before["manifest_id"], "autor2013");
        let (status, _) = call(&app, "PUT", &uri, Some(json!({"snapshot_id": "s-0"}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let paper_dir = state.store.paper_dir(&id);
        tokio::fs::create_dir_all(&paper_dir).await.unwrap();
        let analysis = json!({"generated_at": "2026-11-01T00:00:00Z", "thesis": "Trade shocks are local.",
            "pipeline_version": "core-v4", "outline": [{"title": "I. Introduction"}]});
        tokio::fs::write(paper_dir.join("analysis.json"), analysis.to_string())
            .await
            .unwrap();
        let (_, opened) = call(&app, "GET", &uri, None).await;
        let snapshot_id = opened["snapshot"]["snapshot_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            opened["snapshot"]["fields"]["outline"][0]["title"],
            "I. Introduction"
        );
        assert!(opened["review"].is_null());

        let grades = json!({"snapshot_id": snapshot_id, "overall": 3,
            "components": {"context": {"values": {"accuracy": 4, "orientation": 3}}},
            "issues": [{"id": "i1", "component": "outline", "severity": "minor", "note": "Running head listed."}]});
        let (status, saved) = call(&app, "PUT", &uri, Some(grades.clone())).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["snapshot"]["pipeline_version"], "core-v4");

        let (status, _) = call(
            &app,
            "PUT",
            &uri,
            Some(json!({"snapshot_id": snapshot_id, "overall": 7})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, conflict) =
            call(&app, "PUT", &uri, Some(json!({"snapshot_id": "s-0000"}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(conflict["error"], "stale_snapshot");

        let (_, reopened) = call(&app, "GET", &uri, None).await;
        assert_eq!(
            reopened["review"]["components"]["context"]["values"]["accuracy"],
            4
        );
        let (_, list) = call(&app, "GET", "/api/reviews", None).await;
        assert_eq!(list.as_array().map(Vec::len), Some(1));
        let (_, summary) = call(&app, "GET", "/api/reviews/summary", None).await;
        assert_eq!(summary["groups"][0]["pipeline_version"], "core-v4");
        assert_eq!(
            summary["groups"][0]["components"]["context"]["dimensions"]["accuracy"]["mean"],
            4.0
        );
        let (_, progress) = call(&app, "GET", "/api/review/progress", None).await;
        let autor = progress
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["manifest_id"] == "autor2013")
            .unwrap();
        assert_eq!(autor["reviewed"], true);

        // Re-analysis moves the review to the earlier list.
        let rerun =
            json!({"generated_at": "2026-11-09T00:00:00Z", "thesis": "Trade shocks are local."});
        tokio::fs::write(paper_dir.join("analysis.json"), rerun.to_string())
            .await
            .unwrap();
        let (_, after) = call(&app, "GET", &uri, None).await;
        assert!(after["review"].is_null());
        assert_eq!(after["earlier"][0]["pipeline_version"], "core-v4");
        let (status, _) = call(&app, "GET", "/api/papers/1234567890abcdef/review", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    fn run_with_label(reason: &str) -> Value {
        json!({
            "generated_at": "2026-11-18T00:00:00Z",
            "status": "completed",
            "labeler": "openrouter:example/model",
            "prompt_version": 2,
            "counts": {"edges": 3, "labelled": 2, "unlabelled_with_passages": 1, "perfunctory": 1},
            "core": {"unresolved": {"no_matching_entry": 2}},
            "labels": {"over_budget": 1},
            "edges": [
                {"work_id": "w1", "title": "A reef survey", "mention_count": 2,
                 "label": {"claim_id": "r1", "relation": "uses_method", "reason": reason,
                           "quote": "we use their index", "path": "edge_labeler",
                           "engagement": "substantive", "review_state": "unreviewed"}},
                {"work_id": "w2", "title": "Unlabelled", "mention_count": 1, "label": null}
            ]
        })
    }

    #[tokio::test]
    async fn should_pin_the_citation_labels_a_review_graded_and_group_them_by_label_version() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        tokio::fs::write(library.path().join("paper.pdf"), b"fixture").await.unwrap();
        let id = PaperId::from_relative_path(std::path::Path::new("paper.pdf"));
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let app = crate::build_router(state.clone(), None);
        let uri = format!("/api/papers/{id}/review");
        let paper_dir = state.store.paper_dir(&id);
        tokio::fs::create_dir_all(&paper_dir).await.unwrap();
        tokio::fs::write(
            paper_dir.join("analysis.json"),
            json!({"generated_at": "2026-11-01T00:00:00Z", "thesis": "T.", "pipeline_version": "core-v9"}).to_string(),
        )
        .await
        .unwrap();
        let (_, without) = call(&app, "GET", &uri, None).await;
        assert!(without["snapshot"]["fields"]["edge_labels"].is_null(), "no run, nothing pinned");
        let snapshot_id = without["snapshot"]["snapshot_id"].clone();

        let write_run = |reason: &str| {
            let path = paper_dir.join(crate::edge_labels::EDGE_LABELS_FILE);
            let run = run_with_label(reason).to_string();
            async move { tokio::fs::write(path, run).await.unwrap() }
        };
        write_run("Uses their bleaching index as the outcome.").await;
        let (_, opened) = call(&app, "GET", &uri, None).await;
        assert_eq!(opened["snapshot"]["snapshot_id"], snapshot_id, "labels do not change the snapshot id");
        let pinned = &opened["snapshot"]["fields"]["edge_labels"];
        assert_eq!(pinned["label_version"], "openrouter:example/model @ prompt v2");
        assert_eq!(pinned["sample"].as_array().map(Vec::len), Some(1), "only labelled edges are sampled");
        assert_eq!(pinned["unresolved_keys"], 2);
        assert_eq!(pinned["left_for_later"], 1);

        let grades = json!({"snapshot_id": snapshot_id, "components": {"edge_labels": {"values": {
            "sampled": 1, "wrong_relation": 0, "weak_reason": 1, "specificity": 2}}}});
        let (status, saved) = call(&app, "PUT", &uri, Some(grades.clone())).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(
            saved["snapshot"]["fields"]["edge_labels"]["sample"][0]["label"]["reason"],
            "Uses their bleaching index as the outcome."
        );

        // A later labelling run does not move the grades onto other labels.
        write_run("A rewritten reason.").await;
        let (_, resaved) = call(&app, "PUT", &uri, Some(grades)).await;
        assert_eq!(
            resaved["snapshot"]["fields"]["edge_labels"]["sample"][0]["label"]["reason"],
            "Uses their bleaching index as the outcome."
        );
        let (_, summary) = call(&app, "GET", "/api/reviews/summary", None).await;
        let group = &summary["label_versions"][0];
        assert_eq!(group["label_version"], "openrouter:example/model @ prompt v2");
        assert_eq!(group["component"]["dimensions"]["weak_reason"]["total"], 1);
        assert_eq!(summary["label_actions"], json!([]), "no graph, no actions");
    }
}
