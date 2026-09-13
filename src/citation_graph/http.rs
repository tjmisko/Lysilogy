use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::{
    Url,
    header::{ACCEPT, AUTHORIZATION, HeaderValue, RETRY_AFTER},
    redirect::Policy,
};
use serde_json::Value;

use super::{
    FailureKind, GraphFailure, GraphFuture, GraphResult, Provider,
    budget::{BudgetPolicy, ProviderBudgets, now_ms},
    cache::{
        CacheKey, MAX_RESPONSE_BYTES, ProviderCache, credential_variants, public_url, sanitize,
    },
};

#[derive(Clone)]
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

/// Public response provenance for explicit, immutable truth snapshots.
#[derive(Clone, Debug)]
pub struct ProviderResponse {
    pub value: Value,
    pub fetched_at_ms: u64,
    pub cache_hit: bool,
}

/// Credentials are read from process configuration, never from files or serialized/debugged.
#[derive(Clone)]
pub struct GraphHttp {
    client: reqwest::Client,
    cache: ProviderCache,
    budgets: ProviderBudgets,
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
        let home = std::env::var_os("HOME").ok_or_else(|| {
            crate::Error::Task("HOME is required for shared provider storage".to_owned())
        })?;
        Self::from_environment_with_storage(
            &PathBuf::from(home).join(".cache/lysilogy/providers"),
            [BudgetPolicy::default(); 4],
        )
    }

    /// Alternate storage supports isolated tests or an explicitly shared service cache.
    pub fn from_environment_with_storage(
        root: &Path,
        policy: [BudgetPolicy; 4],
    ) -> crate::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("Lysilogy/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| crate::Error::Task("Could not create citation HTTP client".to_owned()))?;
        Ok(Self {
            client,
            cache: ProviderCache::new(root.join("responses"), Duration::from_secs(7 * 86_400)),
            budgets: ProviderBudgets::new(root.join("budgets"), policy)
                .map_err(|failure| crate::Error::Task(failure.message))?,
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
        request.url = public_url(&request.url);
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

    /// Uses exactly the ordinary cache, credential handling and shared admission budget.
    pub async fn get_with_provenance(&self, request: Request) -> GraphResult<ProviderResponse> {
        let provider = request.provider;
        let secrets = self.secrets();
        let key = CacheKey::new(&request, &secrets)?;
        let builder = self.prepare(request)?;
        if let Some(response) = self
            .cache
            .get_with_provenance(&key, now_ms(), &secrets)
            .await?
        {
            return Ok(ProviderResponse {
                value: response.value,
                fetched_at_ms: response.fetched_at_ms,
                cache_hit: true,
            });
        }
        let mut lease = self.budgets.acquire(provider).await?;
        // Another caller may have filled this lookup while we waited for admission.
        if let Some(response) = self
            .cache
            .get_with_provenance(&key, now_ms(), &secrets)
            .await?
        {
            return Ok(ProviderResponse {
                value: response.value,
                fetched_at_ms: response.fetched_at_ms,
                cache_hit: true,
            });
        }
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
                lease.cooldown(seconds).await?;
            }
            return Err(failure);
        }
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
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| GraphFailure::malformed())?;
        let value = sanitize(value, &secrets);
        if !(value.is_object() || value.is_array()) {
            return Err(GraphFailure::malformed());
        }
        let fetched_at_ms = now_ms();
        if self
            .cache
            .put(&key, &value, fetched_at_ms, &secrets)
            .await
            .is_err()
        {
            tracing::warn!("Provider response succeeded but its cache could not be persisted");
        }
        drop(lease);
        Ok(ProviderResponse {
            value,
            fetched_at_ms,
            cache_hit: false,
        })
    }

    fn secrets(&self) -> Vec<String> {
        credential_variants(
            [
                &self.openalex_key,
                &self.semantic_key,
                &self.opencitations_token,
                &self.mailto,
            ]
            .into_iter()
            .filter_map(Clone::clone),
        )
    }
}
impl Transport for GraphHttp {
    fn get(&self, request: Request) -> GraphFuture<'_, GraphResult<Value>> {
        Box::pin(async move {
            self.get_with_provenance(request)
                .await
                .map(|response| response.value)
        })
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
    fn client(root: &std::path::Path) -> GraphHttp {
        GraphHttp {
            client: reqwest::Client::new(),
            cache: ProviderCache::new(root.join("responses"), Duration::from_secs(60)),
            budgets: ProviderBudgets::new(root.join("budgets"), [BudgetPolicy::default(); 4])
                .unwrap(),
            openalex_key: Some("private-openalex-key".to_owned()),
            semantic_key: Some("private-semantic-key".to_owned()),
            opencitations_token: Some("private-oc-token".to_owned()),
            mailto: None,
        }
    }
    #[tokio::test]
    async fn should_fail_fast_for_one_provider_when_a_shared_retry_after_cooldown_exists() {
        let root = tempfile::tempdir().unwrap();
        let client = client(root.path());
        let mut lease = client
            .budgets
            .acquire(Provider::SemanticScholar)
            .await
            .unwrap();
        lease.cooldown(3_600).await.unwrap();
        drop(lease);
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
        assert!(client.budgets.acquire(Provider::Openalex).await.is_ok());
        assert!(!format!("{client:?}").contains("private-openalex-key"));
    }
    #[test]
    fn should_attach_provider_specific_credentials_when_a_fixed_host_request_is_prepared() {
        let root = tempfile::tempdir().unwrap();
        let client = client(root.path());
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
    #[tokio::test]
    async fn should_return_cached_data_without_network_when_provider_budget_is_cooling_down() {
        let root = tempfile::tempdir().unwrap();
        let client = client(root.path());
        let request = Request::new(
            Provider::Openalex,
            "https://api.openalex.org/",
            &["works", "W1"],
        );
        let key = CacheKey::new(&request, &client.secrets()).unwrap();
        client
            .cache
            .put(
                &key,
                &serde_json::json!({"id": "W1"}),
                now_ms(),
                &client.secrets(),
            )
            .await
            .unwrap();
        let mut lease = client.budgets.acquire(Provider::Openalex).await.unwrap();
        lease.cooldown(600).await.unwrap();
        drop(lease);
        assert_eq!(client.get(request).await.unwrap()["id"], "W1");
    }

    #[tokio::test]
    async fn should_return_verified_fetch_provenance_when_a_truth_snapshot_reuses_cached_metadata()
    {
        let root = tempfile::tempdir().unwrap();
        let client = client(root.path());
        let request = Request::new(
            Provider::Crossref,
            "https://api.crossref.org/",
            &["works", "10.1234/fixture"],
        );
        let key = CacheKey::new(&request, &client.secrets()).unwrap();
        let fetched_at_ms = now_ms() - 1_000;
        client
            .cache
            .put(
                &key,
                &serde_json::json!({"message": {"DOI": "10.1234/fixture"}}),
                fetched_at_ms,
                &client.secrets(),
            )
            .await
            .unwrap();
        let mut lease = client.budgets.acquire(Provider::Crossref).await.unwrap();
        lease.cooldown(600).await.unwrap();
        drop(lease);
        let response = client.get_with_provenance(request).await.unwrap();
        assert!(response.cache_hit);
        assert_eq!(response.fetched_at_ms, fetched_at_ms);
        assert_eq!(response.value["message"]["DOI"], "10.1234/fixture");
    }

    #[tokio::test]
    async fn should_consult_real_budget_when_a_cached_entry_has_expired() {
        let root = tempfile::tempdir().unwrap();
        let client = client(root.path());
        let request = Request::new(
            Provider::Openalex,
            "https://api.openalex.org/",
            &["works", "W1"],
        );
        let key = CacheKey::new(&request, &client.secrets()).unwrap();
        client
            .cache
            .put(
                &key,
                &serde_json::json!({"id": "W1"}),
                now_ms() - 60_000,
                &client.secrets(),
            )
            .await
            .unwrap();
        let mut lease = client.budgets.acquire(Provider::Openalex).await.unwrap();
        lease.cooldown(600).await.unwrap();
        drop(lease);
        assert_eq!(
            client.get(request).await.unwrap_err().kind,
            FailureKind::RateLimited
        );
    }
    #[test]
    fn should_parse_retry_after_when_seconds_or_http_dates_are_supplied() {
        assert_eq!(retry_after("120"), Some(120));
        assert_eq!(retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), Some(0));
        assert_eq!(retry_after("not a retry interval"), None);
    }
}
