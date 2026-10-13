//! Persistence for immutable source records (provider payloads and extraction
//! snapshots), keyed by `(kind, source_key)` so re-ingest is idempotent.

use lysilogy_core::{Coverage, SourceRecordId};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;

/// Reads and writes evidence payloads contributed by a source.
#[derive(Clone)]
pub struct SourceRecordRepository {
    pool: PgPool,
}

impl SourceRecordRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert or refresh a source record, returning its ID.
    pub async fn upsert(
        &self,
        kind: &str,
        source_key: &str,
        coverage: Coverage,
        payload: &Value,
    ) -> Result<SourceRecordId> {
        let id: Uuid = sqlx::query_scalar(
            "insert into source.source_record (kind, source_key, coverage, payload) \
             values ($1, $2, $3, $4) \
             on conflict (kind, source_key) do update \
                 set payload = excluded.payload, coverage = excluded.coverage, retrieved_at = now() \
             returning id",
        )
        .bind(kind)
        .bind(source_key)
        .bind(coverage.as_str())
        .bind(payload)
        .fetch_one(&self.pool)
        .await?;
        Ok(SourceRecordId::from_uuid(id))
    }

    /// Fetch a stored payload by key.
    pub async fn get(&self, kind: &str, source_key: &str) -> Result<Option<Value>> {
        let row = sqlx::query(
            "select payload from source.source_record where kind = $1 and source_key = $2",
        )
        .bind(kind)
        .bind(source_key)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(row.try_get::<Option<Value>, _>("payload")?)
    }

    /// Fetch a stored payload together with its record ID.
    pub async fn find(
        &self,
        kind: &str,
        source_key: &str,
    ) -> Result<Option<(SourceRecordId, Value)>> {
        let row = sqlx::query(
            "select id, payload from source.source_record where kind = $1 and source_key = $2",
        )
        .bind(kind)
        .bind(source_key)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let id = SourceRecordId::from_uuid(row.try_get("id")?);
        let Some(payload) = row.try_get::<Option<Value>, _>("payload")? else {
            return Ok(None);
        };
        Ok(Some((id, payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn upsert_is_idempotent_and_refreshes_payload() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = SourceRecordRepository::new(pool);
        let key = format!("doi:10.9999/{}", uuid::Uuid::new_v4());

        let first = repo
            .upsert(
                "openalex_work",
                &key,
                Coverage::Complete,
                &serde_json::json!({"v": 1}),
            )
            .await
            .expect("first");
        let second = repo
            .upsert(
                "openalex_work",
                &key,
                Coverage::Complete,
                &serde_json::json!({"v": 2}),
            )
            .await
            .expect("second");
        assert_eq!(first, second, "same key reuses the record");

        let stored = repo
            .get("openalex_work", &key)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(stored, serde_json::json!({"v": 2}));
    }
}
