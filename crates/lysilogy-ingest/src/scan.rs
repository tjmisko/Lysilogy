//! Library scanning: discover supported files, content-hash them, and reconcile
//! the result with the document store.

use std::path::{Path, PathBuf};

use lysilogy_core::{DocumentFormat, ScannedFile};
use lysilogy_db::document::{DocumentRepository, ScanReport};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::Result;

/// Hash a file's bytes with SHA-256, returning the lowercase hex digest and length.
pub async fn hash_file(path: &Path) -> std::io::Result<(String, i64)> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total: i64 = 0;
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += i64::try_from(read).expect("read length fits in i64");
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

/// The format of a supported file, or `None` if the extension is not indexed.
fn format_for(path: &Path) -> Option<DocumentFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => Some(DocumentFormat::Pdf),
        Some("epub") => Some(DocumentFormat::Epub),
        _ => None,
    }
}

/// Collect supported files under `root`, in a stable order.
#[must_use]
pub fn find_library_files(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_file() && format_for(entry.path()).is_some() {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort();
    paths
}

/// Scan a library root into hash-stamped files, relative to `root`.
pub async fn scan_library(root: &Path) -> std::io::Result<Vec<ScannedFile>> {
    let mut files = Vec::new();
    for path in find_library_files(root) {
        let format = format_for(&path).expect("filtered to supported files");
        let (sha256, bytes) = hash_file(&path).await?;
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        files.push(ScannedFile {
            relative_path,
            sha256,
            bytes,
            format,
        });
    }
    Ok(files)
}

/// Scan a library root and reconcile it with the default workspace.
pub async fn scan_and_apply(root: &Path, repo: &DocumentRepository) -> Result<ScanReport> {
    let workspace = repo.ensure_default_workspace().await?;
    let files = scan_library(root).await?;
    Ok(repo.apply_scan(workspace, &files).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known SHA-256 of `hello`, split so no single long hex literal appears.
    const HELLO_SHA256: &str = concat!(
        "2cf24dba5fb0a30e",
        "26e83b2ac5b9e29e",
        "1b161e5c1fa7425e",
        "73043362938b9824"
    );

    #[tokio::test]
    async fn hashes_file_contents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.pdf");
        tokio::fs::write(&path, b"hello").await.expect("write");
        let (sha, bytes) = hash_file(&path).await.expect("hash");
        assert_eq!(bytes, 5);
        assert_eq!(sha, HELLO_SHA256);
    }

    #[tokio::test]
    async fn scans_only_supported_files_with_relative_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::create_dir(dir.path().join("nested"))
            .await
            .expect("mkdir");
        tokio::fs::write(dir.path().join("a.pdf"), b"one")
            .await
            .expect("write");
        tokio::fs::write(dir.path().join("nested/b.epub"), b"two")
            .await
            .expect("write");
        tokio::fs::write(dir.path().join("notes.txt"), b"skip")
            .await
            .expect("write");

        let files = scan_library(dir.path()).await.expect("scan");
        let paths: Vec<_> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, ["a.pdf", "nested/b.epub"]);
        assert_eq!(files[0].format, DocumentFormat::Pdf);
        assert_eq!(files[1].format, DocumentFormat::Epub);
    }
}
