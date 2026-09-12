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
}

pub(super) fn paragraphs(page: &SourcePage) -> Vec<SourceParagraph<'_>> {
    let mut by_line = BTreeMap::<u32, Vec<&SourceWord>>::new();
    for word in &page.words {
        by_line.entry(word.line).or_default().push(word);
    }
    let lines = by_line
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
                words,
            })
        })
        .collect::<Vec<_>>();
    let mut heights = page
        .words
        .iter()
        .map(|word| word.rect.y_max - word.rect.y_min)
        .filter(|height| *height > 3.0 && *height < page.height * 0.07)
        .collect::<Vec<_>>();
    heights.sort_by(f32::total_cmp);
    let font = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
    let mut result = Vec::<SourceParagraph<'_>>::new();
    let mut previous: Option<&Line<'_>> = None;
    for line in &lines {
        // Folios and rotated repository stamps are navigational furniture, not prose.
        if omit(line, page, font) {
            continue;
        }
        let inferred_kind = kind(line, page, font);
        let inherited = inferred_kind == "body"
            && previous.is_some_and(|previous| previous.block == line.block)
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
            let hanging = !block_changed
                && shift > 0.0
                && shift < font * 4.0
                && !previous.text.ends_with(['.', '!', '?', ':', ';'])
                && line.text.chars().next().is_some_and(char::is_lowercase);
            let same_column = shift.abs() < font * 2.2 || hanging;
            let continuation = block_changed
                && same_column
                && (-font * 0.2..font * 0.75).contains(&gap)
                && !previous.text.ends_with(['.', '!', '?', ':'])
                && kind == "body"
                && result
                    .last()
                    .is_some_and(|paragraph| paragraph.kind == "body");
            let indented = !inherited
                && !hanging
                && shift > font * 0.7
                && shift < font * 3.5
                && gap > -font * 0.2;
            let new_kind = result
                .last()
                .is_some_and(|paragraph| paragraph.kind != kind);
            new_kind
                || (!continuation && block_changed)
                || gap > font * 0.9
                || gap < -font * 0.65
                || !same_column
                || indented
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
    let mut word_heights = line
        .words
        .iter()
        .map(|word| word.rect.y_max - word.rect.y_min)
        .collect::<Vec<_>>();
    word_heights.sort_by(f32::total_cmp);
    let height = word_heights
        .get(word_heights.len() / 2)
        .copied()
        .unwrap_or(font);
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
