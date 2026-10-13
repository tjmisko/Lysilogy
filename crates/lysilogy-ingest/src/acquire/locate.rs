//! The deterministic locator: derive identifiers for a Work, ask `OpenAlex`
//! and Unpaywall where open-access copies live, and rank the candidate links.

use std::sync::LazyLock;

use lysilogy_core::identifier::normalize;
use lysilogy_core::{
    CandidateOrigin, Coverage, IdentifierScheme, IdentifierStatus, SourceRecordId, WorkIdentifier,
};
use lysilogy_db::acquisition::CandidateRecord;
use lysilogy_db::source_record::SourceRecordRepository;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use regex::Regex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::openalex::{JsonTransport, OpenAlexClient, ProviderError, source_key};

/// The `source_record.kind` for cached `OpenAlex` open-access lookups.
pub const OPENALEX_OA_KIND: &str = "openalex_oa";

/// The `source_record.kind` for cached Unpaywall lookups.
pub const UNPAYWALL_KIND: &str = "unpaywall";

const DATACITE_ARXIV_DOI_PREFIX: &str = "10.48550/arxiv.";

/// Percent-encode everything except unreserved characters and `/`.
const DOI_PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'.')
    .remove(b'-')
    .remove(b'_');

static NEW_STYLE_ARXIV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}\.\d{4,5}(v\d+)?$").expect("arxiv regex"));
static OLD_STYLE_ARXIV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z]+(-[a-z]+)*(\.[A-Za-z]{2})?/\d{7}(v\d+)?$").expect("arxiv regex")
});

/// The identifiers acquisition can search with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionIdentifiers {
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    pub openalex: Option<String>,
}

impl AcquisitionIdentifiers {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.doi.is_none() && self.arxiv.is_none() && self.openalex.is_none()
    }
}

/// A candidate link for a Work's open-access copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkCandidate {
    pub url: String,
    pub origin: CandidateOrigin,
    pub source_record_id: Option<SourceRecordId>,
}

impl From<LinkCandidate> for CandidateRecord {
    fn from(candidate: LinkCandidate) -> Self {
        Self {
            url: candidate.url,
            origin: candidate.origin,
            source_record_id: candidate.source_record_id,
        }
    }
}

/// The first active `doi`, `arxiv`, and `openalex` identifiers; an arXiv ID is
/// derived from a `DataCite` arXiv DOI when no explicit one exists.
#[must_use]
pub fn identifiers_from(identifiers: &[WorkIdentifier]) -> AcquisitionIdentifiers {
    let first = |scheme: IdentifierScheme| {
        identifiers
            .iter()
            .filter(|identifier| identifier.status == IdentifierStatus::Active)
            .filter(|identifier| identifier.scheme == scheme)
            .find_map(|identifier| match scheme {
                IdentifierScheme::Openalex => {
                    let value = identifier.value_normalized.trim();
                    (!value.is_empty()).then(|| value.to_ascii_uppercase())
                }
                _ => normalize(scheme, &identifier.value_normalized),
            })
    };
    let doi = first(IdentifierScheme::Doi);
    let arxiv = first(IdentifierScheme::Arxiv).or_else(|| {
        let suffix = doi.as_deref()?.strip_prefix(DATACITE_ARXIV_DOI_PREFIX)?;
        normalize(IdentifierScheme::Arxiv, suffix)
    });
    AcquisitionIdentifiers {
        doi,
        arxiv,
        openalex: first(IdentifierScheme::Openalex),
    }
}

/// The arXiv PDF URL for a well-formed new- or old-style identifier.
#[must_use]
pub fn arxiv_pdf_url(id: &str) -> Option<String> {
    let valid = NEW_STYLE_ARXIV.is_match(id) || OLD_STYLE_ARXIV.is_match(id);
    valid.then(|| format!("https://arxiv.org/pdf/{id}"))
}

/// The `OpenAlex` open-access lookup URL, preferring the `W...` ID over the DOI.
#[must_use]
pub fn openalex_oa_url(identifiers: &AcquisitionIdentifiers) -> Option<String> {
    let target = identifiers.openalex.as_ref().map_or_else(
        || {
            identifiers.doi.as_ref().map(|doi| {
                utf8_percent_encode(&format!("https://doi.org/{doi}"), NON_ALPHANUMERIC).to_string()
            })
        },
        |id| Some(id.clone()),
    )?;
    Some(format!(
        "https://api.openalex.org/works/{target}\
         ?select=id,doi,ids,open_access,best_oa_location,locations"
    ))
}

fn openalex_oa_key(identifiers: &AcquisitionIdentifiers) -> Option<String> {
    identifiers
        .openalex
        .as_deref()
        .map(source_key)
        .or_else(|| identifiers.doi.as_ref().map(|doi| format!("doi:{doi}")))
}

/// The Unpaywall lookup URL for a DOI and contact email.
#[must_use]
pub fn unpaywall_url(doi: &str, email: &str) -> String {
    format!(
        "https://api.unpaywall.org/v2/{}?email={}",
        utf8_percent_encode(doi, DOI_PATH),
        utf8_percent_encode(email, NON_ALPHANUMERIC)
    )
}

fn http_url(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    let parsed = Url::parse(text).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| text.to_owned())
}

fn candidate(
    url: Option<String>,
    origin: CandidateOrigin,
    record: Option<SourceRecordId>,
) -> Option<LinkCandidate> {
    Some(LinkCandidate {
        url: url?,
        origin,
        source_record_id: record,
    })
}

/// Candidates from an `OpenAlex` open-access response, in provider order per origin.
#[must_use]
pub fn parse_openalex_oa(value: &Value, record: Option<SourceRecordId>) -> Vec<LinkCandidate> {
    let mut found = Vec::new();
    let best = value.get("best_oa_location");
    found.extend(candidate(
        http_url(best.and_then(|best| best.get("pdf_url"))),
        CandidateOrigin::OpenalexBest,
        record,
    ));
    found.extend(candidate(
        http_url(value.pointer("/open_access/oa_url")),
        CandidateOrigin::OpenalexOaUrl,
        record,
    ));
    let locations: Vec<&Value> = value
        .get("locations")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("is_oa").and_then(Value::as_bool) == Some(true))
                .collect()
        })
        .unwrap_or_default();
    for location in &locations {
        found.extend(candidate(
            http_url(location.get("pdf_url")),
            CandidateOrigin::OpenalexLocation,
            record,
        ));
    }
    for location in best.into_iter().chain(locations) {
        found.extend(candidate(
            http_url(location.get("landing_page_url")),
            CandidateOrigin::OpenalexLanding,
            record,
        ));
    }
    found
}

/// Candidates from an Unpaywall response.
#[must_use]
pub fn parse_unpaywall(value: &Value, record: Option<SourceRecordId>) -> Vec<LinkCandidate> {
    let best = value
        .get("best_oa_location")
        .filter(|best| best.is_object());
    let locations: Vec<&Value> = value
        .get("oa_locations")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|item| item.is_object()).collect())
        .unwrap_or_default();
    let mut found = Vec::new();
    found.extend(candidate(
        http_url(best.and_then(|best| best.get("url_for_pdf"))),
        CandidateOrigin::UnpaywallBest,
        record,
    ));
    for location in &locations {
        found.extend(candidate(
            http_url(location.get("url_for_pdf")),
            CandidateOrigin::UnpaywallLocation,
            record,
        ));
    }
    for location in best.into_iter().chain(locations) {
        if http_url(location.get("url_for_pdf")).is_some() {
            continue;
        }
        found.extend(candidate(
            http_url(location.get("url")),
            CandidateOrigin::UnpaywallLanding,
            record,
        ));
    }
    found
}

/// A canonical form of a candidate URL used for deduplication and fetching.
///
/// Lowercases scheme and host, drops the fragment, upgrades `arxiv.org` to
/// https, and maps `arxiv.org/abs/<id>` (and `<id>.pdf`) to `arxiv.org/pdf/<id>`.
#[must_use]
pub fn normalize_candidate_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_owned();
    url.set_fragment(None);
    if matches!(host.as_str(), "arxiv.org" | "www.arxiv.org") {
        url.set_scheme("https").ok()?;
        url.set_host(Some("arxiv.org")).ok()?;
        let path = url.path().to_owned();
        let identifier = path
            .strip_prefix("/abs/")
            .or_else(|| path.strip_prefix("/pdf/"))
            .filter(|rest| !rest.is_empty());
        if let Some(identifier) = identifier {
            let identifier = identifier.strip_suffix(".pdf").unwrap_or(identifier);
            url.set_path(&format!("/pdf/{identifier}"));
        }
    }
    Some(url.to_string())
}

const fn origin_rank(origin: CandidateOrigin) -> u8 {
    match origin {
        CandidateOrigin::Arxiv => 0,
        CandidateOrigin::OpenalexBest => 1,
        CandidateOrigin::OpenalexOaUrl => 2,
        CandidateOrigin::OpenalexLocation => 3,
        CandidateOrigin::UnpaywallBest => 4,
        CandidateOrigin::UnpaywallLocation => 5,
        CandidateOrigin::OpenalexLanding => 6,
        CandidateOrigin::UnpaywallLanding => 7,
        CandidateOrigin::LandingMeta => 8,
        CandidateOrigin::Model => 9,
    }
}

/// Order candidates by origin rank (stable), normalize their URLs, and drop
/// duplicates, keeping the higher-ranked origin. Unusable URLs are dropped.
#[must_use]
pub fn rank_candidates(candidates: Vec<LinkCandidate>) -> Vec<LinkCandidate> {
    let mut normalized: Vec<LinkCandidate> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let url = normalize_candidate_url(&candidate.url)?;
            Some(LinkCandidate { url, ..candidate })
        })
        .collect();
    normalized.sort_by_key(|candidate| origin_rank(candidate.origin));
    let mut ranked: Vec<LinkCandidate> = Vec::with_capacity(normalized.len());
    for candidate in normalized {
        if ranked.iter().all(|kept| kept.url != candidate.url) {
            ranked.push(candidate);
        }
    }
    ranked
}

fn not_found_marker() -> Value {
    json!({ "not_found": true })
}

/// Fetch through the cache; a provider 404 is stored as the not-found marker.
async fn fetch_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    kind: &str,
    key: &str,
    url: &str,
    refresh: bool,
) -> Result<(SourceRecordId, Value), ProviderError> {
    if !refresh && let Some(hit) = records.find(kind, key).await? {
        return Ok(hit);
    }
    let value = match client.fetch_json(url).await {
        Ok(value) => value,
        Err(error) if error.is_not_found() => not_found_marker(),
        Err(error) => return Err(error),
    };
    let record_id = records
        .upsert(kind, key, Coverage::Complete, &value)
        .await?;
    Ok((record_id, value))
}

/// The cached `OpenAlex` open-access response for a Work, or `None` when it has
/// neither an `OpenAlex` ID nor a DOI. `refresh` bypasses the cache read.
pub async fn fetch_openalex_oa_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    identifiers: &AcquisitionIdentifiers,
    refresh: bool,
) -> Result<Option<(SourceRecordId, Value)>, ProviderError> {
    let (Some(url), Some(key)) = (openalex_oa_url(identifiers), openalex_oa_key(identifiers))
    else {
        return Ok(None);
    };
    fetch_cached(records, client, OPENALEX_OA_KIND, &key, &url, refresh)
        .await
        .map(Some)
}

/// The cached Unpaywall response for a DOI. `refresh` bypasses the cache read.
pub async fn fetch_unpaywall_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    doi: &str,
    email: &str,
    refresh: bool,
) -> Result<(SourceRecordId, Value), ProviderError> {
    let url = unpaywall_url(doi, email);
    fetch_cached(records, client, UNPAYWALL_KIND, doi, &url, refresh).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openalex::{ProviderFuture, fake::RecordingTransport};
    use chrono::Utc;
    use lysilogy_core::id::{IdentifierId, WorkId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const OA_ARXIV: &str = include_str!("../../tests/fixtures/acquire/openalex_oa_arxiv.json");
    const OA_CHINA: &str = include_str!("../../tests/fixtures/acquire/openalex_oa_china.json");
    const UNPAYWALL_OK: &str = include_str!("../../tests/fixtures/acquire/unpaywall_ok.json");
    const UNPAYWALL_NO_OA: &str = include_str!("../../tests/fixtures/acquire/unpaywall_no_oa.json");

    fn fixture(text: &str) -> Value {
        serde_json::from_str(text).expect("fixture json")
    }

    fn identifier(
        scheme: IdentifierScheme,
        value: &str,
        status: IdentifierStatus,
    ) -> WorkIdentifier {
        WorkIdentifier {
            id: IdentifierId::new(),
            work_id: WorkId::new(),
            scheme,
            value_normalized: value.to_owned(),
            source_record_id: None,
            status,
            created_at: Utc::now(),
        }
    }

    fn active(scheme: IdentifierScheme, value: &str) -> WorkIdentifier {
        identifier(scheme, value, IdentifierStatus::Active)
    }

    fn link(url: &str, origin: CandidateOrigin) -> LinkCandidate {
        LinkCandidate {
            url: url.to_owned(),
            origin,
            source_record_id: None,
        }
    }

    #[test]
    fn should_derive_arxiv_id_when_doi_is_datacite_arxiv() {
        let found = identifiers_from(&[active(IdentifierScheme::Doi, "10.48550/arxiv.1606.03137")]);
        assert_eq!(found.arxiv.as_deref(), Some("1606.03137"));
        assert_eq!(found.doi.as_deref(), Some("10.48550/arxiv.1606.03137"));
        let old = identifiers_from(&[active(
            IdentifierScheme::Doi,
            "10.48550/arxiv.hep-th/9901001",
        )]);
        assert_eq!(old.arxiv.as_deref(), Some("hep-th/9901001"));
        let plain = identifiers_from(&[active(IdentifierScheme::Doi, "10.1257/aer.103.6.2121")]);
        assert_eq!(plain.arxiv, None);
    }

    #[test]
    fn should_prefer_explicit_arxiv_identifier_when_both_present() {
        let found = identifiers_from(&[
            active(IdentifierScheme::Doi, "10.48550/arxiv.1606.03137"),
            active(IdentifierScheme::Arxiv, "arXiv:2001.00001v2"),
            active(IdentifierScheme::Openalex, "w2410842990"),
        ]);
        assert_eq!(found.arxiv.as_deref(), Some("2001.00001"));
        assert_eq!(found.openalex.as_deref(), Some("W2410842990"));
    }

    #[test]
    fn should_return_empty_identifiers_when_work_has_none() {
        assert!(identifiers_from(&[]).is_empty());
        let inactive = identifiers_from(&[identifier(
            IdentifierScheme::Doi,
            "10.1/x",
            IdentifierStatus::Retracted,
        )]);
        assert!(inactive.is_empty(), "retracted identifiers are not used");
        let malformed = identifiers_from(&[
            active(IdentifierScheme::Doi, "not a doi"),
            active(IdentifierScheme::Openalex, "  "),
            active(IdentifierScheme::Pmid, "123"),
        ]);
        assert!(malformed.is_empty());
    }

    #[test]
    fn should_build_arxiv_pdf_url_when_id_is_new_or_old_style() {
        for (id, expected) in [
            ("1606.03137", "https://arxiv.org/pdf/1606.03137"),
            ("0704.0001", "https://arxiv.org/pdf/0704.0001"),
            ("2101.1234v3", "https://arxiv.org/pdf/2101.1234v3"),
            ("hep-th/9901001", "https://arxiv.org/pdf/hep-th/9901001"),
            ("math.GT/0309136", "https://arxiv.org/pdf/math.GT/0309136"),
        ] {
            assert_eq!(arxiv_pdf_url(id).as_deref(), Some(expected), "{id}");
        }
    }

    #[test]
    fn should_reject_arxiv_pdf_url_when_id_is_malformed() {
        for id in [
            "",
            "1234",
            "../etc",
            "1606.03137/../x",
            "1606.031",
            "1606.031377",
            "hep-th/99010",
            "https://arxiv.org/abs/1606.03137",
            "1606.03137 ",
            "1606.03137\n",
            "a/b/1234567",
        ] {
            assert_eq!(arxiv_pdf_url(id), None, "{id:?}");
        }
    }

    #[test]
    fn should_prefer_openalex_id_over_doi_when_building_oa_url() {
        let both = AcquisitionIdentifiers {
            doi: Some("10.1257/aer.103.6.2121".to_owned()),
            arxiv: None,
            openalex: Some("W2097916756".to_owned()),
        };
        let url = openalex_oa_url(&both).expect("url");
        assert_eq!(
            url,
            "https://api.openalex.org/works/W2097916756\
             ?select=id,doi,ids,open_access,best_oa_location,locations"
        );
        let doi_only = AcquisitionIdentifiers {
            openalex: None,
            ..both
        };
        let url = openalex_oa_url(&doi_only).expect("url");
        assert!(
            url.contains("works/https%3A%2F%2Fdoi%2Eorg%2F10%2E1257%2Faer"),
            "{url}"
        );
        assert_eq!(openalex_oa_url(&AcquisitionIdentifiers::default()), None);
    }

    #[test]
    fn should_percent_encode_doi_and_email_when_building_unpaywall_url() {
        let url = unpaywall_url("10.1002/(sici)1097<x>", "a+b@example.test");
        assert_eq!(
            url,
            "https://api.unpaywall.org/v2/10.1002/%28sici%291097%3Cx%3E?email=a%2Bb%40example%2Etest"
        );
    }

    #[test]
    fn should_rank_arxiv_before_openalex_and_landing_last_when_ranking() {
        let ranked = rank_candidates(vec![
            link(
                "https://l.example.org/landing",
                CandidateOrigin::OpenalexLanding,
            ),
            link(
                "https://u.example.org/b.pdf",
                CandidateOrigin::UnpaywallBest,
            ),
            link(
                "https://o.example.org/loc.pdf",
                CandidateOrigin::OpenalexLocation,
            ),
            link(
                "https://o.example.org/oa.pdf",
                CandidateOrigin::OpenalexOaUrl,
            ),
            link(
                "https://o.example.org/best.pdf",
                CandidateOrigin::OpenalexBest,
            ),
            link("https://arxiv.org/pdf/1606.03137", CandidateOrigin::Arxiv),
            link(
                "https://u.example.org/loc.pdf",
                CandidateOrigin::UnpaywallLocation,
            ),
            link(
                "https://u.example.org/landing",
                CandidateOrigin::UnpaywallLanding,
            ),
            link("ftp://bad.example.org/x.pdf", CandidateOrigin::OpenalexBest),
        ]);
        let origins: Vec<CandidateOrigin> = ranked.iter().map(|c| c.origin).collect();
        assert_eq!(
            origins,
            [
                CandidateOrigin::Arxiv,
                CandidateOrigin::OpenalexBest,
                CandidateOrigin::OpenalexOaUrl,
                CandidateOrigin::OpenalexLocation,
                CandidateOrigin::UnpaywallBest,
                CandidateOrigin::UnpaywallLocation,
                CandidateOrigin::OpenalexLanding,
                CandidateOrigin::UnpaywallLanding,
            ]
        );
    }

    #[test]
    fn should_dedupe_abs_and_pdf_arxiv_urls_when_normalizing() {
        assert_eq!(
            normalize_candidate_url("http://arxiv.org/abs/1606.03137#frag").as_deref(),
            Some("https://arxiv.org/pdf/1606.03137")
        );
        assert_eq!(
            normalize_candidate_url("HTTPS://WWW.Arxiv.org/pdf/1606.03137.pdf").as_deref(),
            Some("https://arxiv.org/pdf/1606.03137")
        );
        assert_eq!(
            normalize_candidate_url("HTTP://Example.ORG/Path/X.pdf#top").as_deref(),
            Some("http://example.org/Path/X.pdf")
        );
        assert_eq!(normalize_candidate_url("mailto:a@b.test"), None);
        assert_eq!(normalize_candidate_url("not a url"), None);
        let ranked = rank_candidates(vec![
            link(
                "http://arxiv.org/abs/1606.03137",
                CandidateOrigin::OpenalexLanding,
            ),
            link(
                "https://arxiv.org/pdf/1606.03137",
                CandidateOrigin::OpenalexBest,
            ),
            link("https://arxiv.org/pdf/1606.03137", CandidateOrigin::Arxiv),
        ]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].origin, CandidateOrigin::Arxiv);
        assert_eq!(ranked[0].url, "https://arxiv.org/pdf/1606.03137");
    }

    #[test]
    fn should_read_best_location_oa_url_and_oa_locations_when_parsing_openalex_fixture() {
        let arxiv = parse_openalex_oa(&fixture(OA_ARXIV), None);
        assert_eq!(arxiv[0].origin, CandidateOrigin::OpenalexBest);
        assert_eq!(arxiv[0].url, "https://arxiv.org/pdf/1606.03137");
        assert_eq!(arxiv[1].origin, CandidateOrigin::OpenalexOaUrl);
        let ranked = rank_candidates(arxiv);
        assert_eq!(ranked[0].url, "https://arxiv.org/pdf/1606.03137");
        assert!(
            ranked
                .iter()
                .skip(1)
                .all(|c| c.origin == CandidateOrigin::OpenalexLanding),
            "{ranked:?}"
        );

        let china = parse_openalex_oa(&fixture(OA_CHINA), None);
        let by = |origin: CandidateOrigin| -> Vec<&str> {
            china
                .iter()
                .filter(|c| c.origin == origin)
                .map(|c| c.url.as_str())
                .collect()
        };
        let aea = "https://www.aeaweb.org/articles/pdf/doi/10.1257/aer.103.6.2121";
        assert_eq!(by(CandidateOrigin::OpenalexBest), [aea]);
        assert_eq!(by(CandidateOrigin::OpenalexOaUrl), [aea]);
        let locations = by(CandidateOrigin::OpenalexLocation);
        assert_eq!(locations[0], aea);
        assert!(locations.contains(&"http://www.nber.org/papers/w18054.pdf"));
        assert!(
            !locations.iter().any(|url| url.contains("docs.iza.org")),
            "is_oa=false locations are skipped"
        );
        assert!(!by(CandidateOrigin::OpenalexLanding).is_empty());
    }

    #[test]
    fn should_skip_non_oa_and_non_http_locations_when_parsing_openalex() {
        let value = json!({
            "best_oa_location": {"pdf_url": "ftp://x.example.org/a.pdf", "landing_page_url": 7},
            "open_access": {"oa_url": ["https://x.example.org/b.pdf"]},
            "locations": [
                {"is_oa": false, "pdf_url": "https://x.example.org/closed.pdf"},
                {"is_oa": "true", "pdf_url": "https://x.example.org/stringy.pdf"},
                {"is_oa": true, "pdf_url": "javascript:alert(1)"},
                {"is_oa": true, "pdf_url": null, "landing_page_url": "mailto:a@b.test"},
                {"is_oa": true, "pdf_url": "https://x.example.org/ok.pdf"},
            ]
        });
        let found = parse_openalex_oa(&value, None);
        assert_eq!(
            found,
            [link(
                "https://x.example.org/ok.pdf",
                CandidateOrigin::OpenalexLocation
            )]
        );
        assert!(parse_openalex_oa(&json!([1, 2]), None).is_empty());
        assert!(parse_openalex_oa(&json!({"locations": "nope"}), None).is_empty());
    }

    #[test]
    fn should_return_no_candidates_when_payload_is_not_found_marker() {
        let marker = json!({"not_found": true});
        assert!(parse_openalex_oa(&marker, None).is_empty());
        assert!(parse_unpaywall(&marker, None).is_empty());
    }

    #[test]
    fn should_attach_source_record_when_provided() {
        let record = SourceRecordId::new();
        let found = parse_openalex_oa(&fixture(OA_ARXIV), Some(record));
        assert!(found.iter().all(|c| c.source_record_id == Some(record)));
    }

    #[test]
    fn should_read_unpaywall_pdf_and_landing_urls_when_parsing_fixture() {
        let found = parse_unpaywall(&fixture(UNPAYWALL_OK), None);
        let summary: Vec<(CandidateOrigin, &str)> =
            found.iter().map(|c| (c.origin, c.url.as_str())).collect();
        assert_eq!(
            summary,
            [
                (
                    CandidateOrigin::UnpaywallBest,
                    "https://repo.example.org/bitstream/1234/5/paper.pdf"
                ),
                (
                    CandidateOrigin::UnpaywallLocation,
                    "https://repo.example.org/bitstream/1234/5/paper.pdf"
                ),
                (
                    CandidateOrigin::UnpaywallLocation,
                    "http://preprints.example.net/p/77.pdf"
                ),
                (
                    CandidateOrigin::UnpaywallLanding,
                    "https://publisher.example.com/article/99"
                ),
            ]
        );
        assert!(parse_unpaywall(&fixture(UNPAYWALL_NO_OA), None).is_empty());
    }

    #[test]
    fn should_tolerate_missing_and_wrongly_typed_fields_when_parsing_unpaywall() {
        for value in [
            json!({}),
            json!(null),
            json!("text"),
            json!({"best_oa_location": "x", "oa_locations": {"a": 1}}),
            json!({"best_oa_location": {"url_for_pdf": 3, "url": false}, "oa_locations": [1, null, "s"]}),
            json!({"oa_locations": [{"url_for_pdf": "gopher://a/b", "url": "ftp://a/b"}]}),
        ] {
            assert!(parse_unpaywall(&value, None).is_empty(), "{value}");
        }
    }

    /// A fake whose responder may answer with an HTTP status error.
    #[derive(Clone)]
    struct StatusTransport {
        urls: Arc<Mutex<Vec<String>>>,
        calls: Arc<AtomicUsize>,
        responder: Arc<dyn Fn(usize) -> Result<Value, u16> + Send + Sync>,
    }

    impl StatusTransport {
        fn new(responder: impl Fn(usize) -> Result<Value, u16> + Send + Sync + 'static) -> Self {
            Self {
                urls: Arc::new(Mutex::new(Vec::new())),
                calls: Arc::new(AtomicUsize::new(0)),
                responder: Arc::new(responder),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn urls(&self) -> Vec<String> {
            self.urls.lock().expect("lock").clone()
        }
    }

    impl JsonTransport for StatusTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, Result<Value, ProviderError>> {
            Box::pin(async move {
                self.urls.lock().expect("lock").push(url.to_owned());
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                (self.responder)(call).map_err(ProviderError::Status)
            })
        }
    }

    async fn records() -> Option<SourceRecordRepository> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = lysilogy_db::connect(&url).await.expect("connect");
        lysilogy_db::migrate(&pool).await.expect("migrate");
        Some(SourceRecordRepository::new(pool))
    }

    fn unique_doi() -> String {
        format!("10.9999/u2-{}", uuid::Uuid::new_v4().simple())
    }

    fn doi_only(doi: &str) -> AcquisitionIdentifiers {
        AcquisitionIdentifiers {
            doi: Some(doi.to_owned()),
            ..AcquisitionIdentifiers::default()
        }
    }

    #[tokio::test]
    async fn should_cache_not_found_marker_when_openalex_returns_404() {
        let Some(records) = records().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = StatusTransport::new(|_| Err(404));
        let client = OpenAlexClient::new(transport.clone());
        let ids = doi_only(&unique_doi());

        let (first_id, first) = fetch_openalex_oa_cached(&records, &client, &ids, false)
            .await
            .expect("first")
            .expect("some");
        assert_eq!(first, json!({"not_found": true}));
        let (second_id, second) = fetch_openalex_oa_cached(&records, &client, &ids, false)
            .await
            .expect("second")
            .expect("some");
        assert_eq!((first_id, first), (second_id, second));
        assert_eq!(transport.calls(), 1, "the marker is served from the cache");
        assert!(parse_openalex_oa(&json!({"not_found": true}), Some(first_id)).is_empty());
    }

    #[tokio::test]
    async fn should_not_cache_anything_when_provider_fails_with_server_error() {
        let Some(records) = records().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = StatusTransport::new(|call| {
            if call == 0 {
                Err(503)
            } else {
                Ok(json!({"id": "W1"}))
            }
        });
        let client = OpenAlexClient::new(transport.clone());
        let ids = doi_only(&unique_doi());

        let failed = fetch_openalex_oa_cached(&records, &client, &ids, false).await;
        assert!(matches!(failed, Err(ProviderError::Status(503))));
        let retried = fetch_openalex_oa_cached(&records, &client, &ids, false)
            .await
            .expect("retry")
            .expect("some");
        assert_eq!(retried.1, json!({"id": "W1"}));
        assert_eq!(transport.calls(), 2);
    }

    #[tokio::test]
    async fn should_return_none_when_work_has_no_openalex_lookup_identifier() {
        let Some(records) = records().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = StatusTransport::new(|_| Ok(json!({})));
        let client = OpenAlexClient::new(transport.clone());
        let ids = AcquisitionIdentifiers {
            arxiv: Some("1606.03137".to_owned()),
            ..AcquisitionIdentifiers::default()
        };
        let result = fetch_openalex_oa_cached(&records, &client, &ids, false).await;
        assert!(matches!(result, Ok(None)));
        assert_eq!(transport.calls(), 0);
    }

    #[tokio::test]
    async fn should_refetch_when_refresh_requested() {
        let Some(records) = records().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = StatusTransport::new(|call| Ok(json!({"call": call})));
        let client = OpenAlexClient::new(transport.clone());
        let ids = doi_only(&unique_doi());

        let (_, first) = fetch_openalex_oa_cached(&records, &client, &ids, false)
            .await
            .expect("first")
            .expect("some");
        let (_, cached) = fetch_openalex_oa_cached(&records, &client, &ids, false)
            .await
            .expect("cached")
            .expect("some");
        assert_eq!(first, cached);
        assert_eq!(transport.calls(), 1);
        let (_, refreshed) = fetch_openalex_oa_cached(&records, &client, &ids, true)
            .await
            .expect("refreshed")
            .expect("some");
        assert_eq!(refreshed, json!({"call": 1}));
        assert_eq!(transport.calls(), 2);
        let (_, after) = fetch_openalex_oa_cached(&records, &client, &ids, false)
            .await
            .expect("after")
            .expect("some");
        assert_eq!(after, refreshed, "refresh upserts the new payload");
    }

    #[tokio::test]
    async fn should_build_unpaywall_url_with_email_when_configured() {
        let Some(records) = records().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = RecordingTransport::new(|_| fixture(UNPAYWALL_OK));
        let client = OpenAlexClient::new(transport.clone());
        let doi = unique_doi();

        let (record, payload) =
            fetch_unpaywall_cached(&records, &client, &doi, "ops@example.test", false)
                .await
                .expect("fetch");
        assert_eq!(
            transport.urls(),
            [format!(
                "https://api.unpaywall.org/v2/{doi}?email=ops%40example%2Etest"
            )]
        );
        assert_eq!(parse_unpaywall(&payload, Some(record)).len(), 4);
        fetch_unpaywall_cached(&records, &client, &doi, "ops@example.test", false)
            .await
            .expect("cached");
        assert_eq!(transport.urls().len(), 1);
    }

    #[tokio::test]
    async fn should_cache_not_found_marker_when_unpaywall_returns_404() {
        let Some(records) = records().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let transport = StatusTransport::new(|_| Err(404));
        let client = OpenAlexClient::new(transport.clone());
        let doi = unique_doi();
        let (_, payload) =
            fetch_unpaywall_cached(&records, &client, &doi, "ops@example.test", false)
                .await
                .expect("fetch");
        assert_eq!(payload, json!({"not_found": true}));
        fetch_unpaywall_cached(&records, &client, &doi, "ops@example.test", false)
            .await
            .expect("cached");
        assert_eq!(transport.calls(), 1);
        assert_eq!(transport.urls().len(), 1);
    }
}
