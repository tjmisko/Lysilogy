//! SQLite is a replaceable projection; allocation and admission records are durable.

mod identifiers;
mod journal;
mod projection;
mod query;
#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Decision, Person, PersonId, Work, WorkId};
pub use journal::{Admission, ArtifactSource, EntityProjection};
pub use query::{
    EntityAssertion, Neighbor, Neighborhood, NeighborhoodEdge, NeighborhoodLimits, SearchResults,
};

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("knowledge base storage: {0}")]
    Io(#[from] std::io::Error),
    #[error("knowledge base database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("knowledge base JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid knowledge base state: {0}")]
    Invalid(String),
}

impl From<StoreError> for crate::Error {
    fn from(value: StoreError) -> Self {
        Self::Task(value.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct KbStore {
    data: Arc<PathBuf>,
    directory: Arc<PathBuf>,
    connection: Arc<Mutex<Connection>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RebuildSummary {
    pub works: u64,
    pub persons: u64,
    pub observations: u64,
    pub citations: u64,
    pub aliases: u64,
    pub canonical_records: u64,
    pub reading_lists: u64,
}

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/001_core.sql"),
    include_str!("migrations/002_search.sql"),
    include_str!("migrations/003_assertions.sql"),
];

impl KbStore {
    /// Open/migrate a data root without discovering PDFs or opening reader notes.
    pub fn open(data_root: impl AsRef<Path>) -> Result<Self> {
        let data = data_root.as_ref();
        fs::create_dir_all(data)?;
        let data = fs::canonicalize(data)?;
        let directory = data.join("kb");
        safe_directory(&directory)?;
        safe_directory(&directory.join("admitted"))?;
        safe_directory(&directory.join("lists"))?;
        let _guard = journal::lock(&directory)?;
        for name in ["kb.sqlite", "kb.sqlite-wal", "kb.sqlite-shm"] {
            check_file(&directory.join(name), true)?;
        }
        let mut connection = Connection::open_with_flags(
            directory.join("kb.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        migrate(&mut connection)?;
        let store = Self {
            data: Arc::new(data),
            directory: Arc::new(directory),
            connection: Arc::new(Mutex::new(connection)),
        };
        // Validate the whole canonical chain on every opening. Existing projection
        // rows may be cached, but no altered/truncated journal is silently accepted.
        store.synchronize(true)?;
        let projected_version: usize = store.connection()?.query_row(
            "SELECT projection_version FROM projection_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if projected_version != MIGRATIONS.len() {
            store.rebuild_locked()?;
        }
        Ok(store)
    }

    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.directory.join("kb.sqlite")
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StoreError::Invalid("database mutex poisoned".into()))
    }

    /// Allocate once by a stable observation origin, never by title or row number.
    pub fn allocate_work(&self, origin: &str) -> Result<WorkId> {
        self.allocate("work", origin)?
            .parse()
            .map_err(StoreError::Invalid)
    }

    pub fn allocate_person(&self, origin: &str) -> Result<PersonId> {
        self.allocate("person", origin)?
            .parse()
            .map_err(StoreError::Invalid)
    }

    fn allocate(&self, kind: &str, origin: &str) -> Result<String> {
        journal::validate_token(origin, "allocation origin", 4096)?;
        let _guard = journal::lock(&self.directory)?;
        self.synchronize(false)?;
        let mut connection = self.connection()?;
        if let Some(id) = connection
            .query_row(
                "SELECT id FROM allocations WHERE kind=?1 AND origin=?2",
                params![kind, origin],
                |row| row.get(0),
            )
            .optional()?
        {
            return Ok(id);
        }
        let mut random = [0_u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let id = format!("{}{}", if kind == "work" { 'W' } else { 'P' }, hex(&random));
        self.append(
            &mut connection,
            journal::Event::Allocate {
                id: id.clone(),
                kind: kind.into(),
                origin: origin.into(),
            },
        )?;
        Ok(id)
    }

    /// Admit a verified source revision. The original source may later expire or
    /// change; the exact admitted payload is retained before its binding is saved.
    pub fn admit(&self, admission: &Admission, source_root: &Path) -> Result<()> {
        admission.verify_source(source_root)?;
        let _guard = journal::lock(&self.directory)?;
        self.synchronize(false)?;
        let mut connection = self.connection()?;
        let bytes = serde_json::to_vec(admission)?;
        let revision = digest(&bytes);
        journal::retain(&self.directory, &revision, &bytes)?;
        self.append(&mut connection, journal::Event::Admit { revision })
    }

    /// Store/replay an already-authorized decision. Matching and user commands
    /// decide when to submit one; the store never invents semantic equivalence.
    pub fn record_decision(&self, decision: &Decision) -> Result<()> {
        let _guard = journal::lock(&self.directory)?;
        self.synchronize(false)?;
        let mut connection = self.connection()?;
        self.append(
            &mut connection,
            journal::Event::Decision {
                decision: decision.clone(),
            },
        )
    }

    fn append(&self, connection: &mut Connection, event: journal::Event) -> Result<()> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = journal::state(&transaction)?;
        let record = journal::Record {
            schema_version: 1,
            sequence: state.sequence + 1,
            previous_sha256: state.sha256.clone(),
            event,
        };
        let bytes = serde_json::to_vec(&record)?;
        let hash = digest(&bytes);
        // Validate the complete change before touching the canonical log.
        projection::apply(&transaction, &self.directory, &record)?;
        let stamp = journal::append(&self.directory, &bytes, &state)?;
        projection::checkpoint(
            &transaction,
            &record,
            &bytes,
            &hash,
            state.bytes
                + u64::try_from(bytes.len())
                    .map_err(|_| StoreError::Invalid("record too large".into()))?
                + 1,
            &stamp,
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn synchronize(&self, full_validation: bool) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        journal::replay(&transaction, &self.directory, full_validation)?;
        journal::mirror_lists(&transaction, &self.directory)?;
        transaction.commit()?;
        Ok(())
    }

    /// Replace the projection in-place in one transaction. Existing readers see
    /// the previous snapshot until commit; WAL files are never renamed or deleted.
    pub fn rebuild(&self) -> Result<RebuildSummary> {
        let _guard = journal::lock(&self.directory)?;
        self.rebuild_locked()
    }

    fn rebuild_locked(&self) -> Result<RebuildSummary> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Check cached canonical hashes before discarding their independent mirror.
        journal::replay(&transaction, &self.directory, true)?;
        projection::clear(&transaction)?;
        journal::replay(&transaction, &self.directory, true)?;
        journal::mirror_lists(&transaction, &self.directory)?;
        let summary = summary(&transaction)?;
        transaction.execute(
            "UPDATE projection_state SET projection_version=?1 WHERE singleton=1",
            [MIGRATIONS.len()],
        )?;
        transaction.commit()?;
        Ok(summary)
    }

    pub fn summary(&self) -> Result<RebuildSummary> {
        let connection = self.connection()?;
        let transaction = connection.unchecked_transaction()?;
        summary(&transaction)
    }

    pub fn work(&self, id: &WorkId) -> Result<Option<Work>> {
        let connection = self.connection()?;
        let transaction = connection.unchecked_transaction()?;
        let resolved = projection::resolve(&transaction, id.as_str())?;
        read_entity(&transaction, "works", &resolved)
    }

    pub fn person(&self, id: &PersonId) -> Result<Option<Person>> {
        let connection = self.connection()?;
        let transaction = connection.unchecked_transaction()?;
        let resolved = projection::resolve(&transaction, id.as_str())?;
        read_entity(&transaction, "persons", &resolved)
    }

    /// Canonical logical snapshot for rebuild comparison; excludes SQLite rowids,
    /// physical pages, mutable timing and redundant FTS implementation tables.
    pub fn snapshot(&self) -> Result<BTreeMap<String, Vec<Value>>> {
        let connection = self.connection()?;
        let transaction = connection.unchecked_transaction()?;
        projection::snapshot(&transaction)
    }

    #[must_use]
    pub fn data_root(&self) -> &Path {
        &self.data
    }
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let version: usize = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > MIGRATIONS.len() {
        return Err(StoreError::Invalid(format!(
            "database schema {version} is newer than this program"
        )));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", index + 1)?;
    }
    transaction.commit()?;
    Ok(())
}

fn summary(connection: &Connection) -> Result<RebuildSummary> {
    let count = |table: &str| -> Result<u64> {
        Ok(
            connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })?,
        )
    };
    Ok(RebuildSummary {
        works: count("works")?,
        persons: count("persons")?,
        observations: count("observations")?,
        citations: count("citations")?,
        aliases: count("aliases")?,
        canonical_records: count("canonical_records")?,
        reading_lists: count("reading_lists")?,
    })
}

fn read_entity<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    table: &str,
    id: &str,
) -> Result<Option<T>> {
    let body: Option<String> = connection
        .query_row(
            &format!("SELECT body FROM {table} WHERE id=?1"),
            [id],
            |row| row.get(0),
        )
        .optional()?;
    body.map(|body| serde_json::from_str(&body).map_err(StoreError::from))
        .transpose()
}

fn safe_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(StoreError::Invalid(format!(
            "expected a real directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn check_file(path: &Path, missing_ok: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(StoreError::Invalid(format!(
            "expected a regular file: {}",
            path.display()
        ))),
        Err(error) if missing_ok && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            write!(text, "{byte:02x}").expect("String writes cannot fail");
            text
        })
}
