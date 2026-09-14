use super::{Figure, FigureReference, ReadingIndex, ReadingToken, TextRange, native::union};
use crate::domain::TextRect;

/// Changes to derived caption/region behavior invalidate objects, not native anchors.
pub const DETECTOR_VERSION: u16 = 4;

struct Caption<'a> {
    paragraph: &'a super::Paragraph,
    kind: &'static str,
    label: String,
    page: u32,
    rect: TextRect,
}

#[must_use]
pub fn find(index: &ReadingIndex) -> Vec<Figure> {
    find_with_images(index, &[])
}

pub fn find_with_images(
    index: &ReadingIndex,
    pages: &[super::graphics::PageGraphics],
) -> Vec<Figure> {
    let mut figures = Vec::<Figure>::new();
    let captions = captions(index);
    for candidate in &captions {
        let paragraph = candidate.paragraph;
        let kind = candidate.kind;
        let label = &candidate.label;
        let caption = utf16_slice(&index.text, paragraph.start, paragraph.end);
        let images = pages
            .iter()
            .find(|page| page.page == candidate.page)
            .map_or(&[][..], |page| page.images.as_slice());
        let rect = figure_region(index, candidate, &captions, images);
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
            let pieces = super::paragraphs::pieces(paragraph);
            if pieces.len() != 1
                || pieces[0].start != paragraph.start
                || pieces[0].end != paragraph.end
            {
                // Figure caption anchors are contiguous and page-local. Do not
                // turn a logical paragraph's bounding span into authored text.
                return None;
            }
            let text = tokens(index, paragraph.start, paragraph.end);
            let (kind, label, _, label_end) = label_at(&text, 0)?;
            let first = text.first()?;
            if text.iter().any(|token| token.page != first.page) {
                return None;
            }
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
    let Some(first) = text.first() else {
        return false;
    };
    let preceding = paragraph_tokens(index, previous, first.page);
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
    let short_reference = preceding.len() >= 6
        && matches!(last.text.to_ascii_lowercase().as_str(), "in" | "see")
        && preceding.iter().all(|token| {
            token
                .rects
                .iter()
                .all(|rect| (rect.y_min - prior.y_min).abs() < font * 0.4)
        });
    font > 0.0
        && (prose_barrier(&preceding, font) || short_reference)
        && caption.y_min >= last_rect.y_max
        && caption.y_min - last_rect.y_max < font * 0.6
        && (prior.x_min - caption.x_min).abs() < font * if short_reference { 2.0 } else { 1.0 }
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

fn paragraph_tokens<'a>(
    index: &'a ReadingIndex,
    paragraph: &super::Paragraph,
    page: u32,
) -> Vec<&'a ReadingToken> {
    super::paragraphs::pieces(paragraph)
        .iter()
        .flat_map(|span| {
            tokens(index, span.start, span.end)
                .into_iter()
                .filter(move |token| {
                    token.page == page && token.start >= span.start && token.end <= span.end
                })
        })
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

fn page_font(index: &ReadingIndex, page: u32) -> f32 {
    let mut fonts = index
        .tokens
        .iter()
        .filter(|t| t.page == page)
        .flat_map(|t| t.rects.iter().map(|r| r.y_max - r.y_min))
        .filter(|h| *h > 0.0)
        .collect::<Vec<_>>();
    fonts.sort_by(f32::total_cmp);
    fonts.get(fonts.len() / 2).copied().unwrap_or(10.0)
}

fn figure_region(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    images: &[TextRect],
) -> Option<TextRect> {
    let page = candidate.page;
    let caption_start = candidate.paragraph.start;
    let caption = candidate.rect;
    let dimensions = index
        .pages
        .iter()
        .find(|candidate| candidate.number == page)?;
    let body_font = page_font(index, page);
    if candidate.kind == "Table"
        && let Some(below) = table_below(index, candidate, captions, body_font, images)
    {
        return Some(below);
    }
    if candidate.kind == "Table"
        && let Some(above) = table_above(index, candidate, captions, body_font, images)
    {
        // Tables also put captions below a grid. Repeated printed numeric cells
        // provide a stronger grid cue than a generic diagram/prose rectangle.
        return Some(above);
    }
    let top = prose_top(index, candidate, captions, body_font, dimensions.height);
    let height = caption.y_min - top;
    if height < 35.0 || height > dimensions.height * 0.70 {
        return None;
    }
    let image_regions = images
        .iter()
        .copied()
        .filter(|rect| {
            rect.y_min >= top
                && rect.y_max <= caption.y_min
                && image_owner(captions, page, *rect)
                    .is_some_and(|owner| owner.paragraph.start == caption_start)
        })
        .collect::<Vec<_>>();
    let image_bounds = image_regions
        .iter()
        .copied()
        .filter(|rect| horizontal_gap(*rect, caption) == 0.0)
        .reduce(|a, b| union([a, b].into_iter()))
        .map(|seed| connected_bounds(seed, &image_regions, body_font * 3.0, body_font * 3.0));
    let native_top = if candidate.kind == "Figure" {
        table_floor(index, candidate, captions, images, body_font, top)
    } else {
        top
    };
    let native = native_regions(index, candidate, captions, native_top, body_font);
    let regions = &native.rects;
    if regions.len() > 1024 {
        return None;
    }
    let native_seed = regions
        .iter()
        .copied()
        .filter(|r| horizontal_gap(*r, caption) == 0.0)
        .reduce(|a, b| union([a, b].into_iter()));
    let enclosed_diagram = candidate.kind == "Figure"
        && image_bounds.is_some_and(|image| native.encloses_image(image));
    let image_only = image_bounds.is_some() && !enclosed_diagram;
    let seed = if enclosed_diagram {
        native_seed
    } else {
        image_bounds.or(native_seed)
    }?;
    // Partial raster inserts must not erase independently enclosing diagram
    // labels. Other image-backed figures retain the zero vertical gap, so a
    // preceding table cannot expand their observed image body upward.
    let bounds = connected_bounds(
        seed,
        regions,
        body_font * 3.0,
        if image_only { 0.0 } else { body_font * 3.0 },
    );
    if image_only {
        return Some(bounds);
    }

    Some(padded(
        bounds,
        body_font * 0.4,
        dimensions.width,
        native_top,
        caption.y_min,
    ))
}

struct NativeRegions {
    rects: Vec<TextRect>,
    diagram_labels: Vec<TextRect>,
    diagram_only: bool,
}

fn table_floor(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    images: &[TextRect],
    font: f32,
    mut top: f32,
) -> f32 {
    // A preceding table's caption is above its cells. The caption alone cannot
    // prevent those short numeric paragraphs from becoming a later figure's
    // native seed. Use the same observed grid as the table detector, never an
    // inferred frame or a caption-only exclusion band.
    for table in captions.iter().filter(|other| {
        other.kind == "Table"
            && other.page == candidate.page
            && other.rect.y_max < candidate.rect.y_min
    }) {
        if let Some(grid) = table_below(index, table, captions, font, images) {
            let overlap =
                grid.x_max.min(candidate.rect.x_max) - grid.x_min.max(candidate.rect.x_min);
            if grid.y_max < candidate.rect.y_min
                && overlap > (candidate.rect.x_max - candidate.rect.x_min) * 0.35
            {
                top = top.max(grid.y_max + 4.0);
            }
        }
    }
    top
}

impl NativeRegions {
    fn encloses_image(&self, image: TextRect) -> bool {
        if !self.diagram_only || self.diagram_labels.len() < 3 {
            return false;
        }
        let bounds = union(self.diagram_labels.iter().copied());
        bounds.x_min <= image.x_min
            && bounds.x_max >= image.x_max
            && bounds.y_min < image.y_min
            && bounds.y_max > image.y_max
            && self
                .diagram_labels
                .iter()
                .any(|label| label.y_max < image.y_min)
            && self
                .diagram_labels
                .iter()
                .any(|label| label.y_min > image.y_max)
    }
}

fn native_regions(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    top: f32,
    body_font: f32,
) -> NativeRegions {
    let mut result = NativeRegions {
        rects: Vec::new(),
        diagram_labels: Vec::new(),
        diagram_only: true,
    };
    let page = candidate.page;
    let caption = candidate.rect;
    let caption_start = candidate.paragraph.start;
    // Printed diagram labels/ticks establish an observed extent. Do not fill
    // whitespace back to a page margin or include the separate caption body.
    for paragraph in index
        .objects
        .paragraph
        .iter()
        .filter(|p| !captions.iter().any(|c| c.paragraph.start == p.start))
    {
        let text = paragraph_tokens(index, paragraph, page);
        if text.is_empty() || prose_barrier(&text, body_font) {
            continue;
        }
        let rect = union(text.iter().flat_map(|t| t.rects.iter().copied()));
        if rect.y_min < top || rect.y_max > caption.y_min {
            continue;
        }
        let nearest = captions
            .iter()
            .filter(|c| c.page == page && c.rect.y_min >= rect.y_max)
            .min_by(|a, b| {
                let distance = |c: &TextRect| {
                    let dx = (c.x_min - rect.x_max).max(rect.x_min - c.x_max).max(0.0);
                    dx * dx + 4.0 * (c.y_min - rect.y_max).powi(2)
                };
                distance(&a.rect).total_cmp(&distance(&b.rect))
            });
        if nearest.is_some_and(|c| c.paragraph.start == caption_start) {
            result.rects.push(rect);
            if horizontal_gap(rect, caption) == 0.0 {
                let diagram_label = text.len() <= 12
                    && text.iter().all(|token| {
                        !token.text.chars().any(char::is_numeric)
                            && !token.rects.is_empty()
                            && token.rects.iter().all(|r| {
                                let height = r.y_max - r.y_min;
                                height > 0.0 && height < body_font * 0.85
                            })
                    });
                result.diagram_only &= diagram_label;
                if diagram_label {
                    result.diagram_labels.push(rect);
                }
            }
        }
    }
    result
}

fn prose_top(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    body_font: f32,
    height: f32,
) -> f32 {
    let page = candidate.page;
    let caption_start = candidate.paragraph.start;
    let caption = candidate.rect;
    let mut top = height * 0.05;
    for paragraph in &index.objects.paragraph {
        if paragraph.start == caption_start {
            continue;
        }
        let text = paragraph_tokens(index, paragraph, page);
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
    top
}

fn vertical_gap(a: TextRect, b: TextRect) -> f32 {
    (a.y_min - b.y_max).max(b.y_min - a.y_max).max(0.0)
}

fn connected_bounds(
    mut bounds: TextRect,
    regions: &[TextRect],
    horizontal: f32,
    vertical: f32,
) -> TextRect {
    // Each successful step consumes one candidate; at most n² bounded checks.
    let mut pending = regions.to_vec();
    for _ in 0..pending.len() {
        let mut changed = false;
        pending.retain(|rect| {
            if horizontal_gap(*rect, bounds) <= horizontal
                && vertical_gap(*rect, bounds) <= vertical
            {
                bounds = union([bounds, *rect].into_iter());
                changed = true;
                false
            } else {
                true
            }
        });
        if !changed {
            break;
        }
    }
    bounds
}

fn image_owner<'a>(
    captions: &'a [Caption<'a>],
    page: u32,
    rect: TextRect,
) -> Option<&'a Caption<'a>> {
    let distance = |caption: &Caption<'_>| {
        4.0_f32.mul_add(
            vertical_gap(caption.rect, rect).powi(2),
            horizontal_gap(caption.rect, rect).powi(2),
        )
    };
    let mut owners = captions
        .iter()
        .filter(|caption| {
            caption.page == page
                && ((caption.kind == "Figure" && caption.rect.y_min >= rect.y_max)
                    || (caption.kind == "Table" && caption.rect.y_max <= rect.y_min))
        })
        .collect::<Vec<_>>();
    owners.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    let first = owners.first()?;
    if owners
        .get(1)
        .is_some_and(|second| distance(first).total_cmp(&distance(second)).is_eq())
    {
        return None;
    }
    Some(*first)
}

fn horizontal_gap(a: TextRect, b: TextRect) -> f32 {
    (a.x_min - b.x_max).max(b.x_min - a.x_max).max(0.0)
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
fn remember_rows(rows: &mut Vec<[f32; 2]>, text: &[&ReadingToken]) {
    rows.extend(
        text.iter()
            .flat_map(|token| &token.rects)
            .map(|cell| [cell.y_min, cell.y_max]),
    );
    rows.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged = Vec::<[f32; 2]>::new();
    for band in rows.drain(..) {
        if let Some(last) = merged.last_mut()
            && band[0] <= last[1]
        {
            // A superscript/subscript overlapping the base glyphs belongs to
            // the same printed line, even when its top differs substantially.
            last[1] = last[1].max(band[1]);
        } else {
            merged.push(band);
        }
    }
    *rows = merged;
}

fn table_below(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    font: f32,
    images: &[TextRect],
) -> Option<TextRect> {
    let page = candidate.page;
    let caption = candidate.rect;
    let dimensions = index.pages.iter().find(|p| p.number == page)?;
    let mut candidates = index
        .objects
        .paragraph
        .iter()
        .filter_map(|p| {
            let text = paragraph_tokens(index, p, page);
            if text.is_empty() {
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
    let image_barrier = images
        .iter()
        .filter(|rect| rect.y_min >= caption.y_max && horizontal_gap(**rect, caption) == 0.0)
        .map(|rect| rect.y_min)
        .reduce(f32::min)
        .unwrap_or(dimensions.height);
    let mut bounds = None;
    let mut numeric = false;
    let mut rows = Vec::new();
    for (paragraph, text, rect) in candidates {
        let gap_limit = if bounds.is_some() {
            font * 2.0
        } else {
            font * 5.0
        };
        if rect.y_max > image_barrier
            || rect.y_min - bounds.map_or(caption.y_max, |r: TextRect| r.y_max) > gap_limit
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
        remember_rows(&mut rows, &text);
        bounds = Some(bounds.map_or(rect, |r| union([r, rect].into_iter())));
    }
    let mut bounds = bounds?;
    if rows.len() < 2 {
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
        let text = paragraph_tokens(index, paragraph, page);
        if text.is_empty() || prose_barrier(&text, font) {
            continue;
        }
        let rect = union(text.iter().flat_map(|t| t.rects.iter().copied()));
        let cell_band = caption.y_max..=bounds.y_max;
        if cell_band.contains(&rect.y_min)
            && cell_band.contains(&rect.y_max)
            && (horizontal_gap(rect, caption) <= font * 2.0
                || repeated_grid_column(index, candidate, captions, rect, bounds, font))
        {
            numeric |= text
                .iter()
                .flat_map(|token| token.text.chars())
                .filter(char::is_ascii_digit)
                .count()
                >= 2;
            bounds = union([bounds, rect].into_iter());
        }
    }
    if !numeric {
        return None;
    }
    Some(padded(
        bounds,
        font * 0.4,
        dimensions.width,
        caption.y_max,
        image_barrier,
    ))
}

fn numeric_cell(token: &ReadingToken) -> bool {
    let text = token.text.trim_matches(['(', ')', '[', ']', ',', '%']);
    text.chars().any(|ch| ch.is_ascii_digit()) && text.parse::<f32>().is_ok_and(f32::is_finite)
}

fn table_column_rows(numeric: &[TextRect], caption_y: f32, font: f32) -> Option<Vec<f32>> {
    if numeric.len() > 256 {
        return None;
    }
    let mut best = Vec::<f32>::new();
    for anchor in numeric {
        let mut column = numeric
            .iter()
            .filter(|cell| (cell.x_min - anchor.x_min).abs() < font * 0.5)
            .map(|cell| cell.y_min)
            .collect::<Vec<_>>();
        column.sort_by(f32::total_cmp);
        column.dedup_by(|a, b| (*a - *b).abs() < font * 0.4);
        let mut run = Vec::new();
        // Start nearest the caption so unrelated earlier numeric columns cannot
        // establish a disconnected band of supposed table rows.
        for y in column.into_iter().rev() {
            if run.last().is_some_and(|last: &f32| *last - y > font * 2.2) {
                break;
            }
            run.push(y);
        }
        if run.len() >= 2 && caption_y - run[0] < font * 5.0 && run.len() > best.len() {
            best = run;
        }
    }
    (!best.is_empty()).then_some(best)
}

fn table_above(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    font: f32,
    images: &[TextRect],
) -> Option<TextRect> {
    let dimensions = index
        .pages
        .iter()
        .find(|page| page.number == candidate.page)?;
    let caption = candidate.rect;
    let mut lower = dimensions.height.mul_add(-0.4, caption.y_min).max(0.0);
    for other in captions.iter().filter(|other| other.page == candidate.page) {
        if other.rect.y_max < caption.y_min && horizontal_gap(other.rect, caption) == 0.0 {
            lower = lower.max(font.mul_add(0.4, other.rect.y_max));
        }
    }
    for image in images {
        if image.y_max < caption.y_min && horizontal_gap(*image, caption) == 0.0 {
            lower = lower.max(image.y_max);
        }
    }
    let cells = index
        .objects
        .paragraph
        .iter()
        .filter(|paragraph| {
            !captions
                .iter()
                .any(|c| c.paragraph.start == paragraph.start)
        })
        .flat_map(|paragraph| paragraph_tokens(index, paragraph, candidate.page))
        .filter(|token| {
            !token.rects.is_empty()
                && token.rects.iter().all(|rect| {
                    rect.y_min >= lower
                        && rect.y_max <= caption.y_min
                        && rect.x_min >= caption.x_min - font
                        && rect.x_max <= caption.x_max + font
                })
        })
        .collect::<Vec<_>>();
    if cells.len() > 2048 {
        return None;
    }
    let numeric = cells
        .iter()
        .copied()
        .filter(|token| numeric_cell(token))
        .map(|token| union(token.rects.iter().copied()))
        .collect::<Vec<_>>();
    let best = table_column_rows(&numeric, caption.y_min, font)?;
    let (&last, &first) = (best.first()?, best.last()?);
    let mut members = cells
        .iter()
        .copied()
        .filter(|token| {
            token
                .rects
                .iter()
                .all(|rect| best.iter().any(|y| (rect.y_min - y).abs() < font * 0.4))
        })
        .collect::<Vec<_>>();
    // Include one adjacent printed header row, keeping arbitrary earlier prose
    // out of the grid. The resulting region still consists of observed tokens.
    let header_y = cells
        .iter()
        .flat_map(|token| &token.rects)
        .filter(|rect| {
            rect.y_min < font.mul_add(-0.4, first) && rect.y_min >= font.mul_add(-2.2, first)
        })
        .map(|rect| rect.y_min)
        .max_by(f32::total_cmp);
    if let Some(y) = header_y {
        members.extend(cells.iter().copied().filter(|token| {
            token
                .rects
                .iter()
                .all(|rect| (rect.y_min - y).abs() < font * 0.4)
        }));
    }
    let bounds = union(members.iter().flat_map(|token| token.rects.iter().copied()));
    if bounds.y_max < last
        || bounds.x_min >= bounds.x_max
        || prose_between_grid_and_caption(index, candidate, bounds, font)
    {
        return None;
    }
    Some(padded(
        bounds,
        font * 0.4,
        dimensions.width,
        lower,
        caption.y_min,
    ))
}

fn prose_between_grid_and_caption(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    grid: TextRect,
    font: f32,
) -> bool {
    index.objects.paragraph.iter().any(|paragraph| {
        let text = paragraph_tokens(index, paragraph, candidate.page);
        if text.is_empty() || !prose_barrier(&text, font) {
            return false;
        }
        let rect = union(text.iter().flat_map(|token| token.rects.iter().copied()));
        // A method column occupying the established grid rows remains a grid
        // member. Separate prose below it cannot be crossed to reach a caption.
        rect.y_min >= grid.y_max
            && rect.y_max <= candidate.rect.y_min
            && horizontal_gap(rect, candidate.rect) == 0.0
    })
}

fn repeated_grid_column(
    index: &ReadingIndex,
    candidate: &Caption<'_>,
    captions: &[Caption<'_>],
    column: TextRect,
    grid: TextRect,
    font: f32,
) -> bool {
    let distance = |caption: &Caption<'_>| {
        let dy = (caption.rect.y_min - column.y_max)
            .max(column.y_min - caption.rect.y_max)
            .max(0.0);
        dy.mul_add(dy, horizontal_gap(caption.rect, column).powi(2))
    };
    if captions.iter().any(|other| {
        other.page == candidate.page
            && other.paragraph.start != candidate.paragraph.start
            && distance(other) < distance(candidate)
    }) {
        return false;
    }
    let owned = index
        .objects
        .paragraph
        .iter()
        .filter(|paragraph| {
            !captions
                .iter()
                .any(|caption| caption.paragraph.start == paragraph.start)
        })
        .flat_map(|paragraph| paragraph_tokens(index, paragraph, candidate.page))
        .flat_map(|token| token.rects.iter().copied())
        .filter(|rect| rect.y_min >= grid.y_min && rect.y_max <= grid.y_max)
        .collect::<Vec<_>>();
    let mut rows = Vec::<f32>::new();
    for remote in owned
        .iter()
        .filter(|rect| horizontal_gap(**rect, column) == 0.0)
    {
        if owned.iter().any(|cell| {
            horizontal_gap(*cell, grid) == 0.0 && (cell.y_min - remote.y_min).abs() < font * 0.35
        }) && rows.iter().all(|y| (*y - remote.y_min).abs() > font * 0.5)
        {
            rows.push(remote.y_min);
        }
    }
    rows.len() >= 2
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
    fn should_reject_a_duplicate_caption_when_a_short_indented_sentence_continues_into_its_reference()
     {
        let index = fixture(&[
            (
                "Figure 4: Independently placed schematic.",
                "caption",
                50.0,
                80.0,
            ),
            (
                "We explain the model and its changes in",
                "float",
                62.0,
                200.0,
            ),
            (
                "Figure 4. A further sentence describes the method.",
                "caption",
                50.0,
                213.0,
            ),
        ]);
        let figures = find(&index);
        assert_eq!(figures.len(), 1);
        assert!(figures[0].caption.contains("Independently placed"));
        assert_eq!(figures[0].references.len(), 1);
        // The body-boundary detector has its separate multi-line threshold.
        let previous = &index.objects.paragraph[1];
        assert!(!prose_barrier(&paragraph_tokens(&index, previous, 1), 10.0));
    }

    #[test]
    fn should_preserve_a_true_caption_when_nearby_text_lacks_sentence_continuation_evidence() {
        for (label, x, y) in [
            ("A short diagram label", 62.0, 200.0),
            ("We explain the model and its changes here", 62.0, 200.0),
            ("We explain the model and its changes in.", 62.0, 200.0),
            ("We explain the model and its changes in", 62.0, 180.0),
            ("We explain the model and its changes in", 330.0, 200.0),
        ] {
            let index = fixture(&[
                (label, "float", x, y),
                (
                    "Figure 4: A separate authored caption.",
                    "caption",
                    50.0,
                    213.0,
                ),
            ]);
            assert_eq!(find(&index).len(), 1, "{label}; {x}; {y}");
        }
    }

    #[test]
    fn should_keep_short_diagram_labels_when_the_caption_continuation_guard_changes() {
        let index = fixture(&[
            (
                "Several independent descriptors are combined in",
                "float",
                70.0,
                100.0,
            ),
            ("Output", "float", 70.0, 130.0),
            ("Figure 1: Full diagram.", "caption", 50.0, 170.0),
        ]);
        let region = find(&index)[0].rect.unwrap();
        assert!(region.y_min < 100.0 && region.y_max > 130.0);
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
    fn should_recover_grid_above_caption_when_following_numbered_heading_is_not_a_table() {
        let index = fixture(&[
            ("Method", "float", 80.0, 130.0),
            ("Score", "float", 210.0, 130.0),
            ("First", "float", 80.0, 146.0),
            ("0.553", "float", 210.0, 146.0),
            ("Second", "float", 80.0, 162.0),
            ("0.603", "float", 210.0, 162.0),
            (
                "Table I: Comparison of independently measured model scores.",
                "caption",
                50.0,
                185.0,
            ),
            ("2.2. Further experiments", "body", 50.0, 222.0),
        ]);
        let table = &find(&index)[0];
        let rect = table.rect.unwrap();
        assert!(rect.y_min < 130.0 && rect.y_min > 120.0);
        assert!(rect.y_max > 172.0 && rect.y_max < 185.0);
        assert!(rect.x_min > 70.0 && rect.x_max > 225.0 && rect.x_max < 245.0);
        assert!(
            table
                .spans
                .iter()
                .all(|span| span.end <= index.objects.paragraph[5].end)
        );
    }

    #[test]
    fn should_withhold_a_table_region_when_its_only_neighbor_is_one_numbered_heading() {
        let mut index = fixture(&[
            (
                "Table 1: A caption with unavailable grid geometry.",
                "caption",
                50.0,
                100.0,
            ),
            ("2.2. Further experiments", "body", 50.0, 135.0),
        ]);
        // The heading is slightly taller than the median native font, as is
        // common with bold text; that cannot manufacture a second printed row.
        for token in index.tokens.iter_mut().skip(8) {
            token.rects[0].y_max += 1.0;
        }
        assert!(find(&index)[0].rect.is_none());
    }

    #[test]
    fn should_count_one_printed_heading_when_scripts_overlap_its_base_glyphs() {
        for (top, bottom) in [(129.0, 136.0), (142.0, 149.0)] {
            let mut index = fixture(&[
                (
                    "Table 1: A caption with unavailable grid geometry.",
                    "caption",
                    50.0,
                    100.0,
                ),
                ("2.2. Further 2 experiments", "body", 50.0, 135.0),
            ]);
            let script = index
                .tokens
                .iter_mut()
                .find(|token| token.text == "2")
                .unwrap();
            script.rects[0].y_min = top;
            script.rects[0].y_max = bottom;
            assert!(
                find(&index)[0].rect.is_none(),
                "script band {top}..{bottom}"
            );
        }
    }

    #[test]
    fn should_withhold_a_disconnected_grid_when_prose_separates_it_from_the_caption() {
        let mut index = fixture(&[
            ("First", "float", 80.0, 130.0),
            ("0.553", "float", 210.0, 130.0),
            ("Second", "float", 80.0, 146.0),
            ("0.603", "float", 210.0, 146.0),
            (
                "This separate paragraph discusses the following experiment",
                "body",
                50.0,
                162.0,
            ),
            (
                "and establishes a prose boundary before its missing table.",
                "body",
                50.0,
                178.0,
            ),
            (
                "Table I: Results whose grid geometry is unavailable.",
                "caption",
                50.0,
                194.0,
            ),
        ]);
        index.objects.paragraph[4].end = index.objects.paragraph[5].end;
        index.objects.paragraph.remove(5);
        assert!(prose_barrier(
            &paragraph_tokens(&index, &index.objects.paragraph[4], 1),
            10.0
        ));
        let result = find(&index);
        assert_eq!(result.len(), 1);
        assert!(result[0].rect.is_none());
    }

    #[test]
    fn should_use_repeated_cell_rows_when_multiline_table_columns_look_like_prose() {
        let mut index = fixture(&[
            ("Method", "body", 70.0, 130.0),
            ("Independent baseline method alpha", "body", 70.0, 146.0),
            ("Independent baseline method beta", "body", 70.0, 162.0),
            ("Independent baseline method gamma", "body", 70.0, 178.0),
            ("Score", "body", 250.0, 130.0),
            ("0.55", "body", 250.0, 146.0),
            ("0.60", "body", 250.0, 162.0),
            ("0.58", "body", 250.0, 178.0),
            (
                "Table II: Detailed results for the independent comparison methods.",
                "caption",
                50.0,
                200.0,
            ),
            (
                "A neighboring column with separate native text",
                "body",
                350.0,
                178.0,
            ),
        ]);
        index.objects.paragraph[0].end = index.objects.paragraph[3].end;
        index.objects.paragraph.drain(1..4);
        assert!(prose_barrier(
            &paragraph_tokens(&index, &index.objects.paragraph[0], 1),
            10.0
        ));
        let table = &find(&index)[0];
        let rect = table.rect.unwrap();
        assert!(rect.y_min < 130.0 && rect.y_max > 188.0 && rect.y_max < 200.0);
        assert!(rect.x_min < 70.0 && rect.x_max > 265.0 && rect.x_max < 300.0);
        assert!(
            table
                .spans
                .iter()
                .any(|span| utf16_slice(&index.text, span.start, span.end).contains("gamma"))
        );
    }

    #[test]
    fn should_exclude_neighboring_column_labels_when_extending_a_table_grid() {
        let index = fixture(&[
            ("Table VI: Model scores.", "body", 50.0, 100.0),
            ("Method Score", "float", 70.0, 130.0),
            ("First 12.5", "float", 70.0, 146.0),
            ("Second 25.0", "float", 70.0, 162.0),
            ("Neighboring plot label", "float", 350.0, 150.0),
        ]);
        let result = find(&index);
        assert!(result[0].rect.unwrap().x_max < 200.0);
        assert!(
            result[0]
                .spans
                .iter()
                .all(|span| span.end <= index.objects.paragraph[3].end)
        );
    }

    #[test]
    fn should_use_only_retained_members_when_a_logical_paragraph_surrounds_another_caption() {
        let mut index = fixture(&[
            ("Upper label", "float", 100.0, 120.0),
            ("Figure 2: Neighbor.", "caption", 400.0, 140.0),
            ("Lower label", "float", 100.0, 160.0),
            ("Figure 1: Main plot.", "caption", 50.0, 220.0),
        ]);
        let first = index.objects.paragraph[0].clone();
        let last = index.objects.paragraph[2].clone();
        index.objects.paragraph[0].end = last.end;
        index.objects.paragraph[0].spans = vec![
            TextRange {
                start: first.start,
                end: first.end,
            },
            TextRange {
                start: last.start,
                end: last.end,
            },
        ];
        index.objects.paragraph.remove(2);
        let result = find(&index);
        let region = result
            .iter()
            .find(|figure| figure.label == "Figure 1")
            .unwrap()
            .rect
            .unwrap();
        assert!(region.x_max < 200.0);
    }

    #[test]
    fn should_exclude_later_page_coordinates_when_logical_members_continue_across_pages() {
        for table in [false, true] {
            let caption = if table {
                "Table II: Measurements."
            } else {
                "Figure 1: Main plot."
            };
            let caption_y = if table { 100.0 } else { 220.0 };
            let mut index = fixture(&[
                ("First 12.5", "float", 100.0, 140.0),
                ("Second 25.0", "float", 100.0, 160.0),
                (caption, "caption", 50.0, caption_y),
                ("Foreign 99.0", "float", 450.0, 150.0),
            ]);
            let first = index.objects.paragraph[0].clone();
            let last = index.objects.paragraph[3].clone();
            for token in index.tokens.iter_mut().filter(|t| t.start >= last.start) {
                token.page = 2;
            }
            let mut second_page = index.pages[0].clone();
            second_page.number = 2;
            second_page.start = last.start;
            index.pages[0].end = last.start - 2;
            index.pages.push(second_page);
            index.objects.paragraph[0].end = last.end;
            index.objects.paragraph[0].spans = vec![
                TextRange {
                    start: first.start,
                    end: first.end,
                },
                TextRange {
                    start: last.start,
                    end: last.end,
                },
            ];
            index.objects.paragraph.pop();
            let result = find(&index);
            assert_eq!(result.len(), 1);
            assert!(result[0].rect.unwrap().x_max < 200.0, "table={table}");
        }
    }

    #[test]
    fn should_withhold_a_caption_when_its_single_anchor_cannot_preserve_piece_or_page_identity() {
        for cross_page in [false, true] {
            let mut index = fixture(&[
                ("Figure 1: Start.", "caption", 50.0, 150.0),
                ("Intervening text", "float", 350.0, 160.0),
                ("Caption continuation.", "caption", 50.0, 174.0),
            ]);
            let first = index.objects.paragraph[0].clone();
            let last = index.objects.paragraph[2].clone();
            index.objects.paragraph[0].end = last.end;
            if cross_page {
                for token in index.tokens.iter_mut().filter(|t| t.start >= last.start) {
                    token.page = 2;
                }
                let mut second_page = index.pages[0].clone();
                second_page.number = 2;
                second_page.start = last.start;
                index.pages[0].end = last.start - 2;
                index.pages.push(second_page);
            } else {
                index.objects.paragraph[0].spans = vec![
                    TextRange {
                        start: first.start,
                        end: first.end,
                    },
                    TextRange {
                        start: last.start,
                        end: last.end,
                    },
                ];
            }
            index.objects.paragraph.truncate(1);
            assert!(find(&index).is_empty());
        }
    }

    #[test]
    fn should_extend_connected_labels_when_a_drawing_is_wider_than_its_caption() {
        let index = fixture(&[
            (
                "A lengthy top diagram label establishes the width",
                "float",
                50.0,
                120.0,
            ),
            ("Intermediate node", "float", 180.0, 170.0),
            ("Action classifier", "float", 260.0, 150.0),
            ("Unrelated heading", "body", 430.0, 150.0),
            ("Figure 2: Diagram.", "caption", 50.0, 220.0),
        ]);
        let figure = &find(&index)[0];
        assert!(figure.rect.unwrap().x_max > 300.0);
        assert!(figure.rect.unwrap().x_max < 400.0);
    }

    #[test]
    fn should_stop_the_grid_when_a_separated_section_heading_follows_its_last_row() {
        let index = fixture(&[
            ("Table IV: Scores.", "body", 50.0, 100.0),
            ("Method Score", "float", 70.0, 130.0),
            ("First 12.5", "float", 70.0, 146.0),
            ("Second 25.0", "float", 70.0, 162.0),
            ("B. Further results", "body", 50.0, 201.0),
        ]);
        assert!(find(&index)[0].rect.unwrap().y_max < 180.0);
    }

    #[test]
    fn should_recover_a_distant_table_column_when_multiple_cell_rows_align() {
        let index = fixture(&[
            ("Table III: Scores.", "body", 50.0, 100.0),
            ("Method", "float", 50.0, 130.0),
            ("Score", "float", 180.0, 130.0),
            ("Result", "float", 330.0, 130.0),
            ("First", "float", 50.0, 146.0),
            ("12.5", "float", 180.0, 146.0),
            ("22.5", "float", 330.0, 146.0),
            ("Second", "float", 50.0, 162.0),
            ("25.0", "float", 180.0, 162.0),
            ("54.70", "float", 330.0, 162.0),
        ]);
        let table = &find(&index)[0];
        assert!(table.rect.unwrap().x_max > 350.0);
        assert!(
            table
                .spans
                .iter()
                .any(|span| utf16_slice(&index.text, span.start, span.end).contains("54.70"))
        );
    }

    #[test]
    fn should_revisit_neighbor_labels_when_a_later_label_connects_their_vertical_band() {
        let mut index = fixture(&[
            ("Seed", "float", 50.0, 100.0),
            ("A", "float", 110.0, 120.0),
            ("B", "float", 125.0, 105.0),
            ("Figure 1: Diagram.", "caption", 50.0, 160.0),
        ]);
        let rects = [
            TextRect {
                x_min: 50.0,
                x_max: 100.0,
                y_min: 100.0,
                y_max: 110.0,
            },
            TextRect {
                x_min: 110.0,
                x_max: 120.0,
                y_min: 120.0,
                y_max: 140.0,
            },
            TextRect {
                x_min: 125.0,
                x_max: 130.0,
                y_min: 105.0,
                y_max: 130.0,
            },
        ];
        for (token, rect) in index.tokens.iter_mut().take(3).zip(rects) {
            token.rects = vec![rect];
        }
        for token in index.tokens.iter_mut().skip(3) {
            token.rects = vec![TextRect {
                x_min: 50.0,
                x_max: 100.0,
                y_min: 160.0,
                y_max: 170.0,
            }];
        }
        assert!(find(&index)[0].rect.unwrap().y_max >= 140.0);
    }

    #[test]
    fn should_preserve_enclosing_diagram_labels_when_only_raster_inserts_are_available() {
        let mut index = fixture(&[
            ("Input", "float", 80.0, 80.0),
            ("Encoder", "float", 200.0, 140.0),
            ("Output", "float", 220.0, 200.0),
            (
                "Figure 1: A labeled system with partially available raster inserts.",
                "caption",
                50.0,
                230.0,
            ),
        ]);
        for token in index.tokens.iter_mut().take(3) {
            token.rects[0].y_max = token.rects[0].y_min + 6.0;
        }
        let native = find(&index);
        let image = TextRect {
            x_min: 90.0,
            y_min: 110.0,
            x_max: 120.0,
            y_max: 170.0,
        };
        let page = super::super::graphics::PageGraphics {
            page: 1,
            status: "partial".into(),
            trace_sha256: None,
            images: vec![image],
            unsupported_images: 2,
            mask: None,
        };
        let with_image = find_with_images(&index, &[page]);
        assert_eq!(with_image[0].rect, native[0].rect);
        assert_eq!(
            serde_json::to_value(&with_image[0].spans).unwrap(),
            serde_json::to_value(&native[0].spans).unwrap()
        );
        assert!(with_image[0].rect.unwrap().x_max > 240.0);
        assert!(with_image[0].rect.unwrap().y_min < 80.0);
        assert!(with_image[0].rect.unwrap().y_max > 206.0);
    }

    #[test]
    fn should_keep_image_bounds_when_native_labels_lack_independent_diagram_enclosure() {
        for case in [
            "above_only",
            "numeric_grid",
            "unicode_numeric_grid",
            "separate_caption",
            "missing_geometry",
        ] {
            let bottom_text = match case {
                "numeric_grid" => "Output 25",
                "unicode_numeric_grid" => "٥٦",
                "missing_geometry" => "Output node",
                _ => "Output",
            };
            let bottom_y = if case == "above_only" { 90.0 } else { 200.0 };
            let (top_text, middle_text) = if case == "unicode_numeric_grid" {
                ("١٢", "٣٤")
            } else {
                ("Input", "Encoder")
            };
            let mut lines = vec![
                (top_text, "float", 80.0, 80.0),
                (middle_text, "float", 200.0, 140.0),
                (bottom_text, "float", 220.0, bottom_y),
            ];
            if case == "separate_caption" {
                lines.push((
                    "Figure 1: An independently owned preceding diagram.",
                    "caption",
                    50.0,
                    100.0,
                ));
            }
            lines.push((
                "Figure 2: A separate image with a complete observed boundary.",
                "caption",
                50.0,
                230.0,
            ));
            let mut index = fixture(&lines);
            for token in &mut index.tokens {
                if token.start < index.objects.paragraph[3].start {
                    token.rects[0].y_max = token.rects[0].y_min + 6.0;
                }
                if token.text == "node" {
                    token.rects.clear();
                }
            }
            let image = TextRect {
                x_min: 90.0,
                // Keep the image beyond the earlier caption's existing
                // four-point ownership barrier in this control case.
                y_min: if case == "separate_caption" {
                    120.0
                } else {
                    110.0
                },
                x_max: 120.0,
                y_max: 170.0,
            };
            let page = super::super::graphics::PageGraphics {
                page: 1,
                status: "partial".into(),
                trace_sha256: None,
                images: vec![image],
                unsupported_images: 2,
                mask: None,
            };
            let result = find_with_images(&index, &[page]);
            let figure = result
                .iter()
                .find(|figure| figure.label == "Figure 2")
                .unwrap();
            assert_eq!(figure.rect, Some(image), "{case}");
        }
    }

    #[test]
    fn should_union_only_owned_image_tiles_when_neighboring_columns_contain_other_floats() {
        let index = fixture(&[
            ("Figure 1: Left result.", "caption", 40.0, 250.0),
            ("Table I: Right table.", "body", 340.0, 105.0),
        ]);
        let images = super::super::graphics::PageGraphics {
            page: 1,
            status: "complete".into(),
            trace_sha256: None,
            unsupported_images: 0,
            mask: None,
            images: vec![
                TextRect {
                    x_min: 40.0,
                    x_max: 130.0,
                    y_min: 130.0,
                    y_max: 230.0,
                },
                TextRect {
                    x_min: 130.0,
                    x_max: 220.0,
                    y_min: 130.0,
                    y_max: 230.0,
                },
                TextRect {
                    x_min: 340.0,
                    x_max: 490.0,
                    y_min: 130.0,
                    y_max: 240.0,
                },
            ],
        };
        let objects = find_with_images(&index, &[images]);
        let figure = &objects[0];
        assert_eq!(
            figure.rect,
            Some(TextRect {
                x_min: 40.0,
                x_max: 220.0,
                y_min: 130.0,
                y_max: 230.0
            })
        );
    }

    #[test]
    fn should_exclude_a_separately_captioned_bitmap_table_when_a_figure_follows_in_the_same_column()
    {
        let index = fixture(&[
            ("Table I: A raster table.", "body", 50.0, 100.0),
            ("Figure 1: A separate plot.", "caption", 50.0, 300.0),
        ]);
        let table = TextRect {
            x_min: 50.0,
            x_max: 200.0,
            y_min: 130.0,
            y_max: 190.0,
        };
        let plot = TextRect {
            x_min: 50.0,
            x_max: 200.0,
            y_min: 220.0,
            y_max: 280.0,
        };
        let images = super::super::graphics::PageGraphics {
            page: 1,
            status: "complete".into(),
            trace_sha256: None,
            unsupported_images: 0,
            mask: None,
            images: vec![table, plot],
        };
        let figures = find_with_images(&index, &[images]);
        assert_eq!(figures[1].rect, Some(plot));
        assert!(figures[0].rect.is_none()); // unsupported raster-table geometry stays explicit
    }

    #[test]
    fn should_separate_table_and_image_when_a_plot_follows_the_last_grid_row() {
        let index = fixture(&[
            ("Table V: Scores.", "body", 50.0, 100.0),
            ("First 12.5", "float", 60.0, 130.0),
            ("Second 25.0", "float", 60.0, 146.0),
            ("Figure 8: Plot.", "caption", 50.0, 280.0),
        ]);
        let rect = TextRect {
            x_min: 50.0,
            x_max: 200.0,
            y_min: 175.0,
            y_max: 265.0,
        };
        let images = super::super::graphics::PageGraphics {
            page: 1,
            status: "complete".into(),
            trace_sha256: None,
            unsupported_images: 0,
            mask: None,
            images: vec![rect],
        };
        let objects = find_with_images(&index, &[images]);
        assert!(objects[0].rect.unwrap().y_max < 175.0);
        assert_eq!(objects[1].rect, Some(rect));
    }

    #[test]
    fn should_exclude_preceding_table_cells_when_a_figure_has_only_native_labels() {
        let index = fixture(&[
            ("Table V: Scores.", "body", 50.0, 100.0),
            ("First 12.5", "float", 60.0, 130.0),
            ("Second 25.0", "float", 60.0, 146.0),
            ("Axis", "float", 70.0, 200.0),
            ("0 1 2", "float", 70.0, 240.0),
            ("Figure 8: Plot.", "caption", 50.0, 280.0),
        ]);
        let original = serde_json::to_vec(&index).unwrap();
        let objects = find(&index);
        let grid = objects[0].rect.unwrap();
        let plot = objects[1].rect.unwrap();
        assert!(grid.y_max < 175.0);
        assert!(plot.y_min > 190.0 && plot.y_max < 260.0);
        assert!(objects[1].spans.iter().all(|span| {
            let text = utf16_slice(&index.text, span.start, span.end);
            !text.contains("First") && !text.contains("Second")
        }));
        assert_eq!(serde_json::to_vec(&index).unwrap(), original);
    }

    #[test]
    fn should_preserve_a_neighboring_plot_when_a_table_occupies_another_column() {
        let index = fixture(&[
            ("Table 1: Scores.", "caption", 350.0, 100.0),
            ("First 12.5", "float", 360.0, 130.0),
            ("Second 25.0", "float", 360.0, 146.0),
            ("Axis", "float", 70.0, 130.0),
            ("0 1 2", "float", 70.0, 200.0),
            ("Figure 1: Plot.", "caption", 50.0, 250.0),
        ]);
        let plot = find(&index)[1].rect.unwrap();
        assert!(plot.y_min < 135.0 && plot.y_max > 200.0);
        assert!(plot.x_max < 200.0);
    }

    #[test]
    fn should_keep_observed_labels_when_a_preceding_table_caption_has_no_grid() {
        let index = fixture(&[
            ("Table 1: Scores.", "caption", 50.0, 100.0),
            ("Legend", "float", 70.0, 140.0),
            ("Group A", "float", 70.0, 155.0),
            ("Figure 1: Plot.", "caption", 50.0, 250.0),
        ]);
        let plot = find(&index)[1].rect.unwrap();
        assert!(plot.y_min < 145.0 && plot.y_max > 155.0);
    }

    #[test]
    fn should_preserve_unavailable_geometry_when_only_a_preceding_grid_is_observed() {
        let index = fixture(&[
            ("Table 1: Scores.", "caption", 50.0, 100.0),
            ("First 12.5", "float", 60.0, 130.0),
            ("Second 25.0", "float", 60.0, 146.0),
            ("Figure 1: Unobserved vector plot.", "caption", 50.0, 280.0),
        ]);
        let objects = find(&index);
        assert!(objects[0].rect.is_some());
        assert!(objects[1].rect.is_none());
        assert!(objects[1].spans.is_empty());
    }

    #[test]
    fn should_preserve_an_owned_image_when_it_starts_inside_the_table_padding_margin() {
        let index = fixture(&[
            ("Table 1: Scores.", "caption", 50.0, 100.0),
            ("First 12.5", "float", 60.0, 130.0),
            ("Second 25.0", "float", 60.0, 146.0),
            ("Figure 1: Plot.", "caption", 50.0, 260.0),
        ]);
        let rect = TextRect {
            x_min: 50.0,
            x_max: 200.0,
            y_min: 162.0,
            y_max: 220.0,
        };
        let graphics = super::super::graphics::PageGraphics {
            page: 1,
            status: "complete".into(),
            trace_sha256: None,
            unsupported_images: 0,
            mask: None,
            images: vec![rect],
        };
        let objects = find_with_images(&index, &[graphics]);
        assert_eq!(objects[1].rect, Some(rect));
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
