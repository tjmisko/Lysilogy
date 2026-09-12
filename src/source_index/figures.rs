use super::{Figure, FigureReference, ReadingIndex, ReadingToken, native::union};
use crate::domain::TextRect;

pub(super) fn find(index: &ReadingIndex) -> Vec<Figure> {
    let mut figures = Vec::<Figure>::new();
    for paragraph in &index.objects.paragraph {
        if paragraph.kind != "caption" {
            continue;
        }
        let tokens = tokens(index, paragraph.start, paragraph.end);
        let Some((label, _, _)) = label_at(&tokens, 0) else {
            continue;
        };
        let Some(first) = tokens.first() else {
            continue;
        };
        let caption_box = union(tokens.iter().flat_map(|token| token.rects.iter().copied()));
        let caption = utf16_slice(&index.text, paragraph.start, paragraph.end);
        let rect = figure_region(index, first.page, paragraph.start, caption_box);
        figures.push(Figure {
            id: format!("figure-{}", label.to_ascii_lowercase()),
            label: format!("Figure {label}"),
            page: first.page,
            caption,
            start: paragraph.start,
            end: paragraph.end,
            rect,
            confidence: "candidate".into(),
            references: Vec::new(),
        });
    }
    let all_tokens = index.tokens.iter().collect::<Vec<_>>();
    for token_number in 0..all_tokens.len() {
        let Some((label, start, end)) = label_at(&all_tokens, token_number) else {
            continue;
        };
        let token = all_tokens[token_number];
        let referenced = tokens(index, start, end);
        let mut labels = vec![label];
        if let Some(token) = referenced.last() {
            labels.extend(range_tail(&token.text));
        }
        for figure in &mut figures {
            if !labels
                .iter()
                .any(|label| figure.id == format!("figure-{}", label.to_ascii_lowercase()))
                || (figure.start <= start && start < figure.end)
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
        if ["figures", "figs"].iter().any(|prefix| {
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
                for figure in &mut figures {
                    if figure.id == format!("figure-{}", number.to_ascii_lowercase())
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
    figures
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

fn label_at(tokens: &[&ReadingToken], number: usize) -> Option<(String, usize, usize)> {
    let token = tokens.get(number)?;
    let lower = token.text.to_ascii_lowercase();
    let prefix = ["figures", "figure", "figs.", "fig.", "figs", "fig"]
        .iter()
        .find(|prefix| lower.starts_with(**prefix))?;
    let suffix = token.text.get(prefix.len()..)?.trim();
    if !suffix.is_empty() {
        return identifier(suffix).map(|label| (label, token.start, token.end));
    }
    let next = tokens.get(number + 1)?;
    if next.page != token.page || next.start.saturating_sub(token.end) > 3 {
        return None;
    }
    identifier(&next.text).map(|label| (label, token.start, next.end))
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
) -> Option<TextRect> {
    let dimensions = index
        .pages
        .iter()
        .find(|candidate| candidate.number == page)?;
    let mut top = dimensions.height * 0.05;
    for paragraph in &index.objects.paragraph {
        if paragraph.start == caption_start
            || paragraph.kind != "body"
            || paragraph.end - paragraph.start < 100
        {
            continue;
        }
        let text = tokens(index, paragraph.start, paragraph.end);
        if text.first().is_none_or(|token| token.page != page) {
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
    Some(TextRect {
        x_min: (caption.x_min - 8.0).max(0.0),
        y_min: top,
        x_max: (caption.x_max + 8.0).min(dimensions.width),
        y_max: (caption.y_max + 4.0).min(dimensions.height),
    })
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
