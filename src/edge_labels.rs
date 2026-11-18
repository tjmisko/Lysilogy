//! Why a paper cites each work: the labelling step after analysis, its run
//! record, and the labels the Abstract page and the review panel read.
//!
//! After a paper is analysed, [`AppState::label_paper_edges`] runs as the
//! `labels` task of the job:
//!
//! 1. find the paper's Work and document version in the citation graph (a
//!    paper the literature flow never linked is skipped, and the run says so);
//! 2. refresh its local evidence (bibliography and in-text mentions; local
//!    work, no network);
//! 3. seed a first label from the core reading's admitted relations
//!    (`lysilogy_ingest::labels::seed`), counting every reference key that
//!    does not resolve;
//! 4. run the edge labeler chosen by the provider setting on the edges with
//!    citing passages and no label, with the paper's thesis, the cited works'
//!    abstracts the library holds, and wider text around each passage;
//! 5. write `edge-labels.json` beside the analysis: what each step did and
//!    the current label of every outgoing edge.
//!
//! A failure in this step never fails the analysis; it is recorded on the
//! run and on the task. The contract is in `docs/edge-labels.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Utc};
use lysilogy_core::{ReviewState, WorkId};
use lysilogy_db::relationship::{LabelPath, StoredClaimWithMention};
use lysilogy_ingest::labels::{
    LABEL_PROMPT_VERSION, LabelBrief, LabelContext, LabelOptions, LabelSummary,
    seed::{CoreReading, CoreSeedSummary, seed_core_labels},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Error, Result,
    domain::{AnalysisProvider, PaperId},
    graph::GraphState,
    literature::evidence::LocalEvidenceTarget,
    store::write_atomic,
};

/// The run record's file name in the paper's directory.
pub const EDGE_LABELS_FILE: &str = "edge-labels.json";

/// Text added on each side of a passage when it is widened, in UTF-16 units.
const SURROUND_UNITS: usize = 450;

/// How many edges the review panel samples per paper.
pub const REVIEW_SAMPLE_SIZE: usize = 8;

/// How a labelling run ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeLabelStatus {
    /// Labels were seeded and the labeler ran (some edges may still be
    /// unlabelled; the counts say how many and why).
    Completed,
    /// Nothing could run; `detail` says why (no graph, not linked, no model).
    #[default]
    Skipped,
    /// A step failed; `detail` holds the error. Labels written before it stay.
    Failed,
}

/// One current label, as the reader and the review panel show it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeLabelBrief {
    pub claim_id: String,
    pub relation: String,
    pub reason: String,
    pub quote: Option<String>,
    /// `core_relation`, `edge_labeler` or `user`.
    pub path: Option<LabelPath>,
    /// `substantive`, `perfunctory` or `not_shown`, when the labeler said.
    pub engagement: Option<String>,
    /// `unreviewed`, `accepted`, `rejected` or `disputed`.
    pub review_state: String,
    pub model: Option<String>,
    pub prompt_version: Option<i32>,
    /// The citing passage the label's mention sits in.
    pub passage: Option<String>,
    pub page: Option<i32>,
}

impl EdgeLabelBrief {
    fn of(stored: &StoredClaimWithMention) -> Self {
        let claim = &stored.claim;
        Self {
            claim_id: claim.id.to_string(),
            relation: claim.relation.clone(),
            reason: claim.reason.clone().unwrap_or_default(),
            quote: claim.quote.clone(),
            path: Some(claim.path()),
            engagement: claim.engagement.clone(),
            review_state: claim.review_state.as_str().to_owned(),
            model: claim.model.clone(),
            prompt_version: claim.prompt_version,
            passage: stored.passage.clone(),
            page: stored.page,
        }
    }

    /// Whether a person rejected it; a rejected label is not evidence.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        self.review_state == ReviewState::Rejected.as_str()
    }
}

/// One edge of the paper (outgoing) or into it (incoming), with its label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelledEdge {
    /// The other end's Work id.
    pub work_id: String,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub first_author: Option<String>,
    /// Stored mentions of the cited work in the citing paper (outgoing only).
    pub mention_count: usize,
    pub label: Option<EdgeLabelBrief>,
}

/// Totals over a paper's outgoing edges after a run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeLabelCounts {
    pub edges: usize,
    pub labelled: usize,
    /// Edges with stored citing passages and no label.
    pub unlabelled_with_passages: usize,
    /// Edges with no stored passage and no label.
    pub unlabelled_without_passages: usize,
    pub by_path: BTreeMap<String, usize>,
    pub by_relation: BTreeMap<String, usize>,
    pub by_engagement: BTreeMap<String, usize>,
    pub perfunctory: usize,
    pub not_shown: usize,
    pub rejected: usize,
}

impl EdgeLabelCounts {
    /// Count `edges`.
    #[must_use]
    pub fn of(edges: &[LabelledEdge]) -> Self {
        let mut counts = Self {
            edges: edges.len(),
            ..Self::default()
        };
        for edge in edges {
            let Some(label) = &edge.label else {
                if edge.mention_count > 0 {
                    counts.unlabelled_with_passages += 1;
                } else {
                    counts.unlabelled_without_passages += 1;
                }
                continue;
            };
            counts.labelled += 1;
            let path = label.path.map_or("unknown", LabelPath::as_str);
            *counts.by_path.entry(path.to_owned()).or_default() += 1;
            *counts
                .by_relation
                .entry(label.relation.clone())
                .or_default() += 1;
            let engagement = label.engagement.as_deref().unwrap_or("unstated");
            *counts
                .by_engagement
                .entry(engagement.to_owned())
                .or_default() += 1;
            match engagement {
                "perfunctory" => counts.perfunctory += 1,
                "not_shown" => counts.not_shown += 1,
                _ => {}
            }
            if label.is_rejected() {
                counts.rejected += 1;
            }
        }
        counts
    }
}

/// What one labelling run did, stored as `edge-labels.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeLabelRun {
    pub generated_at: Option<DateTime<Utc>>,
    pub status: EdgeLabelStatus,
    /// Why the run was skipped or failed.
    pub detail: Option<String>,
    pub provider: Option<String>,
    /// The labeler's model key.
    pub labeler: Option<String>,
    pub prompt_version: u32,
    /// The analysis pipeline whose core relations seeded the labels.
    pub pipeline_version: Option<String>,
    /// The local evidence refresh failed; existing mentions were used.
    pub local_evidence_error: Option<String>,
    pub core: Option<CoreSeedSummary>,
    pub labels: Option<LabelSummary>,
    pub counts: EdgeLabelCounts,
    /// Every outgoing edge with its current label after the run.
    pub edges: Vec<LabelledEdge>,
}

impl EdgeLabelRun {
    fn skipped(provider: AnalysisProvider, detail: &str) -> Self {
        Self {
            generated_at: Some(Utc::now()),
            status: EdgeLabelStatus::Skipped,
            detail: Some(detail.to_owned()),
            provider: Some(provider.to_string()),
            prompt_version: LABEL_PROMPT_VERSION,
            ..Self::default()
        }
    }

    /// One line for the job queue's task detail.
    #[must_use]
    pub fn task_detail(&self) -> String {
        match self.status {
            EdgeLabelStatus::Skipped => format!(
                "Skipped: {}",
                self.detail.as_deref().unwrap_or("nothing to label")
            ),
            EdgeLabelStatus::Failed => format!(
                "Failed: {}",
                self.detail.as_deref().unwrap_or("unknown error")
            ),
            EdgeLabelStatus::Completed => {
                let counts = &self.counts;
                let core = counts.by_path.get("core_relation").copied().unwrap_or(0);
                let labeler = counts.by_path.get("edge_labeler").copied().unwrap_or(0);
                let mut parts = vec![format!(
                    "{} of {} edges labelled ({core} from the core reading, {labeler} by the edge labeler)",
                    counts.labelled, counts.edges
                )];
                if let Some(seed) = &self.core {
                    let unresolved: usize = seed.unresolved.values().sum();
                    if unresolved > 0 {
                        parts.push(format!("{unresolved} reference keys unresolved"));
                    }
                }
                if let Some(labels) = &self.labels {
                    let left = labels.over_limit + labels.over_budget + labels.failed;
                    if left > 0 {
                        parts.push(format!("{left} left for a later run"));
                    }
                    if labels.rejected_for_grounding > 0 {
                        parts.push(format!(
                            "{} answers failed grounding",
                            labels.rejected_for_grounding
                        ));
                    }
                }
                parts.join("; ")
            }
        }
    }

    /// A deterministic sample of labelled edges for the review panel: the
    /// first `size` by a hash of the claim id, so the same run always offers
    /// the same edges and the sample does not favour the top of the list.
    #[must_use]
    pub fn review_sample(&self, size: usize) -> Vec<LabelledEdge> {
        use sha2::{Digest, Sha256};
        let mut labelled: Vec<(String, &LabelledEdge)> = self
            .edges
            .iter()
            .filter_map(|edge| {
                let label = edge.label.as_ref()?;
                let digest = format!("{:x}", Sha256::digest(label.claim_id.as_bytes()));
                Some((digest, edge))
            })
            .collect();
        labelled.sort_by(|left, right| left.0.cmp(&right.0));
        labelled
            .into_iter()
            .take(size)
            .map(|(_, edge)| edge.clone())
            .collect()
    }
}

/// The labels around a paper, for the Abstract page.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PaperEdgeLabels {
    /// `ok`, `no_graph`, `not_linked` or `error`.
    pub status: &'static str,
    pub work_id: Option<String>,
    /// Why this paper cites each work it cites.
    pub outgoing: Vec<LabelledEdge>,
    /// Why later papers in the graph cite this one (labelled edges only).
    pub incoming: Vec<LabelledEdge>,
    /// The last labelling run's record, when there is one.
    pub run: Option<EdgeLabelRun>,
}

/// Wider text around a passage: one sentence before it and one after it.
///
/// The widening stays within the paragraph that holds the passage and capped at
/// [`SURROUND_UNITS`] on each side. Offsets, `sentences` and `paragraphs` are
/// UTF-16 code units of `text`; with no paragraph known the cap alone bounds
/// the widening.
#[must_use]
pub fn surrounding_text(
    text: &[u16],
    sentences: &[(usize, usize)],
    paragraphs: &[(usize, usize)],
    start: usize,
    end: usize,
) -> Option<String> {
    if start >= end || end > text.len() {
        return None;
    }
    let paragraph = paragraphs
        .iter()
        .filter(|(paragraph_start, paragraph_end)| {
            *paragraph_start <= start && *paragraph_end > start
        })
        .min_by_key(|(paragraph_start, paragraph_end)| paragraph_end - paragraph_start)
        .copied();
    let (floor, ceiling) = paragraph.map_or((0, text.len()), |(paragraph_start, paragraph_end)| {
        (paragraph_start, paragraph_end.max(end).min(text.len()))
    });
    let before = sentences
        .iter()
        .rev()
        .find(|(_, sentence_end)| *sentence_end <= start)
        .map_or(start, |(sentence_start, _)| *sentence_start)
        .max(start.saturating_sub(SURROUND_UNITS))
        .max(floor)
        .min(start);
    let after = sentences
        .iter()
        .find(|(sentence_start, _)| *sentence_start >= end)
        .map_or(end, |(_, sentence_end)| *sentence_end)
        .min(end + SURROUND_UNITS)
        .min(ceiling)
        .max(end);
    if before == start && after == end {
        return None;
    }
    Some(String::from_utf16_lossy(&text[before..after]))
}

/// The model key stored on labels seeded from a core reading by `provider`.
#[must_use]
pub fn core_model_key(provider: AnalysisProvider, model: &str) -> String {
    match provider {
        AnalysisProvider::OpenRouter => format!("openrouter:{model}"),
        other => crate::analysis::structured_model_key(other),
    }
}

impl AppState {
    /// The paper's Work in the citation graph, through redirects.
    async fn paper_work(&self, graph: &GraphState, id: &PaperId) -> Result<Option<WorkId>> {
        let Some(relative_path) = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.overview.relative_path.clone())
        else {
            return Err(Error::PaperNotFound(id.to_string()));
        };
        let found = async {
            let workspace = graph.documents.ensure_default_workspace().await?;
            let Some(document) = graph
                .documents
                .find_by_path(workspace, &relative_path)
                .await?
            else {
                return Ok(None);
            };
            let Some(link) = graph.documents.work_link(document.id).await? else {
                return Ok(None);
            };
            graph.works.resolve_redirect(link.work_id).await.map(Some)
        }
        .await;
        found.map_err(|error: lysilogy_db::DbError| {
            Error::Literature(format!("could not read the citation graph: {error}"))
        })
    }

    /// Label why the paper cites each work, after its analysis. Never fails:
    /// a step that fails is recorded on the run, which is also saved.
    pub async fn label_paper_edges(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
    ) -> EdgeLabelRun {
        let run = match self.label_paper_edges_inner(id, provider).await {
            Ok(run) => run,
            Err(error) => {
                tracing::warn!(paper_id = %id, %error, "edge labelling failed");
                EdgeLabelRun {
                    status: EdgeLabelStatus::Failed,
                    detail: Some(error.to_string()),
                    ..EdgeLabelRun::skipped(provider, "")
                }
            }
        };
        if let Err(error) = self.save_edge_label_run(id, &run).await {
            tracing::warn!(paper_id = %id, %error, "could not save the edge label run");
        }
        run
    }

    async fn label_paper_edges_inner(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
    ) -> Result<EdgeLabelRun> {
        let Some(graph) = &self.graph else {
            return Ok(EdgeLabelRun::skipped(
                provider,
                "no citation graph is configured (set LYSILOGY_DATABASE_URL)",
            ));
        };
        let settings = self.analysis.settings().clone();
        let Some(labeler) = crate::labeling::labeler_for(
            provider,
            &settings,
            self.analysis.local_cli(),
            self.store.root().join("labels"),
        )?
        else {
            return Ok(EdgeLabelRun::skipped(
                provider,
                "the offline reader has no model to label citations with",
            ));
        };
        let Some(target) = self.linked_evidence_target(id).await? else {
            return Ok(EdgeLabelRun::skipped(
                provider,
                "this paper is not linked to the citation graph yet; run the literature flow first",
            ));
        };
        let mut run = EdgeLabelRun {
            generated_at: Some(Utc::now()),
            status: EdgeLabelStatus::Completed,
            provider: Some(provider.to_string()),
            labeler: Some(labeler.model_key().to_owned()),
            prompt_version: LABEL_PROMPT_VERSION,
            ..EdgeLabelRun::default()
        };
        if let Err(error) = Box::pin(self.run_local_evidence_at(id, target.clone())).await {
            tracing::warn!(paper_id = %id, %error, "local evidence refresh failed before labelling");
            run.local_evidence_error = Some(error.to_string());
        }
        let context = LabelContext {
            works: &graph.works,
            records: &graph.source_records,
            evidence: &graph.evidence,
            relationships: &graph.relationships,
        };
        let (reading, malformed, pipeline_version) =
            self.core_reading(id, provider, &settings.model).await;
        run.pipeline_version = pipeline_version;
        let seeded = seed_core_labels(
            &context,
            target.citing_work,
            target.document_version,
            &reading,
            malformed,
        )
        .await?;
        run.core = Some(seeded.summary);
        let brief = Box::pin(self.label_brief(graph, id, &target, seeded.extra_passages)).await;
        let options = LabelOptions {
            limit: settings.label_limit,
            ..LabelOptions::default()
        };
        run.labels = Some(
            lysilogy_ingest::labels::label_edges(
                &context,
                target.citing_work,
                target.document_version,
                labeler.as_ref(),
                &brief,
                options,
            )
            .await?,
        );
        run.edges = self.outgoing_edges(graph, &target).await?;
        run.counts = EdgeLabelCounts::of(&run.edges);
        Ok(run)
    }

    /// The core reading of the last analysis: its admitted relations,
    /// citations and references. A paper analysed before the record existed
    /// has none, and seeding then does nothing.
    async fn core_reading(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
        model: &str,
    ) -> (CoreReading, usize, Option<String>) {
        let path = self.store.paper_dir(id).join("paper-artifacts.json");
        let record = tokio::fs::read(&path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let Some(record) = record else {
            return (CoreReading::default(), 0, None);
        };
        let pipeline_version = record
            .get("pipeline_version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let run_key = format!(
            "{id}|{}",
            pipeline_version.as_deref().unwrap_or("unversioned")
        );
        let artifacts = record
            .get("artifacts")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let (reading, malformed) =
            CoreReading::from_artifacts(&artifacts, &run_key, &core_model_key(provider, model));
        (reading, malformed, pipeline_version)
    }

    /// What the labeler is told beyond the graph: the paper's thesis, the
    /// cited works' abstracts the library holds, wider text around each
    /// passage, and the core reading's citing sentences.
    pub(crate) async fn label_brief(
        &self,
        graph: &GraphState,
        id: &PaperId,
        target: &LocalEvidenceTarget,
        extra_passages: HashMap<WorkId, Vec<String>>,
    ) -> LabelBrief {
        let citing_thesis = self
            .store
            .load_analysis(id)
            .await
            .ok()
            .flatten()
            .map(|analysis| analysis.thesis)
            .filter(|thesis| !thesis.trim().is_empty());
        let mut cited_abstracts = HashMap::new();
        if let Ok(edges) = graph
            .evidence
            .local_edges(target.citing_work, target.document_version)
            .await
        {
            let cited: Vec<WorkId> = edges.iter().map(|(work, _, _)| *work).collect();
            if let Ok(held) = self.held_papers_with_titles(graph, &cited).await {
                for (work, papers) in held {
                    for paper in papers {
                        if let Ok(Some(analysis)) = self.store.load_analysis(&paper.paper_id).await
                            && let Some(text) = analysis
                                .author_abstract
                                .filter(|text| !text.trim().is_empty())
                        {
                            cited_abstracts.insert(work, text);
                            break;
                        }
                    }
                }
            }
        }
        let surround = match self.reading_index(id, false).await {
            Ok(index) => {
                let text: Vec<u16> = index.text.encode_utf16().collect();
                let sentences: Vec<(usize, usize)> = index
                    .objects
                    .sentence
                    .iter()
                    .map(|sentence| (sentence.start, sentence.end))
                    .collect();
                let paragraphs: Vec<(usize, usize)> = index
                    .objects
                    .paragraph
                    .iter()
                    .filter(|paragraph| paragraph.kind != "heading")
                    .map(|paragraph| (paragraph.start, paragraph.end))
                    .collect();
                let surround: Box<lysilogy_ingest::labels::Surround> = Box::new(move |mention| {
                    let start = usize::try_from(mention.passage_start?).ok()?;
                    let end = usize::try_from(mention.passage_end?).ok()?;
                    surrounding_text(&text, &sentences, &paragraphs, start, end)
                });
                Some(surround)
            }
            Err(error) => {
                tracing::info!(paper_id = %id, %error, "no reading index; passages are not widened");
                None
            }
        };
        LabelBrief {
            citing_thesis,
            cited_abstracts,
            surround,
            extra_passages,
        }
    }

    /// Every outgoing local edge with its current label.
    async fn outgoing_edges(
        &self,
        graph: &GraphState,
        target: &LocalEvidenceTarget,
    ) -> Result<Vec<LabelledEdge>> {
        let database = |error: lysilogy_db::DbError| {
            Error::Literature(format!("could not read the citation graph: {error}"))
        };
        let edges = graph
            .evidence
            .local_edges(target.citing_work, target.document_version)
            .await
            .map_err(database)?;
        let pairs: Vec<(WorkId, WorkId)> = edges
            .iter()
            .map(|(cited, _, _)| (target.citing_work, *cited))
            .collect();
        let labels = self.labels_by_pair(graph, &pairs).await?;
        let cited: Vec<WorkId> = edges.iter().map(|(work, _, _)| *work).collect();
        let works: HashMap<WorkId, lysilogy_core::Work> = graph
            .works
            .get_many(&cited)
            .await
            .map_err(database)?
            .into_iter()
            .map(|work| (work.id, work))
            .collect();
        let mut seen = std::collections::HashSet::new();
        Ok(edges
            .into_iter()
            .filter(|(cited, _, _)| seen.insert(*cited))
            .map(|(cited, _, mentions)| {
                let work = works.get(&cited);
                LabelledEdge {
                    work_id: cited.to_string(),
                    title: work.and_then(|work| work.display_title.clone()),
                    year: work.and_then(|work| work.display_year),
                    first_author: work
                        .and_then(|work| work.display_authors.as_ref())
                        .and_then(|authors| authors.first().cloned()),
                    mention_count: mentions.len(),
                    label: labels.get(&(target.citing_work, cited)).cloned(),
                }
            })
            .collect())
    }

    /// The newest current label on each edge.
    async fn labels_by_pair(
        &self,
        graph: &GraphState,
        pairs: &[(WorkId, WorkId)],
    ) -> Result<HashMap<(WorkId, WorkId), EdgeLabelBrief>> {
        let claims = graph
            .relationships
            .current_claims_for_edges(pairs)
            .await
            .map_err(|error| {
                Error::Literature(format!("could not read the citation graph: {error}"))
            })?;
        let mut labels = HashMap::new();
        for claim in &claims {
            // Ordered by creation time, so the last one written wins.
            labels.insert((claim.citing, claim.cited), EdgeLabelBrief::of(claim));
        }
        Ok(labels)
    }

    /// The labels around a paper: why it cites each work, and why later
    /// labelled papers cite it.
    pub async fn paper_edge_labels(&self, id: &PaperId) -> Result<PaperEdgeLabels> {
        let run = self.load_edge_label_run(id).await?;
        let Some(graph) = &self.graph else {
            return Ok(PaperEdgeLabels {
                status: "no_graph",
                run,
                ..PaperEdgeLabels::default()
            });
        };
        let Some(center) = self.paper_work(graph, id).await? else {
            return Ok(PaperEdgeLabels {
                status: "not_linked",
                run,
                ..PaperEdgeLabels::default()
            });
        };
        let database = |error: lysilogy_db::DbError| {
            Error::Literature(format!("could not read the citation graph: {error}"))
        };
        let outgoing_works = graph.citations.outgoing(center).await.map_err(database)?;
        let incoming_works = graph.citations.incoming(center).await.map_err(database)?;
        let mut pairs: Vec<(WorkId, WorkId)> = outgoing_works
            .iter()
            .map(|work| (center, work.id))
            .collect();
        pairs.extend(incoming_works.iter().map(|work| (work.id, center)));
        let labels = self.labels_by_pair(graph, &pairs).await?;
        let edge = |work: &lysilogy_core::Work, label: Option<EdgeLabelBrief>| LabelledEdge {
            work_id: work.id.to_string(),
            title: work.display_title.clone(),
            year: work.display_year,
            first_author: work
                .display_authors
                .as_ref()
                .and_then(|authors| authors.first().cloned()),
            mention_count: 0,
            label,
        };
        let outgoing = outgoing_works
            .iter()
            .filter(|work| work.id != center)
            .map(|work| edge(work, labels.get(&(center, work.id)).cloned()))
            .collect();
        let incoming = incoming_works
            .iter()
            .filter(|work| work.id != center)
            .filter_map(|work| {
                let label = labels.get(&(work.id, center)).cloned()?;
                Some(edge(work, Some(label)))
            })
            .collect();
        Ok(PaperEdgeLabels {
            status: "ok",
            work_id: Some(center.to_string()),
            outgoing,
            incoming,
            run,
        })
    }

    /// The last labelling run's record, if any.
    pub async fn load_edge_label_run(&self, id: &PaperId) -> Result<Option<EdgeLabelRun>> {
        load_run(&self.store.paper_dir(id)).await
    }

    async fn save_edge_label_run(&self, id: &PaperId, run: &EdgeLabelRun) -> Result<()> {
        let directory = self.store.paper_dir(id);
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| Error::io(&directory, error))?;
        let mut bytes = serde_json::to_vec_pretty(run)?;
        bytes.push(b'\n');
        write_atomic(&directory.join(EDGE_LABELS_FILE), &bytes).await
    }
}

/// Read `edge-labels.json` from a paper directory; a missing file is `None`.
///
/// # Errors
/// When the file exists but cannot be read or parsed.
pub async fn load_run(paper_dir: &Path) -> Result<Option<EdgeLabelRun>> {
    let path = paper_dir.join(EDGE_LABELS_FILE);
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(&path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(
        id: &str,
        relation: &str,
        engagement: Option<&str>,
        path: LabelPath,
    ) -> EdgeLabelBrief {
        EdgeLabelBrief {
            claim_id: id.to_owned(),
            relation: relation.to_owned(),
            reason: "Uses their index.".to_owned(),
            quote: Some("we use their index".to_owned()),
            path: Some(path),
            engagement: engagement.map(str::to_owned),
            review_state: "unreviewed".to_owned(),
            ..EdgeLabelBrief::default()
        }
    }

    fn edge(work: &str, mentions: usize, label: Option<EdgeLabelBrief>) -> LabelledEdge {
        LabelledEdge {
            work_id: work.to_owned(),
            mention_count: mentions,
            label,
            ..LabelledEdge::default()
        }
    }

    #[test]
    fn should_widen_a_passage_by_one_sentence_each_side() {
        let text = "First one. The citing sentence [1]. After it. Far away.";
        let units: Vec<u16> = text.encode_utf16().collect();
        let sentences = vec![(0, 10), (11, 35), (36, 45), (46, 55)];
        let wide = surrounding_text(&units, &sentences, &[], 11, 35).expect("wider");
        assert_eq!(wide, "First one. The citing sentence [1]. After it.");
        assert_eq!(
            surrounding_text(&units, &[], &[], 11, 35),
            None,
            "nothing to add"
        );
        assert_eq!(surrounding_text(&units, &sentences, &[], 35, 11), None);
        assert_eq!(surrounding_text(&units, &sentences, &[], 11, 999), None);
        let paragraph = [(11, 45)];
        assert_eq!(
            surrounding_text(&units, &sentences, &paragraph, 11, 35).as_deref(),
            Some("The citing sentence [1]. After it."),
            "the widening stays inside the passage's paragraph"
        );
        assert_eq!(
            surrounding_text(&units, &sentences, &[(11, 35)], 11, 35),
            None
        );
    }

    #[test]
    fn should_cap_the_widening_when_sentences_are_long() {
        let long = "x".repeat(2_000);
        let text = format!("{long}. Cited [1]. {long}.");
        let units: Vec<u16> = text.encode_utf16().collect();
        let start = long.len() + 2;
        let end = start + 10;
        let sentences = vec![(0, long.len() + 1), (start, end), (end + 1, units.len())];
        let wide = surrounding_text(&units, &sentences, &[], start, end).expect("wider");
        assert!(wide.contains("Cited [1]."));
        assert!(wide.encode_utf16().count() <= 10 + 2 * SURROUND_UNITS);
    }

    #[test]
    fn should_count_labels_by_path_engagement_and_missing_passages() {
        let mut rejected = label(
            "r3",
            "background",
            Some("perfunctory"),
            LabelPath::CoreRelation,
        );
        rejected.review_state = "rejected".to_owned();
        let edges = vec![
            edge(
                "w1",
                2,
                Some(label(
                    "r1",
                    "uses_method",
                    Some("substantive"),
                    LabelPath::EdgeLabeler,
                )),
            ),
            edge(
                "w2",
                1,
                Some(label(
                    "r2",
                    "other",
                    Some("not_shown"),
                    LabelPath::EdgeLabeler,
                )),
            ),
            edge("w3", 0, Some(rejected)),
            edge("w4", 3, None),
            edge("w5", 0, None),
        ];
        let counts = EdgeLabelCounts::of(&edges);
        assert_eq!((counts.edges, counts.labelled), (5, 3));
        assert_eq!(
            (
                counts.unlabelled_with_passages,
                counts.unlabelled_without_passages
            ),
            (1, 1)
        );
        assert_eq!(counts.by_path.get("edge_labeler"), Some(&2));
        assert_eq!(counts.by_path.get("core_relation"), Some(&1));
        assert_eq!(
            (counts.perfunctory, counts.not_shown, counts.rejected),
            (1, 1, 1)
        );
    }

    #[test]
    fn should_sample_the_same_labelled_edges_for_review_every_time() {
        let edges: Vec<LabelledEdge> = (0..20)
            .map(|index| {
                let labelled = index % 3 != 0;
                edge(
                    &format!("w{index}"),
                    1,
                    labelled.then(|| {
                        label(
                            &format!("r{index}"),
                            "extends",
                            None,
                            LabelPath::EdgeLabeler,
                        )
                    }),
                )
            })
            .collect();
        let run = EdgeLabelRun {
            edges,
            ..EdgeLabelRun::default()
        };
        let sample = run.review_sample(REVIEW_SAMPLE_SIZE);
        assert_eq!(sample.len(), REVIEW_SAMPLE_SIZE);
        assert!(sample.iter().all(|edge| edge.label.is_some()));
        assert_eq!(sample, run.review_sample(REVIEW_SAMPLE_SIZE));
        assert_ne!(
            sample
                .iter()
                .map(|edge| edge.work_id.as_str())
                .collect::<Vec<_>>(),
            ["w1", "w2", "w4", "w5", "w7", "w8", "w10", "w11"],
            "not simply the first labelled edges"
        );
    }

    #[test]
    fn should_describe_the_run_in_one_task_line() {
        let mut run = EdgeLabelRun::skipped(AnalysisProvider::OpenRouter, "no graph");
        assert_eq!(run.task_detail(), "Skipped: no graph");
        run.status = EdgeLabelStatus::Completed;
        run.edges = vec![
            edge(
                "w1",
                1,
                Some(label("r1", "extends", None, LabelPath::CoreRelation)),
            ),
            edge("w2", 1, None),
        ];
        run.counts = EdgeLabelCounts::of(&run.edges);
        run.core = Some(CoreSeedSummary {
            unresolved: BTreeMap::from([("no_matching_entry".to_owned(), 2)]),
            ..CoreSeedSummary::default()
        });
        run.labels = Some(LabelSummary {
            over_budget: 1,
            ..LabelSummary::default()
        });
        let detail = run.task_detail();
        assert!(
            detail.starts_with(
                "1 of 2 edges labelled (1 from the core reading, 0 by the edge labeler)"
            ),
            "{detail}"
        );
        assert!(detail.contains("2 reference keys unresolved"), "{detail}");
        assert!(detail.contains("1 left for a later run"), "{detail}");
    }

    #[tokio::test]
    async fn should_skip_and_record_why_without_a_graph() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        tokio::fs::write(library.path().join("paper.pdf"), b"pdf")
            .await
            .unwrap();
        let state = AppState::new(library.path(), data.path()).await.unwrap();
        let id = PaperId::from_relative_path(Path::new("paper.pdf"));
        let run = state
            .label_paper_edges(&id, AnalysisProvider::OpenRouter)
            .await;
        assert_eq!(run.status, EdgeLabelStatus::Skipped);
        assert!(
            run.detail
                .as_deref()
                .unwrap_or_default()
                .contains("no citation graph")
        );
        let saved = state
            .load_edge_label_run(&id)
            .await
            .unwrap()
            .expect("saved");
        assert_eq!(saved.status, EdgeLabelStatus::Skipped);
        let labels = state.paper_edge_labels(&id).await.unwrap();
        assert_eq!(labels.status, "no_graph");
        assert!(labels.run.is_some());
    }

    #[test]
    fn should_load_a_run_written_with_fewer_fields() {
        let run: EdgeLabelRun =
            serde_json::from_str(r#"{"status":"completed","edges":[{"work_id":"w1"}]}"#)
                .expect("older run");
        assert_eq!(run.status, EdgeLabelStatus::Completed);
        assert!(run.edges[0].label.is_none());
    }
}
