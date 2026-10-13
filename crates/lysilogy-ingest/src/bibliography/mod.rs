//! Deterministic bibliography evidence extracted from a paper's reading text.
//!
//! Structured fields are fallible interpretations of retained authored text.
//! A unique printed citation key establishes a local destination, never a Work
//! or Person identity. All public offsets are UTF-16 code units of
//! [`SourceText::text`], matching the reading index and the frontend.
//!
//! Extraction is pure: no filesystem, models, network, or environment.

mod entries;
mod fields;
mod mentions;
mod source;

use lysilogy_core::CitationStyle;
use serde::{Deserialize, Serialize};

pub use fields::{BibliographicFields, FieldConfidence, ParsedField, parse_fields};

/// Version tag recorded with every extraction.
pub const EXTRACTOR_VERSION: &str = "bib-v1";

/// A half-open range of UTF-16 code units.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
}

/// A rectangle in page coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageRect {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

/// One source word with its page geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceToken {
    pub start: usize,
    pub end: usize,
    pub page: u32,
    pub rects: Vec<PageRect>,
    pub text: String,
}

/// The text range of one page.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourcePage {
    pub number: u32,
    pub start: usize,
    pub end: usize,
}

/// A paragraph-level block; `kind` is one of the reading index kinds
/// (`heading`, `caption`, `body`, `list`, ...). Empty `spans` means the whole
/// range is one contiguous member.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceParagraph {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub spans: Vec<TextSpan>,
}

/// Plain-data mirror of the reading-index fields the extractor needs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceText {
    pub text: String,
    pub pages: Vec<SourcePage>,
    pub tokens: Vec<SourceToken>,
    pub paragraphs: Vec<SourceParagraph>,
    pub sentences: Vec<TextSpan>,
}

/// One bibliography entry with its printed text, parsed fields, and location.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntry {
    /// 0-based order in the bibliography.
    pub ordinal: usize,
    /// Members trimmed and joined by `\n`.
    pub raw_text: String,
    pub fields: BibliographicFields,
    pub page: u32,
    /// First member start to last member end.
    pub span: TextSpan,
    pub member_spans: Vec<TextSpan>,
    /// Union of token rects on `page`.
    pub region: Option<PageRect>,
}

/// One in-text citation marker that resolved to a unique entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedMention {
    pub entry_ordinal: usize,
    pub style: CitationStyle,
    pub marker: TextSpan,
    pub marker_text: String,
    pub page: u32,
    pub rects: Vec<PageRect>,
    pub passage: TextSpan,
    /// Whitespace-collapsed passage text.
    pub passage_text: String,
}

/// Why a marker did not resolve to exactly one entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    MissingTarget,
    AmbiguousTarget,
    UnsupportedRange,
    AmbiguousMarker,
}

/// A marker kept as evidence but never linked.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedMention {
    pub text: String,
    pub key: String,
    pub style: CitationStyle,
    pub reason: UnresolvedReason,
    /// Evidence candidates, never selected destinations.
    pub candidate_ordinals: Vec<usize>,
    pub marker: TextSpan,
    pub page: u32,
}

/// Everything extracted from one paper.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bibliography {
    pub entries: Vec<ExtractedEntry>,
    pub mentions: Vec<ExtractedMention>,
    pub unresolved: Vec<UnresolvedMention>,
}

/// Extract bibliography evidence and in-text mentions from reading text; no
/// I/O, no environment. Without entries there is nothing to cite, so no
/// mentions are reported.
#[must_use]
pub fn extract(text: &SourceText) -> Bibliography {
    let source = source::Source::new(text);
    let (entries, region) = entries::extract_entries(&source);
    if entries.is_empty() {
        return Bibliography::default();
    }
    let (mentions, unresolved) = mentions::resolve_mentions(&source, region, &entries);
    Bibliography {
        entries,
        mentions,
        unresolved,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::source::pattern;
    use super::*;

    /// Build a source text from `(text, paragraph kind, page)` blocks. Each
    /// block is one paragraph and one sentence; tokens get a fixed grid of
    /// rects, one row per block.
    pub fn fixture(blocks: &[(&str, &str, u32)]) -> SourceText {
        let mut index = SourceText::default();
        for (at, (text, kind, page)) in blocks.iter().enumerate() {
            let start = index.text.encode_utf16().count();
            let y = f32::from(u16::try_from(at).unwrap()).mul_add(30.0, 40.0);
            for word in pattern!(r"\S+").find_iter(text) {
                let left = text[..word.start()].encode_utf16().count();
                let right = text[..word.end()].encode_utf16().count();
                index.tokens.push(SourceToken {
                    start: start + left,
                    end: start + right,
                    page: *page,
                    text: word.as_str().to_owned(),
                    rects: vec![PageRect {
                        x_min: f32::from(u16::try_from(left).unwrap()).mul_add(6.0, 40.0),
                        x_max: f32::from(u16::try_from(right).unwrap()).mul_add(6.0, 40.0),
                        y_min: y,
                        y_max: y + 12.0,
                    }],
                });
            }
            index.text.push_str(text);
            let end = index.text.encode_utf16().count();
            index.paragraphs.push(SourceParagraph {
                start,
                end,
                kind: (*kind).to_owned(),
                spans: vec![],
            });
            index.sentences.push(TextSpan { start, end });
            if let Some(existing) = index
                .pages
                .iter_mut()
                .find(|existing| existing.number == *page)
            {
                existing.end = end;
            } else {
                index.pages.push(SourcePage {
                    number: *page,
                    start,
                    end,
                });
            }
            index.text.push_str("\n\n");
        }
        index
    }

    fn slice(index: &SourceText, span: TextSpan) -> String {
        String::from_utf16(&index.text.encode_utf16().collect::<Vec<_>>()[span.start..span.end])
            .unwrap()
    }

    fn member_pages(index: &SourceText, entry: &ExtractedEntry) -> Vec<u32> {
        entry
            .member_spans
            .iter()
            .map(|span| source::Source::new(index).anchor(*span).0)
            .collect()
    }

    /// Every entry has exactly one resolved mention.
    fn assert_each_entry_cited_once(result: &Bibliography) {
        for entry in &result.entries {
            let count = result
                .mentions
                .iter()
                .filter(|mention| mention.entry_ordinal == entry.ordinal)
                .count();
            assert_eq!(count, 1, "entry {}", entry.ordinal);
        }
    }

    fn mentions_of(result: &Bibliography, ordinal: usize) -> Vec<&ExtractedMention> {
        result
            .mentions
            .iter()
            .filter(|mention| mention.entry_ordinal == ordinal)
            .collect()
    }

    fn rect_after(before: PageRect) -> PageRect {
        PageRect {
            x_min: before.x_max + 1.0,
            x_max: before.x_max + 5.0,
            y_min: before.y_min - 3.0,
            y_max: before.y_max - 5.0,
        }
    }

    #[test]
    fn should_split_numbered_entries_when_keys_are_bracketed() {
        let index = fixture(&[
            (
                "😀 Evidence [1, 3–4] and (2) confirms this. Equation (99) is unrelated.",
                "body",
                1,
            ),
            ("References", "heading", 2),
            (
                "[1] Adams. First work. 2020.\n[2] Baker. Second work. 2021.",
                "body",
                2,
            ),
            ("[3] Chen. Third work. 2022.", "body", 3),
            ("[4] Diaz. Fourth work. 2023.", "body", 3),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 4);
        for (at, entry) in result.entries.iter().enumerate() {
            assert_eq!(entry.fields.printed_key.value, Some((at + 1).to_string()));
            assert_eq!(entry.ordinal, at);
        }
        assert_each_entry_cited_once(&result);
        assert!(result.mentions.iter().all(|mention| mention.page == 1));
        assert!(
            result
                .mentions
                .iter()
                .all(|mention| !mention.rects.is_empty())
        );
        assert!(result.unresolved.is_empty());
        assert_eq!(slice(&index, mentions_of(&result, 1)[0].marker), "(2)");
    }

    #[test]
    fn should_keep_printed_conventions_when_entries_use_dots_parentheses_or_keys() {
        let index = fixture(&[
            ("See [12], (13), and [AB20].", "body", 1),
            ("BIBLIOGRAPHY", "heading", 2),
            (
                "12. Jones. A work. 2020.\n(13) Davis. Another work. 2021.\n[AB20] Adams and Baker. Title. 2020.",
                "body",
                2,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 3);
        assert_each_entry_cited_once(&result);
        assert_eq!(
            result.entries[2].fields.printed_key.value.as_deref(),
            Some("AB20")
        );
    }

    #[test]
    fn should_split_author_year_entries_when_no_printed_keys_exist() {
        let index = fixture(&[
            (
                "Smith (2020a,b) and (Jones & Brown, 2018); García et al., 2019; Smith, 2021.",
                "body",
                1,
            ),
            ("References", "heading", 2),
            (
                "Smith, A. (2020a). First study.\nSmith, A. (2020b). Second study.",
                "body",
                2,
            ),
            ("Smith, A. (2021). Followup.", "body", 2),
            ("Jones, C. and Brown, D. (2018). Joint study.", "body", 2),
            ("García, E., Patel, R. (2019). Team study.", "body", 3),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 5);
        assert_each_entry_cited_once(&result);
        assert_eq!(
            result.entries[0].fields.title.value.as_deref(),
            Some("First study")
        );
        assert_eq!(
            result.entries[3].fields.authors.value.as_ref().unwrap(),
            &["Jones, C.", "Brown, D."]
        );
        assert_eq!(
            result.entries[4].fields.authors.value.as_ref().unwrap(),
            &["García, E.", "Patel, R."]
        );
    }

    #[test]
    fn should_extract_a_doi_when_it_wraps_across_lines() {
        for (raw, expected) in [
            ("doi: 10.1234/\n  AbC.567.", "10.1234/AbC.567"),
            ("https://doi.org/10.1000/ABC-\n123", "10.1000/ABC-123"),
            (
                "(doi:10.1002/(SICI)1099-0844(199912)17:4<290::AID-CBF849>3.0.CO;2-P)",
                "10.1002/(SICI)1099-0844(199912)17:4<290::AID-CBF849>3.0.CO;2-P",
            ),
            ("https://doi.org/10.1000/a(b).", "10.1000/a(b)"),
        ] {
            assert_eq!(
                parse_fields(raw, None).doi.value.as_deref(),
                Some(expected),
                "{raw}"
            );
        }
    }

    #[test]
    fn should_preserve_identifiers_when_arxiv_versions_and_legacy_ids_are_printed() {
        for (raw, expected) in [
            ("arXiv:2301.12345v3", "2301.12345v3"),
            (
                "https://arxiv.org/abs/hep-th/9901001v2.",
                "hep-th/9901001v2",
            ),
            ("https://arxiv.org/pdf/0712.1234.pdf", "0712.1234"),
        ] {
            assert_eq!(
                parse_fields(raw, None).arxiv_id.value.as_deref(),
                Some(expected)
            );
        }
        assert_eq!(
            parse_fields("We measured 2301.12345 samples.", None)
                .arxiv_id
                .value,
            None
        );
    }

    #[test]
    fn should_retain_raw_members_when_a_keyed_entry_wraps_across_pages() {
        let index = fixture(&[
            ("See [1].", "body", 1),
            ("References", "heading", 2),
            ("[1] Smith, A. (2020). A long", "body", 2),
            ("title. Journal of Results. doi:10.1000/abc.", "body", 3),
            ("[2] Jones. Another title. 2021.", "body", 3),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        let first = &result.entries[0];
        assert!(first.raw_text.contains("A long\ntitle"));
        assert_eq!(member_pages(&index, first), [2, 3]);
        assert_eq!(first.fields.title.value.as_deref(), Some("A long title"));
        assert_eq!(
            first.fields.venue.value.as_deref(),
            Some("Journal of Results")
        );
        assert_eq!(first.fields.doi.value.as_deref(), Some("10.1000/abc"));
    }

    #[test]
    fn should_keep_field_confidence_separate_when_only_some_metadata_is_present() {
        let fields = parse_fields(
            "[7] Smith, A. (2020). “A quoted title.” Journal of Tests, 5(2).",
            Some("7".into()),
        );
        assert_eq!(fields.printed_key.confidence, FieldConfidence::Explicit);
        assert_eq!(fields.title.value.as_deref(), Some("A quoted title"));
        assert_eq!(fields.title.confidence, FieldConfidence::Heuristic);
        assert_eq!(fields.doi.confidence, FieldConfidence::Missing);
        assert_eq!(fields.doi.value, None);
        assert_eq!(
            fields.venue.value.as_deref(),
            Some("Journal of Tests, 5(2)")
        );
    }

    #[test]
    fn should_exclude_floating_captions_when_bibliography_members_are_disjoint() {
        let mut index = fixture(&[
            ("References", "heading", 1),
            ("[1] Smith, A. (2020). A long", "body", 1),
            ("Figure 1: Floating material.", "caption", 1),
            ("title. Journal of Results.", "body", 1),
        ]);
        let first = index.paragraphs[1].clone();
        let continuation = index.paragraphs.pop().unwrap();
        index.paragraphs[1].end = continuation.end;
        index.paragraphs[1].spans = vec![
            TextSpan {
                start: first.start,
                end: first.end,
            },
            TextSpan {
                start: continuation.start,
                end: continuation.end,
            },
        ];
        let result = extract(&index);
        assert_eq!(result.entries.len(), 1);
        assert!(!result.entries[0].raw_text.contains("Floating"));
        assert_eq!(
            result.entries[0].fields.title.value.as_deref(),
            Some("A long title")
        );
        assert_eq!(result.entries[0].member_spans.len(), 2);
    }

    #[test]
    fn should_leave_the_year_missing_when_only_an_identifier_contains_four_digits() {
        for raw in [
            "doi:10.2020/example",
            "https://example.org/2020/work",
            "arXiv:2020.12345",
        ] {
            assert_eq!(parse_fields(raw, None).year.value, None, "{raw}");
        }
    }

    #[test]
    fn should_withhold_identifier_fields_when_an_entry_prints_conflicting_identifiers() {
        let fields = parse_fields(
            "doi:10.1000/first; doi:10.1000/second. arXiv:2301.12345; arXiv:2302.12345.",
            None,
        );
        assert_eq!(fields.doi.value, None);
        assert_eq!(fields.arxiv_id.value, None);
        assert_eq!(fields.doi.confidence, FieldConfidence::Missing);
    }

    #[test]
    fn should_keep_the_doi_boundary_when_the_next_line_is_following_prose() {
        let raw = "[1] Smith, A. (2020). A title. doi:10.1000/ABC\nAvailable online";
        assert_eq!(
            parse_fields(raw, Some("1".into())).doi.value.as_deref(),
            Some("10.1000/ABC")
        );
        assert_eq!(parse_fields("doi:10.1000/ABC\nDEF", None).doi.value, None);
        assert_eq!(
            parse_fields("doi:10.1000/ABC\n123", None)
                .doi
                .value
                .as_deref(),
            Some("10.1000/ABC123")
        );
    }

    #[test]
    fn should_join_unnumbered_continuations_when_a_later_plain_year_starts_the_next_entry() {
        let index = fixture(&[
            ("Smith (2020) and Jones (2021) agree.", "body", 1),
            ("References", "heading", 2),
            ("Smith, A. (2020). A long", "body", 2),
            ("title. Journal of Results. doi:10.1000/ABC.", "body", 3),
            ("Jones, B. 2021. Next title.", "body", 3),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        let first = &result.entries[0];
        assert_eq!(member_pages(&index, first), [2, 3]);
        assert_eq!(first.fields.title.value.as_deref(), Some("A long title"));
        assert_eq!(first.fields.doi.value.as_deref(), Some("10.1000/ABC"));
        assert_eq!(
            result.entries[1].fields.title.value.as_deref(),
            Some("Next title")
        );
        assert_eq!(result.entries[1].fields.year.value.as_deref(), Some("2021"));
    }

    #[test]
    fn should_preserve_the_next_entry_when_a_block_starts_with_a_continuation() {
        let index = fixture(&[
            ("Smith (2020) and Jones (2021) agree.", "body", 1),
            ("References", "heading", 2),
            ("Smith, A. (2020). A long", "body", 2),
            (
                "title. Journal of Results.\nJones, B. 2021. Next title.",
                "body",
                3,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        assert_eq!(slice(&index, result.mentions[0].marker), "Smith (2020)");
        assert_eq!(slice(&index, result.mentions[1].marker), "Jones (2021)");
        let first = &result.entries[0];
        let second = &result.entries[1];
        assert_eq!(first.fields.title.value.as_deref(), Some("A long title"));
        assert_eq!(second.fields.title.value.as_deref(), Some("Next title"));
        assert_eq!(second.fields.year.value.as_deref(), Some("2021"));
        assert_eq!(first.member_spans.len(), 2);
        assert!(first.span.end <= second.span.start);
    }

    #[test]
    fn should_retain_heading_classified_members_when_a_numbered_entry_wraps_before_its_year() {
        let index = fixture(&[
            ("Compare [1] and [2].", "body", 1),
            ("References", "heading", 2),
            ("[1] Example, A. and", "body", 2),
            ("Sample, B. (2020). A long", "heading", 2),
            ("comparative title. Journal of Results.", "heading", 2),
            ("[2] Other, C. (2021). Independent work.", "heading", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        let first = &result.entries[0];
        assert_eq!(first.member_spans.len(), 3);
        assert_eq!(
            first.fields.authors.value.as_deref(),
            Some(["Example, A.".to_owned(), "Sample, B.".to_owned()].as_slice())
        );
        assert_eq!(
            first.fields.title.value.as_deref(),
            Some("A long comparative title")
        );
        assert_eq!(
            first.fields.venue.value.as_deref(),
            Some("Journal of Results")
        );
        assert_eq!(first.fields.year.value.as_deref(), Some("2020"));
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn should_split_merged_paragraph_entries_when_printed_keys_begin_distinct_physical_lines() {
        let raw = "[1] Example, A. (2020). First 😀 title citing [9]. [2] Sample, B. (2021). Second title.";
        let mut index = fixture(&[
            ("Compare [1] and [2].", "body", 1),
            ("References", "heading", 2),
            (raw, "body", 2),
        ]);
        let byte = index.text.find("[2] Sample").unwrap();
        let boundary = index.text[..byte].encode_utf16().count();
        let first = index
            .tokens
            .iter()
            .find(|token| token.start == boundary)
            .unwrap()
            .rects[0];
        for token in index
            .tokens
            .iter_mut()
            .filter(|token| token.start >= boundary)
        {
            token.rects[0].x_min -= first.x_min - 40.0;
            token.rects[0].x_max -= first.x_min - 40.0;
            token.rects[0].y_min += 20.0;
            token.rects[0].y_max += 20.0;
        }
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        assert!(result.unresolved.is_empty());
        assert_eq!(
            result.entries[0].fields.printed_key.value.as_deref(),
            Some("1")
        );
        assert_eq!(
            result.entries[1].fields.printed_key.value.as_deref(),
            Some("2")
        );
        assert_eq!(result.entries[1].span.start, boundary);
        assert_eq!(
            slice(&index, result.entries[0].member_spans[0]).trim(),
            raw[..raw.find("[2] Sample").unwrap()].trim()
        );
        assert!(result.entries[0].raw_text.contains("citing [9]"));
    }

    #[test]
    fn should_keep_inline_keys_inside_the_entry_when_line_geometry_is_missing_or_raised() {
        for variant in 0..4 {
            let mut index = fixture(&[
                ("See [1].", "body", 1),
                ("References", "heading", 2),
                (
                    "[1] Example, A. (2020). A title mentioning [2] as inline notation.",
                    "body",
                    2,
                ),
            ]);
            let token = index
                .tokens
                .iter_mut()
                .find(|token| token.text == "[2]")
                .unwrap();
            match variant {
                0 => {}
                1 => token.rects.clear(),
                2 => {
                    token.rects[0].y_min -= 24.0;
                    token.rects[0].y_max = token.rects[0].y_min + 4.0;
                }
                _ => token.rects[0].x_min = f32::NAN,
            }
            let result = extract(&index);
            assert_eq!(result.entries.len(), 1, "variant {variant}");
            assert_each_entry_cited_once(&result);
            assert_eq!(
                result.entries[0].fields.printed_key.value.as_deref(),
                Some("1")
            );
            assert!(
                result.entries[0]
                    .raw_text
                    .contains("[2] as inline notation")
            );
        }
    }

    #[test]
    fn should_split_printed_entries_when_a_page_transition_has_no_word_rectangles() {
        let mut index = fixture(&[
            ("See [1] and [2].", "body", 1),
            ("References", "heading", 2),
            (
                "[1] First, A. (2020). Earlier work. [2] Later, B. (2021). Next work.",
                "body",
                2,
            ),
        ]);
        let byte = index.text.find("[2] Later").unwrap();
        let boundary = index.text[..byte].encode_utf16().count();
        for token in index
            .tokens
            .iter_mut()
            .filter(|token| token.start >= boundary)
        {
            token.page = 3;
            token.rects.clear();
        }
        let mut third = index.pages[1];
        third.number = 3;
        third.start = boundary;
        index.pages[1].end = boundary;
        index.pages.push(third);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        assert_eq!(result.entries[1].page, 3);
        assert_eq!(result.entries[1].span.start, boundary);
        assert!(result.entries[1].region.is_none());
    }

    #[test]
    fn should_retain_wrapped_publication_years_when_bracketed_entries_continue_on_new_lines() {
        let index = fixture(&[
            ("See [4] and [5].", "body", 1),
            ("References", "heading", 2),
            ("[4] Ada Example and Bea Sample.", "body", 2),
            ("2021. A complete title. Journal of Results.", "list", 2),
            ("[5] Carl Other.\n2022. A second title.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        assert_eq!(
            result.entries[0].fields.title.value.as_deref(),
            Some("A complete title")
        );
        assert_eq!(result.entries[0].fields.year.value.as_deref(), Some("2021"));
        assert_eq!(result.entries[1].fields.year.value.as_deref(), Some("2022"));
    }

    #[test]
    fn should_preserve_four_digit_keys_when_explicit_brackets_or_an_adjacent_sequence_support_them()
    {
        let index = fixture(&[
            ("See [1799], [1800], and [2021].", "body", 1),
            ("References", "heading", 2),
            ("1799. Example, A. (2020). Earlier work.", "body", 2),
            (
                "1800. Sample, B. (2021). Next work.\n[2021] Other, C. (2022). A keyed work.",
                "body",
                2,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 3);
        assert_each_entry_cited_once(&result);
        assert_eq!(
            result.entries[1].fields.printed_key.value.as_deref(),
            Some("1800")
        );
        assert_eq!(
            result.entries[2].fields.printed_key.value.as_deref(),
            Some("2021")
        );
    }

    #[test]
    fn should_preserve_terminal_title_punctuation_when_the_venue_follows_a_question_or_exclamation()
    {
        for (raw, title, venue) in [
            (
                "Example, A. (2020). What Can Models Learn? Journal of Results.",
                "What Can Models Learn?",
                Some("Journal of Results"),
            ),
            (
                "Example, A. (2020). Careful With That Baseline! Studies in Evaluation.",
                "Careful With That Baseline!",
                Some("Studies in Evaluation"),
            ),
            ("Example, A. (2020). Is There More?", "Is There More?", None),
            (
                "Example, A. (2020). An ordinary title. Journal of Results.",
                "An ordinary title",
                Some("Journal of Results"),
            ),
        ] {
            let fields = parse_fields(raw, None);
            assert_eq!(fields.title.value.as_deref(), Some(title), "{raw}");
            assert_eq!(fields.venue.value.as_deref(), venue, "{raw}");
        }
    }

    #[test]
    fn should_retain_mixed_author_year_entries_when_an_explicit_author_follows_a_complete_numbered_entry()
     {
        let index = fixture(&[
            ("See [1] and Other (2021).", "body", 1),
            ("References", "heading", 2),
            ("[1] Example, A. (2020). Complete title.", "body", 2),
            ("Journal of Results, 2020, 12–18.", "body", 2),
            ("Other, B. (2021). Independent author-year work.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_each_entry_cited_once(&result);
        assert_eq!(result.entries[0].member_spans.len(), 2);
        assert!(result.entries[0].raw_text.contains("Journal of Results"));
        assert_eq!(result.entries[1].fields.printed_key.value, None);
        assert_eq!(
            result.entries[1].fields.authors.value,
            Some(vec!["Other, B.".to_owned()])
        );
    }

    #[test]
    fn should_recognize_year_shaped_first_keys_when_lookahead_confirms_an_independent_dotted_sequence()
     {
        for grouped in [false, true] {
            let mut blocks = vec![
                ("See [2020] and [2021].", "body", 1),
                ("References", "heading", 2),
            ];
            if grouped {
                blocks.push((
                    "2020. Adams. Earlier study.\n2021. Baker. Later study.",
                    "body",
                    2,
                ));
            } else {
                blocks.push(("2020. Adams. Earlier study.", "body", 2));
                blocks.push(("2021. Baker. Later study.", "body", 2));
            }
            let result = extract(&fixture(&blocks));
            assert_eq!(result.entries.len(), 2, "grouped {grouped}");
            assert_each_entry_cited_once(&result);
            assert_eq!(
                result.entries[0].fields.printed_key.value.as_deref(),
                Some("2020")
            );
            assert_eq!(
                result.entries[1].fields.printed_key.value.as_deref(),
                Some("2021")
            );
        }
        let index = fixture(&[
            ("See [1].", "body", 1),
            ("References", "heading", 2),
            ("[1] Example, A. (2019). An annual review.", "body", 2),
            ("2020. Annual Review of Examples.", "body", 2),
            ("2021. An updated edition.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].member_spans.len(), 3);
    }

    #[test]
    fn should_withhold_truncated_arxiv_dois_when_a_line_break_leaves_an_incomplete_structured_suffix()
     {
        for (doi, expected) in [
            ("10.48550/arXiv.2102. 04906", None),
            ("10.48550/arXiv.2102.\n04906", None),
            ("10.48550/arXiv.2102", None),
            (
                "10.48550/arXiv.2102.04906",
                Some("10.48550/arXiv.2102.04906"),
            ),
            (
                "10.48550/arXiv.2102.04906v2",
                Some("10.48550/arXiv.2102.04906v2"),
            ),
            (
                "10.48550/arXiv.hep-th/9901001",
                Some("10.48550/arXiv.hep-th/9901001"),
            ),
            ("10.1000/example.2102", Some("10.1000/example.2102")),
        ] {
            let raw = format!("[1] Example, A. (2021). A study. doi:{doi}");
            let index = fixture(&[
                ("See [1].", "body", 1),
                ("References", "heading", 2),
                (&raw, "body", 2),
            ]);
            let result = extract(&index);
            assert_eq!(result.entries.len(), 1);
            assert_each_entry_cited_once(&result);
            assert_eq!(result.entries[0].raw_text, raw);
            assert_eq!(
                result.entries[0].fields.doi.value.as_deref(),
                expected,
                "{doi}"
            );
            assert_eq!(
                result.entries[0].fields.doi.confidence,
                if expected.is_some() {
                    FieldConfidence::Explicit
                } else {
                    FieldConfidence::Missing
                }
            );
        }
    }

    #[test]
    fn should_return_no_entries_when_no_references_heading_exists() {
        let result = extract(&fixture(&[
            ("See [1], (2) and Smith (2020).", "body", 1),
            ("1. A list step.", "body", 1),
        ]));
        assert!(result.entries.is_empty());
        assert!(result.mentions.is_empty());
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn should_return_no_entries_when_source_text_is_empty() {
        assert_eq!(extract(&SourceText::default()), Bibliography::default());
    }

    #[test]
    fn should_ignore_offsets_when_spans_fall_inside_a_surrogate_pair() {
        let mut index = fixture(&[
            ("😀 References", "heading", 1),
            ("[1] Smith. Title.", "body", 1),
        ]);
        index.paragraphs[0].start = 1;
        assert!(extract(&index).entries.is_empty());

        // An astral character at a member boundary must not panic either.
        let mut index = fixture(&[
            ("References", "heading", 1),
            ("[1] Smith. 😀 Title.", "body", 1),
        ]);
        let emoji = index.text.find('😀').unwrap();
        let inside = index.text[..emoji].encode_utf16().count() + 1;
        index.paragraphs[1].spans = vec![TextSpan {
            start: index.paragraphs[1].start,
            end: inside,
        }];
        assert!(extract(&index).entries.is_empty());
    }

    #[test]
    fn should_skip_paragraph_spans_when_offsets_exceed_text_length() {
        let mut index = fixture(&[
            ("References", "heading", 1),
            ("[1] Smith. Title. 2020.", "body", 1),
        ]);
        let length = index.text.encode_utf16().count();
        index.paragraphs.push(SourceParagraph {
            start: length + 5,
            end: length + 50,
            kind: "body".into(),
            spans: vec![],
        });
        index.paragraphs.push(SourceParagraph {
            start: 0,
            end: 0,
            kind: "body".into(),
            spans: vec![TextSpan {
                start: length,
                end: length + 10,
            }],
        });
        let result = extract(&index);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].fields.printed_key.value.as_deref(),
            Some("1")
        );
    }

    #[test]
    fn should_assign_ordinals_in_bibliography_order_when_entries_span_pages() {
        let index = fixture(&[
            ("References", "heading", 1),
            ("[1] Adams. First work. 2020.", "body", 1),
            ("[2] Baker. Second work. 2021.", "body", 2),
            ("[3] Chen. Third work. 2022.", "body", 3),
        ]);
        let result = extract(&index);
        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| (entry.ordinal, entry.page))
                .collect::<Vec<_>>(),
            [(0, 1), (1, 2), (2, 3)]
        );
        assert!(
            result
                .entries
                .windows(2)
                .all(|pair| pair[0].span.end <= pair[1].span.start)
        );
    }

    #[test]
    fn should_report_region_on_first_page_when_entry_wraps() {
        let index = fixture(&[
            ("References", "heading", 1),
            ("[1] Smith, A. (2020). A long", "body", 1),
            ("title. Journal of Results.", "body", 2),
        ]);
        let result = extract(&index);
        let entry = &result.entries[0];
        assert_eq!(entry.page, 1);
        let region = entry.region.unwrap();
        let first_block = &index.tokens[1];
        let second_block_y = index.tokens.last().unwrap().rects[0].y_min;
        assert!(region.y_min <= first_block.rects[0].y_min);
        assert!(region.y_max < second_block_y);
    }

    #[test]
    fn should_leave_a_marker_unresolved_when_two_entries_match() {
        let index = fixture(&[
            ("Smith (2020) and [3] are ambiguous.", "body", 1),
            ("References", "heading", 2),
            ("[3] Smith, A. (2020). First study.", "body", 2),
            ("[3] Smith, B. (2020). Different study.", "body", 2),
        ]);
        let result = extract(&index);
        assert!(result.mentions.is_empty());
        assert_eq!(result.unresolved.len(), 2);
        assert!(result.unresolved.iter().all(|item| {
            item.reason == UnresolvedReason::AmbiguousTarget && item.candidate_ordinals == [0, 1]
        }));
    }

    #[test]
    fn should_resolve_the_correct_suffixed_entry_when_smith_et_al_2020a_is_cited() {
        let index = fixture(&[
            (
                "Smith et al. 2020a explains this; Smith (2020b, 2021) agrees.",
                "body",
                1,
            ),
            ("References", "heading", 2),
            ("Smith, A. (2020a). First study.", "body", 2),
            ("Smith, A. (2020b). Second study.", "body", 2),
            ("Smith, A. (2021). Followup.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(
            slice(&index, mentions_of(&result, 0)[0].marker),
            "Smith et al. 2020a"
        );
        assert_each_entry_cited_once(&result);
    }

    #[test]
    fn should_preserve_each_occurrence_when_citations_share_a_sentence_with_astral_text() {
        let body = "😀 𝛼 evidence [1] agrees with [1].";
        let index = fixture(&[
            (body, "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. A study. 2020.", "body", 2),
        ]);
        let result = extract(&index);
        let mentions = mentions_of(&result, 0);
        assert_eq!(mentions.len(), 2);
        assert_ne!(mentions[0].marker.start, mentions[1].marker.start);
        assert_ne!(mentions[0].rects, mentions[1].rects);
        for mention in mentions {
            assert_eq!(slice(&index, mention.marker), "[1]");
            assert_eq!(mention.marker_text, "[1]");
            assert_eq!(slice(&index, mention.passage), body);
            assert_eq!(mention.passage_text, body);
            assert_eq!(mention.rects.len(), 1);
        }
    }

    #[test]
    fn should_require_raised_geometry_when_a_bare_number_is_used_as_a_citation() {
        let mut index = fixture(&[
            ("Evidence 1 and 2 are useful.", "body", 1),
            ("References", "heading", 2),
            ("1. First study.", "body", 2),
            ("2. Second study.", "body", 2),
        ]);
        let before = index.tokens[0].rects[0];
        index.tokens[1].rects[0] = rect_after(before);
        let result = extract(&index);
        assert_eq!(mentions_of(&result, 0).len(), 1);
        assert!(mentions_of(&result, 1).is_empty());
        assert_eq!(slice(&index, mentions_of(&result, 0)[0].marker), "1");
        assert_eq!(mentions_of(&result, 0)[0].style, CitationStyle::Superscript);
    }

    #[test]
    fn should_resolve_appendix_mentions_when_the_appendix_follows_the_bibliography() {
        let index = fixture(&[
            ("References", "heading", 1),
            ("[1] Smith, A. (2020). Study.", "body", 1),
            ("Appendix A", "heading", 2),
            ("We reuse [1]. Smith (2020) discusses this.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(mentions_of(&result, 0).len(), 2);
        assert!(result.mentions.iter().all(|mention| mention.page == 2));
    }

    #[test]
    fn should_withhold_links_when_no_bibliography_or_a_range_is_unsupported() {
        let none = extract(&fixture(&[
            ("See [1], (2) and Smith (2020).", "body", 1),
            ("1. A list step.", "body", 1),
        ]));
        assert!(none.entries.is_empty() && none.mentions.is_empty() && none.unresolved.is_empty());
        let result = extract(&fixture(&[
            ("See [1–1000000] and [9–1].", "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. Work.", "body", 2),
        ]));
        assert!(result.mentions.is_empty());
        assert!(
            result
                .unresolved
                .iter()
                .all(|item| item.reason == UnresolvedReason::UnsupportedRange)
        );
        assert_eq!(result.unresolved.len(), 2);
    }

    #[test]
    fn should_preserve_family_particles_when_name_parsing_agrees_on_the_printed_family() {
        let index = fixture(&[
            (
                "van der Waals (1910) agrees with Serre (1955) and de la Cruz (2020).",
                "body",
                1,
            ),
            ("References", "heading", 2),
            ("van der Waals, J. D. (1910). A study.", "body", 2),
            ("J.-P. Serre. A theorem. 1955.", "body", 2),
            ("de la Cruz, A. (2020). A study.", "body", 2),
        ]);
        let result = extract(&index);
        assert_each_entry_cited_once(&result);
        assert_eq!(
            slice(&index, mentions_of(&result, 0)[0].marker),
            "van der Waals (1910)"
        );
    }

    #[test]
    fn should_keep_sentence_context_when_an_abbreviation_crosses_segmenter_boundaries() {
        let body = "Smith et al. 2020a supports this observation.";
        let mut index = fixture(&[
            (body, "body", 1),
            ("References", "heading", 2),
            ("Smith, A. (2020a). A study.", "body", 2),
        ]);
        index.sentences = vec![
            TextSpan { start: 0, end: 12 },
            TextSpan {
                start: 13,
                end: body.len(),
            },
        ];
        let result = extract(&index);
        let mention = &result.mentions[0];
        assert_eq!(slice(&index, mention.marker), "Smith et al. 2020a");
        assert_eq!(slice(&index, mention.passage), body);
    }

    #[test]
    fn should_resolve_each_printed_marker_once_when_keys_resemble_author_year_citations() {
        let index = fixture(&[
            ("See [Smith2020] and [Smith 2020].", "body", 1),
            ("References", "heading", 2),
            ("[Smith2020] Smith, A. (2020). A study.", "body", 2),
        ]);
        let result = extract(&index);
        let mentions = mentions_of(&result, 0);
        assert_eq!(mentions.len(), 2);
        assert_eq!(slice(&index, mentions[0].marker), "[Smith2020]");
        assert_eq!(slice(&index, mentions[1].marker), "Smith 2020");
    }

    #[test]
    fn should_withhold_incompatible_number_markers_when_brackets_establish_the_citation_convention()
    {
        let mut index = fixture(&[
            (
                "Evidence [7]. See [7] and (8). See (8). Evidence [7] confirms (8). The cases are (7) first and (8) second. Equation (7).",
                "body",
                1,
            ),
            ("A footnote 7 and an index 8 are printed here.", "body", 1),
            ("References", "heading", 2),
            ("[7] Example, A. (2020). Earlier work.", "body", 2),
            ("[8] Sample, B. (2021). Next work.", "body", 2),
        ]);
        for at in 1..index.tokens.len() {
            if matches!(index.tokens[at].text.as_str(), "7" | "8") {
                let before = index.tokens[at - 1].rects[0];
                index.tokens[at].rects[0] = rect_after(before);
            }
        }
        let result = extract(&index);
        assert_eq!(mentions_of(&result, 0).len(), 3);
        assert_eq!(mentions_of(&result, 1).len(), 1);
        assert_eq!(slice(&index, mentions_of(&result, 1)[0].marker), "(8)");
        assert_eq!(result.unresolved.len(), 6);
        assert!(result.unresolved.iter().all(|item| {
            item.reason == UnresolvedReason::AmbiguousMarker && item.candidate_ordinals.len() == 1
        }));
        assert_eq!(
            result
                .unresolved
                .iter()
                .filter(|item| item.style == CitationStyle::Superscript)
                .count(),
            2
        );
        assert!(
            result
                .unresolved
                .iter()
                .all(|item| slice(&index, item.marker) == item.text)
        );
    }

    #[test]
    fn should_resolve_a_pure_superscript_citation_when_no_competing_delimited_convention_exists() {
        let mut index = fixture(&[
            ("Evidence 7 supports the result.", "body", 1),
            ("References", "heading", 2),
            ("[7] Example, A. (2020). Earlier work.", "body", 2),
        ]);
        let before = index.tokens[0].rects[0];
        index.tokens[1].rects[0] = rect_after(before);
        let result = extract(&index);
        assert_eq!(mentions_of(&result, 0).len(), 1);
        assert_eq!(slice(&index, mentions_of(&result, 0)[0].marker), "7");
        assert!(result.unresolved.is_empty());
    }

    fn numbered_references(count: usize) -> Vec<String> {
        (1..=count)
            .map(|number| format!("[{number}] Author{number}. Title {number}. 2020."))
            .collect()
    }

    #[test]
    fn should_expand_numeric_range_when_marker_is_bracketed_range() {
        let entries = numbered_references(5).join("\n");
        let index = fixture(&[
            ("Prior work [3-5] is relevant.", "body", 1),
            ("References", "heading", 2),
            (&entries, "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 5);
        assert_eq!(result.mentions.len(), 3);
        let ordinals: Vec<_> = result.mentions.iter().map(|m| m.entry_ordinal).collect();
        assert_eq!(ordinals, [2, 3, 4]);
        assert!(
            result
                .mentions
                .iter()
                .all(|m| m.marker_text == "[3-5]" && m.style == CitationStyle::Numeric)
        );
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn should_resolve_author_year_when_et_al_and_comma_year() {
        let index = fixture(&[
            ("As shown by Smith et al., 2020, this holds.", "body", 1),
            ("References", "heading", 2),
            ("Smith, A., Jones, B. (2020). A study.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(result.mentions.len(), 1);
        assert_eq!(result.mentions[0].style, CitationStyle::AuthorYear);
        assert_eq!(result.mentions[0].marker_text, "Smith et al., 2020");
    }

    #[test]
    fn should_resolve_given_family_entries_when_bibliography_prints_full_names() {
        let index = fixture(&[
            (
                "The Transformer (Vaswani et al. (2017)) is used.",
                "body",
                1,
            ),
            ("References", "heading", 2),
            (
                "Ashish Vaswani, Noam Shazeer, Niki Parmar. Attention is all you need. 2017.",
                "body",
                2,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.mentions.len(), 1, "{:?}", result.unresolved);
        assert_eq!(result.mentions[0].marker_text, "Vaswani et al. (2017)");
    }

    #[test]
    fn should_clip_passage_when_sentence_context_exceeds_limit() {
        let filler = "word ".repeat(400);
        let body = format!("{filler}evidence [1] agrees. {filler}");
        let index = fixture(&[
            (&body, "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. A study. 2020.", "body", 2),
        ]);
        let result = extract(&index);
        let mention = &result.mentions[0];
        let marker = mention.marker;
        assert!(mention.passage.start <= marker.start && marker.end <= mention.passage.end);
        assert!(marker.start - mention.passage.start <= 600);
        assert!(mention.passage.end - marker.end <= 600);
        assert!(mention.passage.start > 0 && mention.passage.end < body.len());
        let clipped = slice(&index, mention.passage);
        assert!(clipped.starts_with("word") && clipped.ends_with("word"));
        assert!(mention.passage_text.contains("evidence [1] agrees."));
    }

    #[test]
    fn should_keep_the_whole_sentence_when_context_is_within_limit() {
        let body = "Short evidence [1] stays whole.";
        let index = fixture(&[
            (body, "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. A study. 2020.", "body", 2),
        ]);
        let source = source::Source::new(&index);
        let marker = TextSpan { start: 15, end: 18 };
        let passage = TextSpan { start: 0, end: 31 };
        assert_eq!(mentions::clip_passage(&source, marker, passage), passage);
    }

    #[test]
    fn should_collapse_whitespace_when_building_passage_text() {
        let index = fixture(&[
            ("Evidence   shows\n  this\t result [1].", "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. A study. 2020.", "body", 2),
        ]);
        let result = extract(&index);
        assert_eq!(
            result.mentions[0].passage_text,
            "Evidence shows this result [1]."
        );
    }

    #[test]
    fn should_ignore_markers_inside_the_bibliography_when_entries_cite_each_other() {
        let index = fixture(&[
            ("Only [1] is cited in the body.", "body", 1),
            ("References", "heading", 2),
            ("[1] Adams. See also [2]. 2020.", "body", 2),
            (
                "[2] Baker. Builds on [1] and Adams (2020). 2021.",
                "body",
                2,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.mentions.len(), 1);
        assert_eq!(result.mentions[0].entry_ordinal, 0);
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn should_not_treat_equation_or_figure_numbers_as_citations_when_prefixed_by_eq_or_fig() {
        let index = fixture(&[
            (
                "By Eq. (1) and Equation (2), see Fig. [3] and Table (1).",
                "body",
                1,
            ),
            ("References", "heading", 2),
            (
                "[1] Adams. One. 2020.\n[2] Baker. Two. 2021.\n[3] Chen. Three. 2022.",
                "body",
                2,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 3);
        assert!(result.mentions.is_empty());
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn should_return_no_mentions_when_bibliography_is_empty() {
        let index = fixture(&[
            ("Cited [1] and Smith (2020).", "body", 1),
            ("References", "heading", 2),
        ]);
        let result = extract(&index);
        assert!(result.entries.is_empty());
        assert!(result.mentions.is_empty());
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn should_emit_one_mention_per_page_when_a_marker_crosses_a_page_break() {
        let mut index = fixture(&[
            ("Cited [1, 2] here.", "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. A study. 2020.", "body", 2),
        ]);
        index.tokens[2].page = 2;
        let result = extract(&index);
        let pages: Vec<_> = mentions_of(&result, 0).iter().map(|m| m.page).collect();
        assert_eq!(pages, [1, 2]);
        assert_eq!(result.unresolved.len(), 2);
        assert!(result.unresolved.iter().all(|item| item.key == "2"));
    }

    #[test]
    fn should_split_given_family_paragraphs_when_each_entry_ends_in_its_year() {
        let index = fixture(&[
            (
                "Ash et al. (2019) and Bahdanau et al. (2018) agree.",
                "body",
                1,
            ),
            ("References", "heading", 2),
            (
                "Jordan Ash, Chicheng Zhang, and Alekh Agarwal. Deep batch active learning. arXiv preprint, 2019.",
                "body",
                2,
            ),
            (
                "Dzmitry Bahdanau, Felix Hill, and Jan Leike. Learning to understand",
                "body",
                2,
            ),
            (
                "goal specifications. arXiv preprint arXiv:1806.01946,",
                "body",
                3,
            ),
            ("2018.", "list", 3),
            (
                "Paul Christiano, Jan Leike, and Dario Amodei. Deep reinforcement learning. 2017.",
                "body",
                3,
            ),
        ]);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries[1].fields.year.value.as_deref(), Some("2018"));
        assert_eq!(result.mentions.len(), 2);
        assert!(result.unresolved.is_empty());
    }
}
