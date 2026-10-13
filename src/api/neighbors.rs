//! Ranked neighborhood of a Work: the graph overlay's fanout.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, Utc};
use lysilogy_core::{ProminenceSignals, TagId, Work, WorkId};
use serde::{Deserialize, Serialize};

use super::{
    AppState, GraphError,
    acquisition::{self, AcquisitionStates},
    curation::{self, Marks, WorkTagIds},
    fetch_work, graph_error, graph_or_unavailable, graph_query_failure, labels, literature,
    parse_work_id,
};
use crate::expansion::ExpansionView;
use crate::graph::GraphState;
use crate::relevance::{Cluster, NeighborCandidate, RankingRequest, TagRef, rank_neighborhood};

/// Largest accepted `limit`.
const LIMIT_MAX: usize = 500;
/// Most `expand` keys per request, and the longest key.
const EXPAND_KEYS_MAX: usize = 32;
const EXPAND_KEY_MAX_CHARS: usize = 64;
/// Most tag ids in one `tags` filter.
const TAG_FILTER_MAX: usize = 16;

/// Parameters are strings so a bad value is our 400, not Axum's rejection.
#[derive(Debug, Deserialize)]
pub(super) struct NeighborsQuery {
    direction: Option<String>,
    limit: Option<String>,
    expand: Option<String>,
    tags: Option<String>,
}

#[derive(Debug, Serialize)]
struct Ranking {
    limit: Option<usize>,
    total: usize,
    visible: usize,
    pinned: usize,
    hidden: usize,
    filtered_out: usize,
}

#[derive(Debug, Serialize)]
struct ProminenceView {
    score: f64,
    signals: ProminenceSignals,
}

#[derive(Debug, Serialize)]
struct NeighborsResponse {
    center: WorkId,
    direction: &'static str,
    nodes: Vec<Work>,
    edges: Vec<labels::GraphEdge>,
    local_papers: literature::LocalPapers,
    acquisitions: AcquisitionStates,
    ranking: Ranking,
    prominence: BTreeMap<String, ProminenceView>,
    marks: Marks,
    work_tags: WorkTagIds,
    clusters: Vec<Cluster>,
    expansions: BTreeMap<String, ExpansionView>,
}

#[allow(clippy::unnecessary_box_returns)]
fn bad_request(code: &str, message: &str) -> GraphError {
    Box::new(graph_error(StatusCode::BAD_REQUEST, code, message))
}

/// `limit`: absent, or an integer in `1..=500` with no surrounding whitespace or sign.
fn parse_limit(text: Option<&str>) -> Result<Option<usize>, GraphError> {
    let Some(text) = text else {
        return Ok(None);
    };
    let invalid = || bad_request("invalid_limit", "limit must be an integer from 1 to 500.");
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    match text.parse::<usize>() {
        Ok(limit) if (1..=LIMIT_MAX).contains(&limit) => Ok(Some(limit)),
        _ => Err(invalid()),
    }
}

/// `expand`: comma-separated cluster keys; empty segments are ignored.
fn parse_expand(text: Option<&str>) -> Result<HashSet<String>, GraphError> {
    let keys: Vec<&str> = text
        .unwrap_or("")
        .split(',')
        .filter(|key| !key.is_empty())
        .collect();
    if keys.len() > EXPAND_KEYS_MAX
        || keys
            .iter()
            .any(|key| key.chars().count() > EXPAND_KEY_MAX_CHARS)
    {
        return Err(bad_request(
            "invalid_expand",
            "expand takes at most 32 cluster keys of at most 64 characters each.",
        ));
    }
    Ok(keys.into_iter().map(str::to_owned).collect())
}

/// `tags`: comma-separated tag ids; `None` when no filter is requested.
fn parse_tag_filter(text: Option<&str>) -> Result<Option<HashSet<TagId>>, GraphError> {
    let segments: Vec<&str> = text
        .unwrap_or("")
        .split(',')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return Ok(None);
    }
    let invalid = || {
        bad_request(
            "invalid_tag_id",
            "tags takes at most 16 comma-separated tag identifiers ('t' followed by 22 characters).",
        )
    };
    if segments.len() > TAG_FILTER_MAX {
        return Err(invalid());
    }
    segments
        .into_iter()
        .map(|segment| segment.parse::<TagId>().map_err(|_| invalid()))
        .collect::<Result<HashSet<_>, _>>()
        .map(Some)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NeighborDirection {
    References,
    Citations,
    Both,
}

impl NeighborDirection {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value.unwrap_or("references") {
            "references" => Some(Self::References),
            "citations" => Some(Self::Citations),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::References => "references",
            Self::Citations => "citations",
            Self::Both => "both",
        }
    }
}

struct Params {
    direction: NeighborDirection,
    limit: Option<usize>,
    expand: HashSet<String>,
    tag_filter: Option<HashSet<TagId>>,
}

fn parse_params(query: &NeighborsQuery) -> Result<Params, GraphError> {
    let direction = NeighborDirection::parse(query.direction.as_deref()).ok_or_else(|| {
        bad_request(
            "invalid_direction",
            "direction must be references, citations, or both.",
        )
    })?;
    Ok(Params {
        direction,
        limit: parse_limit(query.limit.as_deref())?,
        expand: parse_expand(query.expand.as_deref())?,
        tag_filter: parse_tag_filter(query.tags.as_deref())?,
    })
}

/// Distinct neighbors in citation order, and the citation pairs touching `center`.
async fn collect_neighbors(
    graph: &GraphState,
    center: WorkId,
    direction: NeighborDirection,
) -> Result<(Vec<Work>, Vec<(WorkId, WorkId)>), GraphError> {
    let mut neighbors: Vec<Work> = Vec::new();
    let mut seen = HashSet::from([center]);
    let mut pairs = Vec::new();
    let query_failure = |error: lysilogy_db::DbError| Box::new(graph_query_failure(&error));
    if matches!(
        direction,
        NeighborDirection::References | NeighborDirection::Both
    ) {
        for neighbor in graph
            .citations
            .outgoing(center)
            .await
            .map_err(query_failure)?
        {
            let cited = neighbor.id;
            if seen.insert(cited) {
                neighbors.push(neighbor);
            }
            pairs.push((center, cited));
        }
    }
    if matches!(
        direction,
        NeighborDirection::Citations | NeighborDirection::Both
    ) {
        for neighbor in graph
            .citations
            .incoming(center)
            .await
            .map_err(query_failure)?
        {
            let citing = neighbor.id;
            if seen.insert(citing) {
                neighbors.push(neighbor);
            }
            pairs.push((citing, center));
        }
    }
    Ok((neighbors, pairs))
}

pub(super) async fn get_graph_neighbors(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<NeighborsQuery>,
) -> Response {
    match neighbors_response(&state, &id, &query).await {
        Ok(response) | Err(response) => *response,
    }
}

async fn expansions_for(
    state: &AppState,
    graph: &GraphState,
    works: &[WorkId],
) -> Result<BTreeMap<String, ExpansionView>, GraphError> {
    state
        .expansion_views_raw(graph, works)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))
}

async fn neighbors_response(
    state: &AppState,
    id: &str,
    query: &NeighborsQuery,
) -> Result<GraphError, GraphError> {
    let work_id = parse_work_id(id)?;
    let params = parse_params(query)?;
    let graph = graph_or_unavailable(state)?;
    let center = fetch_work(graph, work_id).await?;
    let work_id = center.id;
    let (neighbors, pairs) = collect_neighbors(graph, work_id, params.direction).await?;

    let neighbor_ids: Vec<WorkId> = neighbors.iter().map(|work| work.id).collect();
    let rows = curation::curation_rows(state, &neighbor_ids).await?;
    let prominence = graph
        .prominence
        .prominence_for(rows.workspace, &neighbor_ids, Utc::now().year())
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;

    let tag_refs: Vec<Vec<TagRef>> = neighbors
        .iter()
        .map(|work| {
            rows.tags.get(&work.id).map_or_else(Vec::new, |tags| {
                tags.iter()
                    .map(|tag| TagRef {
                        id: tag.id,
                        name: tag.name.clone(),
                    })
                    .collect()
            })
        })
        .collect();
    let candidates: Vec<NeighborCandidate<'_>> = neighbors
        .iter()
        .zip(&tag_refs)
        .map(|(work, tags)| NeighborCandidate {
            work,
            score: prominence.get(&work.id).map_or(0.0, |row| row.score),
            mark: rows.marks.get(&work.id).map(|stored| stored.mark),
            tags,
        })
        .collect();
    let ranked = rank_neighborhood(
        work_id,
        &candidates,
        &RankingRequest {
            limit: params.limit,
            expand: &params.expand,
            tag_filter: params.tag_filter.as_ref(),
        },
    );

    let mut by_id: HashMap<WorkId, Work> = neighbors
        .iter()
        .cloned()
        .map(|work| (work.id, work))
        .collect();
    let mut nodes = vec![center];
    nodes.extend(ranked.visible.iter().filter_map(|id| by_id.remove(id)));
    let shown_ids: Vec<WorkId> = nodes.iter().map(|work| work.id).collect();
    let shown: HashSet<WorkId> = shown_ids.iter().copied().collect();

    let shown_pairs: Vec<(WorkId, WorkId)> = pairs
        .into_iter()
        .filter(|(citing, cited)| shown.contains(citing) && shown.contains(cited))
        .collect();
    let edges = labels::edge_payloads(state, &shown_pairs).await?;
    let local_papers = literature::local_papers_for(state, &shown_ids).await?;
    let acquisitions = acquisition::states_for(state, &shown_ids).await?;
    let expansions = expansions_for(state, graph, &shown_ids).await?;
    let prominence_payload = shown_ids
        .iter()
        .filter_map(|id| {
            let row = prominence.get(id)?;
            Some((
                id.to_string(),
                ProminenceView {
                    score: row.score,
                    signals: row.signals,
                },
            ))
        })
        .collect();
    let body = NeighborsResponse {
        center: work_id,
        direction: params.direction.as_str(),
        nodes,
        edges,
        local_papers,
        acquisitions,
        ranking: Ranking {
            limit: params.limit,
            total: ranked.total,
            visible: ranked.visible.len(),
            pinned: ranked.pinned,
            hidden: ranked.hidden,
            filtered_out: ranked.filtered_out,
        },
        prominence: prominence_payload,
        marks: rows.marks_payload(&shown_ids),
        work_tags: rows.work_tags_payload(&shown_ids),
        clusters: ranked.clusters,
        expansions,
    };
    Ok(Box::new((StatusCode::OK, Json(body)).into_response()))
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::{
        Coverage, ExpansionMethod, ExpansionState, ExpansionUnavailableReason, TagColor, TagKind,
        TagName, Visibility, WorkKind, WorkMark, WorkspaceId,
    };
    use lysilogy_db::expansion::NewExpansion;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::{api::build_router, graph::GraphState};

    async fn test_graph() -> Option<GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(GraphState::connect(&url).await.expect("connect"))
    }

    struct Fixture {
        _library: tempfile::TempDir,
        _data: tempfile::TempDir,
        state: AppState,
    }

    async fn fixture(graph: Option<GraphState>) -> Fixture {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let mut state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        if let Some(graph) = graph {
            state = state.with_graph(graph);
        }
        Fixture {
            _library: library,
            _data: data,
            state,
        }
    }

    async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
        let response = build_router(state.clone(), None)
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
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

    /// A center Work citing `count` new Works, each given a prominence score
    /// (`score_of(n)`) and year (`1980 + n`), plus the workspace.
    struct Seeded {
        graph: GraphState,
        workspace: WorkspaceId,
        center: WorkId,
        cited: Vec<WorkId>,
    }

    async fn seed(graph: GraphState, count: usize, score_of: impl Fn(usize) -> f64) -> Seeded {
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let center = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("center")
            .id;
        let source = graph
            .source_records
            .upsert(
                "test",
                &uuid::Uuid::new_v4().to_string(),
                Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("source");
        let mut cited = Vec::new();
        let mut scored = Vec::new();
        for n in 0..count {
            let work = graph
                .works
                .create(WorkKind::Article, Visibility::Private)
                .await
                .expect("work")
                .id;
            graph
                .citations
                .add_assertion(center, work, Some(source))
                .await
                .expect("assertion");
            let signals = ProminenceSignals {
                display_year: Some(1980 + i32::try_from(n).unwrap_or(0)),
                ..ProminenceSignals::default()
            };
            scored.push((work, signals, score_of(n)));
            cited.push(work);
        }
        graph
            .prominence
            .store(workspace, &scored)
            .await
            .expect("store");
        Seeded {
            graph,
            workspace,
            center,
            cited,
        }
    }

    fn node_ids(json: &Value) -> Vec<String> {
        json["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .map(|node| node["id"].as_str().expect("id").to_owned())
            .collect()
    }

    fn cluster_count(json: &Value) -> u64 {
        json["clusters"]
            .as_array()
            .expect("clusters")
            .iter()
            .map(|cluster| cluster["count"].as_u64().expect("count"))
            .sum()
    }

    fn ledger(work_id: WorkId, state: ExpansionState) -> NewExpansion {
        NewExpansion {
            work_id,
            state,
            method: (state == ExpansionState::Expanded).then_some(ExpansionMethod::Provider),
            unavailable_reason: (state == ExpansionState::Unavailable)
                .then_some(ExpansionUnavailableReason::ProviderNotFound),
            provider_key: None,
            source_record_id: None,
            references_count: if state == ExpansionState::Expanded {
                7
            } else {
                0
            },
            new_works: 0,
            warnings: Vec::new(),
        }
    }

    #[tokio::test]
    async fn should_include_expansions_for_center_and_visible_nodes_when_reading_neighbors() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let seeded = seed(graph, 4, |n| [0.9, 0.8, 0.7, 0.6][n]).await;
        let graph = &seeded.graph;
        let [expanded, unavailable, known, unknown] = [
            seeded.cited[0],
            seeded.cited[1],
            seeded.cited[2],
            seeded.cited[3],
        ];
        graph
            .expansions
            .record(ledger(seeded.center, ExpansionState::Expanded))
            .await
            .expect("center row");
        graph
            .expansions
            .record(ledger(expanded, ExpansionState::Expanded))
            .await
            .expect("expanded row");
        graph
            .expansions
            .record(ledger(unavailable, ExpansionState::Unavailable))
            .await
            .expect("unavailable row");
        let grandchild = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("grandchild")
            .id;
        graph
            .citations
            .add_assertion(known, grandchild, None)
            .await
            .expect("known fanout");

        let fixture = fixture(Some(seeded.graph.clone())).await;
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}", seeded.center),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let views = &json["expansions"];
        assert_eq!(
            views[seeded.center.to_string()]["state"],
            "expanded",
            "{json}"
        );
        let view = &views[expanded.to_string()];
        assert_eq!(view["state"], "expanded");
        assert_eq!(view["method"], "provider");
        assert_eq!(view["references_count"], 7);
        assert!(view["expanded_at"].is_string(), "{view}");
        let view = &views[unavailable.to_string()];
        assert_eq!(view["state"], "unavailable");
        assert_eq!(view["unavailable_reason"], "provider_not_found");
        assert!(view["method"].is_null());
        let view = &views[known.to_string()];
        assert_eq!(view["state"], "known");
        assert_eq!(view["references_count"], 1);
        assert!(
            view["method"].is_null() && view["expanded_at"].is_null(),
            "{view}"
        );
        assert!(
            views.get(unknown.to_string()).is_none(),
            "unknown fanout has no entry: {views}"
        );

        let (_, limited) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}?limit=1", seeded.center),
        )
        .await;
        let keys: Vec<&String> = limited["expansions"]
            .as_object()
            .expect("map")
            .keys()
            .collect();
        assert!(
            keys.iter().all(|key| { node_ids(&limited).contains(key) }),
            "only visible nodes carry entries: {limited}"
        );
    }

    #[tokio::test]
    async fn should_return_400_when_limit_is_zero_negative_or_not_a_number() {
        let fixture = fixture(None).await;
        let work = WorkId::new();
        for limit in ["0", "-1", "abc", "501", "1.5", "%205", "5%20", "", "+5"] {
            let (status, json) = get(
                &fixture.state,
                &format!("/api/graph/neighbors/{work}?limit={limit}"),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "limit={limit:?}");
            assert_eq!(json["error"], "invalid_limit", "limit={limit:?}");
        }
        // A valid limit gets past parsing and reaches the graph check.
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{work}?limit=500"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "graph_unavailable");
    }

    #[tokio::test]
    async fn should_return_400_when_tag_filter_id_is_malformed() {
        let fixture = fixture(None).await;
        let work = WorkId::new();
        let good = TagId::new();
        let too_many = vec![good.to_string(); 17].join(",");
        for tags in [
            "nonsense".to_owned(),
            format!("{good},nonsense"),
            format!("w{}", &good.to_string()[1..]),
            too_many,
        ] {
            let (status, json) = get(
                &fixture.state,
                &format!("/api/graph/neighbors/{work}?tags={tags}"),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "tags={tags}");
            assert_eq!(json["error"], "invalid_tag_id", "tags={tags}");
        }
    }

    #[tokio::test]
    async fn should_return_400_when_expand_has_too_many_or_too_long_keys() {
        let fixture = fixture(None).await;
        let work = WorkId::new();
        let many = (0..33)
            .map(|n| format!("k{n}"))
            .collect::<Vec<_>>()
            .join(",");
        let long = "x".repeat(65);
        for expand in [many, long] {
            let (status, json) = get(
                &fixture.state,
                &format!("/api/graph/neighbors/{work}?expand={expand}"),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error"], "invalid_expand");
        }
    }

    #[test]
    fn should_ignore_empty_segments_and_duplicates_when_parsing_filters() {
        let tag = TagId::new();
        let text = format!(",{tag},,{tag},");
        let filter = parse_tag_filter(Some(&text))
            .ok()
            .flatten()
            .expect("filter");
        assert_eq!(filter.len(), 1);
        assert!(parse_tag_filter(Some(",,")).ok().flatten().is_none());
        assert!(parse_tag_filter(None).ok().flatten().is_none());
        assert_eq!(
            parse_expand(Some("a,,b,a")).ok().map(|keys| keys.len()),
            Some(2)
        );
        assert_eq!(parse_limit(Some("007")).ok(), Some(Some(7)));
        assert_eq!(parse_limit(None).ok(), Some(None));
    }

    #[tokio::test]
    async fn should_return_top_n_and_clusters_when_limit_given() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 30, |n| {
            f64::from(u32::try_from(n).unwrap_or(0)) / 100.0
        })
        .await;
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}?limit=10", seeded.center),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let ids = node_ids(&json);
        assert_eq!(ids.len(), 11);
        assert_eq!(ids[0], seeded.center.to_string());
        // Highest score first: the last seeded Work.
        assert_eq!(ids[1], seeded.cited[29].to_string());
        assert_eq!(json["ranking"]["total"], 30);
        assert_eq!(json["ranking"]["visible"], 10);
        assert_eq!(json["ranking"]["limit"], 10);
        assert_eq!(cluster_count(&json), 20);
        // Payload maps cover the visible nodes only.
        let prominence = json["prominence"].as_object().expect("prominence");
        assert_eq!(prominence.len(), 10);
        assert!(prominence.keys().all(|key| ids.contains(key)));
        assert_eq!(json["edges"].as_array().map(Vec::len), Some(10));
    }

    #[tokio::test]
    async fn should_keep_ramp_a_shape_when_limit_absent() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 12, |_| 0.5).await;
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}", seeded.center),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["direction"], "references");
        assert_eq!(node_ids(&json).len(), 13);
        assert_eq!(json["edges"].as_array().map(Vec::len), Some(12));
        assert_eq!(json["ranking"]["limit"], Value::Null);
        assert_eq!(json["ranking"]["total"], 12);
        assert_eq!(json["clusters"], serde_json::json!([]));
        for field in [
            "prominence",
            "marks",
            "work_tags",
            "local_papers",
            "acquisitions",
        ] {
            assert!(json[field].is_object(), "{field}");
        }
    }

    #[tokio::test]
    async fn should_hide_hidden_and_pin_pinned_when_marks_exist() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 10, |n| {
            f64::from(u32::try_from(n).unwrap_or(0)) / 10.0
        })
        .await;
        let pinned = seeded.cited[0];
        let hidden = seeded.cited[9];
        let downplayed = seeded.cited[8];
        for (work, mark) in [
            (pinned, WorkMark::Pinned),
            (hidden, WorkMark::Hidden),
            (downplayed, WorkMark::Downplayed),
        ] {
            seeded
                .graph
                .curation
                .set_mark(seeded.workspace, work, mark, Some("why"))
                .await
                .expect("mark");
        }
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}?limit=3", seeded.center),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let ids = node_ids(&json);
        // Center, the pinned Work despite its lowest score, then the best unmarked.
        assert_eq!(ids[1], pinned.to_string());
        assert_eq!(ids[2], seeded.cited[7].to_string());
        assert_eq!(ids[3], seeded.cited[6].to_string());
        assert_eq!(ids.len(), 4);
        assert!(!ids.contains(&hidden.to_string()));
        assert_eq!(json["ranking"]["total"], 9);
        assert_eq!(json["ranking"]["hidden"], 1);
        assert_eq!(json["ranking"]["pinned"], 1);
        assert_eq!(json["marks"][pinned.to_string()]["mark"], "pinned");
        assert_eq!(json["marks"][pinned.to_string()]["note"], "why");
        assert!(json["marks"].get(hidden.to_string()).is_none());
        let keys: Vec<&str> = json["clusters"]
            .as_array()
            .expect("clusters")
            .iter()
            .filter_map(|cluster| cluster["key"].as_str())
            .collect();
        assert!(keys.contains(&"hidden") && keys.contains(&"downplayed"));
    }

    #[tokio::test]
    async fn should_expand_cluster_when_expand_key_given() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 10, |n| {
            f64::from(u32::try_from(n).unwrap_or(0)) / 10.0
        })
        .await;
        seeded
            .graph
            .curation
            .set_mark(seeded.workspace, seeded.cited[9], WorkMark::Hidden, None)
            .await
            .expect("mark");
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let base = format!("/api/graph/neighbors/{}?limit=2", seeded.center);
        let (_, json) = get(&fixture.state, &base).await;
        let key = json["clusters"][0]["key"].as_str().expect("key").to_owned();
        let collapsed = cluster_count(&json);
        let (status, expanded) =
            get(&fixture.state, &format!("{base}&expand={key},nonsense")).await;
        assert_eq!(status, StatusCode::OK, "{expanded}");
        assert!(node_ids(&expanded).len() > 3);
        assert!(cluster_count(&expanded) < collapsed);
        assert!(
            expanded["clusters"]
                .as_array()
                .expect("clusters")
                .iter()
                .all(|cluster| cluster["key"] != key.as_str())
        );
        // Expanding the hidden cluster shows the hidden Work with its mark.
        let (_, hidden) = get(&fixture.state, &format!("{base}&expand=hidden")).await;
        let id = seeded.cited[9].to_string();
        assert!(node_ids(&hidden).contains(&id));
        assert_eq!(hidden["marks"][&id]["mark"], "hidden");
    }

    #[tokio::test]
    async fn should_compute_missing_prominence_on_read_when_cache_empty() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 5, |_| 0.5).await;
        // Drop the seeded rows by using fresh Works with no cache at all.
        let extra = seeded
            .graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        let source = seeded
            .graph
            .source_records
            .upsert(
                "test",
                &uuid::Uuid::new_v4().to_string(),
                Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("source");
        seeded
            .graph
            .citations
            .add_assertion(seeded.center, extra, Some(source))
            .await
            .expect("assertion");
        assert!(
            seeded
                .graph
                .prominence
                .cached(seeded.workspace, &[extra])
                .await
                .expect("cached")
                .is_empty()
        );
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}?limit=50", seeded.center),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert!(json["prominence"][extra.to_string()]["score"].is_number());
        assert_eq!(
            seeded
                .graph
                .prominence
                .cached(seeded.workspace, &[extra])
                .await
                .expect("cached")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn should_scope_edges_and_local_papers_to_visible_nodes() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 8, |n| {
            f64::from(u32::try_from(n).unwrap_or(0)) / 10.0
        })
        .await;
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let (status, json) = get(
            &fixture.state,
            &format!("/api/graph/neighbors/{}?limit=3", seeded.center),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let ids = node_ids(&json);
        let edges = json["edges"].as_array().expect("edges");
        assert_eq!(edges.len(), 3);
        for edge in edges {
            let cited = edge["cited"].as_str();
            let citing = edge["citing"].as_str();
            assert!(
                cited.is_some_and(|id| ids.iter().any(|node| node == id))
                    && citing.is_some_and(|id| ids.iter().any(|node| node == id)),
                "{edge}"
            );
        }
        for field in [
            "local_papers",
            "acquisitions",
            "marks",
            "work_tags",
            "prominence",
        ] {
            assert!(
                json[field]
                    .as_object()
                    .expect(field)
                    .keys()
                    .all(|key| ids.contains(key)),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn should_filter_by_tag_when_tags_param_given() {
        let Some(graph) = test_graph().await else {
            return;
        };
        let seeded = seed(graph, 10, |n| {
            f64::from(u32::try_from(n).unwrap_or(0)) / 10.0
        })
        .await;
        let marker = uuid::Uuid::new_v4().simple().to_string();
        let tag = seeded
            .graph
            .curation
            .create_tag(
                seeded.workspace,
                &TagName::parse(&format!("filter {marker}")).expect("name"),
                &TagColor::parse("#112233").expect("color"),
                TagKind::Topic,
            )
            .await
            .expect("tag");
        for work in [seeded.cited[1], seeded.cited[3]] {
            seeded
                .graph
                .curation
                .assign_tag(seeded.workspace, work, tag.id)
                .await
                .expect("assign");
        }
        seeded
            .graph
            .curation
            .set_mark(seeded.workspace, seeded.cited[5], WorkMark::Pinned, None)
            .await
            .expect("mark");
        let fixture = fixture(Some(seeded.graph.clone())).await;
        let base = format!("/api/graph/neighbors/{}", seeded.center);
        let (status, json) = get(&fixture.state, &format!("{base}?limit=5&tags={}", tag.id)).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(
            node_ids(&json)[1..],
            [seeded.cited[3].to_string(), seeded.cited[1].to_string()]
        );
        assert_eq!(json["ranking"]["filtered_out"], 8);
        assert_eq!(json["ranking"]["total"], 2);
        assert_eq!(
            json["work_tags"][seeded.cited[1].to_string()][0],
            tag.id.to_string()
        );
        assert_eq!(json["clusters"], serde_json::json!([]));
        // A well-formed unknown tag matches nothing.
        let (status, json) = get(
            &fixture.state,
            &format!("{base}?limit=5&tags={}", TagId::new()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(node_ids(&json).len(), 1);
        assert_eq!(json["ranking"]["filtered_out"], 10);
    }
}
