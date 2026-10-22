//! A registry record proposed for a printed reference.
//!
//! A registry record names a *manifestation* (a version), not a Work. Version
//! identifiers such as DOI and `arXiv` are therefore kept separate from Work
//! anchors such as an `OpenAlex` or Semantic Scholar id.

use lysilogy_core::{IdentifierScheme, VersionKind};
use serde::{Deserialize, Serialize};

use super::record::{IdentifierLevel, ReferenceIdentifier, author_families, identifier_level};

/// One registry record proposed for a printed reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CandidateRecord {
    /// Registry name, e.g. `openalex`, `crossref`, `semantic_scholar`.
    pub source: String,
    /// Registry-specific record identity.
    pub source_id: String,
    pub title: Option<String>,
    /// Author names as the registry reports them.
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    /// Identifiers carried by this record, both Work anchors and version ids.
    pub identifiers: Vec<ReferenceIdentifier>,
    pub version_kind: VersionKind,
    pub version_label: Option<String>,
    /// Optional tie-break signal; higher means more cited.
    pub cited_by_count: Option<u64>,
}

impl CandidateRecord {
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
}
