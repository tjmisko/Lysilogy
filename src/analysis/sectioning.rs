//! Shared reading-unit policy and deterministic feedback for model sectioning.
//! Size is a diagnostic, not permission to merge semantically unrelated text.

use serde::Serialize;

use crate::domain::{ExtractedPaper, SectionKind};

use super::{SectionDraft, StructureDraft, resolve_source_span};

pub(super) const POLICY: &str = include_str!("../../prompts/sectioning.md");

pub(super) fn guidance(page_count: usize) -> String {
    let pages = page_count.max(1);
    let lower = pages.div_ceil(2);
    let upper = pages.saturating_mul(2).div_ceil(3).max(lower);
    format!(
        "{POLICY}\nThis PDF contains {pages} pages. Start by considering roughly {lower}–{upper} substantive reading units, then adjust for actual topic boundaries and subtract references, appendices, or adjacent articles from the main-text budget. This is a planning guide, not a quota or a reason to drop content. A short paper may need only one unit."
    )
}

#[derive(Debug, Serialize)]
pub(super) struct SectioningReport {
    pub schema_version: u16,
    pub section_count: usize,
    pub substantive_count: usize,
    pub short_sections: Vec<String>,
    pub long_sections: Vec<String>,
    pub dependent_sections: Vec<String>,
    pub issues: Vec<String>,
}

pub(super) fn assess(paper: &ExtractedPaper, draft: &StructureDraft) -> SectioningReport {
    let substantive = draft.sections.iter().filter(|section| {
        !matches!(
            section.kind,
            SectionKind::References | SectionKind::Appendix
        )
    });
    let mut report = SectioningReport {
        schema_version: 1,
        section_count: draft.sections.len(),
        substantive_count: 0,
        short_sections: Vec::new(),
        long_sections: Vec::new(),
        dependent_sections: Vec::new(),
        issues: Vec::new(),
    };
    for section in substantive {
        report.substantive_count += 1;
        if let Some(size) = occupied_pages(paper, section) {
            let description = format!("{} ({size:.2} pages of content)", section.title);
            if size < 0.75 {
                report.short_sections.push(description);
            } else if size > 5.25 {
                report.long_sections.push(description);
            }
        }
    }
    for pair in draft.sections.windows(2) {
        let left = topic_stem(&pair[0].title);
        let right = topic_stem(&pair[1].title);
        if left.len() >= 6 && left == right {
            report
                .dependent_sections
                .push(format!("{} / {}", pair[0].title, pair[1].title));
        }
    }
    if report.substantive_count > paper.pages.len().max(1) {
        report.issues.push(format!(
            "{} substantive units across {} PDF pages is unusually fragmented; group related subheadings.",
            report.substantive_count,
            paper.pages.len()
        ));
    }
    if report.short_sections.len() >= 2
        && report.short_sections.len() * 3 >= report.substantive_count
    {
        report.issues.push(format!(
            "Many units occupy less than three quarters of a page: {}. Keep only those that are independently useful; fold dependent fragments into their topic.",
            report.short_sections.join("; ")
        ));
    }
    if !report.dependent_sections.is_empty() {
        report.issues.push(format!(
            "Adjacent parent/variant titles suggest a split topic: {}. Consolidate each family if it fits in five pages; preserve its distinctions in the digest.",
            report.dependent_sections.join("; ")
        ));
    }
    if !report.long_sections.is_empty() {
        report.issues.push(format!(
            "These units exceed the preferred five-page size: {}. Consider a meaningful internal topic boundary.",
            report.long_sections.join("; ")
        ));
    }
    report
}

fn topic_stem(title: &str) -> String {
    let normalized = title.replace(" — ", " - ").replace(" – ", " - ");
    normalized
        .split(" - ")
        .next()
        .unwrap_or(title)
        .split(": ")
        .next()
        .unwrap_or(title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn occupied_pages(paper: &ExtractedPaper, section: &SectionDraft) -> Option<f64> {
    let span = resolve_source_span(&paper.layout, section.source_span.as_ref()?)?;
    let endpoint = |number, token: u32| {
        let page = paper
            .layout
            .pages
            .iter()
            .find(|page| page.number == number)?;
        let count = u32::try_from(page.tokens.len()).ok()?.max(1);
        Some(f64::from(number - 1) + f64::from(token) / f64::from(count))
    };
    let start = endpoint(span.start.page, span.start.start_token)?;
    let end = endpoint(span.end.page, span.end.end_token.saturating_add(1))?;
    (end >= start).then_some(end - start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::SourceSpanDraft;
    use crate::domain::{
        DocumentLayout, ExtractedPage, LayoutPage, LayoutToken, PageSpan, PaperMetadata,
        SectionFamily,
    };

    fn paper() -> ExtractedPaper {
        let pages = (1..=2)
            .map(|number| LayoutPage {
                number,
                width: 612.0,
                height: 792.0,
                tokens: (0..100)
                    .map(|index| LayoutToken {
                        index,
                        text: format!("page{number}word{index}"),
                        line: index,
                        rects: Vec::new(),
                    })
                    .collect(),
                sentences: Vec::new(),
            })
            .collect::<Vec<_>>();
        ExtractedPaper {
            metadata: PaperMetadata::default(),
            pages: pages
                .iter()
                .map(|page| ExtractedPage {
                    number: page.number,
                    text: page
                        .tokens
                        .iter()
                        .map(|t| t.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .collect(),
            layout: DocumentLayout {
                schema_version: 1,
                pages,
            },
        }
    }

    fn unit(title: &str, start: u32, end: u32) -> SectionDraft {
        SectionDraft {
            title: title.to_owned(),
            kind: SectionKind::Theory,
            family: SectionFamily::Method,
            pages: PageSpan { start: 1, end: 2 },
            summary: "Summary".to_owned(),
            digest: "Digest".to_owned(),
            source_span: Some(SourceSpanDraft {
                start_text: format!("page1word{start}"),
                start_page: 1,
                end_text: format!("page2word{end}"),
                end_page: 2,
            }),
            key_quotes: Vec::new(),
            related_terms: Vec::new(),
            tile_width: 1,
            tile_height: 1,
        }
    }

    fn structure(sections: Vec<SectionDraft>) -> StructureDraft {
        StructureDraft {
            sections,
            claims: Vec::new(),
            glossary: Vec::new(),
            caveats: Vec::new(),
            reading_path: Vec::new(),
        }
    }

    #[test]
    fn a_page_break_does_not_hide_a_small_fragment() {
        let size = occupied_pages(&paper(), &unit("Mechanism", 90, 9)).expect("verified endpoints");
        assert!((size - 0.2).abs() < 0.0001);
        let mut unverifiable = unit("Mechanism", 90, 9);
        unverifiable.source_span.as_mut().unwrap().end_text = "missing source text".to_owned();
        assert!(occupied_pages(&paper(), &unverifiable).is_none());
    }

    #[test]
    fn diagnoses_parent_and_variant_fragments_but_keeps_a_coherent_topic() {
        let fragmented = structure(vec![
            unit("Extremal Goodhart", 90, 9),
            unit("Extremal Goodhart — Model Insufficiency", 90, 9),
            unit("Extremal Goodhart — Change in Regime", 90, 9),
        ]);
        let report = assess(&paper(), &fragmented);
        assert_eq!(report.dependent_sections.len(), 2);
        assert_eq!(report.short_sections.len(), 3);
        assert!(!report.issues.is_empty());
        let coherent = assess(&paper(), &structure(vec![unit("Extremal Goodhart", 0, 99)]));
        assert!(coherent.issues.is_empty());
        assert!(coherent.short_sections.is_empty());
    }

    #[test]
    fn references_do_not_inflate_the_main_text_budget() {
        let mut sections = vec![unit("Mechanism", 0, 99)];
        let mut references = unit("References", 90, 9);
        references.kind = SectionKind::References;
        sections.push(references);
        let report = assess(&paper(), &structure(sections));
        assert_eq!(report.substantive_count, 1);
        assert!(report.issues.is_empty());
    }
}
