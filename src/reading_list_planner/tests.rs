//! End-to-end tests of the generation pipeline with the recording fake planner: no model
//! call, no paid API, and no network. Provider answers come from a scripted transport.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use http_body_util::BodyExt;
use lysilogy_core::{
    EdgeKind, GenerationState, IdentifierScheme, ListReview, NodeKind, NodeSource, ReadingStatus,
    WorkId,
};
use lysilogy_db::history::NewEvent;
use lysilogy_ingest::openalex::ProviderError;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fake::FakePlanner;
use super::pipeline::GenerationView;
use super::report::Report;
use super::{
    AssumedKnownDraft, ClaimDraft, ContextLinkDraft, FillDraft, FillPickDraft, GenerateRequest,
    GenerationError, GraphWorkDraft, Level, PlanDraft, PlannerError, PrerequisiteDraft, TopicDraft,
    TopicEdgeDraft,
};
use crate::{
    AppState,
    api::build_router,
    expansion::fixtures::{
        ScriptedTransport, batch_reply, marker, new_work, plain_state, test_graph,
    },
    graph::GraphState,
};

macro_rules! graph_or_skip {
    () => {
        match test_graph().await {
            Some(graph) => graph,
            None => {
                eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
                return;
            }
        }
    };
}

// -- fixtures ----------------------------------------------------------------------------

/// The token that makes a claim's title searchable in the scripted provider.
fn needle(marker: &str, index: usize) -> String {
    format!("{marker}k{index}z")
}

fn paper_title(marker: &str, index: usize) -> String {
    format!("Verified paper {}", needle(marker, index))
}

fn record(marker: &str, index: usize) -> (String, Value) {
    let title = paper_title(marker, index);
    (
        needle(marker, index),
        json!({
            "id": format!("https://openalex.org/W{index}{marker}"),
            "display_name": title,
            "publication_year": 2021,
            "authorships": [{"author": {"display_name": "Ada Lovelace"}}],
            "referenced_works_count": 3
        }),
    )
}

/// Answers title searches for records whose title carries their needle, batch lookups with
/// generic titles, and Crossref with an empty record; everything else is a 404.
fn provider(found: Vec<(String, Value)>) -> ScriptedTransport {
    ScriptedTransport::new(move |url| {
        if url.contains("title.search:") {
            for (needle, record) in &found {
                if url.contains(needle.as_str()) {
                    return Ok(json!({"results": [record]}));
                }
            }
            return Ok(json!({"results": []}));
        }
        if url.contains("filter=openalex:") {
            return Ok(batch_reply(url));
        }
        if url.contains("api.crossref.org/works/") {
            return Ok(json!({"message": {}}));
        }
        Err(ProviderError::Status(404))
    })
}

fn verified_claim(marker: &str, index: usize, role: &str) -> ClaimDraft {
    ClaimDraft {
        title: paper_title(marker, index),
        authors: vec!["Ada Lovelace".to_owned()],
        year: Some(2021),
        role: role.to_owned(),
        rationale: "It matters.".to_owned(),
        ..ClaimDraft::default()
    }
}

fn unknown_claim(marker: &str, role: &str) -> ClaimDraft {
    ClaimDraft {
        title: format!("Nothing like this exists anywhere {marker}"),
        authors: vec!["Nobody Atall".to_owned()],
        year: Some(1999),
        role: role.to_owned(),
        rationale: "Remembered, maybe.".to_owned(),
        ..ClaimDraft::default()
    }
}

fn graph_pick(work: WorkId, role: &str) -> GraphWorkDraft {
    GraphWorkDraft {
        work_id: work.to_string(),
        role: role.to_owned(),
        rationale: "Already in the library.".to_owned(),
        context_links: Vec::new(),
    }
}

fn topic(label: &str, claims: Vec<ClaimDraft>, graph_works: Vec<GraphWorkDraft>) -> TopicDraft {
    TopicDraft {
        label: label.to_owned(),
        summary: format!("About {label}."),
        claims,
        graph_works,
    }
}

fn prerequisite(from: &str, to: &str) -> TopicEdgeDraft {
    TopicEdgeDraft {
        from: from.to_owned(),
        to: to.to_owned(),
        kind: "prerequisite".to_owned(),
    }
}

async fn titled_work(graph: &GraphState, tag: &str, title: &str) -> WorkId {
    let work = new_work(graph, IdentifierScheme::Doi, &format!("10.9999/{tag}")).await;
    graph
        .works
        .set_display_title(work, title)
        .await
        .expect("title");
    work
}

async fn state_with(
    graph: &GraphState,
    found: Vec<(String, Value)>,
    planner: &FakePlanner,
) -> AppState {
    plain_state(graph.clone(), provider(found))
        .await
        .with_planner(Arc::new(planner.clone()))
}

/// A goal no other test shares, whose one keyword matches only titles carrying `token`.
fn goal_of(token: &str) -> String {
    token.to_owned()
}

fn request(goal: &str) -> GenerateRequest {
    GenerateRequest {
        size: super::SIZE_MIN,
        ..GenerateRequest::new(goal)
    }
}

async fn run(state: &AppState, request: GenerateRequest) -> GenerationView {
    state.generate_list(request).await.expect("generation ran")
}

fn report(view: &GenerationView) -> Report {
    let json = view.report.clone().expect("report");
    // Round-trip through the public JSON the API serves.
    let mut report = Report::default();
    let get = |path: &[&str]| -> usize {
        path.iter()
            .try_fold(&json, |value, key| value.get(*key))
            .and_then(Value::as_u64)
            .map_or(0, |count| usize::try_from(count).unwrap_or(0))
    };
    report.candidates.seen = get(&["candidates", "seen"]);
    report.candidates.invalid = get(&["candidates", "invalid"]);
    report.candidates.topic_dropped = get(&["candidates", "topic_dropped"]);
    report.candidates.over_limit = get(&["candidates", "over_limit"]);
    report.candidates.rejected_ids = get(&["candidates", "rejected_ids"]);
    report.candidates.matched = get(&["candidates", "matched"]);
    report.candidates.verified = get(&["candidates", "verified"]);
    report.candidates.in_graph = get(&["candidates", "in_graph"]);
    report.candidates.unverified = get(&["candidates", "unverified"]);
    report.candidates.ambiguous = get(&["candidates", "ambiguous"]);
    report.candidates.contradicted = get(&["candidates", "contradicted"]);
    report.candidates.pending = get(&["candidates", "pending"]);
    report.candidates.duplicate = get(&["candidates", "duplicate"]);
    report.candidates.already_in_list = get(&["candidates", "already_in_list"]);
    report.candidates.over_size = get(&["candidates", "over_size"]);
    report.candidates.list_full = get(&["candidates", "list_full"]);
    report
}

fn assert_counts_sum(view: &GenerationView) {
    let counted = report(view).candidates;
    assert_eq!(
        counted.accounted(),
        counted.seen,
        "every claim lands in one bucket: {:?}",
        view.report
    );
}

fn count_at(view: &GenerationView, path: &[&str]) -> u64 {
    let json = view.report.clone().expect("report");
    path.iter()
        .try_fold(&json, |value, key| value.get(*key))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("no count at {path:?} in {json}"))
}

// -- the pipeline ------------------------------------------------------------------------

#[tokio::test]
async fn should_build_an_ordered_generated_list_when_the_plan_verifies() {
    let graph = graph_or_skip!();
    let tag = marker();
    let primer = titled_work(
        &graph,
        &format!("{tag}-primer"),
        &format!("A primer on {tag}"),
    )
    .await;
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![
            topic(
                "Foundations",
                vec![
                    verified_claim(&tag, 1, "foundation"),
                    verified_claim(&tag, 2, "survey"),
                ],
                vec![graph_pick(primer, "survey")],
            ),
            topic(
                "Methods",
                vec![
                    verified_claim(&tag, 3, "survey"),
                    unknown_claim(&tag, "survey"),
                ],
                Vec::new(),
            ),
        ],
        topic_edges: vec![prerequisite("Foundations", "Methods")],
        assumed_known: Vec::new(),
    });
    let found = vec![record(&tag, 1), record(&tag, 2), record(&tag, 3)];
    let state = state_with(&graph, found, &planner).await;
    let mut asked = request(&goal_of(&tag));
    asked.level = Level::Deep;
    let view = run(&state, asked).await;

    assert_eq!(view.state, GenerationState::Succeeded, "{view:?}");
    assert_counts_sum(&view);
    assert_eq!(count_at(&view, &["candidates", "seen"]), 5);
    assert_eq!(count_at(&view, &["candidates", "verified"]), 3);
    assert_eq!(count_at(&view, &["candidates", "in_graph"]), 1);
    assert_eq!(count_at(&view, &["candidates", "unverified"]), 1);
    assert_eq!(count_at(&view, &["admitted", "works"]), 4);
    assert_eq!(count_at(&view, &["admitted", "claims"]), 1);
    assert_eq!(count_at(&view, &["admitted", "topics"]), 2);
    assert_eq!(view.model_key.as_deref(), Some("fake:planner"));

    let requests = planner.plan_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].level, Level::Deep);
    assert_eq!(requests[0].size, super::SIZE_MIN);
    assert!(
        requests[0]
            .graph_context
            .iter()
            .any(|work| work.work_id == primer),
        "the keyword title search found the library Work"
    );

    let list = view.list_id.expect("a list was written");
    let detail = state.list_detail(list).await.expect("detail");
    assert_eq!(
        detail.list.list.origin,
        lysilogy_core::ListOrigin::Generated
    );
    assert_eq!(
        detail.list.list.goal.as_deref(),
        Some(goal_of(&tag).as_str())
    );
    assert_eq!(detail.nodes.len(), 7);
    for node in &detail.nodes {
        assert_eq!(node.node.source, NodeSource::Agent);
        assert_eq!(
            node.node.review,
            ListReview::Accepted,
            "a new list is admitted directly"
        );
        assert_eq!(node.node.generation_id, Some(view.id));
    }
    let position_of = |label: &str| {
        detail
            .nodes
            .iter()
            .find(|node| node.node.label.as_deref() == Some(label))
            .map(|node| node.node.position)
            .expect(label)
    };
    assert!(
        position_of("Foundations") < position_of("Methods"),
        "the prerequisite topic comes first in topological order"
    );
    let covers = detail
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Covers)
        .count();
    assert_eq!(covers, 4, "each Work sits under its topic");
    assert!(
        detail
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Prerequisite)
    );
    let claim = detail
        .nodes
        .iter()
        .find(|node| node.node.kind == NodeKind::Claim)
        .expect("the unverified claim stays a claim node");
    assert_eq!(claim.node.rationale.as_deref(), Some("Remembered, maybe."));

    let fetched = state.get_generation(view.id).await.expect("stored");
    assert_eq!(fetched.state, GenerationState::Succeeded);
    assert_eq!(fetched.list_id, Some(list));
}

#[tokio::test]
async fn should_reject_and_count_a_context_id_when_it_was_not_supplied() {
    let graph = graph_or_skip!();
    let tag = marker();
    let supplied = titled_work(&graph, &format!("{tag}-in"), &format!("Supplied {tag}")).await;
    let stranger = titled_work(&graph, &format!("{tag}-out"), "Unrelated to the goal").await;
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic(
            "Basics",
            Vec::new(),
            vec![
                graph_pick(supplied, "core"),
                graph_pick(stranger, "core"),
                GraphWorkDraft {
                    work_id: "not-an-id".to_owned(),
                    role: "core".to_owned(),
                    ..GraphWorkDraft::default()
                },
            ],
        )],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;

    assert_eq!(count_at(&view, &["candidates", "rejected_ids"]), 2);
    assert_eq!(count_at(&view, &["candidates", "in_graph"]), 1);
    assert_counts_sum(&view);
    let detail = state
        .list_detail(view.list_id.expect("list"))
        .await
        .expect("detail");
    let works: HashSet<WorkId> = detail
        .nodes
        .iter()
        .filter_map(|node| node.node.work_id)
        .collect();
    assert!(works.contains(&supplied));
    assert!(
        !works.contains(&stranger),
        "an unsupplied ID never reaches the list"
    );
}

#[tokio::test]
async fn should_offer_only_neighbors_and_admit_only_offered_picks_when_filling() {
    let graph = graph_or_skip!();
    let tag = marker();
    let member = titled_work(&graph, &format!("{tag}-m"), &format!("Member {tag}")).await;
    let cited = titled_work(&graph, &format!("{tag}-c"), "Cited by the member").await;
    let citing = titled_work(&graph, &format!("{tag}-g"), "Citing the member").await;
    let other = titled_work(&graph, &format!("{tag}-o"), "Cited too").await;
    let stranger = titled_work(&graph, &format!("{tag}-s"), "Nowhere near").await;
    for (from, to) in [(member, cited), (member, other), (citing, member)] {
        graph
            .citations
            .add_assertion(from, to, None)
            .await
            .expect("citation");
    }
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic(
            "Core ideas",
            Vec::new(),
            vec![graph_pick(member, "core")],
        )],
        ..PlanDraft::default()
    })
    .with_fill_fn(move |_| FillDraft {
        picks: vec![
            FillPickDraft {
                work_id: cited.to_string(),
                topic: "core IDEAS".to_owned(),
                rationale: "The member builds on it.".to_owned(),
                context_links: Vec::new(),
            },
            FillPickDraft {
                work_id: stranger.to_string(),
                topic: "Core ideas".to_owned(),
                rationale: "Made up.".to_owned(),
                context_links: Vec::new(),
            },
            FillPickDraft {
                work_id: cited.to_string(),
                topic: "Core ideas".to_owned(),
                rationale: "Again.".to_owned(),
                context_links: Vec::new(),
            },
        ],
        prerequisites: vec![
            PrerequisiteDraft {
                from: cited.to_string(),
                to: member.to_string(),
            },
            PrerequisiteDraft {
                from: stranger.to_string(),
                to: member.to_string(),
            },
        ],
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;

    let asked = planner.fill_requests();
    assert_eq!(
        asked.len(),
        1,
        "the topic had one verified Work, so the fill pass ran"
    );
    let offered: HashSet<WorkId> = asked[0]
        .candidates
        .iter()
        .map(|work| work.work_id)
        .collect();
    assert_eq!(
        offered,
        HashSet::from([cited, citing, other]),
        "neighbors in both directions"
    );
    assert!(asked[0].members.iter().any(|work| work.work_id == member));
    assert_eq!(asked[0].topics[0].verified, 1);
    assert_eq!(count_at(&view, &["fill", "offered"]), 3);
    assert_eq!(count_at(&view, &["fill", "picks_seen"]), 3);
    assert_eq!(count_at(&view, &["fill", "admitted"]), 1);
    assert_eq!(count_at(&view, &["fill", "rejected_ids"]), 1);
    assert_eq!(count_at(&view, &["fill", "duplicate"]), 1);

    let detail = state
        .list_detail(view.list_id.expect("list"))
        .await
        .expect("detail");
    let node_of = |work: WorkId| {
        detail
            .nodes
            .iter()
            .find(|node| node.node.work_id == Some(work))
    };
    assert!(node_of(cited).is_some());
    assert!(node_of(stranger).is_none());
    assert!(node_of(other).is_none(), "offered but not picked");
    let prerequisites: Vec<_> = detail
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Prerequisite)
        .collect();
    assert_eq!(
        prerequisites.len(),
        1,
        "the edge from a stranger was dropped"
    );
    assert_eq!(
        prerequisites[0].from,
        node_of(cited).expect("cited").node.id
    );
    assert_eq!(count_at(&view, &["edges", "planned"]), 2);
    assert_eq!(count_at(&view, &["edges", "added"]), 1);
    assert_eq!(count_at(&view, &["edges", "dropped_unknown"]), 1);
}

#[tokio::test]
async fn should_drop_and_count_the_edge_that_closes_a_cycle_when_admitting() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![
            topic("Alpha", Vec::new(), Vec::new()),
            topic("Beta", Vec::new(), Vec::new()),
            topic("Gamma", Vec::new(), Vec::new()),
        ],
        topic_edges: vec![
            prerequisite("Alpha", "Beta"),
            prerequisite("Beta", "Gamma"),
            prerequisite("Gamma", "Alpha"),
            prerequisite("Alpha", "Ghost"),
        ],
        assumed_known: Vec::new(),
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;

    assert_eq!(view.state, GenerationState::Succeeded, "{view:?}");
    assert_eq!(count_at(&view, &["edges", "planned"]), 4);
    assert_eq!(count_at(&view, &["edges", "added"]), 2);
    assert_eq!(count_at(&view, &["edges", "dropped_cycle"]), 1);
    assert_eq!(count_at(&view, &["edges", "dropped_unknown"]), 1);
    let detail = state
        .list_detail(view.list_id.expect("list"))
        .await
        .expect("detail");
    assert_eq!(detail.edges.len(), 2);
    assert_eq!(count_at(&view, &["topics", "empty"]), 3);
}

#[tokio::test]
async fn should_admit_what_verified_and_stop_before_the_fill_pass_when_the_budget_is_spent() {
    let graph = graph_or_skip!();
    let tag = marker();
    let member = titled_work(&graph, &format!("{tag}-m"), &format!("Member {tag}")).await;
    let neighbor = titled_work(&graph, &format!("{tag}-n"), "Neighbor").await;
    graph
        .citations
        .add_assertion(member, neighbor, None)
        .await
        .expect("citation");
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic(
            "Thin topic",
            vec![verified_claim(&tag, 1, "survey")],
            vec![graph_pick(member, "core")],
        )],
        ..PlanDraft::default()
    })
    .with_costs(0.06, 0.06);
    let state = state_with(&graph, vec![record(&tag, 1)], &planner).await;
    let mut asked = request(&goal_of(&tag));
    asked.budget_usd = 0.05;
    let view = run(&state, asked).await;

    assert_eq!(view.state, GenerationState::OverBudget, "{view:?}");
    assert!(
        planner.fill_requests().is_empty(),
        "the fill pass did not start"
    );
    assert!((view.spend_usd - 0.06).abs() < 1e-6, "{}", view.spend_usd);
    assert_eq!(
        view.report.as_ref().expect("report")["budget"]["stopped_before"],
        "fill"
    );
    let detail = state
        .list_detail(view.list_id.expect("partial list"))
        .await
        .expect("detail");
    assert_eq!(
        count_at(&view, &["admitted", "works"]),
        2,
        "everything verified so far"
    );
    assert_eq!(detail.nodes.len(), 3);
    assert_counts_sum(&view);
}

#[tokio::test]
async fn should_succeed_when_the_plan_overspends_but_no_fill_pass_is_needed() {
    let graph = graph_or_skip!();
    let tag = marker();
    let mut first = Vec::new();
    let mut second = Vec::new();
    for index in 0..4 {
        first.push(graph_pick(
            titled_work(
                &graph,
                &format!("{tag}-a{index}"),
                &format!("First {index} {tag}"),
            )
            .await,
            "core",
        ));
        second.push(graph_pick(
            titled_work(
                &graph,
                &format!("{tag}-b{index}"),
                &format!("Second {index} {tag}"),
            )
            .await,
            "core",
        ));
    }
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![
            topic("One", Vec::new(), first),
            topic("Two", Vec::new(), second),
        ],
        ..PlanDraft::default()
    })
    .with_costs(0.5, 0.5);
    let state = state_with(&graph, Vec::new(), &planner).await;
    let mut asked = request(&goal_of(&tag));
    asked.budget_usd = 0.10;
    let view = run(&state, asked).await;

    assert_eq!(view.state, GenerationState::Succeeded, "{view:?}");
    assert!(planner.fill_requests().is_empty());
    assert!(view.spend_usd > 0.10);
}

#[tokio::test]
async fn should_stop_over_budget_without_a_list_when_the_ledger_refuses_the_plan_pass() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::failing(PlannerError::BudgetExceeded("spent".to_owned()));
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;
    assert_eq!(view.state, GenerationState::OverBudget);
    assert_eq!(view.list_id, None);
    assert_eq!(
        view.report.as_ref().expect("report")["budget"]["stopped_before"],
        "plan"
    );
}

#[tokio::test]
async fn should_propose_nodes_and_leave_the_list_alone_when_extending_an_existing_list() {
    let graph = graph_or_skip!();
    let tag = marker();
    let held = titled_work(&graph, &format!("{tag}-h"), &format!("Held {tag}")).await;
    let fresh = titled_work(&graph, &format!("{tag}-f"), &format!("Fresh {tag}")).await;
    let extra = titled_work(&graph, &format!("{tag}-e"), &format!("Extra {tag}")).await;
    let state = state_with(&graph, Vec::new(), &FakePlanner::new(PlanDraft::default())).await;
    let list = state
        .create_list(
            &crate::lists::parse_title(&format!("Mine {tag}")).expect("title"),
            None,
            None,
        )
        .await
        .expect("list")
        .list
        .list
        .id;
    let basics = state
        .add_list_node(
            list,
            crate::lists::AddNode::Topic {
                label: crate::lists::parse_topic_label("Basics").expect("label"),
                summary: None,
            },
        )
        .await
        .expect("topic");
    let held_node = state
        .add_list_node(
            list,
            crate::lists::AddNode::Work {
                work: held,
                via: None,
                topics: vec![basics.node.id],
                note: Some("mine".to_owned()),
            },
        )
        .await
        .expect("work");

    let planner = FakePlanner::new(PlanDraft {
        topics: vec![
            topic(
                "basics",
                Vec::new(),
                vec![graph_pick(held, "core"), graph_pick(fresh, "core")],
            ),
            topic("Extras", Vec::new(), vec![graph_pick(extra, "frontier")]),
        ],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let mut asked = request(&goal_of(&tag));
    asked.into_list = Some(list);
    let view = run(&state, asked).await;

    assert_eq!(view.state, GenerationState::Succeeded, "{view:?}");
    assert_eq!(view.list_id, Some(list));
    assert_eq!(count_at(&view, &["candidates", "already_in_list"]), 1);
    assert_eq!(count_at(&view, &["topics", "reused"]), 1);
    assert_eq!(count_at(&view, &["topics", "admitted"]), 1);
    assert_counts_sum(&view);

    let detail = state.list_detail(list).await.expect("detail");
    let by_work = |work: WorkId| {
        detail
            .nodes
            .iter()
            .find(|node| node.node.work_id == Some(work))
            .expect("node")
    };
    assert_eq!(by_work(held).node.review, ListReview::Accepted);
    assert_eq!(
        by_work(held).node.position,
        held_node.node.position,
        "existing order kept"
    );
    assert_eq!(by_work(held).node.note.as_deref(), Some("mine"));
    for work in [fresh, extra] {
        let node = by_work(work);
        assert_eq!(
            node.node.review,
            ListReview::Proposed,
            "agent additions wait for review"
        );
        assert_eq!(node.node.source, NodeSource::Agent);
        assert_eq!(node.node.generation_id, Some(view.id));
        assert!(
            node.node.position > held_node.node.position,
            "appended after existing nodes"
        );
    }
    let extras = detail
        .nodes
        .iter()
        .find(|node| node.node.label.as_deref() == Some("Extras"))
        .expect("new topic");
    assert_eq!(extras.node.review, ListReview::Proposed);
    let basics_node = detail
        .nodes
        .iter()
        .find(|node| node.node.id == basics.node.id)
        .expect("basics");
    assert_eq!(basics_node.node.review, ListReview::Accepted);
    let proposed_edges = detail
        .edges
        .iter()
        .filter(|edge| edge.review == ListReview::Proposed)
        .count();
    assert_eq!(
        proposed_edges, 2,
        "fresh under Basics and extra under Extras"
    );
    assert_eq!(detail.list.proposed_count, 3);
}

#[tokio::test]
async fn should_account_for_every_claim_when_the_plan_repeats_and_overflows() {
    let graph = graph_or_skip!();
    let tag = marker();
    let mut topics = vec![topic(
        "Repeats",
        vec![
            verified_claim(&tag, 1, "core"),
            verified_claim(&tag, 1, "foundation"),
            unknown_claim(&tag, "core"),
            ClaimDraft {
                title: "   ".to_owned(),
                role: "core".to_owned(),
                ..ClaimDraft::default()
            },
            ClaimDraft {
                role: "central".to_owned(),
                ..verified_claim(&tag, 2, "core")
            },
        ],
        Vec::new(),
    )];
    for index in 0..13 {
        topics.push(topic(
            &format!("Topic {index}"),
            vec![unknown_claim(&format!("{tag}-{index}"), "survey")],
            Vec::new(),
        ));
    }
    let planner = FakePlanner::new(PlanDraft {
        topics,
        ..PlanDraft::default()
    });
    let state = state_with(&graph, vec![record(&tag, 1), record(&tag, 2)], &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;

    assert_counts_sum(&view);
    let counted = report(&view).candidates;
    assert_eq!(counted.seen, 18);
    assert_eq!(counted.duplicate, 1, "two claims resolved to one Work");
    assert_eq!(counted.invalid, 2, "blank title, unknown role");
    assert_eq!(counted.topic_dropped, 2, "topics past the twelfth");
    assert_eq!(count_at(&view, &["topics", "dropped_over_limit"]), 2);
}

#[tokio::test]
async fn should_cap_works_at_the_requested_size_when_the_plan_names_more() {
    let graph = graph_or_skip!();
    let tag = marker();
    let mut picks = Vec::new();
    for index in 0..10 {
        picks.push(graph_pick(
            titled_work(
                &graph,
                &format!("{tag}-w{index}"),
                &format!("Work {index:02} {tag}"),
            )
            .await,
            "core",
        ));
    }
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![
            topic("All", Vec::new(), picks[..8].to_vec()),
            topic("More", Vec::new(), picks[8..].to_vec()),
        ],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;
    assert_eq!(count_at(&view, &["candidates", "over_size"]), 2);
    assert_eq!(count_at(&view, &["admitted", "works"]), 8);
    assert_counts_sum(&view);
}

// -- the knowledge profile (D16) ---------------------------------------------------------

async fn read_work(state: &AppState, graph: &GraphState, tag: &str, title: &str) -> WorkId {
    let work = titled_work(graph, tag, title).await;
    let service = state.history_service().await.expect("history");
    service
        .ingest(&[NewEvent {
            occurred_at: Utc::now(),
            session_id: uuid::Uuid::new_v4(),
            kind: "paper_left".to_owned(),
            paper_id: None,
            work_id: Some(work),
            list_id: None,
            dwell_ms: Some(3_600_000),
            detail: json!({}),
            client_seq: 1,
        }])
        .await
        .expect("ingest");
    state
        .set_work_reading_state(work, ReadingStatus::Read)
        .await
        .expect("read");
    work
}

fn link(work: &WorkId, relation: &str) -> ContextLinkDraft {
    ContextLinkDraft {
        work_id: work.to_string(),
        relation: relation.to_owned(),
        note: "Same idea, new setting.".to_owned(),
    }
}

#[tokio::test]
async fn should_keep_only_profile_links_when_a_context_link_names_another_work() {
    let graph = graph_or_skip!();
    let tag = marker();
    let outsider = titled_work(&graph, &format!("{tag}-x"), &format!("Not read {tag}")).await;
    let bootstrap = state_with(&graph, Vec::new(), &FakePlanner::new(PlanDraft::default())).await;
    let read = read_work(
        &bootstrap,
        &graph,
        &format!("{tag}-r"),
        &format!("Read before {tag}"),
    )
    .await;
    let mut claim = verified_claim(&tag, 1, "core");
    claim.context_links = vec![link(&read, "builds_on"), link(&outsider, "extends")];
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic("Linked", vec![claim], Vec::new())],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, vec![record(&tag, 1)], &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;

    let supplied = &planner.plan_requests()[0].profile;
    let entry = supplied
        .iter()
        .find(|entry| entry.work_id == read)
        .expect("read Work in the profile");
    assert_eq!(entry.status, ReadingStatus::Read);
    assert_eq!(entry.dwell_minutes, 60);
    assert!(supplied.iter().all(|entry| entry.work_id != outsider));
    assert_eq!(count_at(&view, &["context_links", "accepted"]), 1);
    assert_eq!(count_at(&view, &["context_links", "rejected"]), 1);
    let detail = state
        .list_detail(view.list_id.expect("list"))
        .await
        .expect("detail");
    let linked = detail
        .nodes
        .iter()
        .find(|node| node.node.kind == NodeKind::Work)
        .expect("verified Work");
    let links = linked.node.context_links.as_array().expect("links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["work_id"], read.to_string());
    assert_eq!(links[0]["relation"], "builds_on");
}

#[tokio::test]
async fn should_record_assumed_known_topics_with_their_evidence_works() {
    let graph = graph_or_skip!();
    let tag = marker();
    let bootstrap = state_with(&graph, Vec::new(), &FakePlanner::new(PlanDraft::default())).await;
    let title = format!("Attention is read {tag}");
    let read = read_work(&bootstrap, &graph, &format!("{tag}-r"), &title).await;
    let outsider = titled_work(&graph, &format!("{tag}-x"), "Never read").await;
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic("New ground", Vec::new(), Vec::new())],
        topic_edges: Vec::new(),
        assumed_known: vec![
            AssumedKnownDraft {
                topic: "Transformers".to_owned(),
                reason: "Read the original paper.".to_owned(),
                evidence: vec![read.to_string(), outsider.to_string()],
            },
            AssumedKnownDraft {
                topic: "Optimization".to_owned(),
                reason: "No evidence at all.".to_owned(),
                evidence: vec![outsider.to_string()],
            },
        ],
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;

    let known = &view.report.as_ref().expect("report")["assumed_known"];
    assert_eq!(known.as_array().expect("array").len(), 1);
    assert_eq!(known[0]["topic"], "Transformers");
    assert_eq!(known[0]["evidence"][0]["work_id"], read.to_string());
    assert_eq!(known[0]["evidence"][0]["title"], title);
    assert_eq!(count_at(&view, &["assumed_known_dropped"]), 1);
    assert_eq!(count_at(&view, &["assumed_known_evidence_rejected"]), 2);
}

#[tokio::test]
async fn should_show_anchors_and_their_neighbors_when_the_plan_pass_runs() {
    let graph = graph_or_skip!();
    let tag = marker();
    let anchor = titled_work(&graph, &format!("{tag}-a"), "The anchor paper").await;
    let cited = titled_work(&graph, &format!("{tag}-c"), "Cited by the anchor").await;
    let citing = titled_work(&graph, &format!("{tag}-g"), "Citing the anchor").await;
    graph
        .citations
        .add_assertion(anchor, cited, None)
        .await
        .expect("citation");
    graph
        .citations
        .add_assertion(citing, anchor, None)
        .await
        .expect("citation");
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic("Start", Vec::new(), vec![graph_pick(cited, "core")])],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let mut asked = request(&goal_of(&tag));
    asked.anchors = vec![anchor, anchor];
    let view = run(&state, asked).await;

    let plan = &planner.plan_requests()[0];
    assert_eq!(plan.anchors.len(), 1, "a repeated anchor counts once");
    assert_eq!(plan.anchors[0].work_id, anchor);
    let context: HashSet<WorkId> = plan.graph_context.iter().map(|work| work.work_id).collect();
    assert!(context.contains(&cited) && context.contains(&citing));
    assert!(!context.contains(&anchor));
    assert_eq!(
        count_at(&view, &["candidates", "in_graph"]),
        1,
        "the neighbor was citable by ID"
    );
}

#[tokio::test]
async fn should_cap_expansions_at_ten_when_many_verified_works_want_them() {
    let graph = graph_or_skip!();
    let tag = marker();
    let claims_one: Vec<ClaimDraft> = (0..6)
        .map(|index| verified_claim(&tag, index, "core"))
        .collect();
    let claims_two: Vec<ClaimDraft> = (6..12)
        .map(|index| verified_claim(&tag, index, "frontier"))
        .collect();
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![
            topic("Core", claims_one, Vec::new()),
            topic("Frontier", claims_two, Vec::new()),
        ],
        ..PlanDraft::default()
    });
    let found: Vec<(String, Value)> = (0..12).map(|index| record(&tag, index)).collect();
    let state = state_with(&graph, found, &planner).await;
    let mut asked = request(&goal_of(&tag));
    asked.size = 20;
    let view = run(&state, asked).await;

    assert_eq!(count_at(&view, &["candidates", "verified"]), 12, "{view:?}");
    let attempted = count_at(&view, &["expansions", "run"])
        + count_at(&view, &["expansions", "failed"])
        + count_at(&view, &["expansions", "cached"]);
    assert_eq!(
        attempted + count_at(&view, &["expansions", "skipped_cap"]),
        18,
        "6 core x2 + 6 frontier x1"
    );
    assert_eq!(
        count_at(&view, &["expansions", "run"]) + count_at(&view, &["expansions", "failed"]),
        10
    );
    assert_eq!(count_at(&view, &["expansions", "skipped_cap"]), 8);
}

// -- failures ----------------------------------------------------------------------------

#[tokio::test]
async fn should_record_the_failure_and_write_no_list_when_the_planner_is_unavailable() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::failing(PlannerError::Unavailable("OPENROUTER_API_KEY".to_owned()));
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;
    assert_eq!(view.state, GenerationState::Failed);
    assert_eq!(view.list_id, None);
    assert!(
        view.error
            .as_deref()
            .is_some_and(|error| error.contains("OPENROUTER_API_KEY")),
        "{view:?}"
    );
    assert!(view.finished_at.is_some());
}

#[tokio::test]
async fn should_fail_without_a_list_when_the_plan_admits_nothing() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::new(PlanDraft::default());
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;
    assert_eq!(view.state, GenerationState::Failed);
    assert_eq!(view.list_id, None);
    assert!(view.error.is_some());
}

#[tokio::test]
async fn should_reject_a_request_before_running_when_it_is_malformed() {
    let state = AppState::new(
        tempfile::tempdir().expect("library").keep(),
        tempfile::tempdir().expect("data").keep(),
    )
    .await
    .expect("state");
    let invalid = |request: GenerateRequest| request.validate().expect_err("invalid").code();
    assert_eq!(invalid(GenerateRequest::new("   ")), "invalid_goal");
    assert_eq!(
        invalid(GenerateRequest::new("x".repeat(2001))),
        "invalid_goal"
    );
    for size in [0, 7, 61, 1000] {
        assert_eq!(
            invalid(GenerateRequest {
                size,
                ..GenerateRequest::new("goal")
            }),
            "invalid_size"
        );
    }
    for budget in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            invalid(GenerateRequest {
                budget_usd: budget,
                ..GenerateRequest::new("goal")
            }),
            "invalid_budget"
        );
    }
    GenerateRequest::new("é goal")
        .validate()
        .expect("unicode goals are fine");
    let error = state
        .generate_list(GenerateRequest::new("goal"))
        .await
        .expect_err("no database");
    assert!(matches!(error, GenerationError::Graph(_)));
}

#[tokio::test]
async fn should_compare_goals_without_case_or_spacing_when_keying_runs() {
    let one = GenerateRequest::new("  Learn   RLHF\treward ");
    let two = GenerateRequest::new("learn rlhf reward");
    assert_eq!(one.goal_key(), two.goal_key());
}

// -- the API -----------------------------------------------------------------------------

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let response = build_router(state.clone(), None)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
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

async fn bare_state() -> AppState {
    let library = tempfile::tempdir().expect("library");
    let data = tempfile::tempdir().expect("data");
    let state = AppState::new(library.path(), data.path())
        .await
        .expect("state");
    std::mem::forget((library, data));
    state
}

#[tokio::test]
async fn should_answer_400_before_touching_the_database_when_the_generate_body_is_invalid() {
    let state = bare_state().await;
    let cases = [
        (json!({}), "invalid_body"),
        (json!([]), "invalid_body"),
        (json!({"goal": "g", "extra": 1}), "invalid_body"),
        (json!({"goal": 3}), "invalid_body"),
        (json!({"goal": "g", "size": 7}), "invalid_size"),
        (json!({"goal": "g", "size": -4}), "invalid_size"),
        (json!({"goal": "g", "level": "expert"}), "invalid_level"),
        (json!({"goal": "g", "anchors": ["nope"]}), "invalid_work_id"),
        (json!({"goal": "g", "into_list": "nope"}), "invalid_list_id"),
    ];
    for (body, code) in cases {
        let (status, json) = call(&state, "POST", "/api/lists/generate", Some(&body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
        assert_eq!(json["error"], code, "{body}");
    }
    let (status, json) = call(&state, "POST", "/api/lists/generate", None).await;
    assert_eq!(
        (status, &json["error"]),
        (StatusCode::BAD_REQUEST, &json!("invalid_body"))
    );
}

#[tokio::test]
async fn should_answer_503_when_the_graph_is_not_configured() {
    let state = bare_state().await;
    let (status, json) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": "learn"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{json}");
    assert_eq!(json["error"], "graph_unavailable");
    let id = lysilogy_core::GenerationId::new();
    let (status, json) = call(&state, "GET", &format!("/api/generations/{id}"), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{json}");
    let (status, json) = call(&state, "GET", "/api/generations/not-an-id", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    assert_eq!(json["error"], "invalid_generation_id");
}

#[tokio::test]
async fn should_mount_the_history_routes_when_building_the_router() {
    let state = bare_state().await;
    for (method, uri) in [
        ("GET", "/api/history"),
        ("GET", "/api/history/inspected"),
        ("GET", "/api/history/barely-read"),
    ] {
        let (status, json) = call(&state, method, uri, None).await;
        assert_ne!(json["error"], "unknown_api", "{uri} must be mounted");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}: {json}");
    }
}

async fn wait_for_end(state: &AppState, id: &str) -> Value {
    for _ in 0..200 {
        let (status, json) = call(state, "GET", &format!("/api/generations/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        if json["state"] != "running" {
            return json;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the generation never finished");
}

#[tokio::test]
async fn should_run_in_the_background_when_generation_is_posted() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic(
            "Only topic",
            vec![verified_claim(&tag, 1, "survey")],
            Vec::new(),
        )],
        ..PlanDraft::default()
    })
    .with_costs(0.01, 0.0);
    let state = state_with(&graph, vec![record(&tag, 1)], &planner).await;
    let body = json!({"goal": goal_of(&tag), "size": 8, "level": "intro", "budget_usd": 0.05});
    let (status, json) = call(&state, "POST", "/api/lists/generate", Some(&body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{json}");
    let id = json["generation_id"]
        .as_str()
        .expect("generation id")
        .to_owned();
    assert!(id.starts_with('g'));
    let done = wait_for_end(&state, &id).await;
    assert_eq!(done["state"], "succeeded", "{done}");
    assert!(
        done["list_id"]
            .as_str()
            .is_some_and(|list| list.starts_with('p'))
    );
    assert!((done["spend_usd"].as_f64().expect("spend") - 0.01).abs() < 1e-6);
    assert_eq!(done["report"]["candidates"]["verified"], 1);
    assert_eq!(done["goal"], goal_of(&tag));
    let list = done["list_id"].as_str().expect("list");
    let (status, detail) = call(&state, "GET", &format!("/api/lists/{list}"), None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["list"]["origin"], "generated");
}

#[tokio::test]
async fn should_answer_409_with_the_running_id_when_the_same_goal_runs_twice() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic("Slow", Vec::new(), Vec::new())],
        ..PlanDraft::default()
    })
    .with_delay(Duration::from_millis(400));
    let state = state_with(&graph, Vec::new(), &planner).await;
    let goal = goal_of(&tag);
    let (status, first) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": goal})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    let shouted = goal.to_uppercase();
    let (status, second) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": format!(" {shouted} ")})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{second}");
    assert_eq!(second["error"], "generation_running");
    assert_eq!(second["generation_id"], first["generation_id"]);
    let id = first["generation_id"].as_str().expect("id");
    let (_, running) = call(&state, "GET", &format!("/api/generations/{id}"), None).await;
    assert_eq!(running["state"], "running");
    wait_for_end(&state, id).await;
    // The goal is free again once the run ends.
    let (status, third) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": goal})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{third}");
    wait_for_end(&state, third["generation_id"].as_str().expect("id")).await;
}

#[tokio::test]
async fn should_answer_404_when_the_anchor_list_or_generation_does_not_exist() {
    let graph = graph_or_skip!();
    let planner = FakePlanner::new(PlanDraft::default());
    let state = state_with(&graph, Vec::new(), &planner).await;
    let tag = marker();
    let (status, json) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": goal_of(&tag), "anchors": [WorkId::new().to_string()]})),
    )
    .await;
    assert_eq!(
        (status, &json["error"]),
        (StatusCode::NOT_FOUND, &json!("work_not_found"))
    );
    let (status, json) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(
            &json!({"goal": goal_of(&tag), "into_list": lysilogy_core::ListId::new().to_string()}),
        ),
    )
    .await;
    assert_eq!(
        (status, &json["error"]),
        (StatusCode::NOT_FOUND, &json!("list_not_found"))
    );
    let (status, json) = call(
        &state,
        "GET",
        &format!("/api/generations/{}", lysilogy_core::GenerationId::new()),
        None,
    )
    .await;
    assert_eq!(
        (status, &json["error"]),
        (StatusCode::NOT_FOUND, &json!("generation_not_found"))
    );
    let (status, json) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": goal_of(&tag), "budget_usd": 0})),
    )
    .await;
    assert_eq!(
        (status, &json["error"]),
        (StatusCode::BAD_REQUEST, &json!("invalid_budget"))
    );
    assert!(
        planner.plan_requests().is_empty(),
        "no request ran the planner"
    );
}

#[tokio::test]
async fn should_retry_pending_claims_when_a_generated_list_is_verified() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic(
            "Unsure",
            vec![unknown_claim(&tag, "core")],
            Vec::new(),
        )],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;
    let list = view.list_id.expect("list");
    let (status, json) = call(&state, "POST", &format!("/api/lists/{list}/verify"), None).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["checked"], 0, "an unverified claim is not pending");
}

#[tokio::test]
async fn should_not_double_count_when_two_claims_name_a_work_already_in_the_graph() {
    let graph = graph_or_skip!();
    let tag = marker();
    let existing = titled_work(
        &graph,
        &format!("{tag}-e"),
        &format!("Verified paper {}", needle(&tag, 7)),
    )
    .await;
    graph
        .works
        .set_display_year(existing, 2021)
        .await
        .expect("year");
    let planner = FakePlanner::new(PlanDraft {
        topics: vec![topic(
            "Known",
            vec![verified_claim(&tag, 7, "foundation")],
            Vec::new(),
        )],
        ..PlanDraft::default()
    });
    let state = state_with(&graph, Vec::new(), &planner).await;
    let view = run(&state, request(&goal_of(&tag))).await;
    assert_eq!(count_at(&view, &["candidates", "matched"]), 1, "{view:?}");
    let detail = state
        .list_detail(view.list_id.expect("list"))
        .await
        .expect("detail");
    assert!(
        detail
            .nodes
            .iter()
            .any(|node| node.node.work_id == Some(existing))
    );
    assert_counts_sum(&view);
}

#[tokio::test]
async fn should_close_the_run_and_free_the_goal_when_the_job_panics() {
    let graph = graph_or_skip!();
    let tag = marker();
    let planner = FakePlanner::new(PlanDraft::default()).panicking();
    let state = state_with(&graph, Vec::new(), &planner).await;
    let goal = goal_of(&tag);
    let (status, started) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": goal})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    let done = wait_for_end(&state, started["generation_id"].as_str().expect("id")).await;
    assert_eq!(done["state"], "failed", "{done}");
    assert!(
        done["error"]
            .as_str()
            .is_some_and(|error| error.contains("the job stopped"))
    );
    let (status, again) = call(
        &state,
        "POST",
        "/api/lists/generate",
        Some(&json!({"goal": goal})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the goal is free again: {again}"
    );
}
