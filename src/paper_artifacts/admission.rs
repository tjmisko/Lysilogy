//! The application admits what the model proposes only when it is verbatim.
//!
//! Every quoted string in a paper-artifacts value is aligned against the page
//! text at the word level, after the same canonicalization the reader uses
//! (lowercase alphanumerics, ligatures expanded, line-break hyphens joined).
//! Distance zero is verbatim. A close match is repaired locally by replacing
//! the quote with the matched source span, which is verbatim by construction.
//! Anything else is a failure for the bounded model repair, and then a drop.

use serde::Serialize;
use serde_json::Value;

use crate::{domain::ExtractedPage, layout::canonical_chars};

/// The least word-level similarity a local repair accepts.
pub const LOCAL_REPAIR_SIMILARITY: f64 = 0.85;

/// Shorter quotes are checked by canonical containment and never repaired.
const ALIGNMENT_MINIMUM_WORDS: usize = 3;

#[derive(Clone, Debug)]
struct Word {
    canonical: String,
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct Page<'a> {
    number: u32,
    text: &'a str,
    words: Vec<Word>,
    canonical: String,
}

/// Page text prepared for alignment.
#[derive(Debug)]
pub struct SourceText<'a> {
    pages: Vec<Page<'a>>,
}

impl<'a> SourceText<'a> {
    #[must_use]
    pub fn new(pages: &'a [ExtractedPage]) -> Self {
        Self {
            pages: pages
                .iter()
                .map(|page| {
                    let words = words(&page.text);
                    let canonical = words.iter().map(|word| word.canonical.as_str()).collect();
                    Page {
                        number: page.number,
                        text: &page.text,
                        words,
                        canonical,
                    }
                })
                .collect(),
        }
    }

    /// How `quote` stands against the source, preferring `page` on ties.
    #[must_use]
    pub fn verdict(&self, quote: &str, page: Option<u32>) -> Verdict {
        let needle = words(quote);
        if needle.is_empty() {
            return Verdict::Missing;
        }
        if needle.len() < ALIGNMENT_MINIMUM_WORDS {
            let canonical: String = needle.iter().map(|word| word.canonical.as_str()).collect();
            return self
                .ordered(page)
                .find(|candidate| candidate.canonical.contains(&canonical))
                .map_or(Verdict::Missing, |candidate| Verdict::Verbatim {
                    page: candidate.number,
                });
        }
        let mut best: Option<Match> = None;
        for candidate in self.ordered(page) {
            let Some(found) = align(&needle, candidate) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|best| found.distance < best.distance)
            {
                let exact = found.distance == 0;
                best = Some(found);
                if exact {
                    break;
                }
            }
        }
        let Some(best) = best else {
            return Verdict::Missing;
        };
        if best.distance == 0 {
            return Verdict::Verbatim { page: best.page };
        }
        Verdict::Near(best)
    }

    /// The preferred page first, then the rest in order.
    fn ordered(&self, page: Option<u32>) -> impl Iterator<Item = &Page<'a>> {
        let preferred = self
            .pages
            .iter()
            .filter(move |candidate| Some(candidate.number) == page);
        let rest = self
            .pages
            .iter()
            .filter(move |candidate| Some(candidate.number) != page);
        preferred.chain(rest)
    }
}

/// The best approximate occurrence of a quote on one page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Match {
    pub page: u32,
    /// The matched source span, whitespace compacted and line-break hyphens joined.
    pub text: String,
    pub distance: usize,
    pub needle_words: usize,
}

impl Match {
    #[must_use]
    pub fn similarity(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)] // Word counts are far below 2^52.
        let ratio = self.distance as f64 / self.needle_words.max(1) as f64;
        (1.0 - ratio).max(0.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    Verbatim { page: u32 },
    Near(Match),
    Missing,
}

/// Word-level semi-global alignment: the needle must be consumed whole, the
/// page span may start and end anywhere.
fn align(needle: &[Word], page: &Page<'_>) -> Option<Match> {
    let hay = &page.words;
    if hay.is_empty() {
        return None;
    }
    let width = hay.len() + 1;
    let mut previous: Vec<usize> = vec![0; width];
    let mut previous_start: Vec<usize> = (0..width).collect();
    let mut current = vec![0; width];
    let mut current_start = vec![0; width];
    for (row, word) in needle.iter().enumerate() {
        current[0] = row + 1;
        current_start[0] = 0;
        for column in 1..width {
            let substitution = usize::from(word.canonical != hay[column - 1].canonical);
            let candidates = [
                (
                    previous[column - 1] + substitution,
                    previous_start[column - 1],
                ),
                (previous[column] + 1, previous_start[column]),
                (current[column - 1] + 1, current_start[column - 1]),
            ];
            // The first minimum wins, so a substitution is preferred on ties.
            let (cost, start) = candidates
                .into_iter()
                .reduce(|best, next| if next.0 < best.0 { next } else { best })
                .unwrap_or(candidates[0]);
            current[column] = cost;
            current_start[column] = start;
        }
        std::mem::swap(&mut previous, &mut current);
        std::mem::swap(&mut previous_start, &mut current_start);
    }
    let (end, distance) = previous
        .iter()
        .enumerate()
        .skip(1)
        .min_by_key(|(_, distance)| **distance)
        .map(|(end, distance)| (end, *distance))?;
    let start = previous_start[end];
    if start >= end {
        return None;
    }
    Some(Match {
        page: page.number,
        text: verbatim_span(&page.text[hay[start].start..hay[end - 1].end]),
        distance,
        needle_words: needle.len(),
    })
}

/// Words with byte offsets; a word broken by a hyphen at a line end is one word.
fn words(text: &str) -> Vec<Word> {
    let mut runs = Vec::<(usize, usize)>::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        match (character.is_whitespace(), start) {
            (true, Some(begin)) => {
                runs.push((begin, index));
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(begin) = start {
        runs.push((begin, text.len()));
    }
    let mut merged = Vec::<(usize, usize)>::new();
    let mut index = 0;
    while index < runs.len() {
        let (begin, mut end) = runs[index];
        while index + 1 < runs.len()
            && text[begin..end].ends_with('-')
            && text[end..runs[index + 1].0].contains('\n')
        {
            index += 1;
            end = runs[index].1;
        }
        merged.push((begin, end));
        index += 1;
    }
    merged
        .into_iter()
        .filter_map(|(start, end)| {
            let canonical: String = canonical_chars(&text[start..end]).into_iter().collect();
            (!canonical.is_empty()).then_some(Word {
                canonical,
                start,
                end,
            })
        })
        .collect()
}

fn verbatim_span(text: &str) -> String {
    let mut joined = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if !joined.is_empty() && !joined.ends_with('-') {
            joined.push(' ');
        }
        if joined.ends_with('-') {
            joined.pop();
        }
        joined.push_str(line);
    }
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a quote is for, for reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    Evidence,
    Supercut,
    Citation,
    Relation,
    Abstract,
    Caption,
}

/// How a failing quote is removed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Removal {
    /// Remove the array element at this pointer.
    Element(String),
    /// Set this nullable field to null.
    Null(String),
}

/// One quoted string in an artifacts value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuoteSite {
    pub kind: SiteKind,
    pub quote_pointer: String,
    pub page_pointer: Option<String>,
    pub removal: Removal,
}

impl QuoteSite {
    #[must_use]
    pub fn quote<'v>(&self, artifacts: &'v Value) -> Option<&'v str> {
        artifacts
            .pointer(&self.quote_pointer)
            .and_then(Value::as_str)
    }

    #[must_use]
    pub fn page(&self, artifacts: &Value) -> Option<u32> {
        let pointer = self.page_pointer.as_deref()?;
        artifacts
            .pointer(pointer)
            .and_then(Value::as_u64)
            .and_then(|page| u32::try_from(page).ok())
    }
}

/// Every quoted string the application must admit, in document order.
#[must_use]
pub fn quote_sites(artifacts: &Value) -> Vec<QuoteSite> {
    let mut sites = Vec::new();
    if artifacts
        .pointer("/abstract/text")
        .is_some_and(Value::is_string)
    {
        sites.push(QuoteSite {
            kind: SiteKind::Abstract,
            quote_pointer: "/abstract/text".to_owned(),
            page_pointer: Some("/abstract/page".to_owned()),
            removal: Removal::Null("/abstract/text".to_owned()),
        });
    }
    evidence_sites(artifacts, "", &mut sites);
    for (index, segment) in elements(artifacts, "/supercut") {
        if segment.get("kind").and_then(Value::as_str) == Some("source") {
            sites.push(element_site(SiteKind::Supercut, "/supercut", index, "text"));
        }
    }
    for (index, _) in elements(artifacts, "/citations") {
        sites.push(element_site(
            SiteKind::Citation,
            "/citations",
            index,
            "context",
        ));
    }
    for (index, _) in elements(artifacts, "/relations") {
        sites.push(element_site(
            SiteKind::Relation,
            "/relations",
            index,
            "quote",
        ));
    }
    for (index, object) in elements(artifacts, "/objects") {
        if object.get("caption").is_some_and(Value::is_string) {
            sites.push(QuoteSite {
                kind: SiteKind::Caption,
                quote_pointer: format!("/objects/{index}/caption"),
                page_pointer: Some(format!("/objects/{index}/page")),
                removal: Removal::Null(format!("/objects/{index}/caption")),
            });
        }
    }
    sites
}

fn elements<'v>(artifacts: &'v Value, pointer: &str) -> impl Iterator<Item = (usize, &'v Value)> {
    artifacts
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
}

fn element_site(kind: SiteKind, array: &str, index: usize, field: &str) -> QuoteSite {
    QuoteSite {
        kind,
        quote_pointer: format!("{array}/{index}/{field}"),
        page_pointer: Some(format!("{array}/{index}/page")),
        removal: Removal::Element(format!("{array}/{index}")),
    }
}

fn evidence_sites(value: &Value, pointer: &str, sites: &mut Vec<QuoteSite>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_pointer = format!("{pointer}/{}", escape(key));
                if key == "evidence"
                    && let Value::Array(items) = child
                {
                    for index in 0..items.len() {
                        sites.push(element_site(
                            SiteKind::Evidence,
                            &child_pointer,
                            index,
                            "quote",
                        ));
                    }
                    continue;
                }
                evidence_sites(child, &child_pointer, sites);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                evidence_sites(child, &format!("{pointer}/{index}"), sites);
            }
        }
        _ => {}
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Apply removals so that no removal shifts the index of a later one:
/// deeper pointers first, then higher indices first.
pub fn apply_removals(artifacts: &mut Value, removals: &[Removal]) {
    let mut elements = Vec::<(String, usize)>::new();
    for removal in removals {
        match removal {
            Removal::Null(pointer) => {
                if let Some(slot) = artifacts.pointer_mut(pointer) {
                    *slot = Value::Null;
                }
            }
            Removal::Element(pointer) => {
                if let Some((parent, index)) = pointer.rsplit_once('/')
                    && let Ok(index) = index.parse::<usize>()
                {
                    elements.push((parent.to_owned(), index));
                }
            }
        }
    }
    elements.sort_by(|left, right| {
        let depth = |pointer: &str| pointer.matches('/').count();
        depth(&right.0)
            .cmp(&depth(&left.0))
            .then_with(|| right.0.cmp(&left.0))
            .then_with(|| right.1.cmp(&left.1))
    });
    elements.dedup();
    for (parent, index) in elements {
        if let Some(Value::Array(items)) = artifacts.pointer_mut(&parent)
            && index < items.len()
        {
            items.remove(index);
        }
    }
}

/// Counts of verdicts over every site, without changing anything.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CheckCounts {
    pub sites: usize,
    pub verbatim: usize,
    pub near: usize,
    pub missing: usize,
    /// Verbatim, but on a different page than the one cited.
    pub wrong_page: usize,
}

#[must_use]
pub fn check(source: &SourceText<'_>, artifacts: &Value) -> CheckCounts {
    let mut counts = CheckCounts::default();
    for site in quote_sites(artifacts) {
        counts.sites += 1;
        let cited = site.page(artifacts);
        match site
            .quote(artifacts)
            .map_or(Verdict::Missing, |quote| source.verdict(quote, cited))
        {
            Verdict::Verbatim { page } => {
                counts.verbatim += 1;
                counts.wrong_page += usize::from(cited.is_some_and(|cited| cited != page));
            }
            Verdict::Near(found) if found.similarity() >= LOCAL_REPAIR_SIMILARITY => {
                counts.near += 1;
            }
            Verdict::Near(_) | Verdict::Missing => counts.missing += 1,
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pages() -> Vec<ExtractedPage> {
        vec![
            ExtractedPage {
                number: 1,
                text: "We study the opti-\nmization of sparse\nmodels under noise.\nThe ﬁrst result is a bound.".to_owned(),
            },
            ExtractedPage {
                number: 2,
                text: "Figure 2: Accuracy versus model size for all methods.\nWe conclude that scaling helps.".to_owned(),
            },
        ]
    }

    #[test]
    fn should_accept_a_quote_across_a_line_break_hyphen_and_ligature() {
        let pages = pages();
        let source = SourceText::new(&pages);
        assert_eq!(
            source.verdict("the optimization of sparse models", Some(1)),
            Verdict::Verbatim { page: 1 }
        );
        assert_eq!(
            source.verdict("The first result is a bound.", Some(1)),
            Verdict::Verbatim { page: 1 }
        );
    }

    #[test]
    fn should_ignore_case_punctuation_and_smart_quotes() {
        let pages = pages();
        let source = SourceText::new(&pages);
        assert_eq!(
            source.verdict("“we CONCLUDE that scaling helps”", Some(2)),
            Verdict::Verbatim { page: 2 }
        );
    }

    #[test]
    fn should_report_the_true_page_when_the_cited_page_is_wrong() {
        let pages = pages();
        let source = SourceText::new(&pages);
        assert_eq!(
            source.verdict("We conclude that scaling helps", Some(1)),
            Verdict::Verbatim { page: 2 }
        );
    }

    #[test]
    fn should_offer_the_source_span_when_a_quote_is_paraphrased_slightly() {
        let pages = pages();
        let source = SourceText::new(&pages);
        let Verdict::Near(found) = source.verdict(
            "Accuracy versus the model size for all of the methods",
            Some(2),
        ) else {
            panic!("expected a near match");
        };
        assert_eq!(found.text, "Accuracy versus model size for all methods.");
        assert_eq!(found.page, 2);
        assert!(found.similarity() < 1.0 && found.similarity() >= 0.6);
    }

    #[test]
    fn should_join_a_hyphenated_line_break_when_returning_a_source_span() {
        let pages = pages();
        let source = SourceText::new(&pages);
        let Verdict::Near(found) =
            source.verdict("We study the optimisation of sparse models", None)
        else {
            panic!("expected a near match");
        };
        assert_eq!(found.text, "We study the optimization of sparse models");
    }

    #[test]
    fn should_reject_a_fabricated_quote() {
        let pages = pages();
        let source = SourceText::new(&pages);
        match source.verdict(
            "Transformers outperform recurrent networks on every benchmark",
            Some(1),
        ) {
            Verdict::Missing => {}
            Verdict::Near(found) => {
                assert!(found.similarity() < LOCAL_REPAIR_SIMILARITY, "{found:?}");
            }
            Verdict::Verbatim { .. } => panic!("fabricated quote admitted"),
        }
    }

    #[test]
    fn should_check_short_quotes_by_containment_only() {
        let pages = pages();
        let source = SourceText::new(&pages);
        assert_eq!(
            source.verdict("Figure 2", Some(2)),
            Verdict::Verbatim { page: 2 }
        );
        assert_eq!(source.verdict("Table 9", Some(2)), Verdict::Missing);
        assert_eq!(source.verdict("", Some(2)), Verdict::Missing);
        assert_eq!(source.verdict("— … —", Some(2)), Verdict::Missing);
    }

    #[test]
    fn should_survive_an_empty_document() {
        let source = SourceText::new(&[]);
        assert_eq!(
            source.verdict("anything at all here", Some(1)),
            Verdict::Missing
        );
    }

    fn artifacts() -> Value {
        json!({
            "abstract": {"text": "We study the optimization", "page": 1, "confidence": "high"},
            "card": {"thesis": {"text": "t", "evidence": [{"quote": "a bound", "page": 1}], "confidence": "high"}},
            "claims": [
                {"id": "c1", "evidence": [{"quote": "q0", "page": 1}, {"quote": "q1", "page": 2}]},
                {"id": "c2", "evidence": []}
            ],
            "supercut": [
                {"kind": "source", "text": "s", "page": 1},
                {"kind": "connector", "text": "c", "page": null}
            ],
            "objects": [{"caption": "Figure 2", "page": 2}, {"caption": null, "page": 1}],
            "citations": [{"context": "x", "page": 1, "reference_key": "1"}],
            "relations": [{"quote": "y", "page": 2, "reference_key": "1"}]
        })
    }

    #[test]
    fn should_find_every_quote_site_and_skip_connectors_and_null_captions() {
        let sites = quote_sites(&artifacts());
        let pointers = sites
            .iter()
            .map(|site| site.quote_pointer.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            pointers,
            [
                "/abstract/text",
                "/card/thesis/evidence/0/quote",
                "/claims/0/evidence/0/quote",
                "/claims/0/evidence/1/quote",
                "/supercut/0/text",
                "/citations/0/context",
                "/relations/0/quote",
                "/objects/0/caption",
            ]
        );
    }

    #[test]
    fn should_remove_elements_without_shifting_later_removals() {
        let mut value = artifacts();
        apply_removals(
            &mut value,
            &[
                Removal::Element("/claims/0/evidence/0".to_owned()),
                Removal::Element("/claims/0/evidence/1".to_owned()),
                Removal::Element("/supercut/0".to_owned()),
                Removal::Null("/abstract/text".to_owned()),
                Removal::Element("/claims/0/evidence/1".to_owned()),
                Removal::Element("/claims/9".to_owned()),
            ],
        );
        assert_eq!(value["claims"][0]["evidence"], json!([]));
        assert_eq!(value["supercut"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["supercut"][0]["kind"], "connector");
        assert!(value["abstract"]["text"].is_null());
        assert_eq!(value["claims"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn should_count_verbatim_near_missing_and_wrong_page_sites() {
        let pages = pages();
        let source = SourceText::new(&pages);
        let value = json!({
            "abstract": {"text": "We conclude that scaling helps", "page": 1},
            "citations": [
                {"context": "Accuracy versus the model size for all methods", "page": 2},
                {"context": "Entirely invented sentence about nothing here", "page": 2}
            ]
        });
        let counts = check(&source, &value);
        assert_eq!(
            counts,
            CheckCounts {
                sites: 3,
                verbatim: 1,
                near: 1,
                missing: 1,
                wrong_page: 1
            }
        );
    }
}
