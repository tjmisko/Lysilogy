//! Source-text access: UTF-16 <-> byte mapping, token lookup, and text helpers.

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use super::{SourceText, SourceToken, TextSpan};

macro_rules! pattern {
    ($source:literal) => {{
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new($source).expect("constant bibliography regex")
        });
        &*PATTERN
    }};
}
pub(crate) use pattern;

/// One monotone map supports checked conversion in both directions in O(log n).
/// Repeated byte positions denote the interior of a UTF-16 surrogate pair and
/// are rejected when reading a public source span.
pub(super) struct Source<'a> {
    text: &'a SourceText,
    bytes: Vec<usize>,
    tokens: Vec<&'a SourceToken>,
}

impl<'a> Source<'a> {
    pub(super) fn new(text: &'a SourceText) -> Self {
        let mut bytes = Vec::with_capacity(text.text.len() + 1);
        for (byte, ch) in text.text.char_indices() {
            bytes.extend(std::iter::repeat_n(byte, ch.len_utf16()));
        }
        bytes.push(text.text.len());
        let mut tokens: Vec<_> = text.tokens.iter().collect();
        tokens.sort_by_key(|token| (token.start, token.end));
        Self {
            text,
            bytes,
            tokens,
        }
    }

    pub(super) const fn source_text(&self) -> &'a SourceText {
        self.text
    }

    /// All tokens ordered by start offset.
    pub(super) fn sorted_tokens(&self) -> &[&'a SourceToken] {
        &self.tokens
    }

    /// Length of the source text in UTF-16 code units.
    pub(super) const fn len_utf16(&self) -> usize {
        self.bytes.len() - 1
    }

    pub(super) fn byte(&self, offset: usize) -> Option<usize> {
        let byte = *self.bytes.get(offset)?;
        (offset == 0 || self.bytes[offset - 1] != byte).then_some(byte)
    }

    pub(super) fn text(&self, span: TextSpan) -> Option<&'a str> {
        self.text
            .text
            .get(self.byte(span.start)?..self.byte(span.end)?)
    }

    pub(super) fn range(&self, start: usize, end: usize) -> TextSpan {
        TextSpan {
            start: self.bytes.partition_point(|byte| *byte < start),
            end: self.bytes.partition_point(|byte| *byte < end),
        }
    }

    pub(super) fn tokens(&self, span: TextSpan) -> impl Iterator<Item = &'a SourceToken> + '_ {
        // Source tokens are disjoint, ordered words. Starting at the first
        // intersecting word avoids a full-paper scan per lookup.
        let start = self.tokens.partition_point(|token| token.end <= span.start);
        self.tokens[start..]
            .iter()
            .copied()
            .take_while(move |token| token.start < span.end)
    }

    /// The page holding the span's first token, else the page containing its start.
    pub(super) fn anchor(&self, span: TextSpan) -> (u32, TextSpan) {
        let page = self.tokens(span).next().map_or_else(
            || {
                self.text
                    .pages
                    .iter()
                    .find(|page| (page.start..page.end).contains(&span.start))
                    .map_or(1, |page| page.number)
            },
            |token| token.page,
        );
        (page, span)
    }
}

pub(super) fn label_regex() -> &'static Regex {
    pattern!(r"\A[\t ]*(?:\[([\p{L}\d][\p{L}\d+,:.\-–]{0,30})\]|\((\d{1,4})\)|(\d{1,4})[.)])[\t ]+")
}

pub(super) fn year_regex() -> &'static Regex {
    pattern!(r"\b(?:18|19|20)\d{2}[a-z]?\b")
}

pub(super) fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn trimmed(value: &str) -> Option<String> {
    let value = clean(value.trim_matches(|ch: char| ch.is_whitespace() || ".,;:".contains(ch)));
    (!value.is_empty()).then_some(value)
}

pub(super) fn key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|ch| !ch.is_whitespace())
        .collect()
}
