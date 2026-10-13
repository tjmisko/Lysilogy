//! Postgres persistence for the Lysilogy literature viewer.
//!
//! Connection pooling, schema migrations, and one repository module per aggregate.

use lysilogy_core::ParseStateError;
use sqlx::migrate::MigrateError;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Embedded migrations, applied in filename order. Adding a migration is just a
/// new `migrations/NNNN_*.sql` file; the version is recorded in
/// `_sqlx_migrations` so re-running is a no-op.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub mod citation;
pub mod document;
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
    use super::{connect, migrate, PgPool};

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

        migrate(&pool).await.expect("second migration run");
        let after_second = table_count(&pool).await.expect("count after second run");
        assert_eq!(after_first, after_second, "second migrate must be a no-op");
    }
}
