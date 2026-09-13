//! Expiring, content-verified public responses. Credentials never form cache identity.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{FailureKind, GraphFailure, GraphResult, Provider, Request};

pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = MAX_RESPONSE_BYTES + 4_096;

#[derive(Clone, Debug)]
pub struct CacheKey {
    provider: Provider,
    digest: String,
}
impl CacheKey {
    pub fn new(request: &Request, secrets: &[String]) -> GraphResult<Self> {
        if !request.url.username().is_empty() || request.url.password().is_some() {
            return Err(GraphFailure::invalid(
                "Credential-bearing URLs cannot form provider cache identities",
            ));
        }
        let public = public_url(&request.url);
        if contains_secret(public.as_str(), secrets) {
            return Err(GraphFailure::invalid(
                "Credential material cannot occur in a public provider lookup",
            ));
        }
        Ok(Self {
            provider: request.provider,
            digest: hash(format!("{}\n{public}", request.provider.slug()).as_bytes()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProviderCache {
    root: Arc<PathBuf>,
    ttl: Duration,
}
/// A verified public payload with the original provider-fetch timestamp.
#[derive(Clone, Debug)]
pub struct CachedResponse {
    pub value: Value,
    pub fetched_at_ms: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    schema_version: u32,
    key: String,
    created_ms: u64,
    expires_ms: u64,
    sha256: String,
    value: Value,
}
impl ProviderCache {
    #[must_use]
    pub fn new(root: PathBuf, ttl: Duration) -> Self {
        Self {
            root: Arc::new(root),
            ttl,
        }
    }
    fn path(&self, key: &CacheKey) -> PathBuf {
        self.root
            .join(key.provider.slug())
            .join(format!("{}.json", key.digest))
    }

    pub async fn get(
        &self,
        key: &CacheKey,
        now_ms: u64,
        secrets: &[String],
    ) -> GraphResult<Option<Value>> {
        Ok(self
            .get_with_provenance(key, now_ms, secrets)
            .await?
            .map(|response| response.value))
    }

    /// Preserve fetch time when a truth builder freezes a still-valid cache hit.
    pub async fn get_with_provenance(
        &self,
        key: &CacheKey,
        now_ms: u64,
        secrets: &[String],
    ) -> GraphResult<Option<CachedResponse>> {
        let Some(bytes) = read_bounded(&self.path(key), MAX_ENTRY_BYTES).await? else {
            return Ok(None);
        };
        let Ok(entry) = serde_json::from_slice::<Entry>(&bytes) else {
            return Ok(None);
        };
        if entry.schema_version != 1
            || entry.key != key.digest
            || entry.created_ms > now_ms
            || entry.expires_ms <= now_ms
            || entry.expires_ms <= entry.created_ms
            || now_ms.saturating_sub(entry.created_ms) >= millis(self.ttl)
        {
            return Ok(None);
        }
        let payload = serde_json::to_vec(&entry.value).map_err(|_| GraphFailure::malformed())?;
        if payload.len() > MAX_RESPONSE_BYTES
            || hash(&payload) != entry.sha256
            || !(entry.value.is_object() || entry.value.is_array())
        {
            return Ok(None);
        }
        Ok(Some(CachedResponse {
            value: sanitize(entry.value, secrets),
            fetched_at_ms: entry.created_ms,
        }))
    }

    pub async fn put(
        &self,
        key: &CacheKey,
        value: &Value,
        now_ms: u64,
        secrets: &[String],
    ) -> GraphResult<()> {
        let value = sanitize(value.clone(), secrets);
        let payload = serde_json::to_vec(&value).map_err(|_| GraphFailure::malformed())?;
        if payload.len() > MAX_RESPONSE_BYTES || !(value.is_object() || value.is_array()) {
            return Err(GraphFailure::malformed());
        }
        let entry = Entry {
            schema_version: 1,
            key: key.digest.clone(),
            created_ms: now_ms,
            expires_ms: now_ms.saturating_add(millis(self.ttl)),
            sha256: hash(&payload),
            value,
        };
        let bytes = serde_json::to_vec(&entry).map_err(|_| GraphFailure::malformed())?;
        write_durable(&self.path(key), &bytes).await
    }
}

pub(crate) fn public_url(url: &Url) -> Url {
    let mut public = url.clone();
    let mut pairs: Vec<(String, String)> = public
        .query_pairs()
        .filter(|(key, _)| !sensitive_key(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    public.set_query(None);
    public.set_fragment(None);
    if !pairs.is_empty() {
        public.query_pairs_mut().extend_pairs(pairs);
    }
    public
}
fn sensitive_key(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        key.as_str(),
        "apikey"
            | "xapikey"
            | "authorization"
            | "accesstoken"
            | "token"
            | "secret"
            | "password"
            | "credential"
            | "credentials"
            | "mailto"
            | "email"
    )
}
fn contains_secret(text: &str, secrets: &[String]) -> bool {
    let mut decoded = text.to_owned();
    for _ in 0..3 {
        if secrets
            .iter()
            .any(|secret| !secret.is_empty() && decoded.contains(secret))
        {
            return true;
        }
        let next = percent_encoding::percent_decode_str(&decoded)
            .decode_utf8_lossy()
            .into_owned();
        if next == decoded {
            break;
        }
        decoded = next;
    }
    false
}

pub(crate) fn credential_variants(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut result = Vec::new();
    for value in values.filter(|value| !value.is_empty()) {
        let mut encoded = Url::parse("https://redacted.invalid/").expect("static URL");
        encoded.query_pairs_mut().append_pair("value", &value);
        let query = encoded
            .query()
            .expect("query was added")
            .trim_start_matches("value=")
            .to_owned();
        let escaped =
            percent_encoding::utf8_percent_encode(&value, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        result.extend([
            value,
            query.clone(),
            query.replace('+', "%20"),
            escaped.clone(),
            percent_encoding::utf8_percent_encode(&escaped, percent_encoding::NON_ALPHANUMERIC)
                .to_string(),
        ]);
    }
    result.sort();
    result.dedup();
    result
}

/// Remove credential fields and echoed configured secrets even inside nested text or keys.
pub(crate) fn sanitize(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(key, _)| !sensitive_key(key) && !contains_secret(key, secrets))
                .map(|(key, value)| (key, sanitize(value, secrets)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| sanitize(value, secrets))
                .collect(),
        ),
        Value::String(text) => {
            if contains_secret(&text, secrets) {
                return Value::Null;
            }
            if let Ok(url) = Url::parse(&text)
                && matches!(url.scheme(), "https" | "http")
            {
                if !url.username().is_empty() || url.password().is_some() {
                    return Value::Null;
                }
                let mut clean = public_url(&url);
                clean.set_fragment(url.fragment());
                return Value::String(clean.to_string());
            }
            Value::String(text)
        }
        value => value,
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
fn storage_failure() -> GraphFailure {
    GraphFailure::new(
        FailureKind::Unavailable,
        "Provider storage could not be safely accessed",
    )
}

async fn reject_symlink_ancestors(path: &Path) -> GraphResult<()> {
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match tokio::fs::symlink_metadata(ancestor).await {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(storage_failure()),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(storage_failure()),
        }
    }
    Ok(())
}
pub(crate) async fn ensure_directory(path: &Path) -> GraphResult<()> {
    reject_symlink_ancestors(path).await?;
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|_| storage_failure())
}
pub(crate) async fn read_bounded(path: &Path, limit: usize) -> GraphResult<Option<Vec<u8>>> {
    reject_symlink_ancestors(path).await?;
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(storage_failure()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err(storage_failure());
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| storage_failure())?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| storage_failure())?;
    if bytes.len() > limit {
        return Err(storage_failure());
    }
    Ok(Some(bytes))
}
pub(crate) async fn write_durable(path: &Path, bytes: &[u8]) -> GraphResult<()> {
    let parent = path.parent().ok_or_else(storage_failure)?;
    ensure_directory(parent).await?;
    crate::store::write_atomic(path, bytes)
        .await
        .map_err(|_| storage_failure())?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .map_err(|_| storage_failure())?;
    file.flush().await.map_err(|_| storage_failure())?;
    file.sync_all().await.map_err(|_| storage_failure())?;
    tokio::fs::File::open(parent)
        .await
        .map_err(|_| storage_failure())?
        .sync_all()
        .await
        .map_err(|_| storage_failure())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> Request {
        Request::new(
            Provider::Openalex,
            "https://api.openalex.org/",
            &["works", "W1"],
        )
    }

    #[tokio::test]
    async fn should_serve_cached_response_when_entry_has_not_expired() {
        let root = tempfile::tempdir().unwrap();
        let cache = ProviderCache::new(root.path().to_owned(), Duration::from_secs(60));
        let key = CacheKey::new(&request(), &[]).unwrap();
        cache
            .put(&key, &json!({"id": "W1"}), 1_000, &[])
            .await
            .unwrap();
        assert_eq!(
            cache.get(&key, 2_000, &[]).await.unwrap(),
            Some(json!({"id": "W1"}))
        );
        assert!(cache.get(&key, 61_000, &[]).await.unwrap().is_none());
        assert!(cache.get(&key, 999, &[]).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn should_preserve_original_fetch_time_when_a_truth_builder_reads_a_cached_response() {
        let root = tempfile::tempdir().unwrap();
        let cache = ProviderCache::new(root.path().to_owned(), Duration::from_secs(60));
        let key = CacheKey::new(&request(), &[]).unwrap();
        cache
            .put(&key, &json!({"id": "W1"}), 1_000, &[])
            .await
            .unwrap();
        let response = cache
            .get_with_provenance(&key, 8_000, &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.fetched_at_ms, 1_000);
        assert_eq!(response.value, json!({"id": "W1"}));
        assert!(
            cache
                .get_with_provenance(&key, 61_000, &[])
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn should_ignore_cache_entry_when_its_payload_hash_changes() {
        let root = tempfile::tempdir().unwrap();
        let cache = ProviderCache::new(root.path().to_owned(), Duration::from_secs(60));
        let key = CacheKey::new(&request(), &[]).unwrap();
        cache
            .put(&key, &json!({"id": "W1"}), 1_000, &[])
            .await
            .unwrap();
        let mut value: Value =
            serde_json::from_slice(&tokio::fs::read(cache.path(&key)).await.unwrap()).unwrap();
        value["value"]["id"] = json!("wrong");
        tokio::fs::write(cache.path(&key), serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
        assert!(cache.get(&key, 2_000, &[]).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn should_omit_credentials_when_writing_keys_and_nested_response_echoes() {
        let root = tempfile::tempdir().unwrap();
        let cache = ProviderCache::new(root.path().to_owned(), Duration::from_secs(60));
        let secret = "private/provider+credential".to_owned();
        let secrets = credential_variants([secret.clone()].into_iter());
        let key = CacheKey::new(
            &request()
                .query("api_key", &secret)
                .query("mailto", "private@example.test"),
            &secrets,
        )
        .unwrap();
        let public_key = CacheKey::new(&request(), &secrets).unwrap();
        assert_eq!(key.digest, public_key.digest);
        let value = json!({"id": "W1", "nested": {"Authorization": "Bearer hidden", "api_key": secret, "echo": secrets[0]}, "url": "https://example.test/?token=unconfigured-secret&paper=1"});
        cache.put(&key, &value, 1_000, &secrets).await.unwrap();
        let serialized = tokio::fs::read_to_string(cache.path(&key)).await.unwrap();
        for forbidden in secrets.iter().map(String::as_str).chain([
            "Bearer hidden",
            "unconfigured-secret",
            "private@example.test",
        ]) {
            assert!(!serialized.contains(forbidden));
        }
        let cached = cache.get(&key, 2_000, &secrets).await.unwrap().unwrap();
        assert_eq!(cached["id"], "W1");
        assert!(cached["nested"]["echo"].is_null());
    }

    #[tokio::test]
    async fn should_remove_echoed_credential_when_percent_escapes_have_mixed_case() {
        let root = tempfile::tempdir().unwrap();
        let cache = ProviderCache::new(root.path().to_owned(), Duration::from_secs(60));
        let secrets = credential_variants(["private/provider+credential".to_owned()].into_iter());
        let key = CacheKey::new(&request(), &secrets).unwrap();
        let payload = json!({"id": "W1", "url": "https://example.test/?filter=private%2fprovider%2Bcredential", "double": "private%252fprovider%252bcredential"});
        cache.put(&key, &payload, 1_000, &secrets).await.unwrap();
        let cached = cache.get(&key, 2_000, &secrets).await.unwrap().unwrap();
        assert!(cached["url"].is_null());
        assert!(cached["double"].is_null());
        let bytes = tokio::fs::read_to_string(cache.path(&key)).await.unwrap();
        assert!(!bytes.contains("credential"));
    }

    #[tokio::test]
    async fn should_refuse_cache_io_when_an_intermediate_directory_is_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("responses")).unwrap();
        let cache = ProviderCache::new(root.path().join("responses"), Duration::from_secs(60));
        let key = CacheKey::new(&request(), &[]).unwrap();
        assert!(
            cache
                .put(&key, &json!({"id": "W1"}), 1_000, &[])
                .await
                .is_err()
        );
        assert!(cache.get(&key, 2_000, &[]).await.is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn should_reject_bytes_when_storage_entry_exceeds_its_read_bound() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("entry.json");
        tokio::fs::write(&path, b"123456789").await.unwrap();
        assert!(read_bounded(&path, 8).await.is_err());
    }

    #[test]
    fn should_keep_duplicate_query_order_when_repeated_parameters_may_change_meaning() {
        let a = CacheKey::new(
            &request().query("fields", "id").query("fields", "title"),
            &[],
        )
        .unwrap();
        let b = CacheKey::new(
            &request().query("fields", "title").query("fields", "id"),
            &[],
        )
        .unwrap();
        assert_ne!(a.digest, b.digest);
    }

    #[test]
    fn should_use_same_identity_when_public_query_parameter_order_changes() {
        let a = CacheKey::new(
            &request().query("fields", "id,title").query("limit", "10"),
            &[],
        )
        .unwrap();
        let b = CacheKey::new(
            &request().query("limit", "10").query("fields", "id,title"),
            &[],
        )
        .unwrap();
        assert_eq!(a.digest, b.digest);
        assert_ne!(
            a.digest,
            CacheKey::new(&request().query("limit", "11"), &[])
                .unwrap()
                .digest
        );
    }
}
