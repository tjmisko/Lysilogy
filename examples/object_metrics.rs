//! Read-only bridge through production cached-index and deterministic objects APIs.
use lysilogy::{domain::PaperId, objects::ObjectsArtifact, source_index::load_cached};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use tokio::io::AsyncReadExt;

type Failure = Box<dyn std::error::Error>;
#[derive(Deserialize)]
struct Request {
    corpus_root: PathBuf,
    data_root: PathBuf,
    papers: Vec<FrozenInput>,
}
#[derive(Deserialize)]
struct FrozenInput {
    paper_id: PaperId,
    relative_path: String,
    index_sha256: String,
}
fn direct_pdf(value: &str) -> bool {
    let p = Path::new(value);
    p.components().count() == 1
        && matches!(p.components().next(), Some(Component::Normal(_)))
        && !value.starts_with('.')
        && !value.contains('\\')
        && p.extension().is_some_and(|v| v == "pdf")
}
fn safe_id(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn regular_file(path: &Path) -> Result<(), Failure> {
    for ancestor in path.ancestors() {
        if ancestor.symlink_metadata()?.file_type().is_symlink() {
            return Err("symlinked canonical input".into());
        }
    }
    if !path.symlink_metadata()?.file_type().is_file() {
        return Err("canonical input is not a regular file".into());
    }
    Ok(())
}
async fn bounded_index(path: &Path) -> Result<Vec<u8>, Failure> {
    regular_file(path)?;
    let file = tokio::fs::File::open(path).await?;
    if file.metadata().await?.len() > 32 * 1024 * 1024 {
        return Err("index exceeds 32 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("index grew beyond 32 MiB".into());
    }
    Ok(bytes)
}
#[tokio::main]
async fn main() -> Result<(), Failure> {
    let mut raw = String::new();
    io::stdin().take(1024 * 1024 + 1).read_to_string(&mut raw)?;
    if raw.len() > 1024 * 1024 {
        return Err("request exceeds 1 MiB".into());
    }
    let request: Request = serde_json::from_str(&raw)?;
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME missing")?);
    if request.corpus_root != home.join("Corpora/arxiv/pdf")
        || request.data_root != home.join(".cache/lysilogy/arxiv-kb-data")
        || request.papers.is_empty()
        || request.papers.len() > 1000
    {
        return Err("invalid measurement roots or population".into());
    }
    let mut rows = Vec::new();
    for paper in request.papers {
        if !safe_id(paper.paper_id.as_str()) || !direct_pdf(&paper.relative_path) {
            return Err("unsafe PDF name".into());
        }
        let directory = request
            .data_root
            .join("papers")
            .join(paper.paper_id.as_str());
        let path = directory.join("reading-index.json");
        let before = bounded_index(&path).await?;
        regular_file(&request.corpus_root.join(&paper.relative_path))?;
        if format!("{:x}", Sha256::digest(&before)) != paper.index_sha256 {
            return Err("index receipt differs".into());
        }
        let document = load_cached(&request.corpus_root.join(&paper.relative_path), &directory)
            .await?
            .ok_or("frozen index is not a valid production cache")?;
        if document.etag != format!("\"{}\"", paper.index_sha256)
            || bounded_index(&path).await? != before
        {
            return Err("index changed while loading".into());
        }
        let artifact = ObjectsArtifact::from_reading_index(&paper.paper_id, &document);
        let artifact_json = serde_json::to_string(&artifact)?;
        rows.push(json!({"paper_id":paper.paper_id,"index_sha256":paper.index_sha256,"object_sha256":format!("{:x}",Sha256::digest(artifact_json.as_bytes())),"artifact_json":artifact_json}));
    }
    println!(
        "{}",
        json!({"schema_version":1,"papers":rows,"network_calls":0,"model_calls":0})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn should_reject_unvalidated_ids_when_json_deserialization_bypasses_from_str() {
        for value in [
            "../outside",
            "/outside",
            "ABCDEF1234567890",
            "a",
            "aaaaaaaaaaaaaa/.",
        ] {
            assert!(!safe_id(value));
        }
        assert!(safe_id("0123456789abcdef"));
    }
    #[tokio::test]
    async fn should_reject_oversized_or_redirected_inputs_when_bridge_reads_directly() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("index.json");
        let handle = std::fs::File::create(&file).unwrap();
        handle.set_len(32 * 1024 * 1024 + 1).unwrap();
        assert!(bounded_index(&file).await.is_err());
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(bounded_index(&link).await.is_err());
    }
    #[test]
    fn should_reject_unsafe_names_when_bridge_reads_frozen_pdfs() {
        for value in [
            "../x.pdf", "/x.pdf", "a/x.pdf", "a\\x.pdf", ".x.pdf", "x.txt",
        ] {
            assert!(!direct_pdf(value));
        }
        assert!(direct_pdf("2104.01511v1.pdf"));
    }
}
