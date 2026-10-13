//! Edge labels in the graph API: the payload attached to each edge, and the
//! routes that accept, reject, or edit a label.

use std::{collections::HashMap, path::Path};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{post, put},
};
use chrono::{DateTime, Utc};
use lysilogy_core::{RelationshipClaimId, RelationshipOrigin, ReviewState, WorkId};
use lysilogy_db::{
    DbError,
    relationship::{StoredClaim, StoredClaimWithMention},
};
use lysilogy_ingest::labels::{GroundingRejection, LabelDraft, RELATIONS, ground_quote};
use serde::{Deserialize, Serialize};

use super::{AppState, GraphError, graph_error, graph_or_unavailable, graph_query_failure};
use crate::domain::PaperId;

/// The longest reason a person may write, as in the model grounding rule.
const MAX_REASON_CHARS: usize = 600;

/// Where a label's evidence sits in the citing paper.
#[derive(Debug, Serialize)]
pub(super) struct EdgeMention {
    pub id: lysilogy_core::MentionId,
    /// The citing paper, only when it is in the catalog. Never a path.
    pub paper_id: Option<PaperId>,
    pub page: Option<i32>,
    pub passage_start: Option<i32>,
    pub passage_end: Option<i32>,
    pub marker_start: Option<i32>,
    pub marker_end: Option<i32>,
    pub passage: Option<String>,
}

/// One current claim on an edge.
#[derive(Debug, Serialize)]
pub(super) struct EdgeLabel {
    pub id: RelationshipClaimId,
    pub relation: String,
    pub reason: String,
    pub quote: Option<String>,
    pub origin: RelationshipOrigin,
    pub review_state: ReviewState,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub mention: Option<EdgeMention>,
}

/// An edge with the evidence behind it.
#[derive(Debug, Serialize)]
pub(super) struct GraphEdge {
    pub citing: WorkId,
    pub cited: WorkId,
    /// `provider` and `local`, in that order, when present.
    pub sources: Vec<&'static str>,
    pub mention_count: usize,
    pub labels: Vec<EdgeLabel>,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/relationship-claims/{id}/accept", post(accept_label))
        .route("/api/relationship-claims/{id}/reject", post(reject_label))
        .route("/api/relationship-claims/{id}", put(edit_label))
}

/// Edge payloads for `pairs`, in order and without duplicates.
pub(super) async fn edge_payloads(
    state: &AppState,
    pairs: &[(WorkId, WorkId)],
) -> Result<Vec<GraphEdge>, GraphError> {
    let graph = graph_or_unavailable(state)?;
    let sources = graph
        .citations
        .edge_sources(pairs)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;
    let claims = graph
        .relationships
        .current_claims_for_edges(pairs)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;

    let mut labels: HashMap<(WorkId, WorkId), Vec<EdgeLabel>> = HashMap::new();
    for claim in &claims {
        let label = edge_label(state, claim).await;
        labels
            .entry((claim.citing, claim.cited))
            .or_default()
            .push(label);
    }

    let mut seen = std::collections::HashSet::new();
    let mut edges = Vec::with_capacity(pairs.len());
    for &pair in pairs {
        if !seen.insert(pair) {
            continue;
        }
        let evidence = sources.get(&pair).copied().unwrap_or_default();
        let mut kinds = Vec::new();
        if evidence.provider {
            kinds.push("provider");
        }
        if evidence.local {
            kinds.push("local");
        }
        edges.push(GraphEdge {
            citing: pair.0,
            cited: pair.1,
            sources: kinds,
            mention_count: evidence.mention_count,
            labels: labels.remove(&pair).unwrap_or_default(),
        });
    }
    Ok(edges)
}

async fn edge_label(state: &AppState, stored: &StoredClaimWithMention) -> EdgeLabel {
    let claim = &stored.claim;
    let mention = match claim.mention {
        None => None,
        Some(id) => {
            let paper_id = match &stored.document_path {
                None => None,
                Some(path) => {
                    let paper_id = PaperId::from_relative_path(Path::new(path));
                    let known = state.catalog.read().await.get(&paper_id).is_some();
                    known.then_some(paper_id)
                }
            };
            Some(EdgeMention {
                id,
                paper_id,
                page: stored.page,
                passage_start: stored.passage_start,
                passage_end: stored.passage_end,
                marker_start: stored.marker_start,
                marker_end: stored.marker_end,
                passage: stored.passage.clone(),
            })
        }
    };
    EdgeLabel {
        id: claim.id,
        relation: claim.relation.clone(),
        reason: claim.reason.clone().unwrap_or_default(),
        quote: claim.quote.clone(),
        origin: claim.origin,
        review_state: claim.review_state,
        model: claim.model.clone(),
        created_at: claim.created_at,
        mention,
    }
}

fn parse_claim_id(text: &str) -> Result<RelationshipClaimId, GraphError> {
    text.parse().map_err(|_| {
        Box::new(graph_error(
            StatusCode::BAD_REQUEST,
            "invalid_claim_id",
            "Claim identifiers look like 'r' followed by 22 characters.",
        ))
    })
}

fn claim_not_found() -> Response {
    graph_error(StatusCode::NOT_FOUND, "claim_not_found", "No such claim.")
}

fn claim_superseded() -> Response {
    graph_error(
        StatusCode::CONFLICT,
        "claim_superseded",
        "This claim was replaced by an edit; reload the graph.",
    )
}

fn claim_failure(error: &DbError) -> Response {
    if matches!(error, DbError::Conflict(_)) {
        return claim_superseded();
    }
    if error.is_not_found() {
        return claim_not_found();
    }
    graph_query_failure(error)
}

/// The updated claim as an [`EdgeLabel`], or the failure response.
async fn label_response(state: &AppState, claim: &StoredClaim) -> Response {
    let graph = match graph_or_unavailable(state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    match graph.relationships.get(claim.id).await {
        Ok(Some(stored)) => {
            (StatusCode::OK, Json(edge_label(state, &stored).await)).into_response()
        }
        Ok(None) => claim_not_found(),
        Err(error) => graph_query_failure(&error),
    }
}

async fn review(state: AppState, id: String, review_state: ReviewState) -> Response {
    let claim_id = match parse_claim_id(&id) {
        Ok(claim_id) => claim_id,
        Err(response) => return *response,
    };
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    match graph
        .relationships
        .set_review_state(claim_id, review_state)
        .await
    {
        Ok(claim) => label_response(&state, &claim).await,
        Err(error) => claim_failure(&error),
    }
}

async fn accept_label(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    review(state, id, ReviewState::Accepted).await
}

async fn reject_label(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    review(state, id, ReviewState::Rejected).await
}

#[derive(Debug, Deserialize)]
struct EditRequest {
    reason: String,
    relation: Option<String>,
    quote: Option<String>,
}

fn rejection_response(rejection: GroundingRejection) -> Response {
    let message = match rejection {
        GroundingRejection::EmptyQuote => "The quote is empty.",
        GroundingRejection::QuoteTooShort => "The quote needs at least three words.",
        GroundingRejection::QuoteNotInPassage => {
            "The quote must be copied exactly from the citing passage."
        }
        GroundingRejection::InvalidRelation => "That relation is not in the vocabulary.",
        GroundingRejection::EmptyReason => "The reason is empty.",
        GroundingRejection::ReasonTooLong => "The reason is too long.",
    };
    graph_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        rejection.as_str(),
        message,
    )
}

/// Relation and reason checks for a claim with no passage to ground in.
fn check_without_passage(relation: &str, reason: &str) -> Result<(), GroundingRejection> {
    if !RELATIONS.contains(&relation) {
        return Err(GroundingRejection::InvalidRelation);
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(GroundingRejection::EmptyReason);
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(GroundingRejection::ReasonTooLong);
    }
    Ok(())
}

async fn edit_label(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<EditRequest>,
) -> Response {
    let claim_id = match parse_claim_id(&id) {
        Ok(claim_id) => claim_id,
        Err(response) => return *response,
    };
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let existing = match graph.relationships.get(claim_id).await {
        Ok(Some(existing)) => existing,
        Ok(None) => return claim_not_found(),
        Err(error) => return graph_query_failure(&error),
    };
    if existing.claim.superseded_by.is_some() {
        return claim_superseded();
    }

    let relation = request
        .relation
        .unwrap_or_else(|| existing.claim.relation.clone());
    let reason = request.reason.trim().to_owned();
    let quote = request
        .quote
        .or_else(|| existing.claim.quote.clone())
        .map(|quote| quote.trim().to_owned())
        .filter(|quote| !quote.is_empty());
    let stored_quote = match (&quote, &existing.passage) {
        (Some(quote), Some(passage)) => {
            let draft = LabelDraft {
                relation: relation.clone(),
                reason: reason.clone(),
                quote: quote.clone(),
                passage_index: 0,
            };
            match ground_quote(std::slice::from_ref(passage), &draft) {
                Ok(_) => Some(quote.as_str()),
                Err(rejection) => return rejection_response(rejection),
            }
        }
        (Some(_), None) => return rejection_response(GroundingRejection::QuoteNotInPassage),
        (None, _) => {
            if let Err(rejection) = check_without_passage(&relation, &reason) {
                return rejection_response(rejection);
            }
            None
        }
    };

    match graph
        .relationships
        .supersede_with_user_claim(claim_id, &relation, &reason, stored_quote)
        .await
    {
        Ok(created) => label_response(&state, &created).await,
        Err(error) => claim_failure(&error),
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{DocumentFormat, ScannedFile, Visibility, WorkKind, WorkLinkMethod};
    use lysilogy_ingest::{
        labels::{LabelOptions, ScriptedLabeler},
        literature::{LiteratureStatus, LiteratureSummary},
        scan::hash_file,
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::build_router, graph::GraphState, source_index::test_support::cache_fixture_with_blocks,
    };

    fn marker() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("x{}x{nanos}", std::process::id())
    }

    async fn test_graph() -> Option<GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(GraphState::connect(&url).await.expect("connect"))
    }

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let body = body.map_or_else(Body::empty, |value| {
            builder = std::mem::take(&mut builder).header("content-type", "application/json");
            Body::from(value.to_string())
        });
        let response = build_router(state.clone(), None)
            .oneshot(builder.body(body).expect("request"))
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

    struct Fixture {
        state: AppState,
        paper_id: PaperId,
        citing: WorkId,
        cited: WorkId,
        // Keeps the library and data directories alive.
        _root: tempfile::TempDir,
    }

    /// A linked paper whose cached index cites one entry once, labeled by a
    /// scripted labeler that quotes the whole passage.
    #[allow(clippy::too_many_lines)]
    async fn labeled_fixture(graph: GraphState) -> Fixture {
        let marker = marker();
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        let data = root.path().join("data");
        std::fs::create_dir_all(&library).unwrap();
        let file_name = format!("Someone - 2020 - Paper {marker}.pdf");
        let source = library.join(&file_name);
        std::fs::write(&source, format!("not really a pdf {marker}")).unwrap();

        let state = AppState::new(&library, &data)
            .await
            .unwrap()
            .with_graph(graph);
        let paper_id = state.library().await.papers[0].id.clone();
        cache_fixture_with_blocks(
            &source,
            &state.store.paper_dir(&paper_id),
            &[
                (
                    "Prior work [1] shows reward learning from comparisons is feasible.",
                    "body",
                    1,
                ),
                ("References", "heading", 2),
                (
                    &format!("[1] Adams. Reward learning study {marker}. 2019."),
                    "body",
                    2,
                ),
            ],
        )
        .await;

        let graph = state.graph.as_ref().unwrap();
        let (sha256, bytes) = hash_file(&source).await.unwrap();
        let workspace = graph.documents.ensure_default_workspace().await.unwrap();
        graph
            .documents
            .apply_partial_scan(
                workspace,
                &[ScannedFile {
                    relative_path: file_name.clone(),
                    sha256,
                    bytes,
                    format: DocumentFormat::Pdf,
                }],
            )
            .await
            .unwrap();
        let document = graph
            .documents
            .find_by_path(workspace, &file_name)
            .await
            .unwrap()
            .unwrap();
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .unwrap();
        graph
            .documents
            .set_work_link(document.id, work.id, WorkLinkMethod::Manual, None, "test")
            .await
            .unwrap();

        let summary = LiteratureSummary {
            status: LiteratureStatus::Linked,
            document_id: document.id,
            work_id: Some(work.id),
            link_method: None,
            link_detail: None,
            openalex_id: None,
            references: 0,
            new_stub_works: 0,
            hydrated_works: 0,
            warnings: Vec::new(),
        };
        state
            .run_local_evidence(&paper_id, &summary)
            .await
            .unwrap()
            .expect("evidence");
        let labeler = ScriptedLabeler::new("fake:model", |request| {
            Ok(LabelDraft {
                relation: "background".to_owned(),
                reason: "It motivates the setup.".to_owned(),
                quote: request.passages[0].clone(),
                passage_index: 0,
            })
        });
        let labels = state
            .label_local_edges(&paper_id, &summary, &labeler, LabelOptions::default())
            .await
            .unwrap()
            .expect("labels");
        assert_eq!(labels.labeled, 1, "{labels:?}");

        let cited = graph.citations.outgoing(work.id).await.unwrap()[0].id;
        Fixture {
            state,
            paper_id,
            citing: work.id,
            cited,
            _root: root,
        }
    }

    async fn first_label(fixture: &Fixture) -> Value {
        let uri = format!(
            "/api/graph/neighbors/{}?direction=references",
            fixture.citing
        );
        let (status, json) = send(&fixture.state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        json["edges"][0]["labels"][0].clone()
    }

    #[tokio::test]
    async fn should_include_labels_and_sources_in_neighbor_edges_when_local_evidence_exists() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let uri = format!(
            "/api/graph/neighbors/{}?direction=references",
            fixture.citing
        );
        let (status, json) = send(&fixture.state, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        let edge = &json["edges"][0];
        assert_eq!(edge["cited"], fixture.cited.to_string());
        assert_eq!(edge["sources"], json!(["local"]));
        assert_eq!(edge["mention_count"], 1);
        let label = &edge["labels"][0];
        assert_eq!(label["relation"], "background");
        assert_eq!(label["reason"], "It motivates the setup.");
        assert_eq!(label["origin"], "ai");
        assert_eq!(label["review_state"], "unreviewed");
        assert_eq!(label["model"], "fake:model");
        assert_eq!(label["mention"]["paper_id"], fixture.paper_id.to_string());
        assert_eq!(label["mention"]["page"], 1);
        let passage = label["mention"]["passage"].as_str().expect("passage");
        assert_eq!(label["quote"], passage);
        assert!(
            label["mention"]["passage_end"].as_i64() > label["mention"]["passage_start"].as_i64()
        );
        assert!(
            !json.to_string().contains("Someone - 2020"),
            "paths must not leak: {json}"
        );
    }

    #[tokio::test]
    async fn should_include_edges_in_citations_response_when_work_has_labels() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let (status, json) = send(
            &fixture.state,
            "GET",
            &format!("/api/works/{}/citations", fixture.citing),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let edges = json["edges"].as_array().expect("edges");
        assert_eq!(edges.len(), 1, "{json}");
        assert_eq!(edges[0]["labels"].as_array().map(Vec::len), Some(1));
        let (_, cited_side) = send(
            &fixture.state,
            "GET",
            &format!("/api/works/{}/citations", fixture.cited),
            None,
        )
        .await;
        assert_eq!(
            cited_side["edges"][0]["citing"],
            fixture.citing.to_string(),
            "{cited_side}"
        );
    }

    #[tokio::test]
    async fn should_accept_label_when_posting_accept() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let label = first_label(&fixture).await;
        let id = label["id"].as_str().unwrap();
        let (status, json) = send(
            &fixture.state,
            "POST",
            &format!("/api/relationship-claims/{id}/accept"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["id"], label["id"]);
        assert_eq!(json["review_state"], "accepted");
        assert_eq!(json["mention"]["paper_id"], fixture.paper_id.to_string());
        assert_eq!(first_label(&fixture).await["review_state"], "accepted");

        let (status, json) = send(
            &fixture.state,
            "POST",
            &format!("/api/relationship-claims/{id}/reject"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["review_state"], "rejected");
        assert_eq!(first_label(&fixture).await["review_state"], "rejected");
    }

    #[tokio::test]
    async fn should_return_404_when_accepting_unknown_claim() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let unknown = RelationshipClaimId::new();
        let (status, json) = send(
            &fixture.state,
            "POST",
            &format!("/api/relationship-claims/{unknown}/accept"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "claim_not_found");
        let (status, _) = send(
            &fixture.state,
            "PUT",
            &format!("/api/relationship-claims/{unknown}"),
            Some(json!({"reason": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn should_return_400_when_claim_id_malformed() {
        let fixture = unbacked_state().await;
        for uri in [
            "/api/relationship-claims/not-an-id/accept",
            "/api/relationship-claims/not-an-id/reject",
        ] {
            let (status, json) = send(&fixture, "POST", uri, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(json["error"], "invalid_claim_id");
        }
        let (status, json) = send(
            &fixture,
            "PUT",
            "/api/relationship-claims/not-an-id",
            Some(json!({"reason": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid_claim_id");
    }

    #[tokio::test]
    async fn should_return_503_when_reviewing_without_graph() {
        let state = unbacked_state().await;
        let claim = RelationshipClaimId::new();
        let (status, json) = send(
            &state,
            "POST",
            &format!("/api/relationship-claims/{claim}/accept"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "graph_unavailable");
    }

    async fn unbacked_state() -> AppState {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        let state = AppState::new(&library, root.path().join("data"))
            .await
            .unwrap();
        // The state only needs to outlive the directory listing above.
        std::mem::forget(root);
        state
    }

    #[tokio::test]
    async fn should_return_409_when_reviewing_superseded_claim() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let label = first_label(&fixture).await;
        let old = label["id"].as_str().unwrap().to_owned();
        let (status, _) = send(
            &fixture.state,
            "PUT",
            &format!("/api/relationship-claims/{old}"),
            Some(json!({"reason": "A better reason."})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for action in ["accept", "reject"] {
            let (status, json) = send(
                &fixture.state,
                "POST",
                &format!("/api/relationship-claims/{old}/{action}"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{action}");
            assert_eq!(json["error"], "claim_superseded");
        }
        let (status, json) = send(
            &fixture.state,
            "PUT",
            &format!("/api/relationship-claims/{old}"),
            Some(json!({"reason": "Again."})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], "claim_superseded");
    }

    #[tokio::test]
    async fn should_return_422_when_edited_quote_not_in_passage() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let label = first_label(&fixture).await;
        let id = label["id"].as_str().unwrap();
        let uri = format!("/api/relationship-claims/{id}");
        let cases = [
            (
                json!({"reason": "r", "quote": "words that never appear anywhere"}),
                "quote_not_in_passage",
            ),
            (json!({"reason": "r", "quote": "[1]"}), "quote_too_short"),
            (json!({"reason": "   "}), "empty_reason"),
            (
                json!({"reason": "r", "relation": "friendship"}),
                "invalid_relation",
            ),
            (json!({"reason": "x".repeat(601)}), "reason_too_long"),
        ];
        for (body, code) in cases {
            let (status, json) = send(&fixture.state, "PUT", &uri, Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{code}: {json}");
            assert_eq!(json["error"], code);
        }
        // Nothing was superseded by the rejected edits.
        assert_eq!(first_label(&fixture).await["id"], label["id"]);
    }

    #[tokio::test]
    async fn should_supersede_and_return_user_claim_when_editing() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = labeled_fixture(graph).await;
        let label = first_label(&fixture).await;
        let id = label["id"].as_str().unwrap();
        let (status, edited) = send(
            &fixture.state,
            "PUT",
            &format!("/api/relationship-claims/{id}"),
            Some(json!({
                "reason": "  It is the motivating prior result.  ",
                "relation": "supports",
                "quote": "reward learning from comparisons is feasible",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{edited}");
        assert_ne!(edited["id"], label["id"]);
        assert_eq!(edited["origin"], "user");
        assert_eq!(edited["review_state"], "accepted");
        assert_eq!(edited["relation"], "supports");
        assert_eq!(edited["reason"], "It is the motivating prior result.");
        assert_eq!(
            edited["quote"],
            "reward learning from comparisons is feasible"
        );
        assert_eq!(edited["mention"]["id"], label["mention"]["id"]);

        let current = first_label(&fixture).await;
        assert_eq!(current["id"], edited["id"]);
        let (_, json) = send(
            &fixture.state,
            "GET",
            &format!("/api/graph/neighbors/{}", fixture.citing),
            None,
        )
        .await;
        assert_eq!(
            json["edges"][0]["labels"].as_array().map(Vec::len),
            Some(1),
            "{json}"
        );
    }

    #[tokio::test]
    async fn should_follow_redirect_when_work_was_merged() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let from = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .unwrap();
        let into = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .unwrap();
        graph
            .works
            .merge_work(from.id, into.id, "test", "test")
            .await
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        let state = AppState::new(&library, root.path().join("data"))
            .await
            .unwrap()
            .with_graph(graph);
        let (status, json) = send(&state, "GET", &format!("/api/works/{}", from.id), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["id"], into.id.to_string());
        let (_, json) = send(
            &state,
            "GET",
            &format!("/api/works/{}/citations", from.id),
            None,
        )
        .await;
        assert_eq!(json["work"]["id"], into.id.to_string());
        let (_, json) = send(
            &state,
            "GET",
            &format!("/api/graph/neighbors/{}", from.id),
            None,
        )
        .await;
        assert_eq!(json["center"], into.id.to_string());
    }
}
