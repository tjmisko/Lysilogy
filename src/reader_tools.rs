use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    domain::{AnalysisProvider, ExtractedPaper, PaperId},
};

pub const AGENT_NAME: &str = "Lysilogos";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutFormat {
    TenParagraphs,
    SixPages,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Source,
    Connector,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CutSegment {
    pub kind: SegmentKind,
    pub text: String,
    pub source_page: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CutParagraph {
    pub cut_page: u8,
    pub segments: Vec<CutSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CutDraft {
    pub paragraphs: Vec<CutParagraph>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Supercut {
    pub id: String,
    pub format: CutFormat,
    pub agent: String,
    pub provider: AnalysisProvider,
    pub created_at: DateTime<Utc>,
    pub source_words: usize,
    pub total_words: usize,
    pub paragraphs: Vec<CutParagraph>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceCandidate {
    pub title: String,
    pub landing_url: Option<String>,
    pub pdf_url: Option<String>,
    pub explanation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperEvidence {
    pub paper_id: PaperId,
    pub source_page: u32,
    pub quote: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationVerdict {
    Supports,
    Contradicts,
    Qualifies,
    Unclear,
    Context,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceConnection {
    pub verdict: RelationVerdict,
    pub connector: String,
    pub limitation: String,
    pub evidence: Vec<PaperEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedReference {
    pub id: String,
    pub citation: String,
    pub source_page: Option<u32>,
    pub note: String,
    pub created_at: DateTime<Utc>,
    pub candidate: Option<ReferenceCandidate>,
    pub linked_paper_id: Option<PaperId>,
    pub connection: Option<ReferenceConnection>,
    pub question: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolJobStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolAction {
    Supercut {
        format: CutFormat,
    },
    FindReference {
        reference_id: String,
    },
    ConnectReference {
        reference_id: String,
        question: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolJob {
    pub id: String,
    pub action: ToolAction,
    pub provider: AnalysisProvider,
    pub status: ToolJobStatus,
    pub created_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReaderTools {
    pub supercuts: Vec<Supercut>,
    pub references: Vec<SavedReference>,
    pub jobs: Vec<ToolJob>,
}

pub fn bounded_text(value: &str, label: &str, maximum: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > maximum {
        return Err(Error::InvalidRequest(format!(
            "{label} must contain 1–{maximum} characters"
        )));
    }
    Ok(value.to_owned())
}

/// Matches whole, case-sensitive words; only extraction whitespace may differ.
pub fn exact_passage(paper: &ExtractedPaper, page: u32, quote: &str) -> Result<(usize, usize)> {
    let source = paper
        .pages
        .iter()
        .find(|entry| entry.number == page)
        .ok_or_else(|| Error::InvalidAnalysis("quoted PDF page does not exist".to_owned()))?;
    let source_words: Vec<_> = source.text.split_whitespace().collect();
    let words: Vec<_> = quote.split_whitespace().collect();
    if words.is_empty() || words.len() > source_words.len() {
        return Err(Error::InvalidAnalysis(
            "source passage is empty or absent".to_owned(),
        ));
    }
    let matches: Vec<_> = source_words
        .windows(words.len())
        .enumerate()
        .filter(|(_, window)| *window == words.as_slice())
        .map(|(index, _)| index)
        .collect();
    if matches.len() != 1 {
        return Err(Error::InvalidAnalysis(
            "source passage must match uniquely on its stated PDF page".to_owned(),
        ));
    }
    Ok((matches[0], matches[0] + words.len()))
}

pub fn validate_cut(
    draft: &CutDraft,
    format: CutFormat,
    paper: &ExtractedPaper,
) -> Result<(usize, usize)> {
    if draft.paragraphs.is_empty()
        || draft.paragraphs.len() > 36
        || (format == CutFormat::TenParagraphs && draft.paragraphs.len() != 10)
    {
        return Err(Error::InvalidAnalysis(
            "supercut must have ten paragraphs or up to six pages of paragraphs".to_owned(),
        ));
    }
    let page_count = if format == CutFormat::SixPages { 6 } else { 1 };
    let mut source_words = 0;
    let mut total_words = 0;
    let mut page_words = [0_usize; 6];
    let mut page_chars = [0_usize; 6];
    let mut seen_pages = std::collections::BTreeSet::new();
    let mut ranges = Vec::new();
    let mut quoted_texts = std::collections::HashSet::new();
    let mut previous_page = 1;
    for paragraph in &draft.paragraphs {
        if paragraph.cut_page < previous_page
            || paragraph.cut_page > page_count
            || paragraph.cut_page == 0
            || paragraph.segments.is_empty()
            || paragraph.segments.len() > 8
        {
            return Err(Error::InvalidAnalysis(
                "invalid supercut page order or paragraph segments".to_owned(),
            ));
        }
        previous_page = paragraph.cut_page;
        seen_pages.insert(paragraph.cut_page);
        let mut paragraph_source = 0;
        for segment in &paragraph.segments {
            let words = segment.text.split_whitespace().count();
            if words == 0 || segment.text.contains('\n') || segment.text.contains('\r') {
                return Err(Error::InvalidAnalysis(
                    "segments must contain one nonempty text run".to_owned(),
                ));
            }
            match segment.kind {
                SegmentKind::Source => {
                    if !quoted_texts.insert(
                        segment
                            .text
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" "),
                    ) {
                        return Err(Error::InvalidAnalysis(
                            "supercut repeats source text".to_owned(),
                        ));
                    }
                    let page = segment.source_page.ok_or_else(|| {
                        Error::InvalidAnalysis("source text needs a PDF page".to_owned())
                    })?;
                    let (start, end) = exact_passage(paper, page, &segment.text)?;
                    if ranges
                        .iter()
                        .any(|&(p, s, e)| p == page && start < e && end > s)
                    {
                        return Err(Error::InvalidAnalysis(
                            "supercut repeats overlapping source text".to_owned(),
                        ));
                    }
                    ranges.push((page, start, end));
                    source_words += words;
                    paragraph_source += words;
                }
                SegmentKind::Connector => {
                    if segment.source_page.is_some() || words > 40 {
                        return Err(Error::InvalidAnalysis(
                            "connectors must be flagged, unquoted, and at most 40 words".to_owned(),
                        ));
                    }
                }
            }
            total_words += words;
            let index = usize::from(paragraph.cut_page - 1);
            page_words[index] += words;
            page_chars[index] += segment.text.chars().count();
        }
        if paragraph_source == 0 {
            return Err(Error::InvalidAnalysis(
                "every paragraph needs exact source text".to_owned(),
            ));
        }
    }
    if source_words * 5 < total_words * 4 {
        return Err(Error::InvalidAnalysis(
            "at least 80% of supercut words must be exact source text".to_owned(),
        ));
    }
    validate_cut_budget(
        format,
        seen_pages.len(),
        &page_words,
        &page_chars,
        total_words,
    )?;
    Ok((source_words, total_words))
}

fn validate_cut_budget(
    format: CutFormat,
    page_count: usize,
    page_words: &[usize; 6],
    page_chars: &[usize; 6],
    total_words: usize,
) -> Result<()> {
    if format == CutFormat::SixPages
        && (page_count != 6
            || page_words.iter().any(|&words| words > 420)
            || page_chars.iter().any(|&chars| chars > 2800))
    {
        return Err(Error::InvalidAnalysis(
            "six-page cuts need six nonempty pages, each at most 420 words and 2800 characters"
                .to_owned(),
        ));
    }
    if format == CutFormat::TenParagraphs && total_words > 2400 {
        return Err(Error::InvalidAnalysis(
            "ten-paragraph cuts must stay within 2400 words".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_connection(
    connection: &ReferenceConnection,
    current: (&PaperId, &ExtractedPaper),
    cited: (&PaperId, &ExtractedPaper),
) -> Result<()> {
    bounded_text(&connection.connector, "connector", 2400)?;
    bounded_text(&connection.limitation, "limitation", 1200)?;
    if connection.evidence.len() < 2 || connection.evidence.len() > 8 {
        return Err(Error::InvalidAnalysis(
            "connections need 2–8 source passages across both papers".to_owned(),
        ));
    }
    for evidence in &connection.evidence {
        let paper = if evidence.paper_id == *current.0 {
            current.1
        } else if evidence.paper_id == *cited.0 {
            cited.1
        } else {
            return Err(Error::InvalidAnalysis(
                "connection cites an unrelated paper".to_owned(),
            ));
        };
        exact_passage(paper, evidence.source_page, &evidence.quote)?;
    }
    if !connection
        .evidence
        .iter()
        .any(|item| item.paper_id == *current.0)
        || !connection
            .evidence
            .iter()
            .any(|item| item.paper_id == *cited.0)
    {
        return Err(Error::InvalidAnalysis(
            "connection must quote both papers".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentLayout, ExtractedPage, PaperMetadata};
    use std::path::Path;

    fn paper() -> ExtractedPaper {
        ExtractedPaper {
            metadata: PaperMetadata::default(),
            pages: (1..=10).map(|number| ExtractedPage {
                number, text: format!("Passage {number} describes a distinct mechanism with evidence and qualifications."),
            }).collect(),
            layout: DocumentLayout::default(),
        }
    }

    fn draft() -> CutDraft {
        CutDraft {
            paragraphs: paper()
                .pages
                .iter()
                .map(|page| CutParagraph {
                    cut_page: 1,
                    segments: vec![CutSegment {
                        kind: SegmentKind::Source,
                        text: page.text.clone(),
                        source_page: Some(page.number),
                    }],
                })
                .collect(),
        }
    }

    #[test]
    fn cut_requires_exact_words_and_real_pages() {
        let source = paper();
        let cut = draft();
        let (copied, total) = validate_cut(&cut, CutFormat::TenParagraphs, &source).unwrap();
        assert_eq!(copied, total);
        let mut changed = cut.clone();
        changed.paragraphs[0].segments[0].text =
            "Passage 1 describes a fictional mechanism.".to_owned();
        assert!(validate_cut(&changed, CutFormat::TenParagraphs, &source).is_err());
        changed = cut;
        changed.paragraphs[0].segments[0].source_page = Some(100);
        assert!(validate_cut(&changed, CutFormat::TenParagraphs, &source).is_err());
        assert!(
            exact_passage(
                &source,
                1,
                "Passage  1 describes\na distinct mechanism with evidence and qualifications."
            )
            .is_ok()
        );
        assert!(exact_passage(&source, 1, "passage 1").is_err());
        assert!(exact_passage(&source, 1, "mechanism with evidence and qualification").is_err());
    }

    #[test]
    fn cut_rejects_padding_and_more_than_twenty_percent_connectors() {
        let source = paper();
        let mut cut = draft();
        cut.paragraphs[1] = cut.paragraphs[0].clone();
        assert!(validate_cut(&cut, CutFormat::TenParagraphs, &source).is_err());
        cut = draft();
        for paragraph in &mut cut.paragraphs {
            paragraph.segments.push(CutSegment {
                kind: SegmentKind::Connector,
                text: "This additional prose is generated connective commentary.".to_owned(),
                source_page: None,
            });
        }
        assert!(validate_cut(&cut, CutFormat::TenParagraphs, &source).is_err());
        cut = draft();
        cut.paragraphs[0].segments.push(CutSegment {
            kind: SegmentKind::Connector,
            text: "Next, consider the evidence.".to_owned(),
            source_page: None,
        });
        assert!(validate_cut(&cut, CutFormat::TenParagraphs, &source).is_ok());
        cut.paragraphs.pop();
        assert!(validate_cut(&cut, CutFormat::TenParagraphs, &source).is_err());
    }

    #[test]
    fn six_page_cut_requires_six_bounded_output_pages() {
        let source = paper();
        let mut cut = draft();
        cut.paragraphs.truncate(6);
        for (index, paragraph) in cut.paragraphs.iter_mut().enumerate() {
            paragraph.cut_page = u8::try_from(index + 1).unwrap();
        }
        assert!(validate_cut(&cut, CutFormat::SixPages, &source).is_ok());
        cut.paragraphs[5].cut_page = 5;
        assert!(validate_cut(&cut, CutFormat::SixPages, &source).is_err());
        cut.paragraphs[5].cut_page = 7;
        assert!(validate_cut(&cut, CutFormat::SixPages, &source).is_err());
        cut.paragraphs[5].cut_page = 0;
        assert!(validate_cut(&cut, CutFormat::SixPages, &source).is_err());
    }

    #[test]
    fn connections_require_verified_evidence_from_both_papers() {
        let current = PaperId::from_relative_path(Path::new("current.pdf"));
        let cited = PaperId::from_relative_path(Path::new("cited.pdf"));
        let source = paper();
        let mut connection = ReferenceConnection {
            verdict: RelationVerdict::Qualifies,
            connector: "The second paper restricts the mechanism's scope.".to_owned(),
            limitation: "This is an interpretation of the supplied passages.".to_owned(),
            evidence: vec![
                PaperEvidence {
                    paper_id: current.clone(),
                    source_page: 1,
                    quote: source.pages[0].text.clone(),
                },
                PaperEvidence {
                    paper_id: cited.clone(),
                    source_page: 2,
                    quote: source.pages[1].text.clone(),
                },
            ],
        };
        assert!(validate_connection(&connection, (&current, &source), (&cited, &source)).is_ok());
        connection.evidence[1].quote = "Fabricated support.".to_owned();
        assert!(validate_connection(&connection, (&current, &source), (&cited, &source)).is_err());
        connection.evidence[1] = connection.evidence[0].clone();
        assert!(validate_connection(&connection, (&current, &source), (&cited, &source)).is_err());
    }

    #[test]
    fn ambiguous_passages_cannot_be_certified() {
        let mut source = paper();
        source.pages[0].text = "Repeated text. Repeated text.".to_owned();
        assert!(exact_passage(&source, 1, "Repeated text.").is_err());
    }
}
