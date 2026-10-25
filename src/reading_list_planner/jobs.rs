//! The registry of generation runs, mirroring `ExpansionJobs`: one run per goal at a time,
//! and a semaphore of one so runs share the provider politely.

use std::collections::HashMap;
use std::sync::Arc;

use lysilogy_core::GenerationId;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// One generation at a time: the model and the providers are metered and polite-use.
const MAX_CONCURRENT_JOBS: usize = 1;

/// Which goals have a run in flight.
#[derive(Clone, Debug)]
pub struct GenerationJobs {
    running: Arc<Mutex<HashMap<String, GenerationId>>>,
    permits: Arc<Semaphore>,
}

impl Default for GenerationJobs {
    fn default() -> Self {
        Self {
            running: Arc::default(),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS)),
        }
    }
}

impl GenerationJobs {
    /// Note that `id` runs for the goal key. `Err` carries the run already going for it.
    ///
    /// # Errors
    /// The generation that already holds the goal.
    pub async fn claim(&self, goal_key: &str, id: GenerationId) -> Result<(), GenerationId> {
        let existing = {
            let mut running = self.running.lock().await;
            let existing = running.get(goal_key).copied();
            if existing.is_none() {
                running.insert(goal_key.to_owned(), id);
            }
            existing
        };
        existing.map_or(Ok(()), Err)
    }

    /// Forget the run for the goal key.
    pub async fn release(&self, goal_key: &str) {
        self.running.lock().await.remove(goal_key);
    }

    /// Wait for the turn to run. The permit frees on drop.
    pub async fn turn(&self) -> Option<OwnedSemaphorePermit> {
        self.permits.clone().acquire_owned().await.ok()
    }

    /// The generation running for the goal key, if any.
    pub async fn running_for(&self, goal_key: &str) -> Option<GenerationId> {
        self.running.lock().await.get(goal_key).copied()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn should_refuse_a_second_run_when_the_goal_is_already_running() {
        let jobs = GenerationJobs::default();
        let first = GenerationId::new();
        jobs.claim("learn rlhf", first).await.expect("first");
        let second = GenerationId::new();
        assert_eq!(jobs.claim("learn rlhf", second).await, Err(first));
        jobs.claim("other goal", second).await.expect("other goal");
        jobs.release("learn rlhf").await;
        jobs.claim("learn rlhf", second)
            .await
            .expect("after release");
        assert_eq!(jobs.running_for("learn rlhf").await, Some(second));
    }

    #[tokio::test]
    async fn should_run_one_job_at_a_time_when_several_wait_for_a_turn() {
        let jobs = GenerationJobs::default();
        let permit = jobs.turn().await.expect("first turn");
        let waiting = jobs.clone();
        let second = tokio::spawn(async move { waiting.turn().await.is_some() });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!second.is_finished(), "the second job waits");
        drop(permit);
        assert!(second.await.expect("join"));
    }
}
