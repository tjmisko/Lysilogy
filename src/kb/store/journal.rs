use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Result, StoreError, check_file, digest, projection};
use crate::kb::{
    Authorship, Citation, Decision, Observation, ObservationPayload, ObservationSource, Person,
    Work,
};

const MAX_RECORD: u64 = 8 * 1024 * 1024;

/// The source root is supplied by the caller: the data root for paper artifacts,
/// the provider cache root for provider records. Paths are relative and verified.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSource {
    pub relative_path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "entity", content = "projection", rename_all = "snake_case")]
pub enum EntityProjection {
    Work(Work),
    Person(Person),
}

impl EntityProjection {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Work(work) => work.id.as_str(),
            Self::Person(person) => person.id.as_str(),
        }
    }
}

/// An admission is made by the owning parser/provider adapter, never inferred
/// from arbitrary cache entries during rebuild. Raw payloads remain untrusted.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    pub observation: Observation,
    pub entity: EntityProjection,
    pub source: ArtifactSource,
    #[serde(default)]
    pub authorships: Vec<Authorship>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

impl Admission {
    pub(super) fn verify_source(&self, root: &Path) -> Result<()> {
        validate_hash(&self.source.sha256)?;
        for citation in &self.citations {
            for evidence in &citation.evidence {
                let matches = match (evidence, &self.observation.source) {
                    (
                        crate::kb::CitationEvidence::Local { paper_id, .. },
                        ObservationSource::PdfMetadata { paper_id: source }
                        | ObservationSource::Bibliography {
                            paper_id: source, ..
                        },
                    ) => paper_id == source,
                    (
                        crate::kb::CitationEvidence::Provider { provider, .. },
                        ObservationSource::Provider {
                            provider: source, ..
                        },
                    ) => provider == source,
                    _ => false,
                };
                if !matches {
                    return Err(StoreError::Invalid(
                        "citation evidence does not belong to its admitted observation source"
                            .into(),
                    ));
                }
            }
        }
        let relative = Path::new(&self.source.relative_path);
        if relative.components().next().is_none()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(StoreError::Invalid(
                "source path must be relative without traversal".into(),
            ));
        }
        let mut path = root.to_path_buf();
        for component in relative.components() {
            let text = component.as_os_str().to_string_lossy();
            if text == ".secrets" || text == ".env" || text.starts_with(".env.") {
                return Err(StoreError::Invalid("forbidden source path".into()));
            }
            path.push(component);
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(StoreError::Invalid("source path contains a symlink".into()));
            }
        }
        match &self.observation.source {
            ObservationSource::Bibliography { paper_id, .. }
            | ObservationSource::PdfMetadata { paper_id } => {
                if !paper_id
                    .as_str()
                    .parse::<crate::domain::PaperId>()
                    .is_ok_and(|parsed| parsed == *paper_id)
                {
                    return Err(StoreError::Invalid("invalid source PaperId".into()));
                }
                if !relative.starts_with(Path::new("papers").join(paper_id.as_str())) {
                    return Err(StoreError::Invalid(
                        "paper observation source does not belong to its PaperId".into(),
                    ));
                }
            }
            ObservationSource::Provider { .. } | ObservationSource::AiProposal { .. } => {}
        }
        let bytes = read_bounded(&path)?;
        if digest(&bytes) != self.source.sha256 {
            return Err(StoreError::Invalid(
                "source hash changed before admission".into(),
            ));
        }
        let matches = match &self.observation.payload {
            ObservationPayload::Text(text) => text.as_bytes() == bytes,
            ObservationPayload::Json(value) => serde_json::from_slice::<serde_json::Value>(&bytes)
                .is_ok_and(|source| source == *value),
        };
        if !matches {
            return Err(StoreError::Invalid(
                "observation payload does not match its verified source".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub schema_version: u32,
    pub sequence: u64,
    pub previous_sha256: String,
    pub event: Event,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Event {
    Allocate {
        id: String,
        kind: String,
        origin: String,
    },
    Admit {
        revision: String,
    },
    Decision {
        decision: Decision,
    },
}

pub(super) struct State {
    pub sequence: u64,
    pub bytes: u64,
    pub sha256: String,
}

pub(super) fn state(connection: &Connection) -> Result<State> {
    Ok(connection.query_row(
        "SELECT sequence,journal_bytes,sha256 FROM projection_state WHERE singleton=1",
        [],
        |row| {
            Ok(State {
                sequence: row.get(0)?,
                bytes: row.get(1)?,
                sha256: row.get(2)?,
            })
        },
    )?)
}

pub(super) struct JournalLock(File);

impl Drop for JournalLock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}

pub(super) fn lock(directory: &Path) -> Result<JournalLock> {
    let file = open_file(&directory.join("write.lock"), true, false)?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        .map_err(std::io::Error::from)?;
    Ok(JournalLock(file))
}

fn open_file(path: &Path, create: bool, append: bool) -> Result<File> {
    check_file(path, create)?;
    let mut flags = rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW;
    flags |= if create {
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CREATE
    } else {
        rustix::fs::OFlags::RDONLY
    };
    if append {
        flags |= rustix::fs::OFlags::APPEND;
    }
    let file: File = rustix::fs::open(path, flags, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
        .map_err(std::io::Error::from)?
        .into();
    if !file.metadata()?.is_file() {
        return Err(StoreError::Invalid(
            "expected regular canonical file".into(),
        ));
    }
    Ok(file)
}

pub(super) fn append(directory: &Path, bytes: &[u8], expected_length: u64) -> Result<()> {
    if bytes.len() as u64 > MAX_RECORD {
        return Err(StoreError::Invalid("canonical record exceeds 8 MiB".into()));
    }
    let path = directory.join("decisions.jsonl");
    let mut file = open_file(&path, true, true)?;
    if file.metadata()?.len() != expected_length {
        return Err(StoreError::Invalid(
            "canonical journal changed while writing".into(),
        ));
    }
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub(super) fn retain(directory: &Path, revision: &str, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 > MAX_RECORD {
        return Err(StoreError::Invalid(
            "admitted revision exceeds 8 MiB".into(),
        ));
    }
    let path = directory.join("admitted").join(format!("{revision}.json"));
    if path.try_exists()? {
        if read_bounded(&path)? != bytes {
            return Err(StoreError::Invalid(
                "retained admission hash collision or corruption".into(),
            ));
        }
        return Ok(());
    }
    let temporary = directory
        .join("admitted")
        .join(format!(".{revision}-{}.partial", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::hard_link(&temporary, &path)?;
    fs::remove_file(&temporary)?;
    File::open(directory.join("admitted"))?.sync_all()?;
    Ok(())
}

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_file(path, false, false)?
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(StoreError::Invalid("canonical source exceeds 8 MiB".into()));
    }
    Ok(bytes)
}

pub(super) fn admission(directory: &Path, revision: &str) -> Result<Admission> {
    validate_hash(revision)?;
    let bytes = read_bounded(&directory.join("admitted").join(format!("{revision}.json")))?;
    if digest(&bytes) != revision {
        return Err(StoreError::Invalid(
            "retained observation revision hash mismatch".into(),
        ));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn replay(connection: &Connection, directory: &Path, full: bool) -> Result<()> {
    let saved = state(connection)?;
    let path = directory.join("decisions.jsonl");
    if !path.try_exists()? {
        if saved.sequence != 0 {
            return Err(StoreError::Invalid("canonical journal disappeared".into()));
        }
        return Ok(());
    }
    let file = open_file(&path, false, false)?;
    if file.metadata()?.len() < saved.bytes {
        return Err(StoreError::Invalid(
            "canonical journal was truncated".into(),
        ));
    }
    let mut reader = BufReader::new(file);
    let mut current = if full {
        State {
            sequence: 0,
            bytes: 0,
            sha256: String::new(),
        }
    } else {
        State {
            sequence: saved.sequence,
            bytes: saved.bytes,
            sha256: saved.sha256.clone(),
        }
    };
    reader.seek(SeekFrom::Start(current.bytes))?;
    loop {
        let mut bytes = Vec::new();
        let read = reader
            .by_ref()
            .take(MAX_RECORD + 2)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            break;
        }
        if bytes.len() as u64 > MAX_RECORD + 1 || bytes.pop() != Some(b'\n') {
            return Err(StoreError::Invalid(
                "incomplete or oversized canonical journal record; preserve and repair the source"
                    .into(),
            ));
        }
        let record: Record = serde_json::from_slice(&bytes)?;
        if record.schema_version != 1
            || record.sequence != current.sequence + 1
            || record.previous_sha256 != current.sha256
        {
            return Err(StoreError::Invalid(
                "canonical journal sequence/hash chain mismatch".into(),
            ));
        }
        current = State {
            sequence: record.sequence,
            bytes: current.bytes
                + u64::try_from(read)
                    .map_err(|_| StoreError::Invalid("journal too large".into()))?,
            sha256: digest(&bytes),
        };
        if record.sequence <= saved.sequence {
            let mirrored: Option<String> = connection
                .query_row(
                    "SELECT sha256 FROM canonical_records WHERE sequence=?1",
                    [record.sequence],
                    |row| row.get(0),
                )
                .optional()?;
            if mirrored.as_deref() != Some(&current.sha256) {
                return Err(StoreError::Invalid(
                    "canonical journal differs from the admitted projection".into(),
                ));
            }
        } else {
            projection::apply(connection, directory, &record)?;
            projection::checkpoint(connection, &record, &bytes, &current.sha256, current.bytes)?;
        }
    }
    if current.sequence < saved.sequence {
        return Err(StoreError::Invalid("canonical journal lost records".into()));
    }
    Ok(())
}

pub(super) fn mirror_lists(connection: &Connection, directory: &Path) -> Result<()> {
    let mut paths = fs::read_dir(directory.join("lists"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let mut lists = Vec::new();
    for path in paths {
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| StoreError::Invalid("invalid reading-list filename".into()))?;
        if path.extension().and_then(|value| value.to_str()) != Some("json")
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || id.is_empty()
            || id.len() > 128
        {
            return Err(StoreError::Invalid(
                "invalid canonical reading-list path".into(),
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&read_bounded(&path)?)?;
        if value.get("id").and_then(serde_json::Value::as_str) != Some(id) {
            return Err(StoreError::Invalid(
                "reading-list filename and ID differ".into(),
            ));
        }
        lists.push((id.to_owned(), serde_json::to_string(&value)?));
    }
    connection.execute("DELETE FROM reading_lists", [])?;
    for (id, body) in lists {
        connection.execute("INSERT INTO reading_lists VALUES(?1,?2)", params![id, body])?;
    }
    Ok(())
}

pub(super) fn validate_token(value: &str, label: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(StoreError::Invalid(format!("invalid {label}")));
    }
    Ok(())
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StoreError::Invalid("expected lowercase SHA-256".into()));
    }
    Ok(())
}
