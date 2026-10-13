//! Persistence for bibliographic [`Work`]s, their identifiers, and field claims.

use lysilogy_core::{
    FieldKind, FieldState, IdentifierId, IdentifierScheme, IdentifierStatus, ParseStateError,
    SourceRecordId, Visibility, Work, WorkId, WorkIdentifier, WorkKind, title_key,
};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

pub(crate) const WORK_COLUMNS: &str = "w.id, w.kind, w.visibility, w.display_title, w.title_state, \
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
            let existing = self
                .find_by_identifier(scheme, value)
                .await?
                .ok_or_else(|| {
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

    /// Set the selected display title and mark the title known.
    pub async fn set_display_title(&self, work_id: WorkId, title: &str) -> Result<()> {
        sqlx::query(
            "update graph.work set display_title = $2, title_state = 'known', \
                 title_key = nullif($3, ''), updated_at = now() \
             where id = $1",
        )
        .bind(work_id.as_uuid())
        .bind(title)
        .bind(title_key(title))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set the selected display year and mark the year known.
    pub async fn set_display_year(&self, work_id: WorkId, year: i32) -> Result<()> {
        sqlx::query(
            "update graph.work set display_year = $2, year_state = 'known', updated_at = now() \
             where id = $1",
        )
        .bind(work_id.as_uuid())
        .bind(year)
        .execute(&self.pool)
        .await?;
        Ok(())
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

    /// Record a claim unless an identical one (same Work, field, value, and
    /// source record) already exists. Returns whether a row was inserted.
    pub async fn add_claim_once(
        &self,
        work_id: WorkId,
        field: FieldKind,
        value: &Value,
        state: FieldState,
        source_record_id: Option<SourceRecordId>,
        locator: Option<&str>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "insert into graph.field_claim \
                 (work_id, field, value, state, source_record_id, locator) \
             select $1, $2, $3, $4, $5, $6 \
             where not exists ( \
                 select 1 from graph.field_claim \
                 where work_id = $1 and field = $2 and value = $3 \
                   and source_record_id is not distinct from $5)",
        )
        .bind(work_id.as_uuid())
        .bind(field.as_str())
        .bind(value)
        .bind(state.as_str())
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .bind(locator)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Set the display title only while the title state is not yet known.
    /// Returns whether the Work changed.
    pub async fn fill_display_title_if_unknown(
        &self,
        work_id: WorkId,
        title: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "update graph.work set display_title = $2, title_state = 'known', \
                 title_key = nullif($3, ''), updated_at = now() \
             where id = $1 and title_state <> 'known'",
        )
        .bind(work_id.as_uuid())
        .bind(title)
        .bind(title_key(title))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Set the display year only while the year state is not yet known.
    /// Returns whether the Work changed.
    pub async fn fill_display_year_if_unknown(&self, work_id: WorkId, year: i32) -> Result<bool> {
        let result = sqlx::query(
            "update graph.work set display_year = $2, year_state = 'known', updated_at = now() \
             where id = $1 and year_state <> 'known'",
        )
        .bind(work_id.as_uuid())
        .bind(year)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Works whose title key equals `key`, excluding redirected Works and,
    /// optionally, one Work.
    pub async fn find_by_title_key(&self, key: &str, exclude: Option<WorkId>) -> Result<Vec<Work>> {
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(&format!(
            "select {WORK_COLUMNS} from graph.work w \
             where w.title_key = $1 \
               and ($2::uuid is null or w.id <> $2) \
               and not exists (select 1 from graph.work_redirect r where r.old_work_id = w.id) \
             order by w.created_at, w.id"
        ))
        .bind(key)
        .bind(exclude.map(WorkId::as_uuid))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(work_from_row).collect()
    }

    /// Fill `title_key` for Works whose titles predate the column. Returns how
    /// many Works received a key.
    pub async fn backfill_title_keys(&self) -> Result<usize> {
        const BATCH: i64 = 500;
        let mut filled = 0usize;
        let mut after = Uuid::nil();
        loop {
            let rows = sqlx::query(
                "select id, display_title from graph.work \
                 where title_key is null and display_title is not null and id > $1 \
                 order by id limit $2",
            )
            .bind(after)
            .bind(BATCH)
            .fetch_all(&self.pool)
            .await?;
            let Some(last) = rows.last() else {
                return Ok(filled);
            };
            after = last.try_get("id")?;
            let mut ids = Vec::new();
            let mut keys = Vec::new();
            for row in &rows {
                let title: String = row.try_get("display_title")?;
                let key = title_key(&title);
                if key.is_empty() {
                    continue;
                }
                ids.push(row.try_get::<Uuid, _>("id")?);
                keys.push(key);
            }
            if ids.is_empty() {
                continue;
            }
            let result = sqlx::query(
                "update graph.work w set title_key = v.key \
                 from unnest($1::uuid[], $2::text[]) as v(id, key) \
                 where w.id = v.id and w.title_key is null",
            )
            .bind(&ids)
            .bind(&keys)
            .execute(&self.pool)
            .await?;
            #[allow(clippy::cast_possible_truncation)]
            let updated = result.rows_affected() as usize;
            filled += updated;
        }
    }

    /// Follow `work_redirect` from `id` to the surviving Work, at most 8 hops.
    pub async fn resolve_redirect(&self, id: WorkId) -> Result<WorkId> {
        const MAX_HOPS: usize = 8;
        let mut current = id;
        for _ in 0..MAX_HOPS {
            let next: Option<Uuid> = sqlx::query_scalar(
                "select new_work_id from graph.work_redirect where old_work_id = $1",
            )
            .bind(current.as_uuid())
            .fetch_optional(&self.pool)
            .await?;
            let Some(next) = next else {
                return Ok(current);
            };
            current = WorkId::from_uuid(next);
        }
        Ok(current)
    }

    /// Whether a Work is a pure local stub: every accepted resolution of it is
    /// `local_stub` and no document is linked to it.
    pub async fn is_auto_mergeable_stub(&self, id: WorkId) -> Result<bool> {
        let blocked: bool = sqlx::query_scalar(
            "select exists (select 1 from reader.document where work_id = $1) \
                 or exists (select 1 from graph.entry_resolution \
                            where candidate_work_id = $1 and status = 'accepted' \
                              and method <> 'local_stub')",
        )
        .bind(id.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        Ok(!blocked)
    }

    /// Merge the Work `from` into `into` in one transaction: record a redirect
    /// and a merge decision, repoint citation assertions and documents, move
    /// claims and identifiers, and move accepted entry resolutions.
    ///
    /// Refused with [`DbError::Conflict`] when the Works are equal, when either
    /// already redirects, or when a repointed assertion would cite itself.
    pub async fn merge_work(
        &self,
        from: WorkId,
        into: WorkId,
        reason: &str,
        actor: &str,
    ) -> Result<()> {
        if from == into {
            return Err(DbError::Conflict("cannot merge a Work into itself".into()));
        }
        let (from_id, into_id) = (from.as_uuid(), into.as_uuid());
        let mut transaction = self.pool.begin().await?;

        let locked: Vec<Uuid> = sqlx::query_scalar(
            "select id from graph.work where id = any($1) order by id for update",
        )
        .bind(vec![from_id, into_id])
        .fetch_all(&mut *transaction)
        .await?;
        if locked.len() != 2 {
            return Err(DbError::Conflict("both Works must exist to merge".into()));
        }
        let redirected: bool = sqlx::query_scalar(
            "select exists (select 1 from graph.work_redirect where old_work_id = any($1))",
        )
        .bind(vec![from_id, into_id])
        .fetch_one(&mut *transaction)
        .await?;
        if redirected {
            return Err(DbError::Conflict("a merged Work already redirects".into()));
        }
        let self_loop: bool = sqlx::query_scalar(
            "select exists (select 1 from graph.citation_assertion \
                 where (citing_work_id = $1 and cited_work_id = $2) \
                    or (citing_work_id = $2 and cited_work_id = $1))",
        )
        .bind(from_id)
        .bind(into_id)
        .fetch_one(&mut *transaction)
        .await?;
        if self_loop {
            return Err(DbError::Conflict(
                "merge would turn a citation between the Works into a self-citation".into(),
            ));
        }

        sqlx::query(
            "insert into graph.work_redirect (old_work_id, new_work_id, reason, decided_by) \
             values ($1, $2, $3, $4)",
        )
        .bind(from_id)
        .bind(into_id)
        .bind(reason)
        .bind(actor)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("update graph.work_redirect set new_work_id = $2 where new_work_id = $1")
            .bind(from_id)
            .bind(into_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "insert into graph.resolution_decision (kind, subject_ids, actor_id, reason) \
             values ('merge', $1, $2, $3)",
        )
        .bind(vec![from_id, into_id])
        .bind(actor)
        .bind(reason)
        .execute(&mut *transaction)
        .await?;

        repoint_assertions(&mut transaction, from_id, into_id).await?;
        move_work_content(&mut transaction, from_id, into_id).await?;

        transaction.commit().await?;
        Ok(())
    }

    /// Active `OpenAlex` identifiers for the given Works.
    pub async fn openalex_ids_for(&self, work_ids: &[WorkId]) -> Result<Vec<(WorkId, String)>> {
        if work_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = work_ids.iter().map(|id| id.as_uuid()).collect();
        let rows = sqlx::query(
            "select work_id, value_normalized from graph.work_identifier \
             where work_id = any($1) and scheme = $2 and status = 'active' \
             order by value_normalized",
        )
        .bind(&ids)
        .bind(IdentifierScheme::Openalex.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    WorkId::from_uuid(row.try_get("work_id")?),
                    row.try_get("value_normalized")?,
                ))
            })
            .collect()
    }
}

/// Repoint citation assertions from `from` to `into` on both sides, folding
/// assertions that would duplicate an existing one into the survivor.
async fn repoint_assertions(
    transaction: &mut PgConnection,
    from_id: Uuid,
    into_id: Uuid,
) -> Result<()> {
    for side in ["cited_work_id", "citing_work_id"] {
        let other = if side == "cited_work_id" {
            "citing_work_id"
        } else {
            "cited_work_id"
        };
        // Assertions that would duplicate an existing one after the repoint
        // fold into the survivor so their mentions and labels are kept.
        let duplicates = sqlx::query(&format!(
            "select distinct on (a.id) a.id as dupe, b.id as survivor \
             from graph.citation_assertion a \
             join graph.citation_assertion b on b.{side} = $2 \
              and b.{other} = a.{other} and b.id <> a.id \
              and ((a.source_record_id is not null and b.source_record_id = a.source_record_id) \
                or (a.entry_id is not null and b.entry_id = a.entry_id)) \
             where a.{side} = $1 order by a.id, b.id"
        ))
        .bind(from_id)
        .bind(into_id)
        .fetch_all(&mut *transaction)
        .await?;
        for row in &duplicates {
            let dupe: Uuid = row.try_get("dupe")?;
            let survivor: Uuid = row.try_get("survivor")?;
            sqlx::query(
                "update graph.citation_mention m set assertion_id = $2 \
                 where m.assertion_id = $1 and not exists ( \
                     select 1 from graph.citation_mention o \
                     where o.assertion_id = $2 and m.entry_id is not null \
                       and o.entry_id = m.entry_id \
                       and o.marker_start is not distinct from m.marker_start \
                       and o.page is not distinct from m.page)",
            )
            .bind(dupe)
            .bind(survivor)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "update graph.relationship_claim set assertion_id = $2 where assertion_id = $1",
            )
            .bind(dupe)
            .bind(survivor)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("delete from graph.citation_assertion where id = $1")
                .bind(dupe)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query(&format!(
            "update graph.citation_assertion set {side} = $2 where {side} = $1"
        ))
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;
    }
    Ok(())
}

/// Move documents, versions, claims, identifiers, and accepted entry
/// resolutions from `from` to `into`.
async fn move_work_content(
    transaction: &mut PgConnection,
    from_id: Uuid,
    into_id: Uuid,
) -> Result<()> {
    sqlx::query("update reader.document set work_id = $2, updated_at = now() where work_id = $1")
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("update graph.work_version set work_id = $2 where work_id = $1")
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("update graph.field_claim set work_id = $2 where work_id = $1")
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "delete from graph.work_identifier f where f.work_id = $1 and exists ( \
             select 1 from graph.work_identifier i \
             where i.work_id = $2 and i.scheme = f.scheme \
               and i.value_normalized = f.value_normalized)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("update graph.work_identifier set work_id = $2 where work_id = $1")
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;

    let withdrawn: Vec<Uuid> = sqlx::query_scalar(
        "update graph.entry_resolution set status = 'withdrawn', updated_at = now() \
         where candidate_work_id = $1 and status = 'accepted' returning entry_id",
    )
    .bind(from_id)
    .fetch_all(&mut *transaction)
    .await?;
    sqlx::query(
        "insert into graph.entry_resolution (entry_id, candidate_work_id, status, method) \
         select unnest($1::uuid[]), $2, 'accepted', 'merge' \
         on conflict (entry_id, candidate_work_id) do update \
             set status = 'accepted', method = 'merge', updated_at = now()",
    )
    .bind(&withdrawn)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

fn parse_state<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr<Err = ParseStateError>,
{
    Ok(value.parse()?)
}

pub(crate) fn work_from_row(row: &PgRow) -> Result<Work> {
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
    use lysilogy_core::{ResolutionMethod, ResolutionStatus};
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

    #[tokio::test]
    async fn should_not_duplicate_claim_when_added_twice() {
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
        let value = serde_json::json!("Same Title");

        let first = repo
            .add_claim_once(
                work.id,
                FieldKind::Title,
                &value,
                FieldState::Known,
                None,
                None,
            )
            .await
            .expect("first");
        let second = repo
            .add_claim_once(
                work.id,
                FieldKind::Title,
                &value,
                FieldState::Known,
                None,
                None,
            )
            .await
            .expect("second");
        let other = repo
            .add_claim_once(
                work.id,
                FieldKind::Title,
                &serde_json::json!("Other Title"),
                FieldState::Known,
                None,
                None,
            )
            .await
            .expect("other");
        assert_eq!((first, second, other), (true, false, true));

        let claims: i64 =
            sqlx::query_scalar("select count(*) from graph.field_claim where work_id = $1")
                .bind(work.id.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(claims, 2);
    }

    #[tokio::test]
    async fn should_not_overwrite_known_title_when_filling_display_title() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool);
        let work = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create");

        assert!(
            repo.fill_display_title_if_unknown(work.id, "First")
                .await
                .expect("fill")
        );
        assert!(
            !repo
                .fill_display_title_if_unknown(work.id, "Second")
                .await
                .expect("fill")
        );
        assert!(
            repo.fill_display_year_if_unknown(work.id, 2013)
                .await
                .expect("fill")
        );
        assert!(
            !repo
                .fill_display_year_if_unknown(work.id, 1999)
                .await
                .expect("fill")
        );

        let fetched = repo.get(work.id).await.expect("get").expect("present");
        assert_eq!(fetched.display_title.as_deref(), Some("First"));
        assert_eq!(fetched.display_year, Some(2013));
    }

    #[tokio::test]
    async fn should_return_openalex_ids_when_works_have_them() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool);
        let with_id = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create");
        let without_id = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create");
        let openalex = format!("W{}", Uuid::new_v4().simple());
        repo.add_identifier(
            with_id.id,
            IdentifierScheme::Openalex,
            &openalex,
            None,
            IdentifierStatus::Active,
        )
        .await
        .expect("identifier");
        repo.add_identifier(
            without_id.id,
            IdentifierScheme::Doi,
            &random_doi(),
            None,
            IdentifierStatus::Active,
        )
        .await
        .expect("identifier");

        let found = repo
            .openalex_ids_for(&[with_id.id, without_id.id])
            .await
            .expect("lookup");
        assert_eq!(found, vec![(with_id.id, openalex)]);
        assert!(repo.openalex_ids_for(&[]).await.expect("empty").is_empty());
    }

    // ---- title keys, redirects, and merges ----

    fn marker() -> String {
        Uuid::new_v4().simple().to_string()
    }

    async fn titled_work(repo: &WorkRepository, title: &str) -> WorkId {
        let work = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create");
        repo.set_display_title(work.id, title).await.expect("title");
        work.id
    }

    async fn stored_key(pool: &PgPool, id: WorkId) -> Option<String> {
        sqlx::query_scalar("select title_key from graph.work where id = $1")
            .bind(id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("key")
    }

    #[tokio::test]
    async fn should_write_title_key_when_display_title_is_set() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let tag = marker();
        let title = format!("Attention Is All You Need {tag}");
        let set = titled_work(&repo, &title).await;
        assert_eq!(stored_key(&pool, set).await, Some(title_key(&title)));

        let filled = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("create")
            .id;
        assert!(
            repo.fill_display_title_if_unknown(filled, &title)
                .await
                .expect("fill")
        );
        assert_eq!(stored_key(&pool, filled).await, Some(title_key(&title)));
        assert!(
            !repo
                .fill_display_title_if_unknown(filled, "Other")
                .await
                .expect("no-op")
        );
        assert_eq!(stored_key(&pool, filled).await, Some(title_key(&title)));

        let blank = titled_work(&repo, "   ").await;
        assert_eq!(
            stored_key(&pool, blank).await,
            None,
            "empty key is stored as NULL"
        );
    }

    #[tokio::test]
    async fn should_backfill_title_keys_when_titles_predate_the_column() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let tag = marker();
        let old = titled_work(&repo, &format!("Legacy Title {tag}")).await;
        let blank = titled_work(&repo, "   ").await;
        sqlx::query("update graph.work set title_key = null where id = any($1)")
            .bind(vec![old.as_uuid(), blank.as_uuid()])
            .execute(&pool)
            .await
            .expect("clear");

        let filled = repo.backfill_title_keys().await.expect("backfill");
        assert!(filled >= 1);
        assert_eq!(
            stored_key(&pool, old).await,
            Some(title_key(&format!("Legacy Title {tag}")))
        );
        assert_eq!(stored_key(&pool, blank).await, None);
        assert_eq!(repo.backfill_title_keys().await.expect("second run"), 0);
    }

    #[tokio::test]
    async fn should_find_work_by_title_key_when_not_redirected() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let title = format!("Deep Residual Learning {}", marker());
        let key = title_key(&title);
        let first = titled_work(&repo, &title).await;
        let second = titled_work(&repo, &title).await;

        let found = repo.find_by_title_key(&key, None).await.expect("find");
        let ids: Vec<_> = found.iter().map(|work| work.id).collect();
        assert_eq!(ids, vec![first, second]);

        let excluded = repo
            .find_by_title_key(&key, Some(first))
            .await
            .expect("exclude");
        assert_eq!(
            excluded.iter().map(|work| work.id).collect::<Vec<_>>(),
            vec![second]
        );

        repo.merge_work(first, second, "same paper", "test")
            .await
            .expect("merge");
        let after = repo
            .find_by_title_key(&key, None)
            .await
            .expect("after merge");
        assert_eq!(
            after.iter().map(|work| work.id).collect::<Vec<_>>(),
            vec![second]
        );
        assert!(
            repo.find_by_title_key("", None)
                .await
                .expect("empty")
                .is_empty()
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn should_repoint_assertions_and_record_decision_when_merging_stub() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let citations = crate::citation::CitationRepository::new(pool.clone());
        let tag = marker();
        let stub = works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("stub")
            .id;
        let target = works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("target")
            .id;
        let citing = test_support::new_work(&pool).await;
        let downstream = test_support::new_work(&pool).await;
        let provider = test_support::new_record(&pool).await;
        let version = test_support::new_version(&pool).await;
        let entries = test_support::new_entries(&pool, version, 1).await;

        // Both Works carry the same provider assertion from `citing`; the stub
        // also cites `downstream`.
        citations
            .add_assertion(citing, stub, Some(provider))
            .await
            .expect("stub edge");
        citations
            .add_assertion(citing, target, Some(provider))
            .await
            .expect("target edge");
        citations
            .add_entry_assertion(citing, stub, entries[0])
            .await
            .expect("local edge");
        citations
            .add_assertion(stub, downstream, Some(provider))
            .await
            .expect("downstream");

        let doi_shared = format!("10.9999/shared-{tag}");
        let doi_stub = format!("10.9999/stub-{tag}");
        works
            .add_identifier(
                stub,
                IdentifierScheme::Doi,
                &doi_stub,
                None,
                IdentifierStatus::Active,
            )
            .await
            .expect("id");
        works
            .add_identifier(
                stub,
                IdentifierScheme::Doi,
                &doi_shared,
                None,
                IdentifierStatus::Retracted,
            )
            .await
            .expect("retracted twin");
        works
            .add_identifier(
                target,
                IdentifierScheme::Doi,
                &doi_shared,
                None,
                IdentifierStatus::Active,
            )
            .await
            .expect("shared");
        works
            .add_claim(
                stub,
                FieldKind::Title,
                &serde_json::json!("Stub title"),
                FieldState::Known,
                None,
                None,
            )
            .await
            .expect("claim");
        let document: Uuid = sqlx::query_scalar(
            "insert into reader.document (workspace_id, work_id) \
             select id, $1 from reader.workspace limit 1 returning id",
        )
        .bind(stub.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("document");

        works
            .merge_work(stub, target, "same title", "lit:auto")
            .await
            .expect("merge");

        assert_eq!(
            works.resolve_redirect(stub).await.expect("redirect"),
            target
        );
        let citing_edges = citations.outgoing(citing).await.expect("outgoing");
        assert_eq!(
            citing_edges.iter().map(|w| w.id).collect::<Vec<_>>(),
            vec![target]
        );
        let target_edges = citations.outgoing(target).await.expect("target outgoing");
        assert_eq!(
            target_edges.iter().map(|w| w.id).collect::<Vec<_>>(),
            vec![downstream]
        );
        assert!(
            citations
                .outgoing(stub)
                .await
                .expect("stub outgoing")
                .is_empty()
        );
        let assertion_count: i64 = sqlx::query_scalar(
            "select count(*) from graph.citation_assertion where citing_work_id = $1 and cited_work_id = $2",
        )
        .bind(citing.as_uuid())
        .bind(target.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(
            assertion_count, 2,
            "provider duplicate folded, local assertion kept"
        );

        let linked: Uuid = sqlx::query_scalar("select work_id from reader.document where id = $1")
            .bind(document)
            .fetch_one(&pool)
            .await
            .expect("document work");
        assert_eq!(linked, target.as_uuid());
        let claims: i64 =
            sqlx::query_scalar("select count(*) from graph.field_claim where work_id = $1")
                .bind(target.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("claims");
        assert_eq!(claims, 1);
        let identifiers: Vec<String> = sqlx::query_scalar(
            "select value_normalized from graph.work_identifier where work_id = $1 order by 1",
        )
        .bind(target.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("identifiers");
        assert_eq!(identifiers.len(), 2);
        let leftover: i64 =
            sqlx::query_scalar("select count(*) from graph.work_identifier where work_id = $1")
                .bind(stub.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("leftover");
        assert_eq!(leftover, 0);

        let decisions: i64 = sqlx::query_scalar(
            "select count(*) from graph.resolution_decision \
             where kind = 'merge' and subject_ids = $1 and actor_id = 'lit:auto' and reason = 'same title'",
        )
        .bind(vec![stub.as_uuid(), target.as_uuid()])
        .fetch_one(&pool)
        .await
        .expect("decisions");
        assert_eq!(decisions, 1);
    }

    #[tokio::test]
    async fn should_withdraw_stub_resolution_when_merged() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let evidence = crate::evidence::EvidenceRepository::new(pool.clone());
        let stub = test_support::new_work(&pool).await;
        let target = test_support::new_work(&pool).await;
        let version = test_support::new_version(&pool).await;
        let entry = test_support::new_entries(&pool, version, 1).await[0];
        evidence
            .record_resolution(
                entry,
                stub,
                ResolutionStatus::Accepted,
                ResolutionMethod::LocalStub,
                None,
            )
            .await
            .expect("resolve");
        assert!(
            works
                .is_auto_mergeable_stub(stub)
                .await
                .expect("stub check")
        );

        works
            .merge_work(stub, target, "same", "lit:auto")
            .await
            .expect("merge");

        assert_eq!(
            evidence.accepted_resolution(entry).await.expect("accepted"),
            Some((target, ResolutionMethod::Merge))
        );
        let status: String = sqlx::query_scalar(
            "select status from graph.entry_resolution where entry_id = $1 and candidate_work_id = $2",
        )
        .bind(entry.as_uuid())
        .bind(stub.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("stub row");
        assert_eq!(status, "withdrawn");
    }

    #[tokio::test]
    async fn should_not_treat_work_as_auto_mergeable_when_it_has_document_or_real_resolution() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let evidence = crate::evidence::EvidenceRepository::new(pool.clone());
        let version = test_support::new_version(&pool).await;
        let entry = test_support::new_entries(&pool, version, 1).await[0];

        let resolved = test_support::new_work(&pool).await;
        evidence
            .record_resolution(
                entry,
                resolved,
                ResolutionStatus::Accepted,
                ResolutionMethod::Identifier,
                Some(1.0),
            )
            .await
            .expect("resolve");
        assert!(
            !works
                .is_auto_mergeable_stub(resolved)
                .await
                .expect("resolved")
        );

        let linked = test_support::new_work(&pool).await;
        sqlx::query(
            "insert into reader.document (workspace_id, work_id) \
             select id, $1 from reader.workspace limit 1",
        )
        .bind(linked.as_uuid())
        .execute(&pool)
        .await
        .expect("document");
        assert!(!works.is_auto_mergeable_stub(linked).await.expect("linked"));
    }

    #[tokio::test]
    async fn should_refuse_merge_when_it_would_create_a_self_citation() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let citations = crate::citation::CitationRepository::new(pool.clone());
        let left = test_support::new_work(&pool).await;
        let right = test_support::new_work(&pool).await;
        citations
            .add_assertion(left, right, None)
            .await
            .expect("edge");

        let error = works
            .merge_work(left, right, "oops", "test")
            .await
            .expect_err("refused");
        assert!(matches!(error, DbError::Conflict(_)), "{error:?}");
        let reverse = works
            .merge_work(right, left, "oops", "test")
            .await
            .expect_err("refused");
        assert!(matches!(reverse, DbError::Conflict(_)), "{reverse:?}");
        assert_eq!(works.resolve_redirect(left).await.expect("left"), left);
        assert_eq!(works.resolve_redirect(right).await.expect("right"), right);
        let decisions: i64 = sqlx::query_scalar(
            "select count(*) from graph.resolution_decision where subject_ids && $1",
        )
        .bind(vec![left.as_uuid(), right.as_uuid()])
        .fetch_one(&pool)
        .await
        .expect("decisions");
        assert_eq!(decisions, 0, "a refused merge leaves nothing behind");
    }

    #[tokio::test]
    async fn should_refuse_merge_when_source_already_redirects() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let (first, second, third) = (
            test_support::new_work(&pool).await,
            test_support::new_work(&pool).await,
            test_support::new_work(&pool).await,
        );
        let same = works
            .merge_work(first, first, "self", "test")
            .await
            .expect_err("same");
        assert!(matches!(same, DbError::Conflict(_)));

        works
            .merge_work(first, second, "dup", "test")
            .await
            .expect("merge");
        let again = works
            .merge_work(first, third, "dup", "test")
            .await
            .expect_err("redirected");
        assert!(matches!(again, DbError::Conflict(_)));
        let into_redirected = works
            .merge_work(third, first, "dup", "test")
            .await
            .expect_err("target redirected");
        assert!(matches!(into_redirected, DbError::Conflict(_)));
        assert_eq!(
            works.resolve_redirect(first).await.expect("redirect"),
            second
        );
    }

    #[tokio::test]
    async fn should_follow_redirect_chain_when_resolving_work() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let chain: Vec<WorkId> = {
            let mut ids = Vec::new();
            for _ in 0..11 {
                ids.push(test_support::new_work(&pool).await);
            }
            ids
        };
        assert_eq!(
            works.resolve_redirect(chain[0]).await.expect("no redirect"),
            chain[0]
        );

        // Insert raw redirects so a long chain exists (merge_work flattens them).
        for pair in chain.windows(2) {
            sqlx::query(
                "insert into graph.work_redirect (old_work_id, new_work_id) values ($1, $2)",
            )
            .bind(pair[0].as_uuid())
            .bind(pair[1].as_uuid())
            .execute(&pool)
            .await
            .expect("redirect");
        }
        assert_eq!(
            works.resolve_redirect(chain[8]).await.expect("short"),
            chain[10]
        );
        assert_eq!(
            works
                .resolve_redirect(chain[0])
                .await
                .expect("capped at eight hops"),
            chain[8]
        );
    }

    #[tokio::test]
    async fn should_flatten_older_redirects_when_merging_into_new_target() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let (oldest, middle, newest) = (
            test_support::new_work(&pool).await,
            test_support::new_work(&pool).await,
            test_support::new_work(&pool).await,
        );
        works
            .merge_work(oldest, middle, "a", "test")
            .await
            .expect("first");
        works
            .merge_work(middle, newest, "b", "test")
            .await
            .expect("second");
        let target: Uuid = sqlx::query_scalar(
            "select new_work_id from graph.work_redirect where old_work_id = $1",
        )
        .bind(oldest.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("target");
        assert_eq!(target, newest.as_uuid());
    }
}
