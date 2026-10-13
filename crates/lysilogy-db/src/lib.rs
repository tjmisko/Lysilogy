//! Postgres persistence for the Lysilogy literature viewer.
//!
//! Connection pooling, schema migrations, and one repository module per aggregate.

use lysilogy_core::ParseStateError;
use sqlx::migrate::MigrateError;
use sqlx::postgres::PgPoolOptions;

/// Re-exported so callers can hold a pool without depending on `sqlx` directly.
pub use sqlx::PgPool;

/// Embedded migrations, applied in filename order. Adding a migration is just a
/// new `migrations/NNNN_*.sql` file; the version is recorded in
/// `_sqlx_migrations` so re-running is a no-op.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub mod citation;
pub mod document;
pub mod evidence;
pub mod source_record;
pub mod work;

/// Errors raised by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] MigrateError),
    #[error("invalid state value: {0}")]
    State(#[from] ParseStateError),
    #[error("conflict: {0}")]
    Conflict(String),
}

/// Result alias for the persistence layer.
pub type Result<T> = std::result::Result<T, DbError>;

/// Connect to Postgres and return a connection pool.
pub async fn connect(url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await?;
    Ok(pool)
}

/// Apply all pending migrations to the given pool.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR.run(pool).await?;
    Ok(())
}

/// Shared state for database integration tests.
///
/// Integration tests require a real database and run against one shared test
/// database, so they take a process-wide lock to avoid racing on schema resets.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{PgPool, connect, migrate};

    pub static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Serialize database tests within this test binary.
    pub async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
        DB_LOCK.lock().await
    }

    /// The raw test database URL, if configured.
    pub fn url() -> Option<String> {
        std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())
    }

    /// A fresh Work.
    pub async fn new_work(pool: &PgPool) -> lysilogy_core::WorkId {
        crate::work::WorkRepository::new(pool.clone())
            .create(
                lysilogy_core::WorkKind::Article,
                lysilogy_core::Visibility::Private,
            )
            .await
            .expect("work")
            .id
    }

    /// A fresh document version in its own workspace.
    pub async fn new_version(pool: &PgPool) -> lysilogy_core::DocumentVersionId {
        let workspace: uuid::Uuid = sqlx::query_scalar(
            "insert into reader.workspace (name) values ('evidence-test') returning id",
        )
        .fetch_one(pool)
        .await
        .expect("workspace");
        let document: uuid::Uuid = sqlx::query_scalar(
            "insert into reader.document (workspace_id) values ($1) returning id",
        )
        .bind(workspace)
        .fetch_one(pool)
        .await
        .expect("document");
        let version: uuid::Uuid = sqlx::query_scalar(
            "insert into reader.document_version (document_id, sha256) values ($1, $2) returning id",
        )
        .bind(document)
        .bind(format!("sha-{}", uuid::Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("version");
        lysilogy_core::DocumentVersionId::from_uuid(version)
    }

    /// A fresh `local_bibliography` source record.
    pub async fn new_record(pool: &PgPool) -> lysilogy_core::SourceRecordId {
        crate::source_record::SourceRecordRepository::new(pool.clone())
            .upsert(
                "local_bibliography",
                &format!("test-{}", uuid::Uuid::new_v4()),
                lysilogy_core::Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("record")
    }

    /// Store `count` entries for a version and return their IDs in order.
    pub async fn new_entries(
        pool: &PgPool,
        version: lysilogy_core::DocumentVersionId,
        count: i32,
    ) -> Vec<lysilogy_core::EntryId> {
        let record = new_record(pool).await;
        let entries: Vec<crate::evidence::NewEntry> = (0..count)
            .map(|ordinal| sample_entry("t", ordinal))
            .collect();
        crate::evidence::EvidenceRepository::new(pool.clone())
            .insert_entries(version, record, &entries)
            .await
            .expect("entries")
    }

    /// A sample entry with key `<prefix>:<ordinal + 1>`.
    pub fn sample_entry(prefix: &str, ordinal: i32) -> crate::evidence::NewEntry {
        crate::evidence::NewEntry {
            entry_key: format!("{prefix}:{}", ordinal + 1),
            ordinal,
            raw_text: format!("[{}] Smith. A title. 2020.", ordinal + 1),
            printed_key: Some(format!("{}", ordinal + 1)),
            fields: serde_json::json!({"title": "A title"}),
            page: 3,
            text_start: 100 * ordinal,
            text_end: 100 * ordinal + 90,
            member_spans: serde_json::json!([{"start": 100 * ordinal, "end": 100 * ordinal + 90}]),
            region: Some(
                serde_json::json!({"x_min": 0.1, "y_min": 0.2, "x_max": 0.9, "y_max": 0.3}),
            ),
        }
    }

    /// Connect and migrate, or `None` when `LYSILOGY_TEST_DATABASE_URL` is unset.
    pub async fn pool() -> Option<PgPool> {
        let url = url()?;
        let pool = connect(&url).await.expect("connect to test database");
        migrate(&pool).await.expect("migrate test database");
        Some(pool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn drop_schemas(pool: &PgPool) -> Result<()> {
        sqlx::query("drop schema if exists source, graph, reader, publish, ops cascade")
            .execute(pool)
            .await?;
        sqlx::query("drop table if exists _sqlx_migrations")
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn table_count(pool: &PgPool) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "select count(*) from information_schema.tables \
             where table_schema in ('source', 'graph', 'reader', 'publish', 'ops')",
        )
        .fetch_one(pool)
        .await?;
        Ok(count)
    }

    #[tokio::test]
    async fn migrations_apply_and_are_idempotent() {
        let _guard = test_support::lock().await;
        let Some(url) = test_support::url() else {
            eprintln!("skipping migrations test: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let pool = connect(&url).await.expect("connect");
        drop_schemas(&pool).await.expect("reset");

        migrate(&pool).await.expect("first migration run");
        let after_first = table_count(&pool).await.expect("count after first run");
        assert!(
            after_first >= 14,
            "expected the Ramp A tables, found {after_first}"
        );

        for table in [
            "graph.work",
            "graph.work_identifier",
            "graph.work_version",
            "graph.field_claim",
            "graph.entry_resolution",
            "graph.citation_assertion",
            "graph.citation_mention",
            "graph.relationship_claim",
            "graph.work_redirect",
            "graph.resolution_decision",
            "source.source_record",
            "source.bibliography_entry",
            "reader.workspace",
            "reader.document",
            "reader.document_version",
        ] {
            let exists = sqlx::query_scalar::<_, bool>(
                "select exists (select 1 from information_schema.tables \
                 where table_schema || '.' || table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("table lookup");
            assert!(exists, "missing table {table}");
        }

        for column in [
            "graph.work.title_key",
            "source.bibliography_entry.ordinal",
            "source.bibliography_entry.fields",
            "graph.citation_mention.entry_id",
            "graph.citation_mention.marker_start",
            "graph.citation_mention.rects",
        ] {
            let exists = sqlx::query_scalar::<_, bool>(
                "select exists (select 1 from information_schema.columns \
                 where table_schema || '.' || table_name || '.' || column_name = $1)",
            )
            .bind(column)
            .fetch_one(&pool)
            .await
            .expect("column lookup");
            assert!(exists, "missing column {column}");
        }

        migrate(&pool).await.expect("second migration run");
        let after_second = table_count(&pool).await.expect("count after second run");
        assert_eq!(after_first, after_second, "second migrate must be a no-op");
    }
}
