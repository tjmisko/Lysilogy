use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::{Mutex, OwnedMutexGuard, watch};

use crate::{
    Error, Result,
    source_index::{self, BuildPriority, IndexDocument},
};

type SharedResult = std::result::Result<Arc<IndexDocument>, Arc<Error>>;

#[derive(Debug)]
struct Job {
    interactive: Arc<AtomicBool>,
    result: watch::Receiver<Option<SharedResult>>,
}

#[derive(Clone, Debug, Default)]
pub(in crate::api) struct IndexJobs {
    jobs: Arc<Mutex<HashMap<PathBuf, Job>>>,
}

impl IndexJobs {
    pub(super) async fn get_or_build(
        &self,
        source: PathBuf,
        directory: PathBuf,
        refresh: bool,
        priority: BuildPriority,
        extraction: Arc<Mutex<()>>,
    ) -> Result<Arc<IndexDocument>> {
        if !refresh && let Some(cached) = source_index::load_cached(&source, &directory).await? {
            return Ok(Arc::new(cached));
        }
        let mut jobs = self.jobs.lock().await;
        let mut result = if let Some(job) = jobs.get(&directory) {
            if priority == BuildPriority::Interactive {
                job.interactive.store(true, Ordering::Relaxed);
            }
            job.result.clone()
        } else {
            // A previous task may have persisted its result since the first check.
            if !refresh && let Some(cached) = source_index::load_cached(&source, &directory).await?
            {
                return Ok(Arc::new(cached));
            }
            let (sender, receiver) = watch::channel(None);
            let interactive = Arc::new(AtomicBool::new(priority == BuildPriority::Interactive));
            jobs.insert(
                directory.clone(),
                Job {
                    interactive: interactive.clone(),
                    result: receiver.clone(),
                },
            );
            self.spawn(source, directory, extraction, interactive, sender);
            receiver
        };
        drop(jobs);
        loop {
            let ready = result.borrow().clone();
            if let Some(result) = ready {
                return result.map_err(|error| copy_build_error(&error));
            }
            result.changed().await.map_err(|_| {
                Error::Task("Reading-index worker stopped before publishing its result.".into())
            })?;
        }
    }

    fn spawn(
        &self,
        source: PathBuf,
        directory: PathBuf,
        extraction: Arc<Mutex<()>>,
        interactive: Arc<AtomicBool>,
        result: watch::Sender<Option<SharedResult>>,
    ) {
        let jobs = self.jobs.clone();
        tokio::spawn(async move {
            let build_directory = directory.clone();
            // Detached from HTTP lifetimes; the outer task also cleans up after a panic.
            let worker = tokio::spawn(async move {
                let (_guard, priority) = acquire(extraction, &interactive).await;
                source_index::load_or_build_priority(&source, &build_directory, true, priority)
                    .await
            });
            let built = worker.await.unwrap_or_else(|error| {
                Err(Error::Task(format!("Reading-index worker failed: {error}")))
            });
            // Only active waiters retain the result in memory. Disk remains the cache.
            let mut jobs = jobs.lock().await;
            result.send_replace(Some(built.map(Arc::new).map_err(Arc::new)));
            jobs.remove(&directory);
        });
    }
}

async fn acquire(
    extraction: Arc<Mutex<()>>,
    interactive: &AtomicBool,
) -> (OwnedMutexGuard<()>, BuildPriority) {
    loop {
        if interactive.load(Ordering::Relaxed) {
            return (extraction.lock_owned().await, BuildPriority::Interactive);
        }
        // Tokio's fair mutex gives queued foreground callers precedence over try_lock.
        if let Ok(guard) = extraction.clone().try_lock_owned() {
            return (guard, BuildPriority::Background);
        }
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
}

fn copy_build_error(error: &Error) -> Error {
    match error {
        Error::ProgramUnavailable(program) => Error::ProgramUnavailable(program.clone()),
        Error::InvalidRequest(message) => Error::InvalidRequest(message.clone()),
        Error::PaperNotFound(id) => Error::PaperNotFound(id.clone()),
        _ => Error::Task(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
