//! Deterministic bibliography evidence and paper-local citation links.
//!
//! Structured fields are fallible interpretations of retained authored text.
//! A unique printed citation key establishes a local destination, never a Work
//! or Person identity. All public offsets remain reading-index UTF-16 offsets.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use super::{ObjectMention, PaperObject, PaperObjectKind, ReadingIndexAnchor};
use crate::{
    domain::TextRect,
    source_index::{ReadingIndex, ReadingToken, TextRange},
};

macro_rules! pattern {
    ($source:literal) => {{
        static PATTERN: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new($source).expect("constant bibliography regex"));
        &*PATTERN
    }};
}

/// Categorical evidence, deliberately not an uncalibrated probability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldConfidence {
    Missing,
    Heuristic,
    Explicit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParsedField<T> {
    pub value: Option<T>,
    pub confidence: FieldConfidence,
}

impl<T> ParsedField<T> {
    const fn new(value: Option<T>, confidence: FieldConfidence) -> Self {
        Self {
            confidence: if value.is_some() {
                confidence
            } else {
                FieldConfidence::Missing
            },
            value,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BibliographicFields {
    pub printed_key: ParsedField<String>,
    /// Printed author strings; no name-order guess or identity resolution.
    pub authors: ParsedField<Vec<String>>,
    pub title: ParsedField<String>,
    /// Preserve disambiguating suffixes such as `2020a`.
    pub year: ParsedField<String>,
    pub venue: ParsedField<String>,
    /// Original identifier casing and punctuation, without URL/prefix wrappers.
    pub doi: ParsedField<String>,
    /// Retain a printed arXiv version; Work-level normalization happens later.
    pub arxiv_id: ParsedField<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationStyle {
    Numeric,
    Superscript,
    AuthorYear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    MissingTarget,
    AmbiguousTarget,
    UnsupportedRange,
    AmbiguousMarker,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnresolvedCitation {
    pub text: String,
    pub key: String,
    pub style: CitationStyle,
    pub reason: UnresolvedReason,
    /// Evidence candidates, never selected destinations.
    pub candidate_ids: Vec<String>,
    #[serde(flatten)]
    pub mention: ObjectMention,
}

#[derive(Default)]
pub struct Bibliography {
    pub entries: Vec<PaperObject>,
    pub unresolved: Vec<UnresolvedCitation>,
}

/// One monotone map supports checked conversion in both directions in O(log n).
/// Repeated byte positions denote the interior of a UTF-16 surrogate pair and
/// are rejected when reading a public source span.
struct Source<'a> {
    index: &'a ReadingIndex,
    bytes: Vec<usize>,
    tokens: Vec<&'a ReadingToken>,
}

impl<'a> Source<'a> {
    fn new(index: &'a ReadingIndex) -> Self {
        let mut bytes = Vec::with_capacity(index.text.len() + 1);
        for (byte, ch) in index.text.char_indices() {
            bytes.extend(std::iter::repeat_n(byte, ch.len_utf16()));
        }
        bytes.push(index.text.len());
        let mut tokens: Vec<_> = index.tokens.iter().collect();
        tokens.sort_by_key(|token| (token.start, token.end));
        Self {
            index,
            bytes,
            tokens,
        }
    }

    fn byte(&self, offset: usize) -> Option<usize> {
        let byte = *self.bytes.get(offset)?;
        (offset == 0 || self.bytes[offset - 1] != byte).then_some(byte)
    }

    fn text(&self, span: TextRange) -> Option<&'a str> {
        self.index
            .text
            .get(self.byte(span.start)?..self.byte(span.end)?)
    }

    fn range(&self, start: usize, end: usize) -> TextRange {
        TextRange {
            start: self.bytes.partition_point(|byte| *byte < start),
            end: self.bytes.partition_point(|byte| *byte < end),
        }
    }

    fn tokens(&self, span: TextRange) -> impl Iterator<Item = &'a ReadingToken> + '_ {
        // Reading-index tokens are disjoint, ordered source words. Starting at
        // the first intersecting word avoids a full-paper scan per citation.
        let start = self.tokens.partition_point(|token| token.end <= span.start);
        self.tokens[start..]
            .iter()
            .copied()
            .take_while(move |token| token.start < span.end)
    }

    fn anchor(&self, span: TextRange) -> ReadingIndexAnchor {
        ReadingIndexAnchor {
            page: self.tokens(span).next().map_or_else(
                || {
                    self.index
                        .pages
                        .iter()
                        .find(|page| (page.start..page.end).contains(&span.start))
                        .map_or(1, |page| page.number)
                },
                |token| token.page,
            ),
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Clone)]
struct Block {
    span: TextRange,
    heading: bool,
    caption: bool,
}

#[derive(Default)]
struct Entry {
    spans: Vec<TextRange>,
    key: Option<String>,
}

fn label_regex() -> &'static Regex {
    pattern!(r"\A[\t ]*(?:\[([\p{L}\d][\p{L}\d+,:.\-–]{0,30})\]|\((\d{1,4})\)|(\d{1,4})[.)])[\t ]+")
}

fn year_regex() -> &'static Regex {
    pattern!(r"\b(?:18|19|20)\d{2}[a-z]?\b")
}

fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trimmed(value: &str) -> Option<String> {
    let value = clean(value.trim_matches(|ch: char| ch.is_whitespace() || ".,;:".contains(ch)));
    (!value.is_empty()).then_some(value)
}

fn key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

/// Extract evidence from the existing index; no filesystem, models, or network.
#[must_use]
pub fn extract(index: &ReadingIndex) -> Bibliography {
    let source = Source::new(index);
    let mut blocks: Vec<_> = index
        .objects
        .paragraph
        .iter()
        .flat_map(|paragraph| {
            let spans = if paragraph.spans.is_empty() {
                vec![TextRange {
                    start: paragraph.start,
                    end: paragraph.end,
                }]
            } else {
                paragraph.spans.clone()
            };
            spans
                .into_iter()
                .filter(|span| span.start < span.end && source.text(*span).is_some())
                .map(|span| Block {
                    span,
                    heading: paragraph.kind == "heading",
                    caption: paragraph.kind == "caption",
                })
        })
        .collect();
    blocks.sort_by_key(|block| (block.span.start, block.span.end));
    blocks.dedup_by_key(|block| (block.span.start, block.span.end));
    let Some(heading) = blocks.iter().position(|block| pattern!(r"(?i)^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:references(?: and notes| cited)?|bibliography|literature cited|works cited)\s*[:.]?$")
        .is_match(source.text(block.span).unwrap_or_default().trim())) else { return Bibliography::default() };
    let start = blocks[heading].span.end;
    let end = blocks[heading + 1..].iter().find(|block| block.heading && pattern!(r"(?i)^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:appendix|appendices|supplement|acknowledg)")
        .is_match(source.text(block.span).unwrap_or_default().trim())).map_or(source.bytes.len() - 1, |block| block.span.start);
    let entries = split_entries(
        &source,
        blocks[heading + 1..]
            .iter()
            .filter(|block| block.span.start < end && !block.caption),
    );
    let mut result = Bibliography {
        entries: entries
            .into_iter()
            .enumerate()
            .map(|(at, entry)| object(&source, entry, at))
            .collect(),
        unresolved: vec![],
    };
    resolve_mentions(&source, TextRange { start, end }, &mut result);
    result
}

struct EntryLabel {
    start: usize,
    key: String,
    dotted: bool,
    publication_year: bool,
}

fn entry_labels(source: &Source<'_>, span: TextRange) -> Vec<EntryLabel> {
    let text = source.text(span).unwrap_or_default();
    let base = source.byte(span.start).unwrap_or_default();
    let label = |text: &str, offset: usize| {
        let captures = label_regex().captures(text)?;
        let whole = captures.get(0)?;
        (whole.start() == 0).then(|| EntryLabel {
            start: offset,
            dotted: captures.get(3).is_some(),
            publication_year: captures.get(3).is_some_and(|key| {
                year_regex()
                    .find(key.as_str())
                    .is_some_and(|year| year.as_str() == key.as_str())
            }),
            key: (1..=3)
                .find_map(|group| captures.get(group))
                .expect("printed key capture")
                .as_str()
                .to_owned(),
        })
    };
    let mut labels = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
        .filter_map(|offset| label(&text[offset..], offset))
        .collect::<Vec<_>>();
    // Reading-index prose joins physical PDF lines with spaces. Only a verified
    // page/line transition can add a boundary that is absent from the text;
    // inline bracket expressions and raised markers must not split entries.
    let tokens = source.tokens(span).collect::<Vec<_>>();
    for pair in tokens.windows(2) {
        if starts_new_line(pair[0], pair[1])
            && let Some(byte) = source.byte(pair[1].start)
            && let Some(offset) = byte.checked_sub(base).filter(|offset| *offset < text.len())
            && let Some(found) = label(&text[offset..], offset)
        {
            labels.push(found);
        }
    }
    labels.sort_by_key(|label| label.start);
    labels.dedup_by_key(|label| label.start);
    labels
}

fn starts_new_line(previous: &ReadingToken, current: &ReadingToken) -> bool {
    if previous.page != current.page {
        return true;
    }
    let (Some(left), Some(right)) = (previous.rects.last(), current.rects.first()) else {
        return false;
    };
    if [left, right].into_iter().any(|rect| {
        ![rect.x_min, rect.x_max, rect.y_min, rect.y_max]
            .into_iter()
            .all(f32::is_finite)
            || rect.x_max <= rect.x_min
            || rect.y_max <= rect.y_min
    }) {
        return false;
    }
    // Raised inline citation-like text is smaller than its surrounding line;
    // vertical displacement alone must not turn that marker into a new entry.
    if right.y_max - right.y_min < (left.y_max - left.y_min) * 0.65 {
        return false;
    }
    let short = (left.y_max - left.y_min).min(right.y_max - right.y_min);
    let tall = (left.y_max - left.y_min).max(right.y_max - right.y_min);
    let overlap = (left.y_max.min(right.y_max) - left.y_min.max(right.y_min)).max(0.0);
    let displacement = ((left.y_min + left.y_max) - (right.y_min + right.y_max)).abs() * 0.5;
    overlap <= short * 0.2
        && displacement > tall * 0.65
        && (short.mul_add(0.5, right.x_min) < left.x_min || displacement > tall * 1.5)
}

fn dotted_sequence_starts(
    source: &Source<'_>,
    blocks: &[(&Block, Vec<EntryLabel>)],
) -> BTreeSet<usize> {
    // Lookahead can establish a bibliography whose first actual key resembles
    // a year. Each neighboring dotted key also needs independently printed
    // author/title structure; two isolated continuation years do not suffice.
    let candidates = blocks
        .iter()
        .flat_map(|(block, labels)| {
            let text = source.text(block.span).unwrap_or_default();
            let base = source.byte(block.span.start).unwrap_or_default();
            labels.iter().enumerate().map(move |(at, label)| {
                let end = labels.get(at + 1).map_or(text.len(), |next| next.start);
                let fields = parse_fields(&text[label.start..end], Some(label.key.clone()));
                let author = fields
                    .authors
                    .value
                    .as_ref()
                    .and_then(|authors| authors.first());
                let structured = fields.title.value.is_some()
                    && author.is_some_and(|author| {
                        crate::kb::names::parse_name(author.trim_end_matches('.'))
                            .unparsed_reason
                            .is_none()
                    });
                (base + label.start, label, structured)
            })
        })
        .collect::<Vec<_>>();
    candidates
        .windows(2)
        .filter_map(|pair| {
            let (start, first, first_structured) = pair[0];
            let (_, second, second_structured) = pair[1];
            let next = first
                .key
                .parse::<u32>()
                .ok()
                .and_then(|number| number.checked_add(1));
            (first.publication_year
                && second.dotted
                && first_structured
                && second_structured
                && next.is_some()
                && next == second.key.parse::<u32>().ok())
            .then_some(start)
        })
        .collect()
}

fn split_entries<'a>(source: &Source<'_>, blocks: impl Iterator<Item = &'a Block>) -> Vec<Entry> {
    let blocks = blocks
        .map(|block| (block, entry_labels(source, block.span)))
        .collect::<Vec<_>>();
    let supported_starts = dotted_sequence_starts(source, &blocks);
    let mut entries: Vec<Entry> = Vec::new();
    for (block, candidates) in blocks {
        let text = source.text(block.span).unwrap_or_default();
        let base = source.byte(block.span.start).unwrap_or_default();
        let mut previous_dot_number = entries.last().and_then(|entry| {
            let raw = source.text(*entry.spans.first()?)?;
            let label = label_regex().captures(raw)?.get(3)?;
            label.as_str().parse::<u32>().ok()
        });
        let labels = candidates
            .into_iter()
            .filter(|label| {
                let number = label.key.parse::<u32>().ok();
                // A plain year ending in a period is common on a continuation
                // line. Only an established adjacent dotted-number sequence
                // justifies interpreting it as a four-digit entry key.
                if label.publication_year
                    && !supported_starts.contains(&(base + label.start))
                    && previous_dot_number.is_none_or(|previous| Some(previous + 1) != number)
                {
                    return false;
                }
                previous_dot_number = label.dotted.then_some(number).flatten();
                true
            })
            .collect::<Vec<_>>();
        if labels.is_empty()
            && let Some(previous) = entries.last_mut().filter(|entry| {
                entry.key.is_some() && !starts_mixed_unnumbered_entry(source, entry, text)
            })
        {
            // A printed key governs its complete entry until another printed
            // key begins. A wrapped author list followed by its year is not an
            // independent unnumbered entry inside a numbered bibliography.
            // An explicit new surname/initial/year after an already complete
            // keyed entry can establish a mixed-convention entry instead.
            previous.spans.push(block.span);
            continue;
        }
        if labels.is_empty() {
            // A block can finish a preceding entry and start the next one.
            // Find all author/date boundaries before attaching its leading
            // continuation as another disjoint source member.
            let mut starts = vec![0];
            for (offset, _) in text.match_indices('\n') {
                let remainder = &text[offset + 1..];
                let line = remainder.lines().next().unwrap_or_default();
                if author_year_prefix(line).is_some() {
                    starts.push(offset + 1);
                }
            }
            starts.push(text.len());
            for pair in starts.windows(2) {
                let segment = &text[pair[0]..pair[1]];
                let span = source.range(base + pair[0], base + pair[1]);
                if segment.trim().is_empty() {
                    continue;
                }
                if pair[0] == 0
                    && author_year_prefix(segment).is_none()
                    && let Some(previous) = entries.last_mut()
                {
                    previous.spans.push(span);
                } else {
                    entries.push(Entry {
                        spans: vec![span],
                        key: None,
                    });
                }
            }
            continue;
        }
        for (at, label) in labels.iter().enumerate() {
            if at == 0
                && label.start > 0
                && let Some(previous) = entries.last_mut()
            {
                previous.spans.push(source.range(base, base + label.start));
            }
            let end = labels.get(at + 1).map_or(text.len(), |next| next.start);
            entries.push(Entry {
                spans: vec![source.range(base + label.start, base + end)],
                key: Some(label.key.clone()),
            });
        }
    }
    entries
}

fn entry_text(source: &Source<'_>, entry: &Entry) -> String {
    entry
        .spans
        .iter()
        .filter_map(|span| source.text(*span))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

fn starts_mixed_unnumbered_entry(source: &Source<'_>, previous: &Entry, text: &str) -> bool {
    let Some((author_end, _)) = author_year_prefix(text) else {
        return false;
    };
    // Mixed conventions require an explicit surname/initial form. A venue
    // followed by its year or a wrapped author list is insufficient evidence.
    if !pattern!(r"^[\p{L}\p{M}'’\- ]+,\s+(?:\p{Lu}\.)").is_match(&text[..author_end]) {
        return false;
    }
    let fields = parse_fields(&entry_text(source, previous), previous.key.clone());
    fields.authors.value.is_some() && fields.year.value.is_some() && fields.title.value.is_some()
}

fn object(source: &Source<'_>, entry: Entry, at: usize) -> PaperObject {
    let raw = entry_text(source, &entry);
    let anchor = source.anchor(TextRange {
        start: entry.spans[0].start,
        end: entry.spans.last().expect("entry member").end,
    });
    let region = union(
        entry
            .spans
            .iter()
            .flat_map(|span| source.tokens(*span))
            .filter(|token| token.page == anchor.page)
            .flat_map(|token| token.rects.iter().copied()),
    );
    let fields = parse_fields(&raw, entry.key);
    PaperObject {
        id: format!("bib-{}", at + 1),
        kind: PaperObjectKind::BibEntry {
            bibliography: fields,
        },
        label: raw.clone(),
        page: anchor.page,
        text: raw,
        anchor,
        member_anchors: entry
            .spans
            .iter()
            .map(|span| source.anchor(*span))
            .collect(),
        region,
        confidence: "heuristic".into(),
        mentions: vec![],
    }
}

fn union(rects: impl Iterator<Item = TextRect>) -> Option<TextRect> {
    rects
        .filter(|rect| {
            [rect.x_min, rect.x_max, rect.y_min, rect.y_max]
                .iter()
                .all(|value| value.is_finite())
                && rect.x_max > rect.x_min
                && rect.y_max > rect.y_min
        })
        .reduce(|a, b| TextRect {
            x_min: a.x_min.min(b.x_min),
            y_min: a.y_min.min(b.y_min),
            x_max: a.x_max.max(b.x_max),
            y_max: a.y_max.max(b.y_max),
        })
}

/// An author/date prefix accepts both parenthesized and plain Harvard years.
fn author_year_prefix(text: &str) -> Option<(usize, usize)> {
    let year = year_regex().find(text)?;
    let prefix = text[..year.start()].trim_end();
    let start = prefix
        .strip_suffix(['(', '['])
        .map_or_else(|| year.start(), str::len);
    let authors = text[..start].trim();
    let end = year.end() + usize::from(text[year.end()..].starts_with([')', ']']));
    (start < 250
        && authors.chars().next().is_some_and(char::is_alphabetic)
        && !authors.contains(['/', ':'])
        && sentence_boundary(authors).is_none())
    .then_some((start, end))
}

fn sentence_boundary(text: &str) -> Option<usize> {
    for (at, ch) in text.char_indices() {
        if ch != '.' || !text[at + 1..].starts_with(char::is_whitespace) {
            continue;
        }
        let prefix = &text[..at];
        let last = prefix
            .rsplit(|ch: char| ch.is_whitespace() || ch == ',')
            .next()
            .unwrap_or_default();
        // Author initials and et al. are not author/title boundaries.
        if last
            .chars()
            .all(|ch| ch.is_alphabetic() || ch == '-' || ch == '.')
            && (last.chars().filter(|ch| ch.is_alphabetic()).count() == 1
                || last.contains('.')
                || last.contains('-'))
        {
            continue;
        }
        if last == "al" && prefix.ends_with("et al") {
            continue;
        }
        return Some(at);
    }
    None
}

#[must_use]
pub fn parse_fields(raw: &str, printed_key: Option<String>) -> BibliographicFields {
    let stripped = label_regex()
        .find(raw)
        .filter(|label| label.start() == 0)
        .map_or(raw, |label| &raw[label.end()..]);
    let compact = clean(stripped);
    let year = year_regex()
        .find_iter(&compact)
        .find(|year| {
            let token = compact[..year.start()]
                .rsplit(char::is_whitespace)
                .next()
                .unwrap_or_default();
            !token.contains(['/', ':']) && !token.ends_with('.')
        })
        .map(|year| year.as_str().to_owned());
    let (author_text, remaining) = if let Some((start, end)) = author_year_prefix(&compact) {
        (&compact[..start], &compact[end..])
    } else if let Some(end) = sentence_boundary(&compact) {
        (&compact[..end], &compact[end + 1..])
    } else {
        ("", compact.as_str())
    };
    let authors = split_authors(author_text);
    let remaining =
        remaining.trim_start_matches(|ch: char| ch.is_whitespace() || ".,;:".contains(ch));
    let (title, venue) = title_venue(remaining);
    BibliographicFields {
        printed_key: ParsedField::new(printed_key, FieldConfidence::Explicit),
        authors: ParsedField::new(
            (!authors.is_empty()).then_some(authors),
            FieldConfidence::Heuristic,
        ),
        title: ParsedField::new(title, FieldConfidence::Heuristic),
        year: ParsedField::new(year, FieldConfidence::Heuristic),
        venue: ParsedField::new(venue, FieldConfidence::Heuristic),
        doi: ParsedField::new(extract_doi(raw), FieldConfidence::Explicit),
        arxiv_id: ParsedField::new(extract_arxiv(raw), FieldConfidence::Explicit),
    }
}

fn split_authors(text: &str) -> Vec<String> {
    let text = text.trim().trim_end_matches([',', ';']);
    if text.is_empty() {
        return vec![];
    }
    // Keep surname, initials together. Semicolon / and delimit explicit authors;
    // comma-style lists are paired only when the following item is initials.
    let mut result = Vec::new();
    for item in pattern!(r"\s+(?:and|&)\s+|\s*;\s*").split(text) {
        let pieces: Vec<_> = item
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        let mut at = 0;
        while at < pieces.len() {
            let first = pieces[at];
            if let Some(initials) = pieces
                .get(at + 1)
                .filter(|part| pattern!(r"^(?:\p{Lu}[.\s-]*)+$").is_match(part))
            {
                result.push(format!("{first}, {initials}"));
                at += 2;
            } else {
                result.push(first.to_owned());
                at += 1;
            }
        }
    }
    result
}

fn title_venue(text: &str) -> (Option<String>, Option<String>) {
    if text.is_empty() {
        return (None, None);
    }
    if let Some(open) = text
        .chars()
        .next()
        .filter(|ch| matches!(ch, '"' | '“' | '‘'))
    {
        let close = match open {
            '“' => '”',
            '‘' => '’',
            _ => '"',
        };
        if let Some(end) = text[open.len_utf8()..].find(close) {
            let end = open.len_utf8() + end;
            return (
                trimmed(&text[open.len_utf8()..end]),
                clean_venue(&text[end + close.len_utf8()..]),
            );
        }
    }
    let period = sentence_boundary(text).unwrap_or(text.len());
    let terminal = text.char_indices().find_map(|(at, ch)| {
        (matches!(ch, '?' | '!')
            && (at + 1 == text.len() || text[at + 1..].starts_with(char::is_whitespace)))
        .then_some(at)
    });
    // Question/exclamation punctuation belongs to the title. The separating
    // period follows the existing convention of being omitted from the field.
    let (end, remaining) = terminal.filter(|at| *at < period).map_or_else(
        || (period, text.get(period + 1..).unwrap_or_default()),
        |at| (at + 1, &text[at + 1..]),
    );
    let title = &text[..end];
    if !title.chars().any(char::is_alphabetic)
        || pattern!(r"(?i)^(?:doi\s*:|https?://|arxiv\s*:)").is_match(title)
    {
        return (None, None);
    }
    (trimmed(title), clean_venue(remaining))
}

fn clean_venue(text: &str) -> Option<String> {
    let cutoff = pattern!(r"(?i)\b(?:doi\s*:|https?://|arxiv\s*:)|\b(?:18|19|20)\d{2}[a-z]?\b")
        .find(text)
        .map_or(text.len(), |matched| matched.start());
    trimmed(text[..cutoff].trim_end_matches(|ch: char| ch.is_whitespace() || "([".contains(ch)))
}

fn extract_doi(raw: &str) -> Option<String> {
    let mut identifiers: BTreeSet<_> = pattern!(r"(?i)\b10\.\d{4,9}/")
        .find_iter(raw)
        .filter_map(|matched| read_doi(&raw[matched.start()..]))
        .collect();
    (identifiers.len() == 1)
        .then(|| identifiers.pop_first())
        .flatten()
}

fn read_doi(raw: &str) -> Option<String> {
    let mut token = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\n' || ch == '\r' {
            // Only join physical line breaks. Ordinary spaces terminate IDs;
            // preserve all identifier characters, including a printed hyphen.
            while chars
                .peek()
                .is_some_and(|next| matches!(next, ' ' | '\t' | '\r' | '\n'))
            {
                chars.next();
            }
            if token.ends_with(['.', ',', ';']) {
                break;
            }
            // A slash/hyphen or numeric continuation supports joining a wrapped
            // identifier. A following prose line is a boundary. An otherwise
            // ambiguous bare-word continuation does not justify an explicit ID.
            if !token.ends_with(['/', '-', '_']) && !chars.peek().is_some_and(char::is_ascii_digit)
            {
                let remainder = chars.clone().collect::<String>();
                let next_line = remainder.lines().next().unwrap_or_default().trim();
                if next_line.split_whitespace().count() > 1 || next_line.is_empty() {
                    break;
                }
                return None;
            }
            continue;
        }
        if ch.is_whitespace() || matches!(ch, '"' | '“' | '”') {
            break;
        }
        token.push(ch);
    }
    trim_identifier(&mut token);
    if token.to_ascii_lowercase().starts_with("10.48550/arxiv.") {
        let suffix = &token["10.48550/arxiv.".len()..];
        // This DOI namespace encodes an arXiv identifier. A line break after
        // the dot can leave a syntactically plausible generic DOI prefix that
        // is not a complete arXiv identity. Withhold it instead of inventing
        // an explicit truncated DOI; the original entry remains available.
        if extract_arxiv(&format!("arxiv:{suffix}")).as_deref() != Some(suffix) {
            return None;
        }
    }
    (token
        .split_once('/')
        .is_some_and(|(_, suffix)| !suffix.is_empty()))
    .then_some(token)
}

fn trim_identifier(value: &mut String) {
    loop {
        let Some(last) = value.chars().last() else {
            break;
        };
        let unmatched = matches!(last, ')' | ']' | '}') && {
            let open = match last {
                ')' => '(',
                ']' => '[',
                _ => '{',
            };
            value.matches(last).count() > value.matches(open).count()
        };
        if ".,;:".contains(last) || unmatched {
            value.truncate(value.len() - last.len_utf8());
        } else {
            break;
        }
    }
}

fn extract_arxiv(raw: &str) -> Option<String> {
    let mut identifiers: BTreeSet<_> = pattern!(r"(?i)(?:arxiv\s*:\s*|arxiv\.org/(?:abs|pdf)/)((?:\d{4}\.\d{4,5}|[a-z][a-z.\-]+/\d{7})(?:v\d+)?)(?:\.pdf)?\b")
        .captures_iter(raw).filter_map(|capture| capture.get(1).map(|matched| matched.as_str().to_owned())).collect();
    (identifiers.len() == 1)
        .then(|| identifiers.pop_first())
        .flatten()
}

fn resolve_mentions(source: &Source<'_>, bibliography: TextRange, result: &mut Bibliography) {
    let mut printed: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut author_year: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (at, entry) in result.entries.iter().enumerate() {
        let PaperObjectKind::BibEntry {
            bibliography: fields,
        } = &entry.kind
        else {
            continue;
        };
        if let Some(label) = &fields.printed_key.value {
            printed.entry(key(label)).or_default().push(at);
        }
        if let (Some(authors), Some(year)) = (&fields.authors.value, &fields.year.value)
            && let Some(author) = authors.first().and_then(|author| citation_family(author))
        {
            author_year
                .entry(format!("{}:{year}", key(&author)))
                .or_default()
                .push(at);
        }
    }
    let sentences = sentences(source);
    let mut resolver = Resolver {
        source,
        bibliography,
        sentences,
        seen: BTreeSet::new(),
        result,
    };
    let body = &source.index.text;
    let bracket_convention = pattern!(r"\[([^\]\n]{1,120})\]")
        .captures_iter(body)
        .any(|matched| {
            let whole = matched.get(0).expect("bracketed citation");
            let span = source.range(whole.start(), whole.end());
            (!(bibliography.start..bibliography.end).contains(&span.start))
                && citation_keys(&matched[1]).iter().any(|item| {
                    item.as_ref()
                        .is_ok_and(|key| printed.get(key).is_some_and(|targets| targets.len() == 1))
                })
        });
    let mut printed_occurrences = Vec::new();
    for matched in
        pattern!(r"\[([^\]\n]{1,120})\]|\((\d{1,4}(?:\s*[,;–−-]\s*\d{1,4})*)\)").captures_iter(body)
    {
        let whole = matched.get(0).expect("matched citation");
        let prefix = &body[..whole.start()];
        let mut prefix_start = prefix.len().saturating_sub(60);
        while !prefix.is_char_boundary(prefix_start) {
            prefix_start += 1;
        }
        if pattern!(r"(?i)\b(?:eq(?:uation)?s?|sec(?:tion)?s?|fig(?:ure)?s?|tables?|theorems?|lemmas?|propositions?)\.?\s*$").is_match(&prefix[prefix_start..]) { continue; }
        let content = matched
            .get(1)
            .or_else(|| matched.get(2))
            .expect("citation content")
            .as_str();
        // An author-year date is not an unrecognized numeric citation. Printed
        // four-digit keys still work when the bibliography actually has one.
        if matched.get(2).is_some()
            && year_regex()
                .find(content)
                .is_some_and(|year| year.as_str() == content)
            && !printed.contains_key(content)
        {
            continue;
        }
        let span = source.range(whole.start(), whole.end());
        let cited = citation_keys(content);
        if cited
            .iter()
            .any(|item| item.as_ref().is_ok_and(|key| printed.contains_key(key)))
        {
            printed_occurrences.push(span);
        }
        for item in cited {
            let ambiguous = matched.get(2).is_some()
                && bracket_convention
                && !citation_cue(source, span.start, &printed);
            resolver.resolve_marker(span, CitationStyle::Numeric, item, &printed, ambiguous);
        }
    }
    resolve_superscripts(&mut resolver, &printed, bracket_convention);
    for matched in pattern!(r"((?:(?i:van|von|de|del|della|da|dos|das|la|der|den)\s+){0,3}[\p{L}][\p{L}\p{M}'’−-]+)(?:\s+(?:et\s+al\.?|(?:and|&)\s+[\p{L}][\p{L}\p{M}'’−-]+))?(?:\s*[,(\[]\s*|\s+)((?:18|19|20)\d{2}[a-z]?(?:\s*[,;]\s*(?:(?:18|19|20)\d{2}[a-z]?|[a-z]))*)\b").captures_iter(body) {
        let whole = matched.get(0).expect("author-year citation");
        let author = matched.get(1).expect("citation author").as_str();
        let years = matched.get(2).expect("citation years").as_str();
        let end = whole.end() + usize::from(whole.as_str().contains(['(', '[']) && body[whole.end()..].starts_with([')', ']']));
        let occurrence = source.range(whole.start(), end);
        let at = printed_occurrences.partition_point(|printed| printed.end <= occurrence.start);
        if printed_occurrences.get(at).is_some_and(|printed| printed.start < occurrence.end) { continue; }
        let mut inherited = None;
        for part in years.split([',', ';']).map(str::trim) {
            let year = if part.len() == 1 { format!("{}{part}", inherited.unwrap_or_default()) } else { inherited = part.get(..4); part.to_owned() };
            resolver.resolve(occurrence, CitationStyle::AuthorYear, Ok(format!("{}:{year}", key(author))), &author_year);
        }
    }
}

fn citation_cue(source: &Source<'_>, start: usize, printed: &BTreeMap<String, Vec<usize>>) -> bool {
    let Some(byte) = source.byte(start) else {
        return false;
    };
    // A bounded clause prefix allows explicit mixed conventions, e.g.
    // "See [2] and (3)". "See (3)" alone can refer to an equation or list.
    let prefix = &source.index.text[..byte];
    let clause = prefix
        .rsplit(['.', ';', ':', '!', '?', '\n'])
        .next()
        .unwrap_or_default();
    if clause.chars().count() > 100 {
        return false;
    }
    if pattern!(r"(?i)\b(?:citations?|references?|cited\s+in)\s*$").is_match(clause) {
        return true;
    }
    pattern!(r"\[([^\]\n]{1,120})\]\s*,?\s*(?:(?:and|or|also)\s*)?$")
        .captures(clause)
        .is_some_and(|matched| {
            citation_keys(&matched[1]).iter().any(|item| {
                item.as_ref()
                    .is_ok_and(|key| printed.get(key).is_some_and(|targets| targets.len() == 1))
            })
        })
}

fn resolve_superscripts(
    resolver: &mut Resolver<'_, '_>,
    printed: &BTreeMap<String, Vec<usize>>,
    bracket_convention: bool,
) {
    let source = resolver.source;
    for pair in source.tokens.windows(2) {
        let before = pair[0];
        let token = pair[1];
        if token.page != before.page
            || !pattern!(r"^\d{1,4}(?:[,–−-]\d{1,4})*$").is_match(&token.text)
        {
            continue;
        }
        let (Some(a), Some(b)) = (before.rects.last(), token.rects.first()) else {
            continue;
        };
        if b.y_max >= (a.y_max - a.y_min).mul_add(-0.2, a.y_max)
            || b.y_min > a.y_min
            || b.y_max < a.y_min
            || b.x_min < a.x_max - 2.0
            || b.x_min - a.x_max > 6.0
        {
            continue;
        }
        for item in citation_keys(&token.text) {
            resolver.resolve_marker(
                TextRange {
                    start: token.start,
                    end: token.end,
                },
                CitationStyle::Superscript,
                item,
                printed,
                bracket_convention && !citation_cue(source, token.start, printed),
            );
        }
    }
}

fn citation_family(author: &str) -> Option<String> {
    let parsed = crate::kb::names::parse_name(author);
    let mut families: BTreeSet<_> = parsed
        .alternatives
        .into_iter()
        .map(|parts| {
            parts
                .particles
                .into_iter()
                .chain([parts.family_name])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    // Name parsing can retain several component/order interpretations. Only a
    // shared family across every alternative supplies a citation family key.
    (families.len() == 1)
        .then(|| families.pop_first())
        .flatten()
}

fn citation_keys(text: &str) -> Vec<Result<String, String>> {
    let mut keys = Vec::new();
    for part in text
        .split([',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(range) = pattern!(r"^(\d+)\s*[-–−]\s*(\d+)$").captures(part) {
            let from = range[1].parse::<u32>().ok();
            let to = range[2].parse::<u32>().ok();
            if let (Some(from), Some(to)) = (from, to)
                && to >= from
                && to - from <= 30
            {
                keys.extend((from..=to).map(|number| Ok(number.to_string())));
            } else {
                keys.push(Err(part.to_owned()));
            }
        } else if !part.contains(char::is_whitespace) {
            keys.push(Ok(key(part)));
        }
    }
    keys
}

fn sentences(source: &Source<'_>) -> Vec<TextRange> {
    if !source.index.objects.sentence.is_empty() {
        let mut sentences: Vec<_> = source
            .index
            .objects
            .sentence
            .iter()
            .copied()
            .filter(|span| source.text(*span).is_some())
            .collect();
        sentences.sort_by_key(|span| span.start);
        return sentences;
    }
    // Older indexes/fixtures lack sentence objects. Use the same Unicode
    // segmenter as the reading-index builder, preserving its source offsets.
    source
        .index
        .text
        .split_sentence_bound_indices()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(byte, text)| source.range(byte, byte + text.len()))
        .collect()
}

struct Resolver<'a, 'b> {
    source: &'a Source<'a>,
    bibliography: TextRange,
    sentences: Vec<TextRange>,
    seen: BTreeSet<(usize, usize, String, u32)>,
    result: &'b mut Bibliography,
}

impl Resolver<'_, '_> {
    fn resolve(
        &mut self,
        span: TextRange,
        style: CitationStyle,
        cited: Result<String, String>,
        targets: &BTreeMap<String, Vec<usize>>,
    ) {
        self.resolve_marker(span, style, cited, targets, false);
    }

    fn resolve_marker(
        &mut self,
        span: TextRange,
        style: CitationStyle,
        cited: Result<String, String>,
        targets: &BTreeMap<String, Vec<usize>>,
        ambiguous_marker: bool,
    ) {
        if span.start >= self.bibliography.start && span.start < self.bibliography.end {
            return;
        }
        let (key, unsupported) = match cited {
            Ok(key) => (key, false),
            Err(key) => (key, true),
        };
        let matches = targets.get(&key).map_or(&[][..], Vec::as_slice);
        let sentence_at = self
            .sentences
            .partition_point(|sentence| sentence.end <= span.start);
        // Abbreviations such as "et al." can cross Unicode sentence boundaries.
        // Retain the complete context of every intersected sentence segment.
        let sentence = self.sentences[sentence_at..]
            .iter()
            .take_while(|sentence| sentence.start < span.end)
            .fold(span, |context, sentence| TextRange {
                start: context.start.min(sentence.start),
                end: context.end.max(sentence.end),
            });
        let mut pages: BTreeMap<u32, Vec<TextRect>> = BTreeMap::new();
        for token in self.source.tokens(span) {
            pages
                .entry(token.page)
                .or_default()
                .extend(token.rects.iter().copied());
        }
        if pages.is_empty() {
            pages.insert(self.source.anchor(span).page, vec![]);
        }
        for (page, rects) in pages {
            if !self.seen.insert((span.start, span.end, key.clone(), page)) {
                continue;
            }
            let mention = ObjectMention {
                anchor: ReadingIndexAnchor {
                    page,
                    start: span.start,
                    end: span.end,
                },
                rects,
                sentence_anchor: Some(self.source.anchor(sentence)),
            };
            if matches.len() == 1 && !unsupported && !ambiguous_marker {
                self.result.entries[matches[0]].mentions.push(mention);
            } else {
                self.result.unresolved.push(UnresolvedCitation {
                    text: self.source.text(span).unwrap_or_default().to_owned(),
                    key: key.clone(),
                    style,
                    reason: if ambiguous_marker {
                        UnresolvedReason::AmbiguousMarker
                    } else if unsupported {
                        UnresolvedReason::UnsupportedRange
                    } else if matches.is_empty() {
                        UnresolvedReason::MissingTarget
                    } else {
                        UnresolvedReason::AmbiguousTarget
                    },
                    candidate_ids: matches
                        .iter()
                        .map(|at| self.result.entries[*at].id.clone())
                        .collect(),
                    mention,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::{Paragraph, Provenance, ReadingPage, TextObjects};

    fn fixture(blocks: &[(&str, &str, u32)]) -> ReadingIndex {
        let mut index = ReadingIndex {
            schema_version: crate::source_index::SCHEMA_VERSION,
            text: String::new(),
            pages: vec![],
            tokens: vec![],
            objects: TextObjects::default(),
            figures: vec![],
            gaps: vec![],
        };
        for (at, (text, kind, page)) in blocks.iter().enumerate() {
            let start = index.text.encode_utf16().count();
            let y = f32::from(u16::try_from(at).unwrap()).mul_add(30.0, 40.0);
            for word in pattern!(r"\S+").find_iter(text) {
                let left = text[..word.start()].encode_utf16().count();
                let right = text[..word.end()].encode_utf16().count();
                index.tokens.push(ReadingToken {
                    start: start + left,
                    end: start + right,
                    page: *page,
                    text: word.as_str().to_owned(),
                    provenance: Provenance::Native,
                    rects: vec![TextRect {
                        x_min: f32::from(u16::try_from(left).unwrap()).mul_add(6.0, 40.0),
                        x_max: f32::from(u16::try_from(right).unwrap()).mul_add(6.0, 40.0),
                        y_min: y,
                        y_max: y + 12.0,
                    }],
                });
            }
            index.text.push_str(text);
            let end = index.text.encode_utf16().count();
            index.objects.paragraph.push(Paragraph {
                start,
                end,
                kind: (*kind).to_owned(),
                spans: vec![],
            });
            index.objects.sentence.push(TextRange { start, end });
            if let Some(existing) = index
                .pages
                .iter_mut()
                .find(|existing| existing.number == *page)
            {
                existing.end = end;
            } else {
                index.pages.push(ReadingPage {
                    number: *page,
                    start,
                    end,
                    width: 612.0,
                    height: 792.0,
                    provenance: Provenance::Native,
                    confidence: None,
                });
            }
            index.text.push_str("\n\n");
        }
        index
    }

    fn fields(entry: &PaperObject) -> &BibliographicFields {
        let PaperObjectKind::BibEntry { bibliography } = &entry.kind else {
            panic!("expected bibliography entry")
        };
        bibliography
    }

    fn slice(index: &ReadingIndex, anchor: &ReadingIndexAnchor) -> String {
        String::from_utf16(&index.text.encode_utf16().collect::<Vec<_>>()[anchor.start..anchor.end])
            .unwrap()
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
            assert_eq!(fields(entry).printed_key.value, Some((at + 1).to_string()));
            assert_eq!(entry.mentions.len(), 1);
            assert_eq!(entry.mentions[0].anchor.page, 1);
            assert!(!entry.mentions[0].rects.is_empty());
        }
        assert!(result.unresolved.is_empty());
        assert_eq!(slice(&index, &result.entries[1].mentions[0].anchor), "(2)");
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
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
        assert_eq!(
            fields(&result.entries[2]).printed_key.value.as_deref(),
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
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
        assert_eq!(
            fields(&result.entries[0]).title.value.as_deref(),
            Some("First study")
        );
        assert_eq!(
            fields(&result.entries[3]).authors.value.as_ref().unwrap(),
            &["Jones, C.", "Brown, D."]
        );
        assert_eq!(
            fields(&result.entries[4]).authors.value.as_ref().unwrap(),
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
    fn should_leave_a_marker_unresolved_when_two_entries_match() {
        let index = fixture(&[
            ("Smith (2020) and [3] are ambiguous.", "body", 1),
            ("References", "heading", 2),
            ("[3] Smith, A. (2020). First study.", "body", 2),
            ("[3] Smith, B. (2020). Different study.", "body", 2),
        ]);
        let result = extract(&index);
        assert!(result.entries.iter().all(|entry| entry.mentions.is_empty()));
        assert_eq!(result.unresolved.len(), 2);
        assert!(
            result
                .unresolved
                .iter()
                .all(|item| item.reason == UnresolvedReason::AmbiguousTarget
                    && item.candidate_ids == ["bib-1", "bib-2"])
        );
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
            slice(&index, &result.entries[0].mentions[0].anchor),
            "Smith et al. 2020a"
        );
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
        let mentions = &result.entries[0].mentions;
        assert_eq!(mentions.len(), 2);
        assert_ne!(mentions[0].anchor.start, mentions[1].anchor.start);
        assert_ne!(mentions[0].rects, mentions[1].rects);
        for mention in mentions {
            assert_eq!(slice(&index, &mention.anchor), "[1]");
            assert_eq!(
                slice(&index, mention.sentence_anchor.as_ref().unwrap()),
                body
            );
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
        index.tokens[1].rects[0] = TextRect {
            x_min: before.x_max + 1.0,
            x_max: before.x_max + 5.0,
            y_min: before.y_min - 3.0,
            y_max: before.y_max - 5.0,
        };
        let result = extract(&index);
        assert_eq!(result.entries[0].mentions.len(), 1);
        assert!(result.entries[1].mentions.is_empty());
        assert_eq!(slice(&index, &result.entries[0].mentions[0].anchor), "1");
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
        assert_eq!(result.entries[0].mentions.len(), 2);
        assert!(
            result.entries[0]
                .mentions
                .iter()
                .all(|mention| mention.anchor.page == 2)
        );
    }

    #[test]
    fn should_withhold_links_when_no_bibliography_or_a_range_is_unsupported() {
        assert!(
            extract(&fixture(&[
                ("See [1], (2) and Smith (2020).", "body", 1),
                ("1. A list step.", "body", 1)
            ]))
            .entries
            .is_empty()
        );
        let result = extract(&fixture(&[
            ("See [1–1000000] and [9–1].", "body", 1),
            ("References", "heading", 2),
            ("[1] Smith. Work.", "body", 2),
        ]));
        assert!(result.entries[0].mentions.is_empty());
        assert!(
            result
                .unresolved
                .iter()
                .all(|item| item.reason == UnresolvedReason::UnsupportedRange)
        );
        assert_eq!(result.unresolved.len(), 2);
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
        assert!(first.text.contains("A long\ntitle"));
        assert_eq!(
            first
                .member_anchors
                .iter()
                .map(|anchor| anchor.page)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(fields(first).title.value.as_deref(), Some("A long title"));
        assert_eq!(
            fields(first).venue.value.as_deref(),
            Some("Journal of Results")
        );
        assert_eq!(fields(first).doi.value.as_deref(), Some("10.1000/abc"));
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
    fn should_reject_invalid_utf16_boundaries_when_a_paragraph_splits_a_surrogate_pair() {
        let mut index = fixture(&[
            ("😀 References", "heading", 1),
            ("[1] Smith. Title.", "body", 1),
        ]);
        index.objects.paragraph[0].start = 1;
        assert!(extract(&index).entries.is_empty());
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
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
        assert_eq!(
            slice(&index, &result.entries[0].mentions[0].anchor),
            "van der Waals (1910)"
        );
        assert_eq!(citation_family("John Smith"), None);
    }

    #[test]
    fn should_keep_sentence_context_when_an_abbreviation_crosses_segmenter_boundaries() {
        let body = "Smith et al. 2020a supports this observation.";
        let mut index = fixture(&[
            (body, "body", 1),
            ("References", "heading", 2),
            ("Smith, A. (2020a). A study.", "body", 2),
        ]);
        index.objects.sentence = vec![
            TextRange { start: 0, end: 12 },
            TextRange {
                start: 13,
                end: body.len(),
            },
        ];
        let result = extract(&index);
        let mention = &result.entries[0].mentions[0];
        assert_eq!(slice(&index, &mention.anchor), "Smith et al. 2020a");
        assert_eq!(
            slice(&index, mention.sentence_anchor.as_ref().unwrap()),
            body
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
        let first = index.objects.paragraph[1].clone();
        let continuation = index.objects.paragraph.pop().unwrap();
        index.objects.paragraph[1].end = continuation.end;
        index.objects.paragraph[1].spans = vec![
            TextRange {
                start: first.start,
                end: first.end,
            },
            TextRange {
                start: continuation.start,
                end: continuation.end,
            },
        ];
        let result = extract(&index);
        assert_eq!(result.entries.len(), 1);
        assert!(!result.entries[0].text.contains("Floating"));
        assert_eq!(
            fields(&result.entries[0]).title.value.as_deref(),
            Some("A long title")
        );
        assert_eq!(result.entries[0].member_anchors.len(), 2);
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
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
        let first = &result.entries[0];
        assert_eq!(
            first
                .member_anchors
                .iter()
                .map(|anchor| anchor.page)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(fields(first).title.value.as_deref(), Some("A long title"));
        assert_eq!(fields(first).doi.value.as_deref(), Some("10.1000/ABC"));
        assert_eq!(
            fields(&result.entries[1]).title.value.as_deref(),
            Some("Next title")
        );
        assert_eq!(
            fields(&result.entries[1]).year.value.as_deref(),
            Some("2021")
        );
    }

    #[test]
    fn should_resolve_each_printed_marker_once_when_keys_resemble_author_year_citations() {
        let index = fixture(&[
            ("See [Smith2020] and [Smith 2020].", "body", 1),
            ("References", "heading", 2),
            ("[Smith2020] Smith, A. (2020). A study.", "body", 2),
        ]);
        let result = extract(&index);
        let mentions = &result.entries[0].mentions;
        assert_eq!(mentions.len(), 2);
        assert_eq!(slice(&index, &mentions[0].anchor), "[Smith2020]");
        assert_eq!(slice(&index, &mentions[1].anchor), "Smith 2020");
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
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
        let first = &result.entries[0];
        let second = &result.entries[1];
        assert_eq!(fields(first).title.value.as_deref(), Some("A long title"));
        assert_eq!(fields(second).title.value.as_deref(), Some("Next title"));
        assert_eq!(fields(second).year.value.as_deref(), Some("2021"));
        assert_eq!(first.member_anchors.len(), 2);
        assert!(first.anchor.end <= second.anchor.start);
        assert_eq!(slice(&index, &first.mentions[0].anchor), "Smith (2020)");
        assert_eq!(slice(&index, &second.mentions[0].anchor), "Jones (2021)");
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
        let first = &result.entries[0];
        assert_eq!(first.member_anchors.len(), 3);
        assert_eq!(
            fields(first).authors.value.as_deref(),
            Some(["Example, A.".to_owned(), "Sample, B.".to_owned()].as_slice())
        );
        assert_eq!(
            fields(first).title.value.as_deref(),
            Some("A long comparative title")
        );
        assert_eq!(
            fields(first).venue.value.as_deref(),
            Some("Journal of Results")
        );
        assert_eq!(fields(first).year.value.as_deref(), Some("2020"));
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
        assert_eq!(
            fields(&result.entries[0]).printed_key.value.as_deref(),
            Some("1")
        );
        assert_eq!(
            fields(&result.entries[1]).printed_key.value.as_deref(),
            Some("2")
        );
        assert_eq!(result.entries[1].anchor.start, boundary);
        assert_eq!(
            slice(&index, &result.entries[0].member_anchors[0]).trim(),
            raw[..raw.find("[2] Sample").unwrap()].trim()
        );
        assert!(result.entries[0].text.contains("citing [9]"));
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
        assert!(result.unresolved.is_empty());
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
            assert_eq!(
                fields(&result.entries[0]).printed_key.value.as_deref(),
                Some("1")
            );
            assert!(result.entries[0].text.contains("[2] as inline notation"));
            assert_eq!(result.entries[0].mentions.len(), 1);
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
        let mut third = index.pages[1].clone();
        third.number = 3;
        third.start = boundary;
        index.pages[1].end = boundary;
        index.pages.push(third);
        let result = extract(&index);
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[1].page, 3);
        assert_eq!(result.entries[1].anchor.start, boundary);
        assert!(result.entries[1].region.is_none());
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
        assert_eq!(
            fields(&result.entries[0]).title.value.as_deref(),
            Some("A complete title")
        );
        assert_eq!(
            fields(&result.entries[0]).year.value.as_deref(),
            Some("2021")
        );
        assert_eq!(
            fields(&result.entries[1]).year.value.as_deref(),
            Some("2022")
        );
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
        assert_eq!(
            fields(&result.entries[1]).printed_key.value.as_deref(),
            Some("1800")
        );
        assert_eq!(
            fields(&result.entries[2]).printed_key.value.as_deref(),
            Some("2021")
        );
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
                index.tokens[at].rects[0] = TextRect {
                    x_min: before.x_max + 1.0,
                    x_max: before.x_max + 5.0,
                    y_min: before.y_min - 3.0,
                    y_max: before.y_max - 5.0,
                };
            }
        }
        let result = extract(&index);
        assert_eq!(result.entries[0].mentions.len(), 3);
        assert_eq!(result.entries[1].mentions.len(), 1);
        assert_eq!(slice(&index, &result.entries[1].mentions[0].anchor), "(8)");
        assert_eq!(result.unresolved.len(), 6);
        assert!(
            result
                .unresolved
                .iter()
                .all(|item| item.reason == UnresolvedReason::AmbiguousMarker
                    && item.candidate_ids.len() == 1)
        );
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
                .all(|item| slice(&index, &item.mention.anchor) == item.text)
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
        index.tokens[1].rects[0] = TextRect {
            x_min: before.x_max + 1.0,
            x_max: before.x_max + 5.0,
            y_min: before.y_min - 3.0,
            y_max: before.y_max - 5.0,
        };
        let result = extract(&index);
        assert_eq!(result.entries[0].mentions.len(), 1);
        assert_eq!(slice(&index, &result.entries[0].mentions[0].anchor), "7");
        assert!(result.unresolved.is_empty());
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
        assert_eq!(result.entries[0].member_anchors.len(), 2);
        assert!(result.entries[0].text.contains("Journal of Results"));
        assert_eq!(fields(&result.entries[1]).printed_key.value, None);
        assert_eq!(
            fields(&result.entries[1]).authors.value,
            Some(vec!["Other, B.".to_owned()])
        );
        assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
            assert_eq!(
                fields(&result.entries[0]).printed_key.value.as_deref(),
                Some("2020")
            );
            assert_eq!(
                fields(&result.entries[1]).printed_key.value.as_deref(),
                Some("2021")
            );
            assert!(result.entries.iter().all(|entry| entry.mentions.len() == 1));
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
        assert_eq!(result.entries[0].member_anchors.len(), 3);
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
            assert_eq!(result.entries[0].text, raw);
            assert_eq!(
                fields(&result.entries[0]).doi.value.as_deref(),
                expected,
                "{doi}"
            );
            assert_eq!(
                fields(&result.entries[0]).doi.confidence,
                if expected.is_some() {
                    FieldConfidence::Explicit
                } else {
                    FieldConfidence::Missing
                }
            );
            assert_eq!(result.entries[0].mentions.len(), 1);
        }
    }
}
