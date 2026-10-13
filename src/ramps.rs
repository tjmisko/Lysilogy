//! Reading ramps: the service shared by the API and the CLI.
//!
//! A ramp is an ordered reading list rooted at one seed Work. Every method works in the
//! default workspace, resolves Work IDs through redirects, and fails with a
//! [`RampServiceError`] whose codes the API and CLI share.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Datelike, Utc};
use lysilogy_core::{
    RampError, RampId, RampItemStatus, RampOrderItem, RampTitle, Work, WorkId, WorkspaceId,
    parse_mark_note, suggest_order, title_key,
};
use lysilogy_db::DbError;
use lysilogy_db::ramp::{NewRampItem, RampItem, RampItemPatch, RampSummary, RemoveOutcome};
use serde::Serialize;

use crate::{
    AppState, Error,
    api::{AcquisitionStates, LocalPaper, LocalPapers, local_papers_for, states_for},
    expansion::ExpansionView,
    graph::GraphState,
};

/// Most matching titles an ambiguous ramp reference lists.
const AMBIGUOUS_MATCHES_SHOWN: usize = 5;

/// Why a ramp operation failed.
#[derive(Debug, thiserror::Error)]
pub enum RampServiceError {
    /// `ramp_not_found`, `item_not_found`, or `work_not_found`.
    #[error("{}", not_found_message(.0))]
    NotFound(&'static str),
    /// The input was refused: a code such as `invalid_order` and a message.
    #[error("{1}")]
    Invalid(&'static str, String),
    /// The change conflicts with the ramp: `item_exists`, `via_not_in_ramp`,
    /// `no_citation_edge`, `ramp_too_deep`, `ramp_full`, or `seed_item`.
    #[error("{}", conflict_message(.0))]
    Conflict(&'static str),
    /// The graph database is missing or a query failed.
    #[error("{0}")]
    Graph(Error),
}

fn not_found_message(code: &str) -> &'static str {
    match code {
        "ramp_not_found" => "No such ramp.",
        "item_not_found" => "No such item in the ramp.",
        _ => "No such work.",
    }
}

fn conflict_message(code: &str) -> &'static str {
    match code {
        "item_exists" => "The Work is already in the ramp.",
        "via_not_in_ramp" => "The Work it was reached from is not in the ramp.",
        "no_citation_edge" => "The Work it was reached from does not cite it.",
        "ramp_too_deep" => "The path would be too long.",
        "ramp_full" => "The ramp is full.",
        "seed_item" => "The seed item cannot be removed.",
        _ => "The change conflicts with the ramp.",
    }
}

impl RampServiceError {
    /// The machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound(code) | Self::Invalid(code, _) | Self::Conflict(code) => code,
            Self::Graph(_) => "graph_error",
        }
    }
}

impl From<RampServiceError> for Error {
    fn from(error: RampServiceError) -> Self {
        match error {
            RampServiceError::Graph(error) => error,
            other => Self::InvalidRequest(other.to_string()),
        }
    }
}

fn database(error: &DbError) -> RampServiceError {
    RampServiceError::Graph(Error::Literature(error.to_string()))
}

/// Map a repository failure: stable conflict codes and a missing ramp get their own
/// variants; anything else is a graph failure.
fn ramp_failure(error: &DbError) -> RampServiceError {
    if error.is_not_found() {
        return RampServiceError::NotFound("ramp_not_found");
    }
    if let DbError::Conflict(code) = error {
        for conflict in [
            RampError::ItemExists,
            RampError::ViaNotInRamp,
            RampError::NoCitationEdge,
            RampError::TooDeep,
            RampError::Full,
            RampError::SeedItem,
        ] {
            if conflict.code() == code {
                return RampServiceError::Conflict(conflict.code());
            }
        }
        if code == RampError::InvalidOrder.code() {
            return RampServiceError::Invalid(
                RampError::InvalidOrder.code(),
                RampError::InvalidOrder.to_string(),
            );
        }
    }
    database(error)
}

/// Validate a ramp title.
///
/// # Errors
/// `Invalid("invalid_ramp_title", …)`.
pub fn parse_title(text: &str) -> Result<RampTitle, RampServiceError> {
    RampTitle::parse(text)
        .map_err(|error| RampServiceError::Invalid(error.code(), error.to_string()))
}

/// Validate and normalize an item note: trimmed, empty becomes `None`.
///
/// # Errors
/// `Invalid("invalid_note", …)`.
pub fn parse_note(text: Option<&str>) -> Result<Option<String>, RampServiceError> {
    parse_mark_note(text)
        .map_err(|error| RampServiceError::Invalid(error.code(), error.to_string()))
}

/// Validate an item status.
///
/// # Errors
/// `Invalid("invalid_status", …)`.
pub fn parse_status(text: &str) -> Result<RampItemStatus, RampServiceError> {
    text.parse().map_err(|_| {
        RampServiceError::Invalid(
            "invalid_status",
            "Status is one of unread, reading, read.".to_owned(),
        )
    })
}

/// Items per reading status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct StatusCounts {
    pub unread: i64,
    pub reading: i64,
    pub read: i64,
}

/// A ramp as listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RampSummaryView {
    pub id: RampId,
    pub title: String,
    pub seed_work_id: WorkId,
    pub seed_title: Option<String>,
    pub item_count: i64,
    pub status_counts: StatusCounts,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<RampSummary> for RampSummaryView {
    fn from(summary: RampSummary) -> Self {
        Self {
            id: summary.ramp.id,
            title: summary.ramp.title,
            seed_work_id: summary.ramp.seed_work_id,
            seed_title: summary.seed_title,
            item_count: summary.item_count,
            status_counts: StatusCounts {
                unread: summary.unread,
                reading: summary.reading,
                read: summary.read,
            },
            created_at: summary.ramp.created_at,
            updated_at: summary.ramp.updated_at,
        }
    }
}

/// One Work on an item's path, with the title if known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PathStep {
    pub work_id: WorkId,
    pub title: Option<String>,
}

/// One ramp item as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RampItemView {
    pub work: Work,
    /// Dense position, counting from 1.
    pub ordinal: usize,
    pub depth: usize,
    pub via_work_id: Option<WorkId>,
    pub path: Vec<PathStep>,
    pub status: RampItemStatus,
    pub note: Option<String>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status_changed_at: Option<DateTime<Utc>>,
}

/// A ramp with its items and what the reader knows about them.
#[derive(Debug, Serialize)]
pub struct RampDetail {
    pub ramp: RampSummaryView,
    pub items: Vec<RampItemView>,
    pub local_papers: LocalPapers,
    pub acquisitions: AcquisitionStates,
    pub expansions: BTreeMap<String, ExpansionView>,
}

impl RampDetail {
    /// Local papers held for a Work, if any.
    #[must_use]
    pub fn local_papers_of(&self, work: WorkId) -> &[LocalPaper] {
        self.local_papers
            .get(&work.to_string())
            .map_or(&[], Vec::as_slice)
    }
}

fn graph_of(state: &AppState) -> Result<&GraphState, RampServiceError> {
    state.graph.as_ref().ok_or_else(|| {
        RampServiceError::Graph(Error::InvalidRequest(
            "ramps need a database; set LYSILOGY_DATABASE_URL".to_owned(),
        ))
    })
}

async fn workspace_of(graph: &GraphState) -> Result<WorkspaceId, RampServiceError> {
    graph
        .documents
        .ensure_default_workspace()
        .await
        .map_err(|error| database(&error))
}

/// Resolve a Work through redirects and require it to exist.
async fn existing_work(graph: &GraphState, work: WorkId) -> Result<Work, RampServiceError> {
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
        .ok_or(RampServiceError::NotFound("work_not_found"))
}

fn display_title(work: &Work) -> Option<String> {
    work.display_title.clone()
}

impl AppState {
    async fn require_ramp(
        &self,
        graph: &GraphState,
        workspace: WorkspaceId,
        ramp: RampId,
    ) -> Result<lysilogy_db::ramp::Ramp, RampServiceError> {
        let _ = self;
        graph
            .ramps
            .get(workspace, ramp)
            .await
            .map_err(|error| database(&error))?
            .ok_or(RampServiceError::NotFound("ramp_not_found"))
    }

    async fn summary_view(
        &self,
        graph: &GraphState,
        workspace: WorkspaceId,
        ramp: RampId,
    ) -> Result<RampSummaryView, RampServiceError> {
        let _ = self;
        graph
            .ramps
            .list(workspace, None)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .find(|summary| summary.ramp.id == ramp)
            .map(RampSummaryView::from)
            .ok_or(RampServiceError::NotFound("ramp_not_found"))
    }

    /// Item views in position order, with path titles from one Works lookup over the
    /// items and the union of their path IDs.
    async fn item_views(
        &self,
        graph: &GraphState,
        items: Vec<RampItem>,
    ) -> Result<Vec<RampItemView>, RampServiceError> {
        let _ = self;
        let mut wanted: HashSet<WorkId> = HashSet::new();
        for item in &items {
            wanted.insert(item.work_id);
            wanted.extend(item.path.iter().copied());
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
        let mut views = Vec::with_capacity(items.len());
        for item in items {
            let Some(work) = works.get(&item.work_id) else {
                continue;
            };
            let path = item
                .path
                .iter()
                .map(|step| PathStep {
                    work_id: *step,
                    title: works.get(step).and_then(display_title),
                })
                .collect();
            views.push(RampItemView {
                work: work.clone(),
                ordinal: views.len() + 1,
                depth: item.path.len().saturating_sub(1),
                via_work_id: item.via_work_id,
                path,
                status: item.status,
                note: item.note,
                added_at: item.added_at,
                updated_at: item.updated_at,
                status_changed_at: item.status_changed_at,
            });
        }
        Ok(views)
    }

    async fn item_view(
        &self,
        graph: &GraphState,
        ramp: RampId,
        work: WorkId,
    ) -> Result<RampItemView, RampServiceError> {
        let items = graph
            .ramps
            .items(ramp)
            .await
            .map_err(|error| ramp_failure(&error))?;
        self.item_views(graph, items)
            .await?
            .into_iter()
            .find(|view| view.work.id == work)
            .ok_or(RampServiceError::NotFound("item_not_found"))
    }

    /// Create a ramp seeded at `seed`. The default title is the seed's title.
    ///
    /// # Errors
    /// `NotFound("work_not_found")` for an unknown seed; `Graph` for database failures.
    pub async fn create_ramp(
        &self,
        seed: WorkId,
        title: Option<RampTitle>,
    ) -> Result<RampDetail, RampServiceError> {
        let graph = graph_of(self)?;
        let seed = existing_work(graph, seed).await?;
        let workspace = workspace_of(graph).await?;
        let title = title
            .unwrap_or_else(|| lysilogy_core::default_ramp_title(seed.display_title.as_deref()));
        let ramp = graph
            .ramps
            .create(workspace, seed.id, &title)
            .await
            .map_err(|error| database(&error))?;
        self.ramp_detail(ramp.id).await
    }

    /// The workspace's ramps by update time, optionally only those seeded at one Work.
    ///
    /// # Errors
    /// `Graph` for database failures.
    pub async fn list_ramps(
        &self,
        seed: Option<WorkId>,
    ) -> Result<Vec<RampSummaryView>, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let seed = match seed {
            Some(seed) => Some(
                graph
                    .works
                    .resolve_redirect(seed)
                    .await
                    .map_err(|error| database(&error))?,
            ),
            None => None,
        };
        Ok(graph
            .ramps
            .list(workspace, seed)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .map(RampSummaryView::from)
            .collect())
    }

    /// A ramp with its items, path titles, local papers, acquisition states, and
    /// expansion states.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found")`; `Graph` for database failures.
    pub async fn ramp_detail(&self, ramp: RampId) -> Result<RampDetail, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        self.require_ramp(graph, workspace, ramp).await?;
        let summary = self.summary_view(graph, workspace, ramp).await?;
        let items = graph
            .ramps
            .items(ramp)
            .await
            .map_err(|error| ramp_failure(&error))?;
        let work_ids: Vec<WorkId> = items.iter().map(|item| item.work_id).collect();
        let items = self.item_views(graph, items).await?;
        let graph_failure = |_response| {
            RampServiceError::Graph(Error::Literature("the graph query failed".to_owned()))
        };
        let local_papers = local_papers_for(self, &work_ids)
            .await
            .map_err(graph_failure)?;
        let acquisitions = states_for(self, &work_ids).await.map_err(graph_failure)?;
        let expansions = self
            .expansion_views(&work_ids)
            .await
            .map_err(RampServiceError::Graph)?;
        Ok(RampDetail {
            ramp: summary,
            items,
            local_papers,
            acquisitions,
            expansions,
        })
    }

    /// Rename a ramp.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found")`; `Graph` for database failures.
    pub async fn rename_ramp(
        &self,
        ramp: RampId,
        title: &RampTitle,
    ) -> Result<RampSummaryView, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        graph
            .ramps
            .rename(workspace, ramp, title)
            .await
            .map_err(|error| database(&error))?
            .ok_or(RampServiceError::NotFound("ramp_not_found"))?;
        self.summary_view(graph, workspace, ramp).await
    }

    /// Delete a ramp and its items.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found")`; `Graph` for database failures.
    pub async fn delete_ramp(&self, ramp: RampId) -> Result<(), RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let deleted = graph
            .ramps
            .delete(workspace, ramp)
            .await
            .map_err(|error| database(&error))?;
        if deleted {
            return Ok(());
        }
        Err(RampServiceError::NotFound("ramp_not_found"))
    }

    /// Add a Work to a ramp, optionally reached from another item (`via`) it cites.
    ///
    /// # Errors
    /// `NotFound` for a missing ramp or Work; `Conflict` per the ramp rules; `Invalid`
    /// is not produced here (notes arrive validated).
    pub async fn add_ramp_item(
        &self,
        ramp: RampId,
        work: WorkId,
        via: Option<WorkId>,
        note: Option<String>,
    ) -> Result<RampItemView, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        self.require_ramp(graph, workspace, ramp).await?;
        let work = existing_work(graph, work).await?;
        let via = match via {
            Some(via) => Some(
                graph
                    .works
                    .resolve_redirect(via)
                    .await
                    .map_err(|error| database(&error))?,
            ),
            None => None,
        };
        graph
            .ramps
            .add_item(
                ramp,
                NewRampItem {
                    work_id: work.id,
                    via,
                    note,
                },
            )
            .await
            .map_err(|error| ramp_failure(&error))?;
        self.item_view(graph, ramp, work.id).await
    }

    /// Change an item's status and/or note. `note: Some(None)` clears the note.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found" | "item_not_found")`; `Graph` for database failures.
    pub async fn update_ramp_item(
        &self,
        ramp: RampId,
        work: WorkId,
        status: Option<RampItemStatus>,
        note: Option<Option<String>>,
    ) -> Result<RampItemView, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        self.require_ramp(graph, workspace, ramp).await?;
        let work = graph
            .works
            .resolve_redirect(work)
            .await
            .map_err(|error| database(&error))?;
        graph
            .ramps
            .update_item(ramp, work, RampItemPatch { status, note })
            .await
            .map_err(|error| ramp_failure(&error))?
            .ok_or(RampServiceError::NotFound("item_not_found"))?;
        self.item_view(graph, ramp, work).await
    }

    /// Remove a non-seed item.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found" | "item_not_found")`; `Conflict("seed_item")`.
    pub async fn remove_ramp_item(
        &self,
        ramp: RampId,
        work: WorkId,
    ) -> Result<(), RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        self.require_ramp(graph, workspace, ramp).await?;
        let work = graph
            .works
            .resolve_redirect(work)
            .await
            .map_err(|error| database(&error))?;
        match graph
            .ramps
            .remove_item(ramp, work)
            .await
            .map_err(|error| ramp_failure(&error))?
        {
            RemoveOutcome::Removed => Ok(()),
            RemoveOutcome::NotFound => Err(RampServiceError::NotFound("item_not_found")),
            RemoveOutcome::Seed => Err(RampServiceError::Conflict(RampError::SeedItem.code())),
        }
    }

    /// Set a manual order. The list must be an exact permutation of the ramp's items
    /// after redirect resolution.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found")`; `Invalid("invalid_order", …)`.
    pub async fn reorder_ramp(
        &self,
        ramp: RampId,
        work_ids: &[WorkId],
    ) -> Result<RampDetail, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        self.require_ramp(graph, workspace, ramp).await?;
        let invalid = || {
            RampServiceError::Invalid(
                RampError::InvalidOrder.code(),
                RampError::InvalidOrder.to_string(),
            )
        };
        let mut resolved = Vec::with_capacity(work_ids.len());
        for work in work_ids {
            resolved.push(
                graph
                    .works
                    .resolve_redirect(*work)
                    .await
                    .map_err(|error| database(&error))?,
            );
        }
        let current: HashSet<WorkId> = graph
            .ramps
            .items(ramp)
            .await
            .map_err(|error| ramp_failure(&error))?
            .into_iter()
            .map(|item| item.work_id)
            .collect();
        let listed: HashSet<WorkId> = resolved.iter().copied().collect();
        if listed.len() != resolved.len() || listed != current {
            return Err(invalid());
        }
        graph
            .ramps
            .reorder(ramp, &resolved)
            .await
            .map_err(|error| ramp_failure(&error))?;
        self.ramp_detail(ramp).await
    }

    /// Reorder "foundational first": cited items before citing ones, deeper and more
    /// prominent first, the seed last.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found")`; `Graph` for database failures.
    pub async fn suggest_ramp_order(&self, ramp: RampId) -> Result<RampDetail, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let found = self.require_ramp(graph, workspace, ramp).await?;
        let items = graph
            .ramps
            .items(ramp)
            .await
            .map_err(|error| ramp_failure(&error))?;
        let work_ids: Vec<WorkId> = items.iter().map(|item| item.work_id).collect();
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
        let order_items: Vec<RampOrderItem> = items
            .iter()
            .map(|item| {
                let work = works.get(&item.work_id);
                RampOrderItem {
                    work_id: item.work_id,
                    depth: item.path.len().saturating_sub(1),
                    score: prominence.get(&item.work_id).map_or(0.0, |row| row.score),
                    year: work.and_then(|work| work.display_year),
                    title_key: title_key(
                        work.and_then(|work| work.display_title.as_deref())
                            .unwrap_or_default(),
                    ),
                    is_seed: item.work_id == found.seed_work_id,
                }
            })
            .collect();
        let cites = graph
            .citations
            .edges_among(&work_ids)
            .await
            .map_err(|error| database(&error))?;
        let order = suggest_order(&order_items, &cites);
        self.reorder_ramp(ramp, &order).await
    }

    /// A ramp from a `p...` ID or a unique case-insensitive title prefix.
    ///
    /// # Errors
    /// `NotFound("ramp_not_found")` when nothing matches; `Invalid("ambiguous_ramp", …)`
    /// listing up to five matches when several do.
    pub async fn ramp_reference(&self, text: &str) -> Result<RampId, RampServiceError> {
        let graph = graph_of(self)?;
        let workspace = workspace_of(graph).await?;
        let text = text.trim();
        if let Ok(id) = text.parse::<RampId>()
            && graph
                .ramps
                .get(workspace, id)
                .await
                .map_err(|error| database(&error))?
                .is_some()
        {
            return Ok(id);
        }
        let needle = text.to_lowercase();
        if needle.is_empty() {
            return Err(RampServiceError::NotFound("ramp_not_found"));
        }
        let ramps = graph
            .ramps
            .list(workspace, None)
            .await
            .map_err(|error| database(&error))?;
        let matches: Vec<&RampSummary> = ramps
            .iter()
            .filter(|summary| summary.ramp.title.to_lowercase().starts_with(&needle))
            .collect();
        match matches.as_slice() {
            [] => Err(RampServiceError::NotFound("ramp_not_found")),
            [only] => Ok(only.ramp.id),
            several => {
                let exact: Vec<&&RampSummary> = several
                    .iter()
                    .filter(|summary| summary.ramp.title.to_lowercase() == needle)
                    .collect();
                if let [only] = exact.as_slice() {
                    return Ok(only.ramp.id);
                }
                let listed: Vec<String> = several
                    .iter()
                    .take(AMBIGUOUS_MATCHES_SHOWN)
                    .map(|summary| format!("{} ({})", summary.ramp.title, summary.ramp.id))
                    .collect();
                let more = several.len().saturating_sub(AMBIGUOUS_MATCHES_SHOWN);
                let tail = if more > 0 {
                    format!("; and {more} more")
                } else {
                    String::new()
                };
                Err(RampServiceError::Invalid(
                    "ambiguous_ramp",
                    format!(
                        "\"{text}\" matches {} ramps: {}{tail}",
                        several.len(),
                        listed.join("; ")
                    ),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use lysilogy_core::IdentifierScheme;

    use super::*;
    use crate::expansion::fixtures::{
        ScriptedTransport, marker, new_work, plain_state, test_graph,
    };

    async fn titled_ramp(state: &AppState, graph: &GraphState, label: &str, title: &str) -> RampId {
        let seed = new_work(graph, IdentifierScheme::Doi, &format!("10.9999/{label}")).await;
        let title = parse_title(title).expect("title");
        state
            .create_ramp(seed, Some(title))
            .await
            .expect("create")
            .ramp
            .id
    }

    #[tokio::test]
    async fn should_resolve_ramp_by_unique_title_prefix_and_reject_ambiguous() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let state = plain_state(graph.clone(), ScriptedTransport::serving(Vec::new())).await;
        let tag = marker();
        let alpha = titled_ramp(
            &state,
            &graph,
            &format!("{tag}-a"),
            &format!("Zq{tag} Alpha"),
        )
        .await;
        let beta = titled_ramp(
            &state,
            &graph,
            &format!("{tag}-b"),
            &format!("Zq{tag} Beta"),
        )
        .await;
        let solo = titled_ramp(&state, &graph, &format!("{tag}-c"), &format!("Solo{tag}")).await;

        assert_eq!(
            state.ramp_reference(&alpha.to_string()).await.expect("id"),
            alpha
        );
        assert_eq!(
            state
                .ramp_reference(&format!("  zq{tag} be"))
                .await
                .expect("prefix, case-insensitive"),
            beta
        );
        assert_eq!(
            state
                .ramp_reference(&format!("SOLO{}", tag.to_uppercase()))
                .await
                .expect("uppercase"),
            solo
        );

        let ambiguous = state
            .ramp_reference(&format!("Zq{tag}"))
            .await
            .expect_err("two matches");
        assert_eq!(ambiguous.code(), "ambiguous_ramp");
        let message = ambiguous.to_string();
        assert!(message.contains(&alpha.to_string()), "{message}");
        assert!(message.contains(&beta.to_string()), "{message}");

        let none = state
            .ramp_reference(&format!("Nothing{tag}"))
            .await
            .expect_err("no match");
        assert_eq!(none.code(), "ramp_not_found");
        let blank = state.ramp_reference("   ").await.expect_err("blank");
        assert_eq!(blank.code(), "ramp_not_found");
        // A well-formed ID that names no ramp falls through to title matching.
        let unknown = state
            .ramp_reference(&RampId::new().to_string())
            .await
            .expect_err("unknown id");
        assert_eq!(unknown.code(), "ramp_not_found");

        for ramp in [alpha, beta, solo] {
            state.delete_ramp(ramp).await.expect("delete");
        }
    }

    #[tokio::test]
    async fn should_list_at_most_five_matches_when_prefix_is_ambiguous() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let state = plain_state(graph.clone(), ScriptedTransport::serving(Vec::new())).await;
        let tag = marker();
        let mut ramps = Vec::new();
        for index in 0..7 {
            ramps.push(
                titled_ramp(
                    &state,
                    &graph,
                    &format!("{tag}-{index}"),
                    &format!("Many{tag} {index}"),
                )
                .await,
            );
        }
        let error = state
            .ramp_reference(&format!("Many{tag}"))
            .await
            .expect_err("ambiguous");
        let message = error.to_string();
        assert_eq!(message.matches("(p").count(), 5, "{message}");
        assert!(message.contains("and 2 more"), "{message}");
        for ramp in ramps {
            state.delete_ramp(ramp).await.expect("delete");
        }
    }

    #[test]
    fn should_map_error_codes_when_converting_service_errors() {
        assert_eq!(
            RampServiceError::NotFound("item_not_found").code(),
            "item_not_found"
        );
        assert_eq!(RampServiceError::Conflict("seed_item").code(), "seed_item");
        assert!(matches!(
            ramp_failure(&DbError::Conflict("ramp_full".to_owned())),
            RampServiceError::Conflict("ramp_full")
        ));
        assert!(matches!(
            ramp_failure(&DbError::not_found()),
            RampServiceError::NotFound("ramp_not_found")
        ));
        assert!(matches!(
            ramp_failure(&DbError::Conflict("mystery".to_owned())),
            RampServiceError::Graph(_)
        ));
        assert_eq!(
            parse_title("").expect_err("empty").code(),
            "invalid_ramp_title"
        );
        assert_eq!(
            parse_status("done").expect_err("status").code(),
            "invalid_status"
        );
        assert_eq!(
            parse_note(Some(&"x".repeat(2001)))
                .expect_err("note")
                .code(),
            "invalid_note"
        );
        assert_eq!(parse_note(Some("  ")).expect("blank"), None);
    }
}
