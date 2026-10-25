//! Persistence for per-Work reading status (D6): a Work is `read` in every list or in none.
//! A Work with no row is `unread`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use lysilogy_core::{ReadingStatus, WorkId, WorkspaceId};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;

/// One Work's stored reading status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WorkReadingState {
    pub work_id: WorkId,
    pub status: ReadingStatus,
    pub changed_at: DateTime<Utc>,
}

/// Reads and writes reading status.
#[derive(Clone)]
pub struct ReadingStateRepository {
    pool: PgPool,
}

impl ReadingStateRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Set a Work's status. `changed_at` moves only when the status really changes.
    pub async fn set(
        &self,
        workspace: WorkspaceId,
        work: WorkId,
        status: ReadingStatus,
    ) -> Result<WorkReadingState> {
        let row = sqlx::query(
            "insert into reader.work_reading_state as s (workspace_id, work_id, status) \
             values ($1, $2, $3) \
             on conflict (workspace_id, work_id) do update set \
                 status = excluded.status, \
                 changed_at = case when s.status <> excluded.status then now() \
                     else s.changed_at end \
             returning work_id, status, changed_at",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .bind(status.as_str())
        .fetch_one(&self.pool)
        .await?;
        state_from_row(&row)
    }

    /// The D17 nudge: move an `unread` (or never recorded) Work to `reading`, and leave
    /// `reading` and `read` alone. Whether the status changed.
    pub async fn nudge_to_reading(&self, workspace: WorkspaceId, work: WorkId) -> Result<bool> {
        let result = sqlx::query(
            "insert into reader.work_reading_state as s (workspace_id, work_id, status) \
             values ($1, $2, 'reading') \
             on conflict (workspace_id, work_id) do update set \
                 status = 'reading', changed_at = now() \
             where s.status = 'unread'",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// One Work's status; `Unread` when none is stored.
    pub async fn get(&self, workspace: WorkspaceId, work: WorkId) -> Result<ReadingStatus> {
        let status: Option<String> = sqlx::query_scalar(
            "select status from reader.work_reading_state where workspace_id = $1 and work_id = $2",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        Ok(status.map_or(Ok(ReadingStatus::Unread), |text| text.parse())?)
    }

    /// Stored statuses for the given Works (those with no row are absent).
    pub async fn for_works(
        &self,
        workspace: WorkspaceId,
        works: &[WorkId],
    ) -> Result<HashMap<WorkId, WorkReadingState>> {
        if works.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|work| work.as_uuid()).collect();
        let rows = sqlx::query(
            "select work_id, status, changed_at from reader.work_reading_state \
             where workspace_id = $1 and work_id = any($2)",
        )
        .bind(workspace.as_uuid())
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| state_from_row(row).map(|state| (state.work_id, state)))
            .collect()
    }

    /// Every Work marked `read` or `reading` in the workspace.
    pub async fn known(&self, workspace: WorkspaceId) -> Result<Vec<WorkId>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "select work_id from reader.work_reading_state \
             where workspace_id = $1 and status in ('read', 'reading')",
        )
        .bind(workspace.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(WorkId::from_uuid).collect())
    }

    /// Stored statuses for every Work node of a list, ordered by Work ID.
    pub async fn for_list(
        &self,
        workspace: WorkspaceId,
        list: lysilogy_core::ListId,
    ) -> Result<Vec<WorkReadingState>> {
        let rows = sqlx::query(
            "select s.work_id, s.status, s.changed_at \
             from reader.work_reading_state s \
             where s.workspace_id = $1 and s.work_id in ( \
                 select n.work_id from reader.list_node n \
                 where n.list_id = $2 and n.kind = 'work') \
             order by s.work_id",
        )
        .bind(workspace.as_uuid())
        .bind(list.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(state_from_row).collect()
    }
}

fn state_from_row(row: &sqlx::postgres::PgRow) -> Result<WorkReadingState> {
    let status: String = row.try_get("status")?;
    Ok(WorkReadingState {
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        status: status.parse()?,
        changed_at: row.try_get("changed_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    async fn fixture() -> Option<(PgPool, ReadingStateRepository, WorkspaceId)> {
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return None;
        };
        let workspace = test_support::new_workspace(&pool).await;
        Some((pool.clone(), ReadingStateRepository::new(pool), workspace))
    }

    #[tokio::test]
    async fn should_report_unread_when_no_status_is_stored() {
        let _guard = test_support::lock().await;
        let Some((pool, repo, workspace)) = fixture().await else {
            return;
        };
        let work = test_support::new_work(&pool).await;
        assert_eq!(
            repo.get(workspace, work).await.expect("get"),
            ReadingStatus::Unread
        );
        assert!(
            repo.for_works(workspace, &[work])
                .await
                .expect("for")
                .is_empty()
        );
        assert!(
            repo.for_works(workspace, &[])
                .await
                .expect("none")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn should_move_changed_at_only_when_status_changes() {
        let _guard = test_support::lock().await;
        let Some((pool, repo, workspace)) = fixture().await else {
            return;
        };
        let work = test_support::new_work(&pool).await;
        let first = repo
            .set(workspace, work, ReadingStatus::Reading)
            .await
            .expect("set");
        // now() is the transaction start, so a later statement is strictly later.
        let same = repo
            .set(workspace, work, ReadingStatus::Reading)
            .await
            .expect("set again");
        assert_eq!(same.changed_at, first.changed_at);
        let read = repo
            .set(workspace, work, ReadingStatus::Read)
            .await
            .expect("read");
        assert!(read.changed_at > first.changed_at);
        assert_eq!(
            repo.get(workspace, work).await.expect("get"),
            ReadingStatus::Read
        );
    }

    #[tokio::test]
    async fn should_keep_status_per_workspace_when_two_workspaces_read_one_work() {
        let _guard = test_support::lock().await;
        let Some((pool, repo, workspace)) = fixture().await else {
            return;
        };
        let other = test_support::new_workspace(&pool).await;
        let work = test_support::new_work(&pool).await;
        repo.set(workspace, work, ReadingStatus::Read)
            .await
            .expect("set");
        assert_eq!(
            repo.get(other, work).await.expect("get"),
            ReadingStatus::Unread
        );
    }

    #[tokio::test]
    async fn should_nudge_only_unread_works_when_opening_a_paper() {
        let _guard = test_support::lock().await;
        let Some((pool, repo, workspace)) = fixture().await else {
            return;
        };
        let fresh = test_support::new_work(&pool).await;
        let explicit_unread = test_support::new_work(&pool).await;
        let reading = test_support::new_work(&pool).await;
        let read = test_support::new_work(&pool).await;
        repo.set(workspace, explicit_unread, ReadingStatus::Unread)
            .await
            .expect("unread");
        repo.set(workspace, reading, ReadingStatus::Reading)
            .await
            .expect("reading");
        repo.set(workspace, read, ReadingStatus::Read)
            .await
            .expect("read");

        assert!(
            repo.nudge_to_reading(workspace, fresh)
                .await
                .expect("fresh")
        );
        assert!(
            repo.nudge_to_reading(workspace, explicit_unread)
                .await
                .expect("unread")
        );
        assert!(
            !repo
                .nudge_to_reading(workspace, reading)
                .await
                .expect("reading")
        );
        assert!(!repo.nudge_to_reading(workspace, read).await.expect("read"));
        assert_eq!(
            repo.get(workspace, fresh).await.expect("get"),
            ReadingStatus::Reading
        );
        assert_eq!(
            repo.get(workspace, read).await.expect("get"),
            ReadingStatus::Read
        );
    }
}
