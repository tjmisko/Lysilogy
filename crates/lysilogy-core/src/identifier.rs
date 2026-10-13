//! Detection and normalization of DOI and arXiv identifiers in text.
//!
//! Extraction is deliberately conservative: it recognizes the standard printed
//! forms (`10.x/y`, `arXiv:...`, `https://doi.org/...`) and normalizes them to a
//! stable key. Normalization for DOIs lowercases and strips resolver prefixes;
//! arXiv versions are dropped, because a version is not a Work-level identity.

use crate::state::IdentifierScheme;

/// An identifier found in text, already normalized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedIdentifier {
    pub scheme: IdentifierScheme,
    pub value: String,
}

/// Find every distinct DOI and arXiv identifier in `text`, in first-seen order.
#[must_use]
pub fn extract(text: &str) -> Vec<DetectedIdentifier> {
    let mut found: Vec<DetectedIdentifier> = Vec::new();
    for token in text.split_whitespace() {
        for scheme in [IdentifierScheme::Doi, IdentifierScheme::Arxiv] {
            if let Some(value) = detect(scheme, token)
                && !found
                    .iter()
                    .any(|seen| seen.scheme == scheme && seen.value == value)
            {
                found.push(DetectedIdentifier { scheme, value });
            }
        }
    }
    found
}

/// Normalize a raw identifier value for a scheme, or `None` if it is malformed.
#[must_use]
pub fn normalize(scheme: IdentifierScheme, raw: &str) -> Option<String> {
    match scheme {
        IdentifierScheme::Doi => normalize_doi(raw),
        IdentifierScheme::Arxiv => normalize_arxiv(raw),
        _ => None,
    }
}

fn detect(scheme: IdentifierScheme, token: &str) -> Option<String> {
    match scheme {
        IdentifierScheme::Doi => normalize_doi(&doi_in(token)?),
        IdentifierScheme::Arxiv => normalize_arxiv(&arxiv_in(token)?),
        _ => None,
    }
}

/// Extract the `10.x/y` DOI substring of a token, if present.
fn doi_in(token: &str) -> Option<String> {
    let mut offset = 0;
    while let Some(position) = token[offset..].find("10.") {
        let start = offset + position;
        let boundary_ok = token[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        if boundary_ok && doi_has_shape(&token[start..]) {
            return Some(trim_trailing_punctuation(&token[start..]).to_owned());
        }
        offset = start + 3;
    }
    None
}

/// Whether a slice begins with `10.` + 4–9 digits + `/`.
fn doi_has_shape(slice: &str) -> bool {
    let Some(rest) = slice.strip_prefix("10.") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (4..=9).contains(&digits) && rest.as_bytes().get(digits) == Some(&b'/')
}

fn normalize_doi(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('(').trim_start_matches('<');
    let stripped = [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ]
    .iter()
    .find_map(|prefix| {
        trimmed
            .to_ascii_lowercase()
            .strip_prefix(prefix)
            .map(str::to_owned)
    })
    .unwrap_or_else(|| trimmed.to_owned());
    let value = trim_trailing_punctuation(stripped.trim()).to_ascii_lowercase();
    if value.starts_with("10.") && value.contains('/') {
        Some(value)
    } else {
        None
    }
}

/// Extract the arXiv identifier following an `arXiv:` marker or `/abs/` URL.
fn arxiv_in(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    let start = lower
        .find("arxiv:")
        .map(|position| position + "arxiv:".len())
        .or_else(|| {
            lower
                .find("arxiv.org/abs/")
                .map(|position| position + "arxiv.org/abs/".len())
        })?;
    let candidate = trim_trailing_punctuation(&token[start..]);
    let base = strip_arxiv_version(candidate);
    is_valid_arxiv(base).then(|| base.to_owned())
}

fn normalize_arxiv(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let without_marker = trimmed
        .to_ascii_lowercase()
        .find("arxiv:")
        .map_or(trimmed, |position| &trimmed[position + "arxiv:".len()..]);
    let base = strip_arxiv_version(trim_trailing_punctuation(without_marker.trim()));
    is_valid_arxiv(base).then(|| base.to_ascii_lowercase())
}

/// Drop a trailing `vN` version suffix.
fn strip_arxiv_version(value: &str) -> &str {
    if let Some(position) = value.rfind('v') {
        let suffix = &value[position + 1..];
        let is_version = !suffix.is_empty()
            && suffix.chars().all(|character| character.is_ascii_digit())
            && value[..position]
                .chars()
                .any(|character| character == '.' || character == '/');
        if is_version {
            return &value[..position];
        }
    }
    value
}

fn is_valid_arxiv(value: &str) -> bool {
    if let Some((prefix, suffix)) = value.split_once('/') {
        return !prefix.is_empty()
            && suffix.len() >= 7
            && suffix.chars().all(|character| character.is_ascii_digit());
    }
    let Some((lead, tail)) = value.split_once('.') else {
        return false;
    };
    lead.len() == 4
        && lead.chars().all(|character| character.is_ascii_digit())
        && (4..=5).contains(&tail.len())
        && tail.chars().all(|character| character.is_ascii_digit())
}

/// Trim trailing prose punctuation, including unmatched closing brackets.
fn trim_trailing_punctuation(token: &str) -> &str {
    let mut end = token.len();
    while let Some(character) = token[..end].chars().next_back() {
        let plain = matches!(character, '.' | ',' | ';' | ':' | '"' | '\'' | '>' | '<');
        let unbalanced = match character {
            ')' => token[..end].matches(')').count() > token[..end].matches('(').count(),
            ']' => token[..end].matches(']').count() > token[..end].matches('[').count(),
            '}' => token[..end].matches('}').count() > token[..end].matches('{').count(),
            _ => false,
        };
        if plain || unbalanced {
            end -= character.len_utf8();
        } else {
            break;
        }
    }
    &token[..end]
}

/// Maximum length of [`front_matter`] output, in characters.
const FRONT_MATTER_MAX_CHARS: usize = 20_000;

/// Text of the first two pages, cut at a references heading, for identifier detection.
///
/// Identifiers in a bibliography belong to cited works, not the document itself, so
/// the text stops at the first line that is only a references heading (optionally
/// numbered, optionally followed by a colon). Output is capped on a char boundary.
#[must_use]
pub fn front_matter(pages: &[&str]) -> String {
    let joined = pages.iter().take(2).copied().collect::<Vec<_>>().join("\n");
    let mut kept = String::new();
    for line in joined.split('\n') {
        if is_references_heading(line) {
            break;
        }
        if !kept.is_empty() {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    if let Some((end, _)) = kept.char_indices().nth(FRONT_MATTER_MAX_CHARS) {
        kept.truncate(end);
    }
    kept
}

fn is_references_heading(line: &str) -> bool {
    let lowered = line.trim().to_lowercase();
    let without_number = lowered
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
        .trim_start();
    let heading = without_number.strip_suffix(':').unwrap_or(without_number);
    matches!(
        heading,
        "references" | "bibliography" | "works cited" | "literature cited"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected(scheme: IdentifierScheme, value: &str) -> DetectedIdentifier {
        DetectedIdentifier {
            scheme,
            value: value.to_owned(),
        }
    }

    #[test]
    fn finds_dois_in_prose() {
        let text = "See (10.1145/3292500.3330701). Also doi:10.1038/S41586-020-2649-2.";
        assert_eq!(
            extract(text),
            vec![
                detected(IdentifierScheme::Doi, "10.1145/3292500.3330701"),
                detected(IdentifierScheme::Doi, "10.1038/s41586-020-2649-2"),
            ]
        );
    }

    #[test]
    fn finds_arxiv_new_and_old_style_without_versions() {
        let text = "arXiv:2104.01511v2 and arXiv:math.AP/9912345.";
        assert_eq!(
            extract(text),
            vec![
                detected(IdentifierScheme::Arxiv, "2104.01511"),
                detected(IdentifierScheme::Arxiv, "math.ap/9912345"),
            ]
        );
    }

    #[test]
    fn finds_arxiv_urls() {
        assert_eq!(
            extract("available at https://arxiv.org/abs/1706.03762"),
            vec![detected(IdentifierScheme::Arxiv, "1706.03762")]
        );
    }

    #[test]
    fn deduplicates_repeats() {
        let text = "10.1234/abcd 10.1234/abcd arXiv:2104.01511 arXiv:2104.01511";
        assert_eq!(
            extract(text),
            vec![
                detected(IdentifierScheme::Doi, "10.1234/abcd"),
                detected(IdentifierScheme::Arxiv, "2104.01511"),
            ]
        );
    }

    #[test]
    fn ignores_non_identifiers() {
        let text = "11.12345/not-a-doi version 2104.0151 has v3";
        assert!(extract(text).is_empty());
    }

    #[test]
    fn does_not_mistake_numbers_for_dois() {
        assert!(extract("the value 110.1234/5 appears").is_empty());
    }

    #[test]
    fn normalization_strips_resolver_prefixes() {
        assert_eq!(
            normalize(IdentifierScheme::Doi, "https://doi.org/10.1038/NATURE14539"),
            Some("10.1038/nature14539".to_owned())
        );
        assert_eq!(
            normalize(IdentifierScheme::Arxiv, "arXiv:2104.01511v7"),
            Some("2104.01511".to_owned())
        );
        assert_eq!(normalize(IdentifierScheme::Doi, "not-a-doi"), None);
    }

    #[test]
    fn should_stop_front_matter_when_references_heading_appears() {
        let text = front_matter(&["Title\nDOI 10.1/abc\n  2. References:\n10.9/cited"]);
        assert_eq!(text, "Title\nDOI 10.1/abc");
        assert_eq!(front_matter(&["a\nBIBLIOGRAPHY\nb"]), "a");
        assert_eq!(front_matter(&["a\nWorks Cited\nb"]), "a");
        assert_eq!(front_matter(&["a\nLiterature cited\nb"]), "a");
    }

    #[test]
    fn should_keep_only_first_two_pages_when_given_many() {
        assert_eq!(front_matter(&["one", "two", "three"]), "one\ntwo");
    }

    #[test]
    fn should_ignore_references_word_inside_prose_when_not_a_heading() {
        let text = "See references below.\nreferences are listed\nend";
        assert_eq!(front_matter(&[text]), text);
    }

    #[test]
    fn should_return_empty_front_matter_when_no_pages() {
        assert_eq!(front_matter(&[]), "");
        assert_eq!(front_matter(&["References\nx"]), "");
    }

    #[test]
    fn should_truncate_on_char_boundary_when_front_matter_is_huge() {
        let huge = "é".repeat(30_000);
        let text = front_matter(&[&huge]);
        assert_eq!(text.chars().count(), 20_000);
        assert!(text.chars().all(|c| c == 'é'));
    }
}
