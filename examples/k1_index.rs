//! Offline evaluation bridge: map immutable corpus PDFs and retain native index caches.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::Instant,
};

use lysilogy::{
    library::LibraryCatalog,
    source_index::{BuildPriority, load_or_build_priority},
    store::ArtifactStore,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

type Failure = Box<dyn std::error::Error>;

#[derive(Deserialize)]
struct Request {
    corpus_pdf_root: PathBuf,
    data_root: PathBuf,
    papers: Vec<Paper>,
}

#[derive(Deserialize)]
struct Paper {
    relative_path: String,
    pdf_sha256: String,
}

fn safe_paper(paper: &Paper) -> bool {
    let path = Path::new(&paper.relative_path);
    path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
        && !paper.relative_path.contains('\\')
        && !paper.relative_path.starts_with('.')
        && path
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
        && paper.pdf_sha256.len() == 64
        && paper
            .pdf_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn fingerprint(path: &Path) -> Result<String, Failure> {
    let mut input = tokio::fs::File::open(path).await?;
    if input.metadata().await?.len() > 512 * 1024 * 1024 {
        return Err("PDF exceeds the reading-index byte bound".into());
    }
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut digest = Sha256::new();
    let mut total = 0;
    loop {
        let read = input.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total += read;
        if total > 512 * 1024 * 1024 {
            return Err("PDF grew beyond the reading-index byte bound".into());
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

async fn run(request: Request) -> Result<Value, Failure> {
    if request.papers.is_empty() || request.papers.len() > 1000 {
        return Err("index bridge requires 1–1000 corpus papers".into());
    }
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unavailable")?);
    let corpus = request.corpus_pdf_root.canonicalize()?;
    let data = request.data_root.canonicalize()?;
    if !corpus.starts_with(home.join("Corpora").canonicalize()?)
        || !data.starts_with(home.join(".cache/lysilogy").canonicalize()?)
        || data.starts_with(&corpus)
        || corpus.starts_with(&data)
    {
        return Err("corpus and existing data roots must be separate external storage".into());
    }
    let mut seen = BTreeSet::new();
    for paper in &request.papers {
        if !safe_paper(paper) || !seen.insert(&paper.relative_path) {
            return Err("invalid or duplicate corpus paper request".into());
        }
        let source = corpus.join(&paper.relative_path);
        if !source.symlink_metadata()?.file_type().is_file()
            || source.canonicalize()?.parent() != Some(corpus.as_path())
        {
            return Err("corpus PDF path must be a direct regular file".into());
        }
    }
    let store = ArtifactStore::new(&data);
    let catalog = LibraryCatalog::scan(&corpus, &store).await?;
    if !catalog.identity_conflicts().is_empty() {
        return Err("corpus identities require explicit conflict resolution".into());
    }
    let papers: BTreeMap<_, _> = catalog
        .overviews()
        .into_iter()
        .map(|paper| (paper.relative_path.clone(), paper))
        .collect();
    let mut output = Vec::new();
    for request in request.papers {
        let started = Instant::now();
        let paper = papers
            .get(&request.relative_path)
            .ok_or("requested PDF was not mapped")?;
        if paper.content_hash.as_deref() != Some(&request.pdf_sha256) {
            return Err("mapped content hash disagrees with the frozen corpus receipt".into());
        }
        let source = corpus.join(&request.relative_path);
        if fingerprint(&source).await? != request.pdf_sha256 {
            return Err("source PDF changed before indexing".into());
        }
        let directory = store.paper_dir(&paper.id);
        let result =
            load_or_build_priority(&source, &directory, false, BuildPriority::Background).await;
        if fingerprint(&source).await? != request.pdf_sha256 {
            return Err("source PDF changed during indexing".into());
        }
        let mut row = json!({"paper_id": paper.id, "relative_path": request.relative_path,
                             "pdf_sha256": request.pdf_sha256, "wall_seconds": started.elapsed().as_secs_f64()});
        match result {
            Ok(document) => {
                let path = directory.join("reading-index.json");
                row["index"] = json!({"path": path.strip_prefix(&data)?.to_str().ok_or("cache path is not UTF-8")?,
                                      "sha256": format!("{:x}", Sha256::digest(tokio::fs::read(path).await?))});
                row["generation"] = json!(document.etag);
            }
            Err(error) => row["error"] = json!(error.to_string()),
        }
        output.push(row);
    }
    Ok(
        json!({"schema_version": 1, "corpus_pdf_root": corpus, "data_root": data,
              "identity_registry": "paper-identities.json", "papers": output,
              "model_calls": 0, "network_calls": 0}),
    )
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let mut input = String::new();
    io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 1024 * 1024 {
        return Err("index request exceeds 1 MiB".into());
    }
    let request: Request = serde_json::from_str(&input)?;
    println!("{}", run(request).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_reject_unsafe_papers_when_paths_or_receipt_hashes_are_invalid() {
        for name in [
            "../vault.pdf",
            "/absolute.pdf",
            "dir/file.pdf",
            ".hidden.pdf",
            "dir\\file.pdf",
        ] {
            assert!(!safe_paper(&Paper {
                relative_path: name.into(),
                pdf_sha256: "a".repeat(64)
            }));
        }
        assert!(!safe_paper(&Paper {
            relative_path: "paper.pdf".into(),
            pdf_sha256: "Z".repeat(64)
        }));
        assert!(safe_paper(&Paper {
            relative_path: "0812.5080v5.pdf".into(),
            pdf_sha256: "a".repeat(64)
        }));
    }
}
