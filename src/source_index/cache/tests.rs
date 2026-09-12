use super::*;
use crate::source_index::{
    assemble,
    test_support::{cache_fixture, has_pdftotext, native_pdf},
};

#[tokio::test]
async fn permanent_cache_validates_source_schema_and_legacy_generations() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    let directory = root.path().join("cache");
    tokio::fs::write(&source, b"source identity").await.unwrap();
    cache_fixture(&source, &directory).await;
    let first = load_cached(&source, &directory).await.unwrap().unwrap();
    assert_eq!(
        first.etag,
        load_cached(&source, &directory)
            .await
            .unwrap()
            .unwrap()
            .etag
    );
    let path = directory.join("reading-index.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("generation");
    tokio::fs::write(&path, serde_json::to_vec(&legacy).unwrap())
        .await
        .unwrap();
    assert!(load_cached(&source, &directory).await.unwrap().is_some());
    legacy["index"]["schema_version"] = serde_json::json!(SCHEMA_VERSION - 1);
    tokio::fs::write(&path, serde_json::to_vec(&legacy).unwrap())
        .await
        .unwrap();
    assert!(load_cached(&source, &directory).await.unwrap().is_none());
    cache_fixture(&source, &directory).await;
    tokio::fs::write(&source, b"a different source identity")
        .await
        .unwrap();
    assert!(load_cached(&source, &directory).await.unwrap().is_none());
}

#[tokio::test]
async fn refresh_regenerates_validator_while_normal_reads_remain_stable() {
    if !has_pdftotext() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    let directory = root.path().join("cache");
    tokio::fs::write(&source, native_pdf("This readable native paragraph has enough words to avoid optical character recognition.")).await.unwrap();
    let first = load_or_build_priority(&source, &directory, false, BuildPriority::Background)
        .await
        .unwrap();
    let cached = load_or_build_priority(&source, &directory, false, BuildPriority::Interactive)
        .await
        .unwrap();
    assert_eq!(first.etag, cached.etag);
    let refreshed = load_or_build_priority(&source, &directory, true, BuildPriority::Interactive)
        .await
        .unwrap();
    assert_ne!(first.etag, refreshed.etag);
    assert_eq!(first.index.text, refreshed.index.text);
    assert_eq!(
        refreshed.etag,
        load_cached(&source, &directory)
            .await
            .unwrap()
            .unwrap()
            .etag
    );
}

#[tokio::test]
async fn a_source_changed_during_build_is_never_persisted_or_served_as_current() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    let directory = root.path().join("cache");
    tokio::fs::write(&source, b"old source").await.unwrap();
    tokio::fs::create_dir(&directory).await.unwrap();
    let before_build = source_stamp(&source).await.unwrap();
    tokio::fs::write(&source, b"changed source while extracting")
        .await
        .unwrap();
    let error = persist(&source, &directory, before_build, assemble(&[]))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("changed while"));
    assert!(!directory.join("reading-index.json").exists());
}
