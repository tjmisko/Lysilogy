//! Registry adapters for `OpenAlex`, Crossref, and Semantic Scholar.
//!
//! Every adapter returns [`CandidateRecord`] values: a registry record is a
//! manifestation, so a DOI or `arXiv` id is a version identifier and an
//! `OpenAlex` or Semantic Scholar id is a Work anchor.

use lysilogy_core::{IdentifierScheme, VersionKind, identifier};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use crate::crossref::parse_crossref_work;
use crate::openalex::{
    JsonTransport, OpenAlexClient, OpenAlexSummary, ProviderError, parse_summary, title_search_url,
};

use super::candidate::CandidateRecord;
use super::record::{ReferenceIdentifier, ReferenceRecord};
use super::registry::{ReferenceRegistry, RegistryCapabilities, RegistryError, RegistryFuture};

/// How many candidates each search asks for.
pub const SEARCH_ROWS: usize = 10;

/// The longest printed string sent as a bibliographic query.
const QUERY_MAX_CHARS: usize = 512;

/// `OpenAlex` title search. Uses the title and year fields.
pub struct OpenAlexRegistry<T> {
    client: OpenAlexClient<T>,
}

impl<T: JsonTransport> OpenAlexRegistry<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            client: OpenAlexClient::new(transport),
        }
    }
}

impl<T: JsonTransport> ReferenceRegistry for OpenAlexRegistry<T> {
    fn name(&self) -> &'static str {
        "openalex"
    }

    fn capabilities(&self) -> RegistryCapabilities {
        RegistryCapabilities {
            identifiers: false,
            title: true,
            authors: false,
            year: true,
            venue: false,
        }
    }

    fn search<'a>(
        &'a self,
        record: &'a ReferenceRecord,
    ) -> RegistryFuture<'a, Result<Vec<CandidateRecord>, RegistryError>> {
        Box::pin(async move {
            let Some(title) = record.title.as_deref() else {
                return Ok(Vec::new());
            };
            let Some(url) = title_search_url(title, record.year) else {
                return Ok(Vec::new());
            };
            let value = self
                .client
                .fetch_json(&url)
                .await
                .map_err(|error| registry_error("openalex", &error))?;
            Ok(parse_openalex_candidates(&value))
        })
    }
}

/// Crossref bibliographic search. Sends the printed string as the query.
pub struct CrossrefRegistry<T> {
    client: OpenAlexClient<T>,
}

impl<T: JsonTransport> CrossrefRegistry<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            client: OpenAlexClient::new(transport),
        }
    }
}

impl<T: JsonTransport> ReferenceRegistry for CrossrefRegistry<T> {
    fn name(&self) -> &'static str {
        "crossref"
    }

    fn capabilities(&self) -> RegistryCapabilities {
        RegistryCapabilities::all()
    }

    fn search<'a>(
        &'a self,
        record: &'a ReferenceRecord,
    ) -> RegistryFuture<'a, Result<Vec<CandidateRecord>, RegistryError>> {
        Box::pin(async move {
            let Some(url) = crossref_search_url(&query_text(record)) else {
                return Ok(Vec::new());
            };
            let value = self
                .client
                .fetch_json(&url)
                .await
                .map_err(|error| registry_error("crossref", &error))?;
            Ok(parse_crossref_candidates(&value))
        })
    }
}

/// Semantic Scholar paper search.
pub struct SemanticScholarRegistry<T> {
    client: OpenAlexClient<T>,
}

impl<T: JsonTransport> SemanticScholarRegistry<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            client: OpenAlexClient::new(transport),
        }
    }
}

impl<T: JsonTransport> ReferenceRegistry for SemanticScholarRegistry<T> {
    fn name(&self) -> &'static str {
        "semantic_scholar"
    }

    fn capabilities(&self) -> RegistryCapabilities {
        RegistryCapabilities {
            identifiers: false,
            title: true,
            authors: true,
            year: true,
            venue: true,
        }
    }

    fn search<'a>(
        &'a self,
        record: &'a ReferenceRecord,
    ) -> RegistryFuture<'a, Result<Vec<CandidateRecord>, RegistryError>> {
        Box::pin(async move {
            let Some(url) = semantic_scholar_search_url(&query_text(record)) else {
                return Ok(Vec::new());
            };
            let value = self
                .client
                .fetch_json(&url)
                .await
                .map_err(|error| registry_error("semantic_scholar", &error))?;
            Ok(parse_semantic_scholar_candidates(&value))
        })
    }
}

/// The printed string, falling back to the parsed title.
fn query_text(record: &ReferenceRecord) -> String {
    let raw = record.raw.trim();
    let text = if raw.is_empty() {
        record.title.as_deref().unwrap_or_default()
    } else {
        raw
    };
    text.chars().take(QUERY_MAX_CHARS).collect()
}

fn registry_error(registry: &str, error: &ProviderError) -> RegistryError {
    match error {
        ProviderError::Malformed => RegistryError::Malformed {
            registry: registry.to_owned(),
        },
        other => RegistryError::Search {
            registry: registry.to_owned(),
            message: other.to_string(),
        },
    }
}

/// Whether a DOI names an `arXiv` preprint rather than a published record.
fn is_arxiv_doi(doi: &str) -> bool {
    doi.starts_with("10.48550/arxiv.")
}

fn infer_version_kind(doi: Option<&str>, has_arxiv: bool) -> VersionKind {
    match doi {
        Some(doi) if is_arxiv_doi(doi) => VersionKind::Preprint,
        Some(_) => VersionKind::Published,
        None if has_arxiv => VersionKind::Preprint,
        None => VersionKind::Unknown,
    }
}

/// The `OpenAlex` search URL for a bibliographic query.
#[must_use]
pub fn crossref_search_url(query: &str) -> Option<String> {
    if query.trim().is_empty() {
        return None;
    }
    let encoded = utf8_percent_encode(query.trim(), NON_ALPHANUMERIC);
    Some(format!(
        "https://api.crossref.org/works?query.bibliographic={encoded}&rows={SEARCH_ROWS}\
         &select=DOI,title,author,issued,published,published-print,published-online,\
         container-title,type"
    ))
}

/// The Semantic Scholar search URL for a bibliographic query.
#[must_use]
pub fn semantic_scholar_search_url(query: &str) -> Option<String> {
    if query.trim().is_empty() {
        return None;
    }
    let encoded = utf8_percent_encode(query.trim(), NON_ALPHANUMERIC);
    Some(format!(
        "https://api.semanticscholar.org/graph/v1/paper/search?query={encoded}\
         &limit={SEARCH_ROWS}&fields=title,authors,year,venue,externalIds,citationCount"
    ))
}

/// Parse an `OpenAlex` search response.
#[must_use]
pub fn parse_openalex_candidates(value: &Value) -> Vec<CandidateRecord> {
    value
        .get("results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| parse_summary(item).ok())
                .map(openalex_candidate)
                .collect()
        })
        .unwrap_or_default()
}

fn openalex_candidate(summary: OpenAlexSummary) -> CandidateRecord {
    let version_kind = infer_version_kind(summary.doi.as_deref(), false);
    let mut identifiers = vec![ReferenceIdentifier {
        scheme: IdentifierScheme::Openalex,
        value: summary.openalex_id.clone(),
    }];
    if let Some(doi) = summary.doi {
        identifiers.push(ReferenceIdentifier {
            scheme: IdentifierScheme::Doi,
            value: doi,
        });
    }
    CandidateRecord {
        source: "openalex".to_owned(),
        source_id: summary.openalex_id,
        title: summary.title,
        authors: summary.authors,
        year: summary.year,
        venue: None,
        identifiers,
        version_kind,
        version_label: None,
        cited_by_count: None,
    }
}

/// Parse a Crossref search response.
#[must_use]
pub fn parse_crossref_candidates(value: &Value) -> Vec<CandidateRecord> {
    value
        .pointer("/message/items")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(crossref_candidate).collect())
        .unwrap_or_default()
}

fn crossref_candidate(item: &Value) -> Option<CandidateRecord> {
    let work = parse_crossref_work(item);
    let doi = work.doi?;
    let version_kind = match item.get("type").and_then(Value::as_str) {
        Some("posted-content") => VersionKind::Preprint,
        Some(_) => VersionKind::Published,
        None => VersionKind::Unknown,
    };
    Some(CandidateRecord {
        source: "crossref".to_owned(),
        source_id: doi.clone(),
        title: work.title,
        authors: work.authors,
        year: work.year,
        venue: work.venue,
        identifiers: vec![ReferenceIdentifier {
            scheme: IdentifierScheme::Doi,
            value: doi,
        }],
        version_kind,
        version_label: None,
        cited_by_count: None,
    })
}

/// Parse a Semantic Scholar search response.
#[must_use]
pub fn parse_semantic_scholar_candidates(value: &Value) -> Vec<CandidateRecord> {
    value
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(semantic_scholar_candidate)
                .collect()
        })
        .unwrap_or_default()
}

fn semantic_scholar_candidate(item: &Value) -> Option<CandidateRecord> {
    let paper_id = item
        .get("paperId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())?
        .to_owned();
    let mut identifiers = vec![ReferenceIdentifier {
        scheme: IdentifierScheme::SemanticScholar,
        value: paper_id.clone(),
    }];
    let mut doi = None;
    let mut has_arxiv = false;
    if let Some(external) = item.get("externalIds") {
        if let Some(value) = external.get("DOI").and_then(Value::as_str)
            && let Some(normalized) = identifier::normalize(IdentifierScheme::Doi, value)
        {
            doi = Some(normalized.clone());
            identifiers.push(ReferenceIdentifier {
                scheme: IdentifierScheme::Doi,
                value: normalized,
            });
        }
        if let Some(value) = external.get("ArXiv").and_then(Value::as_str)
            && let Some(normalized) = identifier::normalize(IdentifierScheme::Arxiv, value)
        {
            has_arxiv = true;
            identifiers.push(ReferenceIdentifier {
                scheme: IdentifierScheme::Arxiv,
                value: normalized,
            });
        }
        if let Some(value) = external.get("PubMed").and_then(Value::as_str) {
            identifiers.push(ReferenceIdentifier {
                scheme: IdentifierScheme::Pmid,
                value: value.to_owned(),
            });
        }
    }
    let version_kind = infer_version_kind(doi.as_deref(), has_arxiv);
    let authors = item
        .get("authors")
        .and_then(Value::as_array)
        .map(|authors| {
            authors
                .iter()
                .filter_map(|author| author.get("name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(CandidateRecord {
        source: "semantic_scholar".to_owned(),
        source_id: paper_id,
        title: item
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned),
        authors,
        year: item
            .get("year")
            .and_then(Value::as_i64)
            .and_then(|year| i32::try_from(year).ok()),
        venue: item
            .get("venue")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|venue| !venue.is_empty())
            .map(str::to_owned),
        identifiers,
        version_kind,
        version_label: None,
        cited_by_count: item.get("citationCount").and_then(Value::as_u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openalex::fake::RecordingTransport;
    use crate::reference::registry::{ReferenceRegistry, resolve};
    use serde_json::json;

    fn record() -> ReferenceRecord {
        ReferenceRecord {
            raw: "Smith, J. A Study of Things. Journal, 2010.".to_owned(),
            printed_key: None,
            title: Some("A Study of Things".to_owned()),
            authors: vec!["Smith, J.".to_owned()],
            year: Some(2010),
            venue: None,
            identifiers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn should_parse_openalex_candidates_with_work_and_version_ids() {
        let transport = RecordingTransport::new(|_url| {
            json!({
                "results": [{
                    "id": "https://openalex.org/W123",
                    "doi": "https://doi.org/10.1234/abc",
                    "display_name": "A Study of Things",
                    "publication_year": 2010,
                    "authorships": [{"author": {"display_name": "J. Smith"}}]
                }]
            })
        });
        let registry = OpenAlexRegistry::new(transport);
        let candidates = registry.search(&record()).await.expect("search");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source_id, "W123");
        assert_eq!(candidates[0].version_kind, VersionKind::Published);
        assert!(
            candidates[0]
                .work_identifiers()
                .iter()
                .any(|id| id.scheme == IdentifierScheme::Openalex)
        );
        assert!(
            candidates[0]
                .version_identifiers()
                .iter()
                .any(|id| id.scheme == IdentifierScheme::Doi)
        );
    }

    #[tokio::test]
    async fn should_send_a_crossref_bibliographic_query_and_parse_it() {
        let transport = RecordingTransport::new(|_url| {
            json!({
                "message": {"items": [{
                    "DOI": "10.1234/abc",
                    "title": ["A Study of Things"],
                    "author": [{"given": "J.", "family": "Smith"}],
                    "issued": {"date-parts": [[2010]]},
                    "container-title": ["Journal"],
                    "type": "journal-article"
                }]}
            })
        });
        let registry = CrossrefRegistry::new(transport);
        let candidates = registry.search(&record()).await.expect("search");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].year, Some(2010));
        assert_eq!(candidates[0].venue.as_deref(), Some("Journal"));
        assert_eq!(candidates[0].version_kind, VersionKind::Published);
        assert_eq!(candidates[0].version_identifiers().len(), 1);
    }

    #[tokio::test]
    async fn should_parse_semantic_scholar_candidates() {
        let transport = RecordingTransport::new(|_url| {
            json!({
                "data": [{
                    "paperId": "abc123",
                    "title": "A Study of Things",
                    "authors": [{"name": "J. Smith"}],
                    "year": 2010,
                    "venue": "Journal",
                    "externalIds": {"DOI": "10.1234/abc", "ArXiv": "2010.00671"},
                    "citationCount": 12
                }]
            })
        });
        let registry = SemanticScholarRegistry::new(transport);
        let candidates = registry.search(&record()).await.expect("search");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].cited_by_count, Some(12));
        assert!(
            candidates[0]
                .work_identifiers()
                .iter()
                .any(|id| id.scheme == IdentifierScheme::SemanticScholar)
        );
        assert!(
            candidates[0]
                .version_identifiers()
                .iter()
                .any(|id| id.scheme == IdentifierScheme::Arxiv)
        );
    }

    #[tokio::test]
    async fn should_collapse_the_same_doi_from_two_registries() {
        let record = record();
        let openalex = OpenAlexRegistry::new(RecordingTransport::new(|_url| {
            json!({"results": [{
                "id": "https://openalex.org/W1",
                "doi": "https://doi.org/10.1234/abc",
                "display_name": "A Study of Things",
                "publication_year": 2010
            }]})
        }));
        let crossref = CrossrefRegistry::new(RecordingTransport::new(|_url| {
            json!({"message": {"items": [{
                "DOI": "10.1234/abc",
                "title": ["A Study of Things"],
                "issued": {"date-parts": [[2010]]},
                "type": "journal-article"
            }]}})
        }));
        let result = resolve(&[&openalex, &crossref], &record).await;
        assert_eq!(result.candidates.len(), 1);
    }

    #[test]
    fn should_classify_arxiv_dois_as_preprints() {
        assert_eq!(
            infer_version_kind(Some("10.48550/arxiv.2010.00671"), false),
            VersionKind::Preprint
        );
        assert_eq!(
            infer_version_kind(Some("10.1234/abc"), false),
            VersionKind::Published
        );
        assert_eq!(infer_version_kind(None, true), VersionKind::Preprint);
        assert_eq!(infer_version_kind(None, false), VersionKind::Unknown);
    }

    #[test]
    fn should_omit_a_query_when_there_is_no_text() {
        assert_eq!(crossref_search_url("   "), None);
        assert_eq!(semantic_scholar_search_url(""), None);
    }
}
