//! Assemble the neighbour dossier for a library paper from the citation graph.
//!
//! What is really available locally: the paper's document row links it to a
//! graph Work; `graph.citation_assertion` gives the Works it cites and the
//! Works that cite it, each with a display title, year and author list;
//! `reader.work_prominence` gives citation counts (worldwide, from the
//! provider payload, and within the local graph) and a prominence score; and
//! a neighbour the library holds may have a stored analysis whose abstract
//! and thesis can be quoted. The graph keeps no abstracts of its own.
//!
//! Every failure yields an empty dossier rather than an error: the context
//! node must run for a paper that was never linked or when the database is
//! down, and it says so to the model.

use std::collections::BTreeMap;

use chrono::{Datelike, Utc};
use lysilogy_core::{Work, WorkId};
use lysilogy_db::{DbError, prominence::StoredProminence};

use crate::{
    AppState,
    domain::{DossierEdgeLabel, DossierStatus, PaperId},
    graph::GraphState,
    paper_artifacts::dossier::{Dossier, DossierWork},
};

impl AppState {
    /// The neighbour dossier for one library paper; empty when there is no
    /// graph, the paper is not linked to a Work, or a query fails.
    pub async fn neighbour_dossier(&self, id: &PaperId) -> Dossier {
        let Some(graph) = &self.graph else {
            return Dossier::empty(DossierStatus::NoGraph);
        };
        match self.graph_dossier(graph, id).await {
            Ok(dossier) => dossier,
            Err(error) => {
                tracing::warn!(paper_id = %id, %error, "could not assemble the neighbour dossier");
                Dossier::empty(DossierStatus::Error)
            }
        }
    }

    async fn graph_dossier(&self, graph: &GraphState, id: &PaperId) -> Result<Dossier, DbError> {
        let Some(relative_path) = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.overview.relative_path.clone())
        else {
            return Ok(Dossier::empty(DossierStatus::NotLinked));
        };
        let workspace = graph.documents.ensure_default_workspace().await?;
        let Some(document) = graph
            .documents
            .find_by_path(workspace, &relative_path)
            .await?
        else {
            return Ok(Dossier::empty(DossierStatus::NotLinked));
        };
        let Some(link) = graph.documents.work_link(document.id).await? else {
            return Ok(Dossier::empty(DossierStatus::NotLinked));
        };
        let center = graph.works.resolve_redirect(link.work_id).await?;
        let references = graph.citations.outgoing(center).await?;
        let citing = graph.citations.incoming(center).await?;
        let mut ids: Vec<WorkId> = references
            .iter()
            .chain(&citing)
            .map(|work| work.id)
            .collect();
        ids.push(center);
        let prominence = graph
            .prominence
            .prominence_for(workspace, &ids, Utc::now().year())
            .await?;
        let subject = graph
            .works
            .get(center)
            .await?
            .map(|work| dossier_work(&work, prominence.get(&center)));
        let side = |works: &[Work]| {
            works
                .iter()
                .filter(|work| work.id != center)
                .map(|work| dossier_work(work, prominence.get(&work.id)))
                .collect::<Vec<_>>()
        };
        let mut pairs: Vec<(WorkId, WorkId)> =
            references.iter().map(|work| (center, work.id)).collect();
        pairs.extend(citing.iter().map(|work| (work.id, center)));
        let labels = edge_labels(
            &graph.relationships.current_claims_for_edges(&pairs).await?,
        );
        let with_labels = |mut works: Vec<DossierWork>, citing: bool| {
            for work in &mut works {
                let Ok(id) = work.id.parse::<WorkId>() else {
                    continue;
                };
                let pair = if citing { (id, center) } else { (center, id) };
                work.edge = labels.get(&pair).cloned();
            }
            works
        };
        let mut dossier = Dossier {
            subject,
            references: with_labels(side(&references), false),
            citing: with_labels(side(&citing), true),
            status: DossierStatus::Ok,
        }
        .ranked();
        self.add_held_summaries(graph, &mut dossier).await?;
        Ok(dossier)
    }

    /// Mark the neighbours the library holds and quote the abstract and
    /// thesis of any that has a stored analysis.
    async fn add_held_summaries(
        &self,
        graph: &GraphState,
        dossier: &mut Dossier,
    ) -> Result<(), DbError> {
        let ids: Vec<WorkId> = dossier
            .references
            .iter()
            .chain(&dossier.citing)
            .filter_map(|work| work.id.parse().ok())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let held: BTreeMap<String, PaperId> = self
            .held_papers_with_titles(graph, &ids)
            .await?
            .into_iter()
            .filter_map(|(work, mut papers)| {
                papers.sort_by(|left, right| left.paper_id.cmp(&right.paper_id));
                papers
                    .into_iter()
                    .next()
                    .map(|paper| (work.to_string(), paper.paper_id))
            })
            .collect();
        for work in dossier
            .references
            .iter_mut()
            .chain(dossier.citing.iter_mut())
        {
            let Some(paper_id) = held.get(&work.id) else {
                continue;
            };
            work.local_paper_id = Some(paper_id.to_string());
            if let Ok(Some(analysis)) = self.store.load_analysis(paper_id).await {
                work.abstract_text = analysis.author_abstract;
                work.thesis = Some(analysis.thesis).filter(|thesis| !thesis.trim().is_empty());
            }
        }
        Ok(())
    }
}

/// The newest current label on each edge, leaving out labels a person
/// rejected: a rejected label is not evidence.
fn edge_labels(
    claims: &[lysilogy_db::relationship::StoredClaimWithMention],
) -> BTreeMap<(WorkId, WorkId), DossierEdgeLabel> {
    let mut labels = BTreeMap::new();
    for stored in claims {
        let claim = &stored.claim;
        if claim.review_state == lysilogy_core::ReviewState::Rejected {
            labels.remove(&(stored.citing, stored.cited));
            continue;
        }
        labels.insert(
            (stored.citing, stored.cited),
            DossierEdgeLabel {
                relation: claim.relation.clone(),
                reason: claim.reason.clone().unwrap_or_default(),
                quote: claim.quote.clone(),
                path: Some(claim.path().as_str().to_owned()),
                engagement: claim.engagement.clone(),
                review_state: claim.review_state.as_str().to_owned(),
            },
        );
    }
    labels
}

/// One graph Work as a dossier record.
fn dossier_work(work: &Work, prominence: Option<&StoredProminence>) -> DossierWork {
    DossierWork {
        id: work.id.to_string(),
        title: work
            .display_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned),
        year: work.display_year,
        authors: work.display_authors.clone().unwrap_or_default(),
        cited_by_count: prominence.and_then(|row| row.signals.cited_by_global_count),
        local_citing_count: prominence.map_or(0, |row| row.signals.citing_works_count),
        prominence: prominence.map_or(0.0, |row| row.score),
        local_paper_id: None,
        abstract_text: None,
        thesis: None,
        edge: None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use lysilogy_core::{FieldState, ProminenceSignals, Visibility, WorkKind};

    use super::*;

    fn work(title: Option<&str>) -> Work {
        let now = DateTime::<Utc>::from_timestamp(0, 0).expect("timestamp");
        Work {
            id: WorkId::new(),
            kind: WorkKind::Article,
            visibility: Visibility::Private,
            display_title: title.map(str::to_owned),
            title_state: FieldState::Known,
            display_year: Some(1990),
            year_state: FieldState::Known,
            display_authors: Some(vec!["D. Card".to_owned()]),
            authors_state: FieldState::Known,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn should_carry_the_graph_counts_into_the_record() {
        let work = work(Some("  A Natural Experiment "));
        let row = StoredProminence {
            work_id: work.id,
            signals: ProminenceSignals {
                citing_works_count: 7,
                cited_works_count: 30,
                cited_by_global_count: Some(2_500),
                display_year: Some(1990),
                local_available: false,
                mention_count: 0,
                method_citation_count: 0,
                background_citation_count: 0,
            },
            score: 0.8,
            computed_at: work.created_at,
        };
        let record = dossier_work(&work, Some(&row));
        assert_eq!(record.id, work.id.to_string());
        assert_eq!(record.title.as_deref(), Some("A Natural Experiment"));
        assert_eq!(record.cited_by_count, Some(2_500));
        assert_eq!(record.local_citing_count, 7);
        assert!((record.prominence - 0.8).abs() < f64::EPSILON);
        assert_eq!(record.authors, ["D. Card"]);
    }

    #[test]
    fn should_leave_counts_unknown_without_a_prominence_row() {
        let record = dossier_work(&work(Some("   ")), None);
        assert_eq!(record.title, None);
        assert_eq!(record.cited_by_count, None);
        assert_eq!(record.local_citing_count, 0);
    }

    #[tokio::test]
    async fn should_return_an_empty_dossier_without_a_graph() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let state = AppState::new(
            directory.path().to_path_buf(),
            directory.path().join("data"),
        )
        .await
        .expect("state");
        let id = PaperId::from_relative_path(std::path::Path::new("missing.pdf"));
        assert!(state.neighbour_dossier(&id).await.is_empty());
    }
}
