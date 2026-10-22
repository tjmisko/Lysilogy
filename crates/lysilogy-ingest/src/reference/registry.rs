//! The registry seam: search external catalogues for a printed reference and
//! combine their candidates.
//!
//! Registries are searched independently. A failure is recorded per registry
//! and never discards the others. Candidates that share a *version* identifier
//! (DOI, arXiv, PMID) are the same manifestation record returned twice, so they
//! are merged; distinct versions are kept separate.

use std::future::Future;
use std::pin::Pin;

use lysilogy_core::VersionKind;
use serde::{Deserialize, Serialize};

use super::candidate::CandidateRecord;
use super::record::ReferenceRecord;
use super::score::{MatchDecision, decide};

/// A boxed registry future, matching the provider transport style.
pub type RegistryFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Which query fields a registry can use.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegistryCapabilities {
    pub identifiers: bool,
    pub title: bool,
    pub authors: bool,
    pub year: bool,
    pub venue: bool,
}

impl RegistryCapabilities {
    /// Every supported query field.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            identifiers: true,
            title: true,
            authors: true,
            year: true,
            venue: true,
        }
    }
}

/// A registry search failure.
#[derive(Clone, Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("{registry} search failed: {message}")]
    Search { registry: String, message: String },
    #[error("{registry} returned an unusable response")]
    Malformed { registry: String },
}

/// An external catalogue that can resolve a partial reference query.
pub trait ReferenceRegistry: Send + Sync {
    /// Stable registry name, recorded on every candidate and error.
    fn name(&self) -> &'static str;
    /// Which query fields this registry can use.
    fn capabilities(&self) -> RegistryCapabilities;
    /// Search for candidates for one printed reference.
    fn search<'a>(
        &'a self,
        record: &'a ReferenceRecord,
    ) -> RegistryFuture<'a, Result<Vec<CandidateRecord>, RegistryError>>;
}

/// What one registry returned, including its error when it failed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegistrySearch {
    pub registry: String,
    pub candidates: Vec<CandidateRecord>,
    pub error: Option<String>,
}

/// The combined result of searching several registries for one reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    pub searches: Vec<RegistrySearch>,
    /// Deduplicated candidates, in first-seen order.
    pub candidates: Vec<CandidateRecord>,
    pub decision: MatchDecision,
}

/// Search every registry, merge duplicate manifestations, and decide.
pub async fn resolve(
    registries: &[&dyn ReferenceRegistry],
    record: &ReferenceRecord,
) -> Resolution {
    let mut searches = Vec::with_capacity(registries.len());
    let mut all = Vec::new();
    for registry in registries {
        match registry.search(record).await {
            Ok(candidates) => {
                all.extend(candidates.iter().cloned());
                searches.push(RegistrySearch {
                    registry: registry.name().to_owned(),
                    candidates,
                    error: None,
                });
            }
            Err(error) => searches.push(RegistrySearch {
                registry: registry.name().to_owned(),
                candidates: Vec::new(),
                error: Some(error.to_string()),
            }),
        }
    }
    let candidates = dedup_versions(all);
    let decision = decide(record, &candidates);
    Resolution {
        searches,
        candidates,
        decision,
    }
}

/// Merge candidates that share a version identifier.
///
/// Only version-level identifiers are used. Work anchors are deliberately not:
/// one Work can contain several versions, and merging on a Work anchor would
/// silently collapse them.
#[must_use]
pub fn dedup_versions(candidates: Vec<CandidateRecord>) -> Vec<CandidateRecord> {
    let mut merged: Vec<CandidateRecord> = Vec::new();
    for candidate in candidates {
        let same = merged
            .iter()
            .position(|existing| shares_version_identifier(existing, &candidate));
        match same {
            Some(index) => merge_into(&mut merged[index], candidate),
            None => merged.push(candidate),
        }
    }
    merged
}

fn shares_version_identifier(left: &CandidateRecord, right: &CandidateRecord) -> bool {
    left.version_identifiers().iter().any(|left| {
        right
            .version_identifiers()
            .iter()
            .any(|right| left.scheme == right.scheme && left.value == right.value)
    })
}

fn merge_into(existing: &mut CandidateRecord, incoming: CandidateRecord) {
    let mut identifiers = existing.identifiers.clone();
    for identifier in &incoming.identifiers {
        if !identifiers.contains(identifier) {
            identifiers.push(identifier.clone());
        }
    }
    let version_kind = existing.version_kind;
    let version_label = existing.version_label.clone();
    let cited_by_count = existing.cited_by_count;
    if completeness(&incoming) > completeness(existing) {
        *existing = incoming;
    }
    if existing.version_kind == VersionKind::Unknown {
        existing.version_kind = version_kind;
    }
    if existing.version_label.is_none() {
        existing.version_label = version_label;
    }
    existing.identifiers = identifiers;
    existing.cited_by_count = existing.cited_by_count.max(cited_by_count);
}

fn completeness(candidate: &CandidateRecord) -> usize {
    usize::from(candidate.title.is_some())
        + usize::from(!candidate.authors.is_empty())
        + usize::from(candidate.year.is_some())
        + usize::from(candidate.venue.is_some())
        + candidate.identifiers.len()
        + usize::from(candidate.version_kind != VersionKind::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::record::{IdentifierLevel, ReferenceIdentifier};
    use lysilogy_core::{IdentifierScheme, VersionKind};

    struct FakeRegistry {
        name: &'static str,
        candidates: Vec<CandidateRecord>,
        fail: bool,
    }

    impl ReferenceRegistry for FakeRegistry {
        fn name(&self) -> &'static str {
            self.name
        }

        fn capabilities(&self) -> RegistryCapabilities {
            RegistryCapabilities::all()
        }

        fn search<'a>(
            &'a self,
            _record: &'a ReferenceRecord,
        ) -> RegistryFuture<'a, Result<Vec<CandidateRecord>, RegistryError>> {
            let outcome = if self.fail {
                Err(RegistryError::Search {
                    registry: self.name.to_owned(),
                    message: "boom".to_owned(),
                })
            } else {
                Ok(self.candidates.clone())
            };
            Box::pin(async move { outcome })
        }
    }

    fn record() -> ReferenceRecord {
        ReferenceRecord {
            raw: String::new(),
            printed_key: None,
            title: Some("A Study of Things".to_owned()),
            authors: vec!["Smith, J.".to_owned()],
            year: Some(2010),
            venue: None,
            identifiers: Vec::new(),
        }
    }

    fn candidate(source: &str, doi: &str, kind: VersionKind) -> CandidateRecord {
        CandidateRecord {
            source: source.to_owned(),
            source_id: format!("{source}-1"),
            title: Some("A Study of Things".to_owned()),
            authors: vec!["J. Smith".to_owned()],
            year: Some(2010),
            venue: None,
            identifiers: vec![ReferenceIdentifier {
                scheme: IdentifierScheme::Doi,
                value: doi.to_owned(),
            }],
            version_kind: kind,
            version_label: None,
            cited_by_count: None,
        }
    }

    #[tokio::test]
    async fn should_merge_the_same_version_from_two_registries() {
        let record = record();
        let openalex = FakeRegistry {
            name: "openalex",
            candidates: vec![candidate("openalex", "10.1234/aaa", VersionKind::Published)],
            fail: false,
        };
        let crossref = FakeRegistry {
            name: "crossref",
            candidates: vec![candidate("crossref", "10.1234/aaa", VersionKind::Published)],
            fail: false,
        };
        let result = resolve(&[&openalex, &crossref], &record).await;
        assert_eq!(result.candidates.len(), 1);
        assert!(matches!(result.decision, MatchDecision::Accepted { .. }));
    }

    #[tokio::test]
    async fn should_keep_distinct_versions_separate() {
        let record = record();
        let openalex = FakeRegistry {
            name: "openalex",
            candidates: vec![candidate(
                "openalex",
                "10.1234/preprint",
                VersionKind::Preprint,
            )],
            fail: false,
        };
        let crossref = FakeRegistry {
            name: "crossref",
            candidates: vec![candidate(
                "crossref",
                "10.1234/published",
                VersionKind::Published,
            )],
            fail: false,
        };
        let result = resolve(&[&openalex, &crossref], &record).await;
        assert_eq!(result.candidates.len(), 2);
    }

    #[tokio::test]
    async fn should_keep_results_when_one_registry_fails() {
        let record = record();
        let working = FakeRegistry {
            name: "openalex",
            candidates: vec![candidate("openalex", "10.1234/aaa", VersionKind::Published)],
            fail: false,
        };
        let broken = FakeRegistry {
            name: "crossref",
            candidates: Vec::new(),
            fail: true,
        };
        let result = resolve(&[&working, &broken], &record).await;
        assert!(matches!(result.decision, MatchDecision::Accepted { .. }));
        assert_eq!(result.searches.len(), 2);
        assert!(result.searches[1].error.is_some());
    }

    #[test]
    fn should_classify_doi_as_a_version_identifier() {
        use crate::reference::record::identifier_level;
        assert_eq!(
            identifier_level(IdentifierScheme::Doi),
            IdentifierLevel::Version
        );
        assert_eq!(
            identifier_level(IdentifierScheme::Arxiv),
            IdentifierLevel::Version
        );
        assert_eq!(
            identifier_level(IdentifierScheme::Openalex),
            IdentifierLevel::Work
        );
        assert_eq!(
            identifier_level(IdentifierScheme::SemanticScholar),
            IdentifierLevel::Work
        );
    }
}
