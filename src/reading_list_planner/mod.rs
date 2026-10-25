//! Agent generation of reading lists.
//!
//! The pipeline is staged, with no tool loop: a plan pass (the model proposes topics and
//! cited papers), deterministic verification of every paper, a closed-world fill pass (the
//! model picks only from offered Works), and a deterministic admission. The model proposes
//! and the application admits. [`ReadingListPlanner`] is the model seam, following the
//! `RelationshipLabeler` pattern: an `OpenRouter` implementation for real runs and a recording
//! fake for tests. Every call is billed to the spend ledger, and the run stops at a pass
//! boundary once its budget is spent.

use std::future::Future;
use std::pin::Pin;

use lysilogy_core::{GenerationId, ListId, WorkId};
use serde::{Deserialize, Serialize};

use crate::Error;

pub mod jobs;
pub mod openrouter;
pub mod pipeline;
pub mod prompts;
pub mod report;
pub mod validate;

#[cfg(test)]
pub mod fake;
#[cfg(test)]
#[allow(clippy::too_many_lines)] // scenario tests read top to bottom
mod tests;

/// Strict output schema of the plan pass.
pub const PLAN_SCHEMA: &str = include_str!("../../prompts/reading-list/plan.schema.json");
/// Strict output schema of the fill pass.
pub const FILL_SCHEMA: &str = include_str!("../../prompts/reading-list/fill.schema.json");
/// Instructions of the plan pass.
pub const PLAN_PROMPT: &str = include_str!("../../prompts/reading-list/plan.md");
/// Instructions of the fill pass.
pub const FILL_PROMPT: &str = include_str!("../../prompts/reading-list/fill.md");

/// Fewest items a request may ask for.
pub const SIZE_MIN: usize = 8;
/// Most items a request may ask for.
pub const SIZE_MAX: usize = 60;
/// Items asked for when the request names no size.
pub const SIZE_DEFAULT: usize = 20;
/// The default budget of one run, in US dollars (Q4).
pub const DEFAULT_BUDGET_USD: f64 = 0.15;
/// Longest goal, in characters; the list stores it.
pub const GOAL_MAX_CHARS: usize = 2_000;
/// Most topics a plan keeps.
pub const TOPICS_MAX: usize = 12;
/// Most claims kept under one topic.
pub const CLAIMS_PER_TOPIC_MAX: usize = 8;
/// Most graph Works kept under one topic.
pub const GRAPH_WORKS_PER_TOPIC_MAX: usize = 8;
/// Most context links one item keeps (D16).
pub const CONTEXT_LINKS_PER_ITEM_MAX: usize = 3;
/// Longest context-link note, in characters (D16).
pub const CONTEXT_LINK_NOTE_MAX_CHARS: usize = 200;
/// Longest rationale, in characters.
pub const RATIONALE_MAX_CHARS: usize = 300;
/// Most candidates the fill pass is offered.
pub const FILL_CANDIDATES_MAX: usize = 60;
/// Most expansions one run starts.
pub const EXPANSIONS_MAX: usize = 10;
/// Most Works the keyword title search adds to the plan's context.
pub const GRAPH_TITLE_HITS_MAX: usize = 40;
/// Most anchor neighbors added to the plan's context.
pub const ANCHOR_NEIGHBORS_MAX: usize = 40;
/// Most characters of profile the prompt carries, about 6k tokens (D16).
pub const PROFILE_PROMPT_CHARS_MAX: usize = 24_000;
/// Fewest verified Works a topic should have before the fill pass leaves it alone.
pub const TOPIC_WORKS_MIN: usize = 2;

/// How deep the list goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Intro,
    Survey,
    Deep,
}

impl Level {
    /// The name on the wire and in the CLI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Survey => "survey",
            Self::Deep => "deep",
        }
    }
}

impl std::str::FromStr for Level {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        match text {
            "intro" => Ok(Self::Intro),
            "survey" => Ok(Self::Survey),
            "deep" => Ok(Self::Deep),
            _ => Err(()),
        }
    }
}

/// What part a paper plays in its topic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Foundation,
    Core,
    Survey,
    Frontier,
}

impl std::str::FromStr for Role {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        match text {
            "foundation" => Ok(Self::Foundation),
            "core" => Ok(Self::Core),
            "survey" => Ok(Self::Survey),
            "frontier" => Ok(Self::Frontier),
            _ => Err(()),
        }
    }
}

/// A request to build or extend a list.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerateRequest {
    pub goal: String,
    pub size: usize,
    pub level: Level,
    pub anchors: Vec<WorkId>,
    /// Extend this list instead of making one (D11: the additions are `proposed`).
    pub into_list: Option<ListId>,
    pub budget_usd: f64,
}

impl GenerateRequest {
    /// A request for `goal` with the defaults.
    #[must_use]
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            size: SIZE_DEFAULT,
            level: Level::Survey,
            anchors: Vec::new(),
            into_list: None,
            budget_usd: DEFAULT_BUDGET_USD,
        }
    }

    /// Refuse a request that cannot run.
    ///
    /// # Errors
    /// `Invalid("invalid_goal" | "invalid_size" | "invalid_budget", …)`.
    pub fn validate(&self) -> Result<(), GenerationError> {
        let goal = self.goal.trim();
        if goal.is_empty() || goal.chars().count() > GOAL_MAX_CHARS {
            return Err(GenerationError::Invalid(
                "invalid_goal",
                format!("A goal is 1 to {GOAL_MAX_CHARS} characters."),
            ));
        }
        if !(SIZE_MIN..=SIZE_MAX).contains(&self.size) {
            return Err(GenerationError::Invalid(
                "invalid_size",
                format!("Size is between {SIZE_MIN} and {SIZE_MAX}."),
            ));
        }
        if !self.budget_usd.is_finite() || self.budget_usd <= 0.0 {
            return Err(GenerationError::Invalid(
                "invalid_budget",
                "A budget is a positive number of US dollars.".to_owned(),
            ));
        }
        Ok(())
    }

    /// The goal as the one-run-per-goal key: trimmed, whitespace collapsed, lower case.
    #[must_use]
    pub fn goal_key(&self) -> String {
        self.goal
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
}

/// Why a generation could not start or be read.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    /// `generation_not_found`, `list_not_found` or `work_not_found`.
    #[error("{}", not_found_message(.0))]
    NotFound(&'static str),
    /// The request was refused: a code and a message.
    #[error("{1}")]
    Invalid(&'static str, String),
    /// `generation_running`: a run for this goal is already going.
    #[error("{1}")]
    Conflict(&'static str, String, Option<GenerationId>),
    /// The graph database is missing or a query failed.
    #[error("{0}")]
    Graph(Error),
}

fn not_found_message(code: &str) -> &'static str {
    match code {
        "generation_not_found" => "No such generation.",
        "list_not_found" => "No such list.",
        _ => "No such work.",
    }
}

impl GenerationError {
    /// The machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound(code) | Self::Invalid(code, _) | Self::Conflict(code, ..) => code,
            Self::Graph(_) => "graph_error",
        }
    }
}

impl From<GenerationError> for Error {
    fn from(error: GenerationError) -> Self {
        match error {
            GenerationError::Graph(error) => error,
            other => Self::InvalidRequest(other.to_string()),
        }
    }
}

/// A Work as the model is shown it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkBrief {
    pub work_id: WorkId,
    pub title: String,
    pub year: Option<i32>,
}

/// A Work the reader knows, as the model is shown it (D16).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileBrief {
    pub work_id: WorkId,
    pub title: String,
    pub year: Option<i32>,
    pub status: lysilogy_core::ReadingStatus,
    pub dwell_minutes: i64,
    pub lists: Vec<String>,
}

/// One of the reader's lists, as the model is shown it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExistingList {
    pub title: String,
    pub topics: Vec<String>,
}

/// What the plan pass is told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlanRequest {
    pub goal: String,
    pub size: usize,
    pub level: Level,
    pub anchors: Vec<WorkBrief>,
    pub graph_context: Vec<WorkBrief>,
    pub profile: Vec<ProfileBrief>,
    pub existing_lists: Vec<ExistingList>,
}

/// A topic and how many verified Works it has so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FillTopic {
    pub label: String,
    pub verified: usize,
}

/// What the fill pass is told. It may pick only from `candidates`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FillRequest {
    pub goal: String,
    pub remaining: usize,
    pub topics: Vec<FillTopic>,
    pub members: Vec<WorkBrief>,
    pub candidates: Vec<WorkBrief>,
    pub profile: Vec<ProfileBrief>,
}

/// A link from an item to a Work the reader has read, as the model wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextLinkDraft {
    #[serde(default)]
    pub work_id: String,
    #[serde(default)]
    pub relation: String,
    #[serde(default)]
    pub note: String,
}

/// A paper the model remembers, as it wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimDraft {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub venue: Option<String>,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub arxiv: Option<String>,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub context_links: Vec<ContextLinkDraft>,
}

/// A Work of the supplied context the model picked by ID.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphWorkDraft {
    #[serde(default)]
    pub work_id: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub context_links: Vec<ContextLinkDraft>,
}

/// One topic of the plan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicDraft {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub claims: Vec<ClaimDraft>,
    #[serde(default)]
    pub graph_works: Vec<GraphWorkDraft>,
}

/// An edge between topics, named by label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicEdgeDraft {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub kind: String,
}

/// A topic the model left out because the reader knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssumedKnownDraft {
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// The plan pass's answer, before validation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDraft {
    #[serde(default)]
    pub topics: Vec<TopicDraft>,
    #[serde(default)]
    pub topic_edges: Vec<TopicEdgeDraft>,
    #[serde(default)]
    pub assumed_known: Vec<AssumedKnownDraft>,
}

/// One Work the fill pass picked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillPickDraft {
    #[serde(default)]
    pub work_id: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub context_links: Vec<ContextLinkDraft>,
}

/// An order between two Works, by ID.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerequisiteDraft {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
}

/// The fill pass's answer, before validation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillDraft {
    #[serde(default)]
    pub picks: Vec<FillPickDraft>,
    #[serde(default)]
    pub prerequisites: Vec<PrerequisiteDraft>,
}

/// A draft and what its call cost.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannerReply<T> {
    pub draft: T,
    pub cost_usd: f64,
}

/// Why a planner produced no draft.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlannerError {
    /// The model service is not configured or not reachable.
    #[error("planner unavailable: {0}")]
    Unavailable(String),
    /// The call ran and failed.
    #[error("planner failed: {0}")]
    Failed(String),
    /// The model answered with something that is not a draft.
    #[error("planner returned a malformed answer: {0}")]
    Malformed(String),
    /// The spend ledger refused the call.
    #[error("planner stopped by the spend budget: {0}")]
    BudgetExceeded(String),
}

/// The future returned by a planner call.
pub type PlannerFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<PlannerReply<T>, PlannerError>> + Send + 'a>>;

/// A model-backed source of reading-list drafts.
pub trait ReadingListPlanner: Send + Sync + std::fmt::Debug {
    /// Identifies the model in the generation record.
    fn model_key(&self) -> &str;

    /// How the calls are paid for: `openrouter`, `plan` or `metered`.
    fn funding(&self) -> &'static str;

    /// Propose topics, papers and structure for the goal.
    fn plan<'a>(&'a self, request: &'a PlanRequest) -> PlannerFuture<'a, PlanDraft>;

    /// Pick more Works from the offered candidates.
    fn fill<'a>(&'a self, request: &'a FillRequest) -> PlannerFuture<'a, FillDraft>;
}
