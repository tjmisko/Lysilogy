//! Reference-list location, entry splitting, and entry region extraction.

use std::collections::BTreeSet;

use lysilogy_core::names::looks_like_person_name;

use super::{
    ExtractedEntry, PageRect, SourceToken, TextSpan,
    fields::{author_year_prefix, parse_fields},
    source::{Source, label_regex, pattern, year_regex},
};

#[derive(Clone)]
struct Block {
    span: TextSpan,
    heading: bool,
    caption: bool,
}

#[derive(Default)]
struct Entry {
    spans: Vec<TextSpan>,
    key: Option<String>,
}

/// Locate the reference list and split it into entries; no I/O.
pub(super) fn extract_entries(source: &Source<'_>) -> Vec<ExtractedEntry> {
    let mut blocks: Vec<_> = source
        .source_text()
        .paragraphs
        .iter()
        .flat_map(|paragraph| {
            let spans = if paragraph.spans.is_empty() {
                vec![TextSpan {
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
    let Some(heading) = blocks.iter().position(|block| {
        pattern!(r"(?i)^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:references(?: and notes| cited)?|bibliography|literature cited|works cited)\s*[:.]?$")
            .is_match(source.text(block.span).unwrap_or_default().trim())
    }) else {
        return vec![];
    };
    let end = blocks[heading + 1..]
        .iter()
        .find(|block| {
            block.heading
                && pattern!(r"(?i)^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:appendix|appendices|supplement|acknowledg)")
                    .is_match(source.text(block.span).unwrap_or_default().trim())
        })
        .map_or_else(|| source.len_utf16(), |block| block.span.start);
    split_entries(
        source,
        blocks[heading + 1..]
            .iter()
            .filter(|block| block.span.start < end && !block.caption),
    )
    .into_iter()
    .enumerate()
    .map(|(ordinal, entry)| extracted_entry(source, entry, ordinal))
    .collect()
}

struct EntryLabel {
    start: usize,
    key: String,
    dotted: bool,
    publication_year: bool,
}

fn entry_labels(source: &Source<'_>, span: TextSpan) -> Vec<EntryLabel> {
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

fn starts_new_line(previous: &SourceToken, current: &SourceToken) -> bool {
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
                    && author.is_some_and(|author| looks_like_person_name(author));
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

fn extracted_entry(source: &Source<'_>, entry: Entry, ordinal: usize) -> ExtractedEntry {
    let raw_text = entry_text(source, &entry);
    let span = TextSpan {
        start: entry.spans[0].start,
        end: entry.spans.last().expect("entry member").end,
    };
    let (page, span) = source.anchor(span);
    let region = union(
        entry
            .spans
            .iter()
            .flat_map(|span| source.tokens(*span))
            .filter(|token| token.page == page)
            .flat_map(|token| token.rects.iter().copied()),
    );
    let fields = parse_fields(&raw_text, entry.key);
    ExtractedEntry {
        ordinal,
        raw_text,
        fields,
        page,
        span,
        member_spans: entry.spans,
        region,
    }
}

fn union(rects: impl Iterator<Item = PageRect>) -> Option<PageRect> {
    rects
        .filter(|rect| {
            [rect.x_min, rect.x_max, rect.y_min, rect.y_max]
                .iter()
                .all(|value| value.is_finite())
                && rect.x_max > rect.x_min
                && rect.y_max > rect.y_min
        })
        .reduce(|a, b| PageRect {
            x_min: a.x_min.min(b.x_min),
            y_min: a.y_min.min(b.y_min),
            x_max: a.x_max.max(b.x_max),
            y_max: a.y_max.max(b.y_max),
        })
}
