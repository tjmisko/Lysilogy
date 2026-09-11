mod heuristic;
mod local_cli;
mod prefetch;
mod sources;

use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    domain::{
        AgentSession, AnalysisProvider, Claim, Clarification, ContextNote, ContextSource,
        EvidenceStrength, ExtractedPaper, GlossaryEntry, KeyQuote, LearningRamp, PageSpan,
        PaperAnalysis, PaperSection, PromptExperiment, PromptVariant, QuoteSignificance,
        SectionFamily, SectionKind, SectionSourceSpan,
    },
    error::Error,
    layout::verify_quote,
};

pub use heuristic::HeuristicAnalyzer;
pub use local_cli::LocalCliAnalyzer;

pub const ANALYSIS_SCHEMA: &str = include_str!("../../prompts/paper-analysis.schema.json");
pub const CLARIFICATION_SCHEMA: &str = include_str!("../../prompts/clarification.schema.json");
pub const ORIENTATION_SCHEMA: &str = include_str!("../../prompts/paper-orientation.schema.json");
pub const STRUCTURE_SCHEMA: &str = include_str!("../../prompts/paper-structure.schema.json");
pub const CONTEXT_SCHEMA: &str = include_str!("../../prompts/paper-context.schema.json");
pub const LEARNING_RAMP_SCHEMA: &str = include_str!("../../prompts/learning-ramp.schema.json");

#[derive(Clone, Debug, Default)]
pub struct AnalysisService {
    local_cli: LocalCliAnalyzer,
}

#[derive(Debug)]
pub struct AnalysisOutcome {
    pub analysis: PaperAnalysis,
    pub session: Option<AgentSession>,
}

pub(crate) struct ExperimentVariantRequest<'a> {
    pub experiment: &'a PromptExperiment,
    pub variant: &'a PromptVariant,
    pub reader_baseline: &'a [String],
    pub run_id: &'a str,
    pub blind_label: &'a str,
}

impl AnalysisService {
    #[must_use]
    pub const fn new(local_cli: LocalCliAnalyzer) -> Self {
        Self { local_cli }
    }

    pub async fn analyze(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        artifact_directory: &Path,
        reset_stages: bool,
    ) -> Result<AnalysisOutcome> {
        let (draft, session) = match provider {
            AnalysisProvider::Heuristic => (HeuristicAnalyzer::analyze(paper), None),
            AnalysisProvider::Codex | AnalysisProvider::Claude => {
                let result = self
                    .local_cli
                    .analyze(provider, paper, artifact_directory, reset_stages)
                    .await?;
                (result.draft, result.session)
            }
        };
        let mut analysis = normalize_analysis(draft, provider, paper)?;
        sources::verify_context_sources(&mut analysis).await;
        Ok(AnalysisOutcome { analysis, session })
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
        sources::verify_context_sources(&mut analysis).await;
        Ok(AnalysisOutcome {
            analysis,
            session: result.session,
        })
    }

    pub async fn clarify(
        &self,
        provider: AnalysisProvider,
        paper: &ExtractedPaper,
        analysis: &PaperAnalysis,
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
            AnalysisProvider::Codex | AnalysisProvider::Claude => {
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
                let concepts = matching_glossary(&analysis.glossary, selection);
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
    pub prerequisites: Vec<String>,
    pub sections: Vec<SectionDraft>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub glossary: Vec<GlossaryEntry>,
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub reading_path: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct OrientationDraft {
    pub thesis: String,
    pub author_abstract: Option<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct StructureDraft {
    pub sections: Vec<SectionDraft>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub glossary: Vec<GlossaryEntry>,
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub reading_path: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ExternalContextDraft {
    #[serde(default)]
    pub context_notes: Vec<ContextNoteDraft>,
    #[serde(default)]
    pub context_sources: Vec<ContextSourceDraft>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ContextNoteDraft {
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
    let (outsider_brief, context_notes, context_sources) = normalize_context(
        &draft.outsider_brief,
        draft.context_notes,
        draft.context_sources,
    );
    if draft.sections.is_empty() {
        return Err(Error::InvalidAnalysis(
            "analysis did not contain any sections".to_owned(),
        ));
    }

    let maximum_page = u32::try_from(paper.pages.len()).unwrap_or(u32::MAX).max(1);
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
            Ok(PaperSection {
                id,
                title,
                kind: section.kind,
                family: section.family,
                pages,
                summary: clean_required("section summary", &section.summary)?,
                digest: clean_required("section digest", &section.digest)?,
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
        context_notes,
        context_sources,
        prerequisites: clean_list(draft.prerequisites, 12),
        sections,
        claims: draft.claims.into_iter().take(16).collect(),
        glossary: deduplicate_glossary(draft.glossary),
        caveats: clean_list(draft.caveats, 12),
        reading_path: clean_list(draft.reading_path, 18),
    };
    validate_citations(&mut analysis, &paper.layout)?;
    Ok(analysis)
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
                // This is replaced by the independent link check before the
                // analysis can be persisted or returned to a client.
                verified_at: Utc::now(),
            })
        })
        .take(6)
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
            Some(ContextNote { text, source_ids })
        })
        .take(2)
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
    let abstract_text = compact_whitespace(&candidate?);
    if abstract_text.chars().count() < 30 || abstract_text.chars().count() > 12_000 {
        return None;
    }
    let source_text = compact_whitespace(
        &paper
            .pages
            .iter()
            .map(|page| page.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    source_text
        .contains(&abstract_text)
        .then_some(abstract_text)
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
            context_notes: Vec::new(),
            context_sources: Vec::new(),
            prerequisites: Vec::new(),
            sections,
            claims: Vec::new(),
            glossary: Vec::new(),
            caveats: Vec::new(),
            reading_path: Vec::new(),
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
                text: source.to_owned(),
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
                text: "A broad reception claim.".to_owned(),
                source_ids: vec!["known".to_owned(), "missing".to_owned()],
            }],
            vec![ContextSourceDraft {
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
}
