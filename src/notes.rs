//! Markdown files owned by the reader, separate from generated paper artifacts.

use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, linkat, mkdirat, open, openat, renameat, statat, unlinkat,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_NOTES_BYTES: usize = 2 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct NotesStore {
    root: Arc<PathBuf>,
    write: Arc<Mutex<()>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteDocument {
    pub filename: String,
    pub text: String,
    /// Content revision; `None` means the file does not exist yet.
    pub revision: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SaveNoteRequest {
    pub text: String,
    pub revision: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum NotesError {
    #[error("The notes file changed outside this editor. Reload it before saving again.")]
    Conflict,
    #[error("Notes must be at most 2 MiB.")]
    TooLarge,
    #[error("{0}")]
    InvalidPath(String),
    #[error("Could not access the notes file: {0}")]
    Storage(#[from] std::io::Error),
    #[error("Could not complete the notes operation: {0}")]
    Task(String),
}

impl IntoResponse for NotesError {
    fn into_response(self) -> Response {
        let (status, kind) = match self {
            Self::Conflict => (StatusCode::CONFLICT, "notes_conflict"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "notes_too_large"),
            Self::InvalidPath(_) => (StatusCode::BAD_REQUEST, "invalid_notes_path"),
            Self::Storage(_) | Self::Task(_) => (StatusCode::INTERNAL_SERVER_ERROR, "notes_error"),
        };
        (
            status,
            Json(serde_json::json!({"error":kind,"message":self.to_string()})),
        )
            .into_response()
    }
}

impl From<rustix::io::Errno> for NotesError {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Storage(error.into())
    }
}

impl NotesStore {
    /// Configuration is lazy: this never reads or creates the notes directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
            write: Arc::new(Mutex::new(())),
        }
    }

    pub async fn read(&self, relative_pdf: &str) -> Result<NoteDocument, NotesError> {
        let path = note_path(relative_pdf)?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.read_sync(&path))
            .await
            .map_err(|error| NotesError::Task(error.to_string()))?
    }

    pub async fn save(
        &self,
        relative_pdf: &str,
        text: String,
        revision: Option<String>,
    ) -> Result<NoteDocument, NotesError> {
        if text.len() > MAX_NOTES_BYTES {
            return Err(NotesError::TooLarge);
        }
        let path = note_path(relative_pdf)?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.save_sync(&path, &text, revision.as_deref()))
            .await
            .map_err(|error| NotesError::Task(error.to_string()))?
    }

    fn read_sync(&self, path: &Path) -> Result<NoteDocument, NotesError> {
        let filename = path.to_string_lossy().into_owned();
        let Some((parent, leaf)) = self.parent(path, false)? else {
            return Ok(NoteDocument {
                filename,
                text: String::new(),
                revision: None,
            });
        };
        let (text, revision) = read_file(&parent, &leaf)?;
        Ok(NoteDocument {
            filename,
            text,
            revision,
        })
    }

    fn save_sync(
        &self,
        path: &Path,
        text: &str,
        expected: Option<&str>,
    ) -> Result<NoteDocument, NotesError> {
        let _guard = self
            .write
            .lock()
            .map_err(|error| NotesError::Task(error.to_string()))?;
        // Check before creating directories; a stale editor must not mutate storage.
        if self.read_sync(path)?.revision.as_deref() != expected {
            return Err(NotesError::Conflict);
        }
        let (parent, leaf) = self
            .parent(path, true)?
            .ok_or_else(|| NotesError::InvalidPath("The notes directory is unavailable.".into()))?;
        if read_file(&parent, &leaf)?.1.as_deref() != expected {
            return Err(NotesError::Conflict);
        }
        let temporary = format!(
            ".lysilogy-note-{}-{}.tmp",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut file = File::from(openat(
            &parent,
            &temporary,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        let result = (|| {
            if let Ok(existing) = openat(
                &parent,
                &leaf,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                let metadata = File::from(existing).metadata()?;
                if !metadata.is_file() {
                    return Err(unsafe_link());
                }
                file.set_permissions(metadata.permissions())?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            // An external editor may have changed the file while the replacement was written.
            if read_file(&parent, &leaf)?.1.as_deref() != expected {
                return Err(NotesError::Conflict);
            }
            if expected.is_none() {
                // Unlike rename, hard_link cannot overwrite a file created since the check.
                linkat(&parent, &temporary, &parent, &leaf, AtFlags::empty()).map_err(|error| {
                    if error == rustix::io::Errno::EXIST {
                        NotesError::Conflict
                    } else {
                        error.into()
                    }
                })?;
            } else {
                renameat(&parent, &temporary, &parent, &leaf)?;
            }
            Ok(NoteDocument {
                filename: path.to_string_lossy().into_owned(),
                text: text.to_owned(),
                revision: Some(revision(text.as_bytes())),
            })
        })();
        drop(file);
        // A successful rename already removed it; cleanup errors never mask the save result.
        let _ = unlinkat(&parent, &temporary, AtFlags::empty());
        result
    }

    /// Every note operation stays relative to an open directory capability. A
    /// replaced symlink cannot redirect reads/writes outside that directory.
    fn parent(&self, path: &Path, create: bool) -> Result<Option<(File, PathBuf)>, NotesError> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = match open(self.root.as_ref(), flags, Mode::empty()) {
            Ok(directory) => File::from(directory),
            Err(rustix::io::Errno::NOENT) => {
                if !create {
                    return Ok(None);
                }
                std::fs::create_dir_all(self.root.as_ref())?;
                File::from(open(self.root.as_ref(), flags, Mode::empty())?)
            }
            Err(error) => return Err(error.into()),
        };
        let leaf = path
            .file_name()
            .ok_or_else(|| NotesError::InvalidPath("The paper has no filename.".into()))?;
        if let Some(parent) = path.parent() {
            for component in parent.components() {
                let name = component.as_os_str();
                match statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(metadata)
                        if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory =>
                    {
                        return Err(unsafe_link());
                    }
                    Ok(_) => (),
                    Err(rustix::io::Errno::NOENT) => {
                        if !create {
                            return Ok(None);
                        }
                        mkdirat(&directory, name, Mode::RUSR | Mode::WUSR | Mode::XUSR)?;
                    }
                    Err(error) => return Err(error.into()),
                }
                directory = File::from(openat(&directory, name, flags, Mode::empty())?);
            }
        }
        Ok(Some((directory, PathBuf::from(leaf))))
    }
}

fn note_path(relative_pdf: &str) -> Result<PathBuf, NotesError> {
    let path = Path::new(relative_pdf);
    if relative_pdf.is_empty()
        || relative_pdf.contains('\\')
        || relative_pdf.contains('\0')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return Err(NotesError::InvalidPath(
            "Notes require a relative PDF filename without traversal.".into(),
        ));
    }
    Ok(path.with_extension("md"))
}

fn read_file(directory: &File, path: &Path) -> Result<(String, Option<String>), NotesError> {
    match statat(directory, path, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile => {
            return Err(unsafe_link());
        }
        Ok(metadata) if metadata.st_size > i64::try_from(MAX_NOTES_BYTES).unwrap_or(i64::MAX) => {
            return Err(NotesError::TooLarge);
        }
        Ok(_) => (),
        Err(rustix::io::Errno::NOENT) => return Ok((String::new(), None)),
        Err(error) => return Err(error.into()),
    }
    let mut bytes = Vec::new();
    let file = File::from(openat(
        directory,
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(unsafe_link());
    }
    if metadata.len() > MAX_NOTES_BYTES as u64 {
        return Err(NotesError::TooLarge);
    }
    file.take(MAX_NOTES_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_NOTES_BYTES {
        return Err(NotesError::TooLarge);
    }
    let hash = revision(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| {
        NotesError::InvalidPath("The notes file is not valid UTF-8 Markdown.".into())
    })?;
    Ok((text, Some(hash)))
}

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn unsafe_link() -> NotesError {
    NotesError::InvalidPath("Notes cannot follow symlinks or replace a non-file.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_notes_are_empty_and_storage_is_created_only_on_save() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("reader-notes");
        let store = NotesStore::new(&root);
        let note = store.read("group/Author - Paper.pdf").await.unwrap();
        assert_eq!(note.filename, "group/Author - Paper.md");
        assert_eq!(note.text, "");
        assert!(note.revision.is_none());
        assert!(!root.exists());
        let saved = store
            .save(
                "group/Author - Paper.pdf",
                "# Notes\n\nA useful result.\n".into(),
                None,
            )
            .await
            .unwrap();
        assert!(saved.revision.is_some());
        assert_eq!(
            store.read("group/Author - Paper.pdf").await.unwrap().text,
            saved.text
        );
        assert_eq!(
            std::fs::read_to_string(root.join("group/Author - Paper.md")).unwrap(),
            saved.text
        );
        assert_eq!(std::fs::read_dir(root.join("group")).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn changed_and_deleted_external_files_conflict_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let store = NotesStore::new(root.path());
        let first = store
            .save("Paper.pdf", "Original".into(), None)
            .await
            .unwrap();
        std::fs::write(root.path().join("Paper.md"), "External edit").unwrap();
        assert!(matches!(
            store
                .save("Paper.pdf", "Stale draft".into(), first.revision)
                .await,
            Err(NotesError::Conflict)
        ));
        let external = store.read("Paper.pdf").await.unwrap();
        assert_eq!(external.text, "External edit");
        std::fs::remove_file(root.path().join("Paper.md")).unwrap();
        assert!(matches!(
            store
                .save("Paper.pdf", "Stale draft".into(), external.revision)
                .await,
            Err(NotesError::Conflict)
        ));
        assert!(!root.path().join("Paper.md").exists());
    }

    #[tokio::test]
    async fn simultaneous_creates_cannot_overwrite_each_other() {
        let root = tempfile::tempdir().unwrap();
        let store = NotesStore::new(root.path());
        let (a, b) = tokio::join!(
            store.save("Paper.pdf", "A".into(), None),
            store.save("Paper.pdf", "B".into(), None)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let conflict = if a.is_err() { a } else { b };
        assert!(matches!(conflict, Err(NotesError::Conflict)));
    }

    #[tokio::test]
    async fn valid_revisions_allow_atomic_updates_including_empty_notes() {
        let root = tempfile::tempdir().unwrap();
        let store = NotesStore::new(root.path());
        let first = store
            .save("Paper.pdf", "Original".into(), None)
            .await
            .unwrap();
        let second = store
            .save("Paper.pdf", String::new(), first.revision)
            .await
            .unwrap();
        assert!(second.revision.is_some());
        assert_eq!(std::fs::read(root.path().join("Paper.md")).unwrap(), b"");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn oversized_and_non_text_notes_do_not_overwrite_storage() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("notes");
        let store = NotesStore::new(&root);
        assert!(matches!(
            store
                .save("Paper.pdf", "x".repeat(MAX_NOTES_BYTES + 1), None)
                .await,
            Err(NotesError::TooLarge)
        ));
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("Paper.md"), [0xff, 0xfe]).unwrap();
        assert!(matches!(
            store.read("Paper.pdf").await,
            Err(NotesError::InvalidPath(_))
        ));
        assert!(
            store
                .save("Paper.pdf", "Do not overwrite".into(), None)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(root.join("Paper.md")).unwrap(), [0xff, 0xfe]);
    }

    #[test]
    fn path_validation_keeps_notes_below_the_root() {
        for path in [
            "../Paper.pdf",
            "/Paper.pdf",
            "nested/../../Paper.pdf",
            "nested\\Paper.pdf",
            "",
            "Paper.txt",
        ] {
            assert!(note_path(path).is_err(), "{path}");
        }
        assert_eq!(
            note_path("日本語 - Author.PDF").unwrap(),
            Path::new("日本語 - Author.md")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_files_and_directories_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("Paper.md"), "Untouched").unwrap();
        symlink(
            outside.path().join("Paper.md"),
            root.path().join("Paper.md"),
        )
        .unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();
        let store = NotesStore::new(root.path());
        for path in ["Paper.pdf", "linked/Paper.pdf"] {
            assert!(store.read(path).await.is_err());
            assert!(store.save(path, "Overwrite".into(), None).await.is_err());
        }
        assert_eq!(
            std::fs::read_to_string(outside.path().join("Paper.md")).unwrap(),
            "Untouched"
        );
    }
}
