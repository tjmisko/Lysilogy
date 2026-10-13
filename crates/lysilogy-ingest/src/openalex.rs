//! A minimal `OpenAlex` client: fetch a Work by DOI or `OpenAlex` ID and read its
//! outbound `referenced_works`, with an optional Postgres-backed response cache.
//!
//! The transport is abstracted so parsing and caching can be tested offline; the
//! live [`ReqwestTransport`] reads credentials from the environment and is never
//! exercised by tests.

use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use lysilogy_core::{Coverage, SourceRecordId, title_key};
use lysilogy_db::source_record::SourceRecordRepository;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

/// The `source_record.kind` used for cached `OpenAlex` work responses.
pub const OPENALEX_WORK_KIND: &str = "openalex_work";

/// The `source_record.kind` used for cached batched `OpenAlex` summaries.
pub const OPENALEX_SUMMARY_KIND: &str = "openalex_work_summary";

/// The `source_record.kind` used for cached `OpenAlex` title searches.
pub const OPENALEX_TITLE_SEARCH_KIND: &str = "openalex_title_search";

/// The `source_record.kind` used for cached `OpenAlex` cited-by counts.
pub const OPENALEX_METRICS_KIND: &str = "openalex_metrics";

/// The most IDs sent in one `filter=openalex:` request.
pub const SUMMARY_BATCH: usize = 50;

/// How many candidates a title search asks for.
const TITLE_SEARCH_PAGE: usize = 10;

/// A boxed provider future, mirroring the transport style used elsewhere.
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Errors from provider access, parsing, or the response cache.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned HTTP {0}")]
    Status(u16),
    #[error("provider returned an unexpected response")]
    Malformed,
    #[error(transparent)]
    Db(#[from] lysilogy_db::DbError),
}

impl ProviderError {
    /// Whether the provider answered that the record does not exist.
    #[must_use]
    pub const fn is_not_found(&self) -> bool {
        matches!(self, Self::Status(404))
    }
}

/// A minimal JSON transport, implemented by the live client and by tests.
pub trait JsonTransport: Send + Sync {
    fn get_json<'a>(
        &'a self,
        url: &'a str,
    ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>>;
}

/// The fields of an `OpenAlex` work that the graph needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenAlexWork {
    pub openalex_id: String,
    pub doi: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub referenced_works: Vec<String>,
}

/// The title-level fields of an `OpenAlex` work, enough to label a stub or
/// judge a title-search candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenAlexSummary {
    pub openalex_id: String,
    pub doi: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub first_author: Option<String>,
    pub referenced_works_count: Option<u32>,
}

/// Parse a summary. Only a missing `id` is an error; any other missing or
/// malformed field becomes `None`.
pub fn parse_summary(value: &Value) -> Result<OpenAlexSummary, ProviderError> {
    let openalex_id = value
        .get("id")
        .and_then(Value::as_str)
        .map(short_openalex_id)
        .filter(|id| !id.is_empty())
        .ok_or(ProviderError::Malformed)?;
    let doi = value
        .get("doi")
        .and_then(Value::as_str)
        .and_then(normalize_doi);
    let title = value
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    let year = value
        .get("publication_year")
        .and_then(Value::as_i64)
        .and_then(|year| i32::try_from(year).ok());
    let first_author = value
        .pointer("/authorships/0/author/display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let referenced_works_count = value
        .get("referenced_works_count")
        .and_then(Value::as_u64)
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX));
    Ok(OpenAlexSummary {
        openalex_id,
        doi,
        title,
        year,
        first_author,
        referenced_works_count,
    })
}

/// Parse an `OpenAlex` work response.
pub fn parse_work(value: &Value) -> Result<OpenAlexWork, ProviderError> {
    let openalex_id = value
        .get("id")
        .and_then(Value::as_str)
        .map(short_openalex_id)
        .filter(|id| !id.is_empty())
        .ok_or(ProviderError::Malformed)?;
    let doi = value
        .get("doi")
        .and_then(Value::as_str)
        .and_then(normalize_doi);
    let title = value
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let year = value
        .get("publication_year")
        .and_then(Value::as_i64)
        .and_then(|year| i32::try_from(year).ok());
    let referenced_works = value
        .get("referenced_works")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(short_openalex_id)
                .collect()
        })
        .unwrap_or_default();
    Ok(OpenAlexWork {
        openalex_id,
        doi,
        title,
        year,
        referenced_works,
    })
}

/// The request URL for a work identified by `doi:...` or an `OpenAlex` ID.
#[must_use]
pub fn work_url(id: &str) -> String {
    let target = id
        .strip_prefix("doi:")
        .map_or_else(|| id.to_owned(), |doi| format!("https://doi.org/{doi}"));
    let encoded = utf8_percent_encode(&target, NON_ALPHANUMERIC);
    format!(
        "https://api.openalex.org/works/{encoded}\
         ?select=id,doi,display_name,publication_year,referenced_works"
    )
}

/// The request URL for a batch of at most [`SUMMARY_BATCH`] `OpenAlex` IDs.
#[must_use]
pub fn works_batch_url(ids: &[&str]) -> String {
    debug_assert!(ids.len() <= SUMMARY_BATCH, "batch exceeds {SUMMARY_BATCH}");
    format!(
        "https://api.openalex.org/works?filter=openalex:{}&per_page={}\
         &select=id,doi,display_name,publication_year",
        ids.join("|"),
        ids.len()
    )
}

/// The request URL for a batch of at most [`SUMMARY_BATCH`] cited-by counts.
#[must_use]
pub fn works_metrics_url(ids: &[&str]) -> String {
    debug_assert!(ids.len() <= SUMMARY_BATCH, "batch exceeds {SUMMARY_BATCH}");
    format!(
        "https://api.openalex.org/works?filter=openalex:{}&per_page={}\
         &select=id,cited_by_count",
        ids.join("|"),
        ids.len()
    )
}

/// Read `(lower-case short id, cited_by_count)` from a metrics batch item.
/// `None` when the id is missing or the count is not a non-negative integer.
#[must_use]
pub fn parse_cited_by_count(item: &Value) -> Option<(String, i64)> {
    let id = source_key(&short_openalex_id(item.get("id")?.as_str()?));
    if id.is_empty() {
        return None;
    }
    let count = item
        .get("cited_by_count")?
        .as_i64()
        .filter(|count| *count >= 0)?;
    Some((id, count))
}

/// The request URL for a title search, or `None` without searchable words.
///
/// Only alphanumeric words of the normalized title reach the
/// filter, so `,`, `|`, and `:` can never alter its structure.
#[must_use]
pub fn title_search_url(title: &str, year: Option<i32>) -> Option<String> {
    let key = title_key(title);
    let words: Vec<String> = key
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| utf8_percent_encode(word, NON_ALPHANUMERIC).to_string())
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut filter = format!("title.search:{}", words.join("%20"));
    if let Some(year) = year {
        let (low, high) = (i64::from(year) - 1, i64::from(year) + 1);
        let _ = write!(filter, ",publication_year:{low}-{high}");
    }
    Some(format!(
        "https://api.openalex.org/works?filter={filter}&per_page={TITLE_SEARCH_PAGE}\
         &select=id,doi,display_name,publication_year,referenced_works_count,authorships"
    ))
}

/// The contact email, from the first of the given variables that has a
/// non-blank value. Pure so tests never touch the process environment.
pub fn contact_email_from(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LYSILOGY_CONTACT_EMAIL", "LYSILOGY_CITATION_MAILTO"]
        .iter()
        .filter_map(|name| lookup(name))
        .map(|value| value.trim().to_owned())
        .find(|value| !value.is_empty())
}

/// The contact email configured in the environment, if any.
#[must_use]
pub fn contact_email_from_environment() -> Option<String> {
    contact_email_from(|name| env::var(name).ok())
}

/// A stable cache key for a work identifier.
#[must_use]
pub fn source_key(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

/// An `OpenAlex` client over a JSON transport.
pub struct OpenAlexClient<T> {
    transport: T,
}

impl<T: JsonTransport> OpenAlexClient<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Fetch the raw work response.
    pub async fn fetch_work_value(&self, id: &str) -> Result<Value, ProviderError> {
        self.transport.get_json(&work_url(id)).await
    }

    /// Fetch an arbitrary `OpenAlex` URL as JSON.
    pub async fn fetch_json(&self, url: &str) -> Result<Value, ProviderError> {
        self.transport.get_json(url).await
    }

    /// Fetch and parse a work.
    pub async fn fetch_work(&self, id: &str) -> Result<OpenAlexWork, ProviderError> {
        parse_work(&self.fetch_work_value(id).await?)
    }
}

/// Fetch a work, reusing and refreshing the Postgres response cache.
pub async fn fetch_work_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    id: &str,
) -> Result<OpenAlexWork, ProviderError> {
    fetch_work_cached_record(records, client, id)
        .await
        .map(|(_, work)| work)
}

/// Like [`fetch_work_cached`], also returning the cache record's ID.
pub async fn fetch_work_cached_record<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    id: &str,
) -> Result<(SourceRecordId, OpenAlexWork), ProviderError> {
    let key = source_key(id);
    if let Some((record_id, payload)) = records.find(OPENALEX_WORK_KIND, &key).await?
        && let Ok(work) = parse_work(&payload)
    {
        return Ok((record_id, work));
    }
    let value = client.fetch_work_value(id).await?;
    let record_id = records
        .upsert(OPENALEX_WORK_KIND, &key, Coverage::Complete, &value)
        .await?;
    Ok((record_id, parse_work(&value)?))
}

/// Fetch summaries for many IDs: cached ones come from Postgres, the rest are
/// requested in chunks of [`SUMMARY_BATCH`]. IDs the provider omits are skipped.
pub async fn fetch_summaries_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    ids: &[String],
) -> Result<Vec<(SourceRecordId, OpenAlexSummary)>, ProviderError> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for id in ids {
        let key = source_key(id);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        if let Some((record_id, payload)) = records.find(OPENALEX_SUMMARY_KIND, &key).await?
            && let Ok(summary) = parse_summary(&payload)
        {
            found.push((record_id, summary));
            continue;
        }
        missing.push(id.trim());
    }
    for chunk in missing.chunks(SUMMARY_BATCH) {
        let page = client.fetch_json(&works_batch_url(chunk)).await?;
        let Some(results) = page.get("results").and_then(Value::as_array) else {
            return Err(ProviderError::Malformed);
        };
        for item in results {
            let Ok(summary) = parse_summary(item) else {
                continue;
            };
            let key = source_key(&summary.openalex_id);
            if !seen.contains(&key) {
                continue;
            }
            let record_id = records
                .upsert(OPENALEX_SUMMARY_KIND, &key, Coverage::Complete, item)
                .await?;
            found.push((record_id, summary));
        }
    }
    Ok(found)
}

/// Cited-by counts fetched for a set of Works.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CitedByFetch {
    /// `(lower-case OpenAlex id, count)` for every ID that has a count.
    pub counts: Vec<(String, i64)>,
    /// Provider requests made.
    pub requests: usize,
}

/// Fetch cited-by counts for many IDs: cached ones come from Postgres unless
/// `refresh`, the rest are requested in chunks of [`SUMMARY_BATCH`]. IDs the
/// provider omits, and items without a valid count, are skipped and not cached.
pub async fn fetch_cited_by_counts_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    ids: &[String],
    refresh: bool,
) -> Result<CitedByFetch, ProviderError> {
    let mut seen = HashSet::new();
    let mut fetch = CitedByFetch::default();
    let mut missing: Vec<&str> = Vec::new();
    for id in ids {
        let key = source_key(id);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        if !refresh
            && let Some((_, payload)) = records.find(OPENALEX_METRICS_KIND, &key).await?
            && let Some((_, count)) = parse_cited_by_count(&payload)
        {
            fetch.counts.push((key, count));
            continue;
        }
        missing.push(id.trim());
    }
    for chunk in missing.chunks(SUMMARY_BATCH) {
        let page = client.fetch_json(&works_metrics_url(chunk)).await?;
        fetch.requests += 1;
        let Some(results) = page.get("results").and_then(Value::as_array) else {
            return Err(ProviderError::Malformed);
        };
        for item in results {
            let Some((key, count)) = parse_cited_by_count(item) else {
                continue;
            };
            if !seen.contains(&key) {
                continue;
            }
            records
                .upsert(OPENALEX_METRICS_KIND, &key, Coverage::Complete, item)
                .await?;
            fetch.counts.push((key, count));
        }
    }
    Ok(fetch)
}

/// Search works by title, caching the raw response. `None` when the title has
/// no searchable words (no request is made).
pub async fn search_title_cached<T: JsonTransport>(
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    title: &str,
    year: Option<i32>,
) -> Result<Option<(SourceRecordId, Vec<OpenAlexSummary>)>, ProviderError> {
    let Some(url) = title_search_url(title, year) else {
        return Ok(None);
    };
    let key = format!(
        "{}|{}",
        title_key(title),
        year.map_or_else(|| "-".to_owned(), |year| year.to_string())
    );
    let cached = records.find(OPENALEX_TITLE_SEARCH_KIND, &key).await?;
    let (record_id, payload) = if let Some(hit) = cached {
        hit
    } else {
        let value = client.fetch_json(&url).await?;
        let record_id = records
            .upsert(OPENALEX_TITLE_SEARCH_KIND, &key, Coverage::Complete, &value)
            .await?;
        (record_id, value)
    };
    let Some(results) = payload.get("results").and_then(Value::as_array) else {
        return Err(ProviderError::Malformed);
    };
    let summaries = results
        .iter()
        .filter_map(|item| parse_summary(item).ok())
        .collect();
    Ok(Some((record_id, summaries)))
}

/// The only host that receives the contact email and API key.
const OPENALEX_HOST: &str = "api.openalex.org";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Parse `url`, adding `mailto` and `api_key` query parameters only when the host
/// is `api.openalex.org`, so credentials never reach other providers.
pub fn decorate_url(
    url: &str,
    mailto: Option<&str>,
    api_key: Option<&str>,
) -> Result<reqwest::Url, ProviderError> {
    let mut parsed = reqwest::Url::parse(url).map_err(|_| ProviderError::Malformed)?;
    if parsed.host_str() != Some(OPENALEX_HOST) || (mailto.is_none() && api_key.is_none()) {
        return Ok(parsed);
    }
    {
        let mut pairs = parsed.query_pairs_mut();
        if let Some(mailto) = mailto {
            pairs.append_pair("mailto", mailto);
        }
        if let Some(key) = api_key {
            pairs.append_pair("api_key", key);
        }
    }
    Ok(parsed)
}

/// The live HTTP transport, reading credentials from the environment.
pub struct ReqwestTransport {
    client: reqwest::Client,
    mailto: Option<String>,
    api_key: Option<String>,
}

impl ReqwestTransport {
    pub fn from_environment() -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Lysilogy/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            mailto: contact_email_from_environment(),
            api_key: env::var("LYSILOGY_OPENALEX_API_KEY").ok(),
        })
    }
}

impl JsonTransport for ReqwestTransport {
    fn get_json<'a>(&'a self, url: &'a str) -> ProviderFuture<'a, Result<Value, ProviderError>> {
        Box::pin(async move {
            let parsed = decorate_url(url, self.mailto.as_deref(), self.api_key.as_deref())?;
            let response =
                self.client.get(parsed).send().await.map_err(|_| {
                    ProviderError::Request("request failed or timed out".to_owned())
                })?;
            let status = response.status();
            if !status.is_success() {
                return Err(ProviderError::Status(status.as_u16()));
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|_| ProviderError::Malformed)?;
            serde_json::from_slice(&bytes).map_err(|_| ProviderError::Malformed)
        })
    }
}

/// Strip the `https://openalex.org/` prefix from an `OpenAlex` ID.
#[must_use]
pub fn short_openalex_id(value: &str) -> String {
    value
        .strip_prefix("https://openalex.org/")
        .unwrap_or(value)
        .to_owned()
}

/// Lowercase a DOI and strip any resolver or `doi:` prefix.
#[must_use]
pub fn normalize_doi(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    let doi = ["https://doi.org/", "http://doi.org/", "doi:"]
        .iter()
        .find_map(|prefix| lowered.strip_prefix(prefix))
        .map_or_else(|| lowered.clone(), str::to_owned);
    (!doi.is_empty()).then_some(doi)
}

/// A URL-recording fake transport for tests in this crate.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Responder = dyn Fn(&str) -> Value + Send + Sync;

    #[derive(Clone)]
    pub struct RecordingTransport {
        pub urls: Arc<Mutex<Vec<String>>>,
        responder: Arc<Responder>,
    }

    impl RecordingTransport {
        pub fn new(responder: impl Fn(&str) -> Value + Send + Sync + 'static) -> Self {
            Self {
                urls: Arc::new(Mutex::new(Vec::new())),
                responder: Arc::new(responder),
            }
        }

        pub fn urls(&self) -> Vec<String> {
            self.urls.lock().expect("lock").clone()
        }
    }

    impl JsonTransport for RecordingTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, Result<Value, ProviderError>> {
            Box::pin(async move {
                self.urls.lock().expect("lock").push(url.to_owned());
                Ok((self.responder)(url))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct FakeTransport {
        payload: Value,
        calls: Arc<AtomicUsize>,
    }

    impl FakeTransport {
        fn new(payload: Value) -> Self {
            Self {
                payload,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl JsonTransport for FakeTransport {
        fn get_json<'a>(
            &'a self,
            _url: &'a str,
        ) -> ProviderFuture<'a, Result<Value, ProviderError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(self.payload.clone())
            })
        }
    }

    fn fixture() -> Value {
        serde_json::json!({
            "id": "https://openalex.org/W2741809807",
            "doi": "https://doi.org/10.7717/PeerJ.4375",
            "display_name": "The state of OA",
            "publication_year": 2018,
            "referenced_works": ["https://openalex.org/W111", "W222"]
        })
    }

    async fn pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = lysilogy_db::connect(&url).await.expect("connect");
        lysilogy_db::migrate(&pool).await.expect("migrate");
        Some(pool)
    }

    #[test]
    fn parses_work_fields() {
        let work = parse_work(&fixture()).expect("parse");
        assert_eq!(work.openalex_id, "W2741809807");
        assert_eq!(work.doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(work.title.as_deref(), Some("The state of OA"));
        assert_eq!(work.year, Some(2018));
        assert_eq!(work.referenced_works, ["W111", "W222"]);
    }

    #[test]
    fn rejects_a_response_without_an_id() {
        assert!(matches!(
            parse_work(&serde_json::json!({})),
            Err(ProviderError::Malformed)
        ));
    }

    #[test]
    fn builds_an_encoded_work_url() {
        let url = work_url("doi:10.1234/AbC");
        assert!(url.starts_with("https://api.openalex.org/works/"));
        assert!(url.contains("%2F") || url.contains("%2f"));
        assert!(url.contains("select="));
    }

    #[tokio::test]
    async fn cached_fetch_contacts_provider_once() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let transport = FakeTransport::new(fixture());
        let client = OpenAlexClient::new(transport.clone());
        let id = format!("doi:10.9999/{}", std::process::id());

        let first = fetch_work_cached(&records, &client, &id)
            .await
            .expect("first");
        let second = fetch_work_cached(&records, &client, &id)
            .await
            .expect("second");
        assert_eq!(first, second);
        assert_eq!(transport.calls(), 1, "second call must come from the cache");
    }

    fn summary_json(id: &str, title: &str) -> Value {
        serde_json::json!({
            "id": format!("https://openalex.org/{id}"),
            "doi": null,
            "display_name": title,
            "publication_year": 2020,
        })
    }

    #[test]
    fn should_parse_summary_when_authorships_missing() {
        let summary = parse_summary(&serde_json::json!({
            "id": "https://openalex.org/W5",
            "doi": "https://doi.org/10.1/ABC",
            "display_name": "  A Title ",
            "publication_year": "not a number",
            "referenced_works_count": "many",
            "authorships": "garbage",
        }))
        .expect("parse");
        assert_eq!(summary.openalex_id, "W5");
        assert_eq!(summary.doi.as_deref(), Some("10.1/abc"));
        assert_eq!(summary.title.as_deref(), Some("A Title"));
        assert_eq!(summary.year, None);
        assert_eq!(summary.first_author, None);
        assert_eq!(summary.referenced_works_count, None);
    }

    #[test]
    fn should_read_first_author_when_authorships_present() {
        let summary = parse_summary(&serde_json::json!({
            "id": "W5",
            "referenced_works_count": 115,
            "authorships": [{"author": {"display_name": "David H. Autor"}}, {"author": {}}],
        }))
        .expect("parse");
        assert_eq!(summary.first_author.as_deref(), Some("David H. Autor"));
        assert_eq!(summary.referenced_works_count, Some(115));
    }

    #[test]
    fn should_reject_summary_when_id_missing() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"id": ""}),
            serde_json::json!({"id": 4}),
        ] {
            assert!(matches!(
                parse_summary(&value),
                Err(ProviderError::Malformed)
            ));
        }
    }

    #[test]
    fn should_build_batch_url_when_given_ids() {
        let url = works_batch_url(&["W1", "W2", "W3"]);
        assert!(url.contains("filter=openalex:W1|W2|W3"), "{url}");
        assert!(url.contains("per_page=3"), "{url}");
        assert!(
            url.contains("select=id,doi,display_name,publication_year"),
            "{url}"
        );
    }

    #[test]
    fn should_return_none_title_search_url_when_title_has_no_words() {
        assert_eq!(title_search_url("", None), None);
        assert_eq!(title_search_url("  ,|: -- ", Some(2020)), None);
    }

    #[test]
    fn should_escape_filter_delimiters_when_title_contains_commas_and_pipes() {
        let url = title_search_url("Hello, World | Foo: Bar", Some(2013)).expect("url");
        let filter = url
            .split("filter=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .expect("filter");
        assert_eq!(
            filter,
            "title.search:hello%20world%20foo%20bar,publication_year:2012-2014"
        );
        assert!(!filter.contains('|'));
        assert!(url.contains("per_page=10"));
        assert!(url.contains("referenced_works_count,authorships"));
        let without_year = title_search_url("a, b | c: d", None).expect("url");
        assert!(!without_year.contains("publication_year:"));
        assert!(!without_year.contains("%7C"));
    }

    #[test]
    fn should_prefer_contact_email_when_both_variables_set() {
        let email = contact_email_from(|name| match name {
            "LYSILOGY_CONTACT_EMAIL" => Some("primary@example.test".to_owned()),
            "LYSILOGY_CITATION_MAILTO" => Some("legacy@example.test".to_owned()),
            _ => None,
        });
        assert_eq!(email.as_deref(), Some("primary@example.test"));
        let legacy = contact_email_from(|name| {
            (name == "LYSILOGY_CITATION_MAILTO").then(|| "legacy@example.test".to_owned())
        });
        assert_eq!(legacy.as_deref(), Some("legacy@example.test"));
    }

    #[test]
    fn should_ignore_blank_contact_email_when_set_to_whitespace() {
        let email = contact_email_from(|name| match name {
            "LYSILOGY_CONTACT_EMAIL" => Some("   ".to_owned()),
            "LYSILOGY_CITATION_MAILTO" => Some("legacy@example.test".to_owned()),
            _ => None,
        });
        assert_eq!(email.as_deref(), Some("legacy@example.test"));
        assert_eq!(contact_email_from(|_| Some(" \t".to_owned())), None);
        assert_eq!(contact_email_from(|_| None), None);
    }

    #[test]
    fn should_append_mailto_only_when_host_is_openalex() {
        let url = decorate_url(
            "https://api.openalex.org/works/W1?select=id",
            Some("ops@example.test"),
            Some("secret"),
        )
        .expect("url");
        let query = url.query().unwrap_or_default();
        assert!(query.contains("select=id"), "{query}");
        assert!(query.contains("mailto=ops%40example.test"), "{query}");
        assert!(query.contains("api_key=secret"), "{query}");
        let bare = decorate_url("https://api.openalex.org/works/W1", None, None).expect("url");
        assert_eq!(bare.query(), None);
    }

    #[test]
    fn should_not_leak_api_key_when_url_is_unpaywall() {
        for host_url in [
            "https://api.unpaywall.org/v2/10.1/x?email=a%40b.test",
            "https://api.openalex.org.evil.example/works/W1",
            "https://evil.example/api.openalex.org",
            "https://user@api.openalex.org.evil.example/",
        ] {
            let url =
                decorate_url(host_url, Some("ops@example.test"), Some("secret")).expect("url");
            assert_eq!(url.as_str(), host_url, "unchanged: {host_url}");
            assert!(!url.as_str().contains("secret"));
            assert!(!url.as_str().contains("ops%40"));
        }
        assert!(matches!(
            decorate_url("not a url", Some("a"), Some("b")),
            Err(ProviderError::Malformed)
        ));
    }

    #[test]
    fn should_report_not_found_when_status_is_404() {
        assert!(ProviderError::Status(404).is_not_found());
        assert!(!ProviderError::Status(500).is_not_found());
        assert!(!ProviderError::Malformed.is_not_found());
    }

    #[tokio::test]
    async fn should_fetch_only_uncached_summaries_when_some_are_cached() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let marker = std::process::id();
        let ids: Vec<String> = (0..120).map(|n| format!("W77{marker}{n:03}")).collect();
        let transport = fake::RecordingTransport::new(|url| {
            let filter = url
                .split("filter=openalex:")
                .nth(1)
                .and_then(|rest| rest.split('&').next())
                .unwrap_or_default();
            let mut results: Vec<Value> = filter
                .split('|')
                .map(|id| summary_json(id, "Cached Title"))
                .collect();
            // The provider omits the last requested id.
            results.pop();
            serde_json::json!({ "results": results })
        });
        let client = OpenAlexClient::new(transport.clone());

        // Prime the cache with the first 10 ids.
        let primed = fetch_summaries_cached(&records, &client, &ids[..10])
            .await
            .expect("prime");
        assert_eq!(primed.len(), 9, "omitted id is skipped");
        assert_eq!(transport.urls().len(), 1);

        let all = fetch_summaries_cached(&records, &client, &ids)
            .await
            .expect("all");
        let urls = transport.urls();
        assert_eq!(
            urls.len(),
            4,
            "1 priming + 3 chunks for 111 uncached ids: {urls:?}"
        );
        for url in &urls[1..] {
            assert!(url.contains("filter=openalex:"));
            assert!(
                !url.contains(&ids[0]),
                "cached id must not be refetched: {url}"
            );
            assert!(url.matches('|').count() < SUMMARY_BATCH);
        }
        assert!(urls[1].contains("per_page=50"));
        assert!(urls[3].contains("per_page=11"), "{}", urls[3]);
        assert_eq!(all.len(), 9 + 49 + 49 + 10);
    }

    #[test]
    fn should_join_ids_and_select_cited_by_count_when_building_metrics_url() {
        let url = works_metrics_url(&["W1", "W2"]);
        assert!(url.contains("filter=openalex:W1|W2"), "{url}");
        assert!(url.contains("per_page=2"), "{url}");
        assert!(url.ends_with("select=id,cited_by_count"), "{url}");
    }

    #[test]
    fn should_skip_non_integer_and_negative_counts_when_parsing_metrics_batch() {
        let ok = serde_json::json!({"id": "https://openalex.org/W9", "cited_by_count": 12});
        assert_eq!(parse_cited_by_count(&ok), Some(("w9".to_owned(), 12)));
        for bad in [
            serde_json::json!({"id": "W1", "cited_by_count": -1}),
            serde_json::json!({"id": "W1", "cited_by_count": 1.5}),
            serde_json::json!({"id": "W1", "cited_by_count": "7"}),
            serde_json::json!({"id": "W1", "cited_by_count": null}),
            serde_json::json!({"id": "W1", "cited_by_count": u64::MAX}),
            serde_json::json!({"id": "W1"}),
            serde_json::json!({"id": "", "cited_by_count": 3}),
            serde_json::json!({"id": 4, "cited_by_count": 3}),
            serde_json::json!({"cited_by_count": 3}),
        ] {
            assert_eq!(parse_cited_by_count(&bad), None, "{bad}");
        }
    }

    fn metrics_responder(url: &str) -> Value {
        let filter = url
            .split("filter=openalex:")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or_default();
        let results: Vec<Value> = filter
            .split('|')
            .map(|id| serde_json::json!({"id": format!("https://openalex.org/{id}"), "cited_by_count": 7}))
            .collect();
        serde_json::json!({ "results": results })
    }

    #[tokio::test]
    async fn should_batch_uncached_ids_and_reuse_cache_when_fetching_counts() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let marker = std::process::id();
        let ids: Vec<String> = (0..60).map(|n| format!("W88{marker}{n:03}")).collect();
        let transport = fake::RecordingTransport::new(metrics_responder);
        let client = OpenAlexClient::new(transport.clone());
        let first = fetch_cited_by_counts_cached(&records, &client, &ids, false)
            .await
            .expect("first");
        assert_eq!(first.requests, 2);
        assert_eq!(first.counts.len(), 60);
        assert!(first.counts.iter().all(|(_, count)| *count == 7));
        let second = fetch_cited_by_counts_cached(&records, &client, &ids, false)
            .await
            .expect("second");
        assert_eq!(second.requests, 0);
        assert_eq!(second.counts.len(), 60);
        assert_eq!(transport.urls().len(), 2);
    }

    #[tokio::test]
    async fn should_refetch_when_refresh_requested() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let marker = std::process::id();
        let ids: Vec<String> = (0..3).map(|n| format!("W89{marker}{n:03}")).collect();
        let client = OpenAlexClient::new(fake::RecordingTransport::new(metrics_responder));
        fetch_cited_by_counts_cached(&records, &client, &ids, false)
            .await
            .expect("prime");
        let again = fetch_cited_by_counts_cached(&records, &client, &ids, true)
            .await
            .expect("refresh");
        assert_eq!(again.requests, 1);
        assert_eq!(again.counts.len(), 3);
    }

    #[tokio::test]
    async fn should_not_cache_invalid_or_omitted_counts_when_fetching() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let records = SourceRecordRepository::new(pool);
        let marker = std::process::id();
        let good = format!("W90{marker}001");
        let negative = format!("W90{marker}002");
        let omitted = format!("W90{marker}003");
        let (good_c, negative_c) = (good.clone(), negative.clone());
        let client = OpenAlexClient::new(fake::RecordingTransport::new(move |_| {
            serde_json::json!({"results": [
                {"id": good_c, "cited_by_count": 4},
                {"id": negative_c, "cited_by_count": -3},
                {"id": "W1", "cited_by_count": 9},
            ]})
        }));
        let ids = [good.clone(), negative.clone(), omitted.clone()];
        let fetched = fetch_cited_by_counts_cached(&records, &client, &ids, false)
            .await
            .expect("fetch");
        assert_eq!(fetched.counts, vec![(good.to_ascii_lowercase(), 4)]);
        for id in [&negative, &omitted] {
            let found = records
                .find(OPENALEX_METRICS_KIND, &source_key(id))
                .await
                .expect("find");
            assert!(found.is_none(), "{id} must not be cached");
        }
    }
}
