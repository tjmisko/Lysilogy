//! Shared request admission: fixed windows, spacing, and durable provider cooldowns.
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::{FailureKind, GraphFailure, GraphResult, Provider, cache};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetPolicy {
    pub spacing_ms: u64,
    pub window_ms: u64,
    pub requests_per_window: u32,
}
impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            spacing_ms: 1_100,
            window_ms: 60_000,
            requests_per_window: 50,
        }
    }
}
impl BudgetPolicy {
    pub fn validate(self) -> GraphResult<Self> {
        if self.spacing_ms < 1_100
            || self.window_ms < self.spacing_ms
            || self.requests_per_window == 0
        {
            return Err(GraphFailure::invalid(
                "Provider budgets require spacing >= 1100 ms and a positive window/quota",
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Admission {
    Granted,
    Wait { until_ms: u64 },
    Deferred { until_ms: u64 },
}

/// The same state machine drives HTTP admission and the offline 10k simulation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetState {
    pub window_start_ms: u64,
    pub used: u32,
    pub next_request_ms: u64,
    pub cooldown_until_ms: u64,
}
impl BudgetState {
    #[must_use]
    pub fn reserve(&mut self, now_ms: u64, policy: BudgetPolicy) -> Admission {
        if self.cooldown_until_ms > now_ms.saturating_add(policy.spacing_ms) {
            return Admission::Deferred {
                until_ms: self.cooldown_until_ms,
            };
        }
        if now_ms >= self.window_start_ms.saturating_add(policy.window_ms) {
            self.window_start_ms = now_ms;
            self.used = 0;
        }
        if self.used >= policy.requests_per_window {
            return Admission::Deferred {
                until_ms: self.window_start_ms.saturating_add(policy.window_ms),
            };
        }
        let until_ms = self.next_request_ms.max(self.cooldown_until_ms);
        if until_ms > now_ms.saturating_add(policy.spacing_ms) {
            return Admission::Deferred { until_ms };
        }
        if now_ms < until_ms {
            return Admission::Wait { until_ms };
        }
        self.used += 1;
        self.next_request_ms = now_ms.saturating_add(policy.spacing_ms);
        Admission::Granted
    }

    pub fn cooldown(&mut self, now_ms: u64, seconds: u64) {
        self.cooldown_until_ms = self
            .cooldown_until_ms
            .max(now_ms.saturating_add(seconds.saturating_mul(1_000)));
    }
}

#[derive(Clone, Debug)]
pub struct ProviderBudgets {
    root: Arc<PathBuf>,
    policy: [BudgetPolicy; 4],
    locks: [Arc<Mutex<()>>; 4],
}
impl ProviderBudgets {
    pub fn new(root: PathBuf, policy: [BudgetPolicy; 4]) -> GraphResult<Self> {
        for value in policy {
            value.validate()?;
        }
        Ok(Self {
            root: Arc::new(root),
            policy,
            locks: std::array::from_fn(|_| Arc::new(Mutex::new(()))),
        })
    }

    pub async fn acquire(&self, provider: Provider) -> GraphResult<BudgetLease> {
        let index = provider.index();
        let local = self.locks[index].clone().lock_owned().await;
        cache::ensure_directory(&self.root).await?;
        let lock_path = self.root.join(format!("{}.lock", provider.slug()));
        let connection = ConnectionLock::acquire(&lock_path, local)?;
        let path = self.root.join(format!("{}.json", provider.slug()));
        let mut lease = BudgetLease::load(path, connection).await?;
        loop {
            let now = now_ms();
            match lease.state.reserve(now, self.policy[index]) {
                Admission::Granted => {
                    lease.persist().await?;
                    return Ok(lease);
                }
                Admission::Wait { until_ms } => {
                    tokio::time::sleep(Duration::from_millis(until_ms.saturating_sub(now))).await;
                }
                Admission::Deferred { until_ms } => {
                    return Err(deferred(
                        until_ms.saturating_sub(now).div_ceil(1_000),
                        "Citation provider request budget is exhausted or cooling down",
                    ));
                }
            }
        }
    }
}

/// Owns the request's lock, even while persisted state is still being loaded.
struct ConnectionLock {
    file: File,
    _local: OwnedMutexGuard<()>,
}
impl ConnectionLock {
    fn acquire(path: &Path, local: OwnedMutexGuard<()>) -> GraphResult<Self> {
        let file = File::from(
            open(
                path,
                OFlags::CREATE | OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| storage_failure())?,
        );
        flock(&file, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| deferred(1, "Another request holds this provider's connection budget"))?;
        Ok(Self {
            file,
            _local: local,
        })
    }
}
impl Drop for ConnectionLock {
    fn drop(&mut self) {
        // Closing alone does not unlock while a pre-exec child (or duplicate)
        // retains the same open-file description. End ownership before the local
        // mutex is released; Drop must not panic if the kernel rejects unlock.
        let _ = flock(&self.file, FlockOperation::Unlock);
    }
}

/// Held through the entire HTTP response, preserving the provider connection limit.
pub struct BudgetLease {
    state: BudgetState,
    path: PathBuf,
    _connection: ConnectionLock,
}
impl BudgetLease {
    async fn load(path: PathBuf, connection: ConnectionLock) -> GraphResult<Self> {
        let saved = cache::read_bounded(&path, 4_096).await?;
        let state = match saved {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| storage_failure())?,
            None => BudgetState::default(),
        };
        Ok(Self {
            state,
            path,
            _connection: connection,
        })
    }

    pub async fn cooldown(&mut self, seconds: u64) -> GraphResult<()> {
        self.state.cooldown(now_ms(), seconds);
        self.persist().await
    }
    async fn persist(&self) -> GraphResult<()> {
        let bytes = serde_json::to_vec(&self.state).map_err(|_| storage_failure())?;
        cache::write_durable(&self.path, &bytes).await
    }
}

pub(crate) fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}
fn deferred(seconds: u64, message: &str) -> GraphFailure {
    let mut failure = GraphFailure::new(FailureKind::RateLimited, message);
    failure.retry_after_seconds = Some(seconds.max(1));
    failure
}
fn storage_failure() -> GraphFailure {
    GraphFailure::new(
        FailureKind::Unavailable,
        "Provider budget state could not be safely read or persisted",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_defer_requests_when_provider_window_is_exhausted() {
        let policy = BudgetPolicy {
            requests_per_window: 2,
            ..BudgetPolicy::default()
        };
        let mut state = BudgetState::default();
        assert_eq!(state.reserve(0, policy), Admission::Granted);
        assert_eq!(
            state.reserve(0, policy),
            Admission::Wait { until_ms: 1_100 }
        );
        assert_eq!(state.reserve(1_100, policy), Admission::Granted);
        assert_eq!(
            state.reserve(2_200, policy),
            Admission::Deferred { until_ms: 60_000 }
        );
        assert_eq!(state.reserve(60_000, policy), Admission::Granted);
    }

    #[test]
    fn should_keep_later_cooldown_when_a_shorter_retry_after_arrives() {
        let mut state = BudgetState::default();
        state.cooldown(1_000, 600);
        state.cooldown(2_000, 1);
        assert_eq!(
            state.reserve(3_000, BudgetPolicy::default()),
            Admission::Deferred { until_ms: 601_000 }
        );
    }

    #[tokio::test]
    async fn should_preserve_cooldown_when_a_new_budget_instance_restarts() {
        let root = tempfile::tempdir().unwrap();
        let budgets =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        let mut lease = budgets.acquire(Provider::Openalex).await.unwrap();
        lease.cooldown(600).await.unwrap();
        drop(lease);
        let restarted =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        let failure = restarted.acquire(Provider::Openalex).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert!(failure.retry_after_seconds.unwrap() >= 599);
        assert!(restarted.acquire(Provider::Crossref).await.is_ok());
    }

    #[tokio::test]
    #[allow(clippy::used_underscore_binding)] // Inspect the opaque guard solely to duplicate its descriptor.
    async fn should_expose_saved_cooldown_when_a_duplicate_outlives_the_request_lease() {
        let root = tempfile::tempdir().unwrap();
        let budgets =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        let mut lease = budgets.acquire(Provider::Openalex).await.unwrap();
        lease.cooldown(600).await.unwrap();
        // Like a child before exec, a duplicate shares the open-file description.
        let duplicate = lease._connection.file.try_clone().unwrap();
        drop(lease);
        let restarted =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        let failure = restarted.acquire(Provider::Openalex).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert!(failure.retry_after_seconds.unwrap() >= 599, "{failure:?}");
        assert!(failure.message.contains("cooling down"));
        drop(duplicate);
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)] // The guard moves into load; dropping it again is invalid.
    async fn should_release_duplicated_lock_when_budget_state_cannot_be_loaded() {
        for malformed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("openalex.json");
            if malformed {
                tokio::fs::write(&path, b"malformed JSON").await.unwrap();
            } else {
                tokio::fs::create_dir(&path).await.unwrap();
            }
            let lock_path = root.path().join("openalex.lock");
            let local = Arc::new(Mutex::new(()));
            let connection =
                ConnectionLock::acquire(&lock_path, local.clone().lock_owned().await).unwrap();
            let duplicate = connection.file.try_clone().unwrap();
            let failure = BudgetLease::load(path, connection).await.err().unwrap();
            assert_eq!(failure.kind, FailureKind::Unavailable);
            let _next = ConnectionLock::acquire(&lock_path, local.lock_owned().await).unwrap();
            drop(duplicate);
        }
    }

    #[tokio::test]
    async fn should_defer_other_instances_when_provider_connection_is_in_use() {
        let root = tempfile::tempdir().unwrap();
        let first =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        let second =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        let _lease = first.acquire(Provider::Crossref).await.unwrap();
        let failure = second.acquire(Provider::Crossref).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert!(second.acquire(Provider::Openalex).await.is_ok());
    }

    #[tokio::test]
    async fn should_admit_only_one_concurrent_request_when_window_quota_is_one() {
        let root = tempfile::tempdir().unwrap();
        let policy = BudgetPolicy {
            requests_per_window: 1,
            ..BudgetPolicy::default()
        };
        let budgets = ProviderBudgets::new(root.path().to_owned(), [policy; 4]).unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let shared = budgets.clone();
            tasks.spawn(async move { shared.acquire(Provider::Openalex).await.is_ok() });
        }
        let mut admitted = 0;
        while let Some(result) = tasks.join_next().await {
            admitted += usize::from(result.unwrap());
        }
        assert_eq!(admitted, 1);
    }

    #[tokio::test]
    async fn should_fail_closed_when_persisted_budget_is_corrupt() {
        let root = tempfile::tempdir().unwrap();
        tokio::fs::write(root.path().join("crossref.json"), b"corrupt")
            .await
            .unwrap();
        let budgets =
            ProviderBudgets::new(root.path().to_owned(), [BudgetPolicy::default(); 4]).unwrap();
        assert_eq!(
            budgets
                .acquire(Provider::Crossref)
                .await
                .err()
                .unwrap()
                .kind,
            FailureKind::Unavailable
        );
    }
}
