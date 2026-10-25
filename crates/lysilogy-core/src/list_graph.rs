//! The reading list graph: edge validation, topological linearization, and the frontier.
//!
//! Everything here is pure. Stored edges are `prerequisite` (read the source first),
//! `covers` (topic to Work) and `subtopic` (topic to topic); citation edges among member
//! Works are derived by the caller and passed in as `(citing, cited)` pairs.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::BuildHasher;

use crate::{EdgeKind, ListError, NodeId, NodeKind, ReadingStatus, WorkId};

/// One stored edge of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
}

/// Why an edge was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeRejection {
    /// The kind does not fit the endpoints (`covers` must be topic to Work, and so on).
    InvalidKind,
    /// An endpoint is not a node of the list.
    UnknownNode,
    /// The edge would close a cycle in the ordering the list is read in. When the stored
    /// `prerequisite` / `subtopic` edges themselves cycle, the nodes are listed in edge order
    /// starting at the new edge's source: `from`, `to`, ..., and the last one points back at
    /// `from`. A self edge is the single node `[from]`. When the cycle only appears once
    /// topics are expanded to their Works (D8), the nodes are those Works, starting at the
    /// smallest node ID.
    Cycle(Vec<NodeId>),
}

impl EdgeRejection {
    /// The list error carrying the API code.
    #[must_use]
    pub const fn list_error(&self) -> ListError {
        match self {
            Self::InvalidKind => ListError::InvalidEdgeKind,
            Self::UnknownNode => ListError::NodeNotFound,
            Self::Cycle(_) => ListError::EdgeCycle,
        }
    }

    /// The cycle to name in a 409 `edge_cycle`.
    #[must_use]
    pub fn cycle(&self) -> Option<&[NodeId]> {
        match self {
            Self::Cycle(nodes) => Some(nodes),
            _ => None,
        }
    }
}

fn kinds_fit(kind: EdgeKind, from: NodeKind, to: NodeKind) -> bool {
    use NodeKind::{Topic, Work};
    match kind {
        EdgeKind::Covers => from == Topic && to == Work,
        EdgeKind::Subtopic => from == Topic && to == Topic,
        // Claims are unverified, so nothing may depend on them yet.
        EdgeKind::Prerequisite => from != NodeKind::Claim && to != NodeKind::Claim,
    }
}

const fn is_ordering(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::Prerequisite | EdgeKind::Subtopic)
}

/// Check a new edge against the list (D8).
///
/// `nodes` maps every node of the list to its kind. `prerequisite` and `subtopic` edges
/// together must stay acyclic; the shortest closing cycle is returned. An edge that is
/// already stored is accepted (the store is idempotent).
pub fn validate_edge<S: BuildHasher>(
    nodes: &HashMap<NodeId, NodeKind, S>,
    existing: &[ListEdge],
    edge: ListEdge,
) -> Result<(), EdgeRejection> {
    let (Some(&from_kind), Some(&to_kind)) = (nodes.get(&edge.from), nodes.get(&edge.to)) else {
        return Err(EdgeRejection::UnknownNode);
    };
    if !kinds_fit(edge.kind, from_kind, to_kind) {
        return Err(EdgeRejection::InvalidKind);
    }
    if edge.from == edge.to && is_ordering(edge.kind) {
        return Err(EdgeRejection::Cycle(vec![edge.from]));
    }
    if is_ordering(edge.kind)
        && let Some(cycle) = raw_cycle(existing, edge)
    {
        return Err(EdgeRejection::Cycle(cycle));
    }
    expanded_cycle(nodes, existing, edge).map_or(Ok(()), |cycle| Err(EdgeRejection::Cycle(cycle)))
}

/// The shortest cycle the new `prerequisite` / `subtopic` edge closes among the stored ones.
fn raw_cycle(existing: &[ListEdge], edge: ListEdge) -> Option<Vec<NodeId>> {
    let mut successors: HashMap<NodeId, BTreeSet<NodeId>> = HashMap::new();
    for stored in existing.iter().filter(|stored| is_ordering(stored.kind)) {
        successors.entry(stored.from).or_default().insert(stored.to);
    }
    // Breadth first from the new target; reaching the new source closes a cycle.
    let mut parent: HashMap<NodeId, NodeId> = HashMap::new();
    let mut seen = HashSet::from([edge.to]);
    let mut queue = VecDeque::from([edge.to]);
    while let Some(node) = queue.pop_front() {
        if node == edge.from {
            let mut path = vec![node];
            let mut cursor = node;
            while let Some(&before) = parent.get(&cursor) {
                path.push(before);
                cursor = before;
            }
            path.reverse();
            // `path` runs to ..= from; the cycle starts at from and stops before it returns.
            let mut cycle = vec![edge.from];
            cycle.extend(&path[..path.len() - 1]);
            return Some(cycle);
        }
        let Some(next) = successors.get(&node) else {
            continue;
        };
        for &successor in next {
            if seen.insert(successor) {
                parent.insert(successor, node);
                queue.push_back(successor);
            }
        }
    }
    None
}

/// The Works a node stands for: itself, or every Work under a topic (subtopics included).
fn work_members<S: BuildHasher>(
    nodes: &HashMap<NodeId, NodeKind, S>,
    covered: &HashMap<NodeId, BTreeSet<NodeId>>,
    node: NodeId,
) -> BTreeSet<NodeId> {
    let mut found = BTreeSet::new();
    let mut seen = HashSet::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if !seen.insert(current) {
            continue;
        }
        if nodes.get(&current) == Some(&NodeKind::Work) {
            found.insert(current);
            continue;
        }
        if let Some(children) = covered.get(&current) {
            stack.extend(children);
        }
    }
    found
}

/// The Work-to-Work "read first" relation that `linearize` and `frontier` enforce: every
/// `prerequisite` edge expanded through `covers` and `subtopic`, minus a Work with itself.
fn work_order<S: BuildHasher>(
    nodes: &HashMap<NodeId, NodeKind, S>,
    edges: impl Iterator<Item = ListEdge> + Clone,
) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut covered: HashMap<NodeId, BTreeSet<NodeId>> = HashMap::new();
    for edge in edges.clone() {
        if matches!(edge.kind, EdgeKind::Covers | EdgeKind::Subtopic) {
            covered.entry(edge.from).or_default().insert(edge.to);
        }
    }
    let mut order: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for edge in edges.filter(|e| e.kind == EdgeKind::Prerequisite) {
        let after = work_members(nodes, &covered, edge.to);
        for first in work_members(nodes, &covered, edge.from) {
            order
                .entry(first)
                .or_default()
                .extend(after.iter().copied().filter(|second| *second != first));
        }
    }
    order
}

/// A cycle in a Work order, starting at its smallest node, or `None` when it is acyclic.
fn find_work_cycle(order: &BTreeMap<NodeId, BTreeSet<NodeId>>) -> Option<Vec<NodeId>> {
    fn visit(
        order: &BTreeMap<NodeId, BTreeSet<NodeId>>,
        node: NodeId,
        path: &mut Vec<NodeId>,
        done: &mut HashSet<NodeId>,
    ) -> Option<Vec<NodeId>> {
        if let Some(from) = path.iter().position(|seen| *seen == node) {
            let mut cycle = path[from..].to_vec();
            let smallest = (0..cycle.len()).min_by_key(|&i| cycle[i])?;
            cycle.rotate_left(smallest);
            return Some(cycle);
        }
        if done.contains(&node) {
            return None;
        }
        path.push(node);
        for &next in order.get(&node).into_iter().flatten() {
            if let Some(cycle) = visit(order, next, path, done) {
                return Some(cycle);
            }
        }
        path.pop();
        done.insert(node);
        None
    }
    let mut done = HashSet::new();
    order
        .keys()
        .find_map(|&start| visit(order, start, &mut Vec::new(), &mut done))
}

/// The cycle the new edge adds to the expanded Work order, if it adds one. A list whose
/// stored edges already cycle accepts further edges that add nothing new.
fn expanded_cycle<S: BuildHasher>(
    nodes: &HashMap<NodeId, NodeKind, S>,
    existing: &[ListEdge],
    edge: ListEdge,
) -> Option<Vec<NodeId>> {
    let with_edge = work_order(nodes, existing.iter().copied().chain(std::iter::once(edge)));
    let cycle = find_work_cycle(&with_edge)?;
    if find_work_cycle(&work_order(nodes, existing.iter().copied())).is_some() {
        return None;
    }
    Some(cycle)
}

/// What `linearize` needs to know about one Work or claim node.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearItem {
    pub id: NodeId,
    /// `None` for claims, which take no part in citation constraints.
    pub work_id: Option<WorkId>,
    pub depth: usize,
    pub score: f64,
    pub year: Option<i32>,
    pub title_key: String,
    pub is_seed: bool,
}

/// Higher priority sorts first: deeper, then more prominent, then older (unknown last),
/// then by title key, Work ID and node ID. Same rule as `suggest_order`.
fn priority(left: &LinearItem, right: &LinearItem) -> std::cmp::Ordering {
    right
        .depth
        .cmp(&left.depth)
        .then_with(|| right.score.total_cmp(&left.score))
        .then_with(|| match (left.year, right.year) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| left.title_key.cmp(&right.title_key))
        .then_with(|| left.work_id.cmp(&right.work_id))
        .then_with(|| left.id.cmp(&right.id))
}

/// Adjacency used to expand an edge endpoint into the items it stands for.
struct Membership {
    index_of: HashMap<NodeId, usize>,
    /// Direct `covers` and `subtopic` children of every topic.
    covered: HashMap<NodeId, BTreeSet<NodeId>>,
}

impl Membership {
    fn new(items: &[&LinearItem], edges: &[ListEdge]) -> Self {
        let mut covered: HashMap<NodeId, BTreeSet<NodeId>> = HashMap::new();
        for edge in edges {
            if matches!(edge.kind, EdgeKind::Covers | EdgeKind::Subtopic) {
                covered.entry(edge.from).or_default().insert(edge.to);
            }
        }
        Self {
            index_of: items
                .iter()
                .enumerate()
                .map(|(index, item)| (item.id, index))
                .collect(),
            covered,
        }
    }

    /// The item indices a node stands for: itself, or every item under a topic (through
    /// subtopics too). Unknown nodes stand for nothing.
    fn members(&self, node: NodeId) -> BTreeSet<usize> {
        let mut found = BTreeSet::new();
        let mut seen = HashSet::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            if let Some(&index) = self.index_of.get(&current) {
                found.insert(index);
                continue;
            }
            if let Some(children) = self.covered.get(&current) {
                stack.extend(children);
            }
        }
        found
    }
}

/// The topological linearization of a list's Work and claim nodes (D9).
///
/// Constraints, strongest first: stored `prerequisite` edges (a topic endpoint stands for
/// every Work under it, subtopics included), then derived citations (`cites` holds
/// `(citing, cited)` pairs; the cited Work comes first). Ties break by depth, prominence,
/// older year, title key, Work ID and node ID. A topic's Works stay contiguous where the
/// constraints allow: after placing an item, a free item sharing its topic (the covering
/// topic with the most Works, then the smallest ID) goes next. The seed always comes last,
/// and constraints involving it are ignored. When only cycles remain, citation constraints
/// are dropped first, then the best remaining item is taken. With no topics and no
/// prerequisite edges the result equals `suggest_order`. The result does not depend on the
/// order of the inputs, and a repeated item ID keeps its first entry.
#[must_use]
pub fn linearize(
    items: &[LinearItem],
    edges: &[ListEdge],
    cites: &[(WorkId, WorkId)],
) -> Vec<NodeId> {
    let mut seen = HashSet::new();
    let unique: Vec<&LinearItem> = items.iter().filter(|item| seen.insert(item.id)).collect();
    let (seeds, body): (Vec<&LinearItem>, Vec<&LinearItem>) =
        unique.into_iter().partition(|item| item.is_seed);

    let membership = Membership::new(&body, edges);
    let mut unmet_hard = vec![0usize; body.len()];
    let mut unmet_soft = vec![0usize; body.len()];
    let mut hard_dependants: Vec<Vec<usize>> = vec![Vec::new(); body.len()];
    let mut soft_dependants: Vec<Vec<usize>> = vec![Vec::new(); body.len()];

    let mut hard_pairs = HashSet::new();
    for edge in edges.iter().filter(|e| e.kind == EdgeKind::Prerequisite) {
        let (before, after) = (membership.members(edge.from), membership.members(edge.to));
        for &first in &before {
            for &second in &after {
                if first != second && hard_pairs.insert((first, second)) {
                    unmet_hard[second] += 1;
                    hard_dependants[first].push(second);
                }
            }
        }
    }

    let by_work: HashMap<WorkId, usize> = body
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.work_id.map(|work| (work, index)))
        .collect();
    let mut soft_pairs = HashSet::new();
    for &(citing, cited) in cites {
        let (Some(&citing_index), Some(&cited_index)) = (by_work.get(&citing), by_work.get(&cited))
        else {
            continue;
        };
        if citing_index != cited_index && soft_pairs.insert((cited_index, citing_index)) {
            unmet_soft[citing_index] += 1;
            soft_dependants[cited_index].push(citing_index);
        }
    }

    let primary_topic = primary_topics(&body, edges, &membership);

    let mut remaining: Vec<usize> = (0..body.len()).collect();
    let mut ordered = Vec::with_capacity(items.len());
    let mut last_topic: Option<NodeId> = None;
    while !remaining.is_empty() {
        let mut pool: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|&index| unmet_hard[index] == 0 && unmet_soft[index] == 0)
            .collect();
        if pool.is_empty() {
            pool = remaining
                .iter()
                .copied()
                .filter(|&index| unmet_hard[index] == 0)
                .collect();
        }
        if pool.is_empty() {
            pool.clone_from(&remaining);
        }
        if let Some(topic) = last_topic {
            let same: Vec<usize> = pool
                .iter()
                .copied()
                .filter(|&index| primary_topic[index] == Some(topic))
                .collect();
            if !same.is_empty() {
                pool = same;
            }
        }
        let pick = pool
            .into_iter()
            .min_by(|&a, &b| priority(body[a], body[b]))
            .expect("pool is not empty");
        remaining.retain(|&index| index != pick);
        ordered.push(body[pick].id);
        last_topic = primary_topic[pick];
        for &dependant in &hard_dependants[pick] {
            unmet_hard[dependant] = unmet_hard[dependant].saturating_sub(1);
        }
        for &dependant in &soft_dependants[pick] {
            unmet_soft[dependant] = unmet_soft[dependant].saturating_sub(1);
        }
    }

    let mut seeds = seeds;
    seeds.sort_by(|left, right| priority(left, right));
    ordered.extend(seeds.into_iter().map(|item| item.id));
    ordered
}

/// Each item's grouping topic: the directly covering topic with the most items under it,
/// then the smallest ID. Grouping is by that direct topic only: Works under a subtopic do
/// not join their parent topic's direct Works.
fn primary_topics(
    body: &[&LinearItem],
    edges: &[ListEdge],
    membership: &Membership,
) -> Vec<Option<NodeId>> {
    let mut covering: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); body.len()];
    let mut size: HashMap<NodeId, usize> = HashMap::new();
    for edge in edges.iter().filter(|e| e.kind == EdgeKind::Covers) {
        let Some(&index) = membership.index_of.get(&edge.to) else {
            continue;
        };
        if covering[index].insert(edge.from) {
            *size.entry(edge.from).or_default() += 1;
        }
    }
    covering
        .into_iter()
        .map(|topics| {
            topics
                .into_iter()
                .max_by(|a, b| size[a].cmp(&size[b]).then_with(|| b.cmp(a)))
        })
        .collect()
}

/// For every Work, the unread-or-reading Works that must be read before it, in node ID
/// order. Built in one pass: each topic endpoint is expanded once.
fn blockers_by_work(
    works: &[(NodeId, ReadingStatus)],
    edges: &[ListEdge],
) -> HashMap<NodeId, BTreeSet<NodeId>> {
    let status: HashMap<NodeId, ReadingStatus> = works.iter().copied().collect();
    let mut covered: HashMap<NodeId, BTreeSet<NodeId>> = HashMap::new();
    for edge in edges {
        if matches!(edge.kind, EdgeKind::Covers | EdgeKind::Subtopic) {
            covered.entry(edge.from).or_default().insert(edge.to);
        }
    }
    let kinds: HashMap<NodeId, NodeKind> = status.keys().map(|id| (*id, NodeKind::Work)).collect();
    let mut blockers: HashMap<NodeId, BTreeSet<NodeId>> = HashMap::new();
    for edge in edges.iter().filter(|e| e.kind == EdgeKind::Prerequisite) {
        let unread: BTreeSet<NodeId> = work_members(&kinds, &covered, edge.from)
            .into_iter()
            .filter(|node| status[node] != ReadingStatus::Read)
            .collect();
        for target in work_members(&kinds, &covered, edge.to) {
            blockers
                .entry(target)
                .or_default()
                .extend(unread.iter().copied().filter(|node| *node != target));
        }
    }
    blockers
}

/// The unread-or-reading Works that must be read before `target`, in node ID order.
///
/// A `prerequisite` edge into `target` or into a topic above it (subtopics included) names
/// every Work under its source. Only Works in `works` count, `target` itself is skipped, and
/// the check is direct: a prerequisite that is `read` no longer blocks, whatever its own
/// prerequisites say.
#[must_use]
pub fn unmet_prerequisites(
    works: &[(NodeId, ReadingStatus)],
    edges: &[ListEdge],
    target: NodeId,
) -> Vec<NodeId> {
    blockers_by_work(works, edges)
        .remove(&target)
        .map_or_else(Vec::new, |found| found.into_iter().collect())
}

/// "Next up": Works that are `unread` and whose prerequisites are all `read`, in the order
/// given by `order` (a linearization; Works missing from it go last by node ID).
#[must_use]
pub fn frontier(
    works: &[(NodeId, ReadingStatus)],
    edges: &[ListEdge],
    order: &[NodeId],
) -> Vec<NodeId> {
    let rank: HashMap<NodeId, usize> = order
        .iter()
        .enumerate()
        .rev()
        .map(|(index, id)| (*id, index))
        .collect();
    let blockers = blockers_by_work(works, edges);
    let mut ready: Vec<NodeId> = works
        .iter()
        .filter(|(_, status)| *status == ReadingStatus::Unread)
        .map(|(id, _)| *id)
        .filter(|id| blockers.get(id).is_none_or(BTreeSet::is_empty))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ready.sort_by_key(|id| (rank.get(id).copied().unwrap_or(usize::MAX), *id));
    ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ListOrderItem, suggest_order};
    use uuid::Uuid;

    fn node(number: u128) -> NodeId {
        NodeId::from_uuid(Uuid::from_u128(number))
    }

    fn work(number: u128) -> WorkId {
        WorkId::from_uuid(Uuid::from_u128(number))
    }

    fn item(number: u128, depth: usize, score: f64, year: Option<i32>) -> LinearItem {
        LinearItem {
            id: node(number),
            work_id: Some(work(number)),
            depth,
            score,
            year,
            title_key: format!("title {number}"),
            is_seed: false,
        }
    }

    fn edge(from: u128, to: u128, kind: EdgeKind) -> ListEdge {
        ListEdge {
            from: node(from),
            to: node(to),
            kind,
        }
    }

    fn kinds(spec: &[(u128, NodeKind)]) -> HashMap<NodeId, NodeKind> {
        spec.iter().map(|&(n, kind)| (node(n), kind)).collect()
    }

    fn ids(numbers: &[u128]) -> Vec<NodeId> {
        numbers.iter().map(|&n| node(n)).collect()
    }

    #[test]
    fn should_accept_edges_that_fit_their_endpoints_when_validating() {
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (10, NodeKind::Topic),
            (11, NodeKind::Topic),
        ]);
        for ok in [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 11, EdgeKind::Subtopic),
            edge(1, 2, EdgeKind::Prerequisite),
            edge(1, 10, EdgeKind::Prerequisite),
            edge(10, 1, EdgeKind::Prerequisite),
            edge(10, 11, EdgeKind::Prerequisite),
        ] {
            assert_eq!(validate_edge(&nodes, &[], ok), Ok(()), "{ok:?}");
        }
    }

    #[test]
    fn should_reject_invalid_kinds_when_endpoints_do_not_fit() {
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (3, NodeKind::Claim),
            (10, NodeKind::Topic),
            (11, NodeKind::Topic),
        ]);
        for bad in [
            edge(1, 10, EdgeKind::Covers),
            edge(1, 2, EdgeKind::Covers),
            edge(10, 11, EdgeKind::Covers),
            edge(10, 3, EdgeKind::Covers),
            edge(1, 2, EdgeKind::Subtopic),
            edge(10, 1, EdgeKind::Subtopic),
            edge(1, 3, EdgeKind::Prerequisite),
            edge(3, 1, EdgeKind::Prerequisite),
        ] {
            let rejection = validate_edge(&nodes, &[], bad).expect_err("rejected");
            assert_eq!(rejection, EdgeRejection::InvalidKind, "{bad:?}");
            assert_eq!(rejection.list_error().code(), "invalid_edge_kind");
        }
    }

    #[test]
    fn should_reject_unknown_nodes_when_validating() {
        let nodes = kinds(&[(1, NodeKind::Work)]);
        let rejection =
            validate_edge(&nodes, &[], edge(1, 9, EdgeKind::Prerequisite)).expect_err("unknown");
        assert_eq!(rejection, EdgeRejection::UnknownNode);
        assert_eq!(rejection.list_error().code(), "node_not_found");
        assert!(rejection.cycle().is_none());
    }

    #[test]
    fn should_name_the_cycle_when_prerequisite_edge_closes_one() {
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (3, NodeKind::Work),
            (4, NodeKind::Work),
        ]);
        let existing = [
            edge(2, 3, EdgeKind::Prerequisite),
            edge(3, 1, EdgeKind::Prerequisite),
            edge(2, 4, EdgeKind::Prerequisite),
        ];
        let rejection = validate_edge(&nodes, &existing, edge(1, 2, EdgeKind::Prerequisite))
            .expect_err("cycle");
        assert_eq!(rejection.cycle(), Some(ids(&[1, 2, 3]).as_slice()));
        assert_eq!(rejection.list_error().code(), "edge_cycle");
        // The same edge in the safe direction is fine.
        assert_eq!(
            validate_edge(&nodes, &existing, edge(4, 1, EdgeKind::Prerequisite)),
            Ok(())
        );
    }

    #[test]
    fn should_reject_self_edges_as_one_node_cycles() {
        let nodes = kinds(&[(1, NodeKind::Work), (10, NodeKind::Topic)]);
        assert_eq!(
            validate_edge(&nodes, &[], edge(1, 1, EdgeKind::Prerequisite)),
            Err(EdgeRejection::Cycle(ids(&[1])))
        );
        assert_eq!(
            validate_edge(&nodes, &[], edge(10, 10, EdgeKind::Subtopic)),
            Err(EdgeRejection::Cycle(ids(&[10])))
        );
    }

    #[test]
    fn should_count_subtopic_and_prerequisite_edges_together_when_checking_cycles() {
        let nodes = kinds(&[(10, NodeKind::Topic), (11, NodeKind::Topic)]);
        let existing = [edge(10, 11, EdgeKind::Subtopic)];
        let rejection = validate_edge(&nodes, &existing, edge(11, 10, EdgeKind::Prerequisite))
            .expect_err("cycle");
        assert_eq!(rejection.cycle(), Some(ids(&[11, 10]).as_slice()));
    }

    #[test]
    fn should_ignore_covers_edges_and_duplicates_when_checking_cycles() {
        let nodes = kinds(&[(1, NodeKind::Work), (10, NodeKind::Topic)]);
        // Work 1 is under topic 10 and also a prerequisite of topic 10: not a stored cycle.
        let existing = [
            edge(10, 1, EdgeKind::Covers),
            edge(1, 10, EdgeKind::Prerequisite),
        ];
        assert_eq!(
            validate_edge(&nodes, &existing, edge(10, 1, EdgeKind::Covers)),
            Ok(())
        );
        assert_eq!(
            validate_edge(&nodes, &existing, edge(1, 10, EdgeKind::Prerequisite)),
            Ok(())
        );
    }

    fn linear_ids(items: &[LinearItem], edges: &[ListEdge], cites: &[(u128, u128)]) -> Vec<NodeId> {
        let cites: Vec<(WorkId, WorkId)> = cites.iter().map(|&(a, b)| (work(a), work(b))).collect();
        linearize(items, edges, &cites)
    }

    #[test]
    fn should_match_suggest_order_when_there_are_no_topics_or_prerequisites() {
        let spec = [
            (1, 2, 0.4, Some(2001)),
            (2, 2, 0.4, Some(2001)),
            (3, 1, 0.8, Some(1999)),
            (4, 3, 0.1, None),
            (5, 1, 0.8, Some(1999)),
            (6, 0, 1.0, Some(2020)),
        ];
        let linear: Vec<LinearItem> = spec
            .iter()
            .map(|&(n, depth, score, year)| LinearItem {
                is_seed: n == 6,
                ..item(n, depth, score, year)
            })
            .collect();
        let ramp: Vec<ListOrderItem> = linear
            .iter()
            .map(|i| ListOrderItem {
                work_id: i.work_id.expect("work"),
                depth: i.depth,
                score: i.score,
                year: i.year,
                title_key: i.title_key.clone(),
                is_seed: i.is_seed,
            })
            .collect();
        for cites in [
            vec![],
            vec![(1, 3), (3, 4), (4, 1), (2, 5)],
            vec![(1, 2), (1, 6), (6, 2), (9, 1), (1, 1)],
        ] {
            let pairs: Vec<(WorkId, WorkId)> =
                cites.iter().map(|&(a, b)| (work(a), work(b))).collect();
            let expected: Vec<NodeId> = suggest_order(&ramp, &pairs)
                .into_iter()
                .map(|w| node(w.as_uuid().as_u128()))
                .collect();
            assert_eq!(linear_ids(&linear, &[], &cites), expected, "{cites:?}");
        }
    }

    #[test]
    fn should_put_prerequisite_before_dependant_when_priority_says_otherwise() {
        // Item 1 outranks item 2 on depth, but 2 is a prerequisite of 1.
        let items = [item(1, 5, 0.9, Some(2000)), item(2, 1, 0.1, Some(2020))];
        assert_eq!(linear_ids(&items, &[], &[]), ids(&[1, 2]));
        let edges = [edge(2, 1, EdgeKind::Prerequisite)];
        assert_eq!(linear_ids(&items, &edges, &[]), ids(&[2, 1]));
    }

    #[test]
    fn should_let_prerequisites_beat_citations_when_they_conflict() {
        // Citations say 2 first (1 cites 2); the prerequisite says 1 first.
        let items = [item(1, 1, 0.5, None), item(2, 1, 0.5, None)];
        let edges = [edge(1, 2, EdgeKind::Prerequisite)];
        assert_eq!(linear_ids(&items, &edges, &[(1, 2)]), ids(&[1, 2]));
    }

    #[test]
    fn should_expand_topic_endpoints_to_their_works_when_ordering() {
        // Topic 10 (works 1, 2) is a prerequisite of topic 11 (work 3, plus subtopic 12
        // holding work 4). Higher priority on the dependants must not win.
        let items = [
            item(1, 1, 0.1, None),
            item(2, 1, 0.1, None),
            item(3, 9, 0.9, None),
            item(4, 9, 0.9, None),
        ];
        let edges = [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 2, EdgeKind::Covers),
            edge(11, 3, EdgeKind::Covers),
            edge(11, 12, EdgeKind::Subtopic),
            edge(12, 4, EdgeKind::Covers),
            edge(10, 11, EdgeKind::Prerequisite),
        ];
        let order = linear_ids(&items, &edges, &[]);
        assert_eq!(order.len(), 4);
        let position = |n: u128| order.iter().position(|id| *id == node(n)).expect("present");
        for early in [1, 2] {
            for late in [3, 4] {
                assert!(position(early) < position(late), "{order:?}");
            }
        }
    }

    #[test]
    fn should_keep_a_topic_contiguous_when_unconstrained() {
        // Without topics: 1, 3, 2, 4 by priority. Topic A holds 1 and 2, topic B holds 3 and 4.
        let items = [
            item(1, 4, 0.9, None),
            item(3, 3, 0.9, None),
            item(2, 2, 0.9, None),
            item(4, 1, 0.9, None),
        ];
        assert_eq!(linear_ids(&items, &[], &[]), ids(&[1, 3, 2, 4]));
        let edges = [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 2, EdgeKind::Covers),
            edge(11, 3, EdgeKind::Covers),
            edge(11, 4, EdgeKind::Covers),
        ];
        assert_eq!(linear_ids(&items, &edges, &[]), ids(&[1, 2, 3, 4]));
    }

    #[test]
    fn should_leave_a_topic_when_its_next_work_is_constrained() {
        // Work 2 (topic A) needs work 3 (topic B) first, so A is split around B.
        let items = [
            item(1, 4, 0.9, None),
            item(2, 3, 0.9, None),
            item(3, 1, 0.9, None),
        ];
        let edges = [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 2, EdgeKind::Covers),
            edge(11, 3, EdgeKind::Covers),
            edge(3, 2, EdgeKind::Prerequisite),
        ];
        assert_eq!(linear_ids(&items, &edges, &[]), ids(&[1, 3, 2]));
    }

    #[test]
    fn should_break_cycles_deterministically_when_constraints_loop() {
        let items = [
            item(1, 1, 0.2, Some(2000)),
            item(2, 1, 0.9, Some(2000)),
            item(3, 1, 0.5, Some(2000)),
        ];
        // A prerequisite cycle (not storable, but the function must survive it) and a
        // citation cycle.
        let edges = [
            edge(1, 2, EdgeKind::Prerequisite),
            edge(2, 1, EdgeKind::Prerequisite),
        ];
        // 3 is free of the prerequisite cycle, so it goes first; then 2 is the better of the cycle.
        let order = linear_ids(&items, &edges, &[(3, 1), (1, 3)]);
        assert_eq!(order, ids(&[3, 2, 1]));
        let mut reversed = items.to_vec();
        reversed.reverse();
        assert_eq!(linear_ids(&reversed, &edges, &[(3, 1), (1, 3)]), order);
    }

    #[test]
    fn should_place_the_seed_last_and_ignore_its_constraints() {
        let mut seed = item(9, 0, 1.0, Some(2020));
        seed.is_seed = true;
        let items = [item(1, 1, 0.1, None), seed, item(2, 2, 0.1, None)];
        let edges = [edge(9, 2, EdgeKind::Prerequisite)];
        assert_eq!(linear_ids(&items, &edges, &[(2, 9)]), ids(&[2, 1, 9]));
    }

    #[test]
    fn should_handle_claims_duplicates_unknown_ids_and_empty_input() {
        let mut claim = item(7, 1, 0.0, None);
        claim.work_id = None;
        let items = [
            item(1, 1, 0.5, None),
            item(1, 9, 0.9, None),
            claim,
            item(2, 1, 0.5, None),
        ];
        let edges = [
            edge(99, 1, EdgeKind::Prerequisite),
            edge(2, 98, EdgeKind::Prerequisite),
            edge(50, 51, EdgeKind::Subtopic),
        ];
        let order = linear_ids(&items, &edges, &[(1, 77)]);
        assert_eq!(order.len(), 3);
        assert_eq!(order, ids(&[1, 2, 7]));
        assert!(linear_ids(&[], &edges, &[(1, 2)]).is_empty());
    }

    fn permutations(items: &mut Vec<LinearItem>, start: usize, out: &mut Vec<Vec<LinearItem>>) {
        if start == items.len() {
            out.push(items.clone());
            return;
        }
        for swap in start..items.len() {
            items.swap(start, swap);
            permutations(items, start + 1, out);
            items.swap(start, swap);
        }
    }

    #[test]
    fn should_be_deterministic_when_input_order_is_shuffled() {
        let mut base = vec![
            item(1, 2, 0.4, Some(2001)),
            item(2, 2, 0.4, Some(2001)),
            item(3, 1, 0.8, Some(1999)),
            item(4, 3, 0.1, None),
            item(5, 1, 0.8, Some(1999)),
            LinearItem {
                is_seed: true,
                ..item(6, 0, 1.0, Some(2020))
            },
        ];
        let edges = [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 3, EdgeKind::Covers),
            edge(11, 2, EdgeKind::Covers),
            edge(11, 5, EdgeKind::Covers),
            edge(10, 11, EdgeKind::Subtopic),
            edge(4, 2, EdgeKind::Prerequisite),
            edge(5, 3, EdgeKind::Prerequisite),
            edge(3, 5, EdgeKind::Prerequisite),
        ];
        let cites = [(1, 3), (3, 4), (4, 1), (2, 5)];
        let expected = linear_ids(&base, &edges, &cites);
        assert_eq!(expected.len(), 6);
        assert_eq!(expected.last(), Some(&node(6)));
        let mut all = Vec::new();
        permutations(&mut base, 0, &mut all);
        assert_eq!(all.len(), 720);
        let mut reversed_edges = edges.to_vec();
        reversed_edges.reverse();
        for permutation in all {
            assert_eq!(linear_ids(&permutation, &edges, &cites), expected);
            assert_eq!(linear_ids(&permutation, &reversed_edges, &cites), expected);
        }
    }

    fn status(spec: &[(u128, ReadingStatus)]) -> Vec<(NodeId, ReadingStatus)> {
        spec.iter().map(|&(n, s)| (node(n), s)).collect()
    }

    #[test]
    fn should_list_unread_works_with_all_prerequisites_read_when_computing_frontier() {
        use ReadingStatus::{Read, Reading, Unread};
        let works = status(&[
            (1, Read),
            (2, Unread),
            (3, Unread),
            (4, Reading),
            (5, Unread),
        ]);
        let edges = [
            edge(1, 2, EdgeKind::Prerequisite),
            edge(4, 3, EdgeKind::Prerequisite),
            edge(2, 5, EdgeKind::Prerequisite),
        ];
        let order = ids(&[5, 3, 2, 1, 4]);
        // 3 waits on the work still being read, 5 waits on 2; 4 is not unread.
        assert_eq!(frontier(&works, &edges, &order), ids(&[2]));
        assert_eq!(unmet_prerequisites(&works, &edges, node(3)), ids(&[4]));
    }

    #[test]
    fn should_block_works_through_topics_when_a_topic_prerequisite_is_unread() {
        use ReadingStatus::{Read, Unread};
        // Topic 10 (works 1, 2) is a prerequisite of topic 11, which holds subtopic 12
        // (work 4) and work 3. Work 2 is unread, so 3 and 4 stay blocked.
        let works = status(&[(1, Read), (2, Unread), (3, Unread), (4, Unread)]);
        let edges = [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 2, EdgeKind::Covers),
            edge(11, 3, EdgeKind::Covers),
            edge(11, 12, EdgeKind::Subtopic),
            edge(12, 4, EdgeKind::Covers),
            edge(10, 11, EdgeKind::Prerequisite),
        ];
        let order = ids(&[1, 2, 3, 4]);
        assert_eq!(frontier(&works, &edges, &order), ids(&[2]));
        assert_eq!(unmet_prerequisites(&works, &edges, node(4)), ids(&[2]));
        let after = status(&[(1, Read), (2, Read), (3, Unread), (4, Unread)]);
        assert_eq!(frontier(&after, &edges, &order), ids(&[3, 4]));
    }

    #[test]
    fn should_rank_frontier_by_order_and_put_unlisted_works_last() {
        use ReadingStatus::Unread;
        let works = status(&[(1, Unread), (2, Unread), (3, Unread), (3, Unread)]);
        assert_eq!(frontier(&works, &[], &ids(&[2, 1])), ids(&[2, 1, 3]));
        assert!(frontier(&[], &[], &[]).is_empty());
    }

    #[test]
    fn should_ignore_unknown_nodes_and_empty_topics_when_computing_frontier() {
        use ReadingStatus::Unread;
        let works = status(&[(1, Unread)]);
        let edges = [
            edge(99, 1, EdgeKind::Prerequisite),
            edge(10, 1, EdgeKind::Prerequisite),
            edge(1, 1, EdgeKind::Prerequisite),
        ];
        assert_eq!(frontier(&works, &edges, &[]), ids(&[1]));
    }

    #[test]
    fn should_reject_edge_when_it_closes_a_cycle_through_a_topic() {
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (10, NodeKind::Topic),
        ]);
        // Topic 10 covers work 2; work 1 precedes the topic (so work 2); work 2 then cannot
        // precede work 1.
        let existing = [
            edge(10, 2, EdgeKind::Covers),
            edge(1, 10, EdgeKind::Prerequisite),
        ];
        let rejection = validate_edge(&nodes, &existing, edge(2, 1, EdgeKind::Prerequisite))
            .expect_err("cycle");
        assert_eq!(rejection.cycle(), Some(ids(&[1, 2]).as_slice()));
        assert_eq!(rejection.list_error().code(), "edge_cycle");
        // Order of arrival does not matter: the covers edge can be the one that closes it.
        let existing = [
            edge(1, 10, EdgeKind::Prerequisite),
            edge(2, 1, EdgeKind::Prerequisite),
        ];
        let rejection =
            validate_edge(&nodes, &existing, edge(10, 2, EdgeKind::Covers)).expect_err("cycle");
        assert_eq!(rejection.cycle(), Some(ids(&[1, 2]).as_slice()));
        // A subtopic edge changes membership too.
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (10, NodeKind::Topic),
            (11, NodeKind::Topic),
        ]);
        let existing = [
            edge(11, 2, EdgeKind::Covers),
            edge(1, 10, EdgeKind::Prerequisite),
            edge(2, 1, EdgeKind::Prerequisite),
        ];
        assert!(validate_edge(&nodes, &existing, edge(10, 11, EdgeKind::Subtopic)).is_err());
    }

    #[test]
    fn should_accept_edge_when_a_work_precedes_its_own_topic() {
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (10, NodeKind::Topic),
        ]);
        let existing = [
            edge(10, 1, EdgeKind::Covers),
            edge(10, 2, EdgeKind::Covers),
            edge(1, 10, EdgeKind::Prerequisite),
        ];
        // 1 before 2 is implied; repeating the stored edge stays fine.
        assert_eq!(
            validate_edge(&nodes, &existing, edge(1, 10, EdgeKind::Prerequisite)),
            Ok(())
        );
        assert!(validate_edge(&nodes, &existing, edge(2, 1, EdgeKind::Prerequisite)).is_err());
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // the remainder is below the list length
    fn should_never_block_every_unread_work_when_edges_pass_validation() {
        use ReadingStatus::Unread;
        let nodes = kinds(&[
            (1, NodeKind::Work),
            (2, NodeKind::Work),
            (3, NodeKind::Work),
            (10, NodeKind::Topic),
            (11, NodeKind::Topic),
        ]);
        let mut candidates = Vec::new();
        for topic in [10, 11] {
            for work in [1, 2, 3] {
                candidates.push(edge(topic, work, EdgeKind::Covers));
            }
        }
        candidates.push(edge(10, 11, EdgeKind::Subtopic));
        for from in [1, 2, 3, 10, 11] {
            for to in [1, 2, 3, 10, 11] {
                if from != to {
                    candidates.push(edge(from, to, EdgeKind::Prerequisite));
                }
            }
        }
        let works = status(&[(1, Unread), (2, Unread), (3, Unread)]);
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..400 {
            // Fisher-Yates with a fixed xorshift stream, so the trials are reproducible.
            let mut shuffled = candidates.clone();
            for i in (1..shuffled.len()).rev() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                shuffled.swap(i, (state % (i as u64 + 1)) as usize);
            }
            let mut accepted: Vec<ListEdge> = Vec::new();
            for candidate in shuffled {
                if validate_edge(&nodes, &accepted, candidate).is_ok() {
                    accepted.push(candidate);
                }
            }
            assert!(
                !frontier(&works, &accepted, &[]).is_empty(),
                "all blocked by {accepted:?}"
            );
        }
    }
}
