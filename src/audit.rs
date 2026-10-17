//! Deterministic extraction inventory. Reports reading-index counts and mangle
//! signals. Never calls a model.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::source_index::{Provenance, ReadingIndex};

const SPARSE_PAGE_TOKENS: usize = 20;
const OVERSIZED_PARAGRAPH_CHARS: usize = 4_000;
const LOW_OCR_CONFIDENCE: f32 = 0.6;

#[derive(Clone, Debug, Default, Serialize)]
pub struct ExtractionInventory {
    pub schema_version: u16,
    pub pages: usize,
    pub native_pages: usize,
    pub ocr_pages: usize,
    pub unavailable_pages: usize,
    pub pages_without_text: usize,
    pub sparse_pages: usize,
    pub text_chars: usize,
    pub tokens: usize,
    pub words: usize,
    pub sentences: usize,
    pub paragraphs: usize,
    pub paragraphs_by_kind: BTreeMap<String, usize>,
    pub oversized_paragraphs: usize,
    pub figures: usize,
    pub figures_by_kind: BTreeMap<String, usize>,
    pub figure_references: usize,
    pub duplicate_figure_ids: usize,
    pub gaps: usize,
    pub gaps_by_reason: BTreeMap<String, usize>,
    pub signals: Vec<ExtractionSignal>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExtractionSignal {
    pub code: &'static str,
    pub severity: Severity,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Degraded,
    Unreadable,
}

/// Count and classify one reading index. Pure and deterministic.
#[must_use]
pub fn inventory(index: &ReadingIndex) -> ExtractionInventory {
    let mut report = ExtractionInventory {
        schema_version: index.schema_version,
        text_chars: index.text.chars().count(),
        tokens: index.tokens.len(),
        words: index.objects.word.len(),
        sentences: index.objects.sentence.len(),
        paragraphs: index.objects.paragraph.len(),
        figures: index.figures.len(),
        gaps: index.gaps.len(),
        ..ExtractionInventory::default()
    };
    let tokens_by_page = tokens_per_page(index);
    count_pages(index, &tokens_by_page, &mut report);
    count_paragraphs(index, &mut report);
    count_figures(index, &mut report);
    count_gaps(index, &mut report);
    summarize(index, &mut report);
    report
}

fn tokens_per_page(index: &ReadingIndex) -> BTreeMap<u32, usize> {
    let mut counts = BTreeMap::new();
    for token in &index.tokens {
        *counts.entry(token.page).or_default() += 1;
    }
    counts
}

fn count_pages(
    index: &ReadingIndex,
    tokens_by_page: &BTreeMap<u32, usize>,
    report: &mut ExtractionInventory,
) {
    report.pages = index.pages.len();
    for page in &index.pages {
        match page.provenance {
            Provenance::Native => report.native_pages += 1,
            Provenance::Ocr => report.ocr_pages += 1,
            Provenance::Unavailable => report.unavailable_pages += 1,
        }
        if page.end <= page.start {
            report.pages_without_text += 1;
        }
        let tokens = tokens_by_page.get(&page.number).copied().unwrap_or(0);
        if page.end > page.start && tokens < SPARSE_PAGE_TOKENS {
            report.sparse_pages += 1;
        }
        if page.provenance == Provenance::Ocr
            && page
                .confidence
                .is_none_or(|confidence| confidence < LOW_OCR_CONFIDENCE)
        {
            report.signals.push(ExtractionSignal {
                code: "low_confidence_ocr",
                severity: Severity::Degraded,
                detail: format!("page {}", page.number),
            });
        }
    }
}

fn count_paragraphs(index: &ReadingIndex, report: &mut ExtractionInventory) {
    for paragraph in &index.objects.paragraph {
        *report
            .paragraphs_by_kind
            .entry(paragraph.kind.clone())
            .or_default() += 1;
        let chars = paragraph.end.saturating_sub(paragraph.start);
        if chars > OVERSIZED_PARAGRAPH_CHARS {
            report.oversized_paragraphs += 1;
            report.signals.push(ExtractionSignal {
                code: "oversized_paragraph",
                severity: Severity::Degraded,
                detail: format!("{chars} chars at {}", paragraph.start),
            });
        }
    }
}

fn count_figures(index: &ReadingIndex, report: &mut ExtractionInventory) {
    let mut figure_ids = BTreeSet::new();
    for figure in &index.figures {
        *report
            .figures_by_kind
            .entry(figure.kind.clone())
            .or_default() += 1;
        report.figure_references += figure.references.len();
        if !figure_ids.insert(figure.id.as_str()) {
            report.duplicate_figure_ids += 1;
            report.signals.push(ExtractionSignal {
                code: "duplicate_figure_id",
                severity: Severity::Degraded,
                detail: figure.id.clone(),
            });
        }
    }
}

fn count_gaps(index: &ReadingIndex, report: &mut ExtractionInventory) {
    for gap in &index.gaps {
        *report.gaps_by_reason.entry(gap.reason.clone()).or_default() += 1;
    }
}

fn summarize(index: &ReadingIndex, report: &mut ExtractionInventory) {
    if index.text.trim().is_empty() {
        report.signals.push(ExtractionSignal {
            code: "empty_text",
            severity: Severity::Unreadable,
            detail: "reading index has no text".into(),
        });
    }
    if report.pages_without_text > 0 {
        report.signals.push(ExtractionSignal {
            code: "page_without_text",
            severity: Severity::Degraded,
            detail: format!("{} pages", report.pages_without_text),
        });
    }
    if report.unavailable_pages > 0 {
        report.signals.push(ExtractionSignal {
            code: "unavailable_page",
            severity: Severity::Unreadable,
            detail: format!("{} pages", report.unavailable_pages),
        });
    }
    if report.sparse_pages > 0 {
        report.signals.push(ExtractionSignal {
            code: "sparse_page",
            severity: Severity::Degraded,
            detail: format!(
                "{} pages under {SPARSE_PAGE_TOKENS} tokens",
                report.sparse_pages
            ),
        });
    }
    if report.gaps > 0 {
        report.signals.push(ExtractionSignal {
            code: "index_gap",
            severity: Severity::Degraded,
            detail: format!("{} gaps", report.gaps),
        });
    }
}

/// The worst severity in an inventory, or `None` when no signal fired.
#[must_use]
pub fn worst_severity(report: &ExtractionInventory) -> Option<Severity> {
    report
        .signals
        .iter()
        .map(|signal| signal.severity)
        .max_by_key(|severity| match severity {
            Severity::Degraded => 0,
            Severity::Unreadable => 1,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::{IndexGap, ReadingPage, ReadingToken, TextObjects};

    fn page(number: u32, provenance: Provenance, text: bool) -> ReadingPage {
        ReadingPage {
            number,
            width: 612.0,
            height: 792.0,
            start: 0,
            end: usize::from(text) * 100,
            provenance,
            confidence: None,
        }
    }

    #[test]
    fn should_flag_empty_text_as_unreadable() {
        let index = ReadingIndex {
            schema_version: 6,
            text: String::new(),
            pages: vec![page(1, Provenance::Unavailable, false)],
            tokens: vec![],
            objects: TextObjects::default(),
            figures: vec![],
            gaps: vec![],
        };
        let report = inventory(&index);
        assert_eq!(worst_severity(&report), Some(Severity::Unreadable));
        assert!(
            report
                .signals
                .iter()
                .any(|signal| signal.code == "empty_text")
        );
    }

    #[test]
    fn should_count_figures_and_references() {
        let index = ReadingIndex {
            schema_version: 6,
            text: "x".repeat(100),
            pages: vec![page(1, Provenance::Native, true)],
            tokens: (0..25)
                .map(|_| ReadingToken {
                    start: 0,
                    end: 1,
                    page: 1,
                    rects: vec![],
                    text: "x".into(),
                    provenance: Provenance::Native,
                })
                .collect(),
            objects: TextObjects::default(),
            figures: vec![],
            gaps: vec![IndexGap {
                page: 2,
                reason: "ocr_unavailable".into(),
            }],
        };
        let report = inventory(&index);
        assert_eq!(report.pages, 1);
        assert_eq!(report.tokens, 25);
        assert_eq!(report.sparse_pages, 0);
        assert_eq!(report.gaps_by_reason.get("ocr_unavailable"), Some(&1));
    }
}
