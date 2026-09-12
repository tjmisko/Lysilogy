use std::collections::BTreeMap;

use super::{Provenance, SourcePage, SourceWord};
use crate::{Result, domain::TextRect, layout::parse_verbatim_bbox_layout};

pub(super) fn parse(xml: &str) -> Result<Vec<SourcePage>> {
    let layout = parse_verbatim_bbox_layout(xml)?;
    let block_maps = block_maps(xml);
    Ok(layout
        .pages
        .into_iter()
        .enumerate()
        .map(|(index, page)| SourcePage {
            number: page.number,
            width: page.width,
            height: page.height,
            words: page
                .tokens
                .into_iter()
                .filter_map(|token| {
                    let rect = token.rects.first().copied()?;
                    Some(SourceWord {
                        text: token.text,
                        line: token.line,
                        block: block_maps
                            .get(index)
                            .and_then(|blocks| blocks.get(token.line as usize))
                            .copied()
                            .unwrap_or(token.line),
                        rect,
                    })
                })
                .collect(),
            provenance: Provenance::Native,
            confidence: None,
        })
        .collect())
}

fn block_maps(xml: &str) -> Vec<Vec<u32>> {
    xml.split("<page ")
        .skip(1)
        .map(|page| {
            let content = page.split("</page>").next().unwrap_or(page);
            let mut block = 0_u32;
            let mut blocks = Vec::new();
            for tag in content.split('<') {
                if tag.starts_with("block ") {
                    block += 1;
                }
                if tag.starts_with("line ") {
                    blocks.push(block);
                }
            }
            blocks
        })
        .collect()
}

pub(super) fn usable(page: &SourcePage) -> bool {
    let characters = page
        .words
        .iter()
        .flat_map(|word| word.text.chars())
        .collect::<Vec<_>>();
    let valid = characters.iter().filter(|ch| ch.is_alphanumeric()).count();
    page.words.len() >= 8 && valid >= 35 && valid * 3 >= characters.len()
}

pub(super) struct SourceParagraph<'a> {
    pub words: Vec<&'a SourceWord>,
    pub kind: String,
}

struct Line<'a> {
    words: Vec<&'a SourceWord>,
    block: u32,
    rect: TextRect,
    text: String,
    height: f32,
}

pub(super) fn paragraphs(page: &SourcePage) -> Vec<SourceParagraph<'_>> {
    let mut heights = page
        .words
        .iter()
        .map(|word| word.rect.y_max - word.rect.y_min)
        .filter(|height| *height > 3.0 && *height < page.height * 0.07)
        .collect::<Vec<_>>();
    heights.sort_by(f32::total_cmp);
    let font = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
    let lines = source_lines(page)
        .into_iter()
        .filter(|line| !omit(line, page, font))
        .collect::<Vec<_>>();
    group_lines(&lines, page, font)
}

fn source_lines(page: &SourcePage) -> Vec<Line<'_>> {
    let mut by_line = BTreeMap::<u32, Vec<&SourceWord>>::new();
    for word in &page.words {
        by_line.entry(word.line).or_default().push(word);
    }
    by_line
        .into_values()
        .filter_map(|words| {
            let first = words.first()?;
            Some(Line {
                block: first.block,
                rect: union(words.iter().map(|word| word.rect)),
                text: words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                height: median_height(&words),
                words,
            })
        })
        .collect()
}

fn group_lines<'a>(lines: &[Line<'a>], page: &SourcePage, font: f32) -> Vec<SourceParagraph<'a>> {
    let kinds = lines
        .iter()
        .map(|line| kind(line, page, font))
        .collect::<Vec<_>>();
    let mut result = Vec::<SourceParagraph<'_>>::new();
    let mut previous: Option<&Line<'_>> = None;
    for (index, line) in lines.iter().enumerate() {
        let profile = local_profile(lines, &kinds, index, font);
        let inferred_kind = kinds[index];
        let next = lines.get(index + 1);
        let indented = previous.is_some_and(|previous| {
            first_line_indent(previous, line, next, profile, font)
                && !hanging_continuation(previous, line, next, kinds[index - 1], font)
        });
        let inherited = inferred_kind == "body"
            && !indented
            && previous.is_some_and(|previous| {
                previous.block == line.block
                    && same_column(previous, line, font)
                    && line.rect.y_min - previous.rect.y_max <= profile.paragraph_gap(font)
                    && line.rect.x_min >= font.mul_add(-0.25, previous.rect.x_min)
                    && (line.height - previous.height).abs() < font * 0.15
            })
            && result.last().is_some_and(|paragraph| {
                matches!(paragraph.kind.as_str(), "caption" | "list" | "footnote")
            });
        let kind = if inherited {
            match result.last().map(|paragraph| paragraph.kind.as_str()) {
                Some("caption") => "caption",
                Some("list") => "list",
                _ => "footnote",
            }
        } else {
            inferred_kind
        };
        let new_paragraph = previous.is_none_or(|previous| {
            let gap = line.rect.y_min - previous.rect.y_max;
            let shift = line.rect.x_min - previous.rect.x_min;
            let block_changed = line.block != previous.block;
            let continuation = block_changed
                && same_column(previous, line, font)
                && shift.abs() < font * 0.25
                && gap >= -font * 0.2
                && gap < font.mul_add(0.35, profile.line_gap).max(font * 0.75)
                && !ends_sentence(&previous.text)
                && line.text.chars().next().is_some_and(char::is_lowercase)
                && kind == "body"
                && result
                    .last()
                    .is_some_and(|paragraph| paragraph.kind == "body");
            let ended_hanging = shift < -font * 0.25
                && ends_sentence(&previous.text)
                && result
                    .last()
                    .and_then(|paragraph| paragraph.words.first())
                    .is_some_and(|first| previous.rect.x_min - first.rect.x_min > font * 0.5);
            let new_kind = result
                .last()
                .is_some_and(|paragraph| paragraph.kind != kind);
            new_kind
                || (!continuation && block_changed)
                || gap > profile.paragraph_gap(font)
                || gap < -font * 0.65
                || !same_column(previous, line, font)
                || indented
                || ended_hanging
                || kind == "heading"
                || inferred_kind == "list"
        });
        if new_paragraph {
            result.push(SourceParagraph {
                words: Vec::new(),
                kind: kind.to_owned(),
            });
        }
        if let Some(paragraph) = result.last_mut() {
            paragraph.words.extend(&line.words);
        }
        previous = Some(line);
    }
    result
}

#[derive(Clone, Copy)]
struct ColumnProfile {
    left: f32,
    line_gap: f32,
}

impl ColumnProfile {
    fn paragraph_gap(self, font: f32) -> f32 {
        font.mul_add(0.55, self.line_gap).max(font * 0.9)
    }
}

// Poppler can put many indented paragraphs into one block. Infer the recurring
// prose margin in a bounded reading-order neighborhood, not the previous line's
// x position: a new paragraph can even outdent from a hanging definition.
fn local_profile(lines: &[Line<'_>], kinds: &[&str], index: usize, font: f32) -> ColumnProfile {
    let line = &lines[index];
    let start = index.saturating_sub(16);
    let end = (index + 17).min(lines.len());
    let candidates = (start..end)
        .filter(|&other| {
            let other_line = &lines[other];
            (kinds[other] == "body" || kinds[index] == "footnote" && kinds[other] == "footnote")
                && same_column(line, other_line, font)
                && (line.height - other_line.height).abs() < font * 0.15
                && other_line
                    .text
                    .chars()
                    .filter(|ch| ch.is_alphabetic())
                    .count()
                    >= 12
        })
        .collect::<Vec<_>>();
    let same_block = candidates
        .iter()
        .copied()
        .filter(|&other| lines[other].block == line.block)
        .collect::<Vec<_>>();
    let candidates = if same_block.len() >= 3 {
        &same_block
    } else {
        &candidates
    };
    let mut margins = candidates
        .iter()
        .map(|&other| lines[other].rect.x_min)
        .collect::<Vec<_>>();
    margins.sort_by(f32::total_cmp);
    let tolerance = (font * 0.12).max(0.7);
    let left = margins
        .windows(2)
        .find(|pair| pair[1] - pair[0] <= tolerance)
        .map_or_else(
            || margins.first().copied().unwrap_or(line.rect.x_min),
            |pair| pair[0],
        );
    let mut gaps = candidates
        .windows(2)
        .filter_map(|pair| {
            if pair[1] != pair[0] + 1 {
                return None;
            }
            let gap = lines[pair[1]].rect.y_min - lines[pair[0]].rect.y_max;
            (gap >= -font * 0.2 && gap <= font * 2.2).then_some(gap)
        })
        .collect::<Vec<_>>();
    gaps.sort_by(f32::total_cmp);
    ColumnProfile {
        left,
        // The lower median avoids mistaking a paragraph gap for normal leading.
        line_gap: gaps
            .get(gaps.len().saturating_sub(1) / 2)
            .copied()
            .unwrap_or(font * 0.3),
    }
}

fn same_column(left: &Line<'_>, right: &Line<'_>, font: f32) -> bool {
    let shift = (left.rect.x_min - right.rect.x_min).abs();
    let overlap = left.rect.x_max.min(right.rect.x_max) - left.rect.x_min.max(right.rect.x_min);
    shift < font * 4.0 && (overlap > 0.0 || shift < font * 0.25)
}

fn first_line_indent(
    previous: &Line<'_>,
    line: &Line<'_>,
    next: Option<&Line<'_>>,
    profile: ColumnProfile,
    font: f32,
) -> bool {
    let threshold = (font * 0.25).max(1.8);
    let indent = line.rect.x_min - profile.left;
    if indent < threshold || indent >= font * 4.0 {
        return false;
    }
    let next_returns = next.is_some_and(|next| {
        same_column(line, next, font)
            && next.rect.y_min > line.rect.y_min
            && (next.rect.x_min - profile.left).abs() < threshold
    });
    next_returns
        || line.rect.x_min - previous.rect.x_min >= threshold && ends_sentence(&previous.text)
}

fn hanging_continuation(
    previous: &Line<'_>,
    line: &Line<'_>,
    next: Option<&Line<'_>>,
    previous_kind: &str,
    font: f32,
) -> bool {
    let shift = line.rect.x_min - previous.rect.x_min;
    let label = [" - ", " – ", " — ", ": "]
        .iter()
        .any(|separator| previous.text.contains(separator));
    let keeps_hanging = next.is_some_and(|next| {
        next.block == line.block
            && next.rect.y_min > line.rect.y_min
            && (next.rect.x_min - line.rect.x_min).abs() < font * 0.2
    });
    previous.block == line.block
        && shift > 0.0
        && shift < font * 4.0
        && !ends_sentence(&previous.text)
        && (label || previous_kind == "list" || keeps_hanging)
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['\"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', ':', ';'])
}

fn median_height(words: &[&SourceWord]) -> f32 {
    let mut heights = words
        .iter()
        .map(|word| word.rect.y_max - word.rect.y_min)
        .collect::<Vec<_>>();
    heights.sort_by(f32::total_cmp);
    heights.get(heights.len() / 2).copied().unwrap_or(10.0)
}

fn omit(line: &Line<'_>, page: &SourcePage, font: f32) -> bool {
    let folio = line.text.trim().chars().all(|ch| ch.is_ascii_digit())
        && line.text.len() <= 5
        && (line.rect.y_min > page.height * 0.90
            || line.rect.y_max < page.height * 0.06
            || line.rect.y_min > page.height * 0.80 && line.text.parse::<u32>() == Ok(page.number));
    let rotated =
        line.rect.x_max < page.width * 0.075 && line.rect.y_max - line.rect.y_min > font * 3.0;
    folio || rotated
}

fn kind(line: &Line<'_>, page: &SourcePage, font: f32) -> &'static str {
    // Superscripts/subscripts must not make an ordinary prose line look like a heading.
    let height = line.height;
    let lower = line.text.to_ascii_lowercase();
    if ["figure ", "fig.", "fig ", "table "]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        && (height < font * 0.96
            || line.words.iter().take(3).any(|word| {
                word.text.chars().any(|ch| ch.is_ascii_digit()) && word.text.ends_with([':', '.'])
            }))
    {
        return "caption";
    }
    if line.rect.y_min < page.height * 0.045 || line.rect.y_min > page.height * 0.95 {
        return "header";
    }
    if height < font * 0.90 && line.rect.y_min > page.height * 0.68 {
        return "footnote";
    }
    if height > font * 1.16 && line.words.len() < 24 {
        return "heading";
    }
    if named_heading(&lower) {
        return "heading";
    }
    let first = line.text.split_whitespace().next().unwrap_or("");
    if matches!(first, "•" | "●" | "–" | "—" | "-")
        || first.ends_with([')', '.'])
            && first
                .trim_end_matches([')', '.'])
                .chars()
                .all(|ch| ch.is_ascii_digit())
    {
        return "list";
    }
    "body"
}

fn named_heading(text: &str) -> bool {
    let title =
        text.trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.' || ch.is_whitespace());
    matches!(
        title,
        "abstract"
            | "introduction"
            | "background"
            | "related work"
            | "methods"
            | "methodology"
            | "results"
            | "discussion"
            | "conclusion"
            | "conclusions"
            | "references"
            | "acknowledgments"
            | "acknowledgements"
            | "appendix"
    )
}

pub(super) fn union(rects: impl Iterator<Item = TextRect>) -> TextRect {
    rects
        .reduce(|left, right| TextRect {
            x_min: left.x_min.min(right.x_min),
            y_min: left.y_min.min(right.y_min),
            x_max: left.x_max.max(right.x_max),
            y_max: left.y_max.max(right.y_max),
        })
        .unwrap_or_default()
}
