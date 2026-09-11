use std::{sync::Arc, time::Duration};

use reqwest::{
    Url,
    header::{ACCEPT, AUTHORIZATION, HeaderValue, RETRY_AFTER},
    redirect::Policy,
};
use serde_json::Value;
use tokio::{sync::Mutex, time::Instant};

use super::{FailureKind, GraphFailure, GraphFuture, GraphResult, Provider};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub struct Request {
    pub provider: Provider,
    pub url: Url,
}
impl Request {
    pub(super) fn new(provider: Provider, base: &str, segments: &[&str]) -> Self {
        let mut url = Url::parse(base).expect("static provider URL");
        url.path_segments_mut()
            .expect("HTTPS URL")
            .pop_if_empty()
            .extend(segments);
        Self { provider, url }
    }
    pub(super) fn query(mut self, key: &str, value: &str) -> Self {
        self.url.query_pairs_mut().append_pair(key, value);
        self
    }
}

pub trait Transport: Send + Sync {
    fn get(&self, request: Request) -> GraphFuture<'_, GraphResult<Value>>;
}

/// Credentials are read from process configuration, never from files or serialized/debugged.
#[derive(Clone)]
pub struct GraphHttp {
    client: reqwest::Client,
    next_request: [Arc<Mutex<Instant>>; 4],
    openalex_key: Option<String>,
    semantic_key: Option<String>,
    opencitations_token: Option<String>,
    mailto: Option<String>,
}
impl std::fmt::Debug for GraphHttp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GraphHttp { credentials: [redacted] }")
    }
}
impl GraphHttp {
    pub fn from_environment() -> crate::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("Lysilogy/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| crate::Error::Task("Could not create citation HTTP client".to_owned()))?;
        Ok(Self {
            client,
            next_request: std::array::from_fn(|_| Arc::new(Mutex::new(Instant::now()))),
            openalex_key: std::env::var("LYSILOGY_OPENALEX_API_KEY").ok(),
            semantic_key: std::env::var("LYSILOGY_SEMANTIC_SCHOLAR_API_KEY").ok(),
            opencitations_token: std::env::var("LYSILOGY_OPENCITATIONS_TOKEN").ok(),
            mailto: std::env::var("LYSILOGY_CITATION_MAILTO").ok(),
        })
    }

    fn prepare(&self, mut request: Request) -> GraphResult<reqwest::RequestBuilder> {
        let expected_host = match request.provider {
            Provider::Openalex => "api.openalex.org",
            Provider::SemanticScholar => "api.semanticscholar.org",
            Provider::Opencitations => "api.opencitations.net",
            Provider::Crossref => "api.crossref.org",
        };
        if request.url.scheme() != "https"
            || request.url.host_str() != Some(expected_host)
            || request.url.port().is_some()
            || !request.url.username().is_empty()
            || request.url.password().is_some()
        {
            return Err(GraphFailure::invalid(
                "Citation requests must use the fixed provider HTTPS endpoint",
            ));
        }
        if let Some(key) = &self.openalex_key
            && request.provider == Provider::Openalex
        {
            request.url.query_pairs_mut().append_pair("api_key", key);
        }
        if let Some(mailto) = &self.mailto
            && matches!(request.provider, Provider::Crossref | Provider::Openalex)
        {
            request.url.query_pairs_mut().append_pair("mailto", mailto);
        }
        let mut builder = self
            .client
            .get(request.url)
            .header(ACCEPT, "application/json");
        let credential = match request.provider {
            Provider::SemanticScholar => self.semantic_key.as_ref().map(|key| ("x-api-key", key)),
            Provider::Opencitations => self
                .opencitations_token
                .as_ref()
                .map(|key| (AUTHORIZATION.as_str(), key)),
            _ => None,
        };
        if let Some((name, value)) = credential {
            let mut header = HeaderValue::from_str(value).map_err(|_| {
                GraphFailure::new(
                    FailureKind::Unauthorized,
                    "Invalid provider credential configuration",
                )
            })?;
            header.set_sensitive(true);
            builder = builder.header(name, header);
        }
        Ok(builder)
    }

    async fn request(&self, request: Request) -> GraphResult<Value> {
        let provider = request.provider;
        let builder = self.prepare(request)?;
        // Per-provider throttles are shared by concurrent API requests. Long Retry-After
        // cooldowns fail immediately; they never sleep or block another provider.
        let provider_index = match provider {
            Provider::Openalex => 0,
            Provider::SemanticScholar => 1,
            Provider::Opencitations => 2,
            Provider::Crossref => 3,
        };
        let mut slot = self.next_request[provider_index].lock().await;
        let remaining = slot.saturating_duration_since(Instant::now());
        if remaining > Duration::from_millis(1_100) {
            let mut failure = GraphFailure::new(
                FailureKind::RateLimited,
                "Citation provider is cooling down after Retry-After",
            );
            failure.retry_after_seconds = Some(remaining.as_secs() + 1);
            return Err(failure);
        }
        tokio::time::sleep_until(*slot).await;
        *slot = Instant::now() + Duration::from_millis(1_100);
        // Never format reqwest errors: they can contain the authenticated URL.
        let mut response = builder.send().await.map_err(|_| {
            GraphFailure::new(
                FailureKind::Unavailable,
                "Citation provider request failed or timed out",
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            let kind = match status.as_u16() {
                401 | 403 => FailureKind::Unauthorized,
                404 => FailureKind::NotFound,
                429 => FailureKind::RateLimited,
                _ => FailureKind::Unavailable,
            };
            let mut failure = GraphFailure::new(
                kind,
                format!("Citation provider returned HTTP {}", status.as_u16()),
            );
            failure.retry_after_seconds = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(retry_after);
            if let Some(seconds) = failure.retry_after_seconds {
                *slot = Instant::now() + Duration::from_secs(seconds.min(86_400));
            }
            return Err(failure);
        }
        drop(slot);
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(GraphFailure::new(
                FailureKind::InvalidResponse,
                "Citation response exceeds 8 MiB limit",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            GraphFailure::new(FailureKind::Unavailable, "Citation response interrupted")
        })? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(GraphFailure::new(
                    FailureKind::InvalidResponse,
                    "Citation response exceeds 8 MiB limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| GraphFailure::malformed())
    }
}
impl Transport for GraphHttp {
    fn get(&self, request: Request) -> GraphFuture<'_, GraphResult<Value>> {
        Box::pin(self.request(request))
    }
}

fn retry_after(value: &str) -> Option<u64> {
    value.parse().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .and_then(|date| {
                u64::try_from(
                    (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                        .num_seconds()
                        .max(0),
                )
                .ok()
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client() -> GraphHttp {
        GraphHttp {
            client: reqwest::Client::new(),
            next_request: std::array::from_fn(|_| Arc::new(Mutex::new(Instant::now()))),
            openalex_key: Some("private-openalex-key".to_owned()),
            semantic_key: Some("private-semantic-key".to_owned()),
            opencitations_token: Some("private-oc-token".to_owned()),
            mailto: None,
        }
    }
    #[tokio::test]
    async fn retry_after_cooldowns_are_shared_provider_local_and_fail_fast() {
        let client = client();
        *client.next_request[1].lock().await = Instant::now() + Duration::from_secs(3_600);
        let shared = client.clone();
        let request = Request::new(
            Provider::SemanticScholar,
            "https://api.semanticscholar.org/graph/v1/",
            &["paper", "example"],
        );
        let failure = tokio::time::timeout(Duration::from_millis(50), shared.get(request))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert!(failure.retry_after_seconds.unwrap() >= 3_599);
        assert!(
            client.next_request[0]
                .lock()
                .await
                .saturating_duration_since(Instant::now())
                .is_zero()
        );
        assert!(!format!("{client:?}").contains("private-openalex-key"));
    }
    #[test]
    fn credentials_are_provider_specific_and_urls_cannot_change_hosts() {
        let client = client();
        let wrong = Request::new(
            Provider::Crossref,
            "https://example.com/",
            &["works", "identifier"],
        );
        assert!(client.prepare(wrong).is_err());
        let request = Request::new(
            Provider::Openalex,
            "https://api.openalex.org/",
            &["works", "W1"],
        );
        let authenticated = client.prepare(request).unwrap().build().unwrap();
        assert!(
            authenticated
                .url()
                .query_pairs()
                .any(|(key, value)| key == "api_key" && value == "private-openalex-key")
        );
        assert!(!authenticated.headers().contains_key("x-api-key"));
        let request = Request::new(
            Provider::SemanticScholar,
            "https://api.semanticscholar.org/graph/v1/",
            &["paper", "id"],
        );
        let authenticated = client.prepare(request).unwrap().build().unwrap();
        assert!(authenticated.url().query().is_none());
        assert!(authenticated.headers()["x-api-key"].is_sensitive());
    }
    #[test]
    fn retry_after_parses_seconds_and_http_dates() {
        assert_eq!(retry_after("120"), Some(120));
        assert_eq!(retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), Some(0));
        assert_eq!(retry_after("not a retry interval"), None);
    }
}
