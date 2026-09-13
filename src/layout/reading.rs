//! Page isolation belongs only to the reading index, never saved citation anchors.
//!
//! This recognizes Poppler's bounded XHTML framing without interpreting a DTD,
//! entities, or arbitrary XML. Word text (including Poppler control glyphs) is
//! left to the existing verbatim decoder. A bad page body cannot donate words.

use std::collections::BTreeSet;

use super::{make_layout_page, normalize_rect, parse_page_tokens};
use crate::{
    Error, Result,
    domain::{LayoutPage, TextRect},
};

pub struct ReadingLayoutPage<'a> {
    pub page: LayoutPage,
    pub content: &'a str,
    pub failure: Option<String>,
}

struct PageFrame<'a> {
    content: &'a str,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Open,
    Close,
    Empty,
    Comment,
    Instruction,
    Declaration,
}

struct Tag<'a> {
    name: &'a str,
    attributes: &'a str,
    kind: TagKind,
    start: usize,
    end: usize,
}

/// Validate every document/page boundary before admitting any page-local result.
/// A malformed word or line discards that whole page and retains its error.
pub fn parse_pages(input: &str) -> Result<Vec<ReadingLayoutPage<'_>>> {
    page_frames(input)?
        .into_iter()
        .enumerate()
        .map(|(index, frame)| {
            let number = u32::try_from(index + 1).map_err(|_| invalid("too many page elements"))?;
            let result = validate_page(frame.content, frame.width, frame.height)
                .and_then(|()| parse_page_tokens(frame.content, frame.width, frame.height, false));
            let (tokens, failure) = match result {
                Ok(tokens) => (tokens, None),
                Err(error) => (Vec::new(), Some(error.to_string())),
            };
            Ok(ReadingLayoutPage {
                page: make_layout_page(number, frame.width, frame.height, tokens),
                content: frame.content,
                failure,
            })
        })
        .collect()
}

fn invalid(reason: &str) -> Error {
    Error::InvalidLayout(reason.to_owned())
}

fn page_frames(input: &str) -> Result<Vec<PageFrame<'_>>> {
    let mut pages = Vec::new();
    let mut stack = Vec::<&str>::new();
    let mut active = None::<(usize, f32, f32)>;
    let mut cursor = 0;
    let mut framing_seen = Vec::new();
    while let Some(tag) = next_tag(input, cursor)? {
        if let Some((start, width, height)) = active {
            if tag.name == "page" {
                if tag.kind != TagKind::Close {
                    return Err(invalid(
                        "nested page elements make page boundaries ambiguous",
                    ));
                }
                pages.push(PageFrame {
                    content: &input[start..tag.start],
                    width,
                    height,
                });
                active = None;
            } else if matches!(tag.name, "html" | "head" | "body" | "doc") {
                return Err(invalid("document framing crosses an unclosed page"));
            }
        } else {
            if stack.last() != Some(&"title") && !input[cursor..tag.start].trim().is_empty() {
                return Err(invalid(
                    "text outside the document's page or metadata framing",
                ));
            }
            match tag.kind {
                TagKind::Comment => {}
                TagKind::Instruction | TagKind::Declaration => {
                    if !framing_seen.is_empty() || !stack.is_empty() {
                        return Err(invalid(
                            "document declaration appears inside or after its root",
                        ));
                    }
                }
                TagKind::Close => {
                    if stack.pop() != Some(tag.name) {
                        return Err(invalid("mismatched document or page closing tag"));
                    }
                }
                TagKind::Open | TagKind::Empty if tag.name == "page" => {
                    if stack.last() != Some(&"doc") {
                        return Err(invalid("page element is not a direct child of doc"));
                    }
                    let attributes = attributes(tag.attributes)?;
                    let width = number_attribute(&attributes, "width")?;
                    let height = number_attribute(&attributes, "height")?;
                    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
                        return Err(invalid("page dimensions must be positive finite values"));
                    }
                    if tag.kind == TagKind::Empty {
                        pages.push(PageFrame {
                            content: &input[tag.end..tag.end],
                            width,
                            height,
                        });
                    } else {
                        active = Some((tag.end, width, height));
                    }
                }
                TagKind::Open | TagKind::Empty => {
                    open_framing(&tag, &mut stack, &mut framing_seen)?;
                }
            }
        }
        cursor = tag.end;
    }
    if active.is_some() || !stack.is_empty() || !input[cursor..].trim().is_empty() {
        return Err(invalid(
            "unterminated document/page framing or trailing content",
        ));
    }
    if !framing_seen.contains(&"doc") || pages.is_empty() {
        return Err(invalid("Poppler returned no framed page elements"));
    }
    Ok(pages)
}

fn open_framing<'a>(
    tag: &Tag<'a>,
    stack: &mut Vec<&'a str>,
    seen: &mut Vec<&'a str>,
) -> Result<()> {
    attributes(tag.attributes)?;
    let parent = stack.last().copied();
    let valid = match tag.name {
        "html" => parent.is_none(),
        "head" | "body" => parent == Some("html"),
        "title" | "meta" => parent == Some("head"),
        "doc" => parent.is_none() || parent == Some("body"),
        _ => false,
    };
    if !valid || parent.is_none() && !seen.is_empty() {
        return Err(invalid("invalid or repeated document root/framing element"));
    }
    if tag.name != "meta" {
        if seen.contains(&tag.name) || tag.name == "head" && seen.contains(&"body") {
            return Err(invalid("repeated or misordered document framing"));
        }
        seen.push(tag.name);
    }
    if tag.kind == TagKind::Open {
        stack.push(tag.name);
    }
    Ok(())
}

fn validate_page(content: &str, width: f32, height: f32) -> Result<()> {
    let mut stack = Vec::<&str>::new();
    let mut cursor = 0;
    while let Some(tag) = next_tag(content, cursor)? {
        if stack.last() != Some(&"word") && !content[cursor..tag.start].trim().is_empty() {
            return Err(invalid("native text appears outside a word element"));
        }
        match tag.kind {
            TagKind::Close => {
                if stack.pop() != Some(tag.name) {
                    return Err(invalid("mismatched native word/line element"));
                }
            }
            TagKind::Open => {
                let parent = stack.last().copied();
                let valid = match tag.name {
                    "flow" => parent.is_none(),
                    "block" => parent.is_none() || parent == Some("flow"),
                    "line" => parent.is_none() || matches!(parent, Some("flow" | "block")),
                    "word" => parent == Some("line"),
                    _ => false,
                };
                if !valid {
                    return Err(invalid("invalid native word/line nesting"));
                }
                let attributes = attributes(tag.attributes)?;
                if tag.name == "word" {
                    // The legacy decoder finds attributes by substring. Accept
                    // only Poppler's four numeric word attributes so unrelated
                    // names or quoted values cannot shadow coordinates or the
                    // opening tag boundary. Saved anchor parsing is unchanged.
                    if attributes
                        .iter()
                        .any(|(name, _)| !matches!(*name, "xMin" | "yMin" | "xMax" | "yMax"))
                    {
                        return Err(invalid("unsupported native word attribute"));
                    }
                    normalize_rect(
                        TextRect {
                            x_min: number_attribute(&attributes, "xMin")?,
                            y_min: number_attribute(&attributes, "yMin")?,
                            x_max: number_attribute(&attributes, "xMax")?,
                            y_max: number_attribute(&attributes, "yMax")?,
                        },
                        width,
                        height,
                    )?;
                }
                if matches!(tag.name, "line" | "word") {
                    // The reused Poppler decoder recognizes this exact opening.
                    // Reject other spellings instead of silently skipping their words.
                    if !content[tag.start..tag.end].starts_with(&format!("<{} ", tag.name)) {
                        return Err(invalid("unsupported native word/line opening"));
                    }
                }
                stack.push(tag.name);
            }
            _ => return Err(invalid("unsupported markup inside native page content")),
        }
        cursor = tag.end;
    }
    if !stack.is_empty() || !content[cursor..].trim().is_empty() {
        return Err(invalid("unterminated native word/line content"));
    }
    Ok(())
}

fn number_attribute(attributes: &[(&str, &str)], name: &str) -> Result<f32> {
    let value = attributes
        .iter()
        .find(|(key, _)| *key == name)
        .ok_or_else(|| Error::InvalidLayout(format!("missing {name} attribute")))?
        .1;
    value
        .parse()
        .map_err(|error| Error::InvalidLayout(format!("invalid {name} attribute: {error}")))
}

fn attributes(mut input: &str) -> Result<Vec<(&str, &str)>> {
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    while !input.trim().is_empty() {
        input = input.trim_start();
        let end = input
            .find(|ch: char| !name_character(ch))
            .unwrap_or(input.len());
        if end == 0 {
            return Err(invalid("invalid attribute name"));
        }
        let name = &input[..end];
        input = input[end..]
            .trim_start()
            .strip_prefix('=')
            .ok_or_else(|| invalid("attribute has no equals sign"))?
            .trim_start();
        let quote = input
            .chars()
            .next()
            .ok_or_else(|| invalid("missing attribute value"))?;
        if !matches!(quote, '\'' | '"') {
            return Err(invalid("unquoted attribute value"));
        }
        input = &input[1..];
        let end = input
            .find(quote)
            .ok_or_else(|| invalid("unterminated attribute value"))?;
        let value = &input[..end];
        if value.contains('<') {
            return Err(invalid("unescaped markup inside attribute"));
        }
        if !seen.insert(name) {
            return Err(invalid("duplicate attribute"));
        }
        output.push((name, value));
        input = &input[end + 1..];
        if input.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
            return Err(invalid("attributes must be separated by whitespace"));
        }
    }
    Ok(output)
}

const fn name_character(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '-' | '.')
}

fn next_tag(input: &str, cursor: usize) -> Result<Option<Tag<'_>>> {
    let Some(relative) = input[cursor..].find('<') else {
        return Ok(None);
    };
    let start = cursor + relative;
    let rest = &input[start..];
    for (prefix, suffix, kind) in [
        ("<!--", "-->", TagKind::Comment),
        ("<?", "?>", TagKind::Instruction),
    ] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            let close = stripped
                .find(suffix)
                .ok_or_else(|| invalid("unterminated document comment or instruction"))?;
            return Ok(Some(Tag {
                name: "",
                attributes: "",
                kind,
                start,
                end: start + prefix.len() + close + suffix.len(),
            }));
        }
    }
    let mut quote = None;
    let mut end = None;
    for (offset, ch) in rest[1..].char_indices() {
        if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() {
            match ch {
                '\'' | '"' => quote = Some(ch),
                '>' => {
                    end = Some(start + offset + 2);
                    break;
                }
                '<' => return Err(invalid("unclosed tag before another tag")),
                _ => {}
            }
        }
    }
    let end = end.ok_or_else(|| invalid("unterminated document tag"))?;
    let raw = &input[start + 1..end - 1];
    if raw.starts_with("!DOCTYPE ") && !raw.contains(['[', ']']) {
        return Ok(Some(Tag {
            name: "",
            attributes: "",
            kind: TagKind::Declaration,
            start,
            end,
        }));
    }
    let (raw, kind) = raw.strip_prefix('/').map_or_else(
        || {
            raw.strip_suffix('/').map_or((raw, TagKind::Open), |value| {
                (value.trim_end(), TagKind::Empty)
            })
        },
        |value| (value.trim_end(), TagKind::Close),
    );
    let split = raw
        .find(|ch: char| !name_character(ch))
        .unwrap_or(raw.len());
    if split == 0 {
        return Err(invalid("invalid document tag name"));
    }
    let name = &raw[..split];
    let attributes = &raw[split..];
    if kind == TagKind::Close && !attributes.trim().is_empty() {
        return Err(invalid("closing tag contains attributes"));
    }
    Ok(Some(Tag {
        name,
        attributes,
        kind,
        start,
        end,
    }))
}

#[cfg(test)]
mod tests;
