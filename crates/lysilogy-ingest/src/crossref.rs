//! Crossref as the publisher tier: fetch a Work's registry record by DOI,
//! cache it, and record its metadata as `publisher` claims.
//!
//! Crossref holds publisher-deposited metadata, so it ranks as the publisher
//! (see `lysilogy_core::provenance`). Records are fetched through the same
//! JSON transport as `OpenAlex`; no credentials are added for this host.

use std::sync::LazyLock;

use lysilogy_core::provenance::FieldValue;
use lysilogy_core::{
    ClaimConfidence, ClaimOrigin, Coverage, IdentifierScheme, SourceRecordId, WorkId,
    identifier::normalize,
};
use lysilogy_db::claim::{ClaimRepository, NewClaim};
use lysilogy_db::source_record::SourceRecordRepository;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Result;
use crate::openalex::{JsonTransport, OpenAlexClient, ProviderError};

/// The `source_record.kind` used for cached Crossref work records.
pub const CROSSREF_WORK_KIND: &str = "crossref_work";

/// Extractor recorded on publisher claims read from a Crossref record.
const CROSSREF_EXTRACTOR: &str = "crossref-v1";

/// The most Crossref requests one run makes for cited Works.
pub const PUBLISHER_FETCH_MAX: usize = 200;

/// `DataCite` prefix of arXiv DOIs, which Crossref does not hold.
const ARXIV_DATACITE_PREFIX: &str = "10.48550/";

/// Path characters left readable: unreserved marks and the DOI's own slash.
const DOI_PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// The fields of a Crossref work record that the graph needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossrefWork {
    pub doi: Option<String>,
    pub title: Option<String>,
    /// Ordered, as `given family`, else the record's `name`.
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
}

/// How a Crossref lookup came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossrefOutcome {
    /// Requested just now and cached.
    Fetched(SourceRecordId, CrossrefWork),
    /// Served from the cache.
    Cached(SourceRecordId, CrossrefWork),
    /// Crossref has no record; `cached` when no request was made.
    NotFound { cached: bool },
    /// The DOI is not one Crossref holds (arXiv); nothing was looked up.
    Skipped,
}

/// Whether Crossref can hold this normalized DOI.
#[must_use]
pub fn is_crossref_doi(doi: &str) -> bool {
    !doi.starts_with(ARXIV_DATACITE_PREFIX)
}

/// The request URL for a DOI's Crossref work record.
#[must_use]
pub fn crossref_work_url(doi: &str) -> String {
    format!(
        "https://api.crossref.org/works/{}",
        utf8_percent_encode(doi, DOI_PATH)
    )
}

/// A tag-shaped run such as `<i>`, `</sub>`, `<mml:mi>` or `<br/>`. A bare
/// `<` in running text (`p < 0.05`) does not match.
static MARKUP_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"</?[A-Za-z][A-Za-z0-9:._-]*(?:\s[^<>]*)?/?>").expect("markup tag pattern")
});

static CHARACTER_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(?:#(\d{1,7})|#[xX]([0-9A-Fa-f]{1,6})|(amp|lt|gt|quot|apos|nbsp));")
        .expect("character reference pattern")
});

/// Crossref deposits carry JATS/HTML inline markup (`<i>`, `<sub>`,
/// `<scp>`, `MathML`) and character references. Drop the tags, decode the
/// references, and collapse the whitespace they leave.
fn strip_markup(text: &str) -> String {
    let untagged = MARKUP_TAG.replace_all(text, "");
    let decoded = CHARACTER_REFERENCE.replace_all(&untagged, |captures: &regex::Captures<'_>| {
        let code = captures
            .get(1)
            .and_then(|digits| digits.as_str().parse::<u32>().ok())
            .or_else(|| {
                captures
                    .get(2)
                    .and_then(|hex| u32::from_str_radix(hex.as_str(), 16).ok())
            });
        if let Some(code) = code {
            return char::from_u32(code)
                .filter(|character| !character.is_control())
                .map_or_else(String::new, String::from);
        }
        match captures.get(3).map(|name| name.as_str()) {
            Some("amp") => "&",
            Some("lt") => "<",
            Some("gt") => ">",
            Some("quot") => "\"",
            Some("apos") => "'",
            _ => " ",
        }
        .to_owned()
    });
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn nonempty(value: &Value) -> Option<String> {
    let text = strip_markup(value.as_str()?);
    (!text.is_empty()).then_some(text)
}

fn first_string(message: &Value, key: &str) -> Option<String> {
    message.get(key)?.as_array()?.iter().find_map(nonempty)
}

fn author_name(author: &Value) -> Option<String> {
    let part = |key: &str| author.get(key).and_then(nonempty);
    match (part("given"), part("family")) {
        (Some(given), Some(family)) => Some(format!("{given} {family}")),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => part("name"),
    }
}

fn first_date_year(message: &Value) -> Option<i32> {
    ["published", "published-print", "published-online", "issued"]
        .iter()
        .find_map(|key| {
            let year = message
                .get(key)?
                .pointer("/date-parts/0/0")?
                .as_i64()
                .and_then(|year| i32::try_from(year).ok())?;
            Some(year)
        })
}

/// Read a Crossref response (`{"message": {...}}`) or a bare message.
#[must_use]
pub fn parse_crossref_work(value: &Value) -> CrossrefWork {
    let message = value.get("message").unwrap_or(value);
    CrossrefWork {
        doi: message
            .get("DOI")
            .and_then(Value::as_str)
            .and_then(|doi| normalize(IdentifierScheme::Doi, doi)),
        title: first_string(message, "title"),
        authors: message
            .get("author")
            .and_then(Value::as_array)
            .map(|authors| authors.iter().filter_map(author_name).collect())
            .unwrap_or_default(),
        year: first_date_year(message),
        venue: first_string(message, "container-title"),
    }
}

/// Fetch a DOI's Crossref record, reusing and filling the response cache.
///
/// A 404 is cached as `{"not_found": true}` and never refetched. DOIs Crossref
/// does not hold (`10.48550/`) are skipped without a request or a cache write.
/// Other provider errors are returned and nothing is cached.
pub async fn fetch_crossref_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    doi: &str,
) -> std::result::Result<CrossrefOutcome, ProviderError> {
    let Some(key) = normalize(IdentifierScheme::Doi, doi) else {
        return Ok(CrossrefOutcome::Skipped);
    };
    if !is_crossref_doi(&key) {
        return Ok(CrossrefOutcome::Skipped);
    }
    if let Some((record, payload)) = records.find(CROSSREF_WORK_KIND, &key).await? {
        if payload.get("not_found").and_then(Value::as_bool) == Some(true) {
            return Ok(CrossrefOutcome::NotFound { cached: true });
        }
        return Ok(CrossrefOutcome::Cached(
            record,
            parse_crossref_work(&payload),
        ));
    }
    let value = match client.fetch_json(&crossref_work_url(&key)).await {
        Ok(value) => value,
        Err(error) if error.is_not_found() => {
            records
                .upsert(
                    CROSSREF_WORK_KIND,
                    &key,
                    Coverage::Complete,
                    &json!({ "not_found": true }),
                )
                .await?;
            return Ok(CrossrefOutcome::NotFound { cached: false });
        }
        Err(error) => return Err(error),
    };
    let record = records
        .upsert(CROSSREF_WORK_KIND, &key, Coverage::Complete, &value)
        .await?;
    Ok(CrossrefOutcome::Fetched(
        record,
        parse_crossref_work(&value),
    ))
}

/// Record the Crossref fields as `publisher` claims on a Work. Returns how
/// many claim rows were inserted.
pub async fn record_publisher_claims(
    claims: &ClaimRepository,
    work: WorkId,
    record: SourceRecordId,
    crossref: &CrossrefWork,
) -> Result<usize> {
    let values = [
        crossref.title.clone().map(FieldValue::Title),
        crossref.year.map(FieldValue::Year),
        (!crossref.authors.is_empty()).then(|| FieldValue::Authors(crossref.authors.clone())),
        crossref.venue.clone().map(FieldValue::Venue),
        crossref.doi.clone().map(FieldValue::Doi),
    ];
    let new_claims: Vec<NewClaim<'_>> = values
        .iter()
        .flatten()
        .map(|value| NewClaim {
            work_id: work,
            field: value.field(),
            value,
            origin: ClaimOrigin::Publisher,
            extractor: CROSSREF_EXTRACTOR,
            confidence: ClaimConfidence::Explicit,
            source_record_id: Some(record),
            entry_id: None,
            locator: None,
        })
        .collect();
    if new_claims.is_empty() {
        return Ok(0);
    }
    Ok(claims.record(&new_claims).await?.inserted)
}

/// What one publisher lookup did, as reported by the literature flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherOutcome {
    Fetched,
    Cached,
    NotFound,
    Failed,
}

/// One DOI looked up at the publisher tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublisherFetch {
    pub doi: String,
    pub outcome: PublisherOutcome,
    /// A cited Work's DOI rather than the paper's own.
    pub cited: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openalex::ProviderFuture;
    use crate::openalex::fake::RecordingTransport;
    use lysilogy_core::{WorkKind, provenance::FieldValue};
    use lysilogy_db::work::WorkRepository;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique() -> String {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("x{}x{sequence}", std::process::id())
    }

    async fn pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = lysilogy_db::connect(&url).await.expect("connect");
        lysilogy_db::migrate(&pool).await.expect("migrate");
        Some(pool)
    }

    /// Answers every request with the given status.
    #[derive(Clone)]
    struct StatusTransport {
        status: u16,
        calls: Arc<Mutex<usize>>,
    }

    impl JsonTransport for StatusTransport {
        fn get_json<'a>(
            &'a self,
            _url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async move {
                *self.calls.lock().expect("lock") += 1;
                Err(ProviderError::Status(self.status))
            })
        }
    }

    fn message() -> Value {
        json!({
            "status": "ok",
            "message": {
                "DOI": "10.1000/ABC.Def",
                "title": ["A Careful Study of Things", "Subtitle"],
                "author": [
                    {"given": "Ada", "family": "Lovelace", "sequence": "first"},
                    {"name": "The Consortium"},
                    {"given": "  ", "family": "Babbage"},
                    {}
                ],
                "container-title": ["Journal of Things", "Other"],
                "published-print": {"date-parts": [[2019, 4]]},
                "issued": {"date-parts": [[2018]]}
            }
        })
    }

    #[test]
    fn should_strip_markup_when_crossref_title_has_inline_tags() {
        let work = parse_crossref_work(&json!({"message": {
            "title": ["The <i>Drosophila</i> CO<sub>2</sub>  sensor &amp; <scp>p</scp> &lt; 0.05"],
            "author": [{"given": "Jos&#233;", "family": "Garc&#xED;a"}],
            "container-title": ["<b>Nature</b>"],
        }}));
        assert_eq!(
            work.title.as_deref(),
            Some("The Drosophila CO2 sensor & p < 0.05")
        );
        assert_eq!(work.authors, ["Jos\u{e9} Garc\u{ed}a"]);
        assert_eq!(work.venue.as_deref(), Some("Nature"));
    }

    #[test]
    fn should_keep_bare_angle_brackets_when_crossref_title_has_no_tags() {
        assert_eq!(
            strip_markup("When p < 0.05 and q > 1"),
            "When p < 0.05 and q > 1"
        );
        assert_eq!(
            strip_markup("<mml:math><mml:mi>x</mml:mi></mml:math>-ray &bogus; &#0;"),
            "x-ray &bogus;"
        );
    }

    #[test]
    fn should_skip_title_when_only_markup_remains() {
        let work = parse_crossref_work(&json!({"message": {"title": ["<i> </i>", "Real"]}}));
        assert_eq!(work.title.as_deref(), Some("Real"));
    }

    #[test]
    fn should_parse_title_authors_year_venue_when_crossref_message_given() {
        let work = parse_crossref_work(&message());
        assert_eq!(work.doi.as_deref(), Some("10.1000/abc.def"));
        assert_eq!(work.title.as_deref(), Some("A Careful Study of Things"));
        assert_eq!(work.authors, ["Ada Lovelace", "The Consortium", "Babbage"]);
        assert_eq!(work.year, Some(2019), "published-print precedes issued");
        assert_eq!(work.venue.as_deref(), Some("Journal of Things"));
        assert_eq!(parse_crossref_work(&json!({})), CrossrefWork::default());
        assert_eq!(
            parse_crossref_work(&json!({"message": {"title": "not a list"}})),
            CrossrefWork::default()
        );
    }

    #[test]
    fn should_percent_encode_doi_when_building_crossref_url() {
        assert_eq!(
            crossref_work_url("10.1002/(sici)1097?x=1#y z"),
            "https://api.crossref.org/works/10.1002/%28sici%291097%3Fx%3D1%23y%20z"
        );
        assert_eq!(
            crossref_work_url("10.1000/a-b.c_d"),
            "https://api.crossref.org/works/10.1000/a-b.c_d"
        );
    }

    #[tokio::test]
    async fn should_cache_not_found_and_skip_refetch_when_crossref_returns_404() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let doi = format!("10.5555/{}", unique());
        let calls = Arc::new(Mutex::new(0));
        let client = OpenAlexClient::new(StatusTransport {
            status: 404,
            calls: calls.clone(),
        });
        let first = fetch_crossref_cached(&records, &client, &doi)
            .await
            .expect("first");
        assert_eq!(first, CrossrefOutcome::NotFound { cached: false });
        let second = fetch_crossref_cached(&records, &client, &doi)
            .await
            .expect("second");
        assert_eq!(second, CrossrefOutcome::NotFound { cached: true });
        assert_eq!(*calls.lock().expect("lock"), 1);
        let cached = records
            .get(CROSSREF_WORK_KIND, &doi)
            .await
            .expect("get")
            .expect("payload");
        assert_eq!(cached, json!({"not_found": true}));
    }

    #[tokio::test]
    async fn should_not_cache_when_crossref_fails_with_server_error() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let doi = format!("10.5555/{}", unique());
        let calls = Arc::new(Mutex::new(0));
        let client = OpenAlexClient::new(StatusTransport {
            status: 503,
            calls: calls.clone(),
        });
        let error = fetch_crossref_cached(&records, &client, &doi)
            .await
            .expect_err("503");
        assert!(matches!(error, ProviderError::Status(503)));
        assert!(
            records
                .get(CROSSREF_WORK_KIND, &doi)
                .await
                .expect("get")
                .is_none()
        );
    }

    #[tokio::test]
    async fn should_skip_arxiv_datacite_doi_when_fetching_publisher() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let transport = RecordingTransport::new(|_| json!({}));
        let client = OpenAlexClient::new(transport.clone());
        let outcome = fetch_crossref_cached(&records, &client, "10.48550/arXiv.1706.03762")
            .await
            .expect("skip");
        assert_eq!(outcome, CrossrefOutcome::Skipped);
        assert!(transport.urls().is_empty());
        assert!(!is_crossref_doi("10.48550/arxiv.1706.03762"));
        assert!(is_crossref_doi("10.1000/abc"));
    }

    #[tokio::test]
    async fn should_record_publisher_claims_that_win_selection_when_crossref_fetched() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let claims = ClaimRepository::new(pool.clone());
        let records = SourceRecordRepository::new(pool);
        let marker = unique();
        let doi = format!("10.5555/{marker}");
        let work = works
            .create(WorkKind::Article, lysilogy_core::Visibility::Private)
            .await
            .expect("work");
        let openalex_record = records
            .upsert(
                "openalex_work_summary",
                &format!("w{marker}"),
                Coverage::Complete,
                &json!({}),
            )
            .await
            .expect("record");
        let openalex_title = FieldValue::Title("The OpenAlex Title".to_owned());
        claims
            .record(&[NewClaim {
                work_id: work.id,
                field: openalex_title.field(),
                value: &openalex_title,
                origin: ClaimOrigin::Openalex,
                extractor: "openalex",
                confidence: ClaimConfidence::Explicit,
                source_record_id: Some(openalex_record),
                entry_id: None,
                locator: None,
            }])
            .await
            .expect("openalex claim");

        let payload = json!({"message": {
            "DOI": doi,
            "title": [format!("The Publisher Title {marker}")],
            "author": [{"given": "Grace", "family": "Hopper"}],
            "container-title": ["Annals of Testing"],
            "issued": {"date-parts": [[2020]]}
        }});
        let responder = payload.clone();
        let client = OpenAlexClient::new(RecordingTransport::new(move |_| responder.clone()));
        let CrossrefOutcome::Fetched(record, fetched) =
            fetch_crossref_cached(&records, &client, &doi)
                .await
                .expect("fetch")
        else {
            panic!("expected a fresh fetch");
        };
        let inserted = record_publisher_claims(&claims, work.id, record, &fetched)
            .await
            .expect("claims");
        assert_eq!(inserted, 5);
        let again = record_publisher_claims(&claims, work.id, record, &fetched)
            .await
            .expect("again");
        assert_eq!(again, 0);

        let selected = works.get(work.id).await.expect("get").expect("exists");
        assert_eq!(
            selected.display_title,
            Some(format!("The Publisher Title {marker}"))
        );
        assert_eq!(selected.display_year, Some(2020));
        assert_eq!(
            selected.display_authors,
            Some(vec!["Grace Hopper".to_owned()])
        );
    }
}
