//! Keeping the prominence cache fresh: after `lit` and acquisition, and on
//! demand through `lysilogy prominence`.

use chrono::{Datelike, Utc};
use lysilogy_core::WorkId;
use lysilogy_ingest::prominence::{
    ProminenceContext, RefreshOptions, RefreshSummary, neighborhood_of, refresh_prominence,
};

use crate::{AppState, Error, Result, domain::PaperId, graph::GraphState};

/// How many Works one `--all` recompute step covers.
const ALL_CHUNK: usize = 500;

/// Which Works a recompute covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProminenceScope {
    /// The paper's linked Work and its neighborhood.
    Paper(PaperId),
    /// Every Work in any assertion or with a document.
    All,
}

impl AppState {
    /// Recompute prominence for a Work and its neighbors in both directions.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn refresh_prominence_around(
        &self,
        work: WorkId,
        fetch_metrics: bool,
    ) -> Result<RefreshSummary> {
        let graph = self.prominence_graph()?;
        let works = neighborhood_of(&graph.citations, work).await?;
        self.refresh_prominence_for(graph, &works, fetch_metrics)
            .await
    }

    /// Recompute prominence for a scope.
    ///
    /// # Errors
    /// `InvalidRequest` without a database or for a paper with no linked Work;
    /// `Literature` for database failures.
    pub async fn recompute_prominence(
        &self,
        scope: ProminenceScope,
        fetch_metrics: bool,
    ) -> Result<RefreshSummary> {
        let graph = self.prominence_graph()?;
        match scope {
            ProminenceScope::Paper(paper) => {
                let work = self.linked_work_of(graph, &paper).await?;
                self.refresh_prominence_around(work, fetch_metrics).await
            }
            ProminenceScope::All => {
                let mut total = RefreshSummary::default();
                let mut after = None;
                loop {
                    let page = graph
                        .prominence
                        .participating_works(after, ALL_CHUNK)
                        .await
                        .map_err(|error| Error::Literature(error.to_string()))?;
                    let Some(last) = page.last().copied() else {
                        return Ok(total);
                    };
                    let part = self
                        .refresh_prominence_for(graph, &page, fetch_metrics)
                        .await?;
                    total.works += part.works;
                    total.with_global_count += part.with_global_count;
                    total.metrics_fetched += part.metrics_fetched;
                    total.metric_requests += part.metric_requests;
                    total.warnings.extend(part.warnings);
                    after = Some(last);
                }
            }
        }
    }

    /// Like [`Self::refresh_prominence_around`], logging instead of failing.
    pub async fn refresh_prominence_quietly(&self, work: WorkId, fetch_metrics: bool) {
        match self.refresh_prominence_around(work, fetch_metrics).await {
            Ok(summary) => {
                for warning in &summary.warnings {
                    tracing::warn!(%work, warning, "prominence refresh warning");
                }
            }
            Err(error) => tracing::warn!(%work, %error, "prominence refresh failed"),
        }
    }

    /// Recompute prominence for one Work only (no neighbors, no metrics),
    /// logging instead of failing. For changes that touch only its own signals.
    pub async fn refresh_prominence_alone_quietly(&self, work: WorkId) {
        let refreshed = async {
            let graph = self.prominence_graph()?;
            self.refresh_prominence_for(graph, &[work], false).await
        }
        .await;
        if let Err(error) = refreshed {
            tracing::warn!(%work, %error, "prominence refresh failed");
        }
    }

    fn prominence_graph(&self) -> Result<&GraphState> {
        self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "prominence needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })
    }

    async fn refresh_prominence_for(
        &self,
        graph: &GraphState,
        works: &[WorkId],
        fetch_metrics: bool,
    ) -> Result<RefreshSummary> {
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .map_err(|error| Error::Literature(error.to_string()))?;
        let context = ProminenceContext {
            works: &graph.works,
            citations: &graph.citations,
            records: &graph.source_records,
            prominence: &graph.prominence,
            openalex: Some(self.literature.client()),
        };
        Ok(refresh_prominence(
            &context,
            workspace,
            works,
            RefreshOptions {
                fetch_metrics,
                refresh_metrics: false,
                current_year: Utc::now().year(),
            },
        )
        .await?)
    }

    pub(crate) async fn linked_work_of(
        &self,
        graph: &GraphState,
        paper: &PaperId,
    ) -> Result<WorkId> {
        let relative_path = self
            .catalog
            .read()
            .await
            .get(paper)
            .map(|entry| entry.overview.relative_path.clone())
            .ok_or_else(|| Error::PaperNotFound(paper.to_string()))?;
        let database = |error: lysilogy_db::DbError| Error::Literature(error.to_string());
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .map_err(database)?;
        let document = graph
            .documents
            .find_by_path(workspace, &relative_path)
            .await
            .map_err(database)?;
        document
            .and_then(|document| document.work_id)
            .ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "paper {paper} has no linked Work; run `lysilogy lit` first"
                ))
            })
    }
}
