use super::{Figure, FigureReference, ReadingIndex, ReadingToken, TextRange, native::union};
use crate::domain::TextRect;

/// Changes to derived caption/region behavior invalidate objects, not native anchors.
pub const DETECTOR_VERSION: u16 = 2;

struct Caption<'a> {
    paragraph: &'a super::Paragraph,
    kind: &'static str,
    label: String,
    page: u32,
    rect: TextRect,
}

pub fn find(index: &ReadingIndex) -> Vec<Figure> {
    let mut figures = Vec::<Figure>::new();
    let captions = captions(index);
    for candidate in &captions {
        let paragraph = candidate.paragraph;
        let kind = candidate.kind;
        let label = &candidate.label;
        let caption = utf16_slice(&index.text, paragraph.start, paragraph.end);
        let rect = figure_region(index, candidate, &captions);
        figures.push(Figure {
            id: format!(
                "{}-{}",
                kind.to_ascii_lowercase(),
                label.to_ascii_lowercase()
            ),
            kind: kind.to_ascii_lowercase(),
            label: format!("{kind} {label}"),
            page: candidate.page,
            caption,
            start: paragraph.start,
            end: paragraph.end,
            spans: rect.map_or_else(Vec::new, |r| member_spans(index, candidate.page, r)),
            rect,
            confidence: "candidate".into(),
            references: Vec::new(),
        });
    }
    attach_references(index, &mut figures);
    figures
}

fn captions(index: &ReadingIndex) -> Vec<Caption<'_>> {
    index
        .objects
        .paragraph
        .iter()
        .filter_map(|paragraph| {
            let text = tokens(index, paragraph.start, paragraph.end);
            let (kind, label, _, label_end) = label_at(&text, 0)?;
            let first = text.first()?;
            let printed_label = utf16_slice(&index.text, paragraph.start, label_end);
            // Recover body/footnote-classified captions only with an explicit printed
            // label separator. A sentence merely starting "Figure 2 shows" is prose.
            if paragraph.kind != "caption" && !printed_label.ends_with([':', '.']) {
                return None;
            }
            let rect = union(text.iter().flat_map(|token| token.rects.iter().copied()));
            if continues_prose(index, paragraph, &text, rect) {
                return None;
            }
            Some(Caption {
                paragraph,
                kind,
                label,
                page: first.page,
                rect,
            })
        })
        .collect()
}

fn continues_prose(
    index: &ReadingIndex,
    paragraph: &super::Paragraph,
    text: &[&ReadingToken],
    caption: TextRect,
) -> bool {
    let Some(previous) = index
        .objects
        .paragraph
        .iter()
        .rev()
        .find(|p| p.end <= paragraph.start)
    else {
        return false;
    };
    if paragraph.start.saturating_sub(previous.end) > 4 {
        return false;
    }
    let preceding = tokens(index, previous.start, previous.end);
    let (Some(last), Some(first)) = (preceding.last(), text.first()) else {
        return false;
    };
    if last.page != first.page || last.text.ends_with(['.', '!', '?', ':', ';']) {
        return false;
    }
    let last_rect = union(last.rects.iter().copied());
    let first_rect = union(first.rects.iter().copied());
    let font = first_rect.y_max - first_rect.y_min;
    let prior = union(preceding.iter().flat_map(|t| t.rects.iter().copied()));
    font > 0.0
        && prose_barrier(&preceding, font)
        && caption.y_min >= last_rect.y_max
        && caption.y_min - last_rect.y_max < font * 0.6
        && (prior.x_min - caption.x_min).abs() < font
        && (last_rect.y_max - last_rect.y_min - font).abs() < font * 0.15
}

fn attach_references(index: &ReadingIndex, figures: &mut [Figure]) {
    let all_tokens = index.tokens.iter().collect::<Vec<_>>();
    for token_number in 0..all_tokens.len() {
        let Some((kind, label, start, end)) = label_at(&all_tokens, token_number) else {
            continue;
        };
        let token = all_tokens[token_number];
        let referenced = tokens(index, start, end);
        let mut labels = vec![label];
        if let Some(token) = referenced.last() {
            labels.extend(range_tail(&token.text));
        }
        for figure in figures.iter_mut() {
            if !labels.iter().any(|label| {
                figure.id
                    == format!(
                        "{}-{}",
                        kind.to_ascii_lowercase(),
                        label.to_ascii_lowercase()
                    )
            }) || (figure.start <= start && start < figure.end)
            {
                continue;
            }
            figure.references.push(FigureReference {
                page: token.page,
                start,
                end,
                rects: referenced
                    .iter()
                    .flat_map(|token| token.rects.iter().copied())
                    .collect(),
            });
        }
        // Plural references: "Figures 1, 2 and 3" and bounded "Figs. 1–3".
        if ["figures", "figs", "tables"].iter().any(|prefix| {
            all_tokens[token_number]
                .text
                .to_ascii_lowercase()
                .starts_with(prefix)
        }) {
            let next = all_tokens.iter().skip(token_number + 2).take(8);
            for token in next {
                if matches!(token.text.as_str(), "and" | "&" | ",") {
                    continue;
                }
                let Some(number) = identifier(&token.text) else {
                    break;
                };
                for figure in figures.iter_mut() {
                    if figure.id
                        == format!(
                            "{}-{}",
                            kind.to_ascii_lowercase(),
                            number.to_ascii_lowercase()
                        )
                        && !(figure.start <= start && start < figure.end)
                    {
                        figure.references.push(FigureReference {
                            page: token.page,
                            start: token.start,
                            end: token.end,
                            rects: token.rects.clone(),
                        });
                    }
                }
            }
        }
    }
}

fn range_tail(text: &str) -> Vec<String> {
    let Some((start, end)) = text.split_once(['–', '-']) else {
        return Vec::new();
    };
    let start = start
        .trim_matches(|ch: char| !ch.is_ascii_digit())
        .parse::<u32>();
    let end = end
        .trim_matches(|ch: char| !ch.is_ascii_digit())
        .parse::<u32>();
    let (Ok(start), Ok(end)) = (start, end) else {
        return Vec::new();
    };
    if end <= start || end - start > 20 {
        return Vec::new();
    }
    let prefix = if text.starts_with(['S', 's']) {
        "S"
    } else {
        ""
    };
    (start + 1..=end)
        .map(|number| format!("{prefix}{number}"))
        .collect()
}

fn tokens(index: &ReadingIndex, start: usize, end: usize) -> Vec<&ReadingToken> {
    let first = index.tokens.partition_point(|token| token.end <= start);
    index.tokens[first..]
        .iter()
        .take_while(|token| token.start < end)
        .collect()
}

fn label_at(
    tokens: &[&ReadingToken],
    number: usize,
) -> Option<(&'static str, String, usize, usize)> {
    let token = tokens.get(number)?;
    let lower = token.text.to_ascii_lowercase();
    let kind = if lower.starts_with("tab") {
        "Table"
    } else {
        "Figure"
    };
    let prefix = [
        "figures", "figure", "figs.", "fig.", "figs", "fig", "tables", "table", "tab.",
    ]
    .iter()
    .find(|prefix| lower.starts_with(**prefix))?;
    let suffix = token.text.get(prefix.len()..)?.trim();
    if !suffix.is_empty() {
        return identifier(suffix.split_whitespace().next()?)
            .map(|label| (kind, label, token.start, token.end));
    }
    let next = tokens.get(number + 1)?;
    if next.page != token.page || next.start.saturating_sub(token.end) > 3 {
        return None;
    }
    identifier(&next.text).map(|label| (kind, label, token.start, next.end))
}

fn identifier(text: &str) -> Option<String> {
    let text =
        text.trim_matches(|ch: char| matches!(ch, '(' | ')' | '[' | ']' | ',' | ':' | '.' | ';'));
    let roman = text.to_ascii_uppercase();
    if canonical_roman(&roman) {
        return Some(roman);
    }
    let prefix = text
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, 'S' | 's'))
        .collect::<String>();
    if !prefix.chars().any(|ch| ch.is_ascii_digit()) || prefix.len() > 6 {
        return None;
    }
    let suffix = &text[prefix.len()..];
    if !suffix.is_empty() && !suffix.starts_with(['(', 'a', 'b', 'c', 'd', '–', '-']) {
        return None;
    }
    Some(prefix.to_ascii_uppercase())
}

fn canonical_roman(text: &str) -> bool {
    if text.is_empty() || text.len() > 15 {
        return false;
    }
    let mut remaining = text;
    for (one, five, ten) in [
        ('M', None, None),
        ('C', Some('D'), Some('M')),
        ('X', Some('L'), Some('C')),
        ('I', Some('V'), Some('X')),
    ] {
        if let (Some(five), Some(ten)) = (five, ten) {
            let low = format!("{one}{five}");
            let high = format!("{one}{ten}");
            if let Some(rest) = remaining
                .strip_prefix(&low)
                .or_else(|| remaining.strip_prefix(&high))
            {
                remaining = rest;
                continue;
            }
            remaining = remaining.strip_prefix(five).unwrap_or(remaining);
        }
        for _ in 0..3 {
            remaining = remaining.strip_prefix(one).unwrap_or(remaining);
        }
    }
    remaining.is_empty()
}

fn figure_region(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
) -> Option<TextRect> {
    let page = candidate.page;
    let caption_start = candidate.paragraph.start;
    let caption = candidate.rect;
    let dimensions = index
        .pages
        .iter()
        .find(|candidate| candidate.number == page)?;
    let mut fonts = index
        .tokens
        .iter()
        .filter(|t| t.page == page)
        .flat_map(|t| t.rects.iter().map(|r| r.y_max - r.y_min))
        .filter(|h| *h > 0.0)
        .collect::<Vec<_>>();
    fonts.sort_by(f32::total_cmp);
    let body_font = fonts.get(fonts.len() / 2).copied().unwrap_or(10.0);
    if candidate.kind == "Table"
        && let Some(below) = table_below(index, candidate, captions, body_font)
    {
        return Some(below);
    }
    let mut top = dimensions.height * 0.05;
    for paragraph in &index.objects.paragraph {
        if paragraph.start == caption_start {
            continue;
        }
        let text = super::paragraphs::pieces(paragraph)
            .iter()
            .flat_map(|span| tokens(index, span.start, span.end))
            .filter(|token| token.page == page)
            .collect::<Vec<_>>();
        if text.is_empty() {
            continue;
        }
        if !captions
            .iter()
            .any(|c| c.paragraph.start == paragraph.start)
            && !prose_barrier(&text, body_font)
        {
            continue;
        }
        let rect = union(text.iter().flat_map(|token| token.rects.iter().copied()));
        let overlap = rect.x_max.min(caption.x_max) - rect.x_min.max(caption.x_min);
        if rect.y_max < caption.y_min && overlap > (caption.x_max - caption.x_min) * 0.35 {
            top = top.max(rect.y_max + 4.0);
        }
    }
    let height = caption.y_min - top;
    if height < 35.0 || height > dimensions.height * 0.70 {
        return None;
    }
    let mut bounds = None;
    // Printed diagram labels/ticks establish an observed extent. Do not fill
    // whitespace back to a page margin or include the separate caption body.
    for paragraph in index
        .objects
        .paragraph
        .iter()
        .filter(|p| !captions.iter().any(|c| c.paragraph.start == p.start))
    {
        let text = tokens(index, paragraph.start, paragraph.end);
        if text.first().is_none_or(|t| t.page != page) || prose_barrier(&text, body_font) {
            continue;
        }
        let rect = union(text.iter().flat_map(|t| t.rects.iter().copied()));
        if rect.y_min < top || rect.y_max > caption.y_min {
            continue;
        }
        let nearest = captions
            .iter()
            .filter(|c| {
                c.page == page
                    && c.rect.y_min >= rect.y_max
                    && c.rect.x_max > rect.x_min
                    && c.rect.x_min < rect.x_max
            })
            .min_by(|a, b| {
                let distance = |c: &TextRect| {
                    let dx = (c.x_min - rect.x_max).max(rect.x_min - c.x_max).max(0.0);
                    dx * dx + 4.0 * (c.y_min - rect.y_max).powi(2)
                };
                distance(&a.rect).total_cmp(&distance(&b.rect))
            });
        if nearest.is_some_and(|c| c.paragraph.start == caption_start) {
            bounds = Some(bounds.map_or(rect, |b| union([b, rect].into_iter())));
        }
    }
    bounds.map(|r| padded(r, body_font * 0.4, dimensions.width, top, caption.y_min))
}

fn padded(rect: TextRect, margin: f32, width: f32, top: f32, bottom: f32) -> TextRect {
    TextRect {
        x_min: (rect.x_min - margin).max(0.0),
        y_min: (rect.y_min - margin).max(top),
        x_max: (rect.x_max + margin).min(width),
        y_max: (rect.y_max + margin).min(bottom),
    }
}

pub(super) fn utf16_slice(text: &str, start: usize, end: usize) -> String {
    String::from_utf16_lossy(
        &text
            .encode_utf16()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect::<Vec<_>>(),
    )
}

// A long diagram label can exceed a character threshold without being prose.
// Require consecutive printed lines at body size before it bounds a figure.
fn prose_barrier(text: &[&ReadingToken], body_font: f32) -> bool {
    let content = text
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if content.len() < 80 || content.to_ascii_lowercase().starts_with("step ") {
        return false;
    }
    let mut rows = Vec::<TextRect>::new();
    for rect in text.iter().flat_map(|t| t.rects.iter().copied()) {
        if rect.y_max - rect.y_min < body_font * 0.90 {
            continue;
        }
        if let Some(row) = rows
            .iter_mut()
            .find(|row| (row.y_min - rect.y_min).abs() < body_font * 0.45)
        {
            *row = union([*row, rect].into_iter());
        } else {
            rows.push(rect);
        }
    }
    rows.sort_by(|a, b| a.y_min.total_cmp(&b.y_min));
    rows.windows(2).any(|pair| {
        pair[1].y_min - pair[0].y_min < body_font * 2.2
            && (pair[0].x_min - pair[1].x_min).abs() < body_font * 2.5
    })
}

fn member_spans(index: &ReadingIndex, page: u32, rect: TextRect) -> Vec<TextRange> {
    let mut spans = Vec::<TextRange>::new();
    for token in index.tokens.iter().filter(|token| {
        token.page == page
            && token.rects.iter().all(|r| {
                r.x_min >= rect.x_min - 1.0
                    && r.x_max <= rect.x_max + 1.0
                    && r.y_min >= rect.y_min - 1.0
                    && r.y_max <= rect.y_max + 1.0
            })
    }) {
        if let Some(last) = spans.last_mut()
            && utf16_slice(&index.text, last.end, token.start)
                .chars()
                .all(char::is_whitespace)
        {
            last.end = token.end;
        } else {
            spans.push(TextRange {
                start: token.start,
                end: token.end,
            });
        }
    }
    spans
}

// Tables commonly put their captions above the grid, unlike figure captions.
fn table_below(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    font: f32,
) -> Option<TextRect> {
    let page = candidate.page;
    let caption = candidate.rect;
    let dimensions = index.pages.iter().find(|p| p.number == page)?;
    let mut candidates = index
        .objects
        .paragraph
        .iter()
        .filter_map(|p| {
            let text = tokens(index, p.start, p.end);
            if text.first()?.page != page {
                return None;
            }
            let rect = union(text.iter().flat_map(|t| t.rects.iter().copied()));
            (rect.y_min >= caption.y_max
                && rect.x_max > caption.x_min
                && rect.x_min < caption.x_max)
                .then_some((p, text, rect))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.2.y_min.total_cmp(&b.2.y_min));
    let mut bounds = None;
    let mut numeric = false;
    for (paragraph, text, rect) in candidates {
        if rect.y_min - bounds.map_or(caption.y_max, |r: TextRect| r.y_max) > font * 5.0
            || captions
                .iter()
                .any(|c| c.paragraph.start == paragraph.start)
            || prose_barrier(&text, font)
        {
            break;
        }
        let content = text
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        numeric |= content.chars().filter(char::is_ascii_digit).count() >= 2;
        bounds = Some(bounds.map_or(rect, |r| union([r, rect].into_iter())));
    }
    let mut bounds = bounds?;
    if !numeric || bounds.y_max - bounds.y_min <= font {
        return None;
    }
    // The caption may end before the last column. Extend across the established
    // grid's rows, without absorbing a neighboring prose paragraph.
    for paragraph in &index.objects.paragraph {
        if captions
            .iter()
            .any(|c| c.paragraph.start == paragraph.start)
        {
            continue;
        }
        let text = tokens(index, paragraph.start, paragraph.end);
        if text.first().is_none_or(|t| t.page != page) || prose_barrier(&text, font) {
            continue;
        }
        let rect = union(text.iter().flat_map(|t| t.rects.iter().copied()));
        let cell_band = caption.y_max..=bounds.y_max;
        if cell_band.contains(&rect.y_min) && cell_band.contains(&rect.y_max) {
            bounds = union([bounds, rect].into_iter());
        }
    }
    Some(padded(
        bounds,
        font * 0.4,
        dimensions.width,
        caption.y_max,
        dimensions.height,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::{Paragraph, Provenance, ReadingPage, TextObjects};

    // Independently specified native-coordinate fixtures; no paper artifacts.
    fn fixture(lines: &[(&str, &str, f32, f32)]) -> ReadingIndex {
        let mut index = ReadingIndex {
            schema_version: super::super::SCHEMA_VERSION,
            text: String::new(),
            pages: vec![],
            tokens: vec![],
            objects: TextObjects::default(),
            figures: vec![],
            gaps: vec![],
        };
        for &(text, kind, x, y) in lines {
            if !index.text.is_empty() {
                index.text.push_str("\n\n");
            }
            let start = index.text.encode_utf16().count();
            let mut cursor = start;
            let mut left = x;
            for word in text.split_whitespace() {
                let end = cursor + word.encode_utf16().count();
                let width = u16::try_from(word.len()).map_or(100.0, |n| f32::from(n) * 4.0);
                index.tokens.push(ReadingToken {
                    start: cursor,
                    end,
                    page: 1,
                    text: word.into(),
                    rects: vec![TextRect {
                        x_min: left,
                        x_max: left + width,
                        y_min: y,
                        y_max: y + 10.0,
                    }],
                    provenance: Provenance::Native,
                });
                cursor = end + 1;
                left += width + 4.0;
            }
            index.text.push_str(text);
            index.objects.paragraph.push(Paragraph {
                start,
                end: index.text.encode_utf16().count(),
                kind: kind.into(),
                spans: vec![],
            });
        }
        index.pages.push(ReadingPage {
            number: 1,
            width: 600.0,
            height: 800.0,
            start: 0,
            end: index.text.encode_utf16().count(),
            provenance: Provenance::Native,
            confidence: None,
        });
        index
    }

    #[test]
    fn should_recover_literal_roman_captions_when_native_paragraphs_are_body_or_footnote() {
        for (number, kind) in [
            ("I", "body"),
            ("ii", "footnote"),
            ("III", "body"),
            ("IV", "body"),
            ("V", "body"),
            ("XXIX", "body"),
        ] {
            let caption = format!("TABLE {number}: Comparison results.");
            let index = fixture(&[(caption.as_str(), kind, 50.0, 100.0)]);
            let result = find(&index);
            assert_eq!(result.len(), 1);
            assert_eq!(
                result[0].label,
                format!("Table {}", number.to_ascii_uppercase())
            );
            assert_ne!(result[0].label, "Table 1");
        }
        for invalid in ["IIII", "IC", "VX", "MIXED", "MMMM", "", "IIIIIIIIIIIIIIII"] {
            assert!(!canonical_roman(invalid), "{invalid}");
        }
    }

    #[test]
    fn should_reject_a_false_caption_when_a_prose_line_wrap_ends_with_a_reference() {
        let mut index = fixture(&[
            (
                "The following discussion explains the independent comparison of several methods",
                "body",
                50.0,
                100.0,
            ),
            (
                "and describes the corresponding visual result as shown in",
                "body",
                50.0,
                112.0,
            ),
            (
                "Fig. 2. This image illustrates the observed result.",
                "caption",
                50.0,
                124.0,
            ),
            (
                "Fig. 2: Independently placed diagram caption.",
                "caption",
                320.0,
                300.0,
            ),
        ]);
        let end = index.objects.paragraph[1].end;
        index.objects.paragraph[0].end = end;
        index.objects.paragraph.remove(1);
        let result = find(&index);
        assert_eq!(result.len(), 1);
        assert!(result[0].caption.contains("Independently placed"));
        assert_eq!(result[0].references.len(), 1);
        assert_eq!(index.objects.paragraph[1].kind, "caption");
    }

    #[test]
    fn should_exclude_page_whitespace_and_caption_when_plot_labels_bound_a_diagram() {
        let index = fixture(&[
            ("Upper label", "float", 100.0, 150.0),
            ("Axis labels", "float", 100.0, 240.0),
            (
                "Figure 4: A separate long explanatory caption.",
                "caption",
                50.0,
                280.0,
            ),
        ]);
        let result = find(&index);
        let region = result[0].rect.unwrap();
        assert!(region.x_min > 90.0 && region.y_min > 140.0 && region.y_max < 260.0);
        assert!(
            result[0]
                .spans
                .iter()
                .all(|s| s.end <= index.objects.paragraph[1].end)
        );
    }

    #[test]
    fn should_withhold_region_when_caption_has_no_observed_diagram_labels() {
        let index = fixture(&[("Figure 4: A caption alone.", "caption", 50.0, 280.0)]);
        let result = find(&index);
        assert!(result[0].rect.is_none());
        assert!(result[0].spans.is_empty());
    }

    #[test]
    fn should_use_cell_bounds_when_table_caption_is_wider_than_the_grid() {
        let index = fixture(&[
            (
                "Table IX: A deliberately wide explanatory table caption.",
                "body",
                50.0,
                100.0,
            ),
            ("Method Score", "float", 100.0, 130.0),
            ("First 12.5", "footnote", 100.0, 146.0),
            ("Second 25.0", "footnote", 100.0, 162.0),
        ]);
        let result = find(&index);
        let rect = result[0].rect.unwrap();
        assert!(
            rect.x_min > 90.0 && rect.x_max < 200.0 && rect.y_min > 120.0 && rect.y_max < 180.0
        );
    }

    #[test]
    fn should_keep_separate_columns_when_neighboring_diagrams_share_a_vertical_band() {
        let index = fixture(&[
            ("Left plot", "float", 70.0, 150.0),
            ("Right plot", "float", 360.0, 150.0),
            ("Figure 1: Left result.", "caption", 50.0, 200.0),
            ("Figure 2: Right result.", "caption", 340.0, 200.0),
        ]);
        let result = find(&index);
        assert_eq!(result.len(), 2);
        assert!(result[0].rect.unwrap().x_max < 200.0);
        assert!(result[1].rect.unwrap().x_min > 300.0);
    }
}
