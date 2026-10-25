//! The generation pipeline: context, plan, verify, fill, admit.
//!
//! Everything that can be decided without the model is decided here, in this order:
//! the model proposes and the application admits. A run is one [`Run`]; the service
//! methods on [`AppState`] start it, wait for it, and read it back.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Utc};
use lysilogy_core::{
    Claim, ClaimState, EdgeKind, ExpansionDirection, GenerationId, GenerationState, LIST_EDGES_MAX,
    LIST_TOPICS_MAX, LIST_WORKS_MAX, ListId, ListOrigin, ListReview, ListTitle, NodeKind,
    NodeSource, ProviderRecord, Work, WorkId, WorkspaceId, default_list_title, title_key,
};
use lysilogy_db::DbError;
use lysilogy_db::generation::{
    Admission, AdmissionEdge, GenerationRepository, GenerationRow, NodeRef,
};
use lysilogy_db::list::{ArchiveScope, NewList, NewNode, NewNodeContent};
use lysilogy_ingest::verify_claim::{ClaimResolution, VerifyContext, verify_claim};
use serde::Serialize;
use serde_json::{Value, json};

use super::report::{PassCost, Report};
use super::validate::{
    CheckedLink, CheckedPlan, FillBounds, Supplied, check_fill, check_plan, links_json,
};
use super::{
    ANCHOR_NEIGHBORS_MAX, EXPANSIONS_MAX, ExistingList, FILL_CANDIDATES_MAX, FillRequest,
    FillTopic, GRAPH_TITLE_HITS_MAX, GenerateRequest, GenerationError, PROFILE_PROMPT_CHARS_MAX,
    PlanRequest, PlannerError, ProfileBrief, ReadingListPlanner, Role, TOPIC_WORKS_MIN, WorkBrief,
    openrouter::OpenRouterPlanner,
};
use crate::graph::GraphState;
use crate::history::{HistoryService, Profile};
use crate::lists::OrderStrategy;
use crate::{AppState, Error};

/// Most anchors a request may name.
/// Claims a run may add regardless of size; past it, `max(this, size / 2)` caps claims.
const CLAIMS_MIN_CAP: usize = 5;
const ANCHORS_MAX: usize = 20;
/// Most keywords the goal contributes to the title search.
const KEYWORDS_MAX: usize = 6;
/// Shortest word that counts as a keyword.
const KEYWORD_MIN_CHARS: usize = 3;
/// Most lists whose topics the plan is told about.
const EXISTING_LISTS_MAX: usize = 30;
/// Most topics listed per existing list.
const EXISTING_TOPICS_MAX: usize = 12;
/// Most members shown to the fill pass.
const FILL_MEMBERS_MAX: usize = 40;
/// Words that say nothing about the subject of a goal.
const STOPWORDS: &[&str] = &[
    "about",
    "and",
    "basics",
    "for",
    "from",
    "how",
    "intro",
    "introduction",
    "into",
    "learn",
    "learning",
    "list",
    "papers",
    "read",
    "reading",
    "study",
    "that",
    "the",
    "this",
    "understand",
    "understanding",
    "what",
    "with",
];

/// A run as the API and CLI show it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GenerationView {
    pub id: GenerationId,
    pub goal: String,
    pub state: GenerationState,
    pub report: Option<Value>,
    pub spend_usd: f64,
    pub list_id: Option<ListId>,
    pub model_key: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<GenerationRow> for GenerationView {
    fn from(row: GenerationRow) -> Self {
        Self {
            id: row.id,
            goal: row.goal,
            state: row.state,
            report: row.report,
            spend_usd: row.spend_usd,
            list_id: row.list_id,
            model_key: row.model_key,
            error: row.error,
            created_at: row.created_at,
            finished_at: row.finished_at,
        }
    }
}

impl GenerationView {
    /// A few lines for the terminal.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "Generation {} {}", self.id, self.state);
        if let Some(list) = self.list_id {
            let _ = writeln!(out, "List {list}");
        }
        let _ = writeln!(out, "Spent ${:.4}", self.spend_usd);
        if let Some(error) = &self.error {
            let _ = writeln!(out, "Error: {error}");
        }
        let Some(report) = &self.report else {
            return out;
        };
        let count = |path: &[&str]| {
            path.iter()
                .try_fold(report, |value, key| value.get(*key))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let _ = writeln!(
            out,
            "Claims seen {}: {} verified, {} matched, {} in graph, {} unresolved, {} dropped",
            count(&["candidates", "seen"]),
            count(&["candidates", "verified"]),
            count(&["candidates", "matched"]),
            count(&["candidates", "in_graph"]),
            count(&["candidates", "unverified"])
                + count(&["candidates", "ambiguous"])
                + count(&["candidates", "contradicted"])
                + count(&["candidates", "pending"]),
            count(&["candidates", "invalid"])
                + count(&["candidates", "topic_dropped"])
                + count(&["candidates", "over_limit"])
                + count(&["candidates", "rejected_ids"])
                + count(&["candidates", "duplicate"])
                + count(&["candidates", "already_in_list"])
                + count(&["candidates", "over_size"])
                + count(&["candidates", "list_full"]),
        );
        let _ = writeln!(
            out,
            "Admitted {} works, {} claims, {} topics, {} edges",
            count(&["admitted", "works"]),
            count(&["admitted", "claims"]),
            count(&["admitted", "topics"]),
            count(&["admitted", "edges"]),
        );
        out
    }
}

// ---------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------

/// The words of a goal worth searching titles for: lower case, letters and digits, at
/// least three characters, no filler, each once, at most [`KEYWORDS_MAX`].
#[must_use]
pub fn goal_keywords(goal: &str) -> Vec<String> {
    let mut keywords: Vec<String> = Vec::new();
    for word in goal.split(|character: char| !character.is_alphanumeric()) {
        let key = title_key(word);
        let usable = key.chars().count() >= KEYWORD_MIN_CHARS
            && key.chars().all(char::is_alphanumeric)
            && !STOPWORDS.contains(&key.as_str())
            && !keywords.contains(&key);
        if usable {
            keywords.push(key);
        }
        if keywords.len() >= KEYWORDS_MAX {
            break;
        }
    }
    keywords
}

/// The list title a goal makes.
#[must_use]
pub fn title_from_goal(goal: &str) -> ListTitle {
    default_list_title(Some(goal))
}

/// A run's spend against its budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spend {
    pub budget: f64,
    pub spent: f64,
    /// What the most recent pass cost, taken as the next pass's estimate.
    pub last_pass: f64,
}

impl Spend {
    /// Whether another pass would take the run past its budget.
    #[must_use]
    pub fn exhausted(&self) -> bool {
        self.spent >= self.budget || self.spent + self.last_pass > self.budget
    }

    fn record(&mut self, pass: &'static str, cost_usd: f64, report: &mut Report) {
        let cost = if cost_usd.is_finite() {
            cost_usd.max(0.0)
        } else {
            0.0
        };
        self.spent += cost;
        self.last_pass = cost;
        report.passes.push(PassCost {
            pass,
            cost_usd: cost,
        });
        report.budget.spent_usd = self.spent;
        report.budget.exceeded = self.spent > self.budget;
    }
}

/// How a candidate came out of verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    Matched,
    Verified,
    InGraph,
    Unverified,
    Ambiguous,
    Contradicted,
    Pending,
}

#[derive(Clone, Debug)]
enum Body {
    Work(WorkId),
    Claim {
        claim: Claim,
        state: ClaimState,
        candidates: Vec<ProviderRecord>,
    },
}

/// A claim or graph Work on its way to the list.
#[derive(Clone, Debug)]
struct Item {
    body: Body,
    fate: Fate,
    topic: Option<usize>,
    role: Option<Role>,
    rationale: String,
    links: Vec<CheckedLink>,
}

/// What may still be added to the list.
#[derive(Clone, Copy, Debug)]
struct Room {
    /// Work nodes this run may still add.
    size: usize,
    /// Nodes the list can still hold.
    capacity: usize,
}

/// Decide what each candidate becomes: duplicates, Works already listed, and anything past
/// the size or the list's capacity are counted and dropped. Works come first, in plan
/// order, so claims never use up room a Work could have had.
fn classify(
    items: Vec<Item>,
    listed: &HashSet<WorkId>,
    room: Room,
    report: &mut Report,
) -> Vec<Item> {
    let (works, claims): (Vec<Item>, Vec<Item>) = items
        .into_iter()
        .partition(|item| matches!(item.body, Body::Work(_)));
    let mut taken: HashSet<WorkId> = HashSet::new();
    let mut kept: Vec<Item> = Vec::new();
    let mut work_count = 0;
    for item in works {
        let Body::Work(work) = item.body else {
            continue;
        };
        let candidates = &mut report.candidates;
        if taken.contains(&work) {
            candidates.duplicate += 1;
        } else if listed.contains(&work) {
            candidates.already_in_list += 1;
        } else if work_count >= room.size {
            candidates.over_size += 1;
        } else if kept.len() >= room.capacity {
            candidates.list_full += 1;
        } else {
            taken.insert(work);
            work_count += 1;
            count_fate(candidates, item.fate);
            kept.push(item);
        }
    }
    let claim_cap = (room.size / 2).max(CLAIMS_MIN_CAP);
    let mut claim_count = 0;
    for item in claims {
        if claim_count >= claim_cap {
            report.candidates.over_size += 1;
            continue;
        }
        if kept.len() >= room.capacity {
            report.candidates.list_full += 1;
            continue;
        }
        claim_count += 1;
        count_fate(&mut report.candidates, item.fate);
        kept.push(item);
    }
    kept
}

const fn count_fate(candidates: &mut super::report::Candidates, fate: Fate) {
    match fate {
        Fate::Matched => candidates.matched += 1,
        Fate::Verified => candidates.verified += 1,
        Fate::InGraph => candidates.in_graph += 1,
        Fate::Unverified => candidates.unverified += 1,
        Fate::Ambiguous => candidates.ambiguous += 1,
        Fate::Contradicted => candidates.contradicted += 1,
        Fate::Pending => candidates.pending += 1,
    }
}

/// Which directions a Work of this role is expanded in.
const fn expansion_directions(role: Role) -> &'static [ExpansionDirection] {
    match role {
        Role::Foundation => &[ExpansionDirection::References],
        Role::Core => &[ExpansionDirection::References, ExpansionDirection::CitedBy],
        Role::Survey => &[],
        Role::Frontier => &[ExpansionDirection::CitedBy],
    }
}

fn brief(work: &Work) -> Option<WorkBrief> {
    Some(WorkBrief {
        work_id: work.id,
        title: work.display_title.clone()?,
        year: work.display_year,
    })
}

/// The profile as the prompt carries it: best first, as many as fit the character budget.
fn profile_briefs(profile: &Profile) -> Vec<ProfileBrief> {
    let mut used = 0;
    let mut briefs = Vec::new();
    for entry in &profile.entries {
        let Some(title) = &entry.title else {
            continue;
        };
        let cost = 120
            + title.chars().count()
            + entry
                .lists
                .iter()
                .map(|list| list.title.chars().count() + 4)
                .sum::<usize>();
        if used + cost > PROFILE_PROMPT_CHARS_MAX {
            break;
        }
        used += cost;
        briefs.push(ProfileBrief {
            work_id: entry.work_id,
            title: title.clone(),
            year: entry.year,
            status: entry.status,
            dwell_minutes: entry.dwell_ms / 60_000,
            lists: entry.lists.iter().map(|list| list.title.clone()).collect(),
        });
    }
    briefs
}

// ---------------------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------------------

/// A request that has passed validation and has a row.
#[derive(Clone, Debug)]
pub struct Prepared {
    id: GenerationId,
    request: GenerateRequest,
    goal_key: String,
}

impl Prepared {
    /// The run's ID.
    #[must_use]
    pub const fn id(&self) -> GenerationId {
        self.id
    }
}

/// How a run ended.
struct RunEnd {
    state: GenerationState,
    list_id: Option<ListId>,
}

/// What the model is shown, and what it was shown.
struct Context {
    plan: PlanRequest,
    supplied: Supplied,
    profile_titles: HashMap<WorkId, String>,
}

/// The existing list a run extends.
struct Target {
    id: ListId,
    works: HashSet<WorkId>,
    topics: HashMap<String, lysilogy_core::NodeId>,
    items: usize,
    /// Every topic node, labelled or not.
    topic_total: usize,
    edge_total: usize,
}

struct Run<'a> {
    state: &'a AppState,
    graph: &'a GraphState,
    repository: &'a GenerationRepository,
    planner: &'a Arc<dyn ReadingListPlanner>,
    id: GenerationId,
    request: &'a GenerateRequest,
    workspace: WorkspaceId,
    report: Report,
    spend: Spend,
}

fn database(error: &DbError) -> Error {
    Error::Literature(error.to_string())
}

fn pass_failure(error: &PlannerError) -> Error {
    Error::Task(error.to_string())
}

impl Run<'_> {
    async fn progress(&mut self, stage: &'static str) {
        self.report.stage = stage;
        let report = self.report.to_json();
        if let Err(error) = self
            .repository
            .set_progress(self.id, &report, self.spend.spent)
            .await
        {
            tracing::warn!(%error, "could not record generation progress");
        }
    }

    async fn run(&mut self) -> Result<RunEnd, Error> {
        self.report.budget.budget_usd = self.request.budget_usd;
        self.spend.budget = self.request.budget_usd;
        let target = self.load_target().await?;
        self.progress("context").await;
        let context = self.gather_context().await?;

        self.progress("plan").await;
        let reply = match self.planner.plan(&context.plan).await {
            Ok(reply) => reply,
            Err(PlannerError::BudgetExceeded(_)) => {
                self.report.budget.stopped_before = Some("plan");
                return Ok(RunEnd {
                    state: GenerationState::OverBudget,
                    list_id: None,
                });
            }
            Err(error) => return Err(pass_failure(&error)),
        };
        self.spend.record("plan", reply.cost_usd, &mut self.report);
        let plan = check_plan(reply.draft, &context.supplied, &mut self.report);
        self.report.assumed_known = plan
            .assumed_known
            .iter()
            .map(|known| known.to_report(&context.profile_titles))
            .collect();

        self.progress("verify").await;
        let items = self.verify(&plan).await?;
        let listed = target.as_ref().map(|target| target.works.clone());
        let room = Room {
            size: self.request.size,
            capacity: target.as_ref().map_or(LIST_WORKS_MAX, |target| {
                LIST_WORKS_MAX.saturating_sub(target.items)
            }),
        };
        let mut items = classify(items, &listed.unwrap_or_default(), room, &mut self.report);
        self.progress("expand").await;
        self.expand(&items).await;

        let mut over_budget = false;
        let fill = self
            .fill(
                &plan,
                &items,
                &context,
                target.as_ref(),
                room.capacity.saturating_sub(items.len()),
                &mut over_budget,
            )
            .await?;
        items.extend(fill.0);

        self.progress("admit").await;
        let list_id = self.admit(&plan, items, &fill.1, target.as_ref()).await?;
        self.progress("done").await;
        let state = if over_budget {
            GenerationState::OverBudget
        } else {
            GenerationState::Succeeded
        };
        Ok(RunEnd { state, list_id })
    }

    async fn load_target(&self) -> Result<Option<Target>, Error> {
        let Some(list) = self.request.into_list else {
            return Ok(None);
        };
        let nodes = self
            .graph
            .lists
            .nodes(list)
            .await
            .map_err(|error| database(&error))?;
        let mut target = Target {
            id: list,
            works: HashSet::new(),
            topics: HashMap::new(),
            items: 0,
            topic_total: 0,
            edge_total: self
                .graph
                .lists
                .edges(list)
                .await
                .map_err(|error| database(&error))?
                .len(),
        };
        for node in nodes {
            match node.kind {
                NodeKind::Topic => {
                    target.topic_total += 1;
                    if let Some(label) = &node.label {
                        target.topics.insert(label.to_lowercase(), node.id);
                    }
                }
                NodeKind::Work => {
                    target.items += 1;
                    target.works.extend(node.work_id);
                }
                NodeKind::Claim => target.items += 1,
            }
        }
        Ok(Some(target))
    }

    // -- context ---------------------------------------------------------------------

    async fn gather_context(&mut self) -> Result<Context, Error> {
        let anchors = self
            .graph
            .works
            .get_many(&self.request.anchors)
            .await
            .map_err(|error| database(&error))?;
        let anchor_briefs: Vec<WorkBrief> = anchors.iter().filter_map(brief).collect();
        let mut seen: HashSet<WorkId> = self.request.anchors.iter().copied().collect();
        let mut graph_context: Vec<WorkBrief> = Vec::new();

        let keywords = goal_keywords(&self.request.goal);
        let hits = self
            .repository
            .works_matching_title(&keywords, GRAPH_TITLE_HITS_MAX)
            .await
            .map_err(|error| database(&error))?;
        let hit_ids: Vec<WorkId> = hits.into_iter().filter(|id| seen.insert(*id)).collect();
        for work in self
            .graph
            .works
            .get_many(&hit_ids)
            .await
            .map_err(|error| database(&error))?
        {
            graph_context.extend(brief(&work));
        }

        let mut neighbors: Vec<Work> = Vec::new();
        for anchor in &self.request.anchors {
            neighbors.extend(
                self.graph
                    .citations
                    .outgoing(*anchor)
                    .await
                    .map_err(|error| database(&error))?,
            );
            neighbors.extend(
                self.graph
                    .citations
                    .incoming(*anchor)
                    .await
                    .map_err(|error| database(&error))?,
            );
        }
        let mut added = 0;
        for work in neighbors {
            if added >= ANCHOR_NEIGHBORS_MAX {
                break;
            }
            if !seen.insert(work.id) {
                continue;
            }
            if let Some(found) = brief(&work) {
                graph_context.push(found);
                added += 1;
            }
        }

        let history = HistoryService::connect(self.graph)
            .await
            .map_err(Error::from)?;
        let profile = history
            .knowledge_profile(Utc::now())
            .await
            .map_err(Error::from)?;
        let profile = profile_briefs(&profile);
        let existing_lists = self.existing_lists().await?;

        self.report.context = super::report::ContextReport {
            anchors: anchor_briefs.len(),
            graph_context: graph_context.len(),
            profile: profile.len(),
            existing_lists: existing_lists.len(),
        };
        let supplied = Supplied {
            context: anchor_briefs
                .iter()
                .chain(&graph_context)
                .map(|work| work.work_id)
                .collect(),
            profile: profile.iter().map(|work| work.work_id).collect(),
        };
        let profile_titles = profile
            .iter()
            .map(|work| (work.work_id, work.title.clone()))
            .collect();
        Ok(Context {
            plan: PlanRequest {
                goal: self.request.goal.trim().to_owned(),
                size: self.request.size,
                level: self.request.level,
                anchors: anchor_briefs,
                graph_context,
                profile,
                existing_lists,
            },
            supplied,
            profile_titles,
        })
    }

    async fn existing_lists(&self) -> Result<Vec<ExistingList>, Error> {
        let summaries = self
            .graph
            .lists
            .list(self.workspace, None, ArchiveScope::Active)
            .await
            .map_err(|error| database(&error))?;
        let mut lists = Vec::new();
        for summary in summaries.into_iter().take(EXISTING_LISTS_MAX) {
            let nodes = self
                .graph
                .lists
                .nodes(summary.list.id)
                .await
                .map_err(|error| database(&error))?;
            lists.push(ExistingList {
                title: summary.list.title,
                topics: nodes
                    .into_iter()
                    .filter_map(|node| node.label)
                    .take(EXISTING_TOPICS_MAX)
                    .collect(),
            });
        }
        Ok(lists)
    }

    // -- verify ----------------------------------------------------------------------

    async fn verify(&self, plan: &CheckedPlan) -> Result<Vec<Item>, Error> {
        let context = VerifyContext {
            works: &self.graph.works,
            claims: &self.graph.claims,
            records: &self.graph.source_records,
            client: self.state.literature.client(),
        };
        let mut items = Vec::new();
        for topic in 0..plan.topics.len() {
            for checked in plan.claims.iter().filter(|checked| checked.topic == topic) {
                let (body, fate) = match verify_claim(&context, &checked.claim).await {
                    Ok(ClaimResolution::Work { work, verified, .. }) => (
                        Body::Work(work.id),
                        if verified {
                            Fate::Verified
                        } else {
                            Fate::Matched
                        },
                    ),
                    Ok(ClaimResolution::Unresolved {
                        state, candidates, ..
                    }) => (
                        Body::Claim {
                            claim: checked.claim.clone(),
                            state,
                            candidates,
                        },
                        match state {
                            ClaimState::Unverified => Fate::Unverified,
                            ClaimState::Ambiguous => Fate::Ambiguous,
                            ClaimState::Contradicted => Fate::Contradicted,
                            ClaimState::Pending => Fate::Pending,
                        },
                    ),
                    Err(error) => {
                        tracing::warn!(%error, title = %checked.claim.title, "claim verification failed");
                        (
                            Body::Claim {
                                claim: checked.claim.clone(),
                                state: ClaimState::Pending,
                                candidates: Vec::new(),
                            },
                            Fate::Pending,
                        )
                    }
                };
                items.push(Item {
                    body,
                    fate,
                    topic: Some(topic),
                    role: Some(checked.role),
                    rationale: checked.rationale.clone(),
                    links: checked.links.clone(),
                });
            }
            for picked in plan
                .graph_works
                .iter()
                .filter(|picked| picked.topic == topic)
            {
                items.push(Item {
                    body: Body::Work(picked.work_id),
                    fate: Fate::InGraph,
                    topic: Some(topic),
                    role: None,
                    rationale: picked.rationale.clone(),
                    links: picked.links.clone(),
                });
            }
        }
        Ok(items)
    }

    // -- expand ----------------------------------------------------------------------

    /// Make the fanout of verified Works known, at most [`EXPANSIONS_MAX`] expansions.
    async fn expand(&mut self, items: &[Item]) {
        for item in items {
            let (Body::Work(work), Some(role)) = (&item.body, item.role) else {
                continue;
            };
            for direction in expansion_directions(role) {
                let counts = &mut self.report.expansions;
                if counts.run + counts.failed >= EXPANSIONS_MAX {
                    counts.skipped_cap += 1;
                    continue;
                }
                let cached = self
                    .state
                    .expansion_is_cached_directed(*work, *direction)
                    .await
                    .unwrap_or(false);
                if cached {
                    self.report.expansions.cached += 1;
                    continue;
                }
                match self
                    .state
                    .expand_work_directed(*work, *direction, false, true)
                    .await
                {
                    Ok(_) => self.report.expansions.run += 1,
                    Err(error) => {
                        tracing::warn!(%error, %work, "expansion failed during generation");
                        self.report.expansions.failed += 1;
                    }
                }
            }
        }
    }

    // -- fill ------------------------------------------------------------------------

    /// The closed-world fill pass. Returns the picked Works as items, and the
    /// prerequisite edges between Works.
    async fn fill(
        &mut self,
        plan: &CheckedPlan,
        items: &[Item],
        context: &Context,
        target: Option<&Target>,
        capacity_left: usize,
        over_budget: &mut bool,
    ) -> Result<(Vec<Item>, Vec<(WorkId, WorkId)>), Error> {
        let mut per_topic = vec![0_usize; plan.topics.len()];
        let mut members: Vec<WorkId> = Vec::new();
        for item in items {
            if let Body::Work(work) = &item.body {
                members.push(*work);
                if let Some(topic) = item.topic {
                    per_topic[topic] += 1;
                }
            }
        }
        let deficit: usize = per_topic
            .iter()
            .map(|count| TOPIC_WORKS_MIN.saturating_sub(*count))
            .sum();
        let remaining = self.request.size.saturating_sub(members.len());
        // Picks may not outgrow what the list can still hold.
        let slots = remaining.max(deficit).min(capacity_left);
        if slots == 0 || plan.topics.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        if self.spend.exhausted() {
            self.report.budget.stopped_before = Some("fill");
            *over_budget = true;
            return Ok((Vec::new(), Vec::new()));
        }
        self.progress("fill").await;
        let listed = target.map(|target| &target.works);
        let candidates = self.neighbors(&members, listed).await?;
        if candidates.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let offered: HashSet<WorkId> = candidates.iter().map(|work| work.work_id).collect();
        let topics: Vec<String> = plan
            .topics
            .iter()
            .map(|topic| topic.label.as_str().to_owned())
            .collect();
        let member_briefs = self.briefs(&members, FILL_MEMBERS_MAX).await?;
        let request = FillRequest {
            goal: context.plan.goal.clone(),
            remaining: slots,
            topics: topics
                .iter()
                .zip(&per_topic)
                .map(|(label, verified)| FillTopic {
                    label: label.clone(),
                    verified: *verified,
                })
                .collect(),
            members: member_briefs,
            candidates,
            profile: context.plan.profile.clone(),
        };
        self.report.fill.ran = true;
        self.report.fill.offered = offered.len();
        let reply = match self.planner.fill(&request).await {
            Ok(reply) => reply,
            Err(PlannerError::BudgetExceeded(_)) => {
                self.report.budget.stopped_before = Some("fill");
                *over_budget = true;
                return Ok((Vec::new(), Vec::new()));
            }
            Err(error) => {
                // The plan pass is already paid for; admit its items without a fill.
                tracing::warn!(%error, "fill pass failed; admitting the plan alone");
                self.report.fill.error = Some(error.to_string());
                return Ok((Vec::new(), Vec::new()));
            }
        };
        self.spend.record("fill", reply.cost_usd, &mut self.report);
        let member_set: HashSet<WorkId> = members.iter().copied().collect();
        let checked = check_fill(
            reply.draft,
            &FillBounds {
                offered: &offered,
                members: &member_set,
                topics: &topics,
                profile: &context.supplied.profile,
                remaining: slots,
            },
            &mut self.report,
        );
        let picked = checked
            .picks
            .into_iter()
            .map(|pick| Item {
                body: Body::Work(pick.work_id),
                fate: Fate::InGraph,
                topic: pick.topic,
                role: None,
                rationale: pick.rationale,
                links: pick.links,
            })
            .collect();
        Ok((picked, checked.prerequisites))
    }

    async fn briefs(&self, works: &[WorkId], limit: usize) -> Result<Vec<WorkBrief>, Error> {
        let ids: Vec<WorkId> = works.iter().copied().take(limit).collect();
        Ok(self
            .graph
            .works
            .get_many(&ids)
            .await
            .map_err(|error| database(&error))?
            .iter()
            .filter_map(brief)
            .collect())
    }

    /// Works citing or cited by the members, ranked by prominence, that the list does not
    /// hold: at most [`FILL_CANDIDATES_MAX`].
    async fn neighbors(
        &self,
        members: &[WorkId],
        listed: Option<&HashSet<WorkId>>,
    ) -> Result<Vec<WorkBrief>, Error> {
        let held: HashSet<WorkId> = members.iter().copied().collect();
        let mut links: HashMap<WorkId, (usize, Work)> = HashMap::new();
        for member in members {
            let outgoing = self
                .graph
                .citations
                .outgoing(*member)
                .await
                .map_err(|error| database(&error))?;
            let incoming = self
                .graph
                .citations
                .incoming(*member)
                .await
                .map_err(|error| database(&error))?;
            for work in outgoing.into_iter().chain(incoming) {
                if held.contains(&work.id) || listed.is_some_and(|set| set.contains(&work.id)) {
                    continue;
                }
                let entry = links.entry(work.id).or_insert_with(|| (0, work));
                entry.0 += 1;
            }
        }
        let ids: Vec<WorkId> = links.keys().copied().collect();
        let scores = self
            .graph
            .prominence
            .prominence_for(self.workspace, &ids, Utc::now().year())
            .await
            .map_err(|error| database(&error))?;
        let mut ranked: Vec<(f64, usize, WorkBrief)> = links
            .into_values()
            .filter_map(|(count, work)| {
                let score = scores.get(&work.id).map_or(0.0, |row| row.score);
                Some((score, count, brief(&work)?))
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.work_id.cmp(&b.2.work_id))
        });
        Ok(ranked
            .into_iter()
            .take(FILL_CANDIDATES_MAX)
            .map(|(_, _, brief)| brief)
            .collect())
    }

    // -- admit -----------------------------------------------------------------------

    /// Write the nodes and edges in one transaction, then order a new list (D9).
    #[allow(clippy::too_many_lines)] // one assembly, read top to bottom
    async fn admit(
        &mut self,
        plan: &CheckedPlan,
        items: Vec<Item>,
        prerequisites: &[(WorkId, WorkId)],
        target: Option<&Target>,
    ) -> Result<Option<ListId>, Error> {
        let proposing = target.is_some();
        let review = if proposing {
            ListReview::Proposed
        } else {
            ListReview::Accepted
        };
        let mut nodes: Vec<NewNode> = Vec::new();
        let mut topic_refs: Vec<Option<NodeRef>> = Vec::new();
        let topic_room =
            LIST_TOPICS_MAX.saturating_sub(target.map_or(0, |target| target.topic_total));
        let mut new_topic_count = 0;
        for topic in &plan.topics {
            let existing =
                target.and_then(|target| target.topics.get(&topic.label.as_str().to_lowercase()));
            if let Some(existing) = existing {
                topic_refs.push(Some(NodeRef::Existing(*existing)));
                self.report.topics.reused += 1;
                continue;
            }
            if new_topic_count >= topic_room {
                topic_refs.push(None);
                self.report.topics.dropped_full += 1;
                continue;
            }
            new_topic_count += 1;
            let mut node = NewNode::topic(topic.label.clone());
            node.content = NewNodeContent::Topic {
                label: topic.label.clone(),
                summary: topic.summary.clone(),
            };
            self.stamp(&mut node, review);
            topic_refs.push(Some(NodeRef::New(nodes.len())));
            nodes.push(node);
        }
        let new_topics = nodes.len();
        let mut drafted: Vec<bool> = Vec::new();
        let mut edges: Vec<AdmissionEdge> = Vec::new();
        for edge in &plan.edges {
            let (Some(from), Some(to)) = (topic_refs[edge.from], topic_refs[edge.to]) else {
                self.report.edges.dropped_unknown += 1;
                continue;
            };
            edges.push(AdmissionEdge {
                from,
                to,
                kind: edge.kind,
            });
            drafted.push(true);
        }
        let mut work_refs: HashMap<WorkId, NodeRef> = HashMap::new();
        let mut covered = vec![false; plan.topics.len()];
        let (mut works, mut claims) = (0, 0);
        for item in items {
            let mut node = match &item.body {
                Body::Work(work) => {
                    works += 1;
                    NewNode::work(*work)
                }
                Body::Claim {
                    claim,
                    state,
                    candidates,
                } => {
                    claims += 1;
                    let json = serde_json::to_value(claim)
                        .unwrap_or_else(|_| json!({ "title": claim.title }));
                    let mut node = NewNode::claim(json.clone(), *state);
                    node.content = NewNodeContent::Claim {
                        claim: json,
                        state: *state,
                        candidates: serde_json::to_value(candidates)
                            .unwrap_or_else(|_| Value::Array(Vec::new())),
                    };
                    node
                }
            };
            node.rationale = Some(item.rationale.clone()).filter(|text| !text.is_empty());
            node.context_links = links_json(&item.links);
            self.stamp(&mut node, review);
            let reference = NodeRef::New(nodes.len());
            nodes.push(node);
            if let Body::Work(work) = &item.body {
                work_refs.insert(*work, reference);
                if let Some(topic) = item.topic
                    && let Some(topic_ref) = topic_refs[topic]
                {
                    covered[topic] = true;
                    edges.push(AdmissionEdge {
                        from: topic_ref,
                        to: reference,
                        kind: EdgeKind::Covers,
                    });
                    drafted.push(false);
                }
            }
        }
        for (from, to) in prerequisites {
            match (work_refs.get(from), work_refs.get(to)) {
                (Some(from), Some(to)) => {
                    edges.push(AdmissionEdge {
                        from: *from,
                        to: *to,
                        kind: EdgeKind::Prerequisite,
                    });
                    drafted.push(true);
                }
                _ => self.report.edges.dropped_unknown += 1,
            }
        }
        self.report.topics.admitted = new_topics;
        self.report.topics.empty = covered
            .iter()
            .zip(&topic_refs)
            .filter(|(covered, reference)| !**covered && matches!(reference, Some(NodeRef::New(_))))
            .count();
        if nodes.is_empty() {
            return Ok(target.map(|target| target.id));
        }
        // The list's edge limit: keep the earliest edges, which are the plan's own.
        let edge_room = LIST_EDGES_MAX.saturating_sub(target.map_or(0, |target| target.edge_total));
        if edges.len() > edge_room {
            for was_drafted in drafted.iter().skip(edge_room) {
                if *was_drafted {
                    self.report.edges.dropped_invalid += 1;
                } else {
                    self.report.edges.covers_dropped += 1;
                }
            }
            edges.truncate(edge_room);
            drafted.truncate(edge_room);
        }

        let created = if target.is_none() {
            Some(self.create_list().await?)
        } else {
            None
        };
        let list = target
            .map(|target| target.id)
            .or(created)
            .ok_or_else(|| Error::Task("generation has no list to write to".to_owned()))?;
        let admission = Admission {
            nodes,
            edges,
            edge_review: review,
        };
        let edge_kinds: Vec<EdgeKind> = admission.edges.iter().map(|edge| edge.kind).collect();
        let admitted = match self.repository.admit(list, admission).await {
            Ok(admitted) => admitted,
            Err(error) => {
                if let Some(created) = created
                    && let Err(cleanup) = self.graph.lists.delete(self.workspace, created).await
                {
                    tracing::warn!(%cleanup, "could not remove a list left empty by a failed generation");
                }
                return Err(database(&error));
            }
        };
        let mut skipped_drafted = 0;
        for skipped in &admitted.edges_skipped {
            let covers = edge_kinds[skipped.index] == EdgeKind::Covers;
            if covers {
                self.report.edges.covers_dropped += 1;
                continue;
            }
            skipped_drafted += 1;
            match skipped.rejection {
                lysilogy_core::EdgeRejection::Cycle(_) => self.report.edges.dropped_cycle += 1,
                lysilogy_core::EdgeRejection::UnknownNode => {
                    self.report.edges.dropped_unknown += 1;
                }
                lysilogy_core::EdgeRejection::InvalidKind => {
                    self.report.edges.dropped_invalid += 1;
                }
            }
        }
        let drafted_total = drafted.iter().filter(|drafted| **drafted).count();
        self.report.edges.added = drafted_total.saturating_sub(skipped_drafted);
        self.report.admitted.works = works;
        self.report.admitted.claims = claims;
        self.report.admitted.topics = new_topics;
        self.report.admitted.edges = admitted.edges_added;
        if created.is_some()
            && let Err(error) = self
                .state
                .order_list_by(list, OrderStrategy::Topological)
                .await
        {
            tracing::warn!(%error, "could not order a generated list");
        }
        Ok(Some(list))
    }

    const fn stamp(&self, node: &mut NewNode, review: ListReview) {
        node.source = NodeSource::Agent;
        node.review = review;
        node.generation_id = Some(self.id);
    }

    async fn create_list(&self) -> Result<ListId, Error> {
        let mut new = NewList::manual(title_from_goal(&self.request.goal));
        new.origin = ListOrigin::Generated;
        new.goal = Some(self.request.goal.trim().to_owned());
        Ok(self
            .graph
            .lists
            .create(self.workspace, new)
            .await
            .map_err(|error| database(&error))?
            .id)
    }
}

// ---------------------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------------------

fn graph_failure(error: &DbError) -> GenerationError {
    GenerationError::Graph(database(error))
}

impl AppState {
    /// Use `planner` for generation instead of the `OpenRouter` default (for tests).
    #[must_use]
    pub fn with_planner(mut self, planner: Arc<dyn ReadingListPlanner>) -> Self {
        self.planner = Some(planner);
        self
    }

    fn planner_for(&self, budget_usd: f64) -> Result<Arc<dyn ReadingListPlanner>, GenerationError> {
        if let Some(planner) = &self.planner {
            return Ok(Arc::clone(planner));
        }
        let planner = OpenRouterPlanner::for_budget(budget_usd).map_err(GenerationError::Graph)?;
        Ok(Arc::new(planner))
    }

    fn generation_graph(&self) -> Result<&GraphState, GenerationError> {
        self.graph.as_ref().ok_or_else(|| {
            GenerationError::Graph(Error::InvalidRequest(
                "generation needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            ))
        })
    }

    fn generation_repository(&self) -> Result<GenerationRepository, GenerationError> {
        let graph = self.generation_graph()?;
        Ok(GenerationRepository::new(graph.works.pool().clone()))
    }

    /// The history service over the graph, for the `history` commands.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn history_service(&self) -> Result<HistoryService, Error> {
        let graph = self.generation_graph()?;
        HistoryService::connect(graph).await.map_err(Error::from)
    }

    /// A run by ID.
    ///
    /// # Errors
    /// `NotFound("generation_not_found")`; `Graph` for database failures.
    pub async fn get_generation(
        &self,
        id: GenerationId,
    ) -> Result<GenerationView, GenerationError> {
        let repository = self.generation_repository()?;
        repository
            .get(id)
            .await
            .map_err(|error| graph_failure(&error))?
            .map(GenerationView::from)
            .ok_or(GenerationError::NotFound("generation_not_found"))
    }

    /// Check the request, resolve its Works, claim the goal, and record the run.
    async fn prepare_generation(
        &self,
        mut request: GenerateRequest,
    ) -> Result<(Prepared, Arc<dyn ReadingListPlanner>), GenerationError> {
        request.validate()?;
        let graph = self.generation_graph()?;
        if request.anchors.len() > ANCHORS_MAX {
            return Err(GenerationError::Invalid(
                "invalid_anchors",
                format!("A request names at most {ANCHORS_MAX} anchors."),
            ));
        }
        let mut anchors: Vec<WorkId> = Vec::new();
        for anchor in &request.anchors {
            let resolved = graph
                .works
                .resolve_redirect(*anchor)
                .await
                .map_err(|error| graph_failure(&error))?;
            let exists = graph
                .works
                .get(resolved)
                .await
                .map_err(|error| graph_failure(&error))?
                .is_some();
            if !exists {
                return Err(GenerationError::NotFound("work_not_found"));
            }
            if !anchors.contains(&resolved) {
                anchors.push(resolved);
            }
        }
        request.anchors = anchors;
        if let Some(list) = request.into_list {
            let workspace = graph
                .documents
                .ensure_default_workspace()
                .await
                .map_err(|error| graph_failure(&error))?;
            let found = graph
                .lists
                .get(workspace, list)
                .await
                .map_err(|error| graph_failure(&error))?;
            if found.is_none() {
                return Err(GenerationError::NotFound("list_not_found"));
            }
        }
        let planner = self.planner_for(request.budget_usd)?;
        let repository = self.generation_repository()?;
        let id = GenerationId::new();
        let goal_key = request.goal_key();
        if let Err(existing) = self.generation_jobs.claim(&goal_key, id).await {
            return Err(GenerationError::Conflict(
                "generation_running",
                "A generation for this goal is already running.".to_owned(),
                Some(existing),
            ));
        }
        let options = json!({
            "size": request.size,
            "level": request.level.as_str(),
            "anchors": request.anchors,
            "into_list": request.into_list,
            "budget_usd": request.budget_usd,
            "model": planner.model_key(),
            "funding": planner.funding(),
        });
        if let Err(error) = repository
            .create(
                id,
                request.goal.trim(),
                &options,
                request.into_list,
                planner.model_key(),
            )
            .await
        {
            self.generation_jobs.release(&goal_key).await;
            return Err(graph_failure(&error));
        }
        Ok((
            Prepared {
                id,
                request,
                goal_key,
            },
            planner,
        ))
    }

    /// Close the row of a run whose task died, and free its goal.
    async fn abandon_generation(&self, id: GenerationId, goal_key: &str, why: &str) {
        self.generation_jobs.release(goal_key).await;
        let Ok(repository) = self.generation_repository() else {
            return;
        };
        let error = format!("the job stopped: {why}");
        if let Err(error) = repository
            .finish(
                id,
                GenerationState::Failed,
                &Report::default().to_json(),
                0.0,
                None,
                Some(&error),
            )
            .await
        {
            tracing::error!(%error, "could not close an abandoned generation");
        }
    }

    /// Run a prepared generation to its end and close its row.
    async fn execute_generation(
        &self,
        prepared: Prepared,
        planner: Arc<dyn ReadingListPlanner>,
    ) -> Result<GenerationView, GenerationError> {
        let turn = self.generation_jobs.turn().await;
        let outcome = self.drive_generation(&prepared, &planner).await;
        drop(turn);
        self.generation_jobs.release(&prepared.goal_key).await;
        outcome?;
        self.get_generation(prepared.id).await
    }

    async fn drive_generation(
        &self,
        prepared: &Prepared,
        planner: &Arc<dyn ReadingListPlanner>,
    ) -> Result<(), GenerationError> {
        let graph = self.generation_graph()?;
        let repository = self.generation_repository()?;
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .map_err(|error| graph_failure(&error))?;
        let mut run = Run {
            state: self,
            graph,
            repository: &repository,
            planner,
            id: prepared.id,
            request: &prepared.request,
            workspace,
            report: Report::default(),
            spend: Spend::default(),
        };
        let result = run.run().await;
        run.report.budget.spent_usd = run.spend.spent;
        let report = run.report.to_json();
        let spent = run.spend.spent;
        let (state, list_id, error) = match result {
            Ok(end) => {
                let nothing = run.report.admitted == super::report::AdmittedCounts::default()
                    && end.state == GenerationState::Succeeded;
                if nothing {
                    (
                        GenerationState::Failed,
                        end.list_id,
                        Some("The plan produced nothing that could be added.".to_owned()),
                    )
                } else {
                    (end.state, end.list_id, None)
                }
            }
            Err(error) => {
                tracing::error!(%error, id = %prepared.id, "generation failed");
                (GenerationState::Failed, None, Some(error.to_string()))
            }
        };
        repository
            .finish(
                prepared.id,
                state,
                &report,
                spent,
                list_id,
                error.as_deref(),
            )
            .await
            .map_err(|error| graph_failure(&error))
    }

    /// Start a generation in the background and return its ID; poll
    /// [`Self::get_generation`] for the outcome.
    ///
    /// # Errors
    /// `Invalid` for a bad request, `NotFound` for an unknown anchor or list,
    /// `Conflict("generation_running")` when the goal already runs, `Graph` otherwise.
    pub async fn start_generation(
        &self,
        request: GenerateRequest,
    ) -> Result<GenerationId, GenerationError> {
        let (prepared, planner) = self.prepare_generation(request).await?;
        let id = prepared.id;
        let state = self.clone();
        tokio::spawn(async move {
            let goal_key = prepared.goal_key.clone();
            let worker = {
                let state = state.clone();
                tokio::spawn(async move { state.execute_generation(prepared, planner).await })
            };
            match worker.await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => tracing::error!(%error, "generation could not be closed"),
                Err(stopped) => {
                    state
                        .abandon_generation(id, &goal_key, &stopped.to_string())
                        .await;
                }
            }
        });
        Ok(id)
    }

    /// Run a generation to its end in this task and return the finished run.
    ///
    /// # Errors
    /// As [`Self::start_generation`].
    pub async fn generate_list(
        &self,
        request: GenerateRequest,
    ) -> Result<GenerationView, GenerationError> {
        let (prepared, planner) = self.prepare_generation(request).await?;
        self.execute_generation(prepared, planner).await
    }
}

#[cfg(test)]
mod tests {
    use lysilogy_core::ReadingStatus;

    use super::*;
    use crate::reading_list_planner::report::Candidates;

    fn work_item(fate: Fate) -> Item {
        Item {
            body: Body::Work(WorkId::new()),
            fate,
            topic: None,
            role: Some(Role::Core),
            rationale: String::new(),
            links: Vec::new(),
        }
    }

    fn claim_item(fate: Fate) -> Item {
        Item {
            body: Body::Claim {
                claim: Claim {
                    title: "Unknown".to_owned(),
                    ..Claim::default()
                },
                state: ClaimState::Unverified,
                candidates: Vec::new(),
            },
            fate,
            topic: None,
            role: Some(Role::Core),
            rationale: String::new(),
            links: Vec::new(),
        }
    }

    #[test]
    fn should_pick_subject_words_when_extracting_goal_keywords() {
        assert_eq!(
            goal_keywords("Learn RLHF reward modelling, and the basics of RLHF!"),
            vec!["rlhf", "reward", "modelling"]
        );
    }

    #[test]
    fn should_return_nothing_when_the_goal_has_only_filler_or_symbols() {
        assert!(goal_keywords("learn the basics").is_empty());
        assert!(goal_keywords("!!! ?? 42").is_empty());
        assert!(goal_keywords("").is_empty());
    }

    #[test]
    fn should_cap_keywords_and_fold_unicode_when_extracting_them() {
        let goal = "alpha bravo charlie delta echo foxtrot golf hotel";
        assert_eq!(goal_keywords(goal).len(), KEYWORDS_MAX);
        assert_eq!(
            goal_keywords("Théorie Über"),
            vec![title_key("Théorie"), title_key("Über")]
        );
    }

    #[test]
    fn should_title_a_list_from_a_long_goal_when_it_exceeds_the_limit() {
        let title = title_from_goal(&"word ".repeat(100));
        assert!(title.as_str().chars().count() <= 120);
        assert_eq!(title_from_goal("   ").as_str(), "Reading list");
        assert_eq!(title_from_goal("learn\u{0}  RLHF").as_str(), "learn RLHF");
    }

    #[test]
    fn should_stop_before_a_pass_that_would_exceed_the_budget_when_checking_spend() {
        let mut report = Report::default();
        let mut spend = Spend {
            budget: 0.10,
            ..Spend::default()
        };
        assert!(!spend.exhausted(), "nothing spent yet");
        spend.record("plan", 0.06, &mut report);
        assert!(spend.exhausted(), "0.06 + another 0.06 would pass 0.10");
        let mut cheap = Spend {
            budget: 0.10,
            ..Spend::default()
        };
        cheap.record("plan", 0.02, &mut report);
        assert!(!cheap.exhausted());
        cheap.record("fill", 0.08, &mut report);
        assert!(cheap.exhausted(), "the budget is spent");
        assert_eq!(report.passes.len(), 3);
    }

    #[test]
    fn should_ignore_a_bad_cost_when_recording_spend() {
        let mut report = Report::default();
        let mut spend = Spend {
            budget: 1.0,
            ..Spend::default()
        };
        spend.record("plan", f64::NAN, &mut report);
        spend.record("fill", -3.0, &mut report);
        assert!(spend.spent.abs() < f64::EPSILON);
    }

    #[test]
    fn should_count_every_candidate_once_when_classifying() {
        let shared = WorkId::new();
        let listed = WorkId::new();
        let mut items = vec![
            work_item(Fate::Verified),
            claim_item(Fate::Unverified),
            work_item(Fate::Matched),
            work_item(Fate::InGraph),
            work_item(Fate::Verified),
        ];
        items[0].body = Body::Work(shared);
        items[2].body = Body::Work(shared);
        items[3].body = Body::Work(listed);
        let mut report = Report::default();
        report.candidates.seen = items.len();
        let kept = classify(
            items,
            &HashSet::from([listed]),
            Room {
                size: 1,
                capacity: 10,
            },
            &mut report,
        );
        let Candidates {
            duplicate,
            already_in_list,
            over_size,
            verified,
            unverified,
            ..
        } = report.candidates.clone();
        assert_eq!(duplicate, 1);
        assert_eq!(already_in_list, 1);
        assert_eq!(over_size, 1);
        assert_eq!(verified, 1);
        assert_eq!(unverified, 1, "claims ride along past the size");
        assert_eq!(kept.len(), 2);
        assert_eq!(report.candidates.accounted(), report.candidates.seen);
    }

    #[test]
    fn should_cap_claims_and_count_them_when_claims_exceed_the_cap() {
        let items: Vec<Item> = (0..9).map(|_| claim_item(Fate::Unverified)).collect();
        let mut report = Report::default();
        report.candidates.seen = 9;
        let kept = classify(
            items,
            &HashSet::new(),
            Room {
                size: 4,
                capacity: 50,
            },
            &mut report,
        );
        assert_eq!(kept.len(), 5);
        assert_eq!(report.candidates.over_size, 4);
        assert_eq!(report.candidates.accounted(), 9);
    }

    #[test]
    fn should_put_works_before_claims_when_the_list_is_nearly_full() {
        let items = vec![
            claim_item(Fate::Unverified),
            claim_item(Fate::Pending),
            work_item(Fate::Verified),
        ];
        let mut report = Report::default();
        report.candidates.seen = 3;
        let kept = classify(
            items,
            &HashSet::new(),
            Room {
                size: 5,
                capacity: 2,
            },
            &mut report,
        );
        assert!(matches!(kept[0].body, Body::Work(_)));
        assert_eq!(kept.len(), 2);
        assert_eq!(report.candidates.list_full, 1);
        assert_eq!(report.candidates.accounted(), 3);
    }

    #[test]
    fn should_choose_directions_by_role_when_expanding() {
        assert_eq!(
            expansion_directions(Role::Foundation),
            &[ExpansionDirection::References]
        );
        assert_eq!(
            expansion_directions(Role::Core),
            &[ExpansionDirection::References, ExpansionDirection::CitedBy]
        );
        assert_eq!(
            expansion_directions(Role::Frontier),
            &[ExpansionDirection::CitedBy]
        );
        assert!(expansion_directions(Role::Survey).is_empty());
    }

    #[test]
    fn should_keep_the_profile_within_the_prompt_budget_when_it_is_long() {
        use crate::history::ProfileEntry;
        let entries: Vec<ProfileEntry> = (0..120)
            .map(|index| ProfileEntry {
                work_id: WorkId::new(),
                title: Some(format!("{index} {}", "t".repeat(900))),
                year: Some(2020),
                status: ReadingStatus::Read,
                dwell_ms: 600_000,
                last_active: Utc::now(),
                score: 1.0,
                lists: Vec::new(),
            })
            .chain(std::iter::once(ProfileEntry {
                work_id: WorkId::new(),
                title: None,
                year: None,
                status: ReadingStatus::Read,
                dwell_ms: 0,
                last_active: Utc::now(),
                score: 0.0,
                lists: Vec::new(),
            }))
            .collect();
        let briefs = profile_briefs(&Profile {
            entries,
            omitted: 0,
        });
        let chars: usize = briefs.iter().map(|brief| brief.title.chars().count()).sum();
        assert!(chars < PROFILE_PROMPT_CHARS_MAX, "{chars}");
        assert!(briefs.len() < 120);
        assert_eq!(briefs[0].dwell_minutes, 10);
    }
}
