//! Persistence for bibliographic [`Work`]s, their identifiers, and field claims.

use chrono::{Datelike, NaiveDate};
use lysilogy_core::{
    IdentifierId, IdentifierScheme, IdentifierStatus, ParseStateError, SourceRecordId, VersionKind,
    Visibility, Work, WorkId, WorkIdentifier, WorkKind, WorkVersion, WorkVersionId,
    WorkVersionIdentifier, title_key,
};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

pub(crate) const WORK_COLUMNS: &str = "w.id, w.kind, w.visibility, w.display_title, w.title_state, \
                            w.display_year, w.year_state, w.display_authors, w.authors_state, \
                            w.created_at, w.updated_at";

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

    /// Fetch several Works in one query. Unknown IDs are omitted; order is unspecified.
    pub async fn get_many(&self, ids: &[WorkId]) -> Result<Vec<Work>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let uuids: Vec<uuid::Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
        let rows = sqlx::query(&format!(
            "select {WORK_COLUMNS} from graph.work w where w.id = any($1)"
        ))
        .bind(&uuids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(work_from_row).collect()
    }

    /// Find the Work that currently owns an active identifier.
    ///
    /// Version-level schemes (DOI, arXiv, PMID, ISBN, URL) are looked up on
    /// version identifiers; Work anchors on the Work's own identifiers.
    pub async fn find_by_identifier(
        &self,
        scheme: IdentifierScheme,
        value: &str,
    ) -> Result<Option<Work>> {
        if scheme.is_version_level() {
            return Ok(self
                .find_by_version_identifier(scheme, value)
                .await?
                .map(|(work, _)| work));
        }
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
                       year_state, display_authors, authors_state, created_at, updated_at",
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
        if scheme.is_version_level() {
            return self
                .resolve_or_create_version(scheme, value, kind, source_record_id)
                .await;
        }
        if let Some(work) = self.find_by_identifier(scheme, value).await? {
            return Ok((work, false));
        }

        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "insert into graph.work (kind) values ($1) \
             returning id, kind, visibility, display_title, title_state, display_year, \
                       year_state, display_authors, authors_state, created_at, updated_at",
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
        if scheme.is_version_level() {
            return self
                .add_version_identifier_for_work(work_id, scheme, value, source_record_id, status)
                .await;
        }
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

    /// A Work's active identifiers, ordered by scheme then age.
    ///
    /// Version-level identifiers from every version are included as well, so a
    /// caller sees the Work's full identity in one list.
    pub async fn active_identifiers(&self, work_id: WorkId) -> Result<Vec<WorkIdentifier>> {
        let rows = sqlx::query(
            "select id, work_id, scheme, value_normalized, source_record_id, status, created_at \
             from graph.work_identifier where work_id = $1 and status = 'active' \
             order by scheme, created_at, id",
        )
        .bind(work_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        let mut identifiers: Vec<WorkIdentifier> = rows
            .iter()
            .map(identifier_from_row)
            .collect::<Result<_>>()?;
        let version_rows = sqlx::query(
            "select i.id, i.scheme, i.value_normalized, i.source_record_id, i.status, i.created_at \
             from graph.work_version_identifier i \
             join graph.work_version v on v.id = i.version_id \
             where v.work_id = $1 and i.status = 'active' \
             order by i.scheme, i.created_at, i.id",
        )
        .bind(work_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        for row in &version_rows {
            identifiers.push(WorkIdentifier {
                id: IdentifierId::from_uuid(row.try_get("id")?),
                work_id,
                scheme: parse_state(&row.try_get::<String, _>("scheme")?)?,
                value_normalized: row.try_get("value_normalized")?,
                source_record_id: row
                    .try_get::<Option<Uuid>, _>("source_record_id")?
                    .map(SourceRecordId::from_uuid),
                status: parse_state(&row.try_get::<String, _>("status")?)?,
                created_at: row.try_get("created_at")?,
            });
        }
        identifiers.sort_by(|left, right| {
            left.scheme
                .as_str()
                .cmp(right.scheme.as_str())
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.id.as_uuid().cmp(&right.id.as_uuid()))
        });
        Ok(identifiers)
    }

    /// A Work's versions, oldest first.
    pub async fn versions(&self, work_id: WorkId) -> Result<Vec<WorkVersion>> {
        let rows = sqlx::query(
            "select id, work_id, version_kind, version_label, published_date, date_state, \
                    created_at from graph.work_version where work_id = $1 \
             order by created_at, id",
        )
        .bind(work_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(version_from_row).collect()
    }

    /// Create one manifestation of a Work.
    pub async fn create_version(
        &self,
        work_id: WorkId,
        version_kind: VersionKind,
        version_label: Option<&str>,
        published_date: Option<NaiveDate>,
    ) -> Result<WorkVersion> {
        let row = sqlx::query(
            "insert into graph.work_version (work_id, version_kind, version_label, published_date) \
             values ($1, $2, $3, $4) \
             returning id, work_id, version_kind, version_label, published_date, date_state, \
                       created_at",
        )
        .bind(work_id.as_uuid())
        .bind(version_kind.as_str())
        .bind(version_label)
        .bind(published_date)
        .fetch_one(&self.pool)
        .await?;
        version_from_row(&row)
    }

    /// A version's active identifiers, ordered by scheme then age.
    pub async fn version_identifiers(
        &self,
        version_id: WorkVersionId,
    ) -> Result<Vec<WorkVersionIdentifier>> {
        let rows = sqlx::query(
            "select id, version_id, scheme, value_normalized, source_record_id, status, created_at \
             from graph.work_version_identifier where version_id = $1 and status = 'active' \
             order by scheme, created_at, id",
        )
        .bind(version_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(version_identifier_from_row).collect()
    }

    /// Attach a version-level identifier. Returns `None` if an active claim
    /// already owns that exact identifier.
    pub async fn add_version_identifier(
        &self,
        version_id: WorkVersionId,
        scheme: IdentifierScheme,
        value: &str,
        source_record_id: Option<SourceRecordId>,
        status: IdentifierStatus,
    ) -> Result<Option<WorkVersionIdentifier>> {
        let row = sqlx::query(
            "insert into graph.work_version_identifier \
                 (version_id, scheme, value_normalized, source_record_id, status) \
             values ($1, $2, $3, $4, $5) \
             on conflict (scheme, value_normalized) where status = 'active' do nothing \
             returning id, version_id, scheme, value_normalized, source_record_id, status, created_at",
        )
        .bind(version_id.as_uuid())
        .bind(scheme.as_str())
        .bind(value)
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .bind(status.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(version_identifier_from_row).transpose()
    }

    /// Find the Work and version that own an active version identifier.
    pub async fn find_by_version_identifier(
        &self,
        scheme: IdentifierScheme,
        value: &str,
    ) -> Result<Option<(Work, WorkVersion)>> {
        let row = sqlx::query(&format!(
            "select {WORK_COLUMNS}, v.id as version_id, v.version_kind, v.version_label, \
                    v.published_date, v.date_state, v.created_at as version_created_at \
             from graph.work_version_identifier i \
             join graph.work_version v on v.id = i.version_id \
             join graph.work w on w.id = v.work_id \
             where i.scheme = $1 and i.value_normalized = $2 and i.status = 'active' \
             limit 1"
        ))
        .bind(scheme.as_str())
        .bind(value)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let work = work_from_row(&row)?;
        let version = WorkVersion {
            id: WorkVersionId::from_uuid(row.try_get("version_id")?),
            work_id: work.id,
            version_kind: parse_state(&row.try_get::<String, _>("version_kind")?)?,
            version_label: row.try_get("version_label")?,
            published_date: row.try_get("published_date")?,
            date_state: parse_state(&row.try_get::<String, _>("date_state")?)?,
            created_at: row.try_get("version_created_at")?,
        };
        Ok(Some((work, version)))
    }

    /// Select the display year from versions: the published version's year
    /// when one exists, otherwise the earliest known version year.
    pub async fn recompute_display_year(&self, work_id: WorkId) -> Result<Option<i32>> {
        let rows = sqlx::query(
            "select version_kind, published_date from graph.work_version where work_id = $1",
        )
        .bind(work_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        let mut published: Option<i32> = None;
        let mut earliest: Option<i32> = None;
        for row in &rows {
            let kind: String = row.try_get("version_kind")?;
            let Some(year) = row
                .try_get::<Option<NaiveDate>, _>("published_date")?
                .map(|date| date.year())
            else {
                continue;
            };
            if kind == "published" {
                published = Some(published.map_or(year, |current| current.min(year)));
            }
            earliest = Some(earliest.map_or(year, |current| current.min(year)));
        }
        let chosen = published.or(earliest);
        if let Some(year) = chosen {
            sqlx::query(
                "update graph.work set display_year = $2, year_state = 'known', \
                     updated_at = now() where id = $1",
            )
            .bind(work_id.as_uuid())
            .bind(year)
            .execute(&self.pool)
            .await?;
        }
        Ok(chosen)
    }

    /// Create a version-level identifier for a Work, making a version if needed.
    async fn add_version_identifier_for_work(
        &self,
        work_id: WorkId,
        scheme: IdentifierScheme,
        value: &str,
        source_record_id: Option<SourceRecordId>,
        status: IdentifierStatus,
    ) -> Result<Option<WorkIdentifier>> {
        let version = self.default_version(work_id).await?;
        let inserted = self
            .add_version_identifier(version.id, scheme, value, source_record_id, status)
            .await?;
        Ok(inserted.map(|identifier| WorkIdentifier {
            id: identifier.id,
            work_id,
            scheme: identifier.scheme,
            value_normalized: identifier.value_normalized,
            source_record_id: identifier.source_record_id,
            status: identifier.status,
            created_at: identifier.created_at,
        }))
    }

    /// A Work's first version, or a fresh one when it has none.
    async fn default_version(&self, work_id: WorkId) -> Result<WorkVersion> {
        if let Some(version) = self.versions(work_id).await?.into_iter().next() {
            return Ok(version);
        }
        self.create_version(work_id, VersionKind::Unknown, Some("default"), None)
            .await
    }

    /// Find or create a Work for a version-level identifier.
    async fn resolve_or_create_version(
        &self,
        scheme: IdentifierScheme,
        value: &str,
        kind: WorkKind,
        source_record_id: Option<SourceRecordId>,
    ) -> Result<(Work, bool)> {
        if let Some((work, _)) = self.find_by_version_identifier(scheme, value).await? {
            return Ok((work, false));
        }
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "insert into graph.work (kind) values ($1) \
             returning id, kind, visibility, display_title, title_state, display_year, \
                       year_state, display_authors, authors_state, created_at, updated_at",
        )
        .bind(kind.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let work = work_from_row(&row)?;
        let version_row = sqlx::query(
            "insert into graph.work_version (work_id, version_kind, version_label) \
             values ($1, $2, 'default') \
             returning id, work_id, version_kind, version_label, published_date, date_state, \
                       created_at",
        )
        .bind(work.id.as_uuid())
        .bind(version_kind_for_scheme(scheme).as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let version = version_from_row(&version_row)?;
        let inserted = sqlx::query(
            "insert into graph.work_version_identifier \
                 (version_id, scheme, value_normalized, source_record_id) \
             values ($1, $2, $3, $4) \
             on conflict (scheme, value_normalized) where status = 'active' do nothing \
             returning id",
        )
        .bind(version.id.as_uuid())
        .bind(scheme.as_str())
        .bind(value)
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .fetch_optional(&mut *transaction)
        .await?;
        if inserted.is_none() {
            transaction.rollback().await?;
            let existing = self
                .find_by_version_identifier(scheme, value)
                .await?
                .map(|(work, _)| work)
                .ok_or_else(|| {
                    DbError::Conflict("identifier conflict without a surviving Work".into())
                })?;
            return Ok((existing, false));
        }
        transaction.commit().await?;
        Ok((work, true))
    }

    /// Set the selected display title and mark the title known.
    ///
    /// Fixture-only: this bypasses claim selection, so a later reselect over the
    /// Work's claims may replace it. Production writers record claims instead.
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
    ///
    /// Fixture-only: this bypasses claim selection, like [`Self::set_display_title`].
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
        retract_colliding_overrides(&mut transaction, from_id, into_id).await?;
        move_work_content(&mut transaction, from_id, into_id).await?;
        crate::claim::reselect_in(&mut transaction, into).await?;

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

/// Retract `from`'s active override for every field where `into` also has one,
/// so the survivor's override wins and the moved claims keep the one-active
/// override index.
async fn retract_colliding_overrides(
    transaction: &mut PgConnection,
    from_id: Uuid,
    into_id: Uuid,
) -> Result<()> {
    sqlx::query(
        "update graph.field_claim f set retracted_at = now() \
         where f.work_id = $1 and f.origin = 'user' and f.retracted_at is null \
           and exists ( \
               select 1 from graph.field_claim i \
               where i.work_id = $2 and i.field = f.field \
                 and i.origin = 'user' and i.retracted_at is null)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
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
    sqlx::query(
        "delete from graph.work_version_identifier f using graph.work_version fv \
         where f.version_id = fv.id and fv.work_id = $1 and exists ( \
             select 1 from graph.work_version_identifier i \
             join graph.work_version iv on iv.id = i.version_id \
             where iv.work_id = $2 and i.scheme = f.scheme \
               and i.value_normalized = f.value_normalized)",
    )
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
    move_curation(transaction, from_id, into_id).await?;
    move_lists(transaction, from_id, into_id).await?;
    move_expansion(transaction, from_id, into_id).await
}

/// Move private tags and the mark to the surviving Work (its own mark wins) and
/// drop the merged Work's derived prominence rows.
async fn move_curation(transaction: &mut PgConnection, from_id: Uuid, into_id: Uuid) -> Result<()> {
    sqlx::query(
        "insert into reader.work_tag (workspace_id, work_id, tag_id, created_at) \
         select workspace_id, $2, tag_id, created_at from reader.work_tag where work_id = $1 \
         on conflict do nothing",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("delete from reader.work_tag where work_id = $1")
        .bind(from_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "update reader.work_mark m set work_id = $2 \
         where m.work_id = $1 and not exists ( \
             select 1 from reader.work_mark k \
             where k.workspace_id = m.workspace_id and k.work_id = $2)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("delete from reader.work_mark where work_id = $1")
        .bind(from_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("delete from reader.work_prominence where work_id = $1")
        .bind(from_id)
        .execute(&mut *transaction)
        .await?;
    Ok(())
}

/// Make reading lists, reading status and history follow the surviving Work.
///
/// In a list that already holds `into`, the merged Work's node is deleted: its note is
/// appended to the survivor's (or fills it when empty) and its edges move to the survivor,
/// dropping duplicates and edges that would become self edges. Otherwise the node is
/// repointed. Every path, `via_work_id` and list seed is repointed. Reading status keeps the
/// furthest of the two (read > reading > unread), and history rows are repointed.
async fn move_lists(transaction: &mut PgConnection, from_id: Uuid, into_id: Uuid) -> Result<()> {
    // Lists holding both: fold the merged node into the survivor.
    sqlx::query(
        "insert into reader.list_edge (list_id, from_node, to_node, kind, source, review, created_at) \
         select e.list_id, \
                case when e.from_node = f.id then i.id else e.from_node end, \
                case when e.to_node = f.id then i.id else e.to_node end, \
                e.kind, e.source, e.review, e.created_at \
         from reader.list_node f \
         join reader.list_node i \
             on i.list_id = f.list_id and i.kind = 'work' and i.work_id = $2 \
         join reader.list_edge e on e.from_node = f.id or e.to_node = f.id \
         where f.kind = 'work' and f.work_id = $1 \
           and not (e.from_node in (f.id, i.id) and e.to_node in (f.id, i.id)) \
         on conflict do nothing",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "update reader.list_node i set \
             note = left(case when i.note is null then f.note \
                              when f.note is null then i.note \
                              else i.note || E'\\n\\n' || f.note end, 2000), \
             updated_at = now() \
         from reader.list_node f \
         where f.list_id = i.list_id and f.kind = 'work' and f.work_id = $1 \
           and i.kind = 'work' and i.work_id = $2 and f.note is not null",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "delete from reader.list_node f where f.kind = 'work' and f.work_id = $1 and exists ( \
             select 1 from reader.list_node i \
             where i.list_id = f.list_id and i.kind = 'work' and i.work_id = $2)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "update reader.list_node set work_id = $2, updated_at = now(), \
             path_work_ids = array_replace(path_work_ids, $1, $2) \
         where work_id = $1",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "update reader.list_node set path_work_ids = array_replace(path_work_ids, $1, $2) \
         where $1 = any(path_work_ids)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("update reader.list_node set via_work_id = $2 where via_work_id = $1")
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "update reader.reading_list set seed_work_id = $2, updated_at = now() \
         where seed_work_id = $1",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "insert into reader.work_reading_state as s (workspace_id, work_id, status, changed_at) \
         select workspace_id, $2, status, changed_at \
         from reader.work_reading_state where work_id = $1 \
         on conflict (workspace_id, work_id) do update set \
             status = excluded.status, changed_at = excluded.changed_at \
         where (case excluded.status when 'read' then 2 when 'reading' then 1 else 0 end) \
             > (case s.status when 'read' then 2 when 'reading' then 1 else 0 end)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("delete from reader.work_reading_state where work_id = $1")
        .bind(from_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("update reader.history_event set work_id = $2 where work_id = $1")
        .bind(from_id)
        .bind(into_id)
        .execute(&mut *transaction)
        .await?;
    Ok(())
}

/// Move the merged Work's expansion rows to the survivor, per direction, only where the
/// survivor has no row of that direction.
async fn move_expansion(
    transaction: &mut PgConnection,
    from_id: Uuid,
    into_id: Uuid,
) -> Result<()> {
    sqlx::query(
        "delete from ops.work_expansion f where f.work_id = $1 and exists ( \
             select 1 from ops.work_expansion i \
             where i.work_id = $2 and i.direction = f.direction)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "update ops.work_expansion set work_id = $2, updated_at = now() where work_id = $1",
    )
    .bind(from_id)
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
        display_authors: row.try_get("display_authors")?,
        authors_state: parse_state(&row.try_get::<String, _>("authors_state")?)?,
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

fn version_from_row(row: &PgRow) -> Result<WorkVersion> {
    Ok(WorkVersion {
        id: WorkVersionId::from_uuid(row.try_get("id")?),
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        version_kind: parse_state(&row.try_get::<String, _>("version_kind")?)?,
        version_label: row.try_get("version_label")?,
        published_date: row.try_get("published_date")?,
        date_state: parse_state(&row.try_get::<String, _>("date_state")?)?,
        created_at: row.try_get("created_at")?,
    })
}

const fn version_kind_for_scheme(scheme: IdentifierScheme) -> VersionKind {
    match scheme {
        IdentifierScheme::Arxiv => VersionKind::Preprint,
        IdentifierScheme::Doi | IdentifierScheme::Pmid | IdentifierScheme::Isbn => {
            VersionKind::Published
        }
        IdentifierScheme::Openalex | IdentifierScheme::SemanticScholar | IdentifierScheme::Url => {
            VersionKind::Unknown
        }
    }
}

fn version_identifier_from_row(row: &PgRow) -> Result<WorkVersionIdentifier> {
    Ok(WorkVersionIdentifier {
        id: IdentifierId::from_uuid(row.try_get("id")?),
        version_id: WorkVersionId::from_uuid(row.try_get("version_id")?),
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
    use crate::claim::{ClaimRepository, NewClaim};
    use crate::test_support;
    use lysilogy_core::provenance::FieldValue;
    use lysilogy_core::{
        ClaimConfidence, ClaimOrigin, FieldState, ProvenanceField, ResolutionMethod,
        ResolutionStatus,
    };
    use uuid::Uuid;

    /// Record one openalex title claim, which selects it as the Work's title.
    async fn record_title(pool: &PgPool, work: WorkId, title: &str) {
        let value = FieldValue::Title(title.to_owned());
        ClaimRepository::new(pool.clone())
            .record(&[NewClaim {
                work_id: work,
                field: ProvenanceField::Title,
                value: &value,
                origin: ClaimOrigin::Openalex,
                extractor: "openalex",
                confidence: ClaimConfidence::Explicit,
                source_record_id: None,
                entry_id: None,
                locator: None,
            }])
            .await
            .expect("record title");
    }

    fn random_doi() -> String {
        format!("10.9999/{}", Uuid::new_v4())
    }

    #[tokio::test]
    async fn should_return_known_works_only_when_getting_many() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool);
        let first = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("first");
        let second = repo
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("second");
        let found = repo
            .get_many(&[first.id, second.id, WorkId::new(), first.id])
            .await
            .expect("get many");
        let mut ids: Vec<WorkId> = found.iter().map(|work| work.id).collect();
        ids.sort_unstable();
        let mut expected = vec![first.id, second.id];
        expected.sort_unstable();
        assert_eq!(ids, expected);
        assert!(repo.get_many(&[]).await.expect("empty").is_empty());
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
    async fn should_list_only_active_identifiers_when_work_has_conflicting_ones() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let doi = random_doi();
        let arxiv = format!("9999.{}", Uuid::new_v4().as_u128() % 90000 + 10000);
        let conflicting = random_doi();
        let retracted = random_doi();
        for (scheme, value, status) in [
            (IdentifierScheme::Doi, &doi, IdentifierStatus::Active),
            (IdentifierScheme::Arxiv, &arxiv, IdentifierStatus::Active),
            (
                IdentifierScheme::Doi,
                &conflicting,
                IdentifierStatus::Conflicting,
            ),
            (
                IdentifierScheme::Doi,
                &retracted,
                IdentifierStatus::Retracted,
            ),
        ] {
            repo.add_identifier(work, scheme, value, None, status)
                .await
                .expect("add")
                .expect("inserted");
        }

        let identifiers = repo.active_identifiers(work).await.expect("active");

        let found: Vec<(IdentifierScheme, &str)> = identifiers
            .iter()
            .map(|identifier| (identifier.scheme, identifier.value_normalized.as_str()))
            .collect();
        assert_eq!(
            found,
            vec![
                (IdentifierScheme::Arxiv, arxiv.as_str()),
                (IdentifierScheme::Doi, doi.as_str())
            ]
        );
        let empty = test_support::new_work(&pool).await;
        assert!(repo.active_identifiers(empty).await.unwrap().is_empty());
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

        record_title(&pool, work.id, "A Title").await;

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
        record_title(&pool, filled, &title).await;
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
        record_title(&pool, stub, "Stub title").await;
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
            "select i.value_normalized from graph.work_version_identifier i \
             join graph.work_version v on v.id = i.version_id \
             where v.work_id = $1 order by 1",
        )
        .bind(target.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("identifiers");
        assert_eq!(identifiers.len(), 2);
        let leftover: i64 = sqlx::query_scalar(
            "select count(*) from graph.work_version_identifier i \
             join graph.work_version v on v.id = i.version_id where v.work_id = $1",
        )
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

    async fn tag_names(
        curation: &crate::curation::CurationRepository,
        workspace: lysilogy_core::WorkspaceId,
        work: WorkId,
    ) -> Vec<String> {
        curation
            .tags_for_works(workspace, &[work])
            .await
            .expect("tags")
            .into_iter()
            .map(|(_, tag)| tag.name)
            .collect()
    }

    #[tokio::test]
    async fn should_move_tags_and_keep_target_mark_when_merging_works() {
        use crate::curation::CurationRepository;
        use lysilogy_core::{TagColor, TagKind, TagName, WorkMark};
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let curation = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let other_workspace = test_support::new_workspace(&pool).await;
        let from = test_support::new_work(&pool).await;
        let into = test_support::new_work(&pool).await;
        let color = TagColor::parse("#123456").expect("color");
        let mut tags = Vec::new();
        for name in ["shared", "only-from", "only-into"] {
            let name = TagName::parse(name).expect("name");
            tags.push(
                curation
                    .create_tag(workspace, &name, &color, TagKind::Topic)
                    .await
                    .expect("tag")
                    .id,
            );
        }
        let foreign = curation
            .create_tag(
                other_workspace,
                &TagName::parse("foreign").expect("name"),
                &color,
                TagKind::Topic,
            )
            .await
            .expect("foreign");
        curation
            .assign_tag(workspace, from, tags[0])
            .await
            .expect("assign");
        curation
            .assign_tag(workspace, from, tags[1])
            .await
            .expect("assign");
        curation
            .assign_tag(workspace, into, tags[0])
            .await
            .expect("assign");
        curation
            .assign_tag(workspace, into, tags[2])
            .await
            .expect("assign");
        curation
            .assign_tag(other_workspace, from, foreign.id)
            .await
            .expect("assign");
        curation
            .set_mark(workspace, from, WorkMark::Hidden, Some("from note"))
            .await
            .expect("mark");
        curation
            .set_mark(workspace, into, WorkMark::Pinned, Some("into note"))
            .await
            .expect("mark");

        repo.merge_work(from, into, "same paper", "test")
            .await
            .expect("merge");

        assert_eq!(
            tag_names(&curation, workspace, into).await,
            vec!["only-from", "only-into", "shared"]
        );
        assert!(tag_names(&curation, workspace, from).await.is_empty());
        assert_eq!(
            tag_names(&curation, other_workspace, into).await,
            vec!["foreign"],
            "other workspaces' tags move too"
        );
        let marks = curation
            .marks_for_works(workspace, &[from, into])
            .await
            .expect("marks");
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, into);
        assert_eq!(marks[0].1.mark, WorkMark::Pinned);
        assert_eq!(marks[0].1.note.as_deref(), Some("into note"));
    }

    #[tokio::test]
    async fn should_move_mark_when_target_has_none_and_drop_prominence_when_merging() {
        use crate::curation::CurationRepository;
        use crate::prominence::ProminenceRepository;
        use lysilogy_core::WorkMark;
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = WorkRepository::new(pool.clone());
        let curation = CurationRepository::new(pool.clone());
        let prominence = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let from = test_support::new_work(&pool).await;
        let into = test_support::new_work(&pool).await;
        curation
            .set_mark(workspace, from, WorkMark::Downplayed, Some("carry me"))
            .await
            .expect("mark");
        prominence
            .prominence_for(workspace, &[from, into], 2026)
            .await
            .expect("prominence");
        assert_eq!(
            prominence
                .cached(workspace, &[from, into])
                .await
                .expect("warm")
                .len(),
            2
        );

        repo.merge_work(from, into, "same paper", "test")
            .await
            .expect("merge");

        let marks = curation
            .marks_for_works(workspace, &[from, into])
            .await
            .expect("marks");
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, into);
        assert_eq!(marks[0].1.mark, WorkMark::Downplayed);
        assert_eq!(marks[0].1.note.as_deref(), Some("carry me"));
        let cached = prominence
            .cached(workspace, &[from, into])
            .await
            .expect("cold");
        assert_eq!(
            cached.iter().map(|row| row.work_id).collect::<Vec<_>>(),
            vec![into],
            "the merged Work's row is gone; the survivor's is kept"
        );
    }

    #[tokio::test]
    async fn should_move_expansion_rows_per_direction_when_merging_works() {
        use crate::expansion::{ExpansionRepository, NewExpansion};
        use lysilogy_core::{ExpansionDirection, ExpansionMethod, ExpansionState};

        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let expansions = ExpansionRepository::new(pool.clone());
        let record = |work: WorkId, references: i32| NewExpansion {
            work_id: work,
            state: ExpansionState::Expanded,
            method: Some(ExpansionMethod::Local),
            unavailable_reason: None,
            provider_key: None,
            source_record_id: None,
            references_count: references,
            new_works: 0,
            warnings: Vec::new(),
        };
        let count =
            |row: Option<crate::expansion::ExpansionRow>| row.map(|row| row.references_count);

        // The survivor has no row: the merged Work's moves. With its own, the survivor's wins.
        let from = test_support::new_work(&pool).await;
        let into = test_support::new_work(&pool).await;
        expansions.record(record(from, 7)).await.expect("record");
        works
            .merge_work(from, into, "dup", "test")
            .await
            .expect("merge");
        assert_eq!(count(expansions.get(from).await.expect("get")), None);
        assert_eq!(count(expansions.get(into).await.expect("get")), Some(7));

        let from = test_support::new_work(&pool).await;
        let into = test_support::new_work(&pool).await;
        expansions.record(record(from, 7)).await.expect("record");
        expansions.record(record(into, 2)).await.expect("record");
        works
            .merge_work(from, into, "dup", "test")
            .await
            .expect("merge");
        assert_eq!(count(expansions.get(from).await.expect("get")), None);
        assert_eq!(count(expansions.get(into).await.expect("get")), Some(2));

        // Directions are independent: only the direction the survivor lacks moves.
        let from = test_support::new_work(&pool).await;
        let into = test_support::new_work(&pool).await;
        let cited_by = |work: WorkId| NewExpansion {
            method: Some(ExpansionMethod::Provider),
            ..record(work, 5)
        };
        expansions.record(record(from, 7)).await.expect("record");
        expansions
            .record_directed(ExpansionDirection::CitedBy, cited_by(from))
            .await
            .expect("record");
        expansions
            .record_directed(ExpansionDirection::CitedBy, cited_by(into))
            .await
            .expect("record");
        works
            .merge_work(from, into, "dup", "test")
            .await
            .expect("merge");
        assert_eq!(
            count(expansions.get(into).await.expect("get")),
            Some(7),
            "the references row moved"
        );
        let kept = expansions
            .get_directed(into, ExpansionDirection::CitedBy)
            .await
            .expect("get");
        assert_eq!(count(kept), Some(5));
        assert_eq!(count(expansions.get(from).await.expect("get")), None);
    }
}
