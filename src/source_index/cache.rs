use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use super::{
    BUILD_TIMEOUT, BuildPriority, CachedIndex, ReadingIndex, SCHEMA_VERSION, SourceStamp, build,
};
use crate::{Error, Result, store::write_atomic};

static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct IndexDocument {
    pub index: ReadingIndex,
    pub etag: String,
}

async fn source_stamp(source: &Path) -> Result<SourceStamp> {
    let metadata = tokio::fs::metadata(source)
        .await
        .map_err(|error| Error::io(source, error))?;
    if metadata.len() > 512 * 1024 * 1024 {
        return Err(Error::InvalidRequest(
            "PDF exceeds the 512 MiB reading-index limit".into(),
        ));
    }
    Ok(SourceStamp {
        bytes: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos()),
    })
}

/// A valid persisted index has no age limit. Check source and schema before reuse.
pub async fn load_cached(source: &Path, directory: &Path) -> Result<Option<IndexDocument>> {
    let stamp = source_stamp(source).await?;
    Ok(read_cached(directory, &stamp).await)
}

async fn read_cached(directory: &Path, stamp: &SourceStamp) -> Option<IndexDocument> {
    let bytes = tokio::fs::read(directory.join("reading-index.json"))
        .await
        .ok()?;
    let cached = serde_json::from_slice::<CachedIndex>(&bytes).ok()?;
    (cached.source == *stamp && cached.index.schema_version == SCHEMA_VERSION)
        .then(|| document(cached.index, &bytes))
}

/// Only local PDF processing. Refresh regenerates this cache, never paper analysis.
pub async fn load_or_build(source: &Path, directory: &Path, refresh: bool) -> Result<ReadingIndex> {
    Ok(
        load_or_build_priority(source, directory, refresh, BuildPriority::Interactive)
            .await?
            .index,
    )
}

pub async fn load_or_build_priority(
    source: &Path,
    directory: &Path,
    refresh: bool,
    priority: BuildPriority,
) -> Result<IndexDocument> {
    let stamp = source_stamp(source).await?;
    if !refresh && let Some(cached) = read_cached(directory, &stamp).await {
        return Ok(cached);
    }
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| Error::io(directory, error))?;
    let index = tokio::time::timeout(BUILD_TIMEOUT, build(source, directory, &stamp, priority))
        .await
        .map_err(|_| {
            Error::Task(
                "Reading index timed out after 180 seconds; retry to resume cached OCR pages."
                    .into(),
            )
        })??;
    persist(source, directory, stamp, index).await
}

async fn persist(
    source: &Path,
    directory: &Path,
    stamp: SourceStamp,
    index: ReadingIndex,
) -> Result<IndexDocument> {
    if source_stamp(source).await? != stamp {
        return Err(Error::Task(
            "PDF changed while its reading index was being built; retry with the current source."
                .into(),
        ));
    }
    let generation = format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        GENERATION.fetch_add(1, Ordering::Relaxed)
    );
    let cached = CachedIndex {
        source: stamp,
        generation,
        index,
    };
    let bytes = serde_json::to_vec(&cached)?;
    write_atomic(&directory.join("reading-index.json"), &bytes).await?;
    Ok(document(cached.index, &bytes))
}

fn document(index: ReadingIndex, persisted_bytes: &[u8]) -> IndexDocument {
    IndexDocument {
        index,
        etag: format!("\"{:x}\"", Sha256::digest(persisted_bytes)),
    }
}

#[cfg(test)]
mod tests;
