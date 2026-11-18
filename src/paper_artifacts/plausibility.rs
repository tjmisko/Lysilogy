//! Deterministic sanity checks on a merged record.
//!
//! A run can "succeed" with a degenerate answer: a 16-page paper mapped as a
//! single "Front matter" section, a 48-page paper with no references. These
//! checks compare the record with what the page text itself shows and return
//! plain-language warnings. They never change the record. The outline,
//! section and reference checks also mark an answer worth asking for once
//! more, because those failures are usually a stochastic bad answer rather
//! than a property of the paper. The grouping checks look at how logical
//! units use the outline and are reported only.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{grouping, outline, title::looks_like_identifier};
use crate::domain::{DocumentLayout, ExtractedPage, QualityWarning, QualityWarningKind};

/// An empty outline is suspect when the text shows this many numbered
/// heading lines.
const MIN_NUMBERED_HEADINGS: usize = 3;

/// An outline is sparse when it has under half of the numbered heading lines
/// and the text shows at least this many.
const MIN_NUMBERED_FOR_SPARSE: usize = 6;

/// A grouping of at least this many units, each one heading, is a table of
/// contents rather than a reading map.
const MIN_UNITS_FOR_MIRROR: usize = 4;

/// A section map covering less than this share of the main text is suspect.
const MIN_SECTION_COVERAGE: f64 = 0.6;

/// Papers shorter than this are not held to a unit count.
const MIN_PAGES_FOR_UNIT_COUNT: usize = 6;

/// Headings longer than this are prose, not headings.
const MAX_HEADING_CHARS: usize = 40;

/// The abstract heading is looked for on this many leading pages.
const ABSTRACT_PAGES: usize = 3;

const REFERENCE_HEADINGS: &[&str] = &[
    "references",
    "reference",
    "bibliography",
    "literature cited",
    "works cited",
    "references cited",
    "cited literature",
    "references and notes",
    "literature",
];

/// Every warning for the fields in `fields`, on the final record.
#[must_use]
pub fn check(
    artifacts: &Value,
    fields: &BTreeSet<String>,
    pages: &[ExtractedPage],
) -> Vec<QualityWarning> {
    let mut warnings = Vec::new();
    if fields.contains("identity") {
        warnings.extend(identity_warnings(artifacts));
    }
    if fields.contains("abstract") {
        warnings.extend(abstract_warnings(artifacts, pages));
    }
    if fields.contains("outline") && artifacts.get("outline").is_none() {
        warnings.push(warning(
            QualityWarningKind::OutlineMissing,
            "outline",
            "The plan asked for the outline, but the record has none, so the reader falls back to page-span units.".to_owned(),
        ));
    }
    if fields.contains(super::context::FIELD)
        && !artifacts
            .get(super::context::FIELD)
            .is_some_and(Value::is_object)
    {
        warnings.push(warning(
            QualityWarningKind::ContextMissing,
            super::context::FIELD,
            "The plan asked for the context, but the record has none, so the first view shows no account of why the paper matters.".to_owned(),
        ));
    }
    warnings.extend(degenerate(artifacts, fields, pages));
    if fields.contains("sections") {
        warnings.extend(grouping_warnings(artifacts, pages));
    }
    warnings
}

/// The warnings that suggest a degenerate model answer worth one retry:
/// the section map and the reference list, for the fields a call carries.
#[must_use]
pub fn degenerate(
    artifacts: &Value,
    fields: &BTreeSet<String>,
    pages: &[ExtractedPage],
) -> Vec<QualityWarning> {
    let mut warnings = Vec::new();
    if fields.contains("outline") {
        warnings.extend(outline_warnings(artifacts, pages));
    }
    if fields.contains("sections") {
        warnings.extend(section_warnings(artifacts, pages));
    }
    if fields.contains("references") {
        warnings.extend(reference_warnings(artifacts, pages));
    }
    warnings
}

fn warning(kind: QualityWarningKind, field: &str, message: String) -> QualityWarning {
    QualityWarning {
        kind,
        field: field.to_owned(),
        message,
    }
}

fn identity_warnings(artifacts: &Value) -> Vec<QualityWarning> {
    let mut warnings = Vec::new();
    let Some(identity) = artifacts.get("identity").filter(|value| value.is_object()) else {
        return warnings;
    };
    let title = identity
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if title.chars().count() < 3 {
        warnings.push(warning(
            QualityWarningKind::TitleImplausible,
            "identity",
            format!("The title \"{title}\" is under three characters."),
        ));
    } else if looks_like_identifier(title) {
        warnings.push(warning(
            QualityWarningKind::TitleImplausible,
            "identity",
            format!("The title \"{title}\" looks like an identifier or a margin stamp."),
        ));
    }
    let authors = identity
        .get("authors")
        .and_then(Value::as_array)
        .map_or(0, |authors| {
            authors
                .iter()
                .filter(|author| {
                    let name = author
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| author.as_str())
                        .unwrap_or_default();
                    !name.trim().is_empty()
                })
                .count()
        });
    if authors == 0 {
        warnings.push(warning(
            QualityWarningKind::AuthorsMissing,
            "identity",
            "No authors were returned.".to_owned(),
        ));
    }
    warnings
}

fn abstract_warnings(artifacts: &Value, pages: &[ExtractedPage]) -> Vec<QualityWarning> {
    let text = artifacts
        .pointer("/abstract/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !text.trim().is_empty() {
        return Vec::new();
    }
    abstract_heading_page(pages)
        .map(|page| {
            warning(
                QualityWarningKind::AbstractMissing,
                "abstract",
                format!("No abstract was returned, but page {page} has an \"Abstract\" heading."),
            )
        })
        .into_iter()
        .collect()
}

fn section_warnings(artifacts: &Value, pages: &[ExtractedPage]) -> Vec<QualityWarning> {
    let units = artifacts
        .get("sections")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let page_count = pages.len();
    if units.is_empty() {
        return vec![warning(
            QualityWarningKind::SectionsMissing,
            "sections",
            format!("The section map is empty for a {page_count}-page paper."),
        )];
    }
    // The main text ends where the references begin, when they can be found.
    let body = references_heading_page(pages)
        .map_or(page_count, |page| {
            usize::try_from(page).unwrap_or(page_count)
        })
        .clamp(1, page_count.max(1));
    let mut warnings = Vec::new();
    let substantive = units
        .iter()
        .filter(|unit| {
            !matches!(
                unit.get("kind").and_then(Value::as_str),
                Some("references" | "appendix")
            )
        })
        .count();
    let expected = body.div_ceil(4);
    let floor = expected.div_ceil(2).max(2);
    if body >= MIN_PAGES_FOR_UNIT_COUNT && substantive < floor {
        warnings.push(warning(
            QualityWarningKind::SectionsSparse,
            "sections",
            format!(
                "{substantive} reading unit(s) for {body} pages of main text; about {expected} or more were expected."
            ),
        ));
    }
    let mut covered = BTreeSet::new();
    for unit in units {
        let start = unit.pointer("/pages/start").and_then(Value::as_u64);
        let end = unit.pointer("/pages/end").and_then(Value::as_u64);
        if let (Some(start), Some(end)) = (start, end) {
            let body = body as u64;
            for page in start.max(1)..=end.min(body) {
                covered.insert(page);
            }
        }
    }
    #[allow(clippy::cast_precision_loss)] // Page counts are small.
    let coverage = covered.len() as f64 / body as f64;
    if coverage < MIN_SECTION_COVERAGE {
        warnings.push(warning(
            QualityWarningKind::SectionsLowCoverage,
            "sections",
            format!(
                "The section map covers {} of {body} pages of main text ({:.0}%).",
                covered.len(),
                coverage * 100.0
            ),
        ));
    }
    warnings
}

/// An outline that misses the numbered headings the text plainly shows.
fn outline_warnings(artifacts: &Value, pages: &[ExtractedPage]) -> Vec<QualityWarning> {
    let Some(entries) = artifacts.get("outline").and_then(Value::as_array) else {
        return Vec::new();
    };
    let numbered = outline::candidates(pages, &DocumentLayout::default())
        .into_iter()
        .filter(|candidate| candidate.number.is_some())
        .collect::<Vec<_>>();
    let examples = numbered
        .iter()
        .take(3)
        .map(|candidate| format!("\"{}\" on page {}", candidate.text, candidate.page))
        .collect::<Vec<_>>()
        .join(", ");
    if entries.is_empty() && numbered.len() >= MIN_NUMBERED_HEADINGS {
        return vec![warning(
            QualityWarningKind::OutlineMissing,
            "outline",
            format!(
                "The outline is empty, but the text shows {} numbered heading lines, such as {examples}.",
                numbered.len()
            ),
        )];
    }
    if numbered.len() >= MIN_NUMBERED_FOR_SPARSE && entries.len() * 2 < numbered.len() {
        return vec![warning(
            QualityWarningKind::OutlineSparse,
            "outline",
            format!(
                "The outline has {} entries, but the text shows {} numbered heading lines, such as {examples}.",
                entries.len(),
                numbered.len()
            ),
        )];
    }
    Vec::new()
}

/// How grouped logical units use the outline: every main-text heading in a
/// unit, and not merely one unit per heading.
fn grouping_warnings(artifacts: &Value, pages: &[ExtractedPage]) -> Vec<QualityWarning> {
    let units = artifacts
        .get("sections")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let Some(outline_value) = artifacts.get("outline") else {
        return Vec::new();
    };
    if !grouping::is_grouped(units) {
        return Vec::new();
    }
    let entries = outline::entries(outline_value, pages);
    let ranges = units
        .iter()
        .filter_map(|unit| {
            let first = unit.get("outline_first")?.as_u64()?;
            let last = unit.get("outline_last")?.as_u64()?;
            Some((usize::try_from(first).ok()?, usize::try_from(last).ok()?))
        })
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    let uncovered = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            entry.is_main()
                && !ranges
                    .iter()
                    .any(|(first, last)| (first..=last).contains(&index))
        })
        .map(|(_, entry)| format!("\"{}\"", entry.anchor))
        .collect::<Vec<_>>();
    if !uncovered.is_empty() {
        warnings.push(warning(
            QualityWarningKind::SectionsGroupingInvalid,
            "sections",
            format!(
                "{} main-text heading(s) belong to no reading unit: {}.",
                uncovered.len(),
                uncovered.join(", ")
            ),
        ));
    }
    let main_units = ranges
        .iter()
        .filter(|(first, _)| entries.get(*first).is_some_and(outline::Entry::is_main))
        .collect::<Vec<_>>();
    let lower = outline::body_pages(&entries, pages.len()).div_ceil(4);
    if main_units.len() >= MIN_UNITS_FOR_MIRROR
        && main_units.len() > lower
        && main_units.iter().all(|(first, last)| first == last)
    {
        warnings.push(warning(
            QualityWarningKind::SectionsMirrorOutline,
            "sections",
            format!(
                "Each of the {} reading units is a single printed heading, so the map repeats the outline instead of grouping it.",
                main_units.len()
            ),
        ));
    }
    warnings
}

fn reference_warnings(artifacts: &Value, pages: &[ExtractedPage]) -> Vec<QualityWarning> {
    let count = artifacts
        .get("references")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if count > 0 {
        return Vec::new();
    }
    references_heading_page(pages)
        .map(|page| {
            warning(
                QualityWarningKind::ReferencesMissing,
                "references",
                format!("No references were returned, but page {page} has a references heading."),
            )
        })
        .into_iter()
        .collect()
}

/// The first page after the first one with a references or bibliography
/// heading line. A table of contents on page 1 does not count.
fn references_heading_page(pages: &[ExtractedPage]) -> Option<u32> {
    pages.iter().skip(1).find_map(|page| {
        page.text
            .lines()
            .filter_map(heading_text)
            .any(|heading| REFERENCE_HEADINGS.contains(&heading.as_str()))
            .then_some(page.number)
    })
}

/// The first leading page with a line that opens with the word "Abstract".
fn abstract_heading_page(pages: &[ExtractedPage]) -> Option<u32> {
    pages.iter().take(ABSTRACT_PAGES).find_map(|page| {
        page.text
            .lines()
            .any(|line| {
                let line = despaced(line.trim()).to_lowercase();
                line.strip_prefix("abstract")
                    .is_some_and(|rest| !rest.starts_with(char::is_alphabetic))
            })
            .then_some(page.number)
    })
}

/// A short line reduced to its heading words: lowercased, without a leading
/// section number (`7`, `7.`, `VII.`, `A.`) or trailing punctuation. `None`
/// for a line too long to be a heading.
fn heading_text(line: &str) -> Option<String> {
    let line = despaced(line.trim());
    if line.is_empty() || line.chars().count() > MAX_HEADING_CHARS {
        return None;
    }
    let mut words = line.split_whitespace().peekable();
    if let Some(first) = words.peek()
        && is_section_number(first)
    {
        words.next();
    }
    let text = words
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([':', '.'])
        .to_lowercase();
    (!text.is_empty()).then_some(text)
}

/// `7`, `7.`, `7.1`, `VII.`, `A.`, but not a word.
fn is_section_number(word: &str) -> bool {
    let word = word.trim_end_matches('.');
    if word.is_empty() {
        return false;
    }
    let numeric = word.chars().all(|c| c.is_ascii_digit() || c == '.');
    let roman = word.chars().all(|c| "IVXLC".contains(c));
    let letter = word.chars().count() == 1 && word.chars().all(|c| c.is_ascii_uppercase());
    numeric || roman || letter
}

/// `R E F E R E N C E S` as `REFERENCES`; other lines unchanged.
fn despaced(line: &str) -> String {
    let words = line.split_whitespace().collect::<Vec<_>>();
    if words.len() >= 4 && words.iter().all(|word| word.chars().count() == 1) {
        words.concat()
    } else {
        line.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pages(texts: &[&str]) -> Vec<ExtractedPage> {
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| ExtractedPage {
                number: u32::try_from(index + 1).expect("page"),
                text: (*text).to_owned(),
            })
            .collect()
    }

    fn sixteen_pages_with_references() -> Vec<ExtractedPage> {
        let mut texts = vec!["Body text."; 16];
        texts[0] = "A Title\nAbstract\nWe study things.";
        texts[14] = "6 Conclusion\nWe are done.\nReferences\n[1] A. Author. A work.";
        pages(&texts)
    }

    fn all_fields() -> BTreeSet<String> {
        ["identity", "abstract", "sections", "references"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn kinds(warnings: &[QualityWarning]) -> Vec<QualityWarningKind> {
        warnings.iter().map(|warning| warning.kind).collect()
    }

    fn healthy() -> Value {
        json!({
            "identity": {"title": "Sparse Models Under Noise", "authors": [{"name": "A. Author", "affiliation": null}]},
            "abstract": {"text": "We study things.", "page": 1},
            "sections": [
                {"title": "Setup", "kind": "background", "pages": {"start": 1, "end": 4}},
                {"title": "Method", "kind": "methods", "pages": {"start": 5, "end": 8}},
                {"title": "Results", "kind": "results", "pages": {"start": 9, "end": 12}},
                {"title": "Discussion", "kind": "discussion", "pages": {"start": 13, "end": 15}}
            ],
            "references": [{"key": "1"}]
        })
    }

    #[test]
    fn should_pass_a_healthy_record() {
        let warnings = check(&healthy(), &all_fields(), &sixteen_pages_with_references());
        assert!(warnings.is_empty(), "{warnings:#?}");
    }

    #[test]
    fn should_flag_a_single_front_matter_section_for_a_sixteen_page_paper() {
        let mut record = healthy();
        record["sections"] = json!([
            {"title": "Front matter", "kind": "other", "pages": {"start": 1, "end": 1}}
        ]);
        let warnings = degenerate(&record, &all_fields(), &sixteen_pages_with_references());
        assert_eq!(
            kinds(&warnings),
            [
                QualityWarningKind::SectionsSparse,
                QualityWarningKind::SectionsLowCoverage
            ]
        );
        assert!(
            warnings[1].message.contains("1 of 15 pages"),
            "{}",
            warnings[1].message
        );
    }

    #[test]
    fn should_flag_an_empty_section_map() {
        let mut record = healthy();
        record["sections"] = json!([]);
        let warnings = degenerate(&record, &all_fields(), &sixteen_pages_with_references());
        assert_eq!(kinds(&warnings), [QualityWarningKind::SectionsMissing]);
    }

    #[test]
    fn should_not_hold_a_short_paper_to_a_unit_count() {
        let record = json!({"sections": [
            {"title": "Go To", "kind": "other", "pages": {"start": 1, "end": 4}}
        ]});
        let fields = BTreeSet::from(["sections".to_owned()]);
        assert!(degenerate(&record, &fields, &pages(&["a", "b", "c", "d"])).is_empty());
    }

    #[test]
    fn should_flag_missing_references_only_when_the_text_has_a_references_heading() {
        let mut record = healthy();
        record["references"] = json!([]);
        let warnings = degenerate(&record, &all_fields(), &sixteen_pages_with_references());
        assert_eq!(kinds(&warnings), [QualityWarningKind::ReferencesMissing]);
        assert!(warnings[0].message.contains("page 15"));

        let no_heading = pages(&["A Title\nAbstract\nText.", "More text.", "End."]);
        let fields = BTreeSet::from(["references".to_owned()]);
        assert!(degenerate(&record, &fields, &no_heading).is_empty());
    }

    #[test]
    fn should_recognize_numbered_and_letter_spaced_reference_headings() {
        for heading in [
            "VII. REFERENCES",
            "7 References",
            "R E F E R E N C E S",
            "Bibliography:",
        ] {
            let found =
                references_heading_page(&pages(&["Title", &format!("Text\n{heading}\n[1] X")]));
            assert_eq!(found, Some(2), "{heading}");
        }
        let prose =
            "Our references to earlier work are in the bibliography at the end of this paper.";
        assert_eq!(references_heading_page(&pages(&["Title", prose])), None);
    }

    #[test]
    fn should_flag_a_missing_abstract_when_the_text_has_an_abstract_heading() {
        let mut record = healthy();
        record["abstract"] = json!({"text": null, "page": null});
        let fields = BTreeSet::from(["abstract".to_owned()]);
        let warnings = check(&record, &fields, &sixteen_pages_with_references());
        assert_eq!(kinds(&warnings), [QualityWarningKind::AbstractMissing]);
        for opening in ["ABSTRACT. We show", "Abstract—We show", "a b s t r a c t"] {
            assert_eq!(
                abstract_heading_page(&pages(&[opening])),
                Some(1),
                "{opening}"
            );
        }
        assert_eq!(
            abstract_heading_page(&pages(&["Abstraction is useful."])),
            None
        );
        let essay = pages(&["Go To Statement Considered Harmful\nFor a number of years"]);
        assert!(check(&record, &fields, &essay).is_empty());
    }

    #[test]
    fn should_flag_a_stamp_or_tiny_title_and_missing_authors() {
        let fields = BTreeSet::from(["identity".to_owned()]);
        for title in ["T", "arXiv:1706.03762v5 [cs.CL] 6 Dec 2017"] {
            let record = json!({"identity": {"title": title, "authors": []}});
            let warnings = check(&record, &fields, &[]);
            assert_eq!(
                kinds(&warnings),
                [
                    QualityWarningKind::TitleImplausible,
                    QualityWarningKind::AuthorsMissing
                ],
                "{title}"
            );
        }
    }

    #[test]
    fn should_check_only_the_fields_that_were_asked_for() {
        let record = json!({"supercut": []});
        let fields = BTreeSet::from(["supercut".to_owned()]);
        assert!(check(&record, &fields, &sixteen_pages_with_references()).is_empty());
    }

    #[test]
    fn should_flag_a_planned_outline_or_context_that_the_record_lacks() {
        let fields = BTreeSet::from(["outline".to_owned(), "context".to_owned()]);
        let warnings = check(&json!({}), &fields, &numbered_pages());
        assert_eq!(
            kinds(&warnings),
            [
                QualityWarningKind::OutlineMissing,
                QualityWarningKind::ContextMissing
            ]
        );
        let present = json!({"outline": numbered_outline(), "context": {"lead": []}});
        assert!(check(&present, &fields, &numbered_pages()).is_empty());
    }

    fn numbered_pages() -> Vec<ExtractedPage> {
        pages(&[
            "A Title\n1 Introduction\nWe study things in this paper.",
            "2 Model\nA model of things.\n2.1 Assumptions\nThey are standard.",
            "3 Data\nForty years of data.\n4 Results\nLarge effects.",
            "5 Robustness\nStill large.\n6 Conclusion\nWe are done.",
            "References\n[1] A. Author. A work.",
        ])
    }

    fn numbered_outline() -> Value {
        json!([
            {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main"},
            {"number": "2", "title": "Model", "level": 1, "page": 2, "role": "main"},
            {"number": "2.1", "title": "Assumptions", "level": 2, "page": 2, "role": "main"},
            {"number": "3", "title": "Data", "level": 1, "page": 3, "role": "main"},
            {"number": "4", "title": "Results", "level": 1, "page": 3, "role": "main"},
            {"number": "5", "title": "Robustness", "level": 1, "page": 4, "role": "main"},
            {"number": "6", "title": "Conclusion", "level": 1, "page": 4, "role": "main"},
            {"number": null, "title": "References", "level": 1, "page": 5, "role": "references"}
        ])
    }

    #[test]
    fn should_flag_an_empty_or_sparse_outline_when_the_text_shows_numbered_headings() {
        let pages = numbered_pages();
        let fields = BTreeSet::from(["outline".to_owned()]);
        let empty = degenerate(&json!({"outline": []}), &fields, &pages);
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].kind, QualityWarningKind::OutlineMissing);
        assert!(
            empty[0].message.contains("\"1 Introduction\" on page 1"),
            "{}",
            empty[0].message
        );
        let sparse = degenerate(
            &json!({"outline": [numbered_outline()[0].clone(), numbered_outline()[1].clone()]}),
            &fields,
            &pages,
        );
        assert_eq!(sparse[0].kind, QualityWarningKind::OutlineSparse);
        assert!(degenerate(&json!({"outline": numbered_outline()}), &fields, &pages).is_empty());
        // A letter without headings may have an empty outline.
        let letter = self::pages(&["Dear editor,\nWe write to comment on a paper."]);
        assert!(degenerate(&json!({"outline": []}), &fields, &letter).is_empty());
    }

    #[test]
    fn should_flag_uncovered_headings_and_a_grouping_that_repeats_the_outline() {
        let pages = numbered_pages();
        let fields = BTreeSet::from(["sections".to_owned()]);
        let unit = |first: u64, last: u64, start: u64, end: u64| {
            json!({"title": "T", "kind": "other", "outline_first": first, "outline_last": last,
                   "pages": {"start": start, "end": end}})
        };
        let gap = json!({"outline": numbered_outline(), "sections": [unit(0, 2, 1, 2), unit(5, 6, 4, 4)]});
        let warnings = grouping_warnings(&gap, &pages);
        assert_eq!(warnings.len(), 1, "{warnings:#?}");
        assert_eq!(
            warnings[0].kind,
            QualityWarningKind::SectionsGroupingInvalid
        );
        assert!(warnings[0].message.contains("\"3 Data\", \"4 Results\""));
        let mirror = json!({"outline": numbered_outline(), "sections": [
            unit(0, 0, 1, 1), unit(1, 1, 2, 2), unit(2, 2, 2, 2), unit(3, 3, 3, 3),
            unit(4, 4, 3, 3), unit(5, 5, 4, 4), unit(6, 6, 4, 4)
        ]});
        let warnings = check(&mirror, &fields, &pages);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.kind == QualityWarningKind::SectionsMirrorOutline),
            "{warnings:#?}"
        );
        let good = json!({"outline": numbered_outline(), "sections": [unit(0, 2, 1, 2), unit(3, 6, 3, 4)]});
        assert!(grouping_warnings(&good, &pages).is_empty());
    }
}
