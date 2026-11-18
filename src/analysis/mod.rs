pub mod context;
mod heuristic;
mod local_cli;
pub mod openrouter;
mod prefetch;
mod sectioning;
mod sources;
pub mod spend;

use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    config::AnalysisConfig,
    domain::{
        AgentSession, AnalysisProvider, Claim, Clarification, ContextBasis, ContextKind,
        ContextNote, ContextSource, EvidenceStrength, ExtractedPaper, GlossaryEntry, KeyQuote,
        LearningRamp, OutlineEntry, OutlineRole, PageSpan, PaperAnalysis, PaperContext,
        PaperSection, PromptExperiment, PromptVariant, QualityWarning, QuoteSignificance,
        SectionFamily, SectionKind, SectionSourceSpan,
    },
    error::Error,
    layout::verify_quote,
    paper_artifacts::dossier::Dossier,
};

// Deprecated: the offline reader is hidden from the UI. Kept as an internal
// deterministic fallback and test helper.
pub use heuristic::HeuristicAnalyzer;
pub use local_cli::LocalCliAnalyzer;
pub(crate) use local_cli::{structured_fast_model_key, structured_model_key};

pub const ANALYSIS_SCHEMA: &str = include_str!("../../prompts/paper-analysis.schema.json");
pub const CLARIFICATION_SCHEMA: &str = include_str!("../../prompts/clarification.schema.json");
pub const LEARNING_RAMP_SCHEMA: &str = include_str!("../../prompts/learning-ramp.schema.json");

#[derive(Clone, Debug, Default)]
pub struct AnalysisService {
    local_cli: LocalCliAnalyzer,
    settings: AnalysisConfig,
}

#[derive(Debug)]
pub struct AnalysisOutcome {
    pub analysis: PaperAnalysis,
    pub session: Option<AgentSession>,
    /// Deterministic plausibility warnings about the model output; also
    /// stored on the analysis.
    pub warnings: Vec<QualityWarning>,
    /// The analysis is usable but not a clean success: a model call failed
    /// or a verification pass did not run. Show it, flagged.
    pub degraded: bool,
}

/// A refreshed context and what the run noticed.
#[derive(Debug)]
pub struct ContextOutcome {
    pub context: PaperContext,
    pub warnings: Vec<QualityWarning>,
    pub degraded: bool,
    pub pipeline_version: String,
}

pub(crate) struct ExperimentVariantRequest<'a> {
    pub experiment: &'a PromptExperiment,
    pub variant: &'a PromptVariant,
    pub reader_baseline: &'a [String],
    pub run_id: &'a str,
    pub blind_label: &'a str,
}

pub(crate) struct ReaderToolRequest<'a> {
    pub schema: &'a str,
    pub prompt: &'a str,
    pub live_web: bool,
    /// Run on the cheap model (Haiku for Claude, the codex fast model).
    pub fast: bool,
    /// The substage this call belongs to, for the spend ledger.
    pub purpose: &'a str,
}

impl AnalysisService {
    pub(crate) async fn reader_tool<T: serde::de::DeserializeOwned>(
        &self,
        provider: AnalysisProvider,
        directory: &Path,
        request: ReaderToolRequest<'_>,
    ) -> Result<T> {
        self.local_cli
            .reader_tool(provider, directory, request)
            .await
    }

    #[must_use]
    pub fn new(local_cli: LocalCliAnalyzer) -> Self {
        Self {
            local_cli,
            settings: AnalysisConfig::default(),
        }
    }

    /// Use these model, provider and budget settings for the core pipeline.
    #[must_use]
    pub fn with_settings(mut self, settings: AnalysisConfig) -> Self {
        self.settings = settings;
        self
    }

    /// The model, provider and budget settings the core pipeline runs with.
    #[must_use]
    pub const fn settings(&self) -> &AnalysisConfig {
        &self.settings
    }

    pub async fn extract_abstract(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        directory: &Path,
        force: bool,
    ) -> Result<crate::abstracts::AbstractResult> {
        self.local_cli
            .extract_abstract(provider, paper, directory, force)
            .await
    }

    pub async fn analyze_heuristic(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        artifact_directory: &Path,
        reset_stages: bool,
    ) -> Result<AnalysisOutcome> {
        let abstract_result = self
            .local_cli
            .extract_abstract(provider, paper, artifact_directory, reset_stages)
            .await?;
        let draft = HeuristicAnalyzer::analyze(paper);
        let mut analysis = normalize_analysis(draft, provider, paper)?;
        analysis.author_abstract.clone_from(&abstract_result.text);
        analysis.abstract_extraction = Some(abstract_result);
        sources::verify_context_sources(&mut analysis).await;
        if let Some(assessment) = &analysis.context_assessment {
            crate::store::write_atomic(
                &artifact_directory.join("context-assessment.json"),
                &serde_json::to_vec_pretty(assessment)?,
            )
            .await?;
        }
        Ok(AnalysisOutcome {
            analysis,
            session: None,
            warnings: Vec::new(),
            degraded: false,
        })
    }

    /// Run the spike's core `paper-artifacts` extraction and map it onto the
    /// reader model. This is the new backend for `analyze` on model readers.
    ///
    /// The abstract is taken from the extraction, falling back to the
    /// deterministic abstract extraction when the artifact's is absent. The
    /// context node runs after the core with `dossier`, which may be empty.
    pub async fn analyze_core_artifacts(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        pdf: &Path,
        artifact_directory: &Path,
        dossier: Dossier,
        progress: Option<openrouter::ProgressSink>,
    ) -> Result<AnalysisOutcome> {
        use crate::paper_artifacts::{
            features::Features,
            resources::DEFAULT_DPI,
            run::{CoreMode, ExtractOptions, core_pipeline, extract_with},
        };
        let checkpoint_directory = artifact_directory.join("checkpoints");
        let features = Features::parse("context")?;
        let settings = &self.settings;
        let options = ExtractOptions {
            pdf: pdf.to_path_buf(),
            model: settings.model.clone(),
            judgment_model: settings.judgment_model.clone(),
            passes: 2,
            dpi: DEFAULT_DPI,
            work_directory: artifact_directory.join("paper-artifacts"),
            output: artifact_directory.join("paper-artifacts.json"),
            paper_budget_usd: settings.paper_budget_usd,
            timeout: std::time::Duration::from_secs(20 * 60),
            features,
            object_pages: 6,
            reasoning: openrouter::ReasoningEffort::Low,
            core_mode: CoreMode::Combined,
            providers: settings.providers.clone(),
            plan: Some(core_pipeline(features)),
            progress,
            checkpoints: Some(checkpoint_directory.clone()),
            dossier,
            seed: None,
        };
        let ledger = spend::SpendLedger::with_defaults();
        let backend = self.backend(provider, &options, &ledger)?;
        let record = extract_with(&options, backend).await?;
        let clean = record.succeeded();
        let degraded = record.degraded();
        if degraded {
            tracing::warn!(
                pdf = %pdf.display(),
                failures = record.failures.len(),
                verification = ?record.verification,
                "the core extraction finished degraded"
            );
        }
        let warnings = record.warnings.clone();
        let pipeline_version = record.pipeline_version.clone();
        let reason = record.failures.first().map_or_else(
            || "the extraction produced no artifacts".to_owned(),
            |failure| failure.error.clone(),
        );
        let artifacts = record.artifacts.ok_or(Error::InvalidAnalysis(reason))?;
        let draft = draft_from_artifacts(&artifacts)?;
        let mut analysis = normalize_analysis(draft, provider, paper)?;
        if analysis.author_abstract.is_none() {
            analysis.author_abstract = analysis
                .abstract_extraction
                .as_ref()
                .and_then(|extraction| extraction.text.clone());
        }
        analysis.pipeline_version = Some(pipeline_version);
        analysis.quality_warnings.clone_from(&warnings);
        analysis.abstract_agreement = record.abstract_agreement;
        // A finished run must not be replayed by the next one: a refresh asks
        // the model again. Keep the saved calls when any call failed or a
        // verification pass was skipped, so the retry pays only for what is
        // missing.
        if clean {
            crate::paper_artifacts::checkpoint::clear(&checkpoint_directory).await?;
        }
        Ok(AnalysisOutcome {
            analysis,
            session: None,
            warnings,
            degraded,
        })
    }

    /// Where a structured model call of the paper-artifacts pipeline runs.
    fn backend<'a>(
        &'a self,
        provider: AnalysisProvider,
        options: &crate::paper_artifacts::run::ExtractOptions,
        ledger: &'a spend::SpendLedger,
    ) -> Result<crate::paper_artifacts::run::Backend<'a>> {
        use crate::paper_artifacts::run::Backend;
        Ok(if provider == AnalysisProvider::OpenRouter {
            Backend::OpenRouter {
                client: openrouter::client(options.timeout)?,
                ledger,
            }
        } else {
            Backend::Plan {
                analyzer: &self.local_cli,
                provider,
                directory: options.work_directory.clone(),
            }
        })
    }

    /// Run the context node alone over the core fields of the last full run,
    /// and return the new context. The core fields come from the stored
    /// `paper-artifacts.json` record; an analysis made before that record
    /// existed contributes its thesis and abstract instead.
    pub async fn refresh_context(
        &self,
        provider: AnalysisProvider,
        previous: &PaperAnalysis,
        pdf: &Path,
        artifact_directory: &Path,
        dossier: Dossier,
        progress: Option<openrouter::ProgressSink>,
    ) -> Result<ContextOutcome> {
        use crate::paper_artifacts::{
            features::Features,
            resources::DEFAULT_DPI,
            run::{CoreMode, ExtractOptions, context_pipeline, extract_with},
        };
        let seed = tokio::fs::read(artifact_directory.join("paper-artifacts.json"))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|record| record.get("artifacts").cloned())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| {
            serde_json::json!({
                "card": {"thesis": {"text": previous.thesis, "evidence": [], "confidence": "medium"}},
            })
        });
        let settings = &self.settings;
        let options = ExtractOptions {
            pdf: pdf.to_path_buf(),
            model: settings.model.clone(),
            judgment_model: settings.judgment_model.clone(),
            passes: 1,
            dpi: DEFAULT_DPI,
            work_directory: artifact_directory.join("paper-artifacts"),
            output: artifact_directory.join("paper-artifacts-context.json"),
            paper_budget_usd: settings.paper_budget_usd,
            timeout: std::time::Duration::from_secs(20 * 60),
            features: Features::parse("context")?,
            object_pages: 6,
            reasoning: openrouter::ReasoningEffort::Low,
            core_mode: CoreMode::Combined,
            providers: settings.providers.clone(),
            plan: Some(context_pipeline()),
            progress,
            checkpoints: None,
            dossier,
            seed: Some(seed),
        };
        let ledger = spend::SpendLedger::with_defaults();
        let backend = self.backend(provider, &options, &ledger)?;
        let record = extract_with(&options, backend).await?;
        let degraded = !record.succeeded();
        let context = record
            .artifacts
            .as_ref()
            .and_then(context_from_artifacts)
            .ok_or_else(|| {
                Error::InvalidAnalysis(record.failures.first().map_or_else(
                    || "the context node returned no context".to_owned(),
                    |failure| failure.error.clone(),
                ))
            })?;
        Ok(ContextOutcome {
            context,
            warnings: record.warnings,
            degraded,
            pipeline_version: record.pipeline_version,
        })
    }

    pub async fn revise(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        artifact_directory: &Path,
        feedback: &str,
        session: Option<&AgentSession>,
    ) -> Result<AnalysisOutcome> {
        if provider == AnalysisProvider::Heuristic {
            return Err(Error::InvalidRequest(
                "feedback retries require the Codex or Claude reader".to_owned(),
            ));
        }
        let result = self
            .local_cli
            .revise(provider, paper, artifact_directory, feedback, session)
            .await?;
        let mut analysis = normalize_analysis(result.draft, provider, paper)?;
        let abstract_result = self
            .local_cli
            .extract_abstract(provider, paper, artifact_directory, false)
            .await?;
        analysis.author_abstract.clone_from(&abstract_result.text);
        analysis.abstract_extraction = Some(abstract_result);
        sources::verify_context_sources(&mut analysis).await;
        if let Some(assessment) = &analysis.context_assessment {
            crate::store::write_atomic(
                &artifact_directory.join("context-assessment.json"),
                &serde_json::to_vec_pretty(assessment)?,
            )
            .await?;
        }
        Ok(AnalysisOutcome {
            analysis,
            session: result.session,
            warnings: Vec::new(),
            degraded: false,
        })
    }

    pub async fn clarify(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        analysis: Option<&PaperAnalysis>,
        artifact_directory: &Path,
        selection: &str,
        question: &str,
    ) -> Result<Clarification> {
        if selection.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "choose a passage before asking for clarification".to_owned(),
            ));
        }
        if selection.chars().count() > 8_000 {
            return Err(Error::InvalidRequest(
                "the selected passage is longer than 8,000 characters".to_owned(),
            ));
        }

        match provider {
            AnalysisProvider::Heuristic => Ok(HeuristicAnalyzer::clarify(
                analysis,
                selection.trim(),
                question.trim(),
            )),
            AnalysisProvider::Codex | AnalysisProvider::Claude | AnalysisProvider::OpenRouter => {
                let draft = self
                    .local_cli
                    .clarify(
                        provider,
                        paper,
                        artifact_directory,
                        selection.trim(),
                        question.trim(),
                    )
                    .await?;
                let concepts =
                    analysis.map_or_else(Vec::new, |a| matching_glossary(&a.glossary, selection));
                Ok(Clarification {
                    selection: selection.trim().to_owned(),
                    answer: draft.answer,
                    concepts,
                    connections: draft.connections,
                    limitation: draft.limitation,
                    provider,
                })
            }
        }
    }

    pub(crate) async fn experiment_variant(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        artifact_directory: &Path,
        request: ExperimentVariantRequest<'_>,
    ) -> Result<LearningRamp> {
        if provider == AnalysisProvider::Heuristic {
            return Err(Error::InvalidRequest(
                "prompt experiments require the Codex or Claude reader".to_owned(),
            ));
        }
        let ramp = self
            .local_cli
            .experiment_variant(provider, paper, artifact_directory, request)
            .await?;
        normalize_learning_ramp(ramp, paper)
    }
}

pub(crate) fn normalize_learning_ramp(
    mut ramp: LearningRamp,
    paper: &ExtractedPaper,
) -> Result<LearningRamp> {
    ramp.foothold.question = clean_required("experiment question", &ramp.foothold.question)?;
    ramp.foothold.answer = clean_required("experiment answer", &ramp.foothold.answer)?;
    ramp.foothold.why_care = clean_required("experiment importance", &ramp.foothold.why_care)?;
    ramp.foothold.mechanism = clean_required("experiment mechanism", &ramp.foothold.mechanism)?;
    for concept in &mut ramp.concepts {
        concept.term = compact_whitespace(&concept.term);
        concept.plain_language = compact_whitespace(&concept.plain_language);
        concept.technical_definition = compact_whitespace(&concept.technical_definition);
        concept.why_now = compact_whitespace(&concept.why_now);
        concept.bridge = concept
            .bridge
            .take()
            .map(|value| compact_whitespace(&value));
        concept.bridge_limit = concept
            .bridge_limit
            .take()
            .map(|value| compact_whitespace(&value));
        if concept.bridge_limit.as_ref().is_none_or(String::is_empty) {
            concept.bridge = None;
            concept.bridge_limit = None;
        }
    }
    ramp.concepts.retain(|concept| {
        !concept.term.is_empty()
            && !concept.plain_language.is_empty()
            && !concept.technical_definition.is_empty()
    });
    ramp.concepts.truncate(10);

    let mut seen_passages = std::collections::HashSet::new();
    ramp.essential_passages.retain_mut(|passage| {
        let (status, anchor) = verify_quote(&paper.layout, &passage.quote, passage.page);
        let Some(anchor) = anchor else {
            return false;
        };
        if !matches!(
            status,
            crate::domain::CitationStatus::Exact | crate::domain::CitationStatus::Normalized
        ) {
            return false;
        }
        passage.quote = anchor.exact_text;
        passage.page = anchor.page;
        passage.read_for = compact_whitespace(&passage.read_for);
        passage.context_before = clean_list(std::mem::take(&mut passage.context_before), 4);
        seen_passages.insert((anchor.page, anchor.start_token, anchor.end_token))
    });
    ramp.essential_passages.truncate(10);
    if ramp.essential_passages.is_empty() {
        return Err(Error::InvalidAnalysis(
            "learning-ramp experiment returned no verifiable essential passage".to_owned(),
        ));
    }

    for item in &mut ramp.reception {
        item.claim = compact_whitespace(&item.claim);
        item.source_urls.retain(|url| public_http_url(url));
        item.source_urls.truncate(4);
    }
    ramp.reception.retain(|item| !item.claim.is_empty());
    ramp.reception.truncate(6);
    for item in &mut ramp.counterarguments {
        item.camp = item.camp.take().map(|value| compact_whitespace(&value));
        item.objection = compact_whitespace(&item.objection);
        item.likely_reply = compact_whitespace(&item.likely_reply);
        item.source_urls.retain(|url| public_http_url(url));
        item.source_urls.truncate(4);
    }
    ramp.counterarguments
        .retain(|item| !item.objection.is_empty() && !item.likely_reply.is_empty());
    ramp.counterarguments.truncate(5);
    ramp.uncertainties = clean_list(ramp.uncertainties, 8);
    Ok(ramp)
}

fn public_http_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AnalysisDraft {
    pub thesis: String,
    #[serde(default)]
    pub outsider_brief: String,
    #[serde(default)]
    pub author_abstract: Option<String>,
    #[serde(default)]
    pub context_notes: Vec<ContextNoteDraft>,
    #[serde(default)]
    pub context_sources: Vec<ContextSourceDraft>,
    #[serde(default)]
    pub assessment: Option<context::ContextAssessment>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    /// The printed headings, admitted against the page text upstream.
    #[serde(default)]
    pub outline: Vec<OutlineEntryDraft>,
    pub sections: Vec<SectionDraft>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub glossary: Vec<GlossaryEntry>,
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub reading_path: Vec<String>,
    /// Why the paper matters, from the context node.
    #[serde(default)]
    pub context: Option<PaperContext>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ExternalContextDraft {
    #[serde(default)]
    pub context_notes: Vec<ContextNoteDraft>,
    #[serde(default)]
    pub context_sources: Vec<ContextSourceDraft>,
    #[serde(default)]
    pub assessment: Option<context::ContextAssessment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ContextNoteDraft {
    #[serde(default)]
    pub kind: crate::domain::ContextKind,
    pub text: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ContextSourceDraft {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub url: String,
    pub supports: String,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub relationship: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SectionDraft {
    pub title: String,
    pub kind: SectionKind,
    pub family: SectionFamily,
    pub pages: PageSpan,
    pub summary: String,
    pub digest: String,
    #[serde(default)]
    pub question: Option<String>,
    /// Indices into the draft outline of the headings this unit groups.
    #[serde(default)]
    pub outline_entries: Vec<usize>,
    #[serde(default)]
    pub source_span: Option<SourceSpanDraft>,
    #[serde(default)]
    pub key_quotes: Vec<KeyQuote>,
    #[serde(default)]
    pub related_terms: Vec<String>,
    #[serde(default = "one")]
    pub tile_width: u8,
    #[serde(default = "one")]
    pub tile_height: u8,
}

/// One printed heading before its anchor is resolved in the layout.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct OutlineEntryDraft {
    #[serde(default)]
    pub number: Option<String>,
    pub title: String,
    pub level: u8,
    pub page: u32,
    #[serde(default)]
    pub role: OutlineRole,
    /// The heading line as printed, used to locate it in the layout.
    #[serde(default)]
    pub anchor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SourceSpanDraft {
    pub start_text: String,
    pub start_page: u32,
    pub end_text: String,
    pub end_page: u32,
}

const fn one() -> u8 {
    1
}

/// Convert admitted `paper-artifacts` core JSON into the reader's draft.
///
/// The core fields are identity, type, abstract, card, question, sections,
/// references, citations, and relations. Only the reader-model fields are
/// mapped here; the graph fields feed the reference resolver instead.
pub(crate) fn draft_from_artifacts(artifacts: &serde_json::Value) -> Result<AnalysisDraft> {
    let thesis = artifacts
        .pointer("/card/thesis/text")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::InvalidAnalysis("the extraction returned no thesis".to_owned()))?
        .to_owned();
    let author_abstract = artifacts
        .pointer("/abstract/text")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let outline = artifacts
        .get("outline")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(outline_entry_from_artifact)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let sections = artifacts
        .get("sections")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| section_from_artifact(item, outline.len()))
                .collect()
        })
        .unwrap_or_default();
    Ok(AnalysisDraft {
        thesis,
        outsider_brief: String::new(),
        author_abstract,
        context_notes: Vec::new(),
        context_sources: Vec::new(),
        assessment: None,
        prerequisites: Vec::new(),
        outline,
        sections,
        claims: Vec::new(),
        glossary: Vec::new(),
        caveats: Vec::new(),
        reading_path: Vec::new(),
        context: context_from_artifacts(artifacts),
    })
}

/// The settled context of an artifacts record, when it has one.
pub(crate) fn context_from_artifacts(artifacts: &serde_json::Value) -> Option<PaperContext> {
    let context = artifacts.get(crate::paper_artifacts::context::FIELD)?;
    if !context.is_object() {
        return None;
    }
    serde_json::from_value(context.clone())
        .map_err(|error| tracing::warn!(%error, "the stored context does not parse"))
        .ok()
}

/// Put a refreshed context on a stored analysis: the context itself, its
/// projection onto the brief and notes, and its warnings. Notes from the
/// older web-research design are kept.
pub(crate) fn apply_context(analysis: &mut PaperAnalysis, refreshed: ContextOutcome) {
    let (brief, notes) = context_projection(&refreshed.context);
    if !brief.is_empty() {
        analysis.outsider_brief = brief;
    }
    analysis.context_notes.retain(|note| note.basis.is_none());
    analysis.context_notes.extend(notes);
    analysis.context = Some(refreshed.context);
    let field = crate::paper_artifacts::context::FIELD;
    analysis
        .quality_warnings
        .retain(|warning| warning.field != field);
    analysis.quality_warnings.extend(
        refreshed
            .warnings
            .into_iter()
            .filter(|warning| warning.field == field),
    );
}

/// What the older reader fields show of a context: the lead as the outsider
/// brief, and the before and after claims as notes carrying their basis.
pub(crate) fn context_projection(context: &PaperContext) -> (String, Vec<ContextNote>) {
    let brief = context
        .lead
        .iter()
        .map(|claim| claim.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let notes = [
        (ContextKind::Before, &context.before),
        (ContextKind::After, &context.after),
    ]
    .into_iter()
    .flat_map(|(kind, claims)| {
        claims.iter().map(move |claim| ContextNote {
            kind,
            text: claim.text.clone(),
            source_ids: Vec::new(),
            basis: Some(claim.basis),
            work_ids: if claim.basis == ContextBasis::Dossier {
                claim.work_ids.clone()
            } else {
                Vec::new()
            },
        })
    })
    .collect();
    (brief, notes)
}

fn outline_entry_from_artifact(item: &serde_json::Value) -> Option<OutlineEntryDraft> {
    let title = compact_whitespace(item.get("title")?.as_str()?);
    if title.is_empty() {
        return None;
    }
    let number = item
        .get("number")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let level = item
        .get("level")
        .and_then(serde_json::Value::as_u64)
        .map_or(1, |level| u8::try_from(level.clamp(1, 6)).unwrap_or(1));
    let page = u32::try_from(item.get("page")?.as_u64()?).ok()?;
    let role = item
        .get("role")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let anchor = item
        .get("anchor")
        .and_then(serde_json::Value::as_str)
        .map(compact_whitespace)
        .filter(|value| !value.is_empty());
    Some(OutlineEntryDraft {
        number,
        title,
        level,
        page,
        role,
        anchor,
    })
}

/// The outline entries a unit groups, when it names a valid range of an
/// outline of `outline_len` entries.
fn outline_range(item: &serde_json::Value, outline_len: usize) -> Vec<usize> {
    let index = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
    };
    match (index("outline_first"), index("outline_last")) {
        (Some(first), Some(last)) if first <= last && last < outline_len => {
            (first..=last).collect()
        }
        _ => Vec::new(),
    }
}

fn section_from_artifact(item: &serde_json::Value, outline_len: usize) -> Option<SectionDraft> {
    let title = item.get("title")?.as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    let kind = item
        .get("kind")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let family = item
        .get("family")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let summary = item
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(title)
        .to_owned();
    let pages = item.get("pages")?;
    let start = u32::try_from(pages.get("start")?.as_u64()?).ok()?;
    let end = u32::try_from(pages.get("end")?.as_u64()?).ok()?;
    let source_span = item.get("source_span").and_then(|span| {
        Some(SourceSpanDraft {
            start_text: span.get("start_text")?.as_str()?.to_owned(),
            start_page: u32::try_from(span.get("start_page")?.as_u64()?).ok()?,
            end_text: span.get("end_text")?.as_str()?.to_owned(),
            end_page: u32::try_from(span.get("end_page")?.as_u64()?).ok()?,
        })
    });
    let question = item
        .get("question")
        .and_then(serde_json::Value::as_str)
        .map(compact_whitespace)
        .filter(|value| !value.is_empty());
    Some(SectionDraft {
        title: title.to_owned(),
        kind,
        family,
        pages: PageSpan { start, end },
        digest: summary.clone(),
        summary,
        question,
        outline_entries: outline_range(item, outline_len),
        source_span,
        key_quotes: Vec::new(),
        related_terms: Vec::new(),
        tile_width: 1,
        tile_height: 1,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClarificationDraft {
    pub answer: String,
    #[serde(default)]
    pub connections: Vec<String>,
    pub limitation: Option<String>,
}

// The normalization sequence intentionally keeps all cross-field invariants in one transaction.
#[allow(clippy::too_many_lines)]
fn normalize_analysis(
    mut draft: AnalysisDraft,
    provider: AnalysisProvider,
    paper: &ExtractedPaper,
) -> Result<PaperAnalysis> {
    draft.thesis = clean_required("thesis", &draft.thesis)?;
    let author_abstract = validated_author_abstract(draft.author_abstract, paper);
    let (mut outsider_brief, mut context_notes, context_sources) = normalize_context(
        &draft.outsider_brief,
        draft.context_notes,
        draft.context_sources,
    );
    if let Some(context) = &draft.context {
        let (brief, notes) = context_projection(context);
        if !brief.is_empty() {
            outsider_brief = brief;
        }
        context_notes.extend(notes);
    }
    if draft.sections.is_empty() {
        return Err(Error::InvalidAnalysis(
            "analysis did not contain any sections".to_owned(),
        ));
    }

    let maximum_page = u32::try_from(paper.pages.len()).unwrap_or(u32::MAX).max(1);
    let outline = normalize_outline(draft.outline, &paper.layout, maximum_page);
    let mut used_ids = std::collections::HashSet::new();
    let mut sections = draft
        .sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| {
            let title = clean_required("section title", &section.title)?;
            let mut id = slugify(&title);
            if id.is_empty() {
                id = format!("section-{}", index + 1);
            }
            if !used_ids.insert(id.clone()) {
                id = format!("{id}-{}", index + 1);
                used_ids.insert(id.clone());
            }
            let declared_pages =
                PageSpan::normalized(section.pages.start, section.pages.end, maximum_page);
            let source_span = section
                .source_span
                .as_ref()
                .and_then(|span| resolve_source_span(&paper.layout, span));
            // Exact, deterministically resolved source anchors are authoritative.
            // The analyzer's redundant page range is only a fallback for PDFs
            // whose text cannot be anchored.
            let pages = source_span.as_ref().map_or(declared_pages, |span| {
                PageSpan::normalized(span.start.page, span.end.page, maximum_page)
            });
            let key_quotes = section
                .key_quotes
                .into_iter()
                .filter(|quote| !quote.text.trim().is_empty())
                .map(|mut quote| {
                    quote.text = compact_whitespace(&quote.text);
                    quote.explanation = compact_whitespace(&quote.explanation);
                    quote.page = quote.page.clamp(pages.start, pages.end);
                    quote
                })
                .take(4)
                .collect();
            let mut outline_entries = section.outline_entries;
            outline_entries.retain(|index| *index < outline.len());
            Ok(PaperSection {
                id,
                title,
                kind: section.kind,
                family: section.family,
                pages,
                summary: clean_required("section summary", &section.summary)?,
                digest: clean_required("section digest", &section.digest)?,
                question: section
                    .question
                    .map(|question| compact_whitespace(&question))
                    .filter(|question| !question.is_empty()),
                outline_entries,
                source_span,
                key_quotes,
                related_terms: clean_list(section.related_terms, 12),
                tile_width: section.tile_width.clamp(1, 4),
                tile_height: section.tile_height.clamp(1, 2),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    repair_section_order(&mut sections, &paper.layout);
    validate_section_order(&sections)?;

    let valid_ids = sections
        .iter()
        .map(|section| section.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    for claim in &mut draft.claims {
        claim.statement = compact_whitespace(&claim.statement);
        claim.support = compact_whitespace(&claim.support);
        claim
            .section_ids
            .retain(|id| valid_ids.contains(id.as_str()));
    }
    for entry in &mut draft.glossary {
        entry.term = compact_whitespace(&entry.term);
        entry.plain_language = compact_whitespace(&entry.plain_language);
        entry.technical_definition = compact_whitespace(&entry.technical_definition);
        entry.why_it_matters = compact_whitespace(&entry.why_it_matters);
        entry
            .section_ids
            .retain(|id| valid_ids.contains(id.as_str()));
    }

    let mut analysis = PaperAnalysis {
        schema_version: 5,
        provider,
        generated_at: Utc::now(),
        thesis: draft.thesis,
        outsider_brief,
        author_abstract,
        abstract_extraction: Some(crate::abstracts::extract(paper)),
        context_assessment: draft.assessment,
        context_notes,
        context_sources,
        prerequisites: clean_list(draft.prerequisites, 12),
        outline,
        sections,
        claims: draft.claims.into_iter().take(16).collect(),
        glossary: deduplicate_glossary(draft.glossary),
        caveats: clean_list(draft.caveats, 12),
        reading_path: clean_list(draft.reading_path, 18),
        pipeline_version: None,
        quality_warnings: Vec::new(),
        context: draft.context,
        abstract_agreement: None,
    };
    validate_citations(&mut analysis, &paper.layout)?;
    Ok(analysis)
}

/// The outline as the reader stores it: page numbers clamped to the paper,
/// and each heading located in the PDF text layer when its printed line can
/// be found there. An entry that cannot be anchored is kept without an anchor,
/// because it was already admitted against the page text upstream.
fn normalize_outline(
    entries: Vec<OutlineEntryDraft>,
    layout: &crate::domain::DocumentLayout,
    maximum_page: u32,
) -> Vec<OutlineEntry> {
    entries
        .into_iter()
        .filter_map(|entry| {
            let title = compact_whitespace(&entry.title);
            if title.is_empty() {
                return None;
            }
            let page = entry.page.clamp(1, maximum_page);
            let printed = entry.anchor.clone().unwrap_or_else(|| {
                entry
                    .number
                    .as_ref()
                    .map_or_else(|| title.clone(), |number| format!("{number} {title}"))
            });
            let anchor = match verify_quote(layout, &printed, page) {
                (
                    crate::domain::CitationStatus::Exact
                    | crate::domain::CitationStatus::Normalized,
                    anchor,
                ) => anchor,
                _ => None,
            };
            Some(OutlineEntry {
                number: entry.number,
                title,
                level: entry.level.clamp(1, 6),
                page,
                role: entry.role,
                anchor,
            })
        })
        .collect()
}

fn normalize_context(
    legacy_brief: &str,
    notes: Vec<ContextNoteDraft>,
    sources: Vec<ContextSourceDraft>,
) -> (String, Vec<ContextNote>, Vec<ContextSource>) {
    let mut id_map = std::collections::HashMap::new();
    let mut used_ids = std::collections::HashSet::new();
    let sources = sources
        .into_iter()
        .filter_map(|source| {
            let original_id = source.id.trim().to_owned();
            let id = slugify(&original_id);
            let title = compact_whitespace(&source.title);
            let url = source.url.trim().to_owned();
            let supports = compact_whitespace(&source.supports);
            if original_id.is_empty()
                || id.is_empty()
                || title.is_empty()
                || url.is_empty()
                || supports.is_empty()
                || !used_ids.insert(id.clone())
            {
                return None;
            }
            id_map.insert(original_id, id.clone());
            Some(ContextSource {
                id,
                title,
                authors: clean_list(source.authors, 20),
                year: source.year.filter(|year| (1000..=2100).contains(year)),
                url,
                supports,
                excerpt: source.excerpt,
                location: source.location,
                relationship: source.relationship,
                // This is replaced by the independent link check before the
                // analysis can be persisted or returned to a client.
                verified_at: Utc::now(),
            })
        })
        .take(8)
        .collect::<Vec<_>>();

    let notes = notes
        .into_iter()
        .filter_map(|note| {
            let text = compact_whitespace(&note.text);
            let mut seen = std::collections::HashSet::new();
            let requested_ids = note
                .source_ids
                .into_iter()
                .map(|id| id.trim().to_owned())
                .filter(|id| seen.insert(id.clone()))
                .collect::<Vec<_>>();
            if text.is_empty() || requested_ids.is_empty() || requested_ids.len() > 4 {
                return None;
            }
            let source_ids = requested_ids
                .iter()
                .map(|id| id_map.get(id).cloned())
                .collect::<Option<Vec<_>>>()?;
            Some(ContextNote {
                kind: note.kind,
                text,
                source_ids,
                basis: None,
                work_ids: Vec::new(),
            })
        })
        .take(6)
        .collect::<Vec<_>>();

    let generated_brief = notes
        .iter()
        .map(|note| note.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let legacy_brief = compact_whitespace(legacy_brief);
    let outsider_brief = if generated_brief.is_empty() {
        legacy_brief
    } else {
        generated_brief
    };
    (outsider_brief, notes, sources)
}

fn resolve_source_span(
    layout: &crate::domain::DocumentLayout,
    span: &SourceSpanDraft,
) -> Option<SectionSourceSpan> {
    let maximum_page = layout
        .pages
        .iter()
        .map(|page| page.number)
        .max()
        .unwrap_or(1);
    let start_page = span.start_page.clamp(1, maximum_page);
    let end_page = span.end_page.clamp(start_page, maximum_page);
    let (start_status, start) = verify_quote(layout, &span.start_text, start_page);
    let (end_status, end) = verify_quote(layout, &span.end_text, end_page);
    if !matches!(
        start_status,
        crate::domain::CitationStatus::Exact | crate::domain::CitationStatus::Normalized
    ) || !matches!(
        end_status,
        crate::domain::CitationStatus::Exact | crate::domain::CitationStatus::Normalized
    ) {
        return None;
    }
    let start = start?;
    let end = end?;
    let ordered =
        start.page < end.page || (start.page == end.page && start.start_token <= end.start_token);
    ordered.then_some(SectionSourceSpan { start, end })
}

const fn anchor_precedes(
    left: &crate::domain::TextAnchor,
    right: &crate::domain::TextAnchor,
) -> bool {
    if left.page == right.page {
        left.end_token < right.start_token
    } else {
        left.page < right.page
    }
}

/// Clip each verified section span so it ends before the next verified
/// section's start. Analyzers sometimes cite the last text on a page rather
/// than the last text of the section; the following section's verified start
/// is the more reliable boundary, so the earlier span is trimmed to the
/// sentence tail preceding it. Spans whose starts are themselves out of order
/// are left for `validate_section_order` to reject.
fn repair_section_order(sections: &mut [PaperSection], layout: &crate::domain::DocumentLayout) {
    let maximum_page = layout
        .pages
        .iter()
        .map(|page| page.number)
        .max()
        .unwrap_or(1);
    let mut previous_index: Option<usize> = None;
    for index in 0..sections.len() {
        let Some(next_start) = sections[index]
            .source_span
            .as_ref()
            .map(|span| span.start.clone())
        else {
            continue;
        };
        if let Some(previous_index) = previous_index {
            let previous = &mut sections[previous_index];
            if let Some(span) = previous.source_span.as_mut()
                && !anchor_precedes(&span.end, &next_start)
                && let Some(clipped) = crate::layout::anchor_before(
                    layout,
                    next_start.page,
                    next_start.start_token,
                    &span.start,
                )
            {
                span.end = clipped;
                previous.pages = PageSpan::normalized(span.start.page, span.end.page, maximum_page);
            }
        }
        previous_index = Some(index);
    }
}

fn validate_section_order(sections: &[PaperSection]) -> Result<()> {
    let mut previous: Option<(&str, &SectionSourceSpan)> = None;
    for section in sections {
        let Some(source_span) = &section.source_span else {
            continue;
        };
        if let Some((prior_title, prior_span)) = previous
            && !anchor_precedes(&prior_span.end, &source_span.start)
        {
            return Err(Error::InvalidAnalysis(format!(
                "section source spans overlap or are out of reading order: '{}' ends at page {} token {}, but '{}' starts at page {} token {}",
                prior_title,
                prior_span.end.page,
                prior_span.end.end_token,
                section.title,
                source_span.start.page,
                source_span.start.start_token,
            )));
        }
        previous = Some((&section.title, source_span));
    }
    Ok(())
}

/// Re-resolve every analyzer-supplied quote against deterministic PDF token
/// coordinates. This also migrates analyses written before coordinate anchors
/// became part of the schema.
pub fn validate_citations(
    analysis: &mut PaperAnalysis,
    layout: &crate::domain::DocumentLayout,
) -> Result<()> {
    repair_section_order(&mut analysis.sections, layout);
    for section in &mut analysis.sections {
        if let Some(source_span) = &section.source_span {
            section.pages = PageSpan::normalized(
                source_span.start.page,
                source_span.end.page,
                layout
                    .pages
                    .iter()
                    .map(|page| page.number)
                    .max()
                    .unwrap_or(1),
            );
        }
        for quote in &mut section.key_quotes {
            let (validation, anchor) = verify_quote(layout, &quote.text, quote.page);
            if let Some(resolved) = &anchor {
                quote.page = resolved.page;
            }
            quote.validation = validation;
            quote.anchor = anchor;
        }
    }
    validate_section_order(&analysis.sections)?;
    analysis.schema_version = analysis.schema_version.max(5);
    Ok(())
}

fn validated_author_abstract(candidate: Option<String>, paper: &ExtractedPaper) -> Option<String> {
    let mut proposed = crate::abstracts::locate(paper)?;
    proposed.text = candidate?;
    crate::abstracts::verify(paper, Some(&proposed)).text
}

fn clean_required(field: &str, value: &str) -> Result<String> {
    let cleaned = compact_whitespace(value);
    if cleaned.is_empty() {
        Err(Error::InvalidAnalysis(format!("{field} was empty")))
    } else {
        Ok(cleaned)
    }
}

pub(crate) fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_list(values: Vec<String>, maximum: usize) -> Vec<String> {
    values
        .into_iter()
        .map(|value| compact_whitespace(&value))
        .filter(|value| !value.is_empty())
        .take(maximum)
        .collect()
}

fn deduplicate_glossary(entries: Vec<GlossaryEntry>) -> Vec<GlossaryEntry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|entry| !entry.term.trim().is_empty())
        .filter(|entry| seen.insert(entry.term.to_lowercase()))
        .take(32)
        .collect()
}

fn matching_glossary(entries: &[GlossaryEntry], selection: &str) -> Vec<GlossaryEntry> {
    let lowered = selection.to_lowercase();
    entries
        .iter()
        .filter(|entry| lowered.contains(&entry.term.to_lowercase()))
        .take(6)
        .cloned()
        .collect()
}

pub(crate) fn slugify(value: &str) -> String {
    let mut output = String::new();
    let mut needs_separator = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if needs_separator && !output.is_empty() {
                output.push('-');
            }
            output.extend(character.to_lowercase());
            needs_separator = false;
        } else {
            needs_separator = true;
        }
    }
    output
}

pub(crate) fn fallback_claim(statement: String, section_id: String) -> Claim {
    Claim {
        support: "The paper presents this point in the cited section; inspect the quoted passage before treating it as established beyond the paper's scope.".to_owned(),
        statement,
        strength: EvidenceStrength::Suggestive,
        section_ids: vec![section_id],
    }
}

pub(crate) fn fallback_quote(text: String, page: u32) -> KeyQuote {
    KeyQuote {
        text,
        page,
        explanation: "This sentence anchors the section's main move in the authors' own words."
            .to_owned(),
        significance: QuoteSignificance::TurningPoint,
        anchor: None,
        validation: crate::domain::CitationStatus::Unverified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        DocumentLayout, ExtractedPage, LayoutPage, LayoutSentence, LayoutToken, PaperMetadata,
        TextAnchor,
    };

    #[cfg(unix)]
    #[tokio::test]
    async fn should_run_the_core_pipeline_with_the_configured_settings_and_surface_its_warnings() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("temporary directory");
        let script = directory.path().join("stub-codex");
        // Every core answer maps only page 1 of the two-page fixture.
        std::fs::write(
            &script,
            r#"#!/bin/sh
prompt=$(cat)
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output-last-message" ]; then shift; output="$1"; fi
  shift
done
case "$prompt" in
  *"You are checking a draft record"*) printf '%s' '{"corrections":[]}' > "$output" ;;
  *"why this paper matters"*) printf '%s' '{"context":{"lead":[{"text":"Single views were thought too weak.","basis":"background","evidence":[],"work_ids":[]}],"before":[],"contribution":[],"after":[],"read_alongside":[],"reception_note":"Not known."}}' > "$output" ;;
  *) printf '%s' '{"identity":{"title":"Paragraph Continuation","authors":[{"name":"A. Author","affiliation":null}]},"card":{"thesis":{"text":"Single-view estimation is enough.","evidence":[],"confidence":"high"}},"abstract":{"text":null,"page":null},"references":[],"sections":[{"title":"Front matter","kind":"other","family":"context","pages":{"start":1,"end":1},"summary":"The opening."}]}' > "$output" ;;
esac
"#,
        )
        .expect("stub script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("executable");
        let service = AnalysisService::new(LocalCliAnalyzer::with_commands(&script, &script))
            .with_settings(AnalysisConfig {
                providers: vec!["example".to_owned()],
                ..AnalysisConfig::default()
            });
        let pdf =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paragraph-continuation.pdf");
        let paper = crate::extract::PdfExtractor::default()
            .extract(&pdf, &PaperMetadata::default())
            .await
            .expect("extraction");
        let outcome = service
            .analyze_core_artifacts(
                AnalysisProvider::Codex,
                &paper,
                &pdf,
                directory.path(),
                Dossier::default(),
                None,
            )
            .await
            .expect("analysis");
        assert!(!outcome.degraded);
        assert_eq!(
            outcome
                .warnings
                .iter()
                .map(|warning| warning.kind)
                .collect::<Vec<_>>(),
            [crate::domain::QualityWarningKind::SectionsLowCoverage]
        );
        assert_eq!(outcome.analysis.quality_warnings, outcome.warnings);
        let context = outcome.analysis.context.as_ref().expect("context");
        assert_eq!(context.lead[0].basis, ContextBasis::Background);
        assert_eq!(context.reception_note.as_deref(), Some("Not known."));
        assert_eq!(context.counts.dossier_works, 0);
        assert_eq!(
            outcome.analysis.outsider_brief,
            "Single views were thought too weak."
        );
        let agreement = outcome
            .analysis
            .abstract_agreement
            .as_ref()
            .expect("abstract agreement");
        assert!(!agreement.model_found);
        let version = outcome
            .analysis
            .pipeline_version
            .as_deref()
            .expect("version");
        assert!(version.starts_with("plan:Codex@example#"), "{version}");
        let record: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("paper-artifacts.json")).expect("record"),
        )
        .expect("record JSON");
        assert_eq!(record["paper_budget_usd"], 0.5);
        assert_eq!(record["providers"], serde_json::json!(["example"]));
        assert_eq!(record["plausibility_retries"][0]["kept_retry"], false);
    }

    fn anchor(page: u32, start_token: u32, end_token: u32) -> TextAnchor {
        TextAnchor {
            page,
            start_token,
            end_token,
            sentence_ids: Vec::new(),
            rects: Vec::new(),
            exact_text: "grounded excerpt".to_owned(),
        }
    }

    fn section(title: &str, pages: PageSpan, start: TextAnchor, end: TextAnchor) -> PaperSection {
        PaperSection {
            id: slugify(title),
            title: title.to_owned(),
            kind: SectionKind::Other,
            family: SectionFamily::Evidence,
            pages,
            summary: "Summary".to_owned(),
            digest: "Digest".to_owned(),
            question: None,
            outline_entries: Vec::new(),
            source_span: Some(SectionSourceSpan { start, end }),
            key_quotes: Vec::new(),
            related_terms: Vec::new(),
            tile_width: 1,
            tile_height: 1,
        }
    }

    /// One page whose tokens are `words`, segmented into a sentence at every
    /// token ending with a period.
    fn layout_page(number: u32, words: &[&str]) -> LayoutPage {
        let tokens = words
            .iter()
            .enumerate()
            .map(|(index, word)| LayoutToken {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                text: (*word).to_owned(),
                line: 0,
                rects: Vec::new(),
            })
            .collect::<Vec<_>>();
        let mut sentences = Vec::new();
        let mut start = 0;
        let last_index = tokens.last().map_or(0, |token| token.index);
        for token in &tokens {
            if token.text.ends_with('.') || token.index == last_index {
                sentences.push(LayoutSentence {
                    id: format!("p{number:04}-s{:05}", sentences.len() + 1),
                    page: number,
                    start_token: start,
                    end_token: token.index,
                    text: String::new(),
                    rects: Vec::new(),
                });
                start = token.index + 1;
            }
        }
        LayoutPage {
            number,
            width: 612.0,
            height: 792.0,
            tokens,
            sentences,
        }
    }

    fn analysis_with_sections(sections: Vec<PaperSection>) -> PaperAnalysis {
        PaperAnalysis {
            schema_version: 4,
            provider: AnalysisProvider::Codex,
            generated_at: Utc::now(),
            thesis: "Thesis".to_owned(),
            outsider_brief: String::new(),
            author_abstract: None,
            abstract_extraction: None,
            context_assessment: None,
            context_notes: Vec::new(),
            context_sources: Vec::new(),
            prerequisites: Vec::new(),
            outline: Vec::new(),
            sections,
            claims: Vec::new(),
            glossary: Vec::new(),
            caveats: Vec::new(),
            reading_path: Vec::new(),
            pipeline_version: None,
            quality_warnings: Vec::new(),
            context: None,
            abstract_agreement: None,
        }
    }

    #[test]
    fn creates_stable_readable_slugs() {
        assert_eq!(slugify("Results & Limitations"), "results-limitations");
        assert_eq!(slugify("  A/B  "), "a-b");
    }

    #[test]
    fn accepts_only_abstract_text_present_in_the_paper() {
        let source = "The authored abstract explains the contribution in the authors' own words.";
        let paper = ExtractedPaper {
            metadata: PaperMetadata::default(),
            pages: vec![ExtractedPage {
                number: 1,
                text: format!("Abstract\n{source}\nIntroduction"),
            }],
            layout: DocumentLayout::default(),
        };

        assert_eq!(
            validated_author_abstract(Some(source.to_owned()), &paper).as_deref(),
            Some(source)
        );
        assert_eq!(
            validated_author_abstract(
                Some("A plausible but invented abstract that is not in the paper.".to_owned()),
                &paper,
            ),
            None
        );
    }

    #[test]
    fn verified_source_spans_override_analyzer_page_claims() -> Result<()> {
        let mut analysis = analysis_with_sections(vec![section(
            "Results",
            PageSpan { start: 8, end: 10 },
            anchor(10, 100, 105),
            anchor(10, 180, 185),
        )]);
        let layout = DocumentLayout {
            schema_version: 1,
            pages: (1..=12)
                .map(|number| LayoutPage {
                    number,
                    width: 612.0,
                    height: 792.0,
                    tokens: Vec::new(),
                    sentences: Vec::new(),
                })
                .collect(),
        };

        validate_citations(&mut analysis, &layout)?;

        assert_eq!(analysis.sections[0].pages, PageSpan { start: 10, end: 10 });
        assert_eq!(analysis.schema_version, 5);
        Ok(())
    }

    #[test]
    fn rejects_overlapping_verified_section_spans() {
        let sections = vec![
            section(
                "First",
                PageSpan { start: 1, end: 2 },
                anchor(1, 0, 2),
                anchor(2, 20, 30),
            ),
            section(
                "Second",
                PageSpan { start: 2, end: 3 },
                anchor(2, 25, 35),
                anchor(3, 10, 20),
            ),
        ];

        let error = validate_section_order(&sections).expect_err("overlap must be rejected");
        assert!(
            error
                .to_string()
                .contains("overlap or are out of reading order")
        );
    }

    #[test]
    fn clips_prior_span_to_the_sentence_before_the_next_verified_start() -> Result<()> {
        let layout = DocumentLayout {
            schema_version: 1,
            pages: vec![layout_page(
                7,
                &[
                    "Methods", "end", "here.", "3", "Results", "begin", "with", "a", "list.",
                ],
            )],
        };
        let mut analysis = analysis_with_sections(vec![
            // The analyzer cited the last text on the page instead of the
            // last text of the section.
            section(
                "Methods",
                PageSpan { start: 6, end: 7 },
                anchor(6, 0, 1),
                anchor(7, 5, 8),
            ),
            section(
                "Results",
                PageSpan { start: 7, end: 9 },
                anchor(7, 3, 4),
                anchor(9, 10, 12),
            ),
        ]);

        validate_citations(&mut analysis, &layout)?;

        let end = &analysis.sections[0]
            .source_span
            .as_ref()
            .ok_or_else(|| Error::Task("span dropped".to_owned()))?
            .end;
        assert_eq!((end.page, end.start_token, end.end_token), (7, 0, 2));
        assert_eq!(end.exact_text, "Methods end here.");
        assert_eq!(analysis.sections[0].pages, PageSpan { start: 6, end: 7 });
        Ok(())
    }

    #[test]
    fn clips_prior_span_back_to_the_previous_page_when_next_section_opens_a_page() -> Result<()> {
        let layout = DocumentLayout {
            schema_version: 1,
            pages: vec![
                layout_page(1, &["Intro", "closes", "here."]),
                layout_page(2, &["2", "Methods", "start", "the", "page."]),
            ],
        };
        let mut analysis = analysis_with_sections(vec![
            section(
                "Intro",
                PageSpan { start: 1, end: 2 },
                anchor(1, 0, 0),
                anchor(2, 2, 4),
            ),
            section(
                "Methods",
                PageSpan { start: 2, end: 2 },
                anchor(2, 0, 1),
                anchor(2, 2, 4),
            ),
        ]);

        validate_citations(&mut analysis, &layout)?;

        let end = &analysis.sections[0]
            .source_span
            .as_ref()
            .ok_or_else(|| Error::Task("span dropped".to_owned()))?
            .end;
        assert_eq!((end.page, end.start_token, end.end_token), (1, 0, 2));
        assert_eq!(analysis.sections[0].pages, PageSpan { start: 1, end: 1 });
        Ok(())
    }

    #[test]
    fn still_rejects_sections_whose_starts_are_out_of_order() {
        let layout = DocumentLayout {
            schema_version: 1,
            pages: vec![layout_page(
                1,
                &["First", "words.", "Second", "words.", "Third", "words."],
            )],
        };
        let mut analysis = analysis_with_sections(vec![
            section(
                "Later",
                PageSpan { start: 1, end: 1 },
                anchor(1, 4, 4),
                anchor(1, 5, 5),
            ),
            section(
                "Earlier",
                PageSpan { start: 1, end: 1 },
                anchor(1, 0, 0),
                anchor(1, 1, 1),
            ),
        ]);

        let error = validate_citations(&mut analysis, &layout)
            .expect_err("reversed sections must be rejected");
        assert!(
            error
                .to_string()
                .contains("overlap or are out of reading order")
        );
    }

    #[test]
    fn drops_context_note_when_any_exact_source_record_is_missing() {
        let (brief, notes, sources) = normalize_context(
            "Legacy context must not leak through.",
            vec![ContextNoteDraft {
                kind: crate::domain::ContextKind::Legacy,
                text: "A broad reception claim.".to_owned(),
                source_ids: vec!["known".to_owned(), "missing".to_owned()],
            }],
            vec![ContextSourceDraft {
                excerpt: None,
                location: None,
                relationship: None,
                id: "known".to_owned(),
                title: "Known source".to_owned(),
                authors: vec!["Researcher".to_owned()],
                year: Some(2020),
                url: "https://example.com/known".to_owned(),
                supports: "One part of the claim.".to_owned(),
            }],
        );

        assert!(notes.is_empty());
        assert_eq!(sources.len(), 1);
        assert_eq!(brief, "Legacy context must not leak through.");
    }

    #[test]
    fn maps_core_artifacts_to_a_reader_draft() {
        let artifacts = serde_json::json!({
            "card": {"thesis": {
                "text": "A sharper thesis.",
                "evidence": [{"quote": "a bound", "page": 1}],
                "confidence": "high"
            }},
            "abstract": {"text": "The authors' abstract."},
            "sections": [{
                "title": "Opening",
                "kind": "background",
                "family": "context",
                "summary": "What the section does.",
                "pages": {"start": 1, "end": 3},
                "source_span": {
                    "start_text": "1", "start_page": 1,
                    "end_text": "end", "end_page": 3
                }
            }]
        });
        let draft = draft_from_artifacts(&artifacts).expect("draft");
        assert_eq!(draft.thesis, "A sharper thesis.");
        assert_eq!(
            draft.author_abstract.as_deref(),
            Some("The authors' abstract.")
        );
        assert_eq!(draft.sections.len(), 1);
        assert_eq!(draft.sections[0].title, "Opening");
        assert_eq!(draft.sections[0].kind, SectionKind::Background);
        assert_eq!(draft.sections[0].family, SectionFamily::Context);
        assert_eq!(draft.sections[0].pages.start, 1);
        assert!(draft.sections[0].source_span.is_some());
    }

    fn settled_context() -> serde_json::Value {
        serde_json::json!({
            "lead": [
                {"text": "It overturned the standard prediction.", "basis": "background", "evidence": [], "work_ids": []},
                {"text": "It used a natural experiment.", "basis": "paper",
                 "evidence": [{"quote": "a natural experiment", "page": 2}], "work_ids": []}
            ],
            "before": [{"text": "The competitive model predicted lower wages.", "basis": "dossier",
                        "evidence": [], "work_ids": ["w1"]}],
            "contribution": [{"text": "A comparison city design.", "basis": "paper",
                              "evidence": [{"quote": "comparison cities", "page": 3}], "work_ids": []}],
            "after": [{"text": "A reanalysis disputed it.", "basis": "background", "evidence": [],
                       "work_ids": [], "downgraded": "unknown_work"}],
            "read_alongside": [{"work_id": "w1", "title": "The Old Model", "authors": "A. Smith et al.",
                                "year": 1974, "relation": "predecessor", "why": "The prediction it tested.",
                                "basis": "dossier", "local_paper_id": null}],
            "reception_note": null,
            "counts": {"dossier_works": 3, "paper_claims": 2, "dossier_claims": 1, "background_claims": 2,
                       "downgraded_quote_not_found": 0, "downgraded_unknown_work": 1,
                       "unknown_work_ids": 1, "dropped_read_alongside": 0}
        })
    }

    #[test]
    fn should_map_the_context_onto_the_brief_the_notes_and_the_stored_context() {
        let artifacts = serde_json::json!({
            "card": {"thesis": {"text": "A sharper thesis.", "evidence": [], "confidence": "high"}},
            "context": settled_context(),
        });
        let draft = draft_from_artifacts(&artifacts).expect("draft");
        let context = draft.context.expect("context");
        assert_eq!(context.lead.len(), 2);
        assert_eq!(context.lead[1].basis, ContextBasis::Paper);
        assert_eq!(context.lead[1].evidence[0].page, 2);
        assert_eq!(
            context.after[0].downgraded,
            Some(crate::domain::ContextDowngrade::UnknownWork)
        );
        assert_eq!(context.read_alongside[0].work_id.as_deref(), Some("w1"));
        assert_eq!(
            context.read_alongside[0].relation,
            crate::domain::NeighbourRelation::Predecessor
        );
        assert_eq!(context.counts.unknown_work_ids, 1);
        assert_eq!(context.claims().count(), 5);

        let (brief, notes) = context_projection(&context);
        assert_eq!(
            brief,
            "It overturned the standard prediction. It used a natural experiment."
        );
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].kind, ContextKind::Before);
        assert_eq!(notes[0].basis, Some(ContextBasis::Dossier));
        assert_eq!(notes[0].work_ids, ["w1"]);
        assert_eq!(notes[1].kind, ContextKind::After);
        assert!(notes[1].work_ids.is_empty());
    }

    #[test]
    fn should_replace_only_the_context_nodes_notes_when_a_context_is_refreshed() {
        let mut analysis = analysis_with_sections(Vec::new());
        analysis.context_notes = vec![
            ContextNote {
                kind: ContextKind::Legacy,
                text: "A web-researched note.".to_owned(),
                source_ids: vec!["s1".to_owned()],
                basis: None,
                work_ids: Vec::new(),
            },
            ContextNote {
                kind: ContextKind::Before,
                text: "A stale node note.".to_owned(),
                source_ids: Vec::new(),
                basis: Some(ContextBasis::Background),
                work_ids: Vec::new(),
            },
        ];
        analysis.quality_warnings = vec![QualityWarning {
            kind: crate::domain::QualityWarningKind::ContextClaimsDowngraded,
            field: "context".to_owned(),
            message: "stale".to_owned(),
        }];
        let context: PaperContext = serde_json::from_value(settled_context()).expect("context");
        apply_context(
            &mut analysis,
            ContextOutcome {
                context,
                warnings: Vec::new(),
                degraded: false,
                pipeline_version: "v".to_owned(),
            },
        );
        let texts = analysis
            .context_notes
            .iter()
            .map(|note| note.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            [
                "A web-researched note.",
                "The competitive model predicted lower wages.",
                "A reanalysis disputed it."
            ]
        );
        assert!(analysis.outsider_brief.starts_with("It overturned"));
        assert!(analysis.context.is_some());
        assert!(analysis.quality_warnings.is_empty());
    }

    #[test]
    fn should_load_an_analysis_stored_before_the_context_node() {
        let mut stored = serde_json::to_value(analysis_with_sections(Vec::new())).expect("JSON");
        let fields = stored.as_object_mut().expect("object");
        fields.remove("context");
        fields.remove("abstract_agreement");
        fields.insert(
            "context_notes".to_owned(),
            serde_json::json!([{"kind": "before", "text": "Old note.", "source_ids": ["s1"]}]),
        );
        let analysis: PaperAnalysis = serde_json::from_value(stored).expect("old analysis loads");
        assert!(analysis.context.is_none());
        assert!(analysis.abstract_agreement.is_none());
        assert_eq!(analysis.context_notes[0].basis, None);
        let written = serde_json::to_value(&analysis.context_notes[0]).expect("JSON");
        assert!(written.get("basis").is_none(), "old notes keep their shape");
    }

    #[test]
    fn maps_the_outline_and_grouped_units_into_the_reader_analysis() {
        let artifacts = serde_json::json!({
            "card": {"thesis": {"text": "Pruning keeps accuracy.", "evidence": [], "confidence": "high"}},
            "outline": [
                {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main", "anchor": "1 Introduction"},
                {"number": "2", "title": "Pruning Method", "level": 1, "page": 1, "role": "main", "anchor": "2 Pruning Method"},
                {"number": null, "title": "References", "level": 1, "page": 2, "role": "references", "anchor": "References"}
            ],
            "sections": [
                {"title": "Why and how to prune", "kind": "methods", "family": "method",
                 "outline_first": 0, "outline_last": 1, "summary": "Prune small weights first.",
                 "question": "How is pruning done?", "pages": {"start": 1, "end": 1},
                 "source_span": {"start_text": "1 Introduction", "start_page": 1,
                                 "end_text": "weights first.", "end_page": 1}},
                {"title": "Old map unit", "kind": "other", "family": "evidence",
                 "outline_first": null, "outline_last": null, "summary": "s",
                 "question": "", "pages": {"start": 2, "end": 2},
                 "source_span": {"start_text": "x", "start_page": 2, "end_text": "y", "end_page": 2}},
                {"title": "Out of range", "kind": "other", "family": "evidence",
                 "outline_first": 1, "outline_last": 9, "summary": "s", "question": "q",
                 "pages": {"start": 2, "end": 2},
                 "source_span": {"start_text": "x", "start_page": 2, "end_text": "y", "end_page": 2}}
            ]
        });
        let draft = draft_from_artifacts(&artifacts).expect("draft");
        assert_eq!(draft.outline.len(), 3);
        assert_eq!(draft.outline[2].role, OutlineRole::References);
        assert_eq!(draft.sections[0].outline_entries, [0, 1]);
        assert_eq!(
            draft.sections[0].question.as_deref(),
            Some("How is pruning done?")
        );
        assert!(draft.sections[1].outline_entries.is_empty());
        assert!(draft.sections[1].question.is_none());
        assert!(draft.sections[2].outline_entries.is_empty());

        let words = [
            "1",
            "Introduction",
            "Pruning",
            "matters.",
            "2",
            "Pruning",
            "Method",
            "We",
            "prune",
            "small",
            "weights",
            "first.",
        ];
        let paper = ExtractedPaper {
            metadata: PaperMetadata::default(),
            pages: vec![
                ExtractedPage {
                    number: 1,
                    text: words.join(" "),
                },
                ExtractedPage {
                    number: 2,
                    text: "References".to_owned(),
                },
            ],
            layout: DocumentLayout {
                schema_version: 1,
                pages: vec![layout_page(1, &words), layout_page(2, &["References"])],
            },
        };
        let mut draft = draft;
        draft.sections.truncate(1);
        let analysis =
            normalize_analysis(draft, AnalysisProvider::OpenRouter, &paper).expect("analysis");
        assert_eq!(analysis.outline.len(), 3);
        assert_eq!(analysis.outline[0].number.as_deref(), Some("1"));
        assert_eq!(
            analysis.outline[1]
                .anchor
                .as_ref()
                .map(|anchor| anchor.exact_text.as_str()),
            Some("2 Pruning Method")
        );
        assert!(analysis.outline[0].anchor.is_some());
        assert_eq!(analysis.sections[0].outline_entries, [0, 1]);
        assert_eq!(
            analysis.sections[0].question.as_deref(),
            Some("How is pruning done?")
        );
        // Stored analyses written before the outline existed still load.
        let mut old = serde_json::to_value(&analysis).expect("json");
        old.as_object_mut().expect("object").remove("outline");
        for section in old["sections"].as_array_mut().expect("sections") {
            let fields = section.as_object_mut().expect("section");
            fields.remove("outline_entries");
            fields.remove("question");
        }
        let loaded: PaperAnalysis = serde_json::from_value(old).expect("old analysis");
        assert!(loaded.outline.is_empty() && loaded.sections[0].outline_entries.is_empty());
    }

    #[test]
    fn rejects_artifacts_without_a_thesis() {
        let artifacts = serde_json::json!({"sections": []});
        assert!(draft_from_artifacts(&artifacts).is_err());
    }

    #[test]
    fn rejects_a_thesis_that_is_not_an_object() {
        let artifacts = serde_json::json!({"card": {"thesis": "A bare string."}});
        assert!(draft_from_artifacts(&artifacts).is_err());
    }
}
