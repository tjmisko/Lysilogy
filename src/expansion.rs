//! Expansion: make a Work's outgoing fanout known, and walk backward from a
//! seed to a bounded depth.
//!
//! The neighborhood itself is `graph.citation_assertion`; `ops.work_expansion`
//! records that a Work's fanout was made known, how, and when. Held Works run
//! the local literature flow; every other Work runs the provider flow.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Datelike, Utc};
use lysilogy_core::{
    ExpansionMethod, ExpansionState, ExpansionUnavailableReason, WALK_EXPANSIONS_MAX, WalkBounds,
    WorkId,
};
use lysilogy_db::DbError;
use lysilogy_db::expansion::{ExpansionRow, NewExpansion};
use lysilogy_ingest::expand::{ExpandContext, ProviderExpansion, expand_from_provider};
use lysilogy_ingest::literature::LiteratureStatus;
use serde::Serialize;

use crate::{
    AppState, Error, Result,
    domain::PaperId,
    graph::GraphState,
    walk::{FrontierCandidate, plan_frontier},
};

/// How to expand and how far to walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpandOptions {
    pub bounds: WalkBounds,
    /// Rerun the flow even when the fanout is already known.
    pub refresh: bool,
    /// Also run local evidence for held papers, then the state's field pass.
    pub local: bool,
}

impl Default for ExpandOptions {
    fn default() -> Self {
        Self {
            bounds: WalkBounds::default(),
            refresh: false,
            local: true,
        }
    }
}

/// What expanding one Work did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExpansionReport {
    pub work_id: WorkId,
    /// Answered from the ledger or the existing graph; no flow ran.
    pub cached: bool,
    pub state: ExpansionState,
    pub method: Option<ExpansionMethod>,
    pub unavailable_reason: Option<ExpansionUnavailableReason>,
    pub references: usize,
    pub new_works: usize,
    /// The held paper the local flow ran on.
    pub paper_id: Option<PaperId>,
    pub warnings: Vec<String>,
}

/// What a bounded walk did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WalkReport {
    pub seed: WorkId,
    pub expansions: Vec<ExpansionReport>,
    /// Levels that expanded at least one Work.
    pub levels: usize,
    /// The expansion budget cut the walk short.
    pub truncated: bool,
}

/// Whether a Work's fanout is known, as shown on graph nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionViewState {
    Expanded,
    Unavailable,
    /// Outgoing assertions exist but no ledger row records how.
    Known,
}

/// The client's view of one Work's expansion state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExpansionView {
    pub state: ExpansionViewState,
    pub method: Option<ExpansionMethod>,
    pub references_count: i64,
    pub unavailable_reason: Option<ExpansionUnavailableReason>,
    pub expanded_at: Option<DateTime<Utc>>,
}

impl From<&ExpansionRow> for ExpansionView {
    fn from(row: &ExpansionRow) -> Self {
        Self {
            state: match row.state {
                ExpansionState::Expanded => ExpansionViewState::Expanded,
                ExpansionState::Unavailable => ExpansionViewState::Unavailable,
            },
            method: row.method,
            references_count: i64::from(row.references_count),
            unavailable_reason: row.unavailable_reason,
            expanded_at: Some(row.expanded_at),
        }
    }
}

/// A held paper with the title the catalog shows for it.
pub(crate) struct HeldPaper {
    pub paper_id: PaperId,
    pub title: String,
}

fn database(error: &DbError) -> Error {
    Error::Literature(error.to_string())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
const fn ledger_count(count: usize) -> i32 {
    count as i32
}

#[allow(clippy::cast_possible_truncation)]
const fn ledger_count_of(count: i64) -> i32 {
    count as i32
}

#[allow(clippy::cast_sign_loss)]
const fn report_count(count: i32) -> usize {
    count as usize
}

impl AppState {
    fn expansion_graph(&self) -> Result<&GraphState> {
        self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "expansion needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })
    }

    /// Present local papers linked to the given Works that the catalog holds,
    /// in document order. Works with none have no entry.
    pub(crate) async fn held_papers_with_titles(
        &self,
        graph: &GraphState,
        work_ids: &[WorkId],
    ) -> std::result::Result<BTreeMap<WorkId, Vec<HeldPaper>>, DbError> {
        let workspace = graph.documents.ensure_default_workspace().await?;
        let documents = graph
            .documents
            .local_documents_for_works(workspace, work_ids)
            .await?;
        let mut held: BTreeMap<WorkId, Vec<HeldPaper>> = BTreeMap::new();
        let catalog = self.catalog.read().await;
        for document in documents {
            let paper_id = PaperId::from_relative_path(Path::new(&document.relative_path));
            let Some(entry) = catalog.get(&paper_id) else {
                continue;
            };
            held.entry(document.work_id).or_default().push(HeldPaper {
                paper_id,
                title: entry.overview.metadata.title.clone(),
            });
        }
        drop(catalog);
        Ok(held)
    }

    /// The catalog papers linked to each Work, ordered by paper ID. Works with
    /// no present paper in the catalog have no entry.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn held_papers_for(
        &self,
        work_ids: &[WorkId],
    ) -> Result<BTreeMap<WorkId, Vec<PaperId>>> {
        let graph = self.expansion_graph()?;
        let held = self
            .held_papers_with_titles(graph, work_ids)
            .await
            .map_err(|error| database(&error))?;
        Ok(held
            .into_iter()
            .map(|(work, papers)| {
                let mut ids: Vec<PaperId> =
                    papers.into_iter().map(|paper| paper.paper_id).collect();
                ids.sort_unstable();
                (work, ids)
            })
            .collect())
    }

    /// The Work a catalog paper is linked to (through redirects).
    ///
    /// # Errors
    /// `PaperNotFound` for an unknown paper; `InvalidRequest` without a database
    /// or for a paper with no linked Work.
    pub async fn linked_work(&self, paper: &PaperId) -> Result<WorkId> {
        let graph = self.expansion_graph()?;
        let linked = self.linked_work_of(graph, paper).await?;
        graph
            .works
            .resolve_redirect(linked)
            .await
            .map_err(|error| database(&error))
    }

    /// Display titles of the given Works; Works without one are omitted.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn work_titles(&self, works: &[WorkId]) -> Result<BTreeMap<WorkId, String>> {
        let graph = self.expansion_graph()?;
        let mut titles = BTreeMap::new();
        for work in works {
            let found = graph
                .works
                .get(*work)
                .await
                .map_err(|error| database(&error))?;
            if let Some(title) = found.and_then(|found| found.display_title) {
                titles.insert(*work, title);
            }
        }
        Ok(titles)
    }

    /// Whether expanding the Work without `refresh` would be answered from the
    /// ledger or the existing graph.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn expansion_is_cached(&self, work: WorkId) -> Result<bool> {
        let graph = self.expansion_graph()?;
        if graph
            .expansions
            .get(work)
            .await
            .map_err(|error| database(&error))?
            .is_some()
        {
            return Ok(true);
        }
        let counts = graph
            .citations
            .outgoing_counts(&[work])
            .await
            .map_err(|error| database(&error))?;
        Ok(counts.get(&work).is_some_and(|count| *count > 0))
    }

    /// Make one Work's outgoing fanout known (Decision 3 order): a known fanout
    /// is a cache hit; a cached unavailable row is returned; a held Work runs
    /// the local flow; anything else runs the provider flow.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database or
    /// provider failures. A failure is noted on the ledger row, if one exists.
    pub async fn expand_work(
        &self,
        work: WorkId,
        refresh: bool,
        local: bool,
    ) -> Result<ExpansionReport> {
        let graph = self.expansion_graph()?;
        let outcome = self
            .expand_work_unrecorded(graph, work, refresh, local)
            .await;
        if let Err(error) = &outcome
            && let Err(note) = graph
                .expansions
                .record_failure(work, &error.to_string())
                .await
        {
            tracing::warn!(%work, error = %note, "could not note the expansion failure");
        }
        outcome
    }

    async fn expand_work_unrecorded(
        &self,
        graph: &GraphState,
        work: WorkId,
        refresh: bool,
        local: bool,
    ) -> Result<ExpansionReport> {
        if !refresh && let Some(report) = self.cached_expansion(graph, work).await? {
            return Ok(report);
        }
        let mut warnings = Vec::new();
        let held = self.held_papers_for(&[work]).await?;
        if let Some(paper) = held.get(&work).and_then(|papers| papers.first())
            && let Some(row) = self
                .expand_locally(graph, work, paper, local, &mut warnings)
                .await?
        {
            return Ok(self
                .finish_expansion(work, &row, Some(paper.clone()), warnings)
                .await);
        }
        let row = self.expand_remotely(graph, work, &mut warnings).await?;
        Ok(self.finish_expansion(work, &row, None, warnings).await)
    }

    /// The cache-hit report, writing an `existing` row when only the graph knows.
    async fn cached_expansion(
        &self,
        graph: &GraphState,
        work: WorkId,
    ) -> Result<Option<ExpansionReport>> {
        let row = graph
            .expansions
            .get(work)
            .await
            .map_err(|error| database(&error))?;
        if let Some(row) = row {
            let warnings = match row.state {
                ExpansionState::Expanded => Vec::new(),
                ExpansionState::Unavailable => row.warnings.clone(),
            };
            return Ok(Some(report_from_row(&row, true, 0, None, warnings)));
        }
        let counts = graph
            .citations
            .outgoing_counts(&[work])
            .await
            .map_err(|error| database(&error))?;
        let Some(count) = counts.get(&work).filter(|count| **count > 0) else {
            return Ok(None);
        };
        let row = graph
            .expansions
            .record(NewExpansion {
                work_id: work,
                state: ExpansionState::Expanded,
                method: Some(ExpansionMethod::Existing),
                unavailable_reason: None,
                provider_key: None,
                source_record_id: None,
                references_count: ledger_count_of(*count),
                new_works: 0,
                warnings: Vec::new(),
            })
            .await
            .map_err(|error| database(&error))?;
        Ok(Some(report_from_row(&row, true, 0, None, Vec::new())))
    }

    /// Run the local flow for a held paper. `None` means the flow could not
    /// stand for this Work, with the reason added to `warnings`.
    async fn expand_locally(
        &self,
        graph: &GraphState,
        work: WorkId,
        paper: &PaperId,
        local: bool,
        warnings: &mut Vec<String>,
    ) -> Result<Option<ExpansionRow>> {
        let summary = self.run_literature_flow(paper, false).await?;
        match (&summary.status, summary.work_id) {
            (LiteratureStatus::Linked, Some(linked)) if linked == work => {}
            (LiteratureStatus::Linked, Some(linked)) => {
                warnings.push(format!("local_flow_linked_other_work:{linked}"));
                return Ok(None);
            }
            _ => {
                warnings.push("local_flow_unmatched".to_owned());
                return Ok(None);
            }
        }
        warnings.extend(summary.warnings.iter().cloned());
        if local {
            match self.run_local_evidence(paper, &summary).await {
                Ok(Some(mut evidence)) => {
                    if let Err(error) = self
                        .harvest_default_fields(paper, &summary, &mut evidence)
                        .await
                    {
                        warnings.push(format!("field_pass_failed:{error}"));
                    }
                }
                Ok(None) => {}
                Err(error) => warnings.push(format!("local_evidence_failed:{error}")),
            }
        }
        let row = graph
            .expansions
            .record(NewExpansion {
                work_id: work,
                state: ExpansionState::Expanded,
                method: Some(ExpansionMethod::Local),
                unavailable_reason: None,
                provider_key: summary.openalex_id.clone(),
                source_record_id: None,
                references_count: ledger_count(summary.references),
                new_works: ledger_count(summary.new_stub_works),
                warnings: warnings.clone(),
            })
            .await
            .map_err(|error| database(&error))?;
        Ok(Some(row))
    }

    async fn expand_remotely(
        &self,
        graph: &GraphState,
        work: WorkId,
        warnings: &mut Vec<String>,
    ) -> Result<ExpansionRow> {
        let context = ExpandContext {
            works: &graph.works,
            citations: &graph.citations,
            claims: &graph.claims,
            records: &graph.source_records,
            client: self.literature.client(),
        };
        let expansion = match expand_from_provider(&context, work).await? {
            ProviderExpansion::Expanded {
                provider_key,
                source_record,
                references,
                new_works,
                warnings: provider_warnings,
                ..
            } => {
                warnings.extend(provider_warnings);
                NewExpansion {
                    work_id: work,
                    state: ExpansionState::Expanded,
                    method: Some(ExpansionMethod::Provider),
                    unavailable_reason: None,
                    provider_key: Some(provider_key),
                    source_record_id: Some(source_record),
                    references_count: ledger_count(references),
                    new_works: ledger_count(new_works),
                    warnings: warnings.clone(),
                }
            }
            ProviderExpansion::Unavailable {
                reason,
                provider_key,
                warnings: provider_warnings,
            } => {
                warnings.extend(provider_warnings);
                NewExpansion {
                    work_id: work,
                    state: ExpansionState::Unavailable,
                    method: None,
                    unavailable_reason: Some(reason),
                    provider_key,
                    source_record_id: None,
                    references_count: 0,
                    new_works: 0,
                    warnings: warnings.clone(),
                }
            }
        };
        graph
            .expansions
            .record(expansion)
            .await
            .map_err(|error| database(&error))
    }

    /// Build the report for a run, then rank the new children immediately.
    async fn finish_expansion(
        &self,
        work: WorkId,
        row: &ExpansionRow,
        paper_id: Option<PaperId>,
        warnings: Vec<String>,
    ) -> ExpansionReport {
        let report = report_from_row(row, false, row.new_works, paper_id, warnings);
        if row.state == ExpansionState::Expanded {
            self.refresh_prominence_quietly(work, true).await;
        }
        report
    }

    /// Expansion state of each Work whose fanout is known. Works with unknown
    /// fanout have no entry. Keys are Work IDs.
    pub(crate) async fn expansion_views_raw(
        &self,
        graph: &GraphState,
        works: &[WorkId],
    ) -> std::result::Result<BTreeMap<String, ExpansionView>, DbError> {
        let rows = graph.expansions.rows_for(works).await?;
        let mut views: BTreeMap<String, ExpansionView> = rows
            .iter()
            .map(|row| (row.work_id.to_string(), ExpansionView::from(row)))
            .collect();
        let unrecorded: Vec<WorkId> = works
            .iter()
            .copied()
            .filter(|work| !views.contains_key(&work.to_string()))
            .collect();
        for (work, count) in graph.citations.outgoing_counts(&unrecorded).await? {
            views.insert(
                work.to_string(),
                ExpansionView {
                    state: ExpansionViewState::Known,
                    method: None,
                    references_count: count,
                    unavailable_reason: None,
                    expanded_at: None,
                },
            );
        }
        Ok(views)
    }

    /// Expansion state of each Work whose fanout is known.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn expansion_views(
        &self,
        works: &[WorkId],
    ) -> Result<BTreeMap<String, ExpansionView>> {
        let graph = self.expansion_graph()?;
        self.expansion_views_raw(graph, works)
            .await
            .map_err(|error| database(&error))
    }

    /// Expand `seed`, then breadth first its best-ranked children, within the
    /// options' depth and fanout and at most [`WALK_EXPANSIONS_MAX`] expansions.
    ///
    /// # Errors
    /// The first failing expansion aborts the walk with its error.
    pub async fn walk_from(&self, seed: WorkId, options: ExpandOptions) -> Result<WalkReport> {
        self.walk_from_with_budget(seed, options, WALK_EXPANSIONS_MAX)
            .await
    }

    pub(crate) async fn walk_from_with_budget(
        &self,
        seed: WorkId,
        options: ExpandOptions,
        budget: usize,
    ) -> Result<WalkReport> {
        let graph = self.expansion_graph()?;
        let depth = usize::from(options.bounds.depth);
        let fanout = usize::from(options.bounds.fanout);
        let mut visited = HashSet::from([seed]);
        let mut level = vec![seed];
        let mut expansions: Vec<ExpansionReport> = Vec::new();
        let mut levels = 0;
        let mut truncated = false;
        for level_index in 0..depth {
            if level.is_empty() {
                break;
            }
            levels += 1;
            for work in &level {
                match self
                    .expand_work(*work, options.refresh, options.local)
                    .await
                {
                    Ok(report) => expansions.push(report),
                    Err(error) => {
                        tracing::warn!(%seed, done = expansions.len(), %error, "walk aborted");
                        return Err(error);
                    }
                }
            }
            if level_index + 1 >= depth {
                break;
            }
            let candidates = self.frontier_candidates(graph, &level).await?;
            let remaining = budget.saturating_sub(expansions.len());
            let planned = plan_frontier(&candidates, &level, fanout, &visited, remaining);
            let unbounded = plan_frontier(&candidates, &level, fanout, &visited, usize::MAX);
            truncated |= planned.len() < unbounded.len();
            level = planned.iter().map(|(_, child)| *child).collect();
            visited.extend(level.iter().copied());
        }
        Ok(WalkReport {
            seed,
            expansions,
            levels,
            truncated,
        })
    }

    /// Every outgoing child of the level's Works with its score, year, and mark.
    async fn frontier_candidates(
        &self,
        graph: &GraphState,
        level: &[WorkId],
    ) -> Result<Vec<FrontierCandidate>> {
        let mut children = Vec::new();
        for parent in level {
            let outgoing = graph
                .citations
                .outgoing(*parent)
                .await
                .map_err(|error| database(&error))?;
            children.extend(outgoing.into_iter().map(|child| (*parent, child)));
        }
        let child_ids: Vec<WorkId> = children.iter().map(|(_, child)| child.id).collect();
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .map_err(|error| database(&error))?;
        let prominence = graph
            .prominence
            .prominence_for(workspace, &child_ids, Utc::now().year())
            .await
            .map_err(|error| database(&error))?;
        let marks: BTreeMap<WorkId, _> = graph
            .curation
            .marks_for_works(workspace, &child_ids)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .map(|(work, stored)| (work, stored.mark))
            .collect();
        Ok(children
            .into_iter()
            .map(|(parent, child)| FrontierCandidate {
                parent,
                work_id: child.id,
                score: prominence.get(&child.id).map_or(0.0, |row| row.score),
                year: child.display_year,
                mark: marks.get(&child.id).copied(),
            })
            .collect())
    }
}

const fn report_from_row(
    row: &ExpansionRow,
    cached: bool,
    new_works: i32,
    paper_id: Option<PaperId>,
    warnings: Vec<String>,
) -> ExpansionReport {
    ExpansionReport {
        work_id: row.work_id,
        cached,
        state: row.state,
        method: row.method,
        unavailable_reason: row.unavailable_reason,
        references: report_count(row.references_count),
        new_works: report_count(new_works),
        paper_id,
        warnings,
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::sync::{Arc, Mutex};

    use lysilogy_core::{IdentifierScheme, ProminenceSignals, WorkKind};
    use lysilogy_ingest::openalex::{JsonTransport, ProviderError, ProviderFuture, work_url};
    use serde_json::{Value, json};

    use crate::{AppState, graph::GraphState, literature::LiteratureClient};
    use lysilogy_core::WorkId;

    pub fn marker() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("x{}x{nanos}", std::process::id())
    }

    pub async fn test_graph() -> Option<GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(GraphState::connect(&url).await.expect("connect"))
    }

    pub type Responder = dyn Fn(&str) -> std::result::Result<Value, ProviderError> + Send + Sync;

    /// Records every URL and answers from a closure; never touches the network.
    #[derive(Clone)]
    pub struct ScriptedTransport {
        responder: Arc<Responder>,
        urls: Arc<Mutex<Vec<String>>>,
    }

    impl ScriptedTransport {
        pub fn new(
            responder: impl Fn(&str) -> std::result::Result<Value, ProviderError>
            + Send
            + Sync
            + 'static,
        ) -> Self {
            Self {
                responder: Arc::new(responder),
                urls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// Serves each `(OpenAlex or doi: key, record)` pair, titles summary
        /// batches, answers metrics batches, gives Crossref an empty record, and
        /// 404s everything else.
        pub fn serving(records: Vec<(String, Value)>) -> Self {
            let routes: Vec<(String, Value)> = records
                .into_iter()
                .map(|(key, record)| (work_url(&key), record))
                .collect();
            Self::new(move |url| {
                if url.contains("filter=openalex:") {
                    return Ok(batch_reply(url));
                }
                if url.contains("api.crossref.org/works/") {
                    // A registry record that holds no usable metadata.
                    return Ok(json!({"message": {}}));
                }
                routes
                    .iter()
                    .find(|(route, _)| route == url)
                    .map(|(_, record)| record.clone())
                    .ok_or(ProviderError::Status(404))
            })
        }

        pub fn urls(&self) -> Vec<String> {
            self.urls.lock().expect("lock").clone()
        }

        pub fn record_requests(&self) -> usize {
            self.urls()
                .iter()
                .filter(|url| url.contains("/works/"))
                .count()
        }
    }

    impl JsonTransport for ScriptedTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async move {
                self.urls.lock().expect("lock").push(url.to_owned());
                (self.responder)(url)
            })
        }
    }

    /// Titles summaries and gives metrics requests a cited-by count.
    pub fn batch_reply(url: &str) -> Value {
        let ids = url
            .split("filter=openalex:")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or_default();
        let metrics = url.contains("cited_by_count");
        let results: Vec<Value> = ids
            .split('|')
            .map(|id| {
                if metrics {
                    json!({"id": format!("https://openalex.org/{id}"), "cited_by_count": 4})
                } else {
                    json!({"id": format!("https://openalex.org/{id}"),
                           "display_name": format!("Title of {id}"), "publication_year": 2008,
                           "authorships": []})
                }
            })
            .collect();
        json!({ "results": results })
    }

    pub fn record_json(openalex_id: &str, title: &str, references: &[String]) -> Value {
        let urls: Vec<String> = references
            .iter()
            .map(|id| format!("https://openalex.org/{id}"))
            .collect();
        json!({
            "id": format!("https://openalex.org/{openalex_id}"),
            "display_name": title,
            "publication_year": 2013,
            "referenced_works": urls,
        })
    }

    /// A state with no library papers, talking to the given transport.
    pub async fn plain_state(graph: GraphState, transport: ScriptedTransport) -> AppState {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state")
            .with_graph(graph)
            .with_literature_client(LiteratureClient::new(transport));
        std::mem::forget((library, data));
        state
    }

    pub async fn new_work(graph: &GraphState, scheme: IdentifierScheme, value: &str) -> WorkId {
        graph
            .works
            .resolve_or_create(scheme, value, WorkKind::Article, None)
            .await
            .expect("work")
            .0
            .id
    }

    pub async fn store_score(graph: &GraphState, work: WorkId, score: f64) {
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        graph
            .prominence
            .store(workspace, &[(work, ProminenceSignals::default(), score)])
            .await
            .expect("store");
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use lysilogy_core::{
        DocumentFormat, IdentifierScheme, ScannedFile, Visibility, WorkKind, WorkLinkMethod,
    };
    use lysilogy_ingest::openalex::ProviderError;

    use super::fixtures::{
        ScriptedTransport, marker, new_work, plain_state, record_json, store_score, test_graph,
    };
    use super::*;
    use crate::literature::LiteratureClient;

    struct Held {
        state: AppState,
        paper_id: PaperId,
        source: PathBuf,
        file_name: String,
        graph: GraphState,
    }

    /// A state holding one placeholder PDF whose fake `pdftotext` prints
    /// `front_matter`; the paper is not linked to any Work yet.
    async fn held_state(
        graph: GraphState,
        transport: ScriptedTransport,
        marker: &str,
        front_matter: &str,
    ) -> Held {
        let programs = tempfile::tempdir().expect("programs");
        let pdftotext = programs.path().join("pdftotext");
        std::fs::write(
            &pdftotext,
            format!("#!/bin/sh\ncat <<'EOF'\n{front_matter}\nEOF\n"),
        )
        .expect("script");
        std::fs::set_permissions(&pdftotext, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let file_name = format!("Autor - 2013 - The Study of Things {marker}.pdf");
        let source = library.path().join(&file_name);
        std::fs::write(&source, format!("placeholder {marker}")).expect("write");
        let state = AppState::with_services(
            library.path(),
            data.path(),
            crate::extract::PdfExtractor::with_programs(pdftotext, "pdfinfo"),
            crate::analysis::AnalysisService::default(),
        )
        .await
        .expect("state")
        .with_graph(graph.clone())
        .with_literature_client(LiteratureClient::new(transport));
        std::mem::forget((programs, library, data));
        let paper_id = PaperId::from_relative_path(Path::new(&file_name));
        Held {
            state,
            paper_id,
            source,
            file_name,
            graph,
        }
    }

    impl Held {
        /// Register the file as a present document linked to the Work.
        async fn link_to(&self, work: WorkId) {
            let workspace = self
                .graph
                .documents
                .ensure_default_workspace()
                .await
                .expect("workspace");
            let file = ScannedFile {
                relative_path: self.file_name.clone(),
                sha256: format!("sha-{}", self.file_name),
                bytes: 11,
                format: DocumentFormat::Pdf,
            };
            self.graph
                .documents
                .apply_partial_scan(workspace, &[file])
                .await
                .expect("scan");
            let document = self
                .graph
                .documents
                .find_by_path(workspace, &self.file_name)
                .await
                .expect("find")
                .expect("document");
            self.graph
                .documents
                .set_work_link(document.id, work, WorkLinkMethod::Manual, None, "test")
                .await
                .expect("link");
        }
    }

    #[tokio::test]
    async fn should_fail_when_expanding_without_graph() {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        let outcome = state.expand_work(WorkId::new(), false, true).await;
        assert!(
            matches!(outcome, Err(Error::InvalidRequest(_))),
            "{outcome:?}"
        );
        let walk = state
            .walk_from(WorkId::new(), ExpandOptions::default())
            .await;
        assert!(matches!(walk, Err(Error::InvalidRequest(_))));
        assert!(state.expansion_views(&[WorkId::new()]).await.is_err());
        assert!(state.held_papers_for(&[WorkId::new()]).await.is_err());
    }

    #[tokio::test]
    async fn should_return_cached_without_flow_when_fanout_known() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(503)));
        let state = plain_state(graph.clone(), transport.clone()).await;
        let work = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W1{}", marker()),
        )
        .await;
        let child = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W2{}", marker()),
        )
        .await;
        graph
            .citations
            .add_assertion(work, child, None)
            .await
            .expect("edge");

        let report = state.expand_work(work, false, true).await.expect("expand");
        assert!(report.cached);
        assert_eq!(report.state, ExpansionState::Expanded);
        assert_eq!(report.method, Some(ExpansionMethod::Existing));
        assert_eq!(report.references, 1);
        assert_eq!(report.paper_id, None);
        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.method, Some(ExpansionMethod::Existing));
        assert_eq!(row.references_count, 1);

        let again = state.expand_work(work, false, true).await.expect("again");
        assert!(again.cached);
        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.attempts, 1, "a cache hit is not an attempt");
        assert!(transport.urls().is_empty(), "no network on a cache hit");
    }

    #[tokio::test]
    async fn should_return_cached_unavailable_until_refresh() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let openalex = format!("W1{marker}");
        let record = record_json(&openalex, "Late Arrival", &[format!("W2{marker}")]);
        let transport = ScriptedTransport::serving(vec![(openalex.clone(), record)]);
        let state = plain_state(graph.clone(), transport.clone()).await;
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;

        let first = state.expand_work(work, false, true).await.expect("first");
        assert!(!first.cached);
        assert_eq!(first.state, ExpansionState::Unavailable);
        assert_eq!(
            first.unavailable_reason,
            Some(ExpansionUnavailableReason::NoProviderIdentifier)
        );
        assert!(transport.urls().is_empty());

        graph
            .works
            .add_identifier(
                work,
                IdentifierScheme::Openalex,
                &openalex,
                None,
                lysilogy_core::IdentifierStatus::Active,
            )
            .await
            .expect("identifier");
        let second = state.expand_work(work, false, true).await.expect("second");
        assert!(
            second.cached,
            "an unavailable row is returned until refresh"
        );
        assert_eq!(second.state, ExpansionState::Unavailable);
        assert!(transport.urls().is_empty());

        let third = state.expand_work(work, true, true).await.expect("third");
        assert!(!third.cached);
        assert_eq!(third.state, ExpansionState::Expanded);
        assert_eq!(third.method, Some(ExpansionMethod::Provider));
        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.attempts, 2);
        assert_eq!(row.state, ExpansionState::Expanded);
    }

    #[tokio::test]
    async fn should_run_provider_flow_when_work_not_held() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let openalex = format!("W1{marker}");
        let cited = [format!("W2{marker}"), format!("W3{marker}")];
        let record = record_json(&openalex, "A Stub Paper", &cited);
        let transport = ScriptedTransport::serving(vec![(openalex.clone(), record)]);
        let state = plain_state(graph.clone(), transport.clone()).await;
        let work = new_work(&graph, IdentifierScheme::Openalex, &openalex).await;

        let report = state.expand_work(work, false, true).await.expect("expand");
        assert!(!report.cached);
        assert_eq!(report.state, ExpansionState::Expanded);
        assert_eq!(report.method, Some(ExpansionMethod::Provider));
        assert_eq!((report.references, report.new_works), (2, 2));
        assert_eq!(report.paper_id, None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(transport.record_requests(), 1);

        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.provider_key.as_deref(), Some(openalex.as_str()));
        assert!(row.source_record_id.is_some());
        let children = graph.citations.outgoing(work).await.expect("outgoing");
        assert_eq!(children.len(), 2);
        assert!(children.iter().all(|child| child.display_title.is_some()));

        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .expect("workspace");
        let ranked = graph
            .prominence
            .cached(workspace, &[work, children[0].id, children[1].id])
            .await
            .expect("cached");
        assert_eq!(ranked.len(), 3, "new children are ranked right away");

        let views = state
            .expansion_views(&[work, children[0].id])
            .await
            .expect("views");
        assert_eq!(views[&work.to_string()].state, ExpansionViewState::Expanded);
        assert!(!views.contains_key(&children[0].id.to_string()));
    }

    #[tokio::test]
    async fn should_run_local_flow_when_work_is_held() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = format!("10.5555/{marker}");
        let title = format!("The Study of Things {marker}");
        let record = record_json(&format!("W9{marker}"), &title, &[format!("W8{marker}")]);
        let transport = ScriptedTransport::serving(vec![(format!("doi:{doi}"), record)]);
        let held = held_state(
            graph.clone(),
            transport.clone(),
            &marker,
            &format!("{title}\nDOI: {doi}"),
        )
        .await;
        let work = new_work(&graph, IdentifierScheme::Doi, &doi).await;
        held.link_to(work).await;

        let report = held
            .state
            .expand_work(work, false, false)
            .await
            .expect("expand");
        assert!(!report.cached);
        assert_eq!(report.state, ExpansionState::Expanded);
        assert_eq!(report.method, Some(ExpansionMethod::Local));
        assert_eq!(report.paper_id.as_ref(), Some(&held.paper_id));
        assert_eq!(report.references, 1);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.method, Some(ExpansionMethod::Local));
        assert_eq!(graph.citations.outgoing(work).await.expect("out").len(), 1);

        let again = held
            .state
            .expand_work(work, false, false)
            .await
            .expect("again");
        assert!(again.cached);
        assert_eq!(again.method, Some(ExpansionMethod::Local));
        let held_papers = held.state.held_papers_for(&[work]).await.expect("held");
        assert_eq!(held_papers[&work], std::slice::from_ref(&held.paper_id));
    }

    #[tokio::test]
    async fn should_run_local_evidence_when_held_and_local_enabled() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = format!("10.5555/{marker}");
        let title = format!("The Study of Things {marker}");
        let record = record_json(&format!("W9{marker}"), &title, &[format!("W8{marker}")]);
        let transport = ScriptedTransport::serving(vec![(format!("doi:{doi}"), record)]);
        let held = held_state(
            graph.clone(),
            transport,
            &marker,
            &format!("{title}\nDOI: {doi}"),
        )
        .await;
        crate::source_index::test_support::cache_fixture_with_blocks(
            &held.source,
            &held.state.store.paper_dir(&held.paper_id),
            &[
                (&title, "heading", 1),
                (&format!("DOI: {doi}"), "body", 1),
                ("Earlier work [1] shows this clearly.", "body", 1),
                ("References", "heading", 2),
                (
                    &format!("[1] Adams. Cited evidence {marker}. 2019."),
                    "body",
                    2,
                ),
            ],
        )
        .await;
        let work = new_work(&graph, IdentifierScheme::Doi, &doi).await;
        held.link_to(work).await;

        let report = held
            .state
            .expand_work(work, false, true)
            .await
            .expect("expand");
        assert_eq!(report.method, Some(ExpansionMethod::Local));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let outgoing = graph.citations.outgoing(work).await.expect("outgoing");
        assert_eq!(
            outgoing.len(),
            2,
            "the provider edge plus the local bibliography edge"
        );
    }

    #[tokio::test]
    async fn should_warn_and_still_expand_when_local_evidence_fails() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = format!("10.5555/{marker}");
        let title = format!("The Study of Things {marker}");
        let record = record_json(&format!("W9{marker}"), &title, &[format!("W8{marker}")]);
        let transport = ScriptedTransport::serving(vec![(format!("doi:{doi}"), record)]);
        let held = held_state(
            graph.clone(),
            transport,
            &marker,
            &format!("{title}\nDOI: {doi}"),
        )
        .await;
        let work = new_work(&graph, IdentifierScheme::Doi, &doi).await;
        held.link_to(work).await;
        // The index builder cannot read a placeholder file, so local evidence fails.
        std::fs::remove_file(&held.source).ok();
        std::fs::write(&held.source, "").expect("empty file");

        let report = held
            .state
            .expand_work(work, false, true)
            .await
            .expect("expand");
        assert_eq!(report.state, ExpansionState::Expanded);
        assert_eq!(report.method, Some(ExpansionMethod::Local));
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.starts_with("local_evidence_failed:")),
            "{:?}",
            report.warnings
        );
    }

    #[tokio::test]
    async fn should_fall_back_to_provider_when_local_flow_links_other_work() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = format!("10.5555/{marker}");
        let title = format!("The Study of Things {marker}");
        let own = format!("W1{marker}");
        let routes = vec![
            (
                format!("doi:{doi}"),
                record_json(&format!("W7{marker}"), &title, &[format!("W6{marker}")]),
            ),
            (
                own.clone(),
                record_json(
                    &own,
                    "The Held Work",
                    &[format!("W2{marker}"), format!("W3{marker}")],
                ),
            ),
        ];
        let transport = ScriptedTransport::serving(routes);
        let held = held_state(
            graph.clone(),
            transport,
            &marker,
            &format!("{title}\nDOI: {doi}"),
        )
        .await;
        let work = new_work(&graph, IdentifierScheme::Openalex, &own).await;
        held.link_to(work).await;

        let report = held
            .state
            .expand_work(work, false, false)
            .await
            .expect("expand");
        let other = graph
            .works
            .find_by_identifier(IdentifierScheme::Doi, &doi)
            .await
            .expect("find")
            .expect("the paper's own Work");
        assert_ne!(other.id, work);
        assert_eq!(report.method, Some(ExpansionMethod::Provider));
        assert_eq!(report.paper_id, None);
        assert_eq!(report.references, 2);
        assert!(
            report
                .warnings
                .contains(&format!("local_flow_linked_other_work:{}", other.id)),
            "{:?}",
            report.warnings
        );
        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.warnings, report.warnings);
    }

    #[tokio::test]
    async fn should_fall_back_to_provider_when_local_flow_is_unmatched() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let own = format!("W1{marker}");
        let routes = vec![(
            own.clone(),
            record_json(&own, "The Held Work", &[format!("W2{marker}")]),
        )];
        let transport = ScriptedTransport::serving(routes);
        let held = held_state(
            graph.clone(),
            transport,
            &marker,
            "Nothing to identify here",
        )
        .await;
        let work = new_work(&graph, IdentifierScheme::Openalex, &own).await;
        held.link_to(work).await;

        let report = held
            .state
            .expand_work(work, false, true)
            .await
            .expect("expand");
        assert_eq!(report.method, Some(ExpansionMethod::Provider));
        assert!(
            report.warnings.contains(&"local_flow_unmatched".to_owned()),
            "{:?}",
            report.warnings
        );
        assert_eq!(report.references, 1);
    }

    /// A seed citing three stub children, each of which cites two more.
    struct Tree {
        seed: WorkId,
        children: [WorkId; 3],
        grandchildren: Vec<[String; 2]>,
        transport: ScriptedTransport,
    }

    async fn tree(graph: &GraphState, scores: [f64; 3]) -> Tree {
        let marker = marker();
        let seed = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("seed")
            .id;
        let mut children = Vec::new();
        let mut grandchildren = Vec::new();
        let mut routes = Vec::new();
        for (index, score) in scores.iter().enumerate() {
            let own = format!("W{index}{marker}");
            let child = new_work(graph, IdentifierScheme::Openalex, &own).await;
            graph
                .citations
                .add_assertion(seed, child, None)
                .await
                .expect("edge");
            store_score(graph, child, *score).await;
            let cited = [format!("W5{index}{marker}"), format!("W6{index}{marker}")];
            routes.push((own.clone(), record_json(&own, "A Child", &cited)));
            children.push(child);
            grandchildren.push(cited);
        }
        Tree {
            seed,
            children: [children[0], children[1], children[2]],
            grandchildren,
            transport: ScriptedTransport::serving(routes),
        }
    }

    #[tokio::test]
    async fn should_walk_two_levels_with_fanout_two_when_bounds_given() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let tree = tree(&graph, [0.9, 0.8, 0.1]).await;
        let state = plain_state(graph.clone(), tree.transport.clone()).await;
        let options = ExpandOptions {
            bounds: WalkBounds::parse(Some(2), Some(2)).expect("bounds"),
            refresh: false,
            local: true,
        };

        let report = state.walk_from(tree.seed, options).await.expect("walk");
        assert_eq!(report.seed, tree.seed);
        assert_eq!(report.levels, 2);
        assert!(!report.truncated, "a fanout cut is not a budget cut");
        let order: Vec<WorkId> = report.expansions.iter().map(|r| r.work_id).collect();
        assert_eq!(order, [tree.seed, tree.children[0], tree.children[1]]);
        assert!(report.expansions[0].cached, "the seed was already known");
        assert_eq!(report.expansions[0].method, Some(ExpansionMethod::Existing));
        for expansion in &report.expansions[1..] {
            assert!(!expansion.cached);
            assert_eq!(expansion.method, Some(ExpansionMethod::Provider));
            assert_eq!(expansion.references, 2);
        }
        assert!(
            graph
                .expansions
                .get(tree.children[2])
                .await
                .expect("get")
                .is_none(),
            "the lowest-scoring child is skipped"
        );
        for index in 0..2 {
            let outgoing = graph
                .citations
                .outgoing(tree.children[index])
                .await
                .expect("outgoing");
            assert_eq!(outgoing.len(), 2, "{:?}", tree.grandchildren[index]);
        }
    }

    #[tokio::test]
    async fn should_stop_after_seed_when_depth_is_one() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let tree = tree(&graph, [0.9, 0.8, 0.1]).await;
        let state = plain_state(graph.clone(), tree.transport.clone()).await;
        let report = state
            .walk_from(tree.seed, ExpandOptions::default())
            .await
            .expect("walk");
        assert_eq!(report.levels, 1);
        assert_eq!(report.expansions.len(), 1);
        assert!(!report.truncated);
        assert!(tree.transport.urls().is_empty());
    }

    #[tokio::test]
    async fn should_truncate_walk_when_budget_exhausted() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let tree = tree(&graph, [0.9, 0.8, 0.7]).await;
        let state = plain_state(graph.clone(), tree.transport.clone()).await;
        let options = ExpandOptions {
            bounds: WalkBounds::parse(Some(2), Some(3)).expect("bounds"),
            refresh: false,
            local: true,
        };

        let cut = state
            .walk_from_with_budget(tree.seed, options, 2)
            .await
            .expect("walk");
        assert!(cut.truncated);
        assert_eq!(cut.expansions.len(), 2, "the seed plus one child");
        assert_eq!(cut.expansions[1].work_id, tree.children[0]);

        let tree = self::tree(&graph, [0.9, 0.8, 0.7]).await;
        let state = plain_state(graph.clone(), tree.transport.clone()).await;
        let exact = state
            .walk_from_with_budget(tree.seed, options, 4)
            .await
            .expect("walk");
        assert!(!exact.truncated, "the budget fits exactly");
        assert_eq!(exact.expansions.len(), 4);
    }

    #[tokio::test]
    async fn should_record_failure_and_fail_job_when_provider_errors() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(503)));
        let state = plain_state(graph.clone(), transport).await;
        let work = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W1{}", marker()),
        )
        .await;

        let first = state.expand_work(work, false, true).await;
        assert!(first.is_err(), "a hard provider error fails the run");
        assert!(
            graph.expansions.get(work).await.expect("get").is_none(),
            "no row is written for a Work that never had one"
        );

        graph
            .expansions
            .record(NewExpansion {
                work_id: work,
                state: ExpansionState::Unavailable,
                method: None,
                unavailable_reason: Some(ExpansionUnavailableReason::ProviderNotFound),
                provider_key: None,
                source_record_id: None,
                references_count: 0,
                new_works: 0,
                warnings: Vec::new(),
            })
            .await
            .expect("row");
        let retry = state.expand_work(work, true, true).await;
        assert!(retry.is_err());
        let row = graph.expansions.get(work).await.expect("get").expect("row");
        assert_eq!(row.attempts, 2);
        assert!(row.last_error.is_some(), "{row:?}");
        assert_eq!(row.state, ExpansionState::Unavailable, "state is untouched");

        let walk = state
            .walk_from(
                work,
                ExpandOptions {
                    refresh: true,
                    ..ExpandOptions::default()
                },
            )
            .await;
        assert!(walk.is_err(), "a failing expansion aborts the walk");
    }

    #[tokio::test]
    async fn should_mark_known_and_skip_unknown_when_building_views() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(503)));
        let state = plain_state(graph.clone(), transport).await;
        let known = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W1{}", marker()),
        )
        .await;
        let child = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W2{}", marker()),
        )
        .await;
        let unknown = new_work(
            &graph,
            IdentifierScheme::Openalex,
            &format!("W3{}", marker()),
        )
        .await;
        graph
            .citations
            .add_assertion(known, child, None)
            .await
            .expect("edge");

        let views = state
            .expansion_views(&[known, child, unknown, known])
            .await
            .expect("views");
        let view = &views[&known.to_string()];
        assert_eq!(view.state, ExpansionViewState::Known);
        assert_eq!(view.references_count, 1);
        assert_eq!((view.method, view.expanded_at), (None, None));
        assert!(!views.contains_key(&child.to_string()));
        assert!(!views.contains_key(&unknown.to_string()));
        assert!(state.expansion_views(&[]).await.expect("empty").is_empty());
    }
}
