use super::*;
use crate::source_index::test_support::{cache_fixture, has_pdftotext, native_pdf};

async fn wait_for_jobs(jobs: &IndexJobs, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if jobs.jobs.lock().await.len() == count {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn valid_cache_does_not_wait_for_extraction_capacity() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    let directory = root.path().join("cache");
    tokio::fs::write(&source, b"cached source fixture")
        .await
        .unwrap();
    cache_fixture(&source, &directory).await;
    let extraction = Arc::new(Mutex::new(()));
    let _guard = extraction.lock().await;
    let jobs = IndexJobs::default();
    let cached = tokio::time::timeout(
        Duration::from_millis(250),
        jobs.get_or_build(
            source,
            directory,
            false,
            BuildPriority::Background,
            extraction.clone(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(cached.index.text, "A permanently cached source index.");
    assert!(jobs.jobs.lock().await.is_empty());
}

#[tokio::test]
async fn concurrent_refreshes_join_and_build_survives_its_first_client_canceling() {
    if !has_pdftotext() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    let directory = root.path().join("cache");
    tokio::fs::write(
        &source,
        native_pdf("This native source has enough ordinary words for a complete reading index."),
    )
    .await
    .unwrap();
    let original = source_index::load_or_build_priority(
        &source,
        &directory,
        false,
        BuildPriority::Interactive,
    )
    .await
    .unwrap();
    let extraction = Arc::new(Mutex::new(()));
    let guard = extraction.lock().await;
    let jobs = IndexJobs::default();
    let request = |priority| {
        let (jobs, source, directory, extraction) = (
            jobs.clone(),
            source.clone(),
            directory.clone(),
            extraction.clone(),
        );
        tokio::spawn(async move {
            jobs.get_or_build(source, directory, true, priority, extraction)
                .await
                .unwrap()
        })
    };
    let first = request(BuildPriority::Background);
    wait_for_jobs(&jobs, 1).await;
    first.abort();
    let second = request(BuildPriority::Interactive);
    let third = request(BuildPriority::Background);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if jobs
                .jobs
                .lock()
                .await
                .get(&directory)
                .unwrap()
                .interactive
                .load(Ordering::Relaxed)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Give the third request a turn to join the pending build before releasing it.
    tokio::task::yield_now().await;
    drop(guard);
    let (second, third) = tokio::try_join!(second, third).unwrap();
    assert!(Arc::ptr_eq(&second, &third));
    assert_ne!(original.etag, second.etag);
    assert_eq!(
        source_index::load_cached(&source, &directory)
            .await
            .unwrap()
            .unwrap()
            .etag,
        second.etag
    );
    wait_for_jobs(&jobs, 0).await;
}

#[tokio::test]
async fn pending_background_work_yields_to_queued_foreground_extraction() {
    let extraction = Arc::new(Mutex::new(()));
    let held = extraction.lock().await;
    let background_lock = extraction.clone();
    let background =
        tokio::spawn(async move { acquire(background_lock, &AtomicBool::new(false)).await });
    let foreground_lock = extraction.clone();
    let foreground = tokio::spawn(async move { foreground_lock.lock_owned().await });
    tokio::task::yield_now().await;
    drop(held);
    let foreground = tokio::time::timeout(Duration::from_secs(1), foreground)
        .await
        .unwrap()
        .unwrap();
    assert!(!background.is_finished());
    drop(foreground);
    let (_, priority) = tokio::time::timeout(Duration::from_secs(1), background)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(priority, BuildPriority::Background);
}

#[tokio::test]
async fn an_interactive_join_promotes_pending_background_work() {
    let extraction = Arc::new(Mutex::new(()));
    let held = extraction.lock().await;
    let interactive = Arc::new(AtomicBool::new(false));
    let (worker_lock, worker_priority) = (extraction.clone(), interactive.clone());
    let worker = tokio::spawn(async move { acquire(worker_lock, &worker_priority).await });
    tokio::task::yield_now().await;
    interactive.store(true, Ordering::Relaxed);
    drop(held);
    let (_, priority) = tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(priority, BuildPriority::Interactive);
}

#[tokio::test]
async fn failed_jobs_release_their_slot_and_allow_retry() {
    if !has_pdftotext() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    let directory = root.path().join("cache");
    tokio::fs::write(&source, b"not a PDF").await.unwrap();
    let jobs = IndexJobs::default();
    let extraction = Arc::new(Mutex::new(()));
    assert!(
        jobs.get_or_build(
            source.clone(),
            directory.clone(),
            false,
            BuildPriority::Interactive,
            extraction.clone()
        )
        .await
        .is_err()
    );
    wait_for_jobs(&jobs, 0).await;
    tokio::fs::write(
        &source,
        native_pdf("A repaired source contains enough valid words to rebuild the reading index."),
    )
    .await
    .unwrap();
    assert!(
        jobs.get_or_build(
            source,
            directory,
            false,
            BuildPriority::Interactive,
            extraction
        )
        .await
        .is_ok()
    );
    wait_for_jobs(&jobs, 0).await;
}
