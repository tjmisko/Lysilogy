//! Persistence for reader documents and content-hashed versions.
//!
//! Identity is content-driven: a document keeps its ID when its file moves, and
//! a new content hash creates a new immutable version rather than rewriting the
//! old one.

use std::collections::{HashMap, HashSet};

use lysilogy_core::{Document, DocumentId, ScannedFile, WorkId, WorkspaceId};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::Result;

const DOCUMENT_COLUMNS: &str = "id, workspace_id, work_id, relative_path, created_at, updated_at";

/// Outcome counts from reconciling a library scan against stored documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub added: usize,
    pub unchanged: usize,
    pub moved: usize,
    pub replaced: usize,
    pub duplicates: usize,
    pub tombstoned: usize,
}

/// Reads and writes reader documents and their versions.
#[derive(Clone)]
pub struct DocumentRepository {
    pool: PgPool,
}

impl DocumentRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new workspace with the given name.
    pub async fn create_workspace(&self, name: &str) -> Result<WorkspaceId> {
        let id: Uuid =
            sqlx::query_scalar("insert into reader.workspace (name) values ($1) returning id")
                .bind(name)
                .fetch_one(&self.pool)
                .await?;
        Ok(WorkspaceId::from_uuid(id))
    }

    /// Return the single local workspace, creating it on first use.
    pub async fn ensure_default_workspace(&self) -> Result<WorkspaceId> {
        let existing: Option<Uuid> =
            sqlx::query_scalar("select id from reader.workspace order by created_at, id limit 1")
                .fetch_optional(&self.pool)
                .await?;
        if let Some(id) = existing {
            return Ok(WorkspaceId::from_uuid(id));
        }
        let id: Uuid =
            sqlx::query_scalar("insert into reader.workspace (name) values ($1) returning id")
                .bind("Local")
                .fetch_one(&self.pool)
                .await?;
        Ok(WorkspaceId::from_uuid(id))
    }

    /// Fetch a document by its current library-relative path.
    pub async fn find_by_path(
        &self,
        workspace_id: WorkspaceId,
        path: &str,
    ) -> Result<Option<Document>> {
        let row = sqlx::query(&format!(
            "select {DOCUMENT_COLUMNS} from reader.document \
             where workspace_id = $1 and relative_path = $2"
        ))
        .bind(workspace_id.as_uuid())
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(document_from_row).transpose()
    }

    /// List all documents in a workspace, including tombstoned ones.
    pub async fn list_documents(&self, workspace_id: WorkspaceId) -> Result<Vec<Document>> {
        let rows = sqlx::query(&format!(
            "select {DOCUMENT_COLUMNS} from reader.document \
             where workspace_id = $1 order by relative_path nulls last, id"
        ))
        .bind(workspace_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(document_from_row).collect()
    }

    /// Link a document to a Work, or clear the link with `None`.
    pub async fn set_work(&self, document_id: DocumentId, work_id: Option<WorkId>) -> Result<()> {
        sqlx::query("update reader.document set work_id = $2, updated_at = now() where id = $1")
            .bind(document_id.as_uuid())
            .bind(work_id.map(WorkId::as_uuid))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Reconcile a full library scan with stored documents.
    ///
    /// A document whose file moved is followed by content hash and keeps its ID.
    /// A path whose contents changed gets a new immutable version. A path that
    /// disappeared is tombstoned (its `relative_path` set to NULL), not deleted.
    pub async fn apply_scan(
        &self,
        workspace_id: WorkspaceId,
        files: &[ScannedFile],
    ) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        let mut transaction = self.pool.begin().await?;
        let mut state = ScanState::load(&mut transaction, workspace_id, files).await?;

        for file in files {
            state.reconcile(&mut transaction, file, &mut report).await?;
        }
        state.tombstone(&mut transaction, &mut report).await?;

        transaction.commit().await?;
        Ok(report)
    }
}

/// In-memory view of a workspace's documents, updated as a scan proceeds.
struct ScanState {
    workspace_id: WorkspaceId,
    present: HashSet<String>,
    doc_path: HashMap<String, Uuid>,
    doc_path_of: HashMap<Uuid, Option<String>>,
    sha_docs: HashMap<String, Vec<Uuid>>,
    doc_shas: HashMap<Uuid, HashSet<String>>,
    claimed: HashSet<Uuid>,
}

impl ScanState {
    async fn load(
        connection: &mut PgConnection,
        workspace_id: WorkspaceId,
        files: &[ScannedFile],
    ) -> Result<Self> {
        let present: HashSet<String> = files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();

        let mut doc_path: HashMap<String, Uuid> = HashMap::new();
        let mut doc_path_of: HashMap<Uuid, Option<String>> = HashMap::new();
        let document_rows =
            sqlx::query("select id, relative_path from reader.document where workspace_id = $1")
                .bind(workspace_id.as_uuid())
                .fetch_all(&mut *connection)
                .await?;
        for row in &document_rows {
            let id: Uuid = row.try_get("id")?;
            let path: Option<String> = row.try_get("relative_path")?;
            if let Some(existing) = &path {
                doc_path.insert(existing.clone(), id);
            }
            doc_path_of.insert(id, path);
        }

        let mut sha_docs: HashMap<String, Vec<Uuid>> = HashMap::new();
        let mut doc_shas: HashMap<Uuid, HashSet<String>> = HashMap::new();
        let version_rows = sqlx::query(
            "select v.document_id, v.sha256 from reader.document_version v \
             join reader.document d on d.id = v.document_id where d.workspace_id = $1",
        )
        .bind(workspace_id.as_uuid())
        .fetch_all(&mut *connection)
        .await?;
        for row in &version_rows {
            let document: Uuid = row.try_get("document_id")?;
            let sha: String = row.try_get("sha256")?;
            sha_docs.entry(sha.clone()).or_default().push(document);
            doc_shas.entry(document).or_default().insert(sha);
        }

        Ok(Self {
            workspace_id,
            present,
            doc_path,
            doc_path_of,
            sha_docs,
            doc_shas,
            claimed: HashSet::new(),
        })
    }

    /// Find a document whose file vanished and can be followed to a new path.
    fn move_candidate(&self, sha: &str) -> Option<Uuid> {
        self.sha_docs.get(sha).and_then(|owners| {
            owners.iter().copied().find(|document| {
                !self.claimed.contains(document)
                    && self
                        .doc_path_of
                        .get(document)
                        .and_then(|path| path.as_ref())
                        .is_none_or(|path| !self.present.contains(path))
            })
        })
    }

    async fn reconcile(
        &mut self,
        connection: &mut PgConnection,
        file: &ScannedFile,
        report: &mut ScanReport,
    ) -> Result<()> {
        let path = &file.relative_path;

        if let Some(&document) = self.doc_path.get(path) {
            if self
                .doc_shas
                .get(&document)
                .is_some_and(|shas| shas.contains(&file.sha256))
            {
                report.unchanged += 1;
            } else {
                insert_version(connection, document, file).await?;
                self.doc_shas
                    .entry(document)
                    .or_default()
                    .insert(file.sha256.clone());
                self.sha_docs
                    .entry(file.sha256.clone())
                    .or_default()
                    .push(document);
                self.claimed.insert(document);
                report.replaced += 1;
            }
            return Ok(());
        }

        if let Some(document) = self.move_candidate(&file.sha256) {
            sqlx::query(
                "update reader.document set relative_path = $2, updated_at = now() where id = $1",
            )
            .bind(document)
            .bind(path)
            .execute(&mut *connection)
            .await?;
            self.doc_path.insert(path.clone(), document);
            self.doc_path_of.insert(document, Some(path.clone()));
            self.claimed.insert(document);
            report.moved += 1;
            return Ok(());
        }

        let document: Uuid = sqlx::query_scalar(
            "insert into reader.document (workspace_id, relative_path) \
             values ($1, $2) returning id",
        )
        .bind(self.workspace_id.as_uuid())
        .bind(path)
        .fetch_one(&mut *connection)
        .await?;
        insert_version(connection, document, file).await?;
        self.doc_path.insert(path.clone(), document);
        self.doc_path_of.insert(document, Some(path.clone()));
        self.doc_shas
            .entry(document)
            .or_default()
            .insert(file.sha256.clone());
        let owners = self.sha_docs.entry(file.sha256.clone()).or_default();
        owners.push(document);
        let is_duplicate = owners.len() > 1;
        self.claimed.insert(document);
        if is_duplicate {
            report.duplicates += 1;
        } else {
            report.added += 1;
        }
        Ok(())
    }

    async fn tombstone(&self, connection: &mut PgConnection, report: &mut ScanReport) -> Result<()> {
        for (&document, path) in &self.doc_path_of {
            if let Some(path) = path
                && !self.present.contains(path)
                && !self.claimed.contains(&document)
            {
                sqlx::query(
                    "update reader.document set relative_path = null, updated_at = now() \
                     where id = $1",
                )
                .bind(document)
                .execute(&mut *connection)
                .await?;
                report.tombstoned += 1;
            }
        }
        Ok(())
    }
}

async fn insert_version(
    connection: &mut PgConnection,
    document: Uuid,
    file: &ScannedFile,
) -> Result<()> {
    sqlx::query(
        "insert into reader.document_version (document_id, sha256, bytes, format) \
         values ($1, $2, $3, $4)",
    )
    .bind(document)
    .bind(&file.sha256)
    .bind(file.bytes)
    .bind(file.format.as_str())
    .execute(connection)
    .await?;
    Ok(())
}

fn document_from_row(row: &PgRow) -> Result<Document> {
    Ok(Document {
        id: DocumentId::from_uuid(row.try_get("id")?),
        workspace_id: WorkspaceId::from_uuid(row.try_get("workspace_id")?),
        work_id: row
            .try_get::<Option<Uuid>, _>("work_id")?
            .map(WorkId::from_uuid),
        relative_path: row.try_get("relative_path")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use lysilogy_core::DocumentFormat;

    fn file(path: &str, sha: &str) -> ScannedFile {
        ScannedFile {
            relative_path: path.to_owned(),
            sha256: sha.to_owned(),
            bytes: 10,
            format: DocumentFormat::Pdf,
        }
    }

    #[tokio::test]
    async fn scan_is_idempotent_and_move_tolerant() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = DocumentRepository::new(pool.clone());
        let workspace = repo.ensure_default_workspace().await.expect("workspace");
        sqlx::query("delete from reader.document where workspace_id = $1")
            .bind(workspace.as_uuid())
            .execute(&pool)
            .await
            .expect("clean");

        let report = repo
            .apply_scan(workspace, &[file("a.pdf", "sha-a"), file("b.pdf", "sha-b")])
            .await
            .expect("first scan");
        assert_eq!(
            (
                report.added,
                report.unchanged,
                report.moved,
                report.replaced,
                report.duplicates,
                report.tombstoned,
            ),
            (2, 0, 0, 0, 0, 0)
        );

        let report = repo
            .apply_scan(workspace, &[file("a.pdf", "sha-a"), file("b.pdf", "sha-b")])
            .await
            .expect("rescan");
        assert_eq!(report.unchanged, 2);
        assert_eq!(report.added, 0);

        let before = repo
            .find_by_path(workspace, "a.pdf")
            .await
            .expect("find")
            .expect("present");
        let report = repo
            .apply_scan(
                workspace,
                &[file("sub/c.pdf", "sha-a"), file("b.pdf", "sha-b")],
            )
            .await
            .expect("move");
        assert_eq!((report.moved, report.added, report.tombstoned), (1, 0, 0));
        let after = repo
            .find_by_path(workspace, "sub/c.pdf")
            .await
            .expect("find")
            .expect("present");
        assert_eq!(before.id, after.id, "identity survives the move");
        assert!(repo
            .find_by_path(workspace, "a.pdf")
            .await
            .expect("find")
            .is_none());

        let report = repo
            .apply_scan(
                workspace,
                &[
                    file("sub/c.pdf", "sha-a"),
                    file("dup.pdf", "sha-a"),
                    file("b.pdf", "sha-b"),
                ],
            )
            .await
            .expect("duplicate");
        assert_eq!((report.duplicates, report.added), (1, 0));

        let report = repo
            .apply_scan(
                workspace,
                &[
                    file("sub/c.pdf", "sha-c"),
                    file("dup.pdf", "sha-a"),
                    file("b.pdf", "sha-b"),
                ],
            )
            .await
            .expect("replace");
        assert_eq!(report.replaced, 1);

        let report = repo.apply_scan(workspace, &[]).await.expect("tombstone");
        assert_eq!(report.tombstoned, 3);
        assert_eq!(repo.list_documents(workspace).await.expect("list").len(), 3);
    }
}
