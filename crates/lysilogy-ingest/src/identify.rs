//! Choosing which detected identifier names a paper.

use lysilogy_core::{DetectedIdentifier, IdentifierScheme};

/// Optional bibliographic hints taken from a paper's own extracted metadata.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaperHints<'a> {
    pub title: Option<&'a str>,
    pub year: Option<i32>,
    pub first_author: Option<&'a str>,
    /// Every author name the paper's metadata lists, in order.
    pub authors: &'a [String],
}

/// Prefer a DOI, then an arXiv ID, then whatever was found first.
#[must_use]
pub fn primary_identifier(detected: &[DetectedIdentifier]) -> Option<DetectedIdentifier> {
    detected
        .iter()
        .find(|identifier| identifier.scheme == IdentifierScheme::Doi)
        .or_else(|| {
            detected
                .iter()
                .find(|identifier| identifier.scheme == IdentifierScheme::Arxiv)
        })
        .or_else(|| detected.first())
        .cloned()
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
    fn should_prefer_doi_when_arxiv_is_listed_first() {
        let found = [
            detected(IdentifierScheme::Arxiv, "2104.01511"),
            detected(IdentifierScheme::Doi, "10.5555/x"),
        ];
        let primary = primary_identifier(&found).expect("primary");
        assert_eq!(primary.scheme, IdentifierScheme::Doi);
    }

    #[test]
    fn should_return_none_when_nothing_detected() {
        assert!(primary_identifier(&[]).is_none());
    }
}
