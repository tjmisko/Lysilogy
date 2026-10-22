//! A printed reference string interpreted as a structured, fallible record.

use lysilogy_core::names::citation_family;
use lysilogy_core::{IdentifierScheme, identifier, title_key};
use serde::{Deserialize, Serialize};

use crate::bibliography::{BibliographicFields, parse_fields};

/// One identifier carried by a reference or a candidate, already normalized.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceIdentifier {
    pub scheme: IdentifierScheme,
    pub value: String,
}

/// Whether an identifier names a Work or one of its manifestation versions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierLevel {
    Work,
    Version,
}

/// Classify an identifier scheme by what it names.
///
/// A DOI, `arXiv` id, PMID, ISBN or URL names a manifestation. `OpenAlex` and
/// Semantic Scholar ids are Work anchors.
#[must_use]
pub const fn identifier_level(scheme: IdentifierScheme) -> IdentifierLevel {
    match scheme {
        IdentifierScheme::Openalex | IdentifierScheme::SemanticScholar => IdentifierLevel::Work,
        IdentifierScheme::Doi
        | IdentifierScheme::Arxiv
        | IdentifierScheme::Pmid
        | IdentifierScheme::Isbn
        | IdentifierScheme::Url => IdentifierLevel::Version,
    }
}

/// The structured interpretation of one printed reference.
///
/// The original string is retained; every field may be missing or wrong. The
/// record is a query and a comparison key, never an identity claim.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceRecord {
    /// The printed text this record was read from.
    pub raw: String,
    /// The printed citation key or label, when the entry had one.
    pub printed_key: Option<String>,
    pub title: Option<String>,
    /// Printed author strings in printed order; no identity resolution.
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    /// Normalized DOIs and `arXiv` IDs, deduplicated in first-seen order.
    pub identifiers: Vec<ReferenceIdentifier>,
}

impl ReferenceRecord {
    /// Read a structured record from a raw printed reference string.
    #[must_use]
    pub fn from_raw(raw: &str) -> Self {
        Self::from_fields(raw, &parse_fields(raw, None))
    }

    /// Read a structured record from already-parsed fields.
    #[must_use]
    pub fn from_fields(raw: &str, fields: &BibliographicFields) -> Self {
        Self {
            raw: raw.to_owned(),
            printed_key: fields.printed_key.value.clone(),
            title: clean_optional(fields.title.value.as_deref()),
            authors: fields.authors.value.clone().unwrap_or_default(),
            year: fields.year.value.as_deref().and_then(parse_year),
            venue: clean_optional(fields.venue.value.as_deref()),
            identifiers: identifiers_from(raw, fields),
        }
    }

    /// The normalized identifier for a scheme, if this record has one.
    #[must_use]
    pub fn identifier(&self, scheme: IdentifierScheme) -> Option<&str> {
        self.identifiers
            .iter()
            .find(|identifier| identifier.scheme == scheme)
            .map(|identifier| identifier.value.as_str())
    }

    /// Author family names, folded for comparison.
    #[must_use]
    pub fn author_families(&self) -> Vec<String> {
        author_families(&self.authors)
    }

    /// Work-level anchors (`OpenAlex`, Semantic Scholar).
    #[must_use]
    pub fn work_identifiers(&self) -> Vec<&ReferenceIdentifier> {
        self.identifiers
            .iter()
            .filter(|identifier| identifier_level(identifier.scheme) == IdentifierLevel::Work)
            .collect()
    }

    /// Version-level identifiers (DOI, `arXiv`, PMID, ISBN, URL).
    #[must_use]
    pub fn version_identifiers(&self) -> Vec<&ReferenceIdentifier> {
        self.identifiers
            .iter()
            .filter(|identifier| identifier_level(identifier.scheme) == IdentifierLevel::Version)
            .collect()
    }

    /// The family name of the first printed author, folded for comparison.
    #[must_use]
    pub fn first_author_family(&self) -> Option<String> {
        self.author_families().into_iter().next()
    }
}

/// Fold a printed author list into comparable family names.
pub fn author_families(authors: &[String]) -> Vec<String> {
    authors
        .iter()
        .filter_map(|author| citation_family(author))
        .map(|family| title_key(&family))
        .filter(|family| !family.is_empty())
        .collect()
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// The leading four-digit year of a printed year such as `2020a`.
fn parse_year(printed: &str) -> Option<i32> {
    let digits: String = printed
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (digits.len() == 4).then(|| digits.parse().ok()).flatten()
}

fn identifiers_from(raw: &str, fields: &BibliographicFields) -> Vec<ReferenceIdentifier> {
    let mut found: Vec<ReferenceIdentifier> = Vec::new();
    let mut push = |scheme, value: String| {
        if !found
            .iter()
            .any(|seen| seen.scheme == scheme && seen.value == value)
        {
            found.push(ReferenceIdentifier { scheme, value });
        }
    };
    for (scheme, value) in [
        (IdentifierScheme::Doi, fields.doi.value.as_deref()),
        (IdentifierScheme::Arxiv, fields.arxiv_id.value.as_deref()),
    ] {
        if let Some(normalized) = value.and_then(|value| identifier::normalize(scheme, value)) {
            push(scheme, normalized);
        }
    }
    for detected in identifier::extract(raw) {
        push(detected.scheme, detected.value);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_read_year_and_authors_from_a_printed_entry() {
        let record = ReferenceRecord::from_raw(
            "Autor, D. H., Dorn, D., and Hanson, G. H. The China Syndrome. \
             American Economic Review, 2013.",
        );
        assert_eq!(record.year, Some(2013));
        assert_eq!(
            record.author_families().first().map(String::as_str),
            Some("autor")
        );
    }

    #[test]
    fn should_read_normalized_identifiers_from_the_raw_string() {
        let record = ReferenceRecord::from_raw(
            "Smith, J. A Title. Journal. doi:10.1234/ABC.def arXiv:2010.00671v3",
        );
        assert_eq!(
            record.identifier(IdentifierScheme::Doi),
            Some("10.1234/abc.def")
        );
        assert_eq!(
            record.identifier(IdentifierScheme::Arxiv),
            Some("2010.00671")
        );
    }

    #[test]
    fn should_parse_year_with_a_disambiguating_suffix() {
        assert_eq!(parse_year("2020a"), Some(2020));
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year("n.d."), None);
        assert_eq!(parse_year("123"), None);
    }

    #[test]
    fn should_drop_blank_title_and_venue() {
        let mut fields = parse_fields("", None);
        fields.title.value = Some("   ".to_owned());
        fields.venue.value = Some(String::new());
        let record = ReferenceRecord::from_fields("", &fields);
        assert_eq!(record.title, None);
        assert_eq!(record.venue, None);
    }
}
