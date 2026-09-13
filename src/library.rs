use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use walkdir::{DirEntry, WalkDir};

use crate::{
    Result,
    domain::{
        DuplicatePapers, IdentityConflict, PaperId, PaperMetadata, PaperOverview, ProcessingStage,
        ProcessingStatus,
    },
    error::Error,
    store::ArtifactStore,
};

#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub overview: PaperOverview,
    pub source_path: PathBuf,
    /// Canonical notes location; deliberately does not follow later PDF moves.
    pub notes_relative_path: String,
    source_stamp: FileStamp,
}

impl CatalogEntry {
    /// Reject changed content before using this identity's source or extraction.
    pub(crate) async fn verify_source(&self) -> Result<()> {
        let stamp = FileStamp::read(&self.source_path).await?;
        if stamp == self.source_stamp {
            return Ok(());
        }
        let (hash, _) = hash_pdf(&self.source_path, &stamp).await?;
        if self.overview.content_hash.as_deref() == Some(&hash) {
            Ok(())
        } else {
            Err(Error::InvalidRequest(
                "PDF content changed; rescan the library before using this paper.".into(),
            ))
        }
    }
}

#[derive(Debug)]
pub struct LibraryCatalog {
    root: PathBuf,
    entries: BTreeMap<PaperId, CatalogEntry>,
    duplicates: Vec<DuplicatePapers>,
    identity_conflicts: Vec<IdentityConflict>,
    hashes_computed: usize,
}

impl LibraryCatalog {
    pub async fn scan(root: impl Into<PathBuf>, store: &ArtifactStore) -> Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            return Err(Error::InvalidRequest(format!(
                "library path is not a directory: {}",
                root.display()
            )));
        }

        let root = tokio::fs::canonicalize(&root)
            .await
            .map_err(|error| Error::io(&root, error))?;
        let identity_guard = store.lock_identities().await?;
        let saved = store.load_identities().await?;
        let bootstrap = saved.is_none();
        let mut registry = saved.unwrap_or_else(|| IdentityRegistry {
            schema_version: 1,
            library_root: root.clone(),
            records: BTreeMap::new(),
        });
        registry.validate(&root)?;
        let mut paths = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(visible_entry)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| {
                Error::InvalidRequest(format!("Library scan was incomplete: {error}"))
            })?
            .into_iter()
            .filter(|entry| entry.file_type().is_file() && is_pdf(entry.path()))
            .map(DirEntry::into_path)
            .collect::<Vec<_>>();
        paths.sort();

        let mut snapshots = Vec::new();
        let mut hashes_computed = 0;
        let active: BTreeMap<_, _> = registry
            .records
            .iter()
            .filter(|(_, record)| record.active)
            .map(|(id, record)| (record.relative_path.as_str(), (id, record)))
            .collect();
        for path in paths {
            let relative = path.strip_prefix(&root).map_err(|error| {
                Error::InvalidRequest(format!("could not relativize {}: {error}", path.display()))
            })?;
            let relative = relative
                .to_str()
                .ok_or_else(|| Error::InvalidRequest("PDF paths must be valid UTF-8.".into()))?
                .to_owned();
            if !safe_relative_path(&relative) {
                return Err(Error::InvalidRequest(
                    "PDF paths must be relative PDF filenames without traversal or backslashes."
                        .into(),
                ));
            }
            let stamp = FileStamp::read(&path).await?;
            let content_hash = if let Some((_, record)) = active.get(relative.as_str())
                && record.stamp == stamp
            {
                record.content_hash.clone()
            } else {
                hashes_computed += 1;
                hash_pdf(&path, &stamp).await?.0
            };
            snapshots.push(PdfSnapshot {
                path,
                relative,
                stamp,
                content_hash,
            });
        }
        let (assignments, duplicates, identity_conflicts) =
            registry.reconcile(&snapshots, store, bootstrap).await?;
        let mut entries = BTreeMap::new();
        for (snapshot, id) in snapshots.iter().zip(assignments) {
            entries.insert(
                id.clone(),
                CatalogEntry {
                    overview: overview(snapshot, &id, store).await?,
                    source_path: snapshot.path.clone(),
                    notes_relative_path: registry.records[&id].notes_relative_path.clone(),
                    source_stamp: snapshot.stamp.clone(),
                },
            );
        }
        // A failed/inconsistent scan never publishes partial moves or tombstones.
        for snapshot in &snapshots {
            if FileStamp::read(&snapshot.path).await? != snapshot.stamp {
                return Err(changed_during_scan());
            }
        }
        store.save_identities(&registry).await?;
        drop(identity_guard);
        Ok(Self {
            root,
            entries,
            duplicates,
            identity_conflicts,
            hashes_computed,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn overviews(&self) -> Vec<PaperOverview> {
        let mut papers = self
            .entries
            .values()
            .map(|entry| entry.overview.clone())
            .collect::<Vec<_>>();
        papers.sort_by(|left, right| {
            left.metadata
                .title
                .to_lowercase()
                .cmp(&right.metadata.title.to_lowercase())
        });
        papers
    }

    #[must_use]
    pub fn get(&self, id: &PaperId) -> Option<&CatalogEntry> {
        self.entries.get(id)
    }

    pub fn get_mut(&mut self, id: &PaperId) -> Option<&mut CatalogEntry> {
        self.entries.get_mut(id)
    }

    #[must_use]
    pub fn duplicates(&self) -> &[DuplicatePapers] {
        &self.duplicates
    }

    #[must_use]
    pub fn identity_conflicts(&self) -> &[IdentityConflict] {
        &self.identity_conflicts
    }

    #[must_use]
    pub const fn hashes_computed(&self) -> usize {
        self.hashes_computed
    }

    pub fn replace_with(&mut self, mut replacement: Self) {
        for (id, entry) in &mut replacement.entries {
            if let Some(existing) = self.entries.get(id)
                && matches!(
                    existing.overview.status,
                    ProcessingStatus::Queued { .. }
                        | ProcessingStatus::Extracting
                        | ProcessingStatus::Analyzing { .. }
                )
            {
                entry.overview.status = existing.overview.status.clone();
            }
        }
        *self = replacement;
    }

    pub fn mark_failure(&mut self, id: &PaperId, stage: ProcessingStage, error: &Error) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.overview.status = ProcessingStatus::Failed {
                stage,
                message: error.to_string(),
                retryable: !matches!(error, Error::EmptyExtraction(_)),
            };
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct FileStamp {
    bytes: u64,
    modified_nanos: u128,
    device: u64,
    inode: u64,
    changed_seconds: i64,
    changed_nanos: i64,
}

impl FileStamp {
    async fn read(path: &Path) -> Result<Self> {
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|error| Error::io(path, error))?;
        Self::from_metadata(path, &metadata)
    }

    fn from_metadata(path: &Path, metadata: &std::fs::Metadata) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file() {
            return Err(Error::InvalidRequest(
                "PDF source must be a regular file, not a symbolic link.".into(),
            ));
        }
        let modified_nanos = metadata
            .modified()
            .map_err(|error| Error::io(path, error))?
            .duration_since(UNIX_EPOCH)
            .map_err(|error| Error::InvalidRequest(error.to_string()))?
            .as_nanos();
        Ok(Self {
            bytes: metadata.len(),
            modified_nanos,
            device: metadata.dev(),
            inode: metadata.ino(),
            changed_seconds: metadata.ctime(),
            changed_nanos: metadata.ctime_nsec(),
        })
    }
}

struct PdfSnapshot {
    path: PathBuf,
    relative: String,
    stamp: FileStamp,
    content_hash: String,
}

async fn hash_pdf(path: &Path, expected: &FileStamp) -> Result<(String, FileStamp)> {
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| Error::io(path, error.into()))?;
    let mut file = tokio::fs::File::from_std(descriptor.into());
    let before = FileStamp::from_metadata(
        path,
        &file
            .metadata()
            .await
            .map_err(|error| Error::io(path, error))?,
    )?;
    if before != *expected {
        return Err(changed_during_scan());
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|error| Error::io(path, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let after = FileStamp::from_metadata(
        path,
        &file
            .metadata()
            .await
            .map_err(|error| Error::io(path, error))?,
    )?;
    if before != after || FileStamp::read(path).await? != before {
        return Err(changed_during_scan());
    }
    Ok((format!("{:x}", hasher.finalize()), before))
}

fn changed_during_scan() -> Error {
    Error::InvalidRequest(
        "PDF changed during identity verification; retry the library scan.".into(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct IdentityRegistry {
    schema_version: u16,
    library_root: PathBuf,
    records: BTreeMap<PaperId, PaperIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PaperIdentity {
    relative_path: String,
    notes_relative_path: String,
    content_hash: String,
    stamp: FileStamp,
    active: bool,
}

impl IdentityRegistry {
    fn validate(&self, root: &Path) -> Result<()> {
        if self.schema_version != 1 || self.library_root != root {
            return Err(Error::InvalidRequest("The paper identity registry has an unsupported schema or belongs to a different library root; use that library's data root.".into()));
        }
        let mut notes = BTreeSet::new();
        let mut active = BTreeSet::new();
        for (id, record) in &self.records {
            if !id
                .as_str()
                .parse::<PaperId>()
                .is_ok_and(|parsed| parsed == *id)
                || !safe_relative_path(&record.relative_path)
                || !safe_relative_path(&record.notes_relative_path)
                || record.content_hash.len() != 64
                || !record
                    .content_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || !notes.insert(Path::new(&record.notes_relative_path).with_extension("md"))
                || (record.active && !active.insert(&record.relative_path))
            {
                return Err(Error::InvalidRequest(
                    "Paper identity registry is invalid; its canonical records were not changed."
                        .into(),
                ));
            }
        }
        Ok(())
    }

    async fn reconcile(
        &mut self,
        snapshots: &[PdfSnapshot],
        store: &ArtifactStore,
        bootstrap: bool,
    ) -> Result<(Vec<PaperId>, Vec<DuplicatePapers>, Vec<IdentityConflict>)> {
        let mut by_hash: BTreeMap<&str, Vec<&PdfSnapshot>> = BTreeMap::new();
        for snapshot in snapshots {
            by_hash
                .entry(&snapshot.content_hash)
                .or_default()
                .push(snapshot);
        }
        let duplicates = duplicate_groups(&by_hash);
        let mut assigned = BTreeMap::new();
        let mut used = BTreeSet::new();
        let active_by_path: BTreeMap<_, _> = self
            .records
            .iter()
            .filter(|(_, record)| record.active)
            .map(|(id, record)| (record.relative_path.as_str(), (id, record)))
            .collect();
        let mut records_by_hash: BTreeMap<&str, Vec<(&PaperId, &PaperIdentity)>> = BTreeMap::new();
        for (id, record) in &self.records {
            records_by_hash
                .entry(&record.content_hash)
                .or_default()
                .push((id, record));
        }
        for snapshot in snapshots {
            if let Some((id, record)) = active_by_path.get(snapshot.relative.as_str())
                && record.content_hash == snapshot.content_hash
            {
                assigned.insert(snapshot.relative.clone(), (*id).clone());
                used.insert((*id).clone());
            }
        }
        let mut conflicts = Vec::new();
        for (hash, files) in by_hash {
            let unmatched: Vec<_> = files
                .iter()
                .filter(|file| !assigned.contains_key(&file.relative))
                .collect();
            if unmatched.is_empty() {
                continue;
            }
            let candidates: Vec<_> = records_by_hash
                .get(hash)
                .into_iter()
                .flatten()
                .filter(|(id, _)| !used.contains(*id))
                .collect();
            if files.len() == 1 && candidates.len() == 1 {
                let id = candidates[0].0.clone();
                assigned.insert(unmatched[0].relative.clone(), id.clone());
                used.insert(id);
            } else if !candidates.is_empty() {
                conflicts.push(IdentityConflict {
                    content_hash: hash.to_owned(),
                    current_paths: unmatched.iter().map(|file| file.relative.clone()).collect(),
                    previous_paths: candidates
                        .iter()
                        .map(|(_, record)| record.relative_path.clone())
                        .collect(),
                });
            }
        }
        for record in self.records.values_mut() {
            record.active = false;
        }
        let mut result = Vec::new();
        let mut notes_paths: BTreeSet<_> = self
            .records
            .values()
            .map(|record| Path::new(&record.notes_relative_path).with_extension("md"))
            .collect();
        for snapshot in snapshots {
            let id = if let Some(id) = assigned.remove(&snapshot.relative) {
                id
            } else {
                self.allocate_id(snapshot, store, bootstrap).await?
            };
            let notes_relative_path = self.records.get(&id).map_or_else(
                || allocate_notes_path(&snapshot.relative, &id, &mut notes_paths),
                |record| record.notes_relative_path.clone(),
            );
            self.records.insert(
                id.clone(),
                PaperIdentity {
                    relative_path: snapshot.relative.clone(),
                    notes_relative_path,
                    content_hash: snapshot.content_hash.clone(),
                    stamp: snapshot.stamp.clone(),
                    active: true,
                },
            );
            result.push(id);
        }
        Ok((result, duplicates, conflicts))
    }

    async fn allocate_id(
        &self,
        snapshot: &PdfSnapshot,
        store: &ArtifactStore,
        bootstrap: bool,
    ) -> Result<PaperId> {
        let mut sequence = 0_u64;
        let mut id = PaperId::from_relative_path(Path::new(&snapshot.relative));
        loop {
            if !self.records.contains_key(&id)
                && (bootstrap
                    || !tokio::fs::try_exists(store.paper_dir(&id))
                        .await
                        .map_err(|error| Error::io(store.paper_dir(&id), error))?)
            {
                return Ok(id);
            }
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| Error::Task("Paper identity namespace exhausted.".into()))?;
            id = PaperId::for_revision(&snapshot.relative, &snapshot.content_hash, sequence);
        }
    }
}

fn allocate_notes_path(relative: &str, id: &PaperId, used: &mut BTreeSet<PathBuf>) -> String {
    if used.insert(Path::new(relative).with_extension("md")) {
        return relative.to_owned();
    }
    let mut path = PathBuf::from(relative);
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut sequence = 0_u64;
    loop {
        path.set_file_name(format!("{stem} [paper-{id}-{sequence}].pdf"));
        if used.insert(path.with_extension("md")) {
            return path.to_string_lossy().into_owned();
        }
        sequence += 1;
    }
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', '\0'])
        && is_pdf(Path::new(value))
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn duplicate_groups(by_hash: &BTreeMap<&str, Vec<&PdfSnapshot>>) -> Vec<DuplicatePapers> {
    by_hash
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(hash, files)| DuplicatePapers {
            content_hash: (*hash).to_owned(),
            paths: files.iter().map(|file| file.relative.clone()).collect(),
        })
        .collect()
}

async fn overview(
    snapshot: &PdfSnapshot,
    id: &PaperId,
    store: &ArtifactStore,
) -> Result<PaperOverview> {
    let analysis = store.load_analysis(id).await?;
    let extraction = store.load_extraction(id).await?;
    let mut metadata = metadata_from_filename(&snapshot.path);
    if let Some(extracted) = &extraction {
        merge_metadata(&mut metadata, &extracted.metadata);
    }
    let status = if analysis.is_some() {
        ProcessingStatus::Ready
    } else if extraction.is_some() {
        ProcessingStatus::Extracted
    } else {
        ProcessingStatus::Discovered
    };
    Ok(PaperOverview {
        id: id.clone(),
        metadata,
        relative_path: snapshot.relative.clone(),
        content_hash: Some(snapshot.content_hash.clone()),
        status,
        analyzed_at: analysis.as_ref().map(|value| value.generated_at),
        one_line_summary: analysis.as_ref().map(|value| value.thesis.clone()),
    })
}

fn visible_entry(entry: &DirEntry) -> bool {
    entry.depth() == 0
        || entry
            .file_name()
            .to_str()
            .is_none_or(|name| !name.starts_with('.'))
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn metadata_from_filename(path: &Path) -> PaperMetadata {
    let mut stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled paper")
        .trim()
        .to_owned();
    while stem.to_ascii_lowercase().ends_with(".pdf") {
        stem.truncate(stem.len().saturating_sub(4));
    }
    let pieces = stem
        .split(" - ")
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>();

    let (authors, year, title) = match pieces.as_slice() {
        [author, year, title @ ..] if parse_year(year).is_some() => (
            vec![(*author).to_owned()],
            parse_year(year),
            title.join(" — "),
        ),
        [date, title @ ..] if year_from_date(date).is_some() => {
            (Vec::new(), year_from_date(date), title.join(" — "))
        }
        [author, title @ ..] if !title.is_empty() => {
            (vec![(*author).to_owned()], None, title.join(" — "))
        }
        _ => (Vec::new(), None, stem),
    };

    PaperMetadata {
        title,
        authors,
        year,
        page_count: None,
        subject: None,
    }
}

fn parse_year(value: &str) -> Option<u16> {
    (value.len() == 4)
        .then(|| value.parse::<u16>().ok())
        .flatten()
        .filter(|year| (1400..=2200).contains(year))
}

fn year_from_date(value: &str) -> Option<u16> {
    value.get(..4).and_then(parse_year)
}

fn merge_metadata(target: &mut PaperMetadata, extracted: &PaperMetadata) {
    if !extracted.title.trim().is_empty() {
        target.title.clone_from(&extracted.title);
    }
    if !extracted.authors.is_empty() {
        target.authors.clone_from(&extracted.authors);
    }
    target.year = extracted.year.or(target.year);
    target.page_count = extracted.page_count.or(target.page_count);
    target.subject.clone_from(&extracted.subject);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    async fn fixture(
        root: &Path,
        store: &ArtifactStore,
        relative: &str,
    ) -> (PaperId, Vec<(String, Vec<u8>)>) {
        let path = root.join(relative);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"original PDF fixture bytes")
            .await
            .unwrap();
        store.initialize().await.unwrap();
        let catalog = LibraryCatalog::scan(root, store).await.unwrap();
        let id = catalog.overviews()[0].id.clone();
        let analysis = serde_json::from_value(json!({
            "schema_version":5,"provider":"heuristic","generated_at":"2026-09-12T12:00:00Z",
            "thesis":"Preserved analysis.","outsider_brief":"A source-local orientation."
        }))
        .unwrap();
        store
            .save_analysis_projection(&id, &analysis)
            .await
            .unwrap();
        store
            .save_extraction(
                &id,
                &crate::domain::ExtractedPaper {
                    metadata: PaperMetadata {
                        title: "Mapped title".into(),
                        ..PaperMetadata::default()
                    },
                    pages: vec![],
                    layout: crate::domain::DocumentLayout::default(),
                },
            )
            .await
            .unwrap();
        let directory = store.paper_dir(&id);
        for filename in ["highlights.jsonl", "reader-tools.json", "objects.json"] {
            tokio::fs::write(
                directory.join(filename),
                format!("preserved fixture for {filename}"),
            )
            .await
            .unwrap();
        }
        let mut artifacts = Vec::new();
        for filename in [
            "analysis.json",
            "highlights.jsonl",
            "reader-tools.json",
            "objects.json",
            "source.txt",
            "source.md",
        ] {
            artifacts.push((
                filename.into(),
                tokio::fs::read(directory.join(filename)).await.unwrap(),
            ));
        }
        (id, artifacts)
    }

    async fn assert_preserved(
        store: &ArtifactStore,
        id: &PaperId,
        artifacts: &[(String, Vec<u8>)],
    ) {
        for (filename, bytes) in artifacts {
            assert_eq!(
                &tokio::fs::read(store.paper_dir(id).join(filename))
                    .await
                    .unwrap(),
                bytes,
                "{filename}"
            );
        }
    }

    #[tokio::test]
    async fn should_reattach_artifacts_when_mapped_pdf_is_renamed() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let (id, artifacts) = fixture(root.path(), &store, "old.pdf").await;
        tokio::fs::rename(root.path().join("old.pdf"), root.path().join("new.pdf"))
            .await
            .unwrap();
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let entry = catalog.get(&id).unwrap();
        assert_eq!(entry.overview.relative_path, "new.pdf");
        assert_eq!(entry.notes_relative_path, "old.pdf");
        assert!(matches!(entry.overview.status, ProcessingStatus::Ready));
        assert_eq!(entry.overview.metadata.title, "Mapped title");
        assert_preserved(&store, &id, &artifacts).await;
        assert!(
            !store
                .paper_dir(&PaperId::from_relative_path(Path::new("new.pdf")))
                .exists()
        );
        let reopened = LibraryCatalog::scan(root.path(), &ArtifactStore::new(data.path()))
            .await
            .unwrap();
        assert_eq!(reopened.overviews()[0].id, id);
    }

    #[tokio::test]
    async fn should_reattach_artifacts_when_pdf_moves_between_subdirectories() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let (id, artifacts) = fixture(root.path(), &store, "old/paper.pdf").await;
        tokio::fs::create_dir_all(root.path().join("new/deeper"))
            .await
            .unwrap();
        tokio::fs::rename(
            root.path().join("old/paper.pdf"),
            root.path().join("new/deeper/paper.pdf"),
        )
        .await
        .unwrap();
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        assert_eq!(
            catalog.get(&id).unwrap().overview.relative_path,
            "new/deeper/paper.pdf"
        );
        assert_eq!(
            catalog.get(&id).unwrap().notes_relative_path,
            "old/paper.pdf"
        );
        assert_preserved(&store, &id, &artifacts).await;
    }

    #[tokio::test]
    async fn should_report_both_paths_when_identical_pdfs_exist_twice() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        tokio::fs::write(root.path().join("a.pdf"), b"same bytes")
            .await
            .unwrap();
        tokio::fs::write(root.path().join("b.pdf"), b"same bytes")
            .await
            .unwrap();
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let papers = catalog.overviews();
        assert_eq!(papers.len(), 2);
        assert_ne!(papers[0].id, papers[1].id);
        assert_eq!(papers[0].content_hash, papers[1].content_hash);
        assert_eq!(catalog.duplicates()[0].paths, ["a.pdf", "b.pdf"]);
    }

    #[tokio::test]
    async fn should_not_rehash_when_size_and_mtime_are_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        tokio::fs::write(root.path().join("paper.pdf"), b"hash me once")
            .await
            .unwrap();
        let first = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let second = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        assert_eq!(first.hashes_computed(), 1);
        assert_eq!(second.hashes_computed(), 0);
        assert_eq!(
            first.overviews()[0].content_hash,
            Some(format!("{:x}", Sha256::digest(b"hash me once")))
        );
        assert_eq!(
            first.overviews()[0].content_hash,
            second.overviews()[0].content_hash
        );
    }

    #[tokio::test]
    async fn should_isolate_artifacts_and_notes_when_content_at_a_path_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let (old, artifacts) = fixture(root.path(), &store, "paper.pdf").await;
        tokio::fs::write(root.path().join("paper.pdf"), b"different PDF content")
            .await
            .unwrap();
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let paper = &catalog.overviews()[0];
        assert_ne!(paper.id, old);
        assert!(matches!(paper.status, ProcessingStatus::Discovered));
        assert_ne!(
            catalog.get(&paper.id).unwrap().notes_relative_path,
            "paper.pdf"
        );
        assert!(!store.paper_dir(&paper.id).exists());
        assert_preserved(&store, &old, &artifacts).await;
        assert_eq!(
            store
                .load_identities()
                .await
                .unwrap()
                .unwrap()
                .records
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn should_rehash_when_atomic_replacement_preserves_size_and_mtime() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let path = root.path().join("paper.pdf");
        tokio::fs::write(&path, b"first").await.unwrap();
        let first = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let replacement = root.path().join("replacement.tmp");
        std::fs::write(&replacement, b"other").unwrap();
        std::fs::File::open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(mtime))
            .unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let second = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        assert_eq!(second.hashes_computed(), 1);
        assert_ne!(first.overviews()[0].id, second.overviews()[0].id);
    }

    #[tokio::test]
    async fn should_reject_stale_source_when_content_changes_before_extraction() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let path = root.path().join("paper.pdf");
        tokio::fs::write(&path, b"first").await.unwrap();
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let entry = catalog.get(&catalog.overviews()[0].id).unwrap();
        tokio::fs::write(&path, b"other").await.unwrap();
        assert!(entry.verify_source().await.is_err());
        assert!(hash_pdf(&path, &entry.source_stamp).await.is_err());
    }

    #[tokio::test]
    async fn should_keep_old_ids_unassigned_when_duplicate_moves_are_ambiguous() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        for name in ["a.pdf", "b.pdf"] {
            tokio::fs::write(root.path().join(name), b"same bytes")
                .await
                .unwrap();
        }
        let first = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        tokio::fs::rename(root.path().join("a.pdf"), root.path().join("c.pdf"))
            .await
            .unwrap();
        tokio::fs::rename(root.path().join("b.pdf"), root.path().join("d.pdf"))
            .await
            .unwrap();
        let second = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        for paper in first.overviews() {
            assert!(second.get(&paper.id).is_none());
        }
        assert_eq!(second.identity_conflicts().len(), 1);
        assert_eq!(
            second.identity_conflicts()[0].current_paths,
            ["c.pdf", "d.pdf"]
        );
        assert_eq!(second.duplicates()[0].paths, ["c.pdf", "d.pdf"]);
    }

    #[tokio::test]
    async fn should_retain_tombstones_when_pdf_is_absent_before_it_reappears() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let (id, artifacts) = fixture(root.path(), &store, "old.pdf").await;
        tokio::fs::rename(root.path().join("old.pdf"), root.path().join("holding.tmp"))
            .await
            .unwrap();
        assert!(
            LibraryCatalog::scan(root.path(), &store)
                .await
                .unwrap()
                .overviews()
                .is_empty()
        );
        tokio::fs::write(
            data.path().join("paper-identities.tmp-interrupted"),
            b"partial transaction",
        )
        .await
        .unwrap();
        tokio::fs::rename(
            root.path().join("holding.tmp"),
            root.path().join("returned.pdf"),
        )
        .await
        .unwrap();
        let recovered = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        assert_eq!(recovered.overviews()[0].id, id);
        assert_preserved(&store, &id, &artifacts).await;
    }

    #[tokio::test]
    async fn should_reject_registry_when_root_or_canonical_records_are_invalid() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let path = data.path().join("paper-identities.json");
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert!(LibraryCatalog::scan(other.path(), &store).await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
        tokio::fs::write(&path, b"{broken canonical state")
            .await
            .unwrap();
        assert!(LibraryCatalog::scan(root.path(), &store).await.is_err());
        assert_eq!(
            tokio::fs::read(&path).await.unwrap(),
            b"{broken canonical state"
        );
    }

    #[tokio::test]
    async fn should_serialize_identity_updates_when_separate_store_handles_scan() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        let guard = store.lock_identities().await.unwrap();
        let another = ArtifactStore::new(data.path());
        let source = root.path().to_owned();
        let mut pending = tokio::spawn(async move { LibraryCatalog::scan(source, &another).await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut pending)
                .await
                .is_err()
        );
        drop(guard);
        assert!(pending.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn should_keep_active_job_identity_when_pdf_moves_during_processing() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        tokio::fs::write(root.path().join("old.pdf"), b"active job fixture")
            .await
            .unwrap();
        let mut first = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let id = first.overviews()[0].id.clone();
        first.get_mut(&id).unwrap().overview.status = ProcessingStatus::Analyzing {
            provider: crate::domain::AnalysisProvider::Heuristic,
        };
        tokio::fs::rename(root.path().join("old.pdf"), root.path().join("new.pdf"))
            .await
            .unwrap();
        let replacement = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        first.replace_with(replacement);
        assert!(matches!(
            first.get(&id).unwrap().overview.status,
            ProcessingStatus::Analyzing { .. }
        ));
        assert_eq!(first.get(&id).unwrap().overview.relative_path, "new.pdf");
    }

    #[tokio::test]
    async fn should_keep_distinct_notes_when_pdf_extensions_differ_only_in_case() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        for name in ["paper.pdf", "paper.PDF"] {
            tokio::fs::write(root.path().join(name), name)
                .await
                .unwrap();
        }
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let notes: BTreeSet<_> = catalog
            .entries
            .values()
            .map(|entry| Path::new(&entry.notes_relative_path).with_extension("md"))
            .collect();
        assert_eq!(notes.len(), 2);
        LibraryCatalog::scan(root.path(), &store).await.unwrap();
    }

    #[tokio::test]
    async fn should_preserve_orphan_artifacts_when_new_path_collides_with_their_id() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let reserved = PaperId::from_relative_path(Path::new("new.pdf"));
        let directory = store.paper_dir(&reserved);
        tokio::fs::create_dir_all(&directory).await.unwrap();
        tokio::fs::write(
            directory.join("analysis.json"),
            b"orphaned canonical artifact",
        )
        .await
        .unwrap();
        tokio::fs::write(root.path().join("new.pdf"), b"unrelated content")
            .await
            .unwrap();
        let catalog = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        assert_ne!(catalog.overviews()[0].id, reserved);
        assert_eq!(
            tokio::fs::read(directory.join("analysis.json"))
                .await
                .unwrap(),
            b"orphaned canonical artifact"
        );
    }

    #[tokio::test]
    async fn should_follow_content_when_two_existing_pdf_paths_are_swapped() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(data.path());
        tokio::fs::write(root.path().join("a.pdf"), b"first content")
            .await
            .unwrap();
        tokio::fs::write(root.path().join("b.pdf"), b"second content")
            .await
            .unwrap();
        let first = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        let a = PaperId::from_relative_path(Path::new("a.pdf"));
        let b = PaperId::from_relative_path(Path::new("b.pdf"));
        assert!(first.get(&a).is_some() && first.get(&b).is_some());
        tokio::fs::rename(root.path().join("a.pdf"), root.path().join("swap.tmp"))
            .await
            .unwrap();
        tokio::fs::rename(root.path().join("b.pdf"), root.path().join("a.pdf"))
            .await
            .unwrap();
        tokio::fs::rename(root.path().join("swap.tmp"), root.path().join("b.pdf"))
            .await
            .unwrap();
        let second = LibraryCatalog::scan(root.path(), &store).await.unwrap();
        assert_eq!(second.get(&a).unwrap().overview.relative_path, "b.pdf");
        assert_eq!(second.get(&b).unwrap().overview.relative_path, "a.pdf");
        assert_eq!(second.get(&a).unwrap().notes_relative_path, "a.pdf");
        assert_eq!(second.get(&b).unwrap().notes_relative_path, "b.pdf");
    }

    #[test]
    fn parses_conventional_vault_filename() {
        let metadata = metadata_from_filename(Path::new(
            "Autor, Dorn, and Hanson - 2013 - The China Syndrome.pdf",
        ));
        assert_eq!(metadata.year, Some(2013));
        assert_eq!(metadata.authors, ["Autor, Dorn, and Hanson"]);
        assert_eq!(metadata.title, "The China Syndrome");
    }

    #[test]
    fn parses_date_first_filename() {
        let metadata =
            metadata_from_filename(Path::new("2026-01-14 - Anthropic Economic Index.pdf"));
        assert_eq!(metadata.year, Some(2026));
        assert!(metadata.authors.is_empty());
        assert_eq!(metadata.title, "Anthropic Economic Index");
    }
}
