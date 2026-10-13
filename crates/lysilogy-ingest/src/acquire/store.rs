//! The content-addressed blob store and the library copy under `Fetched/`.

use std::io;
use std::path::{Path, PathBuf};

use lysilogy_core::WorkId;

use crate::scan::hash_file;

/// Directory inside the library that holds fetched copies.
pub const FETCHED_DIR: &str = "Fetched";

/// Longest file name stem, in characters (the `.pdf` suffix is extra).
const MAX_NAME_CHARS: usize = 150;

/// Most collision suffixes tried before giving up.
const MAX_COLLISION_SUFFIX: u32 = 1000;

/// The blob's key in `reader.blob`: `sha256/<sha[0..2]>/<sha>.pdf`.
#[must_use]
pub fn blob_storage_key(sha256: &str) -> String {
    let prefix = sha256.get(..2).unwrap_or(sha256);
    format!("sha256/{prefix}/{sha256}.pdf")
}

/// Where the blob lives on disk under `root`.
#[must_use]
pub fn blob_path(root: &Path, sha256: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(blob_storage_key(sha256).split('/'));
    path
}

/// Move a finished download into the store. When a blob with this hash already
/// exists it wins and the temp file is deleted.
pub async fn store_blob(blob_root: &Path, temp_path: &Path, sha256: &str) -> io::Result<PathBuf> {
    let target = blob_path(blob_root, sha256);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::try_exists(&target).await? {
        tokio::fs::remove_file(temp_path).await?;
        return Ok(target);
    }
    tokio::fs::rename(temp_path, &target).await?;
    Ok(target)
}

/// The library file name for a fetched paper.
///
/// `<year> - <title>.pdf`, else
/// `<title>.pdf`, else `<work id>.pdf`. Path separators, reserved characters and
/// control characters are removed, whitespace is collapsed, and the stem is
/// at most 150 characters.
#[must_use]
pub fn fetched_filename(title: Option<&str>, year: Option<i32>, work_id: WorkId) -> String {
    let title = title.map(sanitize_name_part).unwrap_or_default();
    let stem = match (title.is_empty(), year) {
        (true, _) => work_id.to_string(),
        (false, Some(year)) => format!("{year} - {title}"),
        (false, None) => title,
    };
    let stem = truncate_chars(&stem, MAX_NAME_CHARS);
    format!("{}.pdf", stem.trim_end_matches([' ', '.']))
}

fn sanitize_name_part(text: &str) -> String {
    let stripped: String = text
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
        .collect();
    let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_matches('.').trim().to_owned()
}

fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Put a real copy of the blob into `<library>/Fetched/` and return its
/// library-relative path (always `/`-separated).
///
/// An existing file with the same bytes is reused; a different file with the
/// same name pushes the copy to `name (2).pdf`, `name (3).pdf`, and so on. The
/// copy goes through a `.part` file and is renamed into place. It is a copy, not
/// a hard link, so edits to the library file never change the blob.
pub async fn place_library_copy(
    library_root: &Path,
    blob: &Path,
    filename: &str,
    sha256: &str,
) -> io::Result<String> {
    let directory = library_root.join(FETCHED_DIR);
    tokio::fs::create_dir_all(&directory).await?;
    let stem = filename.strip_suffix(".pdf").unwrap_or(filename);
    for suffix in 1..=MAX_COLLISION_SUFFIX {
        let name = if suffix == 1 {
            format!("{stem}.pdf")
        } else {
            format!("{stem} ({suffix}).pdf")
        };
        let target = directory.join(&name);
        if !tokio::fs::try_exists(&target).await? {
            copy_into_place(blob, &target).await?;
            return Ok(format!("{FETCHED_DIR}/{name}"));
        }
        let (existing_sha, _) = hash_file(&target).await?;
        if existing_sha == sha256 {
            return Ok(format!("{FETCHED_DIR}/{name}"));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free library name for {filename}"),
    ))
}

async fn copy_into_place(blob: &Path, target: &Path) -> io::Result<()> {
    let mut partial = target.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    if let Err(error) = tokio::fs::copy(blob, &partial).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::rename(&partial, target).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn should_shard_blob_key_and_path_when_given_a_hash() {
        let hash = "ab".repeat(32);
        assert_eq!(blob_storage_key(&hash), format!("sha256/ab/{hash}.pdf"));
        assert_eq!(
            blob_path(Path::new("/data/blobs"), &hash),
            PathBuf::from(format!("/data/blobs/sha256/ab/{hash}.pdf"))
        );
    }

    #[test]
    fn should_sanitize_fetched_filename_when_title_has_path_separators() {
        let work = WorkId::new();
        assert_eq!(
            fetched_filename(Some("A/B\\C: \"D\" <E>|F*G?"), Some(2016), work),
            "2016 - ABC D EFG.pdf"
        );
        assert_eq!(
            fetched_filename(Some("  Spaced \t\n out\u{7}  "), None, work),
            "Spaced out.pdf"
        );
        assert_eq!(
            fetched_filename(Some("../../etc/passwd"), None, work),
            "etcpasswd.pdf"
        );
        assert_eq!(
            fetched_filename(Some("Title."), Some(1999), work),
            "1999 - Title.pdf"
        );
    }

    #[test]
    fn should_truncate_fetched_filename_when_title_is_long() {
        let long = "word ".repeat(100);
        let name = fetched_filename(Some(&long), Some(2020), WorkId::new());
        let stem = name.strip_suffix(".pdf").expect("suffix");
        assert!(stem.chars().count() <= 150, "{}", stem.chars().count());
        assert!(stem.starts_with("2020 - word word"));
        assert!(!stem.ends_with(' '));

        let wide = "é".repeat(400);
        let name = fetched_filename(Some(&wide), None, WorkId::new());
        assert_eq!(name.chars().count(), 150 + 4, "truncates on characters");
    }

    #[test]
    fn should_fall_back_to_work_id_when_title_unknown() {
        let work = WorkId::new();
        let expected = format!("{work}.pdf");
        assert_eq!(fetched_filename(None, Some(2001), work), expected);
        assert_eq!(fetched_filename(Some("   "), None, work), expected);
        assert_eq!(fetched_filename(Some("///"), Some(2001), work), expected);
    }

    #[tokio::test]
    async fn should_rename_into_place_when_storing_blob() {
        let root = tempfile::tempdir().expect("root");
        let temp = root.path().join("tmp");
        std::fs::create_dir_all(&temp).expect("tmp");
        let partial = temp.join("x.part");
        std::fs::write(&partial, b"%PDF-body").expect("write");
        let hash = sha(b"%PDF-body");

        let stored = store_blob(root.path(), &partial, &hash)
            .await
            .expect("store");

        assert_eq!(stored, blob_path(root.path(), &hash));
        assert_eq!(std::fs::read(&stored).expect("blob"), b"%PDF-body");
        assert!(!partial.exists());
    }

    #[tokio::test]
    async fn should_delete_temp_when_blob_already_exists() {
        let root = tempfile::tempdir().expect("root");
        let hash = sha(b"%PDF-body");
        let target = blob_path(root.path(), &hash);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("dirs");
        std::fs::write(&target, b"%PDF-body").expect("existing");
        let partial = root.path().join("dup.part");
        std::fs::write(&partial, b"%PDF-body").expect("write");

        let stored = store_blob(root.path(), &partial, &hash)
            .await
            .expect("store");

        assert_eq!(stored, target);
        assert!(!partial.exists(), "temp removed");
        assert_eq!(std::fs::read(&target).expect("blob"), b"%PDF-body");
    }

    #[tokio::test]
    async fn should_copy_not_link_when_placing_library_copy() {
        let root = tempfile::tempdir().expect("root");
        let library = root.path().join("library");
        let blob = root.path().join("blob.pdf");
        std::fs::write(&blob, b"%PDF-one").expect("blob");

        let relative = place_library_copy(&library, &blob, "2016 - Paper.pdf", &sha(b"%PDF-one"))
            .await
            .expect("place");

        assert_eq!(relative, "Fetched/2016 - Paper.pdf");
        let placed = library.join("Fetched").join("2016 - Paper.pdf");
        assert_eq!(std::fs::read(&placed).expect("copy"), b"%PDF-one");
        std::fs::write(&placed, b"edited in place").expect("edit");
        assert_eq!(
            std::fs::read(&blob).expect("blob"),
            b"%PDF-one",
            "not linked"
        );
        assert!(!library.join("Fetched/2016 - Paper.pdf.part").exists());
    }

    #[tokio::test]
    async fn should_add_suffix_when_library_name_taken_by_different_bytes() {
        let root = tempfile::tempdir().expect("root");
        let library = root.path().join("library");
        std::fs::create_dir_all(library.join("Fetched")).expect("dirs");
        std::fs::write(library.join("Fetched/Paper.pdf"), b"%PDF-other").expect("taken");
        std::fs::write(library.join("Fetched/Paper (2).pdf"), b"%PDF-other2").expect("taken");
        let blob = root.path().join("blob.pdf");
        std::fs::write(&blob, b"%PDF-mine").expect("blob");

        let relative = place_library_copy(&library, &blob, "Paper.pdf", &sha(b"%PDF-mine"))
            .await
            .expect("place");

        assert_eq!(relative, "Fetched/Paper (3).pdf");
        assert_eq!(
            std::fs::read(library.join("Fetched/Paper.pdf")).expect("untouched"),
            b"%PDF-other"
        );
    }

    #[tokio::test]
    async fn should_reuse_library_copy_when_same_bytes_exist() {
        let root = tempfile::tempdir().expect("root");
        let library = root.path().join("library");
        std::fs::create_dir_all(library.join("Fetched")).expect("dirs");
        std::fs::write(library.join("Fetched/Paper.pdf"), b"%PDF-mine").expect("existing");
        let blob = root.path().join("blob.pdf");
        std::fs::write(&blob, b"%PDF-mine").expect("blob");

        let relative = place_library_copy(&library, &blob, "Paper.pdf", &sha(b"%PDF-mine"))
            .await
            .expect("place");

        assert_eq!(relative, "Fetched/Paper.pdf");
        assert!(!library.join("Fetched/Paper (2).pdf").exists());
    }

    #[tokio::test]
    async fn should_fail_and_leave_no_partial_when_blob_is_missing() {
        let root = tempfile::tempdir().expect("root");
        let library = root.path().join("library");
        let missing = root.path().join("missing.pdf");

        let result = place_library_copy(&library, &missing, "Paper.pdf", "abc").await;

        assert!(result.is_err());
        assert!(!library.join("Fetched/Paper.pdf").exists());
        assert!(!library.join("Fetched/Paper.pdf.part").exists());
    }
}
