//! The generation report: counts per pass and per verification state. Nothing is dropped
//! silently, so the candidate buckets sum to the candidates seen.

use lysilogy_core::WorkId;
use serde::Serialize;

/// One model pass and what it cost.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassCost {
    pub pass: &'static str,
    pub cost_usd: f64,
}

/// The budget and where the run met it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BudgetReport {
    pub budget_usd: f64,
    pub spent_usd: f64,
    /// The pass that did not run because the budget was spent.
    pub stopped_before: Option<&'static str>,
    /// A pass cost more than what was left. The run still admits its verified items, so
    /// its state stays `succeeded`; this flag marks the overspend.
    pub exceeded: bool,
}

/// What the model was shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ContextReport {
    pub anchors: usize,
    pub graph_context: usize,
    pub profile: usize,
    pub existing_lists: usize,
}

/// What became of the plan's topics.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TopicCounts {
    pub planned: usize,
    pub dropped_over_limit: usize,
    pub dropped_invalid: usize,
    pub dropped_duplicate: usize,
    /// New topic nodes written.
    pub admitted: usize,
    /// Topics that were already in the extended list.
    pub reused: usize,
    /// Topics written with no Work under them.
    pub empty: usize,
    /// Dropped because the extended list holds the most topics it may.
    pub dropped_full: usize,
}

/// Every claim and graph Work the plan named, and what became of each. The buckets sum to
/// `seen`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Candidates {
    pub seen: usize,
    /// No title, or a role outside the vocabulary.
    pub invalid: usize,
    /// Under a topic the plan could not keep.
    pub topic_dropped: usize,
    /// Past the per-topic limit.
    pub over_limit: usize,
    /// A graph Work whose ID was not in the context or profile supplied.
    pub rejected_ids: usize,
    /// A claim that matched a Work already in the graph.
    pub matched: usize,
    /// A claim a provider verified.
    pub verified: usize,
    /// A graph Work taken by ID.
    pub in_graph: usize,
    pub unverified: usize,
    pub ambiguous: usize,
    pub contradicted: usize,
    pub pending: usize,
    /// Resolved to a Work another candidate already took.
    pub duplicate: usize,
    /// Resolved to a Work the extended list already holds.
    pub already_in_list: usize,
    /// Past the requested size.
    pub over_size: usize,
    /// Past the list's capacity.
    pub list_full: usize,
}

impl Candidates {
    /// The sum of every bucket; equals `seen` when nothing was lost.
    #[must_use]
    pub const fn accounted(&self) -> usize {
        self.invalid
            + self.topic_dropped
            + self.over_limit
            + self.rejected_ids
            + self.matched
            + self.verified
            + self.in_graph
            + self.unverified
            + self.ambiguous
            + self.contradicted
            + self.pending
            + self.duplicate
            + self.already_in_list
            + self.over_size
            + self.list_full
    }
}

/// Context links the model wrote (D16).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkCounts {
    pub accepted: usize,
    /// Outside the supplied profile, an unknown relation, a repeat, or past the limit.
    pub rejected: usize,
}

/// A topic the plan skipped because the reader knows it (D16).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssumedKnownReport {
    pub topic: String,
    pub reason: String,
    pub evidence: Vec<EvidenceWork>,
}

/// A profile Work that justifies an assumption.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EvidenceWork {
    pub work_id: WorkId,
    pub title: Option<String>,
}

/// Expansions of verified Works (cited-by and references, depth one).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ExpansionCounts {
    pub run: usize,
    /// Fanout already known; nothing to do.
    pub cached: usize,
    pub failed: usize,
    /// Wanted but past the cap of ten.
    pub skipped_cap: usize,
}

/// The closed-world fill pass.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FillReport {
    pub ran: bool,
    pub offered: usize,
    pub picks_seen: usize,
    pub admitted: usize,
    /// Not among the offered Works.
    pub rejected_ids: usize,
    pub duplicate: usize,
    pub over_size: usize,
    /// Admitted with no topic because the label matched none.
    pub unassigned_topic: usize,
    /// Why the pass failed; the plan's items were admitted without it.
    pub error: Option<String>,
}

/// Edges the plan and fill pass proposed: `planned` is `added` plus every `dropped_*`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EdgeCounts {
    pub planned: usize,
    pub added: usize,
    /// Dropped because they would close a cycle (D8).
    pub dropped_cycle: usize,
    /// Dropped for a missing endpoint or a self edge.
    pub dropped_unknown: usize,
    /// Dropped because the kind does not fit the endpoints, or it repeats another.
    pub dropped_invalid: usize,
    /// Covers edges (topic to Work) the list refused; not model-proposed.
    pub covers_dropped: usize,
}

/// What was written to the list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AdmittedCounts {
    pub works: usize,
    pub claims: usize,
    pub topics: usize,
    pub edges: usize,
}

/// The whole report, stored on the generation row.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    /// The pass in progress or last finished.
    pub stage: &'static str,
    pub passes: Vec<PassCost>,
    pub budget: BudgetReport,
    pub context: ContextReport,
    pub topics: TopicCounts,
    pub candidates: Candidates,
    pub context_links: LinkCounts,
    pub assumed_known: Vec<AssumedKnownReport>,
    pub assumed_known_dropped: usize,
    pub assumed_known_evidence_rejected: usize,
    pub expansions: ExpansionCounts,
    pub fill: FillReport,
    pub edges: EdgeCounts,
    pub admitted: AdmittedCounts,
}

impl Report {
    /// The report as stored JSON.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
