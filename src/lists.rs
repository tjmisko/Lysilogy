//! Reading lists: the service shared by the API and the CLI.
//!
//! A list is one graph of Work, topic and claim nodes with typed edges. Every method works
//! in the default workspace, resolves Work IDs through redirects, and fails with a
//! [`ListServiceError`] whose codes the API and CLI share. Reading status belongs to the
//! Work (D6), never to the list.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Datelike, Utc};
use lysilogy_core::{
    Claim, ClaimState, EdgeKind, EdgeRejection, EdgeSource, IdentifierScheme, LinearItem, ListEdge,
    ListError, ListId, ListReview, ListTitle, NodeId, NodeKind, ProviderRecord, ReadingStatus,
    TopicLabel, Work, WorkId, WorkspaceId, extract_identifiers, linearize, parse_list_description,
    parse_mark_note, title_key,
};
use lysilogy_db::DbError;
use lysilogy_db::list::{
    ArchiveScope, List, ListNode, ListPatch, ListSummary, Membership, NewEdge, NewList, NewNode,
    NewNodeContent, NodePatch, RemovedNode, StoredEdge,
};
use lysilogy_db::reading_state::WorkReadingState;
use lysilogy_ingest::openalex::search_title_cached;
use lysilogy_ingest::verify_claim::{ClaimResolution, VerifyContext, verify_claim};
use serde::Serialize;
use serde_json::{Value, json};

pub mod render;

use crate::{
    AppState, Error,
    api::{AcquisitionStates, LocalPaper, LocalPapers, local_papers_for, states_for},
    domain::PaperId,
    expansion::ExpansionView,
    graph::GraphState,
};

/// Most matching titles an ambiguous list reference lists.
const AMBIGUOUS_MATCHES_SHOWN: usize = 5;
/// Most results a title search returns.
const SEARCH_LIMIT: usize = 20;
/// Most nodes one batch adds.
pub const BATCH_MAX: usize = 100;
/// Most provider results a Work search keeps.
const PROVIDER_RESULTS_MAX: usize = 10;

/// Why a list operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ListServiceError {
    /// `list_not_found`, `node_not_found`, `edge_not_found`, `work_not_found`, or
    /// `paper_not_found`.
    #[error("{}", not_found_message(.0))]
    NotFound(&'static str),
    /// The input was refused: a code such as `invalid_order` and a message.
    #[error("{1}")]
    Invalid(&'static str, String),
    /// The change conflicts with the list: `item_exists`, `list_full`, `via_not_in_list`,
    /// `no_citation_edge`, `paper_not_linked`, `invalid_edge_kind`, and the like.
    #[error("{1}")]
    Conflict(&'static str, String),
    /// The edge would close a cycle; the nodes name it.
    #[error("The edge would close a cycle.")]
    EdgeCycle(Vec<NodeId>),
    /// The provider could not be reached or answered badly.
    #[error("{0}")]
    Provider(String),
    /// The graph database is missing or a query failed.
    #[error("{0}")]
    Graph(Error),
}

fn not_found_message(code: &str) -> &'static str {
    match code {
        "list_not_found" => "No such list.",
        "node_not_found" => "No such node in the list.",
        "edge_not_found" => "No such edge in the list.",
        "paper_not_found" => "No such paper.",
        _ => "No such work.",
    }
}

impl ListServiceError {
    /// The machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound(code) | Self::Invalid(code, _) | Self::Conflict(code, _) => code,
            Self::EdgeCycle(_) => "edge_cycle",
            Self::Provider(_) => "provider_error",
            Self::Graph(_) => "graph_error",
        }
    }
}

impl From<ListServiceError> for Error {
    fn from(error: ListServiceError) -> Self {
        match error {
            ListServiceError::Graph(error) => error,
            ListServiceError::EdgeCycle(nodes) => {
                let named: Vec<String> = nodes.iter().map(ToString::to_string).collect();
                Self::InvalidRequest(format!(
                    "the edge would close a cycle: {}",
                    named.join(" -> ")
                ))
            }
            other => Self::InvalidRequest(other.to_string()),
        }
    }
}

fn database(error: &DbError) -> ListServiceError {
    ListServiceError::Graph(Error::Literature(error.to_string()))
}

fn invalid(code: &'static str, message: &str) -> ListServiceError {
    ListServiceError::Invalid(code, message.to_owned())
}

/// The refusals the list repository reports as `DbError::Conflict`, as service errors.
fn list_error(error: ListError) -> ListServiceError {
    match error {
        ListError::NodeNotFound => ListServiceError::NotFound("node_not_found"),
        ListError::InvalidOrder
        | ListError::EmptyTitle
        | ListError::TitleTooLong
        | ListError::TitleControlCharacter
        | ListError::DescriptionTooLong
        | ListError::InvalidTopicLabel => {
            ListServiceError::Invalid(error.code(), error.to_string())
        }
        _ => ListServiceError::Conflict(error.code(), error.to_string()),
    }
}

/// Map a repository failure: a missing list, the stable conflict codes, or a graph failure.
fn list_failure(error: &DbError) -> ListServiceError {
    if error.is_not_found() {
        return ListServiceError::NotFound("list_not_found");
    }
    let DbError::Conflict(code) = error else {
        return database(error);
    };
    for known in [
        ListError::ItemExists,
        ListError::ViaNotInList,
        ListError::NoCitationEdge,
        ListError::TooDeep,
        ListError::Full,
        ListError::TopicsFull,
        ListError::EdgesFull,
        ListError::SeedItem,
        ListError::PaperNotLinked,
        ListError::EdgeCycle,
        ListError::InvalidEdgeKind,
        ListError::InvalidOrder,
        ListError::NodeNotFound,
    ] {
        if known.code() == code {
            return list_error(known);
        }
    }
    match code.as_str() {
        "invalid_node_kind" => invalid(
            "invalid_node_kind",
            "Only topic nodes have a label or summary.",
        ),
        "not_a_claim" => ListServiceError::Conflict(
            "not_a_claim",
            "Only claim nodes can be resolved to a Work.".to_owned(),
        ),
        "invalid_context_links" => invalid(
            "invalid_context_links",
            "A node carries at most five context links.",
        ),
        "invalid_claim" => invalid("invalid_claim", "The claim is not valid."),
        _ => database(error),
    }
}

/// Validate a list title.
///
/// # Errors
/// `Invalid("invalid_list_title", …)`.
pub fn parse_title(text: &str) -> Result<ListTitle, ListServiceError> {
    ListTitle::parse(text).map_err(list_error)
}

/// Validate and normalize a list description: trimmed, empty becomes `None`.
///
/// # Errors
/// `Invalid("invalid_description", …)`.
pub fn parse_description(text: Option<&str>) -> Result<Option<String>, ListServiceError> {
    parse_list_description(text).map_err(list_error)
}

/// Validate a topic label.
///
/// # Errors
/// `Invalid("invalid_topic_label", …)`.
pub fn parse_topic_label(text: &str) -> Result<TopicLabel, ListServiceError> {
    TopicLabel::parse(text).map_err(list_error)
}

/// Validate and normalize a node note: trimmed, empty becomes `None`.
///
/// # Errors
/// `Invalid("invalid_note", …)`.
pub fn parse_note(text: Option<&str>) -> Result<Option<String>, ListServiceError> {
    parse_mark_note(text)
        .map_err(|error| ListServiceError::Invalid("invalid_note", error.to_string()))
}

/// Validate and normalize a topic summary: trimmed, empty becomes `None`.
///
/// # Errors
/// `Invalid("invalid_summary", …)`.
pub fn parse_summary(text: Option<&str>) -> Result<Option<String>, ListServiceError> {
    parse_mark_note(text)
        .map_err(|_| invalid("invalid_summary", "A summary is at most 2000 characters."))
}

/// Validate a Work's reading status.
///
/// # Errors
/// `Invalid("invalid_status", …)`.
pub fn parse_status(text: &str) -> Result<ReadingStatus, ListServiceError> {
    text.parse()
        .map_err(|_| invalid("invalid_status", "Status is one of unread, reading, read."))
}

/// Validate an edge kind.
///
/// # Errors
/// `Invalid("invalid_edge_kind", …)`.
pub fn parse_edge_kind(text: &str) -> Result<EdgeKind, ListServiceError> {
    text.parse().map_err(|_| {
        invalid(
            "invalid_edge_kind",
            "Edge kind is one of prerequisite, covers, subtopic.",
        )
    })
}

/// How `PUT order` rewrites positions from a strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderStrategy {
    /// Prerequisites, then citations, then topic order (D9).
    Topological,
    /// Older first; unknown years last.
    Year,
    /// Cited before citing, deeper and more prominent first; ignores topics and
    /// prerequisites.
    FoundationalFirst,
}

/// Validate an order strategy; `-` is accepted for `_`.
///
/// # Errors
/// `Invalid("invalid_strategy", …)`.
pub fn parse_strategy(text: &str) -> Result<OrderStrategy, ListServiceError> {
    match text.replace('-', "_").as_str() {
        "topological" => Ok(OrderStrategy::Topological),
        "year" => Ok(OrderStrategy::Year),
        "foundational_first" => Ok(OrderStrategy::FoundationalFirst),
        _ => Err(invalid(
            "invalid_strategy",
            "Strategy is one of topological, year, foundational_first.",
        )),
    }
}

/// Items per reading status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct StatusCounts {
    pub unread: i64,
    pub reading: i64,
    pub read: i64,
}

/// A list as listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListSummaryView {
    #[serde(flatten)]
    pub list: List,
    pub seed_title: Option<String>,
    /// Work and claim nodes.
    pub item_count: i64,
    pub topic_count: i64,
    pub claim_count: i64,
    pub proposed_count: i64,
    pub status_counts: StatusCounts,
}

/// One Work on a node's citation path, with the title if known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PathStep {
    pub work_id: WorkId,
    pub title: Option<String>,
}

/// One node as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NodeView {
    #[serde(flatten)]
    pub node: ListNode,
    /// The Work of a Work node.
    pub work: Option<Work>,
    /// Counting from 1 in position order.
    pub ordinal: usize,
    /// Citation hops from the walk's root; 0 for nodes not reached by walking.
    pub depth: usize,
    pub path_steps: Vec<PathStep>,
}

/// A Work's reading status as shown; `changed_at` is `None` for the implicit `unread`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ReadingStateView {
    pub status: ReadingStatus,
    pub changed_at: Option<DateTime<Utc>>,
}

/// A list with its graph and what the reader knows about its Works.
#[derive(Debug, Serialize)]
pub struct ListDetail {
    pub list: ListSummaryView,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<StoredEdge>,
    /// Derived `(citing, cited)` pairs among member Works.
    pub citations: Vec<(WorkId, WorkId)>,
    /// Keyed by Work ID, for every Work node.
    pub reading_states: BTreeMap<String, ReadingStateView>,
    pub local_papers: LocalPapers,
    pub acquisitions: AcquisitionStates,
    pub expansions: BTreeMap<String, ExpansionView>,
}

impl ListDetail {
    /// Local papers held for a Work, if any.
    #[must_use]
    pub fn local_papers_of(&self, work: WorkId) -> &[LocalPaper] {
        self.local_papers
            .get(&work.to_string())
            .map_or(&[], Vec::as_slice)
    }

    /// A Work's reading status; `Unread` when none is stored.
    #[must_use]
    pub fn status_of(&self, work: WorkId) -> ReadingStatus {
        self.reading_states
            .get(&work.to_string())
            .map_or(ReadingStatus::Unread, |state| state.status)
    }
}

/// What to add to a list.
#[derive(Clone, Debug)]
pub enum AddNode {
    Work {
        work: WorkId,
        via: Option<WorkId>,
        topics: Vec<NodeId>,
        note: Option<String>,
    },
    /// A held paper, resolved to its linked Work (D5).
    Paper {
        paper: PaperId,
        topics: Vec<NodeId>,
        note: Option<String>,
    },
    Topic {
        label: TopicLabel,
        summary: Option<String>,
    },
    /// A citation by name; it is verified first.
    Claim { claim: Claim, note: Option<String> },
}

/// A partial change to a node.
#[derive(Clone, Debug, Default)]
pub struct NodeChange {
    pub note: Option<Option<String>>,
    pub label: Option<TopicLabel>,
    pub summary: Option<Option<String>>,
    /// Accept a `proposed` node.
    pub accept: bool,
    /// Turn a claim node into a node of this Work.
    pub resolve_to: Option<WorkId>,
}

/// One hit of a Work search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkHit {
    /// The graph Work, if the hit is in the graph.
    pub work_id: Option<WorkId>,
    /// The held paper, if the hit is in the library.
    pub paper_id: Option<PaperId>,
    pub title: String,
    pub year: Option<i32>,
    pub authors: Vec<String>,
    /// `library`, `graph` or `provider`.
    pub source: &'static str,
    pub in_graph: bool,
    pub openalex_id: Option<String>,
    pub doi: Option<String>,
}

/// What `verify` did to a list's pending claims.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct VerifyReport {
    /// Pending claims retried.
    pub checked: usize,
    /// Claims that became Work nodes.
    pub resolved: usize,
    /// Claims that stay pending (the provider is still unreachable).
    pub still_pending: usize,
    /// Claims whose state changed but which remain claims.
    pub reclassified: usize,
}

fn graph_of(state: &AppState) -> Result<&GraphState, ListServiceError> {
    state.graph.as_ref().ok_or_else(|| {
        ListServiceError::Graph(Error::InvalidRequest(
            "lists need a database; set LYSILOGY_DATABASE_URL".to_owned(),
        ))
    })
}

async fn workspace_of(graph: &GraphState) -> Result<WorkspaceId, ListServiceError> {
    graph
        .documents
        .ensure_default_workspace()
        .await
        .map_err(|error| database(&error))
}

/// Resolve a Work through redirects and require it to exist.
async fn existing_work(graph: &GraphState, work: WorkId) -> Result<Work, ListServiceError> {
    let resolved = graph
        .works
        .resolve_redirect(work)
        .await
        .map_err(|error| database(&error))?;
    graph
        .works
        .get(resolved)
        .await
        .map_err(|error| database(&error))?
        .ok_or(ListServiceError::NotFound("work_not_found"))
}

async fn resolved(graph: &GraphState, work: WorkId) -> Result<WorkId, ListServiceError> {
    graph
        .works
        .resolve_redirect(work)
        .await
        .map_err(|error| database(&error))
}

fn claim_of(node: &ListNode) -> Option<Claim> {
    serde_json::from_value(node.claim.clone()?).ok()
}

/// Fill the identifiers a claim's text carries when none were given, and require a title.
fn normalized_claim(mut claim: Claim) -> Result<Claim, ListServiceError> {
    let trimmed = claim.title.trim().to_owned();
    claim.title = trimmed;
    if claim.title.is_empty() {
        return Err(invalid("invalid_claim", "A claim needs a title."));
    }
    let text = claim.raw.clone().unwrap_or_else(|| claim.title.clone());
    for found in extract_identifiers(&text) {
        match found.scheme {
            IdentifierScheme::Doi if claim.doi.is_none() => claim.doi = Some(found.value),
            IdentifierScheme::Arxiv if claim.arxiv.is_none() => claim.arxiv = Some(found.value),
            _ => {}
        }
    }
    Ok(claim)
}

impl AppState {
    async fn require_list(
        graph: &GraphState,
        workspace: WorkspaceId,
        list: ListId,
    ) -> Result<List, ListServiceError> {
        graph
            .lists
            .get(workspace, list)
            .await
            .map_err(|error| database(&error))?
            .ok_or(ListServiceError::NotFound("list_not_found"))
    }

    async fn summary_views(
        graph: &GraphState,
        summaries: Vec<ListSummary>,
    ) -> Result<Vec<ListSummaryView>, ListServiceError> {
        let seeds: Vec<WorkId> = summaries
            .iter()
            .filter_map(|summary| summary.list.seed_work_id)
            .collect();
        let titles: HashMap<WorkId, String> = graph
            .works
            .get_many(&seeds)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .filter_map(|work| Some((work.id, work.display_title?)))
            .collect();
        Ok(summaries
            .into_iter()
            .map(|summary| ListSummaryView {
                seed_title: summary
                    .list
                    .seed_work_id
                    .and_then(|seed| titles.get(&seed).cloned()),
                item_count: summary.item_count,
                topic_count: summary.topic_count,
                claim_count: summary.claim_count,
                proposed_count: summary.proposed_count,
                status_counts: StatusCounts {
                    unread: summary.unread,
                    reading: summary.reading,
                    read: summary.read,
                },
                list: summary.list,
            })
            .collect())
    }

    async fn summary_view(
        graph: &GraphState,
        workspace: WorkspaceId,
        list: ListId,
    ) -> Result<ListSummaryView, ListServiceError> {
        let summary = graph
            .lists
            .list(workspace, None, ArchiveScope::All)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .find(|summary| summary.list.id == list)
            .ok_or(ListServiceError::NotFound("list_not_found"))?;
        Ok(Self::summary_views(graph, vec![summary]).await?.remove(0))
    }

    /// Node views in position order, with path titles from one Works lookup over the
    /// nodes and the union of their path IDs.
    async fn node_views(
        graph: &GraphState,
        nodes: Vec<ListNode>,
    ) -> Result<Vec<NodeView>, ListServiceError> {
        let mut wanted: HashSet<WorkId> = HashSet::new();
        for node in &nodes {
            wanted.extend(node.work_id);
            wanted.extend(node.path.iter().flatten().copied());
        }
        let wanted: Vec<WorkId> = wanted.into_iter().collect();
        let works: HashMap<WorkId, Work> = graph
            .works
            .get_many(&wanted)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .map(|work| (work.id, work))
            .collect();
        Ok(nodes
            .into_iter()
            .enumerate()
            .map(|(index, node)| NodeView {
                work: node.work_id.and_then(|id| works.get(&id).cloned()),
                ordinal: index + 1,
                depth: node
                    .path
                    .as_ref()
                    .map_or(0, |path| path.len().saturating_sub(1)),
                path_steps: node
                    .path
                    .iter()
                    .flatten()
                    .map(|step| PathStep {
                        work_id: *step,
                        title: works.get(step).and_then(|work| work.display_title.clone()),
                    })
                    .collect(),
                node,
            })
            .collect())
    }

    async fn node_view(
        graph: &GraphState,
        list: ListId,
        node: NodeId,
    ) -> Result<NodeView, ListServiceError> {
        let nodes = graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| list_failure(&error))?;
        Self::node_views(graph, nodes)
            .await?
            .into_iter()
            .find(|view| view.node.id == node)
            .ok_or(ListServiceError::NotFound("node_not_found"))
    }

    /// Create a list. With a seed the list is a walk list and holds the seed Work.
    ///
    /// # Errors
    /// `NotFound("work_not_found")` for an unknown seed; `Graph` for database failures.
    pub async fn create_list(
        &self,
        title: &ListTitle,
        description: Option<String>,
        seed: Option<WorkId>,
    ) -> Result<ListDetail, ListServiceError> {
        let graph = graph_of(self)?;
        let seed = match seed {
            Some(seed) => Some(existing_work(graph, seed).await?),
            None => None,
        };
        let workspace = workspace_of(graph).await?;
        let mut new = NewList::manual(title.clone());
        new.description = description;
        if let Some(seed) = &seed {
            new.origin = lysilogy_core::ListOrigin::Walk;
            new.seed_work_id = Some(seed.id);
        }
        let list = graph
            .lists
            .create(workspace, new)
            .await
            .map_err(|error| database(&error))?;
        if let Some(seed) = seed {
            let mut node = NewNode::work(seed.id);
            node.source = lysilogy_core::NodeSource::Walk;
            graph
                .lists
                .add_node(list.id, node)
                .await
                .map_err(|error| list_failure(&error))?;
        }
        self.list_detail(list.id).await
    }

    /// The workspace's lists by update time, optionally only those holding one Work.
    ///
    /// # Errors
    /// `Graph` for database failures.
    pub async fn list_summaries(
        &self,
        work: Option<WorkId>,
        scope: ArchiveScope,
    ) -> Result<Vec<ListSummaryView>, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let work = match work {
            Some(work) => Some(resolved(graph, work).await?),
            None => None,
        };
        let summaries = graph
            .lists
            .list(workspace, work, scope)
            .await
            .map_err(|error| database(&error))?;
        Self::summary_views(graph, summaries).await
    }

    /// A list with its nodes, edges, derived citations, reading states, local papers,
    /// acquisition states, and expansion states.
    ///
    /// # Errors
    /// `NotFound("list_not_found")`; `Graph` for database failures.
    pub async fn list_detail(&self, list: ListId) -> Result<ListDetail, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        let summary = Self::summary_view(graph, workspace, list).await?;
        let nodes = graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| list_failure(&error))?;
        let edges = graph
            .lists
            .edges(list)
            .await
            .map_err(|error| list_failure(&error))?;
        let work_ids: Vec<WorkId> = nodes.iter().filter_map(|node| node.work_id).collect();
        let stored = graph
            .reading_states
            .for_works(workspace, &work_ids)
            .await
            .map_err(|error| database(&error))?;
        let reading_states = work_ids
            .iter()
            .map(|work| {
                let state = stored.get(work);
                (
                    work.to_string(),
                    ReadingStateView {
                        status: state.map_or(ReadingStatus::Unread, |state| state.status),
                        changed_at: state.map(|state| state.changed_at),
                    },
                )
            })
            .collect();
        let citations = graph
            .citations
            .edges_among(&work_ids)
            .await
            .map_err(|error| database(&error))?;
        let nodes = Self::node_views(graph, nodes).await?;
        let graph_failure = |_response| {
            ListServiceError::Graph(Error::Literature("the graph query failed".to_owned()))
        };
        let local_papers = local_papers_for(self, &work_ids)
            .await
            .map_err(graph_failure)?;
        let acquisitions = states_for(self, &work_ids).await.map_err(graph_failure)?;
        let expansions = self
            .expansion_views(&work_ids)
            .await
            .map_err(ListServiceError::Graph)?;
        Ok(ListDetail {
            list: summary,
            nodes,
            edges,
            citations,
            reading_states,
            local_papers,
            acquisitions,
            expansions,
        })
    }

    /// Retitle, describe, archive or unarchive a list.
    ///
    /// # Errors
    /// `NotFound("list_not_found")`; `Graph` for database failures.
    pub async fn update_list(
        &self,
        list: ListId,
        patch: ListPatch,
    ) -> Result<ListSummaryView, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        graph
            .lists
            .update(workspace, list, patch)
            .await
            .map_err(|error| database(&error))?
            .ok_or(ListServiceError::NotFound("list_not_found"))?;
        Self::summary_view(graph, workspace, list).await
    }

    /// Delete a list and its nodes.
    ///
    /// # Errors
    /// `NotFound("list_not_found")`; `Graph` for database failures.
    pub async fn delete_list(&self, list: ListId) -> Result<(), ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let deleted = graph
            .lists
            .delete(workspace, list)
            .await
            .map_err(|error| database(&error))?;
        if deleted {
            return Ok(());
        }
        Err(ListServiceError::NotFound("list_not_found"))
    }

    /// The Work a catalog paper is linked to, without linking it. `None` when unlinked.
    async fn linked_work_quiet(
        &self,
        graph: &GraphState,
        paper: &PaperId,
    ) -> Result<Option<WorkId>, ListServiceError> {
        let relative_path = self
            .catalog
            .read()
            .await
            .get(paper)
            .map(|entry| entry.overview.relative_path.clone())
            .ok_or(ListServiceError::NotFound("paper_not_found"))?;
        let workspace = workspace_of(graph).await?;
        let document = graph
            .documents
            .find_by_path(workspace, &relative_path)
            .await
            .map_err(|error| database(&error))?;
        match document.and_then(|document| document.work_id) {
            Some(work) => Ok(Some(resolved(graph, work).await?)),
            None => Ok(None),
        }
    }

    /// A paper's Work (D5): the linked one, linking the paper first when it has none.
    async fn work_for_paper(
        &self,
        graph: &GraphState,
        paper: &PaperId,
    ) -> Result<WorkId, ListServiceError> {
        if let Some(work) = self.linked_work_quiet(graph, paper).await? {
            return Ok(work);
        }
        let unlinked = || {
            ListServiceError::Conflict(
                ListError::PaperNotLinked.code(),
                "The paper could not be linked to a Work.".to_owned(),
            )
        };
        if let Err(error) = self.run_literature_flow(paper, false).await {
            tracing::warn!(%error, %paper, "could not link a paper to add it to a list");
            return Err(unlinked());
        }
        self.linked_work_quiet(graph, paper)
            .await?
            .ok_or_else(unlinked)
    }

    /// Verify a claim against the graph and the providers.
    async fn resolve_claim_text(
        &self,
        graph: &GraphState,
        claim: &Claim,
    ) -> Result<ClaimResolution, ListServiceError> {
        let context = VerifyContext {
            works: &graph.works,
            claims: &graph.claims,
            records: &graph.source_records,
            client: self.literature.client(),
        };
        verify_claim(&context, claim)
            .await
            .map_err(|error| ListServiceError::Graph(Error::from(error)))
    }

    async fn prepare_node(
        &self,
        graph: &GraphState,
        add: AddNode,
    ) -> Result<NewNode, ListServiceError> {
        match add {
            AddNode::Work {
                work,
                via,
                topics,
                note,
            } => {
                let work = existing_work(graph, work).await?;
                let via = match via {
                    Some(via) => Some(resolved(graph, via).await?),
                    None => None,
                };
                Ok(work_node(work.id, via, topics, note))
            }
            AddNode::Paper {
                paper,
                topics,
                note,
            } => {
                let work = self.work_for_paper(graph, &paper).await?;
                Ok(work_node(work, None, topics, note))
            }
            AddNode::Topic { label, summary } => {
                let mut node = NewNode::topic(label.clone());
                node.content = NewNodeContent::Topic { label, summary };
                Ok(node)
            }
            AddNode::Claim { claim, note } => {
                let claim = normalized_claim(claim)?;
                let mut node = match self.resolve_claim_text(graph, &claim).await? {
                    ClaimResolution::Work { work, .. } => NewNode::work(work.id),
                    ClaimResolution::Unresolved {
                        state, candidates, ..
                    } => claim_node(claim_json(&claim), state, &candidates),
                };
                node.note = note;
                Ok(node)
            }
        }
    }

    /// Add one node. A claim is verified first and becomes a Work node when it verifies.
    ///
    /// # Errors
    /// `NotFound` for a missing list, Work or paper; `Conflict` per the list rules.
    pub async fn add_list_node(
        &self,
        list: ListId,
        add: AddNode,
    ) -> Result<NodeView, ListServiceError> {
        let mut added = self.add_list_nodes(list, vec![add]).await?;
        Ok(added.remove(0))
    }

    /// Add many nodes, all or nothing: the first refusal adds none.
    ///
    /// # Errors
    /// As [`Self::add_list_node`]; `Invalid("batch_too_large")` over [`BATCH_MAX`].
    pub async fn add_list_nodes(
        &self,
        list: ListId,
        adds: Vec<AddNode>,
    ) -> Result<Vec<NodeView>, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        if adds.is_empty() || adds.len() > BATCH_MAX {
            return Err(invalid(
                "invalid_batch",
                "A batch adds between 1 and 100 nodes.",
            ));
        }
        let mut news = Vec::with_capacity(adds.len());
        for add in adds {
            news.push(self.prepare_node(graph, add).await?);
        }
        let added = graph
            .lists
            .add_nodes(list, news)
            .await
            .map_err(|error| list_failure(&error))?;
        let ids: HashSet<NodeId> = added.iter().map(|node| node.id).collect();
        let nodes = graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| list_failure(&error))?;
        let mut views = Self::node_views(graph, nodes).await?;
        views.retain(|view| ids.contains(&view.node.id));
        let rank: HashMap<NodeId, usize> = added
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
        views.sort_by_key(|view| rank.get(&view.node.id).copied());
        Ok(views)
    }

    /// Change a node, or resolve a claim node to a Work.
    ///
    /// # Errors
    /// `NotFound("list_not_found" | "node_not_found" | "work_not_found")`; `Conflict` for
    /// `item_exists` and `not_a_claim`; `Invalid("invalid_node_kind")`.
    pub async fn update_list_node(
        &self,
        list: ListId,
        node: NodeId,
        change: NodeChange,
    ) -> Result<NodeView, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        if let Some(work) = change.resolve_to {
            let work = existing_work(graph, work).await?;
            graph
                .lists
                .resolve_claim(list, node, work.id)
                .await
                .map_err(|error| list_failure(&error))?
                .ok_or(ListServiceError::NotFound("node_not_found"))?;
        }
        let patch = NodePatch {
            note: change.note,
            label: change.label,
            summary: change.summary,
            accept: change.accept,
        };
        let edits = patch.note.is_some()
            || patch.label.is_some()
            || patch.summary.is_some()
            || patch.accept;
        if edits {
            graph
                .lists
                .update_node(list, node, patch)
                .await
                .map_err(|error| list_failure(&error))?
                .ok_or(ListServiceError::NotFound("node_not_found"))?;
        }
        Self::node_view(graph, list, node).await
    }

    /// Remove a node and its edges. The result carries what was deleted so the caller can
    /// undo. Removing the seed Work clears the list's seed (D14).
    ///
    /// # Errors
    /// `NotFound("list_not_found" | "node_not_found")`.
    pub async fn remove_list_node(
        &self,
        list: ListId,
        node: NodeId,
    ) -> Result<RemovedNode, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        graph
            .lists
            .remove_node(list, node)
            .await
            .map_err(|error| list_failure(&error))?
            .ok_or(ListServiceError::NotFound("node_not_found"))
    }

    /// Add an edge (D8).
    ///
    /// # Errors
    /// `NotFound("node_not_found")` for an endpoint outside the list; `Conflict` for
    /// `invalid_edge_kind` and `edges_full`; `EdgeCycle` naming the cycle.
    pub async fn add_list_edge(
        &self,
        list: ListId,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    ) -> Result<StoredEdge, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        let new = NewEdge {
            from,
            to,
            kind,
            source: EdgeSource::User,
            review: ListReview::Accepted,
        };
        match graph
            .lists
            .add_edge(list, new)
            .await
            .map_err(|error| list_failure(&error))?
        {
            Ok(edge) => Ok(edge),
            Err(EdgeRejection::Cycle(nodes)) => Err(ListServiceError::EdgeCycle(nodes)),
            Err(EdgeRejection::UnknownNode) => Err(ListServiceError::NotFound("node_not_found")),
            Err(EdgeRejection::InvalidKind) => Err(ListServiceError::Conflict(
                ListError::InvalidEdgeKind.code(),
                ListError::InvalidEdgeKind.to_string(),
            )),
        }
    }

    /// Remove an edge.
    ///
    /// # Errors
    /// `NotFound("list_not_found" | "edge_not_found")`.
    pub async fn remove_list_edge(
        &self,
        list: ListId,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    ) -> Result<(), ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        let removed = graph
            .lists
            .remove_edge(list, from, to, kind)
            .await
            .map_err(|error| list_failure(&error))?;
        if removed {
            return Ok(());
        }
        Err(ListServiceError::NotFound("edge_not_found"))
    }

    /// Accept a `proposed` edge.
    ///
    /// # Errors
    /// `NotFound("list_not_found" | "edge_not_found")`.
    pub async fn accept_list_edge(
        &self,
        list: ListId,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    ) -> Result<(), ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        let found = graph
            .lists
            .accept_edge(list, from, to, kind)
            .await
            .map_err(|error| list_failure(&error))?;
        if found {
            return Ok(());
        }
        Err(ListServiceError::NotFound("edge_not_found"))
    }

    /// Set a manual order. `order` must be an exact permutation of the list's nodes.
    ///
    /// # Errors
    /// `NotFound("list_not_found")`; `Invalid("invalid_order", …)`.
    pub async fn reorder_list(
        &self,
        list: ListId,
        order: &[NodeId],
    ) -> Result<ListDetail, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        graph
            .lists
            .reorder(list, order)
            .await
            .map_err(|error| list_failure(&error))?;
        self.list_detail(list).await
    }

    /// Rewrite positions from a strategy (D10). Topics sit just before the first Work they
    /// cover.
    ///
    /// # Errors
    /// `NotFound("list_not_found")`; `Graph` for database failures.
    pub async fn order_list_by(
        &self,
        list: ListId,
        strategy: OrderStrategy,
    ) -> Result<ListDetail, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let found = Self::require_list(graph, workspace, list).await?;
        let nodes = graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| list_failure(&error))?;
        let edges = graph
            .lists
            .edges(list)
            .await
            .map_err(|error| list_failure(&error))?;
        let work_ids: Vec<WorkId> = nodes.iter().filter_map(|node| node.work_id).collect();
        let works: HashMap<WorkId, Work> = graph
            .works
            .get_many(&work_ids)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .map(|work| (work.id, work))
            .collect();
        let prominence = graph
            .prominence
            .prominence_for(workspace, &work_ids, Utc::now().year())
            .await
            .map_err(|error| database(&error))?;
        let cites = graph
            .citations
            .edges_among(&work_ids)
            .await
            .map_err(|error| database(&error))?;
        let items: Vec<LinearItem> = nodes
            .iter()
            .filter(|node| node.kind != NodeKind::Topic)
            .map(|node| {
                let work = node.work_id.and_then(|id| works.get(&id));
                let claim = claim_of(node);
                LinearItem {
                    id: node.id,
                    work_id: node.work_id,
                    depth: node
                        .path
                        .as_ref()
                        .map_or(0, |path| path.len().saturating_sub(1)),
                    score: node
                        .work_id
                        .and_then(|id| prominence.get(&id))
                        .map_or(0.0, |row| row.score),
                    year: work
                        .and_then(|work| work.display_year)
                        .or_else(|| claim.as_ref().and_then(|claim| claim.year)),
                    title_key: title_key(
                        work.and_then(|work| work.display_title.as_deref())
                            .or_else(|| claim.as_ref().map(|claim| claim.title.as_str()))
                            .unwrap_or_default(),
                    ),
                    is_seed: node.work_id.is_some() && node.work_id == found.seed_work_id,
                }
            })
            .collect();
        let stored: Vec<ListEdge> = edges
            .iter()
            .map(|edge| ListEdge {
                from: edge.from,
                to: edge.to,
                kind: edge.kind,
            })
            .collect();
        let item_order = match strategy {
            OrderStrategy::Topological => linearize(&items, &stored, &cites),
            OrderStrategy::FoundationalFirst => linearize(&items, &[], &cites),
            OrderStrategy::Year => year_order(&items),
        };
        let order = with_topics(&nodes, &stored, item_order);
        graph
            .lists
            .reorder(list, &order)
            .await
            .map_err(|error| list_failure(&error))?;
        self.list_detail(list).await
    }

    /// Set a Work's reading status in every list (D6).
    ///
    /// # Errors
    /// `NotFound("work_not_found")`; `Graph` for database failures.
    pub async fn set_work_reading_state(
        &self,
        work: WorkId,
        status: ReadingStatus,
    ) -> Result<WorkReadingState, ListServiceError> {
        let graph = graph_of(self)?;
        let work = existing_work(graph, work).await?;
        let workspace = workspace_of(graph).await?;
        graph
            .reading_states
            .set(workspace, work.id, status)
            .await
            .map_err(|error| database(&error))
    }

    /// The lists a Work is in.
    ///
    /// # Errors
    /// `NotFound("work_not_found")`; `Graph` for database failures.
    pub async fn work_memberships(
        &self,
        work: WorkId,
    ) -> Result<(WorkId, Vec<Membership>), ListServiceError> {
        let graph = graph_of(self)?;
        let work = existing_work(graph, work).await?;
        let workspace = workspace_of(graph).await?;
        let memberships = graph
            .lists
            .memberships_by_work(workspace, work.id, ArchiveScope::Active)
            .await
            .map_err(|error| database(&error))?;
        Ok((work.id, memberships))
    }

    /// The lists a held paper's Work is in; empty when the paper has no linked Work.
    ///
    /// # Errors
    /// `NotFound("paper_not_found")`; `Graph` for database failures.
    pub async fn paper_memberships(
        &self,
        paper: &PaperId,
    ) -> Result<(Option<WorkId>, Vec<Membership>), ListServiceError> {
        let graph = graph_of(self)?;
        let Some(work) = self.linked_work_quiet(graph, paper).await? else {
            return Ok((None, Vec::new()));
        };
        let workspace = workspace_of(graph).await?;
        let memberships = graph
            .lists
            .memberships_by_work(workspace, work, ArchiveScope::Active)
            .await
            .map_err(|error| database(&error))?;
        Ok((Some(work), memberships))
    }

    /// Active lists whose title matches `query`, for the picker.
    ///
    /// # Errors
    /// `Graph` for database failures.
    pub async fn search_lists(&self, query: &str) -> Result<Vec<List>, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        graph
            .lists
            .search_by_title(workspace, query, 20)
            .await
            .map_err(|error| database(&error))
    }

    /// Search held papers and graph Works by title, and with `provider` the `OpenAlex`
    /// title search, whose hits not yet in the graph have `in_graph: false`.
    ///
    /// # Errors
    /// `Invalid("invalid_query")` for a blank query; `Provider` when the provider fails.
    pub async fn search_works(
        &self,
        query: &str,
        provider: bool,
    ) -> Result<Vec<WorkHit>, ListServiceError> {
        let graph = graph_of(self)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(invalid("invalid_query", "A search needs some text."));
        }
        let mut hits = self.library_hits(graph, query).await?;
        let mut seen: HashSet<WorkId> = hits.iter().filter_map(|hit| hit.work_id).collect();
        let graph_matches = graph
            .works
            .find_by_title_key(&title_key(query), None)
            .await
            .map_err(|error| database(&error))?;
        for work in graph_matches {
            if !seen.insert(work.id) {
                continue;
            }
            hits.push(WorkHit {
                work_id: Some(work.id),
                paper_id: None,
                title: work.display_title.unwrap_or_default(),
                year: work.display_year,
                authors: work.display_authors.unwrap_or_default(),
                source: "graph",
                in_graph: true,
                openalex_id: None,
                doi: None,
            });
        }
        if provider {
            self.provider_hits(graph, query, &mut seen, &mut hits)
                .await?;
        }
        hits.truncate(SEARCH_LIMIT + PROVIDER_RESULTS_MAX);
        Ok(hits)
    }

    async fn library_hits(
        &self,
        graph: &GraphState,
        query: &str,
    ) -> Result<Vec<WorkHit>, ListServiceError> {
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let matching: Vec<_> = self
            .catalog
            .read()
            .await
            .overviews()
            .into_iter()
            .filter(|paper| {
                let haystack = format!(
                    "{} {}",
                    paper.metadata.title,
                    paper.metadata.authors.join(" ")
                )
                .to_lowercase();
                words.iter().all(|word| haystack.contains(word))
            })
            .take(SEARCH_LIMIT)
            .collect();
        let workspace = workspace_of(graph).await?;
        let mut hits = Vec::with_capacity(matching.len());
        for paper in matching {
            let work = match graph
                .documents
                .find_by_path(workspace, &paper.relative_path)
                .await
                .map_err(|error| database(&error))?
                .and_then(|document| document.work_id)
            {
                Some(work) => Some(resolved(graph, work).await?),
                None => None,
            };
            hits.push(WorkHit {
                work_id: work,
                paper_id: Some(paper.id),
                title: paper.metadata.title,
                year: paper.metadata.year.map(i32::from),
                authors: paper.metadata.authors,
                source: "library",
                in_graph: work.is_some(),
                openalex_id: None,
                doi: None,
            });
        }
        Ok(hits)
    }

    async fn provider_hits(
        &self,
        graph: &GraphState,
        query: &str,
        seen: &mut HashSet<WorkId>,
        hits: &mut Vec<WorkHit>,
    ) -> Result<(), ListServiceError> {
        let found =
            search_title_cached(&graph.source_records, self.literature.client(), query, None)
                .await
                .map_err(|error| ListServiceError::Provider(error.to_string()))?;
        let Some((_, summaries)) = found else {
            return Ok(());
        };
        for summary in summaries.into_iter().take(PROVIDER_RESULTS_MAX) {
            let Some(title) = summary.title else {
                continue;
            };
            let work = graph
                .works
                .find_by_identifier(IdentifierScheme::Openalex, &summary.openalex_id)
                .await
                .map_err(|error| database(&error))?;
            let work = match work {
                Some(work) => Some(resolved(graph, work.id).await?),
                None => None,
            };
            if let Some(work) = work
                && !seen.insert(work)
            {
                continue;
            }
            hits.push(WorkHit {
                work_id: work,
                paper_id: None,
                title,
                year: summary.year,
                authors: summary.authors,
                source: "provider",
                in_graph: work.is_some(),
                openalex_id: Some(summary.openalex_id),
                doi: summary.doi,
            });
        }
        Ok(())
    }

    /// Retry a list's `pending` claims. A claim that verifies becomes a Work node; one that
    /// settles on another state is replaced by a claim node at the same position.
    ///
    /// # Errors
    /// `NotFound("list_not_found")`; `Graph` for database failures.
    pub async fn verify_list(&self, list: ListId) -> Result<VerifyReport, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        Self::require_list(graph, workspace, list).await?;
        let mut report = VerifyReport::default();
        let nodes = graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| list_failure(&error))?;
        for node in nodes {
            if node.claim_state != Some(ClaimState::Pending) {
                continue;
            }
            let Some(claim) = claim_of(&node) else {
                continue;
            };
            report.checked += 1;
            match self.resolve_claim_text(graph, &claim).await? {
                ClaimResolution::Work { work, .. } => {
                    match graph.lists.resolve_claim(list, node.id, work.id).await {
                        Ok(_) => report.resolved += 1,
                        // The Work is already a member: the claim is a duplicate of it.
                        Err(DbError::Conflict(code)) if code == ListError::ItemExists.code() => {
                            report.still_pending += 1;
                        }
                        Err(error) => return Err(list_failure(&error)),
                    }
                }
                ClaimResolution::Unresolved {
                    state: ClaimState::Pending,
                    ..
                } => {
                    report.still_pending += 1;
                }
                ClaimResolution::Unresolved {
                    state, candidates, ..
                } => {
                    self.reclassify_claim(graph, list, &node, state, &candidates)
                        .await?;
                    report.reclassified += 1;
                }
            }
        }
        Ok(report)
    }

    /// Replace a claim node by one in `state`, keeping its place in the order.
    async fn reclassify_claim(
        &self,
        graph: &GraphState,
        list: ListId,
        node: &ListNode,
        state: ClaimState,
        candidates: &[ProviderRecord],
    ) -> Result<(), ListServiceError> {
        // The order is read before the node is replaced and used after, across awaits.
        #[allow(clippy::needless_collect)]
        let before: Vec<NodeId> = graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| list_failure(&error))?
            .into_iter()
            .map(|existing| existing.id)
            .collect();
        let Some(claim) = node.claim.clone() else {
            return Ok(());
        };
        graph
            .lists
            .remove_node(list, node.id)
            .await
            .map_err(|error| list_failure(&error))?;
        let mut replacement = claim_node(claim, state, candidates);
        replacement.note.clone_from(&node.note);
        replacement.rationale.clone_from(&node.rationale);
        replacement.context_links.clone_from(&node.context_links);
        replacement.source = node.source;
        replacement.review = node.review;
        replacement.generation_id = node.generation_id;
        let added = graph
            .lists
            .add_node(list, replacement)
            .await
            .map_err(|error| list_failure(&error))?;
        let order: Vec<NodeId> = before
            .into_iter()
            .map(|id| if id == node.id { added.id } else { id })
            .collect();
        graph
            .lists
            .reorder(list, &order)
            .await
            .map_err(|error| list_failure(&error))
    }

    /// A list from a `p...` ID or a unique case-insensitive title prefix.
    ///
    /// # Errors
    /// `NotFound("list_not_found")` when nothing matches; `Invalid("ambiguous_list", …)`
    /// listing up to five matches when several do.
    pub async fn list_reference(&self, text: &str) -> Result<ListId, ListServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let text = text.trim();
        if let Ok(id) = text.parse::<ListId>()
            && graph
                .lists
                .get(workspace, id)
                .await
                .map_err(|error| database(&error))?
                .is_some()
        {
            return Ok(id);
        }
        let needle = text.to_lowercase();
        if needle.is_empty() {
            return Err(ListServiceError::NotFound("list_not_found"));
        }
        let lists = graph
            .lists
            .list(workspace, None, ArchiveScope::All)
            .await
            .map_err(|error| database(&error))?;
        let matches: Vec<&List> = lists
            .iter()
            .map(|summary| &summary.list)
            .filter(|list| list.title.to_lowercase().starts_with(&needle))
            .collect();
        match matches.as_slice() {
            [] => Err(ListServiceError::NotFound("list_not_found")),
            [only] => Ok(only.id),
            several => {
                let exact: Vec<&&List> = several
                    .iter()
                    .filter(|list| list.title.to_lowercase() == needle)
                    .collect();
                if let [only] = exact.as_slice() {
                    return Ok(only.id);
                }
                let listed: Vec<String> = several
                    .iter()
                    .take(AMBIGUOUS_MATCHES_SHOWN)
                    .map(|list| format!("{} ({})", list.title, list.id))
                    .collect();
                let more = several.len().saturating_sub(AMBIGUOUS_MATCHES_SHOWN);
                let tail = if more > 0 {
                    format!("; and {more} more")
                } else {
                    String::new()
                };
                Err(ListServiceError::Invalid(
                    "ambiguous_list",
                    format!(
                        "\"{text}\" matches {} lists: {}{tail}",
                        several.len(),
                        listed.join("; ")
                    ),
                ))
            }
        }
    }

    /// A node of a list from an `n...` ID or a Work reference (the node holding that Work).
    ///
    /// # Errors
    /// `NotFound("node_not_found")` when nothing in the list matches.
    pub async fn list_node_reference(
        &self,
        list: ListId,
        text: &str,
    ) -> Result<NodeId, ListServiceError> {
        let graph = graph_of(self)?;
        let text = text.trim();
        if let Ok(id) = text.parse::<NodeId>()
            && graph
                .lists
                .node(list, id)
                .await
                .map_err(|error| list_failure(&error))?
                .is_some()
        {
            return Ok(id);
        }
        let work = self
            .resolve_work_argument(text)
            .await
            .map_err(|_| ListServiceError::NotFound("node_not_found"))?;
        graph
            .lists
            .node_for_work(list, work)
            .await
            .map_err(|error| list_failure(&error))?
            .map(|node| node.id)
            .ok_or(ListServiceError::NotFound("node_not_found"))
    }
}

fn work_node(
    work: WorkId,
    via: Option<WorkId>,
    topics: Vec<NodeId>,
    note: Option<String>,
) -> NewNode {
    let mut new = NewNode::work(work);
    new.content = NewNodeContent::Work {
        work_id: work,
        via,
        topics,
    };
    new.note = note;
    new
}

fn claim_node(claim: Value, state: ClaimState, candidates: &[ProviderRecord]) -> NewNode {
    let mut node = NewNode::claim(claim.clone(), state);
    node.content = NewNodeContent::Claim {
        claim,
        state,
        candidates: serde_json::to_value(candidates).unwrap_or(Value::Array(Vec::new())),
    };
    node
}

fn claim_json(claim: &Claim) -> Value {
    serde_json::to_value(claim).unwrap_or_else(|_| json!({ "title": claim.title }))
}

/// Older first, unknown years last, then title key and node ID.
fn year_order(items: &[LinearItem]) -> Vec<NodeId> {
    let mut sorted: Vec<&LinearItem> = items.iter().collect();
    sorted.sort_by(|left, right| {
        let year = match (left.year, right.year) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        year.then_with(|| left.title_key.cmp(&right.title_key))
            .then_with(|| left.id.cmp(&right.id))
    });
    sorted.into_iter().map(|item| item.id).collect()
}

/// Insert every topic just before the first item it covers (through subtopics), keeping the
/// topics' own relative order; topics covering nothing go last.
fn with_topics(nodes: &[ListNode], edges: &[ListEdge], items: Vec<NodeId>) -> Vec<NodeId> {
    let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in edges {
        if matches!(edge.kind, EdgeKind::Covers | EdgeKind::Subtopic) {
            children.entry(edge.from).or_default().push(edge.to);
        }
    }
    let rank: HashMap<NodeId, usize> = items
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    let mut topics: Vec<&ListNode> = nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Topic)
        .collect();
    topics.sort_by_key(|topic| (topic.position, topic.id));
    let mut before: HashMap<usize, Vec<NodeId>> = HashMap::new();
    let mut last = Vec::new();
    for topic in topics {
        let mut seen = HashSet::new();
        let mut stack = vec![topic.id];
        let mut first: Option<usize> = None;
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            if let Some(index) = rank.get(&current) {
                first = Some(first.map_or(*index, |best| best.min(*index)));
            }
            stack.extend(children.get(&current).into_iter().flatten().copied());
        }
        match first {
            Some(index) => before.entry(index).or_default().push(topic.id),
            None => last.push(topic.id),
        }
    }
    let mut order = Vec::with_capacity(nodes.len());
    for (index, item) in items.into_iter().enumerate() {
        if let Some(topics) = before.remove(&index) {
            order.extend(topics);
        }
        order.push(item);
    }
    order.extend(last);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(label: &str, position: i32) -> ListNode {
        node_of(NodeKind::Topic, Some(label), position)
    }

    fn node_of(kind: NodeKind, label: Option<&str>, position: i32) -> ListNode {
        ListNode {
            id: NodeId::new(),
            list_id: ListId::new(),
            kind,
            work_id: (kind == NodeKind::Work).then(WorkId::new),
            label: label.map(str::to_owned),
            summary: None,
            claim: None,
            claim_state: None,
            candidates: Value::Array(Vec::new()),
            position,
            note: None,
            rationale: None,
            context_links: Value::Array(Vec::new()),
            source: lysilogy_core::NodeSource::User,
            review: ListReview::Accepted,
            via_work_id: None,
            path: None,
            generation_id: None,
            added_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn covers(from: &ListNode, to: &ListNode) -> ListEdge {
        ListEdge {
            from: from.id,
            to: to.id,
            kind: EdgeKind::Covers,
        }
    }

    #[test]
    fn should_place_topic_before_its_first_work_when_interleaving_topics() {
        let first = node_of(NodeKind::Work, None, 0);
        let second = node_of(NodeKind::Work, None, 1);
        let third = node_of(NodeKind::Work, None, 2);
        let topic = topic("Alignment", 3);
        let nodes = vec![first.clone(), second.clone(), third.clone(), topic.clone()];
        let order = with_topics(
            &nodes,
            &[covers(&topic, &second), covers(&topic, &third)],
            vec![first.id, second.id, third.id],
        );
        assert_eq!(order, vec![first.id, topic.id, second.id, third.id]);
    }

    #[test]
    fn should_put_uncovering_topics_last_when_interleaving_topics() {
        let work = node_of(NodeKind::Work, None, 0);
        let empty = topic("Empty", 1);
        let nodes = vec![work.clone(), empty.clone()];
        let order = with_topics(&nodes, &[], vec![work.id]);
        assert_eq!(order, vec![work.id, empty.id]);
    }

    #[test]
    fn should_reach_works_through_subtopics_when_interleaving_topics() {
        let work = node_of(NodeKind::Work, None, 0);
        let parent = topic("Parent", 1);
        let child = topic("Child", 2);
        let nodes = vec![work.clone(), parent.clone(), child.clone()];
        let edges = [
            ListEdge {
                from: parent.id,
                to: child.id,
                kind: EdgeKind::Subtopic,
            },
            covers(&child, &work),
        ];
        let order = with_topics(&nodes, &edges, vec![work.id]);
        assert_eq!(order, vec![parent.id, child.id, work.id]);
    }

    #[test]
    fn should_order_unknown_years_last_when_sorting_by_year() {
        let item = |year: Option<i32>, key: &str| LinearItem {
            id: NodeId::new(),
            work_id: None,
            depth: 0,
            score: 0.0,
            year,
            title_key: key.to_owned(),
            is_seed: false,
        };
        let late = item(Some(2020), "a");
        let early = item(Some(1999), "b");
        let unknown = item(None, "c");
        let order = year_order(&[late.clone(), unknown.clone(), early.clone()]);
        assert_eq!(order, vec![early.id, late.id, unknown.id]);
    }

    #[test]
    fn should_fill_identifiers_from_text_when_normalizing_claim() {
        let claim = normalized_claim(Claim {
            title: "  See 10.1234/abc.5 and arXiv:2401.00001  ".to_owned(),
            ..Claim::default()
        })
        .expect("claim");
        assert_eq!(claim.doi.as_deref(), Some("10.1234/abc.5"));
        assert_eq!(claim.arxiv.as_deref(), Some("2401.00001"));
    }

    #[test]
    fn should_reject_blank_title_when_normalizing_claim() {
        let error = normalized_claim(Claim {
            title: "   ".to_owned(),
            ..Claim::default()
        })
        .expect_err("blank");
        assert_eq!(error.code(), "invalid_claim");
    }

    #[test]
    fn should_keep_given_identifiers_when_text_names_others() {
        let claim = normalized_claim(Claim {
            title: "Paper".to_owned(),
            doi: Some("10.1/given".to_owned()),
            raw: Some("10.2/other".to_owned()),
            ..Claim::default()
        })
        .expect("claim");
        assert_eq!(claim.doi.as_deref(), Some("10.1/given"));
    }

    #[test]
    fn should_parse_strategies_when_spelled_with_dashes_or_underscores() {
        assert_eq!(
            parse_strategy("foundational-first").expect("strategy"),
            OrderStrategy::FoundationalFirst
        );
        assert_eq!(
            parse_strategy("foundational_first").expect("strategy"),
            OrderStrategy::FoundationalFirst
        );
        assert_eq!(
            parse_strategy("topological").expect("strategy"),
            OrderStrategy::Topological
        );
        assert_eq!(
            parse_strategy("alphabetical").expect_err("unknown").code(),
            "invalid_strategy"
        );
    }

    #[test]
    fn should_map_repository_conflicts_to_service_errors_when_codes_are_known() {
        let cases = [
            ("item_exists", "Conflict"),
            ("list_full", "Conflict"),
            ("invalid_order", "Invalid"),
            ("node_not_found", "NotFound"),
            ("not_a_claim", "Conflict"),
            ("invalid_node_kind", "Invalid"),
        ];
        for (code, variant) in cases {
            let mapped = list_failure(&DbError::Conflict(code.to_owned()));
            assert_eq!(mapped.code(), code);
            let found = match mapped {
                ListServiceError::Conflict(..) => "Conflict",
                ListServiceError::Invalid(..) => "Invalid",
                ListServiceError::NotFound(_) => "NotFound",
                _ => "other",
            };
            assert_eq!(found, variant, "{code}");
        }
        assert_eq!(
            list_failure(&DbError::Conflict("surprise".to_owned())).code(),
            "graph_error"
        );
    }
}
