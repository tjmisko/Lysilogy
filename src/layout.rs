use crate::{
    Result,
    domain::{
        CitationStatus, DocumentLayout, LayoutPage, LayoutSentence, LayoutToken, TextAnchor,
        TextRect,
    },
    error::Error,
};

const LAYOUT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug)]
struct ParsedToken {
    text: String,
    line: u32,
    rects: Vec<TextRect>,
}

/// Parse Poppler's `pdftotext -bbox-layout` XHTML into a stable, page-local
/// coordinate model. Coordinates remain in PDF points so they can be scaled
/// without losing alignment with the rendered page.
pub fn parse_bbox_layout(input: &str) -> Result<DocumentLayout> {
    let mut pages = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = input[cursor..].find("<page ") {
        let start = cursor + relative_start;
        let tag_end = input[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| Error::InvalidLayout("unterminated page tag".to_owned()))?;
        let close = input[tag_end..]
            .find("</page>")
            .map(|offset| tag_end + offset)
            .ok_or_else(|| Error::InvalidLayout("unterminated page element".to_owned()))?;
        let tag = &input[start..=tag_end];
        let width = parse_attribute(tag, "width")?;
        let height = parse_attribute(tag, "height")?;
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(Error::InvalidLayout(
                "page dimensions must be positive finite values".to_owned(),
            ));
        }

        let number = u32::try_from(pages.len() + 1).unwrap_or(u32::MAX);
        let parsed = parse_page_tokens(&input[tag_end + 1..close], width, height)?;
        let tokens = parsed
            .into_iter()
            .enumerate()
            .map(|(index, token)| LayoutToken {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                text: token.text,
                line: token.line,
                rects: token.rects,
            })
            .collect::<Vec<_>>();
        let sentences = segment_sentences(number, &tokens);
        pages.push(LayoutPage {
            number,
            width,
            height,
            tokens,
            sentences,
        });
        cursor = close + "</page>".len();
    }

    if pages.is_empty() {
        return Err(Error::InvalidLayout(
            "Poppler returned no page elements".to_owned(),
        ));
    }
    Ok(DocumentLayout {
        schema_version: LAYOUT_SCHEMA_VERSION,
        pages,
    })
}

fn parse_page_tokens(content: &str, page_width: f32, page_height: f32) -> Result<Vec<ParsedToken>> {
    let mut output = Vec::<ParsedToken>::new();
    let mut cursor = 0;
    let mut line_number = 0_u32;
    while let Some(relative_start) = content[cursor..].find("<line ") {
        let start = cursor + relative_start;
        let tag_end = content[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| Error::InvalidLayout("unterminated line tag".to_owned()))?;
        let close = content[tag_end..]
            .find("</line>")
            .map(|offset| tag_end + offset)
            .ok_or_else(|| Error::InvalidLayout("unterminated line element".to_owned()))?;
        let line_content = &content[tag_end + 1..close];
        parse_line_tokens(
            line_content,
            line_number,
            page_width,
            page_height,
            &mut output,
        )?;
        line_number = line_number.saturating_add(1);
        cursor = close + "</line>".len();
    }
    Ok(output)
}

fn parse_line_tokens(
    content: &str,
    line: u32,
    page_width: f32,
    page_height: f32,
    output: &mut Vec<ParsedToken>,
) -> Result<()> {
    let mut cursor = 0;
    while let Some(relative_start) = content[cursor..].find("<word ") {
        let start = cursor + relative_start;
        let tag_end = content[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| Error::InvalidLayout("unterminated word tag".to_owned()))?;
        let close = content[tag_end..]
            .find("</word>")
            .map(|offset| tag_end + offset)
            .ok_or_else(|| Error::InvalidLayout("unterminated word element".to_owned()))?;
        let tag = &content[start..=tag_end];
        let rect = normalize_rect(
            TextRect {
                x_min: parse_attribute(tag, "xMin")?,
                y_min: parse_attribute(tag, "yMin")?,
                x_max: parse_attribute(tag, "xMax")?,
                y_max: parse_attribute(tag, "yMax")?,
            },
            page_width,
            page_height,
        )?;
        let text = decode_xml_text(content[tag_end + 1..close].trim());
        if !text.is_empty() {
            push_or_merge_token(
                output,
                ParsedToken {
                    text,
                    line,
                    rects: vec![rect],
                },
            );
        }
        cursor = close + "</word>".len();
    }
    Ok(())
}

fn push_or_merge_token(output: &mut Vec<ParsedToken>, mut incoming: ParsedToken) {
    let Some(previous) = output.last_mut() else {
        output.push(incoming);
        return;
    };
    let previous_rect = previous.rects.last().copied().unwrap_or_default();
    let incoming_rect = incoming.rects[0];
    let height = (previous_rect.y_max - previous_rect.y_min)
        .min(incoming_rect.y_max - incoming_rect.y_min)
        .max(1.0);
    let gap = incoming_rect.x_min - previous_rect.x_max;
    let same_line_fragment = previous.line == incoming.line
        && (-0.5..=height * 0.24).contains(&gap)
        && previous
            .text
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        && incoming
            .text
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
    let hyphenated_line_break = previous.line != incoming.line
        && previous.text.ends_with('-')
        && incoming.text.chars().next().is_some_and(char::is_lowercase);

    if same_line_fragment || hyphenated_line_break {
        if hyphenated_line_break {
            previous.text.pop();
        }
        previous.text.push_str(&incoming.text);
        if same_line_fragment {
            if let Some(rect) = previous.rects.last_mut() {
                rect.x_max = rect.x_max.max(incoming_rect.x_max);
                rect.y_min = rect.y_min.min(incoming_rect.y_min);
                rect.y_max = rect.y_max.max(incoming_rect.y_max);
            }
        } else {
            previous.rects.append(&mut incoming.rects);
        }
    } else {
        output.push(incoming);
    }
}

fn parse_attribute(tag: &str, name: &str) -> Result<f32> {
    let prefix = format!(r#"{name}=""#);
    let start = tag
        .find(&prefix)
        .map(|index| index + prefix.len())
        .ok_or_else(|| Error::InvalidLayout(format!("missing {name} attribute")))?;
    let end = tag[start..]
        .find('"')
        .map(|offset| start + offset)
        .ok_or_else(|| Error::InvalidLayout(format!("unterminated {name} attribute")))?;
    tag[start..end]
        .parse::<f32>()
        .map_err(|error| Error::InvalidLayout(format!("invalid {name} attribute: {error}")))
}

fn normalize_rect(mut rect: TextRect, width: f32, height: f32) -> Result<TextRect> {
    let values = [rect.x_min, rect.y_min, rect.x_max, rect.y_max];
    if !values.into_iter().all(f32::is_finite) || rect.x_min > rect.x_max || rect.y_min > rect.y_max
    {
        return Err(Error::InvalidLayout(
            "word rectangle must contain ordered finite coordinates".to_owned(),
        ));
    }
    // Some PDFs intentionally paint glyphs beyond their crop box. The viewer
    // cannot display that area, so preserving alignment means clipping to the
    // rendered page rather than rejecting an otherwise readable document.
    rect.x_min = rect.x_min.clamp(0.0, width);
    rect.y_min = rect.y_min.clamp(0.0, height);
    rect.x_max = rect.x_max.clamp(rect.x_min, width);
    rect.y_max = rect.y_max.clamp(rect.y_min, height);
    Ok(rect)
}

fn decode_xml_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative_ampersand) = value[cursor..].find('&') {
        let ampersand = cursor + relative_ampersand;
        output.push_str(&value[cursor..ampersand]);
        let Some(relative_semicolon) = value[ampersand..].find(';') else {
            output.push_str(&value[ampersand..]);
            return output;
        };
        let semicolon = ampersand + relative_semicolon;
        let entity = &value[ampersand + 1..semicolon];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            numeric if numeric.starts_with("#x") => u32::from_str_radix(&numeric[2..], 16)
                .ok()
                .and_then(char::from_u32),
            numeric if numeric.starts_with('#') => {
                numeric[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        if let Some(character) = decoded {
            output.push(character);
        } else {
            output.push_str(&value[ampersand..=semicolon]);
        }
        cursor = semicolon + 1;
    }
    output.push_str(&value[cursor..]);
    output
}

fn segment_sentences(page: u32, tokens: &[LayoutToken]) -> Vec<LayoutSentence> {
    let mut sentences = Vec::new();
    let mut start = 0_usize;
    for (index, token) in tokens.iter().enumerate() {
        let length = index.saturating_sub(start) + 1;
        if (ends_sentence(&token.text) || length >= 120) && index >= start {
            sentences.push(make_sentence(page, sentences.len(), &tokens[start..=index]));
            start = index + 1;
        }
    }
    if start < tokens.len() {
        sentences.push(make_sentence(page, sentences.len(), &tokens[start..]));
    }
    sentences
}

fn ends_sentence(token: &str) -> bool {
    let trimmed = token.trim_end_matches(['\'', '"', '’', '”', ')', ']']);
    if !trimmed.ends_with(['.', '?', '!']) {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    ![
        "e.g.", "i.e.", "mr.", "mrs.", "dr.", "prof.", "fig.", "sec.", "etc.", "vs.",
    ]
    .iter()
    .any(|abbreviation| lowered.ends_with(abbreviation))
}

fn make_sentence(page: u32, sentence_index: usize, tokens: &[LayoutToken]) -> LayoutSentence {
    let start_token = tokens.first().map_or(0, |token| token.index);
    let end_token = tokens.last().map_or(start_token, |token| token.index);
    LayoutSentence {
        id: format!("p{page:04}-s{:05}", sentence_index + 1),
        page,
        start_token,
        end_token,
        text: tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        rects: merged_rects(tokens.iter().flat_map(|token| token.rects.iter().copied())),
    }
}

fn merged_rects(rectangles: impl Iterator<Item = TextRect>) -> Vec<TextRect> {
    let mut rectangles = rectangles.collect::<Vec<_>>();
    rectangles.sort_by(|left, right| {
        left.y_min
            .total_cmp(&right.y_min)
            .then_with(|| left.x_min.total_cmp(&right.x_min))
    });
    let mut output = Vec::<TextRect>::new();
    for rect in rectangles {
        if let Some(previous) = output.last_mut() {
            let vertical_overlap = previous.y_min <= rect.y_max && rect.y_min <= previous.y_max;
            if vertical_overlap {
                previous.x_min = previous.x_min.min(rect.x_min);
                previous.y_min = previous.y_min.min(rect.y_min);
                previous.x_max = previous.x_max.max(rect.x_max);
                previous.y_max = previous.y_max.max(rect.y_max);
                continue;
            }
        }
        output.push(rect);
    }
    output
}

/// Resolve a proposed citation against PDF-coordinate tokens. Matching ignores
/// whitespace, punctuation, case, and common ligatures, but requires complete
/// token boundaries.
///
/// This tolerates PDF extraction artifacts without allowing
/// the analyzer to cite text that is absent from the document.
#[must_use]
pub fn verify_quote(
    layout: &DocumentLayout,
    quote: &str,
    preferred_page: u32,
) -> (CitationStatus, Option<TextAnchor>) {
    let needle = canonical_chars(quote);
    if needle.len() < 8 {
        return (CitationStatus::Missing, None);
    }

    let preferred_matches = layout
        .pages
        .iter()
        .filter(|page| page.number == preferred_page)
        .flat_map(|page| page_matches(page, &needle))
        .collect::<Vec<_>>();
    let matches = if preferred_matches.is_empty() {
        layout
            .pages
            .iter()
            .flat_map(|page| page_matches(page, &needle))
            .collect::<Vec<_>>()
    } else {
        preferred_matches
    };
    if matches.len() > 1 {
        return (CitationStatus::Ambiguous, None);
    }
    let Some(anchor) = matches.into_iter().next() else {
        return (CitationStatus::Missing, None);
    };
    let exact = compact_whitespace(quote) == compact_whitespace(&anchor.exact_text);
    (
        if exact {
            CitationStatus::Exact
        } else {
            CitationStatus::Normalized
        },
        Some(anchor),
    )
}

fn page_matches(page: &LayoutPage, needle: &[char]) -> Vec<TextAnchor> {
    let mut haystack = Vec::<char>::new();
    let mut owners = Vec::<u32>::new();
    for token in &page.tokens {
        for character in canonical_chars(&token.text) {
            haystack.push(character);
            owners.push(token.index);
        }
    }
    if needle.len() > haystack.len() {
        return Vec::new();
    }

    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(start, window)| {
            if *window != needle {
                return false;
            }
            let end = start + needle.len();
            (*start == 0 || owners[*start - 1] != owners[*start])
                && (end == owners.len() || owners[end - 1] != owners[end])
        })
        .filter_map(|(start, _)| {
            let start_token = owners[start];
            let end_token = owners[start + needle.len() - 1];
            anchor_from_token_range(page, start_token, end_token)
        })
        .collect()
}

fn canonical_chars(value: &str) -> Vec<char> {
    value
        .chars()
        .flat_map(|character| match character {
            'ﬁ' => vec!['f', 'i'],
            'ﬂ' => vec!['f', 'l'],
            'ﬀ' => vec!['f', 'f'],
            'ﬃ' => vec!['f', 'f', 'i'],
            'ﬄ' => vec!['f', 'f', 'l'],
            other if other.is_alphanumeric() => other.to_lowercase().collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn anchor_from_token_range(page: &LayoutPage, start: u32, end: u32) -> Option<TextAnchor> {
    let tokens = page
        .tokens
        .iter()
        .filter(|token| (start..=end).contains(&token.index))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return None;
    }
    let sentence_ids = page
        .sentences
        .iter()
        .filter(|sentence| sentence.start_token <= end && sentence.end_token >= start)
        .map(|sentence| sentence.id.clone())
        .collect();
    Some(TextAnchor {
        page: page.number,
        start_token: start,
        end_token: end,
        sentence_ids,
        rects: merged_rects(tokens.iter().flat_map(|token| token.rects.iter().copied())),
        exact_text: tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    })
}

#[must_use]
pub fn anchor_for_sentence_range(
    layout: &DocumentLayout,
    start_id: &str,
    end_id: Option<&str>,
) -> Option<TextAnchor> {
    let start = layout
        .pages
        .iter()
        .flat_map(|page| &page.sentences)
        .find(|sentence| sentence.id == start_id)?;
    let end = end_id.map_or(Some(start), |id| {
        layout
            .pages
            .iter()
            .flat_map(|page| &page.sentences)
            .find(|sentence| sentence.id == id)
    })?;
    if start.page != end.page {
        return None;
    }
    let page = layout.pages.iter().find(|page| page.number == start.page)?;
    let first = start.start_token.min(end.start_token);
    let last = start.end_token.max(end.end_token);
    anchor_from_token_range(page, first, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BBOX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<doc><page width="200.000000" height="300.000000"><flow><block>
<line xMin="10" yMin="20" xMax="150" yMax="30">
<word xMin="10" yMin="20" xMax="20" yMax="30">The</word>
<word xMin="24" yMin="20" xMax="45" yMax="30">main</word>
<word xMin="49" yMin="20" xMax="75" yMax="30">claim</word>
<word xMin="79" yMin="20" xMax="90" yMax="30">is</word>
<word xMin="94" yMin="20" xMax="130" yMax="30">grounded.</word>
</line></block></flow></page></doc>"#;

    #[test]
    fn parses_pages_tokens_sentences_and_coordinates() -> Result<()> {
        let layout = parse_bbox_layout(BBOX)?;
        assert_eq!(layout.pages.len(), 1);
        assert_eq!(layout.pages[0].tokens.len(), 5);
        assert_eq!(layout.pages[0].sentences.len(), 1);
        assert_eq!(layout.pages[0].sentences[0].id, "p0001-s00001");
        assert!((layout.pages[0].sentences[0].rects[0].x_min - 10.0).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn verifies_quotes_with_normalized_punctuation() -> Result<()> {
        let layout = parse_bbox_layout(BBOX)?;
        let (status, anchor) = verify_quote(&layout, "THE MAIN CLAIM is grounded", 1);
        assert_eq!(status, CitationStatus::Normalized);
        let anchor = anchor.ok_or_else(|| Error::Task("missing anchor".to_owned()))?;
        assert_eq!(anchor.start_token, 0);
        assert_eq!(anchor.end_token, 4);
        Ok(())
    }

    #[test]
    fn rejects_text_that_is_not_in_the_pdf() -> Result<()> {
        let layout = parse_bbox_layout(BBOX)?;
        let (status, anchor) = verify_quote(&layout, "A fabricated sentence is not evidence.", 1);
        assert_eq!(status, CitationStatus::Missing);
        assert!(anchor.is_none());
        Ok(())
    }

    #[test]
    fn clips_authored_glyphs_to_the_pdf_crop_box() -> Result<()> {
        let rect = normalize_rect(
            TextRect {
                x_min: -12.0,
                y_min: 20.0,
                x_max: 240.0,
                y_max: 35.0,
            },
            200.0,
            300.0,
        )?;
        assert!(rect.x_min.abs() < f32::EPSILON);
        assert!((rect.x_max - 200.0).abs() < f32::EPSILON);
        Ok(())
    }
}
