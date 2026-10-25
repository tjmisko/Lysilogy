//! The ledger of Works whose outgoing fanout has been made known.
//!
//! The neighborhood itself is `graph.citation_assertion`; this table only records that a
//! Work was expanded, how, and when, so a repeated expansion can be answered from cache.

use chrono::{DateTime, Utc};
use lysilogy_core::{
    ExpansionDirection, ExpansionMethod, ExpansionState, ExpansionUnavailableReason,
    SourceRecordId, WorkId,
};
use serde::Serialize;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;

const EXPANSION_COLUMNS: &str = "work_id, direction, state, method, unavailable_reason, provider_key, \
    source_record_id, references_count, new_works, warnings, attempts, last_error, \
    expanded_at, created_at, updated_at";

/// One stored expansion outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExpansionRow {
    pub work_id: WorkId,
    pub direction: ExpansionDirection,
    pub state: ExpansionState,
    pub method: Option<ExpansionMethod>,
    pub unavailable_reason: Option<ExpansionUnavailableReason>,
    pub provider_key: Option<String>,
    pub source_record_id: Option<SourceRecordId>,
    pub references_count: i32,
    pub new_works: i32,
    pub warnings: Vec<String>,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub expanded_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An expansion outcome to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewExpansion {
    pub work_id: WorkId,
    pub state: ExpansionState,
    pub method: Option<ExpansionMethod>,
    pub unavailable_reason: Option<ExpansionUnavailableReason>,
    pub provider_key: Option<String>,
    pub source_record_id: Option<SourceRecordId>,
    pub references_count: i32,
    pub new_works: i32,
    pub warnings: Vec<String>,
}

/// Reads and writes the expansion ledger.
#[derive(Clone)]
pub struct ExpansionRepository {
    pool: PgPool,
}

impl ExpansionRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The references ledger row for a Work, if any.
    pub async fn get(&self, work: WorkId) -> Result<Option<ExpansionRow>> {
        self.get_directed(work, ExpansionDirection::References)
            .await
    }

    /// The ledger row for a Work in one direction, if any.
    pub async fn get_directed(
        &self,
        work: WorkId,
        direction: ExpansionDirection,
    ) -> Result<Option<ExpansionRow>> {
        let row = sqlx::query(&format!(
            "select {EXPANSION_COLUMNS} from ops.work_expansion \
             where work_id = $1 and direction = $2"
        ))
        .bind(work.as_uuid())
        .bind(direction.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(expansion_from_row).transpose()
    }

    /// References ledger rows for the given Works, ordered by Work ID.
    pub async fn rows_for(&self, works: &[WorkId]) -> Result<Vec<ExpansionRow>> {
        self.rows_for_directed(works, ExpansionDirection::References)
            .await
    }

    /// Ledger rows in one direction for the given Works, ordered by Work ID.
    pub async fn rows_for_directed(
        &self,
        works: &[WorkId],
        direction: ExpansionDirection,
    ) -> Result<Vec<ExpansionRow>> {
        if works.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|work| work.as_uuid()).collect();
        let rows = sqlx::query(&format!(
            "select {EXPANSION_COLUMNS} from ops.work_expansion \
             where work_id = any($1) and direction = $2 order by work_id"
        ))
        .bind(ids)
        .bind(direction.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(expansion_from_row).collect()
    }

    /// Store a references outcome, replacing any earlier one. Each call counts as an
    /// attempt, stamps `expanded_at`, and clears the last error.
    pub async fn record(&self, expansion: NewExpansion) -> Result<ExpansionRow> {
        self.record_directed(ExpansionDirection::References, expansion)
            .await
    }

    /// Like [`Self::record`], for one direction; the other direction's row is untouched.
    pub async fn record_directed(
        &self,
        direction: ExpansionDirection,
        expansion: NewExpansion,
    ) -> Result<ExpansionRow> {
        let row = sqlx::query(&format!(
            "insert into ops.work_expansion as e \
                 (work_id, direction, state, method, unavailable_reason, provider_key, \
                  source_record_id, references_count, new_works, warnings, attempts) \
             values ($1, $10, $2, $3, $4, $5, $6, $7, $8, $9, 1) \
             on conflict (work_id, direction) do update set \
                 state = excluded.state, method = excluded.method, \
                 unavailable_reason = excluded.unavailable_reason, \
                 provider_key = excluded.provider_key, \
                 source_record_id = excluded.source_record_id, \
                 references_count = excluded.references_count, \
                 new_works = excluded.new_works, warnings = excluded.warnings, \
                 attempts = e.attempts + 1, last_error = null, \
                 expanded_at = now(), updated_at = now() \
             returning {EXPANSION_COLUMNS}"
        ))
        .bind(expansion.work_id.as_uuid())
        .bind(expansion.state.as_str())
        .bind(expansion.method.map(ExpansionMethod::as_str))
        .bind(
            expansion
                .unavailable_reason
                .map(ExpansionUnavailableReason::as_str),
        )
        .bind(expansion.provider_key)
        .bind(expansion.source_record_id.map(SourceRecordId::as_uuid))
        .bind(expansion.references_count)
        .bind(expansion.new_works)
        .bind(serde_json::json!(expansion.warnings))
        .bind(direction.as_str())
        .fetch_one(&self.pool)
        .await?;
        expansion_from_row(&row)
    }

    /// Note a failed attempt on an existing references row; a Work with no row is left
    /// untouched.
    pub async fn record_failure(&self, work: WorkId, message: &str) -> Result<()> {
        self.record_failure_directed(work, ExpansionDirection::References, message)
            .await
    }

    /// Like [`Self::record_failure`], for one direction.
    pub async fn record_failure_directed(
        &self,
        work: WorkId,
        direction: ExpansionDirection,
        message: &str,
    ) -> Result<()> {
        sqlx::query(
            "update ops.work_expansion set attempts = attempts + 1, last_error = $2, \
                 updated_at = now() where work_id = $1 and direction = $3",
        )
        .bind(work.as_uuid())
        .bind(message)
        .bind(direction.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn expansion_from_row(row: &PgRow) -> Result<ExpansionRow> {
    let direction: String = row.try_get("direction")?;
    let state: String = row.try_get("state")?;
    let method: Option<String> = row.try_get("method")?;
    let reason: Option<String> = row.try_get("unavailable_reason")?;
    let warnings: serde_json::Value = row.try_get("warnings")?;
    Ok(ExpansionRow {
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        direction: direction.parse()?,
        state: state.parse()?,
        method: method.map(|text| text.parse()).transpose()?,
        unavailable_reason: reason.map(|text| text.parse()).transpose()?,
        provider_key: row.try_get("provider_key")?,
        source_record_id: row
            .try_get::<Option<Uuid>, _>("source_record_id")?
            .map(SourceRecordId::from_uuid),
        references_count: row.try_get("references_count")?,
        new_works: row.try_get("new_works")?,
        warnings: warnings
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        attempts: row.try_get("attempts")?,
        last_error: row.try_get("last_error")?,
        expanded_at: row.try_get("expanded_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn expanded(work: WorkId, references: i32) -> NewExpansion {
        NewExpansion {
            work_id: work,
            state: ExpansionState::Expanded,
            method: Some(ExpansionMethod::Provider),
            unavailable_reason: None,
            provider_key: Some("W1".into()),
            source_record_id: None,
            references_count: references,
            new_works: 2,
            warnings: vec!["a".into(), "b".into()],
        }
    }

    #[tokio::test]
    async fn should_upsert_and_bump_attempts_when_recording_expansion() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ExpansionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        assert!(repo.get(work).await.expect("get").is_none());

        let first = repo.record(expanded(work, 3)).await.expect("first");
        assert_eq!(first.attempts, 1);
        assert_eq!(first.references_count, 3);
        assert_eq!(first.warnings, ["a", "b"]);
        assert_eq!(first.method, Some(ExpansionMethod::Provider));

        repo.record_failure(work, "boom").await.expect("failure");
        let failed = repo.get(work).await.expect("get").expect("row");
        assert_eq!(failed.attempts, 2);
        assert_eq!(failed.last_error.as_deref(), Some("boom"));

        let mut unavailable = expanded(work, 0);
        unavailable.state = ExpansionState::Unavailable;
        unavailable.method = None;
        unavailable.unavailable_reason = Some(ExpansionUnavailableReason::ProviderNotFound);
        unavailable.warnings = Vec::new();
        let second = repo.record(unavailable).await.expect("second");
        assert_eq!(second.attempts, 3);
        assert_eq!(second.state, ExpansionState::Unavailable);
        assert_eq!(second.method, None);
        assert_eq!(second.last_error, None);
        assert!(second.warnings.is_empty());
        assert_eq!(second.created_at, first.created_at);
        assert!(second.expanded_at >= first.expanded_at);

        let other = test_support::new_work(&pool).await;
        repo.record(expanded(other, 1)).await.expect("other");
        let rows = repo.rows_for(&[work, other]).await.expect("rows");
        assert_eq!(rows.len(), 2);
        assert!(repo.rows_for(&[]).await.expect("empty").is_empty());
    }

    #[tokio::test]
    async fn should_violate_check_when_unavailable_row_has_no_reason() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = test_support::new_work(&pool).await;
        for (state, method, reason) in [
            ("unavailable", None, None),
            ("unavailable", Some("local"), Some("provider_not_found")),
            ("expanded", None, None),
            ("expanded", Some("local"), Some("provider_not_found")),
            ("expanded", Some("cached"), None),
            ("unavailable", None, Some("paywalled")),
            ("pending", None, None),
        ] {
            let result = sqlx::query(
                "insert into ops.work_expansion (work_id, state, method, unavailable_reason) \
                 values ($1, $2, $3, $4)",
            )
            .bind(work.as_uuid())
            .bind(state)
            .bind(method)
            .bind(reason)
            .execute(&pool)
            .await;
            assert!(
                result.is_err(),
                "{state} {method:?} {reason:?} must be refused"
            );
        }
        let negative = sqlx::query(
            "insert into ops.work_expansion (work_id, state, method, references_count) \
             values ($1, 'expanded', 'local', -1)",
        )
        .bind(work.as_uuid())
        .execute(&pool)
        .await;
        assert!(negative.is_err());
        sqlx::query(
            "insert into ops.work_expansion (work_id, state, unavailable_reason) \
             values ($1, 'unavailable', 'identifier_conflict')",
        )
        .bind(work.as_uuid())
        .execute(&pool)
        .await
        .expect("a consistent row is accepted");
    }

    #[tokio::test]
    async fn should_record_failure_only_on_existing_row() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ExpansionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        repo.record_failure(work, "nothing to bump")
            .await
            .expect("no-op");
        assert!(repo.get(work).await.expect("get").is_none());

        repo.record(expanded(work, 0)).await.expect("record");
        repo.record_failure(work, "first").await.expect("first");
        repo.record_failure(work, "second").await.expect("second");
        let row = repo.get(work).await.expect("get").expect("row");
        assert_eq!(row.attempts, 3);
        assert_eq!(row.last_error.as_deref(), Some("second"));
        assert_eq!(row.state, ExpansionState::Expanded);
    }

    #[tokio::test]
    async fn should_keep_directions_independent_when_recording_same_work() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ExpansionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let cited_by = ExpansionDirection::CitedBy;

        repo.record(expanded(work, 3)).await.expect("references");
        assert!(
            repo.get_directed(work, cited_by)
                .await
                .expect("get")
                .is_none()
        );

        let row = repo
            .record_directed(cited_by, expanded(work, 9))
            .await
            .expect("cited by");
        assert_eq!(row.direction, cited_by);
        assert_eq!(row.attempts, 1, "attempts are counted per direction");
        assert_eq!(
            repo.get(work)
                .await
                .expect("get")
                .expect("row")
                .references_count,
            3
        );

        repo.record_failure_directed(work, cited_by, "boom")
            .await
            .expect("failure");
        let failed = repo
            .get_directed(work, cited_by)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(failed.last_error.as_deref(), Some("boom"));
        let untouched = repo.get(work).await.expect("get").expect("row");
        assert_eq!(untouched.last_error, None);
        assert_eq!(untouched.attempts, 1);

        let rows = repo.rows_for(&[work]).await.expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].direction, ExpansionDirection::References);
        let rows = repo
            .rows_for_directed(&[work], cited_by)
            .await
            .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].references_count, 9);
    }

    #[tokio::test]
    async fn should_refuse_local_method_when_direction_is_cited_by() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ExpansionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let mut local = expanded(work, 1);
        local.method = Some(ExpansionMethod::Local);
        assert!(
            repo.record_directed(ExpansionDirection::CitedBy, local.clone())
                .await
                .is_err()
        );
        repo.record_directed(ExpansionDirection::References, local)
            .await
            .expect("local references are fine");
    }
}
