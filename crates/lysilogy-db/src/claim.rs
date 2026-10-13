//! Persistence for field claims: append-only, sourced values for a Work's title,
//! year, authors, venue and DOI, plus the selection that picks the displayed one.
//!
//! Every write that can change what is selected (record, override, review,
//! merge) reselects the affected Work in the same transaction, using the pure
//! [`lysilogy_core::provenance::select_field`].

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use lysilogy_core::provenance::{ClaimCandidate, FieldValue, select_field};
use lysilogy_core::{
    ClaimConfidence, ClaimOrigin, EntryId, FieldClaimId, FieldState, ParseStateError,
    ProvenanceField, ReviewState, SourceRecordId, WorkId, title_key,
};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

/// Page size for [`ClaimRepository::reselect_all`].
const RESELECT_PAGE: i64 = 500;

/// SQL predicate selecting the claim rows provenance tracks: title, year, author
/// and venue claims, and DOI identifier claims.
const TRACKED_CLAIMS: &str = "(c.field in ('title', 'year', 'author', 'venue') \
     or (c.field = 'identifier' and c.value ->> 'scheme' = 'doi'))";

const CLAIM_COLUMNS: &str = "c.id, c.work_id, c.field, c.value, c.origin, c.extractor, \
     c.confidence, c.source_record_id, c.entry_id, c.locator, c.review_state, \
     c.retracted_at, c.created_at";

/// A claim to record.
#[derive(Clone, Copy, Debug)]
pub struct NewClaim<'a> {
    pub work_id: WorkId,
    pub field: ProvenanceField,
    pub value: &'a FieldValue,
    pub origin: ClaimOrigin,
    pub extractor: &'a str,
    pub confidence: ClaimConfidence,
    pub source_record_id: Option<SourceRecordId>,
    pub entry_id: Option<EntryId>,
    pub locator: Option<&'a str>,
}

/// A stored claim row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredClaim {
    pub id: FieldClaimId,
    pub work_id: WorkId,
    pub field: ProvenanceField,
    pub value: Value,
    pub origin: ClaimOrigin,
    pub extractor: String,
    pub confidence: ClaimConfidence,
    pub source_record_id: Option<SourceRecordId>,
    pub entry_id: Option<EntryId>,
    pub locator: Option<String>,
    pub review_state: ReviewState,
    pub retracted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// What a reselect changed on a Work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // one flag per selected field, plus the title transition
pub struct Reselection {
    pub title_changed: bool,
    pub year_changed: bool,
    pub authors_changed: bool,
    /// The Work had no title and now has one.
    pub title_became_known: bool,
}

impl Reselection {
    /// Whether any selected field changed.
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.title_changed || self.year_changed || self.authors_changed
    }
}

/// The result of [`ClaimRepository::record`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordOutcome {
    /// Rows actually inserted (duplicates are skipped).
    pub inserted: usize,
    /// The reselection of every Work that received at least one new row.
    pub reselected: BTreeMap<WorkId, Reselection>,
}

/// The identity of a source record, for provenance display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRecordBrief {
    pub id: SourceRecordId,
    pub kind: String,
    pub source_key: String,
    pub retrieved_at: DateTime<Utc>,
}

/// A bibliography entry, for provenance display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryBrief {
    pub id: EntryId,
    pub entry_key: String,
    pub raw_text: String,
    /// The Work owning the document whose bibliography holds the entry.
    pub citing_work_id: Option<WorkId>,
}

/// Reads and writes field claims and drives selection.
#[derive(Clone)]
pub struct ClaimRepository {
    pool: PgPool,
}

impl ClaimRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert each claim once and reselect every Work that got a new row.
    ///
    /// Claims are deduplicated on Work, field, value, origin, extractor, source
    /// record and entry. One transaction per Work.
    pub async fn record(&self, claims: &[NewClaim<'_>]) -> Result<RecordOutcome> {
        let mut by_work: BTreeMap<WorkId, Vec<&NewClaim<'_>>> = BTreeMap::new();
        for claim in claims {
            by_work.entry(claim.work_id).or_default().push(claim);
        }
        let mut outcome = RecordOutcome::default();
        for (work, work_claims) in by_work {
            let mut transaction = self.pool.begin().await?;
            lock_work(&mut transaction, work).await?;
            let mut inserted = 0;
            for claim in work_claims {
                inserted += insert_claim_once(&mut transaction, claim).await?;
            }
            if inserted == 0 {
                transaction.rollback().await?;
                continue;
            }
            let reselection = reselect_in(&mut transaction, work).await?;
            transaction.commit().await?;
            outcome.inserted += inserted;
            outcome.reselected.insert(work, reselection);
        }
        Ok(outcome)
    }

    /// Every tracked claim for a Work, oldest first.
    pub async fn claims_for(&self, work: WorkId) -> Result<Vec<StoredClaim>> {
        let rows = sqlx::query(&format!(
            "select {CLAIM_COLUMNS} from graph.field_claim c \
             where c.work_id = $1 and {TRACKED_CLAIMS} \
             order by c.created_at, c.id"
        ))
        .bind(work.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().filter_map(claim_from_row).collect()
    }

    /// Fetch one tracked claim.
    pub async fn get(&self, claim: FieldClaimId) -> Result<Option<StoredClaim>> {
        get_in(&self.pool, claim).await
    }

    /// Set the curator override for a field: retract the active one, insert a
    /// new `user` claim and reselect.
    pub async fn set_override(
        &self,
        work: WorkId,
        field: ProvenanceField,
        value: &FieldValue,
    ) -> Result<StoredClaim> {
        if value.field() != field {
            return Err(DbError::Conflict("override_field_mismatch".into()));
        }
        let mut transaction = self.pool.begin().await?;
        lock_work(&mut transaction, work).await?;
        retract_active_override(&mut transaction, work, field).await?;
        let row = sqlx::query(&format!(
            "with c as ( \
                 insert into graph.field_claim \
                     (work_id, field, value, origin, extractor, confidence, review_state) \
                 values ($1, $2, $3, 'user', 'curator', 'explicit', 'accepted') \
                 returning * ) \
             select {CLAIM_COLUMNS} from c"
        ))
        .bind(work.as_uuid())
        .bind(field.field_kind().as_str())
        .bind(value.to_json())
        .fetch_one(&mut *transaction)
        .await?;
        let stored = claim_from_row(&row).ok_or_else(DbError::not_found)??;
        reselect_in(&mut transaction, work).await?;
        transaction.commit().await?;
        Ok(stored)
    }

    /// Retract the active override for a field and reselect. Returns whether
    /// there was one.
    pub async fn clear_override(&self, work: WorkId, field: ProvenanceField) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        lock_work(&mut transaction, work).await?;
        let retracted = retract_active_override(&mut transaction, work, field).await?;
        if !retracted {
            transaction.rollback().await?;
            return Ok(false);
        }
        reselect_in(&mut transaction, work).await?;
        transaction.commit().await?;
        Ok(true)
    }

    /// Accept, reject or reset a non-user claim and reselect its Work.
    ///
    /// Returns `None` when the claim does not exist. User overrides are cleared,
    /// not reviewed: they fail with `DbError::Conflict("override_not_reviewable")`.
    pub async fn set_review_state(
        &self,
        claim: FieldClaimId,
        state: ReviewState,
    ) -> Result<Option<StoredClaim>> {
        let Some(existing) = self.get(claim).await? else {
            return Ok(None);
        };
        if existing.origin == ClaimOrigin::User {
            return Err(DbError::Conflict("override_not_reviewable".into()));
        }
        let mut transaction = self.pool.begin().await?;
        lock_work(&mut transaction, existing.work_id).await?;
        sqlx::query("update graph.field_claim set review_state = $2 where id = $1")
            .bind(claim.as_uuid())
            .bind(state.as_str())
            .execute(&mut *transaction)
            .await?;
        reselect_in(&mut transaction, existing.work_id).await?;
        let stored = get_in(&mut *transaction, claim).await?;
        transaction.commit().await?;
        Ok(stored)
    }

    /// Recompute a Work's selected title, year and authors from its claims.
    pub async fn reselect(&self, work: WorkId) -> Result<Reselection> {
        let mut transaction = self.pool.begin().await?;
        let reselection = reselect_in(&mut transaction, work).await?;
        transaction.commit().await?;
        Ok(reselection)
    }

    /// Reselect every Work that has at least one claim row. Returns how many.
    pub async fn reselect_all(&self) -> Result<usize> {
        let mut after = Uuid::nil();
        let mut total = 0;
        loop {
            let works: Vec<Uuid> = sqlx::query_scalar(
                "select distinct work_id from graph.field_claim where work_id > $1 \
                 order by work_id limit $2",
            )
            .bind(after)
            .bind(RESELECT_PAGE)
            .fetch_all(&self.pool)
            .await?;
            let Some(last) = works.last().copied() else {
                return Ok(total);
            };
            for work in works {
                self.reselect(WorkId::from_uuid(work)).await?;
                total += 1;
            }
            after = last;
        }
    }

    /// Identity of the given source records.
    pub async fn source_records(&self, ids: &[SourceRecordId]) -> Result<Vec<SourceRecordBrief>> {
        let uuids: Vec<Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
        let rows = sqlx::query(
            "select id, kind, source_key, retrieved_at from source.source_record \
             where id = any($1) order by retrieved_at, id",
        )
        .bind(uuids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SourceRecordBrief {
                    id: SourceRecordId::from_uuid(row.try_get("id")?),
                    kind: row.try_get("kind")?,
                    source_key: row.try_get("source_key")?,
                    retrieved_at: row.try_get("retrieved_at")?,
                })
            })
            .collect()
    }

    /// The bibliography entries with the given IDs and the Work citing from each.
    pub async fn entries(&self, ids: &[EntryId]) -> Result<Vec<EntryBrief>> {
        let uuids: Vec<Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
        let rows = sqlx::query(
            "select e.id, e.entry_key, e.raw_text, d.work_id as citing_work_id \
             from source.bibliography_entry e \
             join reader.document_version v on v.id = e.citing_version_id \
             join reader.document d on d.id = v.document_id \
             where e.id = any($1) order by e.created_at, e.id",
        )
        .bind(uuids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(EntryBrief {
                    id: EntryId::from_uuid(row.try_get("id")?),
                    entry_key: row.try_get("entry_key")?,
                    raw_text: row.try_get("raw_text")?,
                    citing_work_id: row
                        .try_get::<Option<Uuid>, _>("citing_work_id")?
                        .map(WorkId::from_uuid),
                })
            })
            .collect()
    }
}

fn parse_state<T>(value: &str) -> Result<T>
where
    T: FromStr<Err = ParseStateError>,
{
    Ok(value.parse()?)
}

fn field_from_kind(kind: &str) -> Option<ProvenanceField> {
    match kind {
        "title" => Some(ProvenanceField::Title),
        "year" => Some(ProvenanceField::Year),
        "author" => Some(ProvenanceField::Authors),
        "venue" => Some(ProvenanceField::Venue),
        "identifier" => Some(ProvenanceField::Doi),
        _ => None,
    }
}

/// Map a row to a claim; `None` for rows of an untracked field.
fn claim_from_row(row: &PgRow) -> Option<Result<StoredClaim>> {
    let kind: String = match row.try_get("field") {
        Ok(kind) => kind,
        Err(error) => return Some(Err(error.into())),
    };
    let field = field_from_kind(&kind)?;
    Some(stored_claim(row, field))
}

fn stored_claim(row: &PgRow, field: ProvenanceField) -> Result<StoredClaim> {
    Ok(StoredClaim {
        id: FieldClaimId::from_uuid(row.try_get("id")?),
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        field,
        value: row.try_get("value")?,
        origin: parse_state(&row.try_get::<String, _>("origin")?)?,
        extractor: row.try_get("extractor")?,
        confidence: parse_state(&row.try_get::<String, _>("confidence")?)?,
        source_record_id: row
            .try_get::<Option<Uuid>, _>("source_record_id")?
            .map(SourceRecordId::from_uuid),
        entry_id: row
            .try_get::<Option<Uuid>, _>("entry_id")?
            .map(EntryId::from_uuid),
        locator: row.try_get("locator")?,
        review_state: parse_state(&row.try_get::<String, _>("review_state")?)?,
        retracted_at: row.try_get("retracted_at")?,
        created_at: row.try_get("created_at")?,
    })
}

async fn get_in<'e, E>(executor: E, claim: FieldClaimId) -> Result<Option<StoredClaim>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(&format!(
        "select {CLAIM_COLUMNS} from graph.field_claim c where c.id = $1 and {TRACKED_CLAIMS}"
    ))
    .bind(claim.as_uuid())
    .fetch_optional(executor)
    .await?;
    row.as_ref().and_then(claim_from_row).transpose()
}

/// Lock the Work row for the rest of the transaction; fails if it is missing.
async fn lock_work(connection: &mut PgConnection, work: WorkId) -> Result<()> {
    let locked: Option<Uuid> =
        sqlx::query_scalar("select id from graph.work where id = $1 for update")
            .bind(work.as_uuid())
            .fetch_optional(&mut *connection)
            .await?;
    locked.map(|_| ()).ok_or_else(DbError::not_found)
}

async fn insert_claim_once(connection: &mut PgConnection, claim: &NewClaim<'_>) -> Result<usize> {
    let result = sqlx::query(
        "insert into graph.field_claim \
             (work_id, field, value, origin, extractor, confidence, \
              source_record_id, entry_id, locator) \
         select $1, $2, $3::jsonb, $4, $5, $6, $7::uuid, $8::uuid, $9::text \
         where not exists ( \
             select 1 from graph.field_claim \
             where work_id = $1 and field = $2 and value = $3::jsonb \
               and origin = $4 and extractor = $5 \
               and source_record_id is not distinct from $7::uuid \
               and entry_id is not distinct from $8::uuid)",
    )
    .bind(claim.work_id.as_uuid())
    .bind(claim.field.field_kind().as_str())
    .bind(claim.value.to_json())
    .bind(claim.origin.as_str())
    .bind(claim.extractor)
    .bind(claim.confidence.as_str())
    .bind(claim.source_record_id.map(SourceRecordId::as_uuid))
    .bind(claim.entry_id.map(EntryId::as_uuid))
    .bind(claim.locator)
    .execute(&mut *connection)
    .await?;
    Ok(usize::try_from(result.rows_affected()).unwrap_or(usize::MAX))
}

async fn retract_active_override(
    connection: &mut PgConnection,
    work: WorkId,
    field: ProvenanceField,
) -> Result<bool> {
    let mut sql = String::from(
        "update graph.field_claim set retracted_at = now() \
         where work_id = $1 and field = $2 and origin = 'user' and retracted_at is null",
    );
    if field == ProvenanceField::Doi {
        sql.push_str(" and value ->> 'scheme' = 'doi'");
    }
    let result = sqlx::query(&sql)
        .bind(work.as_uuid())
        .bind(field.field_kind().as_str())
        .execute(&mut *connection)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// A Work's selectable claims grouped by field.
struct ClaimGroups {
    titles: Vec<ClaimCandidate>,
    years: Vec<ClaimCandidate>,
    authors: Vec<ClaimCandidate>,
}

async fn load_claim_groups(connection: &mut PgConnection, work: WorkId) -> Result<ClaimGroups> {
    let rows = sqlx::query(
        "select id, field, value, origin, confidence, review_state, retracted_at, created_at \
         from graph.field_claim where work_id = $1 and field in ('title', 'year', 'author')",
    )
    .bind(work.as_uuid())
    .fetch_all(&mut *connection)
    .await?;
    let mut groups = ClaimGroups {
        titles: Vec::new(),
        years: Vec::new(),
        authors: Vec::new(),
    };
    for row in &rows {
        let kind: String = row.try_get("field")?;
        let Some(field) = field_from_kind(&kind) else {
            continue;
        };
        let value: Value = row.try_get("value")?;
        let candidate = ClaimCandidate {
            id: FieldClaimId::from_uuid(row.try_get("id")?),
            origin: parse_state(&row.try_get::<String, _>("origin")?)?,
            confidence: parse_state(&row.try_get::<String, _>("confidence")?)?,
            value: FieldValue::parse(field, &value),
            review_state: parse_state(&row.try_get::<String, _>("review_state")?)?,
            retracted: row
                .try_get::<Option<DateTime<Utc>>, _>("retracted_at")?
                .is_some(),
            created_at: row.try_get("created_at")?,
        };
        match field {
            ProvenanceField::Title => groups.titles.push(candidate),
            ProvenanceField::Year => groups.years.push(candidate),
            _ => groups.authors.push(candidate),
        }
    }

    Ok(groups)
}

/// Recompute and write a Work's selected title, year and authors.
///
/// Locks the Work row. A field with claim rows but none eligible is written as
/// NULL and `unknown`; a field with no claim rows is left untouched (keeping
/// fixture values). `updated_at` moves only when something changed.
pub(crate) async fn reselect_in(
    connection: &mut PgConnection,
    work: WorkId,
) -> Result<Reselection> {
    let current = sqlx::query(
        "select display_title, title_state, title_key, display_year, year_state, \
                display_authors, authors_state \
         from graph.work where id = $1 for update",
    )
    .bind(work.as_uuid())
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(DbError::not_found)?;
    let current_title: Option<String> = current.try_get("display_title")?;
    let current_title_state: FieldState =
        parse_state(&current.try_get::<String, _>("title_state")?)?;
    let current_key: Option<String> = current.try_get("title_key")?;
    let current_year: Option<i32> = current.try_get("display_year")?;
    let current_year_state: FieldState = parse_state(&current.try_get::<String, _>("year_state")?)?;
    let current_authors: Option<Vec<String>> = current.try_get("display_authors")?;
    let current_authors_state: FieldState =
        parse_state(&current.try_get::<String, _>("authors_state")?)?;

    let ClaimGroups {
        titles,
        years,
        authors,
    } = load_claim_groups(connection, work).await?;

    let (mut title, mut title_state, mut key) = (
        current_title.clone(),
        current_title_state,
        current_key.clone(),
    );
    if !titles.is_empty() {
        let selection = select_field(ProvenanceField::Title, &titles);
        title = match selection.value {
            Some(FieldValue::Title(text)) => Some(text),
            _ => None,
        };
        title_state = selection.state;
        key = title
            .as_deref()
            .map(title_key)
            .filter(|computed| !computed.is_empty());
    }
    let (mut year, mut year_state) = (current_year, current_year_state);
    if !years.is_empty() {
        let selection = select_field(ProvenanceField::Year, &years);
        year = match selection.value {
            Some(FieldValue::Year(value)) => Some(value),
            _ => None,
        };
        year_state = selection.state;
    }
    let (mut names, mut authors_state) = (current_authors.clone(), current_authors_state);
    if !authors.is_empty() {
        let selection = select_field(ProvenanceField::Authors, &authors);
        names = match selection.value {
            Some(FieldValue::Authors(list)) => Some(list),
            _ => None,
        };
        authors_state = selection.state;
    }

    let reselection = Reselection {
        title_changed: title != current_title || title_state != current_title_state,
        year_changed: year != current_year || year_state != current_year_state,
        authors_changed: names != current_authors || authors_state != current_authors_state,
        title_became_known: title.is_some()
            && current_title.is_none()
            && !matches!(
                current_title_state,
                FieldState::Known | FieldState::Conflicting
            ),
    };
    if !reselection.changed() {
        return Ok(reselection);
    }
    sqlx::query(
        "update graph.work set display_title = $2, title_state = $3, title_key = $4, \
             display_year = $5, year_state = $6, display_authors = $7, authors_state = $8, \
             updated_at = now() \
         where id = $1",
    )
    .bind(work.as_uuid())
    .bind(title)
    .bind(title_state.as_str())
    .bind(key)
    .bind(year)
    .bind(year_state.as_str())
    .bind(names)
    .bind(authors_state.as_str())
    .execute(&mut *connection)
    .await?;
    Ok(reselection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::work::WorkRepository;
    use lysilogy_core::provenance::PROVENANCE_FIELDS;

    macro_rules! database {
        () => {{
            let _guard = test_support::lock().await;
            let Some(pool) = test_support::pool().await else {
                eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
                return;
            };
            (_guard, pool)
        }};
    }

    fn marker() -> String {
        Uuid::new_v4().simple().to_string()
    }

    fn title(text: &str) -> FieldValue {
        FieldValue::Title(text.to_owned())
    }

    fn authors(names: &[&str]) -> FieldValue {
        FieldValue::Authors(names.iter().map(|name| (*name).to_owned()).collect())
    }

    fn claim(
        work: WorkId,
        value: &FieldValue,
        origin: ClaimOrigin,
        confidence: ClaimConfidence,
    ) -> NewClaim<'_> {
        NewClaim {
            work_id: work,
            field: value.field(),
            value,
            origin,
            extractor: "test",
            confidence,
            source_record_id: None,
            entry_id: None,
            locator: None,
        }
    }

    async fn stored_work(pool: &PgPool, work: WorkId) -> lysilogy_core::Work {
        WorkRepository::new(pool.clone())
            .get(work)
            .await
            .expect("get")
            .expect("present")
    }

    async fn claim_count(pool: &PgPool, work: WorkId) -> i64 {
        sqlx::query_scalar("select count(*) from graph.field_claim where work_id = $1")
            .bind(work.as_uuid())
            .fetch_one(pool)
            .await
            .expect("count")
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn should_apply_migration_backfill_when_legacy_claims_exist() {
        let (_guard, pool) = database!();
        let sql = include_str!("../migrations/0009_field_provenance.sql");
        let start = sql.find("-- Legacy rows").expect("legacy block");
        let end = sql.find("-- New writers").expect("end of legacy block");
        let statements: Vec<&str> = sql[start..end]
            .split(';')
            .map(str::trim)
            .filter(|statement| statement.contains("UPDATE"))
            .collect();
        assert_eq!(statements.len(), 3);

        let work = test_support::new_work(&pool).await;
        let version = test_support::new_version(&pool).await;
        let entries = test_support::new_entries(&pool, version, 1).await;
        let bibliography_record: Uuid = sqlx::query_scalar(
            "select source_record_id from source.bibliography_entry where id = $1",
        )
        .bind(entries[0].as_uuid())
        .fetch_one(&pool)
        .await
        .expect("bibliography record");
        let records = crate::source_record::SourceRecordRepository::new(pool.clone());
        let openalex_record = records
            .upsert(
                "openalex_work_summary",
                &format!("legacy-{}", marker()),
                lysilogy_core::Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("record");

        // The column defaults the migration used before dropping them.
        let mut transaction = pool.begin().await.expect("begin");
        sqlx::query(
            "insert into graph.entry_resolution (entry_id, candidate_work_id, status, method) \
             values ($1, $2, 'accepted', 'identifier')",
        )
        .bind(entries[0].as_uuid())
        .bind(work.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("resolution");
        for (source, label) in [
            (Some(openalex_record.as_uuid()), "openalex"),
            (Some(bibliography_record), "bibliography"),
            (None, "catalog"),
        ] {
            sqlx::query(
                "insert into graph.field_claim \
                     (work_id, field, value, source_record_id, locator, origin, extractor, confidence) \
                 values ($1, 'title', to_jsonb($2::text), $3, null, \
                         'library_metadata', 'legacy', 'heuristic')",
            )
            .bind(work.as_uuid())
            .bind(label)
            .bind(source)
            .execute(&mut *transaction)
            .await
            .expect("legacy claim");
        }
        for statement in statements {
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .expect("backfill");
        }
        let rows: Vec<(String, String, String, String, Option<Uuid>)> = sqlx::query_as(
            "select value #>> '{}', origin, extractor, confidence, entry_id \
             from graph.field_claim where work_id = $1 order by value #>> '{}'",
        )
        .bind(work.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .expect("rows");
        transaction.rollback().await.expect("rollback");

        let expected_entry = Some(entries[0].as_uuid());
        let by_label = |label: &str| rows.iter().find(|row| row.0 == label).expect(label);
        assert_eq!(
            by_label("openalex"),
            &(
                "openalex".into(),
                "openalex".into(),
                "openalex".into(),
                "explicit".into(),
                None
            )
        );
        assert_eq!(
            by_label("bibliography"),
            &(
                "bibliography".into(),
                "citing_bibliography".into(),
                "bib-v1".into(),
                "heuristic".into(),
                expected_entry
            )
        );
        assert_eq!(
            by_label("catalog"),
            &(
                "catalog".into(),
                "library_metadata".into(),
                "catalog-metadata".into(),
                "heuristic".into(),
                None
            )
        );
    }

    #[tokio::test]
    async fn should_select_highest_trust_claim_when_recording_claims() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let tag = marker();
        let openalex = title(&format!("OpenAlex {tag}"));
        let citing = title(&format!("Citing {tag}"));
        let publisher = title(&format!("Publisher {tag}"));

        let first = claims
            .record(&[claim(
                work,
                &openalex,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )])
            .await
            .expect("record");
        assert_eq!(first.inserted, 1);
        assert!(first.reselected[&work].title_changed);
        assert!(first.reselected[&work].title_became_known);
        assert_eq!(
            stored_work(&pool, work).await.display_title.as_deref(),
            Some(format!("OpenAlex {tag}").as_str())
        );

        let lower = claims
            .record(&[claim(
                work,
                &citing,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
            )])
            .await
            .expect("record");
        assert!(!lower.reselected[&work].changed());
        let after_lower = stored_work(&pool, work).await;
        assert_eq!(
            after_lower.display_title.as_deref(),
            Some(format!("OpenAlex {tag}").as_str())
        );
        assert_eq!(after_lower.title_state, FieldState::Known);

        let higher = claims
            .record(&[claim(
                work,
                &publisher,
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
            )])
            .await
            .expect("record");
        assert!(higher.reselected[&work].title_changed);
        assert!(!higher.reselected[&work].title_became_known);
        let stored = stored_work(&pool, work).await;
        assert_eq!(
            stored.display_title.as_deref(),
            Some(format!("Publisher {tag}").as_str())
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "select title_key from graph.work where id = $1"
            )
            .bind(work.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("key"),
            Some(title_key(&format!("Publisher {tag}")))
        );
    }

    #[tokio::test]
    async fn should_mark_title_conflicting_only_when_winning_tier_disagrees() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let tag = marker();
        let one = title(&format!("One {tag}"));
        let two = title(&format!("Two {tag}"));
        let upper = title(&format!("ONE {tag}"));
        claims
            .record(&[
                claim(work, &one, ClaimOrigin::Openalex, ClaimConfidence::Explicit),
                claim(
                    work,
                    &upper,
                    ClaimOrigin::Openalex,
                    ClaimConfidence::Explicit,
                ),
                claim(
                    work,
                    &two,
                    ClaimOrigin::CitingBibliography,
                    ClaimConfidence::Heuristic,
                ),
            ])
            .await
            .expect("record");
        let agreeing = stored_work(&pool, work).await;
        assert_eq!(agreeing.title_state, FieldState::Known);

        let other = title(&format!("Other {tag}"));
        claims
            .record(&[claim(
                work,
                &other,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )])
            .await
            .expect("record");
        let conflicting = stored_work(&pool, work).await;
        assert_eq!(conflicting.title_state, FieldState::Conflicting);
        assert!(conflicting.display_title.is_some());
    }

    #[tokio::test]
    async fn should_write_display_authors_when_author_claims_recorded() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let truncated = authors(&["Paul Christiano et al."]);
        let full = authors(&["Paul Christiano", "Jan Leike"]);

        let outcome = claims
            .record(&[claim(
                work,
                &truncated,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
            )])
            .await
            .expect("record");
        assert!(outcome.reselected[&work].authors_changed);
        let stored = stored_work(&pool, work).await;
        assert_eq!(
            stored.display_authors,
            Some(vec!["Paul Christiano et al.".to_owned()])
        );
        assert_eq!(stored.authors_state, FieldState::Known);

        claims
            .record(&[claim(
                work,
                &full,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
            )])
            .await
            .expect("record");
        let stored = stored_work(&pool, work).await;
        assert_eq!(
            stored.display_authors,
            Some(vec!["Paul Christiano".to_owned(), "Jan Leike".to_owned()])
        );
        assert_eq!(stored.authors_state, FieldState::Known);
    }

    #[tokio::test]
    async fn should_not_duplicate_claim_when_recorded_twice() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let same = title("Same Title");
        let other = title("Other Title");
        let record = test_support::new_record(&pool).await;
        let mut sourced = claim(
            work,
            &same,
            ClaimOrigin::Openalex,
            ClaimConfidence::Explicit,
        );

        let first = claims.record(&[sourced]).await.expect("first").inserted;
        let second = claims.record(&[sourced]).await.expect("second");
        sourced.source_record_id = Some(record);
        let other_source = claims
            .record(&[sourced])
            .await
            .expect("other source")
            .inserted;
        let different = claims
            .record(&[claim(
                work,
                &other,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )])
            .await
            .expect("other")
            .inserted;
        assert_eq!(
            (first, second.inserted, other_source, different),
            (1, 0, 1, 1)
        );
        assert!(second.reselected.is_empty(), "no new row means no reselect");
        assert_eq!(claim_count(&pool, work).await, 3);
    }

    #[tokio::test]
    async fn should_mask_any_field_when_override_set_and_restore_when_cleared() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let tag = marker();
        let tiered = [
            title(&format!("Tiered {tag}")),
            FieldValue::Year(2001),
            authors(&["Tier Author"]),
            FieldValue::Venue(format!("Tier venue {tag}")),
            FieldValue::Doi(format!("10.9999/tier-{tag}")),
        ];
        let mine = [
            title(&format!("Mine {tag}")),
            FieldValue::Year(1999),
            authors(&["My Author", "Second Author"]),
            FieldValue::Venue(format!("My venue {tag}")),
            FieldValue::Doi(format!("10.9999/mine-{tag}")),
        ];
        let batch: Vec<NewClaim<'_>> = tiered
            .iter()
            .map(|value| {
                claim(
                    work,
                    value,
                    ClaimOrigin::Publisher,
                    ClaimConfidence::Explicit,
                )
            })
            .collect();
        claims.record(&batch).await.expect("record");

        let selected = |field: ProvenanceField, stored: &[StoredClaim]| {
            let candidates: Vec<ClaimCandidate> = stored
                .iter()
                .filter(|row| row.field == field)
                .map(|row| ClaimCandidate {
                    id: row.id,
                    origin: row.origin,
                    confidence: row.confidence,
                    value: FieldValue::parse(field, &row.value),
                    review_state: row.review_state,
                    retracted: row.retracted_at.is_some(),
                    created_at: row.created_at,
                })
                .collect();
            select_field(field, &candidates).value
        };

        for value in &mine {
            let stored = claims
                .set_override(work, value.field(), value)
                .await
                .expect("override");
            assert_eq!(stored.origin, ClaimOrigin::User);
            assert_eq!(stored.extractor, "curator");
            assert_eq!(stored.review_state, ReviewState::Accepted);
        }
        let masked = stored_work(&pool, work).await;
        assert_eq!(
            masked.display_title.as_deref(),
            Some(format!("Mine {tag}").as_str())
        );
        assert_eq!(masked.display_year, Some(1999));
        assert_eq!(
            masked.display_authors,
            Some(vec!["My Author".into(), "Second Author".into()])
        );
        let all = claims.claims_for(work).await.expect("claims");
        for (field, value) in PROVENANCE_FIELDS.iter().zip(&mine) {
            assert_eq!(selected(*field, &all).as_ref(), Some(value), "{field}");
        }

        for field in PROVENANCE_FIELDS {
            assert!(claims.clear_override(work, field).await.expect("clear"));
        }
        let restored = stored_work(&pool, work).await;
        assert_eq!(
            restored.display_title.as_deref(),
            Some(format!("Tiered {tag}").as_str())
        );
        assert_eq!(restored.display_year, Some(2001));
        assert_eq!(restored.display_authors, Some(vec!["Tier Author".into()]));
        let all = claims.claims_for(work).await.expect("claims");
        for (field, value) in PROVENANCE_FIELDS.iter().zip(&tiered) {
            assert_eq!(selected(*field, &all).as_ref(), Some(value), "{field}");
        }
    }

    #[tokio::test]
    async fn should_keep_one_active_override_when_set_twice() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let first = title("First override");
        let second = title("Second override");
        let first_claim = claims
            .set_override(work, ProvenanceField::Title, &first)
            .await
            .expect("first");
        let second_claim = claims
            .set_override(work, ProvenanceField::Title, &second)
            .await
            .expect("second");
        assert_ne!(first_claim.id, second_claim.id);

        let all = claims.claims_for(work).await.expect("claims");
        let active: Vec<_> = all
            .iter()
            .filter(|row| row.retracted_at.is_none())
            .collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, second_claim.id);
        assert_eq!(all.len(), 2, "history is kept");
        assert_eq!(
            stored_work(&pool, work).await.display_title.as_deref(),
            Some("Second override")
        );

        let duplicate = sqlx::query(
            "insert into graph.field_claim (work_id, field, value, origin, extractor, confidence) \
             values ($1, 'title', '\"x\"', 'user', 'curator', 'explicit')",
        )
        .bind(work.as_uuid())
        .execute(&pool)
        .await;
        assert!(
            duplicate.is_err(),
            "the unique index allows one active override"
        );
    }

    #[tokio::test]
    async fn should_return_false_when_clearing_without_override() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let before = stored_work(&pool, work).await;
        assert!(
            !claims
                .clear_override(work, ProvenanceField::Year)
                .await
                .expect("clear")
        );
        assert_eq!(stored_work(&pool, work).await, before);
    }

    #[tokio::test]
    async fn should_exclude_rejected_claim_when_review_state_set() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let publisher = title("Publisher title");
        let openalex = title("OpenAlex title");
        claims
            .record(&[
                claim(
                    work,
                    &publisher,
                    ClaimOrigin::Publisher,
                    ClaimConfidence::Explicit,
                ),
                claim(
                    work,
                    &openalex,
                    ClaimOrigin::Openalex,
                    ClaimConfidence::Explicit,
                ),
            ])
            .await
            .expect("record");
        assert_eq!(
            stored_work(&pool, work).await.display_title.as_deref(),
            Some("Publisher title")
        );
        let publisher_claim = claims
            .claims_for(work)
            .await
            .expect("claims")
            .into_iter()
            .find(|row| row.origin == ClaimOrigin::Publisher)
            .expect("publisher claim");

        let rejected = claims
            .set_review_state(publisher_claim.id, ReviewState::Rejected)
            .await
            .expect("reject")
            .expect("present");
        assert_eq!(rejected.review_state, ReviewState::Rejected);
        assert_eq!(
            stored_work(&pool, work).await.display_title.as_deref(),
            Some("OpenAlex title")
        );

        claims
            .set_review_state(publisher_claim.id, ReviewState::Unreviewed)
            .await
            .expect("reset");
        assert_eq!(
            stored_work(&pool, work).await.display_title.as_deref(),
            Some("Publisher title")
        );
        assert!(
            claims
                .set_review_state(FieldClaimId::new(), ReviewState::Accepted)
                .await
                .expect("missing")
                .is_none()
        );
    }

    #[tokio::test]
    async fn should_refuse_review_of_user_claim() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let mine = title("Mine");
        let stored = claims
            .set_override(work, ProvenanceField::Title, &mine)
            .await
            .expect("override");
        let error = claims
            .set_review_state(stored.id, ReviewState::Rejected)
            .await
            .expect_err("refused");
        assert!(matches!(error, DbError::Conflict(code) if code == "override_not_reviewable"));
        assert_eq!(
            stored_work(&pool, work).await.display_title.as_deref(),
            Some("Mine")
        );
    }

    #[tokio::test]
    async fn should_leave_fixture_title_when_work_has_no_claims() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let works = WorkRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        works
            .set_display_title(work, "Fixture title")
            .await
            .expect("title");
        works.set_display_year(work, 2010).await.expect("year");
        let before = stored_work(&pool, work).await;

        let reselection = claims.reselect(work).await.expect("reselect");
        assert_eq!(reselection, Reselection::default());
        assert_eq!(
            stored_work(&pool, work).await,
            before,
            "updated_at is untouched too"
        );
    }

    #[tokio::test]
    async fn should_null_display_title_when_only_claim_is_rejected() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let only = title("Only title");
        claims
            .record(&[claim(
                work,
                &only,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )])
            .await
            .expect("record");
        let stored = claims.claims_for(work).await.expect("claims").remove(0);
        claims
            .set_review_state(stored.id, ReviewState::Rejected)
            .await
            .expect("reject");
        let work_row = stored_work(&pool, work).await;
        assert_eq!(work_row.display_title, None);
        assert_eq!(work_row.title_state, FieldState::Unknown);
        let key: Option<String> =
            sqlx::query_scalar("select title_key from graph.work where id = $1")
                .bind(work.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("key");
        assert_eq!(key, None);
    }

    #[tokio::test]
    async fn should_reselect_survivor_and_keep_its_override_when_merging_works() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let works = WorkRepository::new(pool.clone());
        let stub = test_support::new_work(&pool).await;
        let survivor = test_support::new_work(&pool).await;
        let tag = marker();
        let stub_openalex = title(&format!("Stub OpenAlex {tag}"));
        let survivor_citing = title(&format!("Survivor citing {tag}"));
        claims
            .record(&[
                claim(
                    stub,
                    &stub_openalex,
                    ClaimOrigin::Openalex,
                    ClaimConfidence::Explicit,
                ),
                claim(
                    survivor,
                    &survivor_citing,
                    ClaimOrigin::CitingBibliography,
                    ClaimConfidence::Heuristic,
                ),
            ])
            .await
            .expect("record");
        let stub_year = FieldValue::Year(2003);
        claims
            .set_override(stub, ProvenanceField::Year, &stub_year)
            .await
            .expect("stub year");
        let stub_title = title("Stub override title");
        let survivor_title = title("Survivor override title");
        claims
            .set_override(stub, ProvenanceField::Title, &stub_title)
            .await
            .expect("stub title");
        claims
            .set_override(survivor, ProvenanceField::Title, &survivor_title)
            .await
            .expect("survivor title");

        works
            .merge_work(stub, survivor, "same paper", "test")
            .await
            .expect("merge");

        let merged = stored_work(&pool, survivor).await;
        assert_eq!(
            merged.display_title.as_deref(),
            Some("Survivor override title")
        );
        assert_eq!(
            merged.display_year,
            Some(2003),
            "the stub's year override moves over"
        );
        let all = claims.claims_for(survivor).await.expect("claims");
        let active_title_overrides = all
            .iter()
            .filter(|row| {
                row.field == ProvenanceField::Title
                    && row.origin == ClaimOrigin::User
                    && row.retracted_at.is_none()
            })
            .count();
        assert_eq!(active_title_overrides, 1);
        assert!(
            all.iter()
                .any(|row| row.value == serde_json::json!(format!("Stub OpenAlex {tag}")))
        );
    }

    #[tokio::test]
    async fn should_reselect_every_claimed_work_when_reselecting_all() {
        let (_guard, pool) = database!();
        let claims = ClaimRepository::new(pool.clone());
        let first = test_support::new_work(&pool).await;
        let second = test_support::new_work(&pool).await;
        let unclaimed = test_support::new_work(&pool).await;
        for (work, text) in [(first, "First legacy"), (second, "Second legacy")] {
            // A legacy-shaped row written without selection.
            sqlx::query(
                "insert into graph.field_claim \
                     (work_id, field, value, origin, extractor, confidence) \
                 values ($1, 'title', to_jsonb($2::text), 'openalex', 'openalex', 'explicit')",
            )
            .bind(work.as_uuid())
            .bind(text)
            .execute(&pool)
            .await
            .expect("insert");
        }
        assert_eq!(stored_work(&pool, first).await.display_title, None);

        let reselected = claims.reselect_all().await.expect("reselect all");
        assert!(reselected >= 2);
        assert_eq!(
            stored_work(&pool, first).await.display_title.as_deref(),
            Some("First legacy")
        );
        assert_eq!(
            stored_work(&pool, second).await.display_title.as_deref(),
            Some("Second legacy")
        );
        assert_eq!(
            stored_work(&pool, unclaimed).await.title_state,
            FieldState::Unknown
        );
    }
}
