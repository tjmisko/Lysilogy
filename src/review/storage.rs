//! Reviews as plain JSON files: `papers/<id>/reviews/<snapshot id>.json`.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::fs;

use super::Review;
use crate::{
    Error, Result,
    domain::PaperId,
    store::{ArtifactStore, write_atomic},
};

const REVIEWS_DIRECTORY: &str = "reviews";

/// The paper's current analysis as untyped JSON, or `None` before analysis.
pub async fn load_analysis_json(store: &ArtifactStore, id: &PaperId) -> Result<Option<Value>> {
    read_json(&store.paper_dir(id).join("analysis.json")).await
}

pub async fn load_review(
    store: &ArtifactStore,
    id: &PaperId,
    snapshot_id: &str,
) -> Result<Option<Review>> {
    read_json(&review_path(store, id, snapshot_id)?).await
}

pub async fn save_review(store: &ArtifactStore, review: &Review) -> Result<()> {
    let path = review_path(store, &review.paper_id, &review.snapshot.snapshot_id)?;
    let directory = reviews_dir(store, &review.paper_id);
    fs::create_dir_all(&directory)
        .await
        .map_err(|error| Error::io(&directory, error))?;
    let mut json = serde_json::to_vec_pretty(review)?;
    json.push(b'\n');
    write_atomic(&path, &json).await
}

/// Every review of one paper, newest first. Malformed files are skipped with
/// a warning so one bad edit cannot hide the rest.
pub async fn load_paper_reviews(store: &ArtifactStore, id: &PaperId) -> Result<Vec<Review>> {
    let directory = reviews_dir(store, id);
    let mut reviews = Vec::new();
    for path in json_files(&directory).await? {
        match read_json::<Review>(&path).await {
            Ok(Some(review)) => reviews.push(review),
            Ok(None) => {}
            Err(Error::Json(error)) => {
                tracing::warn!(path = %path.display(), %error, "ignoring malformed review");
            }
            Err(error) => return Err(error),
        }
    }
    reviews.sort_by_key(|review| std::cmp::Reverse(review.updated_at));
    Ok(reviews)
}

/// Every stored review in the data root, whether or not its paper is still
/// in the library, newest first.
pub async fn load_all_reviews(store: &ArtifactStore) -> Result<Vec<Review>> {
    let papers = store.root().join("papers");
    let mut entries = match fs::read_dir(&papers).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(&papers, error)),
    };
    let mut reviews = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| Error::io(&papers, error))?
    {
        let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<PaperId>().ok())
        else {
            continue;
        };
        reviews.extend(load_paper_reviews(store, &id).await?);
    }
    reviews.sort_by_key(|review| std::cmp::Reverse(review.updated_at));
    Ok(reviews)
}

fn reviews_dir(store: &ArtifactStore, id: &PaperId) -> PathBuf {
    store.paper_dir(id).join(REVIEWS_DIRECTORY)
}

fn review_path(store: &ArtifactStore, id: &PaperId, snapshot_id: &str) -> Result<PathBuf> {
    let valid = !snapshot_id.is_empty()
        && snapshot_id.len() <= 64
        && snapshot_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if !valid {
        return Err(Error::InvalidRequest(
            "invalid review snapshot id".to_owned(),
        ));
    }
    Ok(reviews_dir(store, id).join(format!("{snapshot_id}.json")))
}

async fn json_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = match fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(directory, error)),
    };
    let mut paths = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| Error::io(directory, error))?
    {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(path, error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(Error::from)
}
