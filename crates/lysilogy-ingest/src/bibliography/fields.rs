//! Field parsing for printed bibliography entries.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::source::{clean, label_regex, pattern, trimmed, year_regex};

/// Categorical evidence, deliberately not an uncalibrated probability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldConfidence {
    Missing,
    Heuristic,
    Explicit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParsedField<T> {
    pub value: Option<T>,
    pub confidence: FieldConfidence,
}

impl<T> ParsedField<T> {
    const fn new(value: Option<T>, confidence: FieldConfidence) -> Self {
        Self {
            confidence: if value.is_some() {
                confidence
            } else {
                FieldConfidence::Missing
            },
            value,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BibliographicFields {
    pub printed_key: ParsedField<String>,
    /// Printed author strings; no name-order guess or identity resolution.
    pub authors: ParsedField<Vec<String>>,
    pub title: ParsedField<String>,
    /// Preserve disambiguating suffixes such as `2020a`.
    pub year: ParsedField<String>,
    pub venue: ParsedField<String>,
    /// Original identifier casing and punctuation, without URL/prefix wrappers.
    pub doi: ParsedField<String>,
    /// Retain a printed arXiv version; Work-level normalization happens later.
    pub arxiv_id: ParsedField<String>,
}

/// An author/date prefix accepts both parenthesized and plain Harvard years.
pub(super) fn author_year_prefix(text: &str) -> Option<(usize, usize)> {
    let year = year_regex().find(text)?;
    let prefix = text[..year.start()].trim_end();
    let start = prefix
        .strip_suffix(['(', '['])
        .map_or_else(|| year.start(), str::len);
    let authors = text[..start].trim();
    let end = year.end() + usize::from(text[year.end()..].starts_with([')', ']']));
    (start < 250
        && authors.chars().next().is_some_and(char::is_alphabetic)
        && !authors.contains(['/', ':'])
        && sentence_boundary(authors).is_none())
    .then_some((start, end))
}

pub(super) fn sentence_boundary(text: &str) -> Option<usize> {
    for (at, ch) in text.char_indices() {
        if ch != '.' || !text[at + 1..].starts_with(char::is_whitespace) {
            continue;
        }
        let prefix = &text[..at];
        let last = prefix
            .rsplit(|ch: char| ch.is_whitespace() || ch == ',')
            .next()
            .unwrap_or_default();
        // Author initials and et al. are not author/title boundaries.
        if last
            .chars()
            .all(|ch| ch.is_alphabetic() || ch == '-' || ch == '.')
            && (last.chars().filter(|ch| ch.is_alphabetic()).count() == 1
                || last.contains('.')
                || last.contains('-'))
        {
            continue;
        }
        if last == "al" && prefix.ends_with("et al") {
            continue;
        }
        return Some(at);
    }
    None
}

#[must_use]
pub fn parse_fields(raw: &str, printed_key: Option<String>) -> BibliographicFields {
    let stripped = label_regex()
        .find(raw)
        .filter(|label| label.start() == 0)
        .map_or(raw, |label| &raw[label.end()..]);
    let compact = clean(stripped);
    let year = year_regex()
        .find_iter(&compact)
        .find(|year| {
            let token = compact[..year.start()]
                .rsplit(char::is_whitespace)
                .next()
                .unwrap_or_default();
            !token.contains(['/', ':']) && !token.ends_with('.')
        })
        .map(|year| year.as_str().to_owned());
    let (author_text, remaining) = if let Some((start, end)) = author_year_prefix(&compact) {
        (&compact[..start], &compact[end..])
    } else if let Some(end) = sentence_boundary(&compact) {
        (&compact[..end], &compact[end + 1..])
    } else {
        ("", compact.as_str())
    };
    let authors = split_authors(author_text);
    let remaining =
        remaining.trim_start_matches(|ch: char| ch.is_whitespace() || ".,;:".contains(ch));
    let (title, venue) = title_venue(remaining);
    BibliographicFields {
        printed_key: ParsedField::new(printed_key, FieldConfidence::Explicit),
        authors: ParsedField::new(
            (!authors.is_empty()).then_some(authors),
            FieldConfidence::Heuristic,
        ),
        title: ParsedField::new(title, FieldConfidence::Heuristic),
        year: ParsedField::new(year, FieldConfidence::Heuristic),
        venue: ParsedField::new(venue, FieldConfidence::Heuristic),
        doi: ParsedField::new(extract_doi(raw), FieldConfidence::Explicit),
        arxiv_id: ParsedField::new(extract_arxiv(raw), FieldConfidence::Explicit),
    }
}

fn split_authors(text: &str) -> Vec<String> {
    let text = text.trim().trim_end_matches([',', ';']);
    if text.is_empty() {
        return vec![];
    }
    // Keep surname, initials together. Semicolon / and delimit explicit authors;
    // comma-style lists are paired only when the following item is initials.
    let mut result = Vec::new();
    for item in pattern!(r"\s+(?:and|&)\s+|\s*;\s*").split(text) {
        let pieces: Vec<_> = item
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        let mut at = 0;
        while at < pieces.len() {
            let first = pieces[at];
            if let Some(initials) = pieces
                .get(at + 1)
                .filter(|part| pattern!(r"^(?:\p{Lu}[.\s-]*)+$").is_match(part))
            {
                result.push(format!("{first}, {initials}"));
                at += 2;
            } else {
                result.push(first.to_owned());
                at += 1;
            }
        }
    }
    result
}

fn title_venue(text: &str) -> (Option<String>, Option<String>) {
    if text.is_empty() {
        return (None, None);
    }
    if let Some(open) = text
        .chars()
        .next()
        .filter(|ch| matches!(ch, '"' | '“' | '‘'))
    {
        let close = match open {
            '“' => '”',
            '‘' => '’',
            _ => '"',
        };
        if let Some(end) = text[open.len_utf8()..].find(close) {
            let end = open.len_utf8() + end;
            return (
                trimmed(&text[open.len_utf8()..end]),
                clean_venue(&text[end + close.len_utf8()..]),
            );
        }
    }
    let period = sentence_boundary(text).unwrap_or(text.len());
    let terminal = text.char_indices().find_map(|(at, ch)| {
        (matches!(ch, '?' | '!')
            && (at + 1 == text.len() || text[at + 1..].starts_with(char::is_whitespace)))
        .then_some(at)
    });
    // Question/exclamation punctuation belongs to the title. The separating
    // period follows the existing convention of being omitted from the field.
    let (end, remaining) = terminal.filter(|at| *at < period).map_or_else(
        || (period, text.get(period + 1..).unwrap_or_default()),
        |at| (at + 1, &text[at + 1..]),
    );
    let title = &text[..end];
    if !title.chars().any(char::is_alphabetic)
        || pattern!(r"(?i)^(?:doi\s*:|https?://|arxiv\s*:)").is_match(title)
    {
        return (None, None);
    }
    (trimmed(title), clean_venue(remaining))
}

fn clean_venue(text: &str) -> Option<String> {
    let cutoff = pattern!(r"(?i)\b(?:doi\s*:|https?://|arxiv\s*:)|\b(?:18|19|20)\d{2}[a-z]?\b")
        .find(text)
        .map_or(text.len(), |matched| matched.start());
    trimmed(text[..cutoff].trim_end_matches(|ch: char| ch.is_whitespace() || "([".contains(ch)))
}

fn extract_doi(raw: &str) -> Option<String> {
    let mut identifiers: BTreeSet<_> = pattern!(r"(?i)\b10\.\d{4,9}/")
        .find_iter(raw)
        .filter_map(|matched| read_doi(&raw[matched.start()..]))
        .collect();
    (identifiers.len() == 1)
        .then(|| identifiers.pop_first())
        .flatten()
}

fn read_doi(raw: &str) -> Option<String> {
    let mut token = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\n' || ch == '\r' {
            // Only join physical line breaks. Ordinary spaces terminate IDs;
            // preserve all identifier characters, including a printed hyphen.
            while chars
                .peek()
                .is_some_and(|next| matches!(next, ' ' | '\t' | '\r' | '\n'))
            {
                chars.next();
            }
            if token.ends_with(['.', ',', ';']) {
                break;
            }
            // A slash/hyphen or numeric continuation supports joining a wrapped
            // identifier. A following prose line is a boundary. An otherwise
            // ambiguous bare-word continuation does not justify an explicit ID.
            if !token.ends_with(['/', '-', '_']) && !chars.peek().is_some_and(char::is_ascii_digit)
            {
                let remainder = chars.clone().collect::<String>();
                let next_line = remainder.lines().next().unwrap_or_default().trim();
                if next_line.split_whitespace().count() > 1 || next_line.is_empty() {
                    break;
                }
                return None;
            }
            continue;
        }
        if ch.is_whitespace() || matches!(ch, '"' | '“' | '”') {
            break;
        }
        token.push(ch);
    }
    trim_identifier(&mut token);
    if token.to_ascii_lowercase().starts_with("10.48550/arxiv.") {
        let suffix = &token["10.48550/arxiv.".len()..];
        // This DOI namespace encodes an arXiv identifier. A line break after
        // the dot can leave a syntactically plausible generic DOI prefix that
        // is not a complete arXiv identity. Withhold it instead of inventing
        // an explicit truncated DOI; the original entry remains available.
        if extract_arxiv(&format!("arxiv:{suffix}")).as_deref() != Some(suffix) {
            return None;
        }
    }
    (token
        .split_once('/')
        .is_some_and(|(_, suffix)| !suffix.is_empty()))
    .then_some(token)
}

fn trim_identifier(value: &mut String) {
    while let Some(last) = value.chars().last() {
        let unmatched = matches!(last, ')' | ']' | '}') && {
            let open = match last {
                ')' => '(',
                ']' => '[',
                _ => '{',
            };
            value.matches(last).count() > value.matches(open).count()
        };
        if !(".,;:".contains(last) || unmatched) {
            break;
        }
        value.truncate(value.len() - last.len_utf8());
    }
}

fn extract_arxiv(raw: &str) -> Option<String> {
    let mut identifiers: BTreeSet<_> = pattern!(r"(?i)(?:arxiv\s*:\s*|arxiv\.org/(?:abs|pdf)/)((?:\d{4}\.\d{4,5}|[a-z][a-z.\-]+/\d{7})(?:v\d+)?)(?:\.pdf)?\b")
        .captures_iter(raw).filter_map(|capture| capture.get(1).map(|matched| matched.as_str().to_owned())).collect();
    (identifiers.len() == 1)
        .then(|| identifiers.pop_first())
        .flatten()
}
