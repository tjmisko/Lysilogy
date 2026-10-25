//! Plain-text views of a list for the CLI: sequence, topics, next up, acquire, review.
//!
//! Every view is a projection of the one [`ListDetail`]; nothing here writes.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use lysilogy_core::{
    EdgeKind, ListEdge, ListReview, NodeId, NodeKind, ReadingStatus, WorkId, frontier,
    unmet_prerequisites,
};

use super::{ListDetail, ListSummaryView, NodeView};

/// Deepest topic nesting the topics view prints, which also bounds malformed cycles.
const TOPIC_DEPTH_MAX: usize = 8;

/// Which projection `list show` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ListView {
    Sequence,
    Topics,
    Next,
    Acquire,
    Review,
}

/// The status glyph used by the sidebar.
#[must_use]
pub const fn glyph(status: ReadingStatus) -> &'static str {
    match status {
        ReadingStatus::Unread => "○",
        ReadingStatus::Reading => "◐",
        ReadingStatus::Read => "●",
    }
}

/// One line per list, most recently updated first (as given).
#[must_use]
pub fn render_list_summaries(lists: &[ListSummaryView]) -> String {
    if lists.is_empty() {
        return "No lists.\n".to_owned();
    }
    let mut text = String::new();
    for list in lists {
        let archived = if list.list.archived_at.is_some() {
            " [archived]"
        } else {
            ""
        };
        let _ = writeln!(
            text,
            "{}  {}{archived} ({} items: {} unread, {} reading, {} read)",
            list.list.id,
            list.list.title,
            list.item_count,
            list.status_counts.unread,
            list.status_counts.reading,
            list.status_counts.read
        );
    }
    text
}

fn title_of(node: &NodeView) -> String {
    match node.node.kind {
        NodeKind::Work => node
            .work
            .as_ref()
            .and_then(|work| work.display_title.clone())
            .or_else(|| node.node.work_id.map(|work| work.to_string()))
            .unwrap_or_default(),
        NodeKind::Topic => node.node.label.clone().unwrap_or_default(),
        NodeKind::Claim => node
            .node
            .claim
            .as_ref()
            .and_then(|claim| claim.get("title"))
            .and_then(|title| title.as_str())
            .unwrap_or_default()
            .to_owned(),
    }
}

fn year_of(node: &NodeView) -> String {
    let year = match node.node.kind {
        NodeKind::Work => node.work.as_ref().and_then(|work| work.display_year),
        _ => node
            .node
            .claim
            .as_ref()
            .and_then(|claim| claim.get("year"))
            .and_then(serde_json::Value::as_i64)
            .and_then(|year| i32::try_from(year).ok()),
    };
    year.map_or_else(|| "n.d.".to_owned(), |year| year.to_string())
}

fn local_state(detail: &ListDetail, work: WorkId) -> String {
    let key = work.to_string();
    if detail.local_papers.contains_key(&key) {
        return "held".to_owned();
    }
    detail.acquisitions.get(&key).map_or_else(
        || "not held".to_owned(),
        |brief| format!("fetch:{}", brief.state),
    )
}

fn work_line(detail: &ListDetail, node: &NodeView) -> String {
    let status = node
        .node
        .work_id
        .map_or(ReadingStatus::Unread, |work| detail.status_of(work));
    format!("{} {} ({})", glyph(status), title_of(node), year_of(node))
}

fn header(detail: &ListDetail) -> String {
    let list = &detail.list;
    let mut text = format!(
        "{} ({}) · {} items · {} topics · {} unread, {} reading, {} read\n",
        list.list.title,
        list.list.id,
        list.item_count,
        list.topic_count,
        list.status_counts.unread,
        list.status_counts.reading,
        list.status_counts.read
    );
    if list.proposed_count > 0 || list.claim_count > 0 {
        let _ = writeln!(
            text,
            "{} proposed, {} claims awaiting verification or resolution",
            list.proposed_count, list.claim_count
        );
    }
    text
}

/// The list in the requested view.
#[must_use]
pub fn render_list_detail(detail: &ListDetail, view: ListView) -> String {
    let mut text = header(detail);
    text.push_str(&match view {
        ListView::Sequence => sequence(detail),
        ListView::Topics => topics(detail),
        ListView::Next => next_up(detail),
        ListView::Acquire => acquire(detail),
        ListView::Review => review(detail),
    });
    text
}

fn topics_of<'a>(detail: &'a ListDetail, node: &NodeView) -> Vec<&'a str> {
    detail
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Covers && edge.to == node.node.id)
        .filter_map(|edge| {
            detail
                .nodes
                .iter()
                .find(|candidate| candidate.node.id == edge.from)
                .and_then(|topic| topic.node.label.as_deref())
        })
        .collect()
}

fn sequence(detail: &ListDetail) -> String {
    let mut text = String::new();
    for node in &detail.nodes {
        let mut line = match node.node.kind {
            NodeKind::Work => {
                let mut line = format!("{}. {}", node.ordinal, work_line(detail, node));
                if let Some(work) = node.node.work_id {
                    let _ = write!(line, " · {}", local_state(detail, work));
                }
                let topics = topics_of(detail, node);
                if !topics.is_empty() {
                    let _ = write!(line, " · {}", topics.join(", "));
                }
                line
            }
            NodeKind::Topic => format!("{}. # {}", node.ordinal, title_of(node)),
            NodeKind::Claim => format!(
                "{}. ? {} ({}) [{}]",
                node.ordinal,
                title_of(node),
                year_of(node),
                node.node
                    .claim_state
                    .map_or_else(String::new, |state| state.to_string())
            ),
        };
        if node.node.review == ListReview::Proposed {
            line.push_str(" · proposed");
        }
        let _ = writeln!(text, "{line}");
        if let Some(note) = &node.node.note {
            let _ = writeln!(text, "    note: {note}");
        }
    }
    text
}

fn topics(detail: &ListDetail) -> String {
    let by_id: HashMap<NodeId, &NodeView> = detail
        .nodes
        .iter()
        .map(|node| (node.node.id, node))
        .collect();
    let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut filed: HashSet<NodeId> = HashSet::new();
    let mut subtopics: HashSet<NodeId> = HashSet::new();
    for edge in &detail.edges {
        match edge.kind {
            EdgeKind::Covers => {
                filed.insert(edge.to);
                children.entry(edge.from).or_default().push(edge.to);
            }
            EdgeKind::Subtopic => {
                subtopics.insert(edge.to);
                children.entry(edge.from).or_default().push(edge.to);
            }
            EdgeKind::Prerequisite => {}
        }
    }
    let mut text = String::new();
    for node in detail
        .nodes
        .iter()
        .filter(|node| node.node.kind == NodeKind::Topic && !subtopics.contains(&node.node.id))
    {
        topic_block(detail, &by_id, &children, node, 0, &mut text);
    }
    let unfiled: Vec<&NodeView> = detail
        .nodes
        .iter()
        .filter(|node| node.node.kind != NodeKind::Topic && !filed.contains(&node.node.id))
        .collect();
    if !unfiled.is_empty() {
        text.push_str("Unfiled\n");
        for node in unfiled {
            let _ = writeln!(text, "  {}", member_line(detail, node));
        }
    }
    if text.is_empty() {
        text.push_str("No topics.\n");
    }
    text
}

fn member_line(detail: &ListDetail, node: &NodeView) -> String {
    match node.node.kind {
        NodeKind::Work => work_line(detail, node),
        _ => format!("? {} ({})", title_of(node), year_of(node)),
    }
}

fn topic_block(
    detail: &ListDetail,
    by_id: &HashMap<NodeId, &NodeView>,
    children: &HashMap<NodeId, Vec<NodeId>>,
    topic: &NodeView,
    depth: usize,
    text: &mut String,
) {
    let indent = "  ".repeat(depth);
    let _ = writeln!(text, "{indent}# {}", title_of(topic));
    if depth >= TOPIC_DEPTH_MAX {
        return;
    }
    let mut members: Vec<&NodeView> = children
        .get(&topic.node.id)
        .into_iter()
        .flatten()
        .filter_map(|id| by_id.get(id).copied())
        .collect();
    members.sort_by_key(|member| member.node.position);
    for member in members {
        if member.node.kind == NodeKind::Topic {
            topic_block(detail, by_id, children, member, depth + 1, text);
            continue;
        }
        let _ = writeln!(text, "{indent}  {}", member_line(detail, member));
    }
}

/// Unread Works whose prerequisites are all read, in the list's stored order.
fn next_up(detail: &ListDetail) -> String {
    let works: Vec<(NodeId, ReadingStatus)> = detail
        .nodes
        .iter()
        .filter(|node| node.node.kind == NodeKind::Work)
        .filter_map(|node| Some((node.node.id, detail.status_of(node.node.work_id?))))
        .collect();
    let edges: Vec<ListEdge> = detail
        .edges
        .iter()
        .map(|edge| ListEdge {
            from: edge.from,
            to: edge.to,
            kind: edge.kind,
        })
        .collect();
    let order: Vec<NodeId> = detail.nodes.iter().map(|node| node.node.id).collect();
    let ready = frontier(&works, &edges, &order);
    let by_id: HashMap<NodeId, &NodeView> = detail
        .nodes
        .iter()
        .map(|node| (node.node.id, node))
        .collect();
    if !ready.is_empty() {
        let mut text = String::new();
        for (index, id) in ready.iter().enumerate() {
            if let Some(node) = by_id.get(id) {
                let _ = writeln!(text, "{}. {}", index + 1, work_line(detail, node));
            }
        }
        return text;
    }
    let Some(first) = detail.nodes.iter().find(|node| {
        node.node
            .work_id
            .is_some_and(|work| detail.status_of(work) == ReadingStatus::Unread)
    }) else {
        return "Nothing is waiting: every Work is read or being read.\n".to_owned();
    };
    let blockers = unmet_prerequisites(&works, &edges, first.node.id);
    let names: Vec<String> = blockers
        .iter()
        .filter_map(|id| by_id.get(id))
        .map(|node| title_of(node))
        .collect();
    format!(
        "Nothing is ready. {} waits for: {}\n",
        title_of(first),
        names.join("; ")
    )
}

fn acquire(detail: &ListDetail) -> String {
    let mut text = String::new();
    for node in detail
        .nodes
        .iter()
        .filter(|node| node.node.kind == NodeKind::Work)
    {
        let Some(work) = node.node.work_id else {
            continue;
        };
        if detail.local_papers.contains_key(&work.to_string()) {
            continue;
        }
        let _ = writeln!(
            text,
            "{} · {}",
            work_line(detail, node),
            local_state(detail, work)
        );
    }
    if text.is_empty() {
        text.push_str("Every Work is held.\n");
    }
    text
}

fn review(detail: &ListDetail) -> String {
    let mut text = String::new();
    for node in detail
        .nodes
        .iter()
        .filter(|node| node.node.review == ListReview::Proposed)
    {
        let _ = writeln!(text, "proposed {} {}", node.node.kind, title_of(node));
    }
    let titles: HashMap<NodeId, String> = detail
        .nodes
        .iter()
        .map(|node| (node.node.id, title_of(node)))
        .collect();
    for edge in detail
        .edges
        .iter()
        .filter(|edge| edge.review == ListReview::Proposed)
    {
        let name = |id: &NodeId| titles.get(id).cloned().unwrap_or_else(|| id.to_string());
        let _ = writeln!(
            text,
            "proposed edge {} -[{}]-> {}",
            name(&edge.from),
            edge.kind,
            name(&edge.to)
        );
    }
    for node in detail
        .nodes
        .iter()
        .filter(|node| node.node.kind == NodeKind::Claim)
    {
        let candidates = node
            .node
            .candidates
            .as_array()
            .map_or(0, std::vec::Vec::len);
        let _ = writeln!(
            text,
            "claim [{}] {} ({}) · {candidates} candidates",
            node.node
                .claim_state
                .map_or_else(String::new, |state| state.to_string()),
            title_of(node),
            year_of(node)
        );
    }
    if text.is_empty() {
        text.push_str("Nothing to review.\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use lysilogy_core::{
        ClaimState, EdgeSource, FieldState, ListId, ListOrigin, NodeSource, Visibility, Work,
        WorkKind, WorkspaceId,
    };
    use lysilogy_db::list::{List, ListNode, StoredEdge};
    use serde_json::json;

    use super::*;
    use crate::lists::{ReadingStateView, StatusCounts};

    fn work(title: &str, year: i32) -> Work {
        let now = Utc::now();
        Work {
            id: WorkId::new(),
            kind: WorkKind::Article,
            visibility: Visibility::Private,
            display_title: Some(title.to_owned()),
            title_state: FieldState::Known,
            display_year: Some(year),
            year_state: FieldState::Known,
            display_authors: None,
            authors_state: FieldState::Unknown,
            created_at: now,
            updated_at: now,
        }
    }

    fn node(kind: NodeKind, ordinal: usize) -> NodeView {
        let now = Utc::now();
        NodeView {
            node: ListNode {
                id: NodeId::new(),
                list_id: ListId::new(),
                kind,
                work_id: None,
                label: None,
                summary: None,
                claim: None,
                claim_state: None,
                candidates: json!([]),
                position: i32::try_from(ordinal).unwrap_or(0) - 1,
                note: None,
                rationale: None,
                context_links: json!([]),
                source: NodeSource::User,
                review: ListReview::Accepted,
                via_work_id: None,
                path: None,
                generation_id: None,
                added_at: now,
                updated_at: now,
            },
            work: None,
            ordinal,
            depth: 0,
            path_steps: Vec::new(),
        }
    }

    fn work_node(title: &str, year: i32, ordinal: usize) -> NodeView {
        let work = work(title, year);
        let mut view = node(NodeKind::Work, ordinal);
        view.node.work_id = Some(work.id);
        view.work = Some(work);
        view
    }

    fn topic_node(label: &str, ordinal: usize) -> NodeView {
        let mut view = node(NodeKind::Topic, ordinal);
        view.node.label = Some(label.to_owned());
        view
    }

    fn edge(from: &NodeView, to: &NodeView, kind: EdgeKind) -> StoredEdge {
        StoredEdge {
            list_id: from.node.list_id,
            from: from.node.id,
            to: to.node.id,
            kind,
            source: EdgeSource::User,
            review: ListReview::Accepted,
            created_at: Utc::now(),
        }
    }

    fn detail(nodes: Vec<NodeView>, edges: Vec<StoredEdge>) -> ListDetail {
        let now = Utc::now();
        let items = i64::try_from(
            nodes
                .iter()
                .filter(|n| n.node.kind != NodeKind::Topic)
                .count(),
        )
        .unwrap_or(0);
        ListDetail {
            list: ListSummaryView {
                list: List {
                    id: ListId::new(),
                    workspace_id: WorkspaceId::new(),
                    title: "RLHF basics".to_owned(),
                    description: None,
                    origin: ListOrigin::Manual,
                    seed_work_id: None,
                    goal: None,
                    archived_at: None,
                    created_at: now,
                    updated_at: now,
                },
                seed_title: None,
                item_count: items,
                topic_count: 0,
                claim_count: 0,
                proposed_count: 0,
                status_counts: StatusCounts {
                    unread: items,
                    reading: 0,
                    read: 0,
                },
            },
            nodes,
            edges,
            citations: Vec::new(),
            reading_states: BTreeMap::new(),
            local_papers: BTreeMap::new(),
            acquisitions: BTreeMap::new(),
            expansions: BTreeMap::new(),
        }
    }

    fn mark(detail: &mut ListDetail, node: &NodeView, status: ReadingStatus) {
        let work = node.node.work_id.expect("work node");
        detail.reading_states.insert(
            work.to_string(),
            ReadingStateView {
                status,
                changed_at: None,
            },
        );
    }

    #[test]
    fn should_print_no_lists_message_when_there_are_none() {
        assert_eq!(render_list_summaries(&[]), "No lists.\n");
    }

    #[test]
    fn should_print_sequence_with_glyphs_topics_and_notes_when_rendering_sequence() {
        let topic = topic_node("Alignment", 1);
        let mut first = work_node("Reward models", 2017, 2);
        first.node.note = Some("start".to_owned());
        let second = work_node("Policy gradients", 2015, 3);
        let mut claim = node(NodeKind::Claim, 4);
        claim.node.claim = Some(json!({"title": "A remembered paper", "year": 2020}));
        claim.node.claim_state = Some(ClaimState::Ambiguous);
        let edges = vec![edge(&topic, &first, EdgeKind::Covers)];
        let mut list = detail(vec![topic, first.clone(), second, claim], edges);
        mark(&mut list, &first, ReadingStatus::Read);
        let text = render_list_detail(&list, ListView::Sequence);
        assert!(text.starts_with("RLHF basics ("), "{text}");
        assert!(
            text.contains("2. ● Reward models (2017) · not held · Alignment"),
            "{text}"
        );
        assert!(text.contains("    note: start"), "{text}");
        assert!(text.contains("1. # Alignment"), "{text}");
        assert!(text.contains("3. ○ Policy gradients (2015)"), "{text}");
        assert!(
            text.contains("4. ? A remembered paper (2020) [ambiguous]"),
            "{text}"
        );
    }

    #[test]
    fn should_group_works_under_topics_and_list_unfiled_when_rendering_topics() {
        let parent = topic_node("Parent", 1);
        let child = topic_node("Child", 2);
        let filed = work_node("Filed work", 2019, 3);
        let loose = work_node("Loose work", 2018, 4);
        let edges = vec![
            edge(&parent, &child, EdgeKind::Subtopic),
            edge(&child, &filed, EdgeKind::Covers),
        ];
        let text = render_list_detail(
            &detail(vec![parent, child, filed, loose], edges),
            ListView::Topics,
        );
        let body = text.split_once('\n').expect("header").1;
        assert_eq!(
            body,
            "# Parent\n  # Child\n    ○ Filed work (2019)\nUnfiled\n  ○ Loose work (2018)\n"
        );
    }

    #[test]
    fn should_stop_nesting_when_topics_form_a_cycle_in_the_data() {
        let first = topic_node("First", 1);
        let second = topic_node("Second", 2);
        let edges = vec![
            edge(&first, &second, EdgeKind::Subtopic),
            edge(&second, &first, EdgeKind::Subtopic),
        ];
        // Both are subtopics of the other, so neither is a root: nothing loops.
        let text = render_list_detail(&detail(vec![first, second], edges), ListView::Topics);
        assert!(text.ends_with("No topics.\n"), "{text}");
    }

    #[test]
    fn should_offer_only_ready_works_when_rendering_next_up() {
        let basics = work_node("Basics", 2010, 1);
        let advanced = work_node("Advanced", 2020, 2);
        let done = work_node("Done", 2005, 3);
        let edges = vec![edge(&basics, &advanced, EdgeKind::Prerequisite)];
        let mut list = detail(vec![basics.clone(), advanced, done.clone()], edges);
        mark(&mut list, &done, ReadingStatus::Read);
        let text = render_list_detail(&list, ListView::Next);
        let body = text.split_once('\n').expect("header").1;
        assert_eq!(body, "1. ○ Basics (2010)\n");
        // Once the prerequisite is read the dependant is next.
        mark(&mut list, &basics, ReadingStatus::Read);
        let text = render_list_detail(&list, ListView::Next);
        assert!(text.ends_with("1. ○ Advanced (2020)\n"), "{text}");
    }

    #[test]
    fn should_explain_what_blocks_when_nothing_is_ready() {
        let basics = work_node("Basics", 2010, 1);
        let advanced = work_node("Advanced", 2020, 2);
        let edges = vec![edge(&basics, &advanced, EdgeKind::Prerequisite)];
        let mut list = detail(vec![basics.clone(), advanced.clone()], edges);
        mark(&mut list, &basics, ReadingStatus::Reading);
        let text = render_list_detail(&list, ListView::Next);
        assert!(
            text.ends_with("Nothing is ready. Advanced waits for: Basics\n"),
            "{text}"
        );
        mark(&mut list, &advanced, ReadingStatus::Read);
        let text = render_list_detail(&list, ListView::Next);
        assert!(
            text.ends_with("Nothing is waiting: every Work is read or being read.\n"),
            "{text}"
        );
    }

    #[test]
    fn should_list_unheld_works_when_rendering_acquire() {
        let held = work_node("Held", 2000, 1);
        let missing = work_node("Missing", 2001, 2);
        let mut list = detail(vec![held.clone(), missing], Vec::new());
        list.local_papers.insert(
            held.node.work_id.expect("work").to_string(),
            vec![crate::api::LocalPaper {
                paper_id: crate::domain::PaperId::from_relative_path(std::path::Path::new("a.pdf")),
                title: "Held".to_owned(),
            }],
        );
        let text = render_list_detail(&list, ListView::Acquire);
        let body = text.split_once('\n').expect("header").1;
        assert_eq!(body, "○ Missing (2001) · not held\n");
        let all_held = detail(Vec::new(), Vec::new());
        assert!(
            render_list_detail(&all_held, ListView::Acquire).ends_with("Every Work is held.\n")
        );
    }

    #[test]
    fn should_show_proposed_nodes_edges_and_claims_when_rendering_review() {
        let first = work_node("First", 2000, 1);
        let mut proposed = work_node("Proposed", 2001, 2);
        proposed.node.review = ListReview::Proposed;
        let mut claim = node(NodeKind::Claim, 3);
        claim.node.claim = Some(json!({"title": "Unsure"}));
        claim.node.claim_state = Some(ClaimState::Unverified);
        claim.node.candidates = json!([{}, {}]);
        let mut proposed_edge = edge(&first, &proposed, EdgeKind::Prerequisite);
        proposed_edge.review = ListReview::Proposed;
        let text = render_list_detail(
            &detail(vec![first, proposed, claim], vec![proposed_edge]),
            ListView::Review,
        );
        let body = text.split_once('\n').expect("header").1;
        assert_eq!(
            body,
            "proposed work Proposed\nproposed edge First -[prerequisite]-> Proposed\n\
             claim [unverified] Unsure (n.d.) · 2 candidates\n"
        );
        let empty = detail(Vec::new(), Vec::new());
        assert!(render_list_detail(&empty, ListView::Review).ends_with("Nothing to review.\n"));
    }
}
