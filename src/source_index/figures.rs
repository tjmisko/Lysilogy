use super::{Figure, FigureReference, ReadingIndex, ReadingToken, TextRange, native::union};
use crate::domain::TextRect;

pub(super) fn find(index: &ReadingIndex) -> Vec<Figure> {
    let mut figures = Vec::<Figure>::new();
    for paragraph in &index.objects.paragraph {
        if paragraph.kind != "caption" {
            continue;
        }
        let tokens = tokens(index, paragraph.start, paragraph.end);
        let Some((kind, label, _, _)) = label_at(&tokens, 0) else {
            continue;
        };
        let Some(first) = tokens.first() else {
            continue;
        };
        let caption_box = union(tokens.iter().flat_map(|token| token.rects.iter().copied()));
        let caption = utf16_slice(&index.text, paragraph.start, paragraph.end);
        let rect = figure_region(
            index,
            first.page,
            paragraph.start,
            caption_box,
            kind == "Table",
        );
        figures.push(Figure {
            id: format!(
                "{}-{}",
                kind.to_ascii_lowercase(),
                label.to_ascii_lowercase()
            ),
            kind: kind.to_ascii_lowercase(),
            label: format!("{kind} {label}"),
            page: first.page,
            caption,
            start: paragraph.start,
            end: paragraph.end,
            spans: member_spans(index, first.page, rect.unwrap_or(caption_box)),
            rect,
            confidence: "candidate".into(),
            references: Vec::new(),
        });
    }
    attach_references(index, &mut figures);
    figures
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

fn figure_region(
    index: &ReadingIndex,
    page: u32,
    caption_start: usize,
    caption: TextRect,
    table: bool,
) -> Option<TextRect> {
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
    if table && let Some(below) = table_below(index, page, caption, body_font) {
        return Some(below);
    }
    let mut top = dimensions.height * 0.05;
    for paragraph in &index.objects.paragraph {
        if paragraph.start == caption_start
            || !matches!(paragraph.kind.as_str(), "body" | "caption")
        {
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
        if paragraph.kind == "body" && !prose_barrier(&text, body_font) {
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
    let mut bounds = TextRect {
        x_min: (caption.x_min - 8.0).max(0.0),
        y_min: top,
        x_max: (caption.x_max + 8.0).min(dimensions.width),
        y_max: (caption.y_max + 4.0).min(dimensions.height),
    };
    // Captions are often much narrower than the table rows/chart labels above
    // them. Include the associated float, while keeping adjacent captions apart.
    let captions = index
        .objects
        .paragraph
        .iter()
        .filter(|p| p.kind == "caption")
        .filter_map(|p| {
            let text = tokens(index, p.start, p.end);
            (text.first()?.page == page).then(|| {
                (
                    p.start,
                    union(text.iter().flat_map(|t| t.rects.iter().copied())),
                )
            })
        })
        .collect::<Vec<_>>();
    for paragraph in index
        .objects
        .paragraph
        .iter()
        .filter(|p| matches!(p.kind.as_str(), "float" | "body" | "list"))
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
            .filter(|(_, c)| c.y_min >= rect.y_max)
            .min_by(|(_, a), (_, b)| {
                let distance = |c: &TextRect| {
                    let dx = (c.x_min - rect.x_max).max(rect.x_min - c.x_max).max(0.0);
                    dx * dx + 4.0 * (c.y_min - rect.y_max).powi(2)
                };
                distance(a).total_cmp(&distance(b))
            });
        if nearest.is_some_and(|(start, _)| *start == caption_start) {
            bounds = union([bounds, rect].into_iter());
        }
    }
    Some(bounds)
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
fn table_below(index: &ReadingIndex, page: u32, caption: TextRect, font: f32) -> Option<TextRect> {
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
    let mut bounds = caption;
    let mut numeric = false;
    for (paragraph, text, rect) in candidates {
        if rect.y_min - bounds.y_max > font * 5.0
            || paragraph.kind == "caption"
            || prose_barrier(&text, font)
        {
            break;
        }
        let content = text
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if content.len() > 100 && !matches!(paragraph.kind.as_str(), "float" | "list") {
            break;
        }
        numeric |= content.chars().filter(char::is_ascii_digit).count() >= 2;
        bounds = union([bounds, rect].into_iter());
    }
    if !numeric || bounds.y_max - caption.y_max <= font {
        return None;
    }
    // The caption may end before the last column. Extend across the established
    // grid's rows, without absorbing a neighboring prose paragraph.
    for paragraph in &index.objects.paragraph {
        if paragraph.kind == "caption" {
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
    Some(bounds)
}
