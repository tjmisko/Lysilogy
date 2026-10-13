//! Persistence for reading ramps: ordered items, the citation path that led to each, and
//! per-item reading status and notes. Owned by one workspace.

use chrono::{DateTime, Utc};
use lysilogy_core::{
    RAMP_ITEMS_MAX, RAMP_PATH_MAX, RampError, RampId, RampItemStatus, RampTitle, WorkId,
    WorkspaceId,
};
use serde::Serialize;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

const RAMP_COLUMNS: &str =
    "r.id, r.workspace_id, r.seed_work_id, r.title, r.created_at, r.updated_at";
const ITEM_COLUMNS: &str = "i.ramp_id, i.work_id, i.position, i.via_work_id, i.path_work_ids, \
    i.status, i.note, i.added_at, i.updated_at, i.status_changed_at";

/// A reading ramp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Ramp {
    pub id: RampId,
    pub workspace_id: WorkspaceId,
    pub seed_work_id: WorkId,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A ramp with its seed title and item counts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RampSummary {
    #[serde(flatten)]
    pub ramp: Ramp,
    pub seed_title: Option<String>,
    pub item_count: i64,
    pub unread: i64,
    pub reading: i64,
    pub read: i64,
}

/// One Work on a ramp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RampItem {
    pub ramp_id: RampId,
    pub work_id: WorkId,
    pub position: i32,
    pub via_work_id: Option<WorkId>,
    /// Seed or root through this Work, inclusive; the last element is `work_id`.
    pub path: Vec<WorkId>,
    pub status: RampItemStatus,
    pub note: Option<String>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status_changed_at: Option<DateTime<Utc>>,
}

/// A Work to add to a ramp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRampItem {
    pub work_id: WorkId,
    /// The ramp item this Work was reached from, which must cite it.
    pub via: Option<WorkId>,
    pub note: Option<String>,
}

/// A partial change to an item. `note: Some(None)` clears the note.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RampItemPatch {
    pub status: Option<RampItemStatus>,
    pub note: Option<Option<String>>,
}

/// What removing an item did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    Removed,
    NotFound,
    /// The seed item stays until the ramp is deleted.
    Seed,
}

fn conflict(error: RampError) -> DbError {
    DbError::Conflict(error.code().to_owned())
}

/// Reads and writes ramps and their items.
#[derive(Clone)]
pub struct RampRepository {
    pool: PgPool,
}

impl RampRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a ramp and its seed item (position 0, path `[seed]`) in one transaction.
    pub async fn create(
        &self,
        workspace: WorkspaceId,
        seed: WorkId,
        title: &RampTitle,
    ) -> Result<Ramp> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "insert into reader.ramp as r (workspace_id, seed_work_id, title) \
             values ($1, $2, $3) \
             returning r.id, r.workspace_id, r.seed_work_id, r.title, r.created_at, r.updated_at",
        )
        .bind(workspace.as_uuid())
        .bind(seed.as_uuid())
        .bind(title.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let ramp = ramp_from_row(&row)?;
        sqlx::query(
            "insert into reader.ramp_item (ramp_id, work_id, position, path_work_ids) \
             values ($1, $2, 0, $3)",
        )
        .bind(ramp.id.as_uuid())
        .bind(seed.as_uuid())
        .bind(vec![seed.as_uuid()])
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ramp)
    }

    /// The workspace's ramps, most recently updated first, optionally only those seeded
    /// at one Work.
    pub async fn list(
        &self,
        workspace: WorkspaceId,
        seed: Option<WorkId>,
    ) -> Result<Vec<RampSummary>> {
        let rows = sqlx::query(&format!(
            "select {RAMP_COLUMNS}, w.display_title as seed_title, \
                    count(i.work_id) as item_count, \
                    count(i.work_id) filter (where i.status = 'unread') as unread, \
                    count(i.work_id) filter (where i.status = 'reading') as reading, \
                    count(i.work_id) filter (where i.status = 'read') as read \
             from reader.ramp r \
             join graph.work w on w.id = r.seed_work_id \
             left join reader.ramp_item i on i.ramp_id = r.id \
             where r.workspace_id = $1 and ($2::uuid is null or r.seed_work_id = $2) \
             group by r.id, w.display_title \
             order by r.updated_at desc, r.id"
        ))
        .bind(workspace.as_uuid())
        .bind(seed.map(WorkId::as_uuid))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(RampSummary {
                    ramp: ramp_from_row(row)?,
                    seed_title: row.try_get("seed_title")?,
                    item_count: row.try_get("item_count")?,
                    unread: row.try_get("unread")?,
                    reading: row.try_get("reading")?,
                    read: row.try_get("read")?,
                })
            })
            .collect()
    }

    /// One ramp of the workspace.
    pub async fn get(&self, workspace: WorkspaceId, ramp: RampId) -> Result<Option<Ramp>> {
        let row = sqlx::query(&format!(
            "select {RAMP_COLUMNS} from reader.ramp r where r.workspace_id = $1 and r.id = $2"
        ))
        .bind(workspace.as_uuid())
        .bind(ramp.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(ramp_from_row).transpose()
    }

    /// A ramp's items by position.
    pub async fn items(&self, ramp: RampId) -> Result<Vec<RampItem>> {
        let rows = sqlx::query(&format!(
            "select {ITEM_COLUMNS} from reader.ramp_item i where i.ramp_id = $1 \
             order by i.position, i.work_id"
        ))
        .bind(ramp.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// One item of a ramp.
    pub async fn item(&self, ramp: RampId, work: WorkId) -> Result<Option<RampItem>> {
        let row = sqlx::query(&format!(
            "select {ITEM_COLUMNS} from reader.ramp_item i where i.ramp_id = $1 and i.work_id = $2"
        ))
        .bind(ramp.as_uuid())
        .bind(work.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(item_from_row).transpose()
    }

    /// Retitle a ramp; `None` when it is not in the workspace.
    pub async fn rename(
        &self,
        workspace: WorkspaceId,
        ramp: RampId,
        title: &RampTitle,
    ) -> Result<Option<Ramp>> {
        let row = sqlx::query(
            "update reader.ramp as r set title = $3, updated_at = now() \
             where r.workspace_id = $1 and r.id = $2 \
             returning r.id, r.workspace_id, r.seed_work_id, r.title, r.created_at, r.updated_at",
        )
        .bind(workspace.as_uuid())
        .bind(ramp.as_uuid())
        .bind(title.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(ramp_from_row).transpose()
    }

    /// Delete a ramp and, by cascade, its items. Whether a ramp was removed.
    pub async fn delete(&self, workspace: WorkspaceId, ramp: RampId) -> Result<bool> {
        let result = sqlx::query("delete from reader.ramp where workspace_id = $1 and id = $2")
            .bind(workspace.as_uuid())
            .bind(ramp.as_uuid())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Append a Work to the ramp. Under a lock on the ramp row this checks that the Work is
    /// new, that `via` is an item citing it, that the path stays within
    /// [`RAMP_PATH_MAX`], and that the ramp has room, refusing with
    /// [`DbError::Conflict`] carrying the [`RampError::code`]. A missing ramp is
    /// [`DbError::not_found`].
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub async fn add_item(&self, ramp: RampId, new: NewRampItem) -> Result<RampItem> {
        let mut transaction = self.pool.begin().await?;
        let locked: Option<Uuid> =
            sqlx::query_scalar("select id from reader.ramp where id = $1 for update")
                .bind(ramp.as_uuid())
                .fetch_optional(&mut *transaction)
                .await?;
        if locked.is_none() {
            return Err(DbError::not_found());
        }

        let exists: bool = sqlx::query_scalar(
            "select exists (select 1 from reader.ramp_item where ramp_id = $1 and work_id = $2)",
        )
        .bind(ramp.as_uuid())
        .bind(new.work_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await?;
        if exists {
            return Err(conflict(RampError::ItemExists));
        }

        let path = match new.via {
            None => vec![new.work_id.as_uuid()],
            Some(via) => {
                let via_path: Option<Vec<Uuid>> = sqlx::query_scalar(
                    "select path_work_ids from reader.ramp_item \
                     where ramp_id = $1 and work_id = $2",
                )
                .bind(ramp.as_uuid())
                .bind(via.as_uuid())
                .fetch_optional(&mut *transaction)
                .await?;
                let Some(mut via_path) = via_path else {
                    return Err(conflict(RampError::ViaNotInRamp));
                };
                let cites: bool = sqlx::query_scalar(
                    "select exists (select 1 from graph.citation_assertion \
                     where citing_work_id = $1 and cited_work_id = $2 \
                       and status in ('asserted', 'accepted'))",
                )
                .bind(via.as_uuid())
                .bind(new.work_id.as_uuid())
                .fetch_one(&mut *transaction)
                .await?;
                if !cites {
                    return Err(conflict(RampError::NoCitationEdge));
                }
                if via_path.len() + 1 > RAMP_PATH_MAX {
                    return Err(conflict(RampError::TooDeep));
                }
                via_path.push(new.work_id.as_uuid());
                via_path
            }
        };

        let (count, next_position): (i64, i32) = sqlx::query_as(
            "select count(*), coalesce(max(position), -1) + 1 \
             from reader.ramp_item where ramp_id = $1",
        )
        .bind(ramp.as_uuid())
        .fetch_one(&mut *transaction)
        .await?;
        if count as usize >= RAMP_ITEMS_MAX {
            return Err(conflict(RampError::Full));
        }

        let row = sqlx::query(&format!(
            "insert into reader.ramp_item as i \
                 (ramp_id, work_id, position, via_work_id, path_work_ids, note) \
             values ($1, $2, $3, $4, $5, $6) returning {ITEM_COLUMNS}"
        ))
        .bind(ramp.as_uuid())
        .bind(new.work_id.as_uuid())
        .bind(next_position)
        .bind(new.via.map(WorkId::as_uuid))
        .bind(path)
        .bind(new.note.filter(|note| !note.is_empty()))
        .fetch_one(&mut *transaction)
        .await?;
        touch_ramp(&mut transaction, ramp).await?;
        transaction.commit().await?;
        item_from_row(&row)
    }

    /// Change an item's status and/or note; `None` when the item does not exist.
    /// `status_changed_at` moves only when the status really changes.
    pub async fn update_item(
        &self,
        ramp: RampId,
        work: WorkId,
        patch: RampItemPatch,
    ) -> Result<Option<RampItem>> {
        let (set_note, note) = patch.note.map_or((false, None), |note| {
            (true, note.filter(|text| !text.is_empty()))
        });
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&format!(
            "update reader.ramp_item as i set \
                 status = coalesce($3, i.status), \
                 status_changed_at = case when $3::text is not null and $3 <> i.status \
                     then now() else i.status_changed_at end, \
                 note = case when $4 then $5 else i.note end, \
                 updated_at = now() \
             where i.ramp_id = $1 and i.work_id = $2 returning {ITEM_COLUMNS}"
        ))
        .bind(ramp.as_uuid())
        .bind(work.as_uuid())
        .bind(patch.status.map(RampItemStatus::as_str))
        .bind(set_note)
        .bind(note)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        touch_ramp(&mut transaction, ramp).await?;
        transaction.commit().await?;
        item_from_row(&row).map(Some)
    }

    /// Remove an item, leaving a gap in positions. The seed item is refused.
    pub async fn remove_item(&self, ramp: RampId, work: WorkId) -> Result<RemoveOutcome> {
        let mut transaction = self.pool.begin().await?;
        let seed: Option<Uuid> =
            sqlx::query_scalar("select seed_work_id from reader.ramp where id = $1 for update")
                .bind(ramp.as_uuid())
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(seed) = seed else {
            return Ok(RemoveOutcome::NotFound);
        };
        if seed == work.as_uuid() {
            return Ok(RemoveOutcome::Seed);
        }
        let result =
            sqlx::query("delete from reader.ramp_item where ramp_id = $1 and work_id = $2")
                .bind(ramp.as_uuid())
                .bind(work.as_uuid())
                .execute(&mut *transaction)
                .await?;
        if result.rows_affected() == 0 {
            return Ok(RemoveOutcome::NotFound);
        }
        touch_ramp(&mut transaction, ramp).await?;
        transaction.commit().await?;
        Ok(RemoveOutcome::Removed)
    }

    /// Rewrite positions to `0..n` following `order`, in one transaction. The caller has
    /// validated the permutation; a list that is not exactly the ramp's items is refused
    /// with `Conflict("invalid_order")` and nothing changes.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    pub async fn reorder(&self, ramp: RampId, order: &[WorkId]) -> Result<()> {
        let ids: Vec<Uuid> = order.iter().map(|work| work.as_uuid()).collect();
        let mut transaction = self.pool.begin().await?;
        let locked: Option<Uuid> =
            sqlx::query_scalar("select id from reader.ramp where id = $1 for update")
                .bind(ramp.as_uuid())
                .fetch_optional(&mut *transaction)
                .await?;
        if locked.is_none() {
            return Err(DbError::not_found());
        }
        let total: i64 =
            sqlx::query_scalar("select count(*) from reader.ramp_item where ramp_id = $1")
                .bind(ramp.as_uuid())
                .fetch_one(&mut *transaction)
                .await?;
        let result = sqlx::query(
            "update reader.ramp_item i set position = (o.ordinal - 1)::int, updated_at = now() \
             from unnest($2::uuid[]) with ordinality as o(work_id, ordinal) \
             where i.ramp_id = $1 and i.work_id = o.work_id",
        )
        .bind(ramp.as_uuid())
        .bind(&ids)
        .execute(&mut *transaction)
        .await?;
        let distinct = ids.iter().collect::<std::collections::HashSet<_>>().len();
        if result.rows_affected() as usize != ids.len()
            || ids.len() != distinct
            || ids.len() as i64 != total
        {
            return Err(conflict(RampError::InvalidOrder));
        }
        touch_ramp(&mut transaction, ramp).await?;
        transaction.commit().await?;
        Ok(())
    }
}

async fn touch_ramp(transaction: &mut sqlx::PgConnection, ramp: RampId) -> Result<()> {
    sqlx::query("update reader.ramp set updated_at = now() where id = $1")
        .bind(ramp.as_uuid())
        .execute(&mut *transaction)
        .await?;
    Ok(())
}

fn ramp_from_row(row: &PgRow) -> Result<Ramp> {
    Ok(Ramp {
        id: RampId::from_uuid(row.try_get("id")?),
        workspace_id: WorkspaceId::from_uuid(row.try_get("workspace_id")?),
        seed_work_id: WorkId::from_uuid(row.try_get("seed_work_id")?),
        title: row.try_get("title")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn item_from_row(row: &PgRow) -> Result<RampItem> {
    let status: String = row.try_get("status")?;
    let path: Vec<Uuid> = row.try_get("path_work_ids")?;
    Ok(RampItem {
        ramp_id: RampId::from_uuid(row.try_get("ramp_id")?),
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        position: row.try_get("position")?,
        via_work_id: row
            .try_get::<Option<Uuid>, _>("via_work_id")?
            .map(WorkId::from_uuid),
        path: path.into_iter().map(WorkId::from_uuid).collect(),
        status: status.parse()?,
        note: row.try_get("note")?,
        added_at: row.try_get("added_at")?,
        updated_at: row.try_get("updated_at")?,
        status_changed_at: row.try_get("status_changed_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expansion::{ExpansionRepository, NewExpansion};
    use crate::test_support;
    use crate::work::WorkRepository;
    use lysilogy_core::{ExpansionMethod, ExpansionState};

    type Attempt = (&'static str, Vec<Uuid>, i32, &'static str, Option<String>);

    struct Fixture {
        pool: PgPool,
        repo: RampRepository,
        workspace: WorkspaceId,
    }

    async fn fixture() -> Option<Fixture> {
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return None;
        };
        let workspace = test_support::new_workspace(&pool).await;
        Some(Fixture {
            repo: RampRepository::new(pool.clone()),
            pool,
            workspace,
        })
    }

    fn title(text: &str) -> RampTitle {
        RampTitle::parse(text).expect("title")
    }

    fn add(work: WorkId, via: Option<WorkId>) -> NewRampItem {
        NewRampItem {
            work_id: work,
            via,
            note: None,
        }
    }

    fn conflict_code(result: Result<RampItem>) -> String {
        match result {
            Err(DbError::Conflict(code)) => code,
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    /// A ramp seeded at a fresh Work that cites a fresh child.
    async fn ramp_with_child(fixture: &Fixture) -> (Ramp, WorkId, WorkId) {
        let seed = test_support::new_work(&fixture.pool).await;
        let child = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, seed, child).await;
        let ramp = fixture
            .repo
            .create(fixture.workspace, seed, &title("Ramp"))
            .await
            .expect("create");
        (ramp, seed, child)
    }

    #[tokio::test]
    async fn should_insert_seed_item_when_creating_ramp() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let seed = test_support::new_work(&fixture.pool).await;
        let ramp = fixture
            .repo
            .create(fixture.workspace, seed, &title("First ramp"))
            .await
            .expect("create");
        assert_eq!(ramp.seed_work_id, seed);
        assert_eq!(ramp.title, "First ramp");
        assert_eq!(ramp.workspace_id, fixture.workspace);

        let items = fixture.repo.items(ramp.id).await.expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].work_id, seed);
        assert_eq!(items[0].position, 0);
        assert_eq!(items[0].path, vec![seed]);
        assert_eq!(items[0].via_work_id, None);
        assert_eq!(items[0].status, RampItemStatus::Unread);
        assert_eq!(items[0].note, None);
        assert_eq!(items[0].status_changed_at, None);
        let fetched = fixture
            .repo
            .get(fixture.workspace, ramp.id)
            .await
            .expect("get");
        assert_eq!(fetched, Some(ramp));
    }

    #[tokio::test]
    async fn should_list_ramps_by_update_time_with_status_counts() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (older, seed, child) = ramp_with_child(&fixture).await;
        let (newer, _, _) = ramp_with_child(&fixture).await;
        fixture
            .repo
            .add_item(older.id, add(child, Some(seed)))
            .await
            .expect("add");
        fixture
            .repo
            .update_item(
                older.id,
                child,
                RampItemPatch {
                    status: Some(RampItemStatus::Read),
                    note: None,
                },
            )
            .await
            .expect("update");
        fixture
            .repo
            .update_item(
                older.id,
                seed,
                RampItemPatch {
                    status: Some(RampItemStatus::Reading),
                    note: None,
                },
            )
            .await
            .expect("update");

        // Touching the older ramp last puts it first.
        let list = fixture
            .repo
            .list(fixture.workspace, None)
            .await
            .expect("list");
        let ids: Vec<RampId> = list.iter().map(|summary| summary.ramp.id).collect();
        assert_eq!(ids, vec![older.id, newer.id]);
        assert_eq!(list[0].item_count, 2);
        assert_eq!((list[0].unread, list[0].reading, list[0].read), (0, 1, 1));
        assert_eq!(list[1].item_count, 1);
        assert_eq!((list[1].unread, list[1].reading, list[1].read), (1, 0, 0));

        fixture
            .repo
            .rename(fixture.workspace, newer.id, &title("Renamed"))
            .await
            .expect("rename");
        let list = fixture
            .repo
            .list(fixture.workspace, None)
            .await
            .expect("list");
        assert_eq!(list[0].ramp.id, newer.id);
        assert_eq!(list[0].ramp.title, "Renamed");
    }

    #[tokio::test]
    async fn should_filter_ramps_by_seed_when_listing() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (first, first_seed, _) = ramp_with_child(&fixture).await;
        let (_second, _, _) = ramp_with_child(&fixture).await;
        let other = fixture
            .repo
            .create(fixture.workspace, first_seed, &title("Same seed"))
            .await
            .expect("second ramp on the same seed");
        let filtered = fixture
            .repo
            .list(fixture.workspace, Some(first_seed))
            .await
            .expect("list");
        let mut ids: Vec<RampId> = filtered.iter().map(|summary| summary.ramp.id).collect();
        ids.sort();
        let mut expected = vec![first.id, other.id];
        expected.sort();
        assert_eq!(ids, expected);
        let none = fixture
            .repo
            .list(fixture.workspace, Some(WorkId::new()))
            .await
            .expect("list");
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn should_build_path_from_via_when_adding_item() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        let grandchild = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, child, grandchild).await;
        let direct = test_support::new_work(&fixture.pool).await;

        let first = fixture
            .repo
            .add_item(
                ramp.id,
                NewRampItem {
                    work_id: child,
                    via: Some(seed),
                    note: Some("read me".into()),
                },
            )
            .await
            .expect("child");
        assert_eq!(first.path, vec![seed, child]);
        assert_eq!(first.via_work_id, Some(seed));
        assert_eq!(first.position, 1);
        assert_eq!(first.note.as_deref(), Some("read me"));

        let second = fixture
            .repo
            .add_item(ramp.id, add(grandchild, Some(child)))
            .await
            .expect("grandchild");
        assert_eq!(second.path, vec![seed, child, grandchild]);
        assert_eq!(second.position, 2);

        let unrooted = fixture
            .repo
            .add_item(ramp.id, add(direct, None))
            .await
            .expect("direct add");
        assert_eq!(unrooted.path, vec![direct]);
        assert_eq!(unrooted.via_work_id, None);
        assert_eq!(unrooted.position, 3);

        let empty_note = fixture
            .repo
            .add_item(
                ramp.id,
                NewRampItem {
                    work_id: WorkId::from_uuid(
                        sqlx::query_scalar("insert into graph.work (kind, visibility) values ('article', 'private') returning id")
                            .fetch_one(&fixture.pool)
                            .await
                            .expect("work"),
                    ),
                    via: None,
                    note: Some(String::new()),
                },
            )
            .await
            .expect("empty note stores as none");
        assert_eq!(empty_note.note, None);

        let missing_ramp = fixture
            .repo
            .add_item(RampId::new(), add(direct, None))
            .await;
        assert!(matches!(missing_ramp, Err(ref error) if error.is_not_found()));
    }

    #[tokio::test]
    async fn should_conflict_when_item_already_in_ramp() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        fixture
            .repo
            .add_item(ramp.id, add(child, Some(seed)))
            .await
            .expect("add");
        assert_eq!(
            conflict_code(fixture.repo.add_item(ramp.id, add(child, Some(seed))).await),
            "item_exists"
        );
        assert_eq!(
            conflict_code(fixture.repo.add_item(ramp.id, add(seed, None)).await),
            "item_exists"
        );
        assert_eq!(fixture.repo.items(ramp.id).await.expect("items").len(), 2);
    }

    #[tokio::test]
    async fn should_conflict_when_via_is_not_an_item() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, _seed, child) = ramp_with_child(&fixture).await;
        let stranger = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, stranger, child).await;
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_item(ramp.id, add(child, Some(stranger)))
                    .await
            ),
            "via_not_in_ramp"
        );
    }

    #[tokio::test]
    async fn should_conflict_when_via_does_not_cite_work() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, _child) = ramp_with_child(&fixture).await;
        let unrelated = test_support::new_work(&fixture.pool).await;
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_item(ramp.id, add(unrelated, Some(seed)))
                    .await
            ),
            "no_citation_edge"
        );
        // The edge must run from via to work, not the other way round.
        let citing_seed = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, citing_seed, seed).await;
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_item(ramp.id, add(citing_seed, Some(seed)))
                    .await
            ),
            "no_citation_edge"
        );

        let rejected = test_support::new_work(&fixture.pool).await;
        let assertion = test_support::cite(&fixture.pool, seed, rejected).await;
        for status in ["rejected", "withdrawn"] {
            sqlx::query("update graph.citation_assertion set status = $2 where id = $1")
                .bind(assertion)
                .bind(status)
                .execute(&fixture.pool)
                .await
                .expect("status");
            assert_eq!(
                conflict_code(
                    fixture
                        .repo
                        .add_item(ramp.id, add(rejected, Some(seed)))
                        .await
                ),
                "no_citation_edge"
            );
        }
        sqlx::query("update graph.citation_assertion set status = 'accepted' where id = $1")
            .bind(assertion)
            .execute(&fixture.pool)
            .await
            .expect("accept");
        let item = fixture
            .repo
            .add_item(ramp.id, add(rejected, Some(seed)))
            .await
            .expect("an accepted edge is enough");
        assert_eq!(item.path, vec![seed, rejected]);
    }

    #[tokio::test]
    async fn should_conflict_when_path_exceeds_nine_works() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let mut chain = Vec::new();
        for _ in 0..10 {
            chain.push(test_support::new_work(&fixture.pool).await);
        }
        for pair in chain.windows(2) {
            test_support::cite(&fixture.pool, pair[0], pair[1]).await;
        }
        let ramp = fixture
            .repo
            .create(fixture.workspace, chain[0], &title("Deep"))
            .await
            .expect("create");
        let mut last = None;
        for pair in chain[..9].windows(2) {
            last = Some(
                fixture
                    .repo
                    .add_item(ramp.id, add(pair[1], Some(pair[0])))
                    .await
                    .expect("within nine"),
            );
        }
        assert_eq!(last.expect("added").path.len(), 9);
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_item(ramp.id, add(chain[9], Some(chain[8])))
                    .await
            ),
            "ramp_too_deep"
        );
        // Without `via` the path is a single Work, so depth never blocks it.
        fixture
            .repo
            .add_item(ramp.id, add(chain[9], None))
            .await
            .expect("direct add");
    }

    #[tokio::test]
    async fn should_conflict_when_ramp_is_full() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        // The seed item plus 498 more make 499 of the 500 allowed.
        sqlx::query(
            "with made as (insert into graph.work (kind, visibility) \
                 select 'article', 'private' from generate_series(1, 498) returning id) \
             insert into reader.ramp_item (ramp_id, work_id, position, path_work_ids) \
             select $1, id, (row_number() over ())::int, array[id] from made",
        )
        .bind(ramp.id.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("bulk items");
        let last = fixture
            .repo
            .add_item(ramp.id, add(child, Some(seed)))
            .await
            .expect("the 500th item fits");
        assert_eq!(last.position, 499);
        let overflow = test_support::new_work(&fixture.pool).await;
        assert_eq!(
            conflict_code(fixture.repo.add_item(ramp.id, add(overflow, None)).await),
            "ramp_full"
        );
        assert_eq!(fixture.repo.items(ramp.id).await.expect("items").len(), 500);
    }

    #[tokio::test]
    async fn should_set_status_changed_at_only_when_status_changes() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        let status = |status| RampItemPatch {
            status: Some(status),
            note: None,
        };

        let note_only = fixture
            .repo
            .update_item(
                ramp.id,
                seed,
                RampItemPatch {
                    status: None,
                    note: Some(Some("hello".into())),
                },
            )
            .await
            .expect("note")
            .expect("item");
        assert_eq!(note_only.status_changed_at, None);
        assert_eq!(note_only.note.as_deref(), Some("hello"));

        let same = fixture
            .repo
            .update_item(ramp.id, seed, status(RampItemStatus::Unread))
            .await
            .expect("same")
            .expect("item");
        assert_eq!(
            same.status_changed_at, None,
            "unread to unread is no change"
        );

        let changed = fixture
            .repo
            .update_item(ramp.id, seed, status(RampItemStatus::Reading))
            .await
            .expect("change")
            .expect("item");
        let stamp = changed.status_changed_at.expect("stamped");
        assert_eq!(changed.status, RampItemStatus::Reading);
        assert_eq!(changed.note.as_deref(), Some("hello"), "note is kept");

        let repeated = fixture
            .repo
            .update_item(ramp.id, seed, status(RampItemStatus::Reading))
            .await
            .expect("repeat")
            .expect("item");
        assert_eq!(repeated.status_changed_at, Some(stamp));

        let moved = fixture
            .repo
            .update_item(ramp.id, seed, status(RampItemStatus::Read))
            .await
            .expect("move")
            .expect("item");
        assert!(moved.status_changed_at.expect("stamped") >= stamp);
        assert_eq!(moved.status, RampItemStatus::Read);

        let missing = fixture
            .repo
            .update_item(ramp.id, child, status(RampItemStatus::Read))
            .await
            .expect("missing");
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn should_clear_note_when_patch_sets_null() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, _child) = ramp_with_child(&fixture).await;
        let set = |note: Option<Option<String>>| RampItemPatch { status: None, note };
        let with_note = fixture
            .repo
            .update_item(ramp.id, seed, set(Some(Some("keep".into()))))
            .await
            .expect("set")
            .expect("item");
        assert_eq!(with_note.note.as_deref(), Some("keep"));
        let untouched = fixture
            .repo
            .update_item(ramp.id, seed, set(None))
            .await
            .expect("absent")
            .expect("item");
        assert_eq!(untouched.note.as_deref(), Some("keep"));
        let cleared = fixture
            .repo
            .update_item(ramp.id, seed, set(Some(None)))
            .await
            .expect("clear")
            .expect("item");
        assert_eq!(cleared.note, None);
        let from_empty = fixture
            .repo
            .update_item(ramp.id, seed, set(Some(Some(String::new()))))
            .await
            .expect("empty")
            .expect("item");
        assert_eq!(from_empty.note, None);
    }

    #[tokio::test]
    async fn should_refuse_to_remove_seed_item() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        assert_eq!(
            fixture.repo.remove_item(ramp.id, seed).await.expect("seed"),
            RemoveOutcome::Seed
        );
        assert_eq!(
            fixture
                .repo
                .remove_item(ramp.id, child)
                .await
                .expect("absent"),
            RemoveOutcome::NotFound
        );
        assert_eq!(
            fixture
                .repo
                .remove_item(RampId::new(), seed)
                .await
                .expect("no ramp"),
            RemoveOutcome::NotFound
        );
        fixture
            .repo
            .add_item(ramp.id, add(child, Some(seed)))
            .await
            .expect("add");
        assert_eq!(
            fixture
                .repo
                .remove_item(ramp.id, child)
                .await
                .expect("remove"),
            RemoveOutcome::Removed
        );
        assert_eq!(fixture.repo.items(ramp.id).await.expect("items").len(), 1);
    }

    #[tokio::test]
    async fn should_keep_descendant_paths_when_removing_intermediate_item() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        let grandchild = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, child, grandchild).await;
        fixture
            .repo
            .add_item(ramp.id, add(child, Some(seed)))
            .await
            .expect("child");
        fixture
            .repo
            .add_item(ramp.id, add(grandchild, Some(child)))
            .await
            .expect("grandchild");
        assert_eq!(
            fixture
                .repo
                .remove_item(ramp.id, child)
                .await
                .expect("remove"),
            RemoveOutcome::Removed
        );
        let kept = fixture
            .repo
            .item(ramp.id, grandchild)
            .await
            .expect("item")
            .expect("kept");
        assert_eq!(kept.path, vec![seed, child, grandchild]);
        assert_eq!(kept.via_work_id, Some(child));
        // The gap left behind: the next append goes after the highest position.
        let next = test_support::new_work(&fixture.pool).await;
        let added = fixture
            .repo
            .add_item(ramp.id, add(next, None))
            .await
            .expect("append");
        assert_eq!(added.position, 3);
    }

    #[tokio::test]
    async fn should_rewrite_positions_when_reordering() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        let extra = test_support::new_work(&fixture.pool).await;
        let dropped = test_support::new_work(&fixture.pool).await;
        fixture
            .repo
            .add_item(ramp.id, add(child, Some(seed)))
            .await
            .expect("child");
        fixture
            .repo
            .add_item(ramp.id, add(dropped, None))
            .await
            .expect("dropped");
        fixture
            .repo
            .add_item(ramp.id, add(extra, None))
            .await
            .expect("extra");
        fixture
            .repo
            .remove_item(ramp.id, dropped)
            .await
            .expect("remove leaves a gap");
        let positions: Vec<i32> = fixture
            .repo
            .items(ramp.id)
            .await
            .expect("items")
            .iter()
            .map(|item| item.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 3]);

        // A full reversal swaps every pair at once, which the deferred constraint allows.
        fixture
            .repo
            .reorder(ramp.id, &[extra, child, seed])
            .await
            .expect("reorder");
        let items = fixture.repo.items(ramp.id).await.expect("items");
        let order: Vec<WorkId> = items.iter().map(|item| item.work_id).collect();
        assert_eq!(order, vec![extra, child, seed]);
        let positions: Vec<i32> = items.iter().map(|item| item.position).collect();
        assert_eq!(positions, vec![0, 1, 2]);

        for bad in [
            vec![extra, child],
            vec![extra, child, seed, dropped],
            vec![extra, extra, seed],
            vec![extra, child, WorkId::new()],
            vec![],
        ] {
            let result = fixture.repo.reorder(ramp.id, &bad).await;
            assert!(
                matches!(result, Err(DbError::Conflict(ref code)) if code == "invalid_order"),
                "{bad:?} must be refused, got {result:?}"
            );
        }
        let after: Vec<WorkId> = fixture
            .repo
            .items(ramp.id)
            .await
            .expect("items")
            .iter()
            .map(|item| item.work_id)
            .collect();
        assert_eq!(after, order, "a refused order changes nothing");
        let missing = fixture.repo.reorder(RampId::new(), &[seed]).await;
        assert!(matches!(missing, Err(ref error) if error.is_not_found()));
    }

    #[tokio::test]
    async fn should_return_none_when_ramp_belongs_to_other_workspace() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, _child) = ramp_with_child(&fixture).await;
        let other = test_support::new_workspace(&fixture.pool).await;
        assert_eq!(fixture.repo.get(other, ramp.id).await.expect("get"), None);
        assert_eq!(
            fixture
                .repo
                .rename(other, ramp.id, &title("Hijacked"))
                .await
                .expect("rename"),
            None
        );
        assert!(!fixture.repo.delete(other, ramp.id).await.expect("delete"));
        assert!(
            fixture
                .repo
                .list(other, Some(seed))
                .await
                .expect("list")
                .is_empty()
        );
        let still = fixture
            .repo
            .get(fixture.workspace, ramp.id)
            .await
            .expect("get")
            .expect("ramp");
        assert_eq!(still.title, "Ramp");
    }

    #[tokio::test]
    async fn should_cascade_items_when_deleting_ramp() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, child) = ramp_with_child(&fixture).await;
        fixture
            .repo
            .add_item(ramp.id, add(child, Some(seed)))
            .await
            .expect("add");
        assert!(
            fixture
                .repo
                .delete(fixture.workspace, ramp.id)
                .await
                .expect("delete")
        );
        assert!(
            !fixture
                .repo
                .delete(fixture.workspace, ramp.id)
                .await
                .expect("again")
        );
        assert!(fixture.repo.items(ramp.id).await.expect("items").is_empty());
        let leftover: i64 =
            sqlx::query_scalar("select count(*) from reader.ramp_item where ramp_id = $1")
                .bind(ramp.id.as_uuid())
                .fetch_one(&fixture.pool)
                .await
                .expect("count");
        assert_eq!(leftover, 0);
        // The Works themselves survive.
        assert!(
            WorkRepository::new(fixture.pool.clone())
                .get(seed)
                .await
                .expect("work")
                .is_some()
        );
        // Deleting a Work cascades to the ramps seeded at it.
        let (second, second_seed, _) = ramp_with_child(&fixture).await;
        sqlx::query("delete from graph.work where id = $1")
            .bind(second_seed.as_uuid())
            .execute(&fixture.pool)
            .await
            .expect("delete work");
        assert_eq!(
            fixture
                .repo
                .get(fixture.workspace, second.id)
                .await
                .expect("get"),
            None
        );
    }

    #[tokio::test]
    async fn should_violate_check_when_path_does_not_end_with_work() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let (ramp, seed, _child) = ramp_with_child(&fixture).await;
        let work = test_support::new_work(&fixture.pool).await;
        let other = test_support::new_work(&fixture.pool).await;
        let ten: Vec<Uuid> = (0..9)
            .map(|_| Uuid::new_v4())
            .chain(std::iter::once(work.as_uuid()))
            .collect();
        let nine: Vec<Uuid> = (0..8)
            .map(|_| Uuid::new_v4())
            .chain(std::iter::once(work.as_uuid()))
            .collect();
        let attempts: Vec<Attempt> = vec![
            (
                "path ends elsewhere",
                vec![work.as_uuid(), other.as_uuid()],
                5,
                "unread",
                None,
            ),
            ("empty path", vec![], 5, "unread", None),
            ("path of ten", ten, 5, "unread", None),
            (
                "negative position",
                vec![work.as_uuid()],
                -1,
                "unread",
                None,
            ),
            ("unknown status", vec![work.as_uuid()], 5, "skimmed", None),
            (
                "empty note",
                vec![work.as_uuid()],
                5,
                "unread",
                Some(String::new()),
            ),
            (
                "long note",
                vec![work.as_uuid()],
                5,
                "unread",
                Some("x".repeat(2001)),
            ),
            ("taken position", vec![work.as_uuid()], 0, "unread", None),
        ];
        for (label, path, position, status, note) in attempts {
            let result = sqlx::query(
                "insert into reader.ramp_item \
                     (ramp_id, work_id, position, path_work_ids, status, note) \
                 values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(ramp.id.as_uuid())
            .bind(work.as_uuid())
            .bind(position)
            .bind(path)
            .bind(status)
            .bind(note)
            .execute(&fixture.pool)
            .await;
            assert!(result.is_err(), "{label} must be refused");
        }
        for (label, path, note) in [
            ("path of nine", nine, Some("x".repeat(2000))),
            ("single work", vec![work.as_uuid()], None),
        ] {
            sqlx::query("delete from reader.ramp_item where ramp_id = $1 and work_id = $2")
                .bind(ramp.id.as_uuid())
                .bind(work.as_uuid())
                .execute(&fixture.pool)
                .await
                .expect("clean");
            sqlx::query(
                "insert into reader.ramp_item (ramp_id, work_id, position, path_work_ids, note) \
                 values ($1, $2, 5, $3, $4)",
            )
            .bind(ramp.id.as_uuid())
            .bind(work.as_uuid())
            .bind(path)
            .bind(note)
            .execute(&fixture.pool)
            .await
            .unwrap_or_else(|error| panic!("{label} must be accepted: {error}"));
        }
        let bad_title = sqlx::query(
            "insert into reader.ramp (workspace_id, seed_work_id, title) values ($1, $2, ' padded ')",
        )
        .bind(fixture.workspace.as_uuid())
        .bind(seed.as_uuid())
        .execute(&fixture.pool)
        .await;
        assert!(
            bad_title.is_err(),
            "a title with edge spaces must be refused"
        );
    }

    /// Two ramps that both hold `from` and `into`/`from` in the shapes merges must handle.
    #[tokio::test]
    async fn should_repoint_items_paths_and_seed_when_merging_works() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let works = WorkRepository::new(fixture.pool.clone());
        // seed (becomes `from`) -> middle -> leaf, with `into` unrelated.
        let from = test_support::new_work(&fixture.pool).await;
        let into = test_support::new_work(&fixture.pool).await;
        let middle = test_support::new_work(&fixture.pool).await;
        let leaf = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, from, middle).await;
        test_support::cite(&fixture.pool, middle, leaf).await;
        let ramp = fixture
            .repo
            .create(fixture.workspace, from, &title("Merge me"))
            .await
            .expect("create");
        fixture
            .repo
            .add_item(ramp.id, add(middle, Some(from)))
            .await
            .expect("middle");
        fixture
            .repo
            .add_item(ramp.id, add(leaf, Some(middle)))
            .await
            .expect("leaf");
        // A second ramp where `from` is a middle item (via) of a path.
        let seed_two = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, seed_two, from).await;
        let ramp_two = fixture
            .repo
            .create(fixture.workspace, seed_two, &title("Via ramp"))
            .await
            .expect("create");
        fixture
            .repo
            .add_item(ramp_two.id, add(from, Some(seed_two)))
            .await
            .expect("from as item");
        fixture
            .repo
            .add_item(ramp_two.id, add(middle, Some(from)))
            .await
            .expect("middle via from");

        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");

        let first = fixture
            .repo
            .get(fixture.workspace, ramp.id)
            .await
            .expect("get")
            .expect("ramp");
        assert_eq!(first.seed_work_id, into);
        let items = fixture.repo.items(ramp.id).await.expect("items");
        let by_work = |work: WorkId| {
            items
                .iter()
                .find(|item| item.work_id == work)
                .expect("item")
        };
        assert_eq!(by_work(into).path, vec![into]);
        assert_eq!(by_work(into).position, 0);
        assert!(items.iter().all(|item| item.work_id != from));
        assert_eq!(by_work(middle).path, vec![into, middle]);
        assert_eq!(by_work(middle).via_work_id, Some(into));
        assert_eq!(by_work(leaf).path, vec![into, middle, leaf]);
        assert_eq!(by_work(leaf).via_work_id, Some(middle));
        // The seed item is still protected under its new identity.
        assert_eq!(
            fixture.repo.remove_item(ramp.id, into).await.expect("seed"),
            RemoveOutcome::Seed
        );

        let second = fixture.repo.items(ramp_two.id).await.expect("items");
        let moved = second
            .iter()
            .find(|item| item.work_id == into)
            .expect("repointed item");
        assert_eq!(moved.path, vec![seed_two, into]);
        assert_eq!(moved.via_work_id, Some(seed_two));
        let below = second
            .iter()
            .find(|item| item.work_id == middle)
            .expect("middle");
        assert_eq!(below.path, vec![seed_two, into, middle]);
        assert_eq!(below.via_work_id, Some(into));
    }

    #[tokio::test]
    async fn should_drop_merged_item_when_survivor_already_in_ramp() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let works = WorkRepository::new(fixture.pool.clone());
        let seed = test_support::new_work(&fixture.pool).await;
        let from = test_support::new_work(&fixture.pool).await;
        let into = test_support::new_work(&fixture.pool).await;
        let below = test_support::new_work(&fixture.pool).await;
        test_support::cite(&fixture.pool, seed, from).await;
        test_support::cite(&fixture.pool, seed, into).await;
        test_support::cite(&fixture.pool, from, below).await;
        let ramp = fixture
            .repo
            .create(fixture.workspace, seed, &title("Both"))
            .await
            .expect("create");
        fixture
            .repo
            .add_item(ramp.id, add(from, Some(seed)))
            .await
            .expect("from");
        fixture
            .repo
            .add_item(
                ramp.id,
                NewRampItem {
                    work_id: into,
                    via: Some(seed),
                    note: Some("survivor note".into()),
                },
            )
            .await
            .expect("into");
        fixture
            .repo
            .add_item(ramp.id, add(below, Some(from)))
            .await
            .expect("below");
        fixture
            .repo
            .update_item(
                ramp.id,
                from,
                RampItemPatch {
                    status: Some(RampItemStatus::Read),
                    note: Some(Some("lost note".into())),
                },
            )
            .await
            .expect("update");

        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");

        let items = fixture.repo.items(ramp.id).await.expect("items");
        assert_eq!(items.len(), 3, "the merged item is dropped, not duplicated");
        assert!(items.iter().all(|item| item.work_id != from));
        let survivor = items
            .iter()
            .find(|item| item.work_id == into)
            .expect("survivor");
        assert_eq!(survivor.note.as_deref(), Some("survivor note"));
        assert_eq!(
            survivor.status,
            RampItemStatus::Unread,
            "the survivor's row wins"
        );
        assert_eq!(survivor.path, vec![seed, into]);
        let descendant = items
            .iter()
            .find(|item| item.work_id == below)
            .expect("below");
        assert_eq!(descendant.path, vec![seed, into, below]);
        assert_eq!(descendant.via_work_id, Some(into));
    }

    #[tokio::test]
    async fn should_move_expansion_row_only_when_survivor_has_none() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let works = WorkRepository::new(fixture.pool.clone());
        let expansions = ExpansionRepository::new(fixture.pool.clone());
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

        // Survivor has no row: the merged Work's row moves.
        let from = test_support::new_work(&fixture.pool).await;
        let into = test_support::new_work(&fixture.pool).await;
        expansions.record(record(from, 7)).await.expect("record");
        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");
        assert_eq!(expansions.get(from).await.expect("get"), None);
        let moved = expansions.get(into).await.expect("get").expect("moved");
        assert_eq!(moved.references_count, 7);

        // Survivor has its own row: it is kept and the merged Work's row is deleted.
        let from = test_support::new_work(&fixture.pool).await;
        let into = test_support::new_work(&fixture.pool).await;
        expansions.record(record(from, 7)).await.expect("record");
        expansions.record(record(into, 2)).await.expect("record");
        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");
        assert_eq!(expansions.get(from).await.expect("get"), None);
        assert_eq!(
            expansions
                .get(into)
                .await
                .expect("get")
                .expect("kept")
                .references_count,
            2
        );

        // Survivor-only row stays; a merge with neither is fine.
        let from = test_support::new_work(&fixture.pool).await;
        let into = test_support::new_work(&fixture.pool).await;
        expansions.record(record(into, 4)).await.expect("record");
        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");
        assert_eq!(
            expansions
                .get(into)
                .await
                .expect("get")
                .expect("kept")
                .references_count,
            4
        );
    }
}
