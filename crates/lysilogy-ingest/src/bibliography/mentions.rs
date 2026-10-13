//! In-text citation marker detection and resolution against extracted entries.
//!
//! A marker resolves only when its printed key identifies exactly one entry.
//! Everything else is retained as an [`UnresolvedMention`], never guessed.

use std::collections::{BTreeMap, BTreeSet};

use lysilogy_core::CitationStyle;
use lysilogy_core::names::citation_family;
use unicode_segmentation::UnicodeSegmentation;

use super::source::{Source, clean, key, pattern, year_regex};
use super::{
    ExtractedEntry, ExtractedMention, PageRect, TextSpan, UnresolvedMention, UnresolvedReason,
};

/// Sentence contexts longer than this are clipped around the marker.
const PASSAGE_LIMIT: usize = 1500;
/// Units kept on each side of the marker when clipping.
const PASSAGE_MARGIN: usize = 600;

/// Resolve every in-text marker outside `bibliography` against `entries`.
pub(super) fn resolve_mentions(
    source: &Source<'_>,
    bibliography: TextSpan,
    entries: &[ExtractedEntry],
) -> (Vec<ExtractedMention>, Vec<UnresolvedMention>) {
    let mut printed: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut author_year: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (at, entry) in entries.iter().enumerate() {
        let fields = &entry.fields;
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
    let mut resolver = Resolver {
        source,
        bibliography,
        sentences: sentences(source),
        seen: BTreeSet::new(),
        mentions: vec![],
        unresolved: vec![],
    };
    let body = &source.source_text().text;
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
        if pattern!(r"(?i)\b(?:eq(?:uation)?s?|sec(?:tion)?s?|fig(?:ure)?s?|tables?|theorems?|lemmas?|propositions?)\.?\s*$").is_match(&prefix[prefix_start..]) {
            continue;
        }
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
    resolve_author_year(&mut resolver, &printed_occurrences, &author_year);
    (resolver.mentions, resolver.unresolved)
}

/// Author-year markers (`Smith et al., 2020`, `Jones (2021a, b)`), skipping
/// any occurrence that overlaps an already recognized printed-key marker.
fn resolve_author_year(
    resolver: &mut Resolver<'_>,
    printed_occurrences: &[TextSpan],
    author_year: &BTreeMap<String, Vec<usize>>,
) {
    let body = &resolver.source.source_text().text;
    for matched in pattern!(r"((?:(?i:van|von|de|del|della|da|dos|das|la|der|den)\s+){0,3}[\p{L}][\p{L}\p{M}'’−-]+)(?:\s+(?:et\s+al\.?|(?:and|&)\s+[\p{L}][\p{L}\p{M}'’−-]+))?(?:\s*[,(\[]\s*|\s+)((?:18|19|20)\d{2}[a-z]?(?:\s*[,;]\s*(?:(?:18|19|20)\d{2}[a-z]?|[a-z]))*)\b").captures_iter(body) {
        let whole = matched.get(0).expect("author-year citation");
        let author = matched.get(1).expect("citation author").as_str();
        let years = matched.get(2).expect("citation years").as_str();
        let end = whole.end()
            + usize::from(
                whole.as_str().contains(['(', '[']) && body[whole.end()..].starts_with([')', ']']),
            );
        let occurrence = resolver.source.range(whole.start(), end);
        let at = printed_occurrences.partition_point(|printed| printed.end <= occurrence.start);
        if printed_occurrences
            .get(at)
            .is_some_and(|printed| printed.start < occurrence.end)
        {
            continue;
        }
        let mut inherited = None;
        for part in years.split([',', ';']).map(str::trim) {
            let year = if part.len() == 1 {
                format!("{}{part}", inherited.unwrap_or_default())
            } else {
                inherited = part.get(..4);
                part.to_owned()
            };
            resolver.resolve_marker(
                occurrence,
                CitationStyle::AuthorYear,
                Ok(format!("{}:{year}", key(author))),
                author_year,
                false,
            );
        }
    }
}

fn citation_cue(source: &Source<'_>, start: usize, printed: &BTreeMap<String, Vec<usize>>) -> bool {
    let Some(byte) = source.byte(start) else {
        return false;
    };
    // A bounded clause prefix allows explicit mixed conventions, e.g.
    // "See [2] and (3)". "See (3)" alone can refer to an equation or list.
    let prefix = &source.source_text().text[..byte];
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
    resolver: &mut Resolver<'_>,
    printed: &BTreeMap<String, Vec<usize>>,
    bracket_convention: bool,
) {
    let source = resolver.source;
    for pair in source.sorted_tokens().windows(2) {
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
                TextSpan {
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

fn sentences(source: &Source<'_>) -> Vec<TextSpan> {
    let text = source.source_text();
    if !text.sentences.is_empty() {
        let mut sentences: Vec<_> = text
            .sentences
            .iter()
            .copied()
            .filter(|span| source.text(*span).is_some())
            .collect();
        sentences.sort_by_key(|span| span.start);
        return sentences;
    }
    // Sources without sentence objects use the same Unicode segmenter as the
    // reading-index builder, preserving source offsets.
    text.text
        .split_sentence_bound_indices()
        .filter(|(_, sentence)| !sentence.trim().is_empty())
        .map(|(byte, sentence)| source.range(byte, byte + sentence.len()))
        .collect()
}

/// Bound a sentence context to `PASSAGE_MARGIN` units around the marker when
/// it exceeds `PASSAGE_LIMIT`, snapping the cut points to whitespace.
pub(super) fn clip_passage(source: &Source<'_>, marker: TextSpan, passage: TextSpan) -> TextSpan {
    if passage.end - passage.start <= PASSAGE_LIMIT {
        return passage;
    }
    let text = &source.source_text().text;
    let mut start = marker
        .start
        .saturating_sub(PASSAGE_MARGIN)
        .max(passage.start);
    if start > passage.start {
        let found = (start..marker.start).find(|offset| {
            source.byte(*offset).is_some_and(|byte| {
                text[..byte]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            })
        });
        start = found.unwrap_or(marker.start);
    }
    let mut end = (marker.end + PASSAGE_MARGIN).min(passage.end);
    if end < passage.end {
        let found = (marker.end..=end).rev().find(|offset| {
            source
                .byte(*offset)
                .is_some_and(|byte| text[byte..].chars().next().is_some_and(char::is_whitespace))
        });
        end = found.unwrap_or(marker.end);
    }
    TextSpan { start, end }
}

struct Resolver<'a> {
    source: &'a Source<'a>,
    bibliography: TextSpan,
    sentences: Vec<TextSpan>,
    seen: BTreeSet<(usize, usize, String, u32)>,
    mentions: Vec<ExtractedMention>,
    unresolved: Vec<UnresolvedMention>,
}

impl Resolver<'_> {
    fn resolve_marker(
        &mut self,
        span: TextSpan,
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
            .fold(span, |context, sentence| TextSpan {
                start: context.start.min(sentence.start),
                end: context.end.max(sentence.end),
            });
        let passage = clip_passage(self.source, span, sentence);
        let passage_text = clean(self.source.text(passage).unwrap_or_default());
        let mut pages: BTreeMap<u32, Vec<PageRect>> = BTreeMap::new();
        for token in self.source.tokens(span) {
            pages
                .entry(token.page)
                .or_default()
                .extend(token.rects.iter().copied());
        }
        if pages.is_empty() {
            pages.insert(self.source.anchor(span).0, vec![]);
        }
        for (page, rects) in pages {
            if !self.seen.insert((span.start, span.end, key.clone(), page)) {
                continue;
            }
            if matches.len() == 1 && !unsupported && !ambiguous_marker {
                self.mentions.push(ExtractedMention {
                    entry_ordinal: matches[0],
                    style,
                    marker: span,
                    marker_text: self.source.text(span).unwrap_or_default().to_owned(),
                    page,
                    rects,
                    passage,
                    passage_text: passage_text.clone(),
                });
                continue;
            }
            let reason = if ambiguous_marker {
                UnresolvedReason::AmbiguousMarker
            } else if unsupported {
                UnresolvedReason::UnsupportedRange
            } else if matches.is_empty() {
                UnresolvedReason::MissingTarget
            } else {
                UnresolvedReason::AmbiguousTarget
            };
            self.unresolved.push(UnresolvedMention {
                text: self.source.text(span).unwrap_or_default().to_owned(),
                key: key.clone(),
                style,
                reason,
                candidate_ordinals: matches.to_vec(),
                marker: span,
                page,
            });
        }
    }
}
