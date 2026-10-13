//! Ranked neighborhood of a Work: the graph overlay's fanout.

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use lysilogy_core::{Work, WorkId};
use serde::{Deserialize, Serialize};

use super::{
    AppState,
    acquisition::{self, AcquisitionStates},
    fetch_work, graph_error, graph_or_unavailable, graph_query_failure, labels, literature,
    parse_work_id,
};

#[derive(Debug, Deserialize)]
pub(super) struct NeighborsQuery {
    direction: Option<String>,
}

#[derive(Debug, Serialize)]
struct NeighborsResponse {
    center: WorkId,
    direction: &'static str,
    nodes: Vec<Work>,
    edges: Vec<labels::GraphEdge>,
    local_papers: literature::LocalPapers,
    acquisitions: AcquisitionStates,
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

pub(super) async fn get_graph_neighbors(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<NeighborsQuery>,
) -> Response {
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let work_id = match parse_work_id(&id) {
        Ok(work_id) => work_id,
        Err(response) => return *response,
    };
    let Some(direction) = NeighborDirection::parse(query.direction.as_deref()) else {
        return graph_error(
            StatusCode::BAD_REQUEST,
            "invalid_direction",
            "direction must be references, citations, or both.",
        );
    };
    let center = match fetch_work(graph, work_id).await {
        Ok(work) => work,
        Err(response) => return *response,
    };
    let work_id = center.id;

    let mut nodes = vec![center];
    let mut seen = HashSet::from([work_id]);
    let mut pairs = Vec::new();

    if matches!(
        direction,
        NeighborDirection::References | NeighborDirection::Both
    ) {
        let neighbors = match graph.citations.outgoing(work_id).await {
            Ok(works) => works,
            Err(error) => return graph_query_failure(&error),
        };
        for neighbor in neighbors {
            let cited = neighbor.id;
            if seen.insert(cited) {
                nodes.push(neighbor);
            }
            pairs.push((work_id, cited));
        }
    }
    if matches!(
        direction,
        NeighborDirection::Citations | NeighborDirection::Both
    ) {
        let neighbors = match graph.citations.incoming(work_id).await {
            Ok(works) => works,
            Err(error) => return graph_query_failure(&error),
        };
        for neighbor in neighbors {
            let citing = neighbor.id;
            if seen.insert(citing) {
                nodes.push(neighbor);
            }
            pairs.push((citing, work_id));
        }
    }

    let edges = match labels::edge_payloads(&state, &pairs).await {
        Ok(edges) => edges,
        Err(response) => return *response,
    };
    let work_ids: Vec<WorkId> = nodes.iter().map(|work| work.id).collect();
    let local_papers = match literature::local_papers_for(&state, &work_ids).await {
        Ok(local_papers) => local_papers,
        Err(response) => return *response,
    };
    let acquisitions = match acquisition::states_for(&state, &work_ids).await {
        Ok(acquisitions) => acquisitions,
        Err(response) => return *response,
    };
    (
        StatusCode::OK,
        Json(NeighborsResponse {
            center: work_id,
            direction: direction.as_str(),
            nodes,
            edges,
            local_papers,
            acquisitions,
        }),
    )
        .into_response()
}
