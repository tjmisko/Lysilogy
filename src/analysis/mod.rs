mod heuristic;
mod local_cli;
mod sources;

use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    domain::{
        AnalysisProvider, Claim, Clarification, ContextNote, ContextSource, EvidenceStrength,
        ExtractedPaper, GlossaryEntry, KeyQuote, PageSpan, PaperAnalysis, PaperSection,
        QuoteSignificance, SectionFamily, SectionKind, SectionSourceSpan,
    },
    error::Error,
    layout::verify_quote,
};

pub use heuristic::HeuristicAnalyzer;
pub use local_cli::LocalCliAnalyzer;

pub const ANALYSIS_SCHEMA: &str = include_str!("../../prompts/paper-analysis.schema.json");
pub const CLARIFICATION_SCHEMA: &str = include_str!("../../prompts/clarification.schema.json");

#[derive(Clone, Debug, Default)]
pub struct AnalysisService {
    local_cli: LocalCliAnalyzer,
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
    ) -> Result<PaperAnalysis> {
        let draft = match provider {
            AnalysisProvider::Heuristic => HeuristicAnalyzer::analyze(paper),
            AnalysisProvider::Codex | AnalysisProvider::Claude => {
                self.local_cli
                    .analyze(provider, paper, artifact_directory)
                    .await?
            }
        };
        let mut analysis = normalize_analysis(draft, provider, paper)?;
        sources::verify_context_sources(&mut analysis).await;
        Ok(analysis)
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
    let sections = draft
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
            let pages = PageSpan::normalized(section.pages.start, section.pages.end, maximum_page);
            let source_span = section
                .source_span
                .as_ref()
                .and_then(|span| resolve_source_span(&paper.layout, pages, span));
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
        schema_version: 4,
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
    validate_citations(&mut analysis, &paper.layout);
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
    pages: PageSpan,
    span: &SourceSpanDraft,
) -> Option<SectionSourceSpan> {
    let start_page = span.start_page.clamp(pages.start, pages.end);
    let end_page = span.end_page.clamp(start_page, pages.end);
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

/// Re-resolve every analyzer-supplied quote against deterministic PDF token
/// coordinates. This also migrates analyses written before coordinate anchors
/// became part of the schema.
pub fn validate_citations(analysis: &mut PaperAnalysis, layout: &crate::domain::DocumentLayout) {
    for section in &mut analysis.sections {
        for quote in &mut section.key_quotes {
            let (validation, anchor) = verify_quote(layout, &quote.text, quote.page);
            if let Some(resolved) = &anchor {
                quote.page = resolved.page;
            }
            quote.validation = validation;
            quote.anchor = anchor;
        }
    }
    analysis.schema_version = analysis.schema_version.max(2);
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
    use crate::domain::{DocumentLayout, ExtractedPage, PaperMetadata};

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
