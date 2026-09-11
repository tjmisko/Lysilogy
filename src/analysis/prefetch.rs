use serde::{Deserialize, Serialize};

use crate::{abstracts::target_start_index, domain::ExtractedPaper};

use super::compact_whitespace;

const MAX_STRUCTURE_CHARS: usize = 500_000;
const ORIENTATION_EDGE_CHARS: usize = 8_000;
const CLARIFICATION_CONTEXT_CHARS: usize = 4_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct PrefetchedPaperContext {
    pub schema_version: u16,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub page_count: usize,
    pub author_abstract: Option<String>,
    pub abstract_page: Option<u32>,
    pub heading_candidates: Vec<String>,
    pub orientation_text: String,
    pub structure_text: String,
    pub full_document: bool,
}

impl PrefetchedPaperContext {
    #[must_use]
    pub fn from_paper(paper: &ExtractedPaper) -> Self {
        Self::with_abstract(paper, &crate::abstracts::extract(paper))
    }

    pub fn with_abstract(
        paper: &ExtractedPaper,
        abstract_result: &crate::abstracts::AbstractResult,
    ) -> Self {
        let target_start = target_start_index(paper);
        let author_abstract = abstract_result.text.clone();
        let abstract_page = abstract_result.start_page;
        let heading_candidates = heading_candidates(paper, target_start);
        let full_text = page_marked_text(paper);
        let target_window = page_marked_pages(
            paper
                .pages
                .iter()
                .skip(target_start)
                .take(12)
                .collect::<Vec<_>>()
                .as_slice(),
        );
        let full_document = full_text.chars().count() <= MAX_STRUCTURE_CHARS;
        let structure_text = if full_document {
            full_text
        } else {
            sampled_page_text(paper, MAX_STRUCTURE_CHARS)
        };
        let orientation_text = orientation_text(
            paper,
            author_abstract.as_deref(),
            &heading_candidates,
            &target_window,
        );

        Self {
            schema_version: 1,
            title: paper.metadata.title.clone(),
            authors: paper.metadata.authors.clone(),
            year: paper.metadata.year,
            page_count: paper.pages.len(),
            author_abstract,
            abstract_page,
            heading_candidates,
            orientation_text,
            structure_text,
            full_document,
        }
    }
}

#[must_use]
pub(super) fn find_author_abstract(paper: &ExtractedPaper) -> Option<(String, u32)> {
    let result = crate::abstracts::extract(paper);
    Some((result.text?, result.start_page?))
}

#[must_use]
pub(super) fn clarification_context(paper: &ExtractedPaper, selection: &str) -> String {
    let normalized_selection = compact_whitespace(selection);
    for page in &paper.pages {
        let normalized_page = compact_whitespace(&page.text);
        if let Some(start) = normalized_page.find(&normalized_selection) {
            let start = floor_char_boundary(
                &normalized_page,
                start.saturating_sub(CLARIFICATION_CONTEXT_CHARS / 2),
            );
            let end = ceil_char_boundary(
                &normalized_page,
                (start + CLARIFICATION_CONTEXT_CHARS).min(normalized_page.len()),
            );
            return format!(
                "[PDF page {}]\n{}",
                page.number,
                &normalized_page[start..end]
            );
        }
    }

    paper
        .pages
        .iter()
        .take(2)
        .map(|page| {
            format!(
                "[PDF page {}]\n{}",
                page.number,
                truncate(&page.text, 2_000)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn heading_candidates(paper: &ExtractedPaper, target_start: usize) -> Vec<String> {
    let mut headings = Vec::new();
    for page in paper.pages.iter().skip(target_start) {
        for line in page.text.lines().map(str::trim) {
            if line.is_empty() || line.chars().count() > 120 || line.split_whitespace().count() > 14
            {
                continue;
            }
            let lowered = line.to_ascii_lowercase();
            let conventional = [
                "abstract",
                "introduction",
                "background",
                "method",
                "methods",
                "results",
                "discussion",
                "limitations",
                "conclusion",
                "conclusions",
                "references",
                "appendix",
            ]
            .iter()
            .any(|candidate| lowered == *candidate || lowered.ends_with(candidate));
            let numbered = line
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
                && !line.ends_with('.');
            if conventional || numbered || (line == line.to_ascii_uppercase() && line.len() >= 4) {
                let candidate = format!("p{}: {}", page.number, compact_whitespace(line));
                if !headings.contains(&candidate) {
                    headings.push(candidate);
                }
            }
        }
    }
    headings.truncate(80);
    headings
}

fn orientation_text(
    paper: &ExtractedPaper,
    author_abstract: Option<&str>,
    headings: &[String],
    full_text: &str,
) -> String {
    let opening = truncate(full_text, ORIENTATION_EDGE_CHARS);
    let closing = truncate_from_end(full_text, ORIENTATION_EDGE_CHARS);
    format!(
        "Title: {}\nAuthors: {}\nYear: {}\nPages: {}\n\nAuthor abstract:\n{}\n\nHeading candidates:\n{}\n\nOpening excerpt:\n{}\n\nClosing excerpt:\n{}",
        paper.metadata.title,
        if paper.metadata.authors.is_empty() {
            "unknown".to_owned()
        } else {
            paper.metadata.authors.join(", ")
        },
        paper
            .metadata
            .year
            .map_or_else(|| "unknown".to_owned(), |year| year.to_string()),
        paper.pages.len(),
        author_abstract.unwrap_or("No authored abstract was located deterministically."),
        if headings.is_empty() {
            "(none)".to_owned()
        } else {
            headings.join("\n")
        },
        opening,
        closing,
    )
}

fn page_marked_text(paper: &ExtractedPaper) -> String {
    page_marked_pages(&paper.pages.iter().collect::<Vec<_>>())
}

fn page_marked_pages(pages: &[&crate::domain::ExtractedPage]) -> String {
    pages
        .iter()
        .map(|page| format!("[PDF page {}]\n{}", page.number, page.text.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn sampled_page_text(paper: &ExtractedPaper, maximum: usize) -> String {
    let per_page = (maximum / paper.pages.len().max(1)).max(1_000);
    paper
        .pages
        .iter()
        .map(|page| {
            let prefix = truncate(&page.text, per_page.saturating_mul(2) / 3);
            let suffix = truncate_from_end(&page.text, per_page / 3);
            format!(
                "[PDF page {}]\n{}\n[… deterministic middle-page truncation …]\n{}",
                page.number, prefix, suffix
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn truncate(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        value
    } else {
        &value[..floor_char_boundary(value, maximum)]
    }
}

fn truncate_from_end(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        value
    } else {
        &value[ceil_char_boundary(value, value.len() - maximum)..]
    }
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use crate::domain::{DocumentLayout, ExtractedPage, PaperMetadata};

    use super::*;

    fn paper(text: &str) -> ExtractedPaper {
        ExtractedPaper {
            metadata: PaperMetadata {
                title: "A useful paper".to_owned(),
                ..PaperMetadata::default()
            },
            pages: vec![ExtractedPage {
                number: 1,
                text: text.to_owned(),
            }],
            layout: DocumentLayout::default(),
        }
    }

    #[test]
    fn extracts_abstract_until_the_next_heading() {
        let paper = paper(
            "ABSTRACT\nThis paper gives a sufficiently long authored account of its central contribution.\n1. Introduction\nThis is not part of the abstract.",
        );
        let (abstract_text, page) = find_author_abstract(&paper).expect("abstract");
        assert_eq!(page, 1);
        assert_eq!(
            abstract_text,
            "This paper gives a sufficiently long authored account of its central contribution."
        );
    }

    #[test]
    fn extracts_inline_abstract() {
        let paper = paper(
            "Abstract—This inline abstract is long enough to be preserved exactly after whitespace normalization.\nKeywords: testing",
        );
        assert_eq!(
            find_author_abstract(&paper).map(|value| value.0).as_deref(),
            Some(
                "This inline abstract is long enough to be preserved exactly after whitespace normalization."
            )
        );
    }

    #[test]
    fn prefetch_marks_small_documents_as_complete() {
        let context = PrefetchedPaperContext::from_paper(&paper(
            "Abstract\nA long enough abstract explaining the useful contribution in full.\nIntroduction\nBody",
        ));
        assert!(context.full_document);
        assert!(context.structure_text.contains("[PDF page 1]"));
        assert!(context.author_abstract.is_some());
    }

    #[test]
    fn skips_an_abstract_before_the_target_title() {
        let paper = ExtractedPaper {
            metadata: PaperMetadata {
                title: "Target discovery".to_owned(),
                ..PaperMetadata::default()
            },
            pages: vec![
                ExtractedPage {
                    number: 1,
                    text: "Abstract\nThis belongs to an earlier article and must never be selected as the target abstract.\nIntroduction"
                        .to_owned(),
                },
                ExtractedPage {
                    number: 2,
                    text: "Target discovery\nAbstract\nThis is the target paper's authored abstract and is long enough to retain safely.\nIntroduction"
                        .to_owned(),
                },
                ExtractedPage {
                    number: 3,
                    text: "This body prose must not be appended after the introduction boundary."
                        .to_owned(),
                },
            ],
            layout: DocumentLayout::default(),
        };

        assert_eq!(
            find_author_abstract(&paper).map(|value| value.0).as_deref(),
            Some(
                "This is the target paper's authored abstract and is long enough to retain safely."
            )
        );
    }
}
