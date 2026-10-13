//! Persistence for bibliographic [`Work`]s, their identifiers, and field claims.

use lysilogy_core::{
    FieldKind, FieldState, IdentifierId, IdentifierScheme, IdentifierStatus, ParseStateError,
    SourceRecordId, Visibility, Work, WorkId, WorkIdentifier, WorkKind,
};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

const WORK_COLUMNS: &str = "w.id, w.kind, w.visibility, w.display_title, w.title_state, \
                            w.display_year, w.year_state, w.created_at, w.updated_at";

/// Reads and writes Works and their supporting claims.
#[derive(Clone)]
pub struct WorkRepository {
    pool: PgPool,
}

impl WorkRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Fetch a Work by its opaque identifier.
    pub async fn get(&self, id: WorkId) -> Result<Option<Work>> {
        let row = sqlx::query(&format!(
            "select {WORK_COLUMNS} from graph.work w where w.id = $1"
        ))
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(work_from_row).transpose()
    }

    /// Find the Work that currently owns an active identifier.
    pub async fn find_by_identifier(
        &self,
        scheme: IdentifierScheme,
        value: &str,
    ) -> Result<Option<Work>> {
        let row = sqlx::query(&format!(
            "select {WORK_COLUMNS} from graph.work w \
             join graph.work_identifier i on i.work_id = w.id \
             where i.scheme = $1 and i.value_normalized = $2 and i.status = 'active' \
             limit 1"
        ))
        .bind(scheme.as_str())
        .bind(value)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(work_from_row).transpose()
    }

    /// Create a Work with no identifiers or selected fields.
    pub async fn create(&self, kind: WorkKind, visibility: Visibility) -> Result<Work> {
        let row = sqlx::query(
            "insert into graph.work (kind, visibility) values ($1, $2) \
             returning id, kind, visibility, display_title, title_state, display_year, \
                       year_state, created_at, updated_at",
        )
        .bind(kind.as_str())
        .bind(visibility.as_str())
        .fetch_one(&self.pool)
        .await?;
        work_from_row(&row)
    }

    /// Find the Work owning an identifier, or create one and attach the identifier.
    ///
    /// Returns the Work and whether it was newly created. Safe to call repeatedly:
    /// if another writer claims the identifier first, the loser is rolled back and
    /// the winner's Work is returned.
    pub async fn resolve_or_create(
        &self,
        scheme: IdentifierScheme,
        value: &str,
        kind: WorkKind,
        source_record_id: Option<SourceRecordId>,
    ) -> Result<(Work, bool)> {
        if let Some(work) = self.find_by_identifier(scheme, value).await? {
            return Ok((work, false));
        }

        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "insert into graph.work (kind) values ($1) \
             returning id, kind, visibility, display_title, title_state, display_year, \
                       year_state, created_at, updated_at",
        )
        .bind(kind.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let work = work_from_row(&row)?;

        let inserted = sqlx::query(
            "insert into graph.work_identifier \
                 (work_id, scheme, value_normalized, source_record_id) \
             values ($1, $2, $3, $4) \
             on conflict (scheme, value_normalized) where status = 'active' do nothing \
             returning id",
        )
        .bind(work.id.as_uuid())
        .bind(scheme.as_str())
        .bind(value)
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .fetch_optional(&mut *transaction)
        .await?;

        if inserted.is_none() {
            transaction.rollback().await?;
            let existing = self.find_by_identifier(scheme, value).await?.ok_or_else(|| {
                DbError::Conflict("identifier conflict without a surviving Work".into())
            })?;
            return Ok((existing, false));
        }

        transaction.commit().await?;
        Ok((work, true))
    }

    /// Attach an identifier to a Work. Returns `None` if an active claim already
    /// owns that exact identifier.
    pub async fn add_identifier(
        &self,
        work_id: WorkId,
        scheme: IdentifierScheme,
        value: &str,
        source_record_id: Option<SourceRecordId>,
        status: IdentifierStatus,
    ) -> Result<Option<WorkIdentifier>> {
        let row = sqlx::query(
            "insert into graph.work_identifier \
                 (work_id, scheme, value_normalized, source_record_id, status) \
             values ($1, $2, $3, $4, $5) \
             on conflict (scheme, value_normalized) where status = 'active' do nothing \
             returning id, work_id, scheme, value_normalized, source_record_id, status, created_at",
        )
        .bind(work_id.as_uuid())
        .bind(scheme.as_str())
        .bind(value)
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .bind(status.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(identifier_from_row).transpose()
    }

    /// Record a provenance-bearing claim for a Work field.
    pub async fn add_claim(
        &self,
        work_id: WorkId,
        field: FieldKind,
        value: &Value,
        state: FieldState,
        source_record_id: Option<SourceRecordId>,
        locator: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "insert into graph.field_claim \
                 (work_id, field, value, state, source_record_id, locator) \
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(work_id.as_uuid())
        .bind(field.as_str())
        .bind(value)
        .bind(state.as_str())
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .bind(locator)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn parse_state<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr<Err = ParseStateError>,
{
    Ok(value.parse()?)
}

fn work_from_row(row: &PgRow) -> Result<Work> {
    Ok(Work {
        id: WorkId::from_uuid(row.try_get("id")?),
        kind: parse_state(&row.try_get::<String, _>("kind")?)?,
        visibility: parse_state(&row.try_get::<String, _>("visibility")?)?,
        display_title: row.try_get("display_title")?,
        title_state: parse_state(&row.try_get::<String, _>("title_state")?)?,
        display_year: row.try_get("display_year")?,
        year_state: parse_state(&row.try_get::<String, _>("year_state")?)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn identifier_from_row(row: &PgRow) -> Result<WorkIdentifier> {
    Ok(WorkIdentifier {
        id: IdentifierId::from_uuid(row.try_get("id")?),
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        scheme: parse_state(&row.try_get::<String, _>("scheme")?)?,
        value_normalized: row.try_get("value_normalized")?,
        source_record_id: row
            .try_get::<Option<Uuid>, _>("source_record_id")?
            .map(SourceRecordId::from_uuid),
        status: parse_state(&row.try_get::<String, _>("status")?)?,
        created_at: row.try_get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use uuid::Uuid;

    fn random_doi() -> String {
        format!("10.9999/{}", Uuid::new_v4())
    }

    #[tokio::test]
    async fn creates_and_reads_a_work() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool);

        let created = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create");
        assert_eq!(created.kind, WorkKind::Article);
        assert_eq!(created.title_state, FieldState::Unknown);

        let fetched = repo.get(created.id).await.expect("get").expect("present");
        assert_eq!(fetched, created);
    }

    #[tokio::test]
    async fn resolve_or_create_is_idempotent() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool);
        let doi = random_doi();

        let (first, created_first) = repo
            .resolve_or_create(IdentifierScheme::Doi, &doi, WorkKind::Article, None)
            .await
            .expect("first resolve");
        assert!(created_first);

        let (second, created_second) = repo
            .resolve_or_create(IdentifierScheme::Doi, &doi, WorkKind::Article, None)
            .await
            .expect("second resolve");
        assert!(!created_second);
        assert_eq!(first, second);

        let found = repo
            .find_by_identifier(IdentifierScheme::Doi, &doi)
            .await
            .expect("find");
        assert_eq!(found, Some(first));
    }

    #[tokio::test]
    async fn unknown_identifier_is_none() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool);
        let found = repo
            .find_by_identifier(IdentifierScheme::Doi, &random_doi())
            .await
            .expect("find");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn claims_and_identifiers_are_recorded() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let work = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create");

        repo.add_claim(
            work.id,
            FieldKind::Title,
            &serde_json::json!("A Title"),
            FieldState::Known,
            None,
            None,
        )
        .await
        .expect("claim");

        let identifier = repo
            .add_identifier(
                work.id,
                IdentifierScheme::Doi,
                &random_doi(),
                None,
                IdentifierStatus::Active,
            )
            .await
            .expect("identifier")
            .expect("inserted");
        assert_eq!(identifier.work_id, work.id);

        let claims: i64 =
            sqlx::query_scalar("select count(*) from graph.field_claim where work_id = $1")
                .bind(work.id.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count claims");
        assert_eq!(claims, 1);
    }
}
