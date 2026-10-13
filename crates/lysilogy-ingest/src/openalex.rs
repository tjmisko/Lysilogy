//! A minimal `OpenAlex` client: fetch a Work by DOI or `OpenAlex` ID and read its
//! outbound `referenced_works`, with an optional Postgres-backed response cache.
//!
//! The transport is abstracted so parsing and caching can be tested offline; the
//! live [`ReqwestTransport`] reads credentials from the environment and is never
//! exercised by tests.

use std::env;
use std::future::Future;
use std::pin::Pin;

use lysilogy_core::Coverage;
use lysilogy_db::source_record::SourceRecordRepository;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

/// The `source_record.kind` used for cached `OpenAlex` work responses.
pub const OPENALEX_WORK_KIND: &str = "openalex_work";

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
    let key = source_key(id);
    if let Some(payload) = records.get(OPENALEX_WORK_KIND, &key).await?
        && let Ok(work) = parse_work(&payload)
    {
        return Ok(work);
    }
    let value = client.fetch_work_value(id).await?;
    records
        .upsert(OPENALEX_WORK_KIND, &key, Coverage::Complete, &value)
        .await?;
    parse_work(&value)
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
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            mailto: env::var("LYSILOGY_CITATION_MAILTO").ok(),
            api_key: env::var("LYSILOGY_OPENALEX_API_KEY").ok(),
        })
    }
}

impl JsonTransport for ReqwestTransport {
    fn get_json<'a>(&'a self, url: &'a str) -> ProviderFuture<'a, Result<Value, ProviderError>> {
        Box::pin(async move {
            let mut parsed = reqwest::Url::parse(url).map_err(|_| ProviderError::Malformed)?;
            {
                let mut pairs = parsed.query_pairs_mut();
                if let Some(mailto) = &self.mailto {
                    pairs.append_pair("mailto", mailto);
                }
                if let Some(key) = &self.api_key {
                    pairs.append_pair("api_key", key);
                }
            }
            let response = self
                .client
                .get(parsed)
                .send()
                .await
                .map_err(|_| ProviderError::Request("request failed or timed out".to_owned()))?;
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

fn short_openalex_id(value: &str) -> String {
    value
        .strip_prefix("https://openalex.org/")
        .unwrap_or(value)
        .to_owned()
}

fn normalize_doi(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    let doi = ["https://doi.org/", "http://doi.org/", "doi:"]
        .iter()
        .find_map(|prefix| lowered.strip_prefix(prefix))
        .map_or_else(|| lowered.clone(), str::to_owned);
    (!doi.is_empty()).then_some(doi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
        assert!(matches!(parse_work(&serde_json::json!({})), Err(ProviderError::Malformed)));
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
}
