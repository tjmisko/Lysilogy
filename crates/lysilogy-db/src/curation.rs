//! Persistence for private curation: tags, work tags, and work marks.
//!
//! Everything here is owned by one workspace and never leaves it.

use chrono::{DateTime, Utc};
use lysilogy_core::{TagColor, TagId, TagKind, TagName, WorkId, WorkMark, WorkspaceId};
use serde::Serialize;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

const TAG_COLUMNS: &str =
    "t.id, t.workspace_id, t.name, t.color, t.kind, t.created_at, t.updated_at";

/// A private, named, colored label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Tag {
    pub id: TagId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub color: String,
    pub kind: TagKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A tag with the number of Works carrying it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagWithCount {
    #[serde(flatten)]
    pub tag: Tag,
    pub work_count: i64,
}

/// A stored mark on a Work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StoredMark {
    pub mark: WorkMark,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Reads and writes tags, assignments, and marks.
#[derive(Clone)]
pub struct CurationRepository {
    pool: PgPool,
}

impl CurationRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a tag; a name that matches an existing one ignoring case is `Conflict("tag_exists")`.
    pub async fn create_tag(
        &self,
        workspace: WorkspaceId,
        name: &TagName,
        color: &TagColor,
        kind: TagKind,
    ) -> Result<Tag> {
        let row = sqlx::query(
            "insert into reader.tag as t (workspace_id, name, color, kind) \
             values ($1, $2, $3, $4) \
             returning t.id, t.workspace_id, t.name, t.color, t.kind, t.created_at, t.updated_at",
        )
        .bind(workspace.as_uuid())
        .bind(name.as_str())
        .bind(color.as_str())
        .bind(kind.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(tag_error)?;
        tag_from_row(&row)
    }

    /// Change any of a tag's fields; `None` when the tag is not in the workspace.
    pub async fn update_tag(
        &self,
        workspace: WorkspaceId,
        tag: TagId,
        name: Option<&TagName>,
        color: Option<&TagColor>,
        kind: Option<TagKind>,
    ) -> Result<Option<Tag>> {
        let row = sqlx::query(
            "update reader.tag as t set \
                 name = coalesce($3, t.name), color = coalesce($4, t.color), \
                 kind = coalesce($5, t.kind), updated_at = now() \
             where t.workspace_id = $1 and t.id = $2 \
             returning t.id, t.workspace_id, t.name, t.color, t.kind, t.created_at, t.updated_at",
        )
        .bind(workspace.as_uuid())
        .bind(tag.as_uuid())
        .bind(name.map(TagName::as_str))
        .bind(color.map(TagColor::as_str))
        .bind(kind.map(TagKind::as_str))
        .fetch_optional(&self.pool)
        .await
        .map_err(tag_error)?;
        row.as_ref().map(tag_from_row).transpose()
    }

    /// Delete a tag and, by cascade, its assignments. Whether a tag was removed.
    pub async fn delete_tag(&self, workspace: WorkspaceId, tag: TagId) -> Result<bool> {
        let result = sqlx::query("delete from reader.tag where workspace_id = $1 and id = $2")
            .bind(workspace.as_uuid())
            .bind(tag.as_uuid())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Every tag in the workspace, by lower-cased name, with its Work count.
    pub async fn list_tags(&self, workspace: WorkspaceId) -> Result<Vec<TagWithCount>> {
        let rows = sqlx::query(&format!(
            "select {TAG_COLUMNS}, \
                    (select count(*) from reader.work_tag wt \
                      where wt.workspace_id = t.workspace_id and wt.tag_id = t.id) as work_count \
             from reader.tag t where t.workspace_id = $1 \
             order by lower(t.name), t.id"
        ))
        .bind(workspace.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(TagWithCount {
                    tag: tag_from_row(row)?,
                    work_count: row.try_get("work_count")?,
                })
            })
            .collect()
    }

    /// How many tags the workspace has.
    pub async fn count_tags(&self, workspace: WorkspaceId) -> Result<i64> {
        let count = sqlx::query_scalar("select count(*) from reader.tag where workspace_id = $1")
            .bind(workspace.as_uuid())
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Fetch one tag of the workspace.
    pub async fn get_tag(&self, workspace: WorkspaceId, tag: TagId) -> Result<Option<Tag>> {
        let row = sqlx::query(&format!(
            "select {TAG_COLUMNS} from reader.tag t where t.workspace_id = $1 and t.id = $2"
        ))
        .bind(workspace.as_uuid())
        .bind(tag.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(tag_from_row).transpose()
    }

    /// Attach a tag to a Work; assigning twice is a no-op.
    pub async fn assign_tag(&self, workspace: WorkspaceId, work: WorkId, tag: TagId) -> Result<()> {
        sqlx::query(
            "insert into reader.work_tag (workspace_id, work_id, tag_id) values ($1, $2, $3) \
             on conflict do nothing",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .bind(tag.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Detach a tag from a Work; an absent assignment is fine.
    pub async fn unassign_tag(
        &self,
        workspace: WorkspaceId,
        work: WorkId,
        tag: TagId,
    ) -> Result<()> {
        sqlx::query(
            "delete from reader.work_tag where workspace_id = $1 and work_id = $2 and tag_id = $3",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .bind(tag.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Tags on the given Works, ordered by Work then lower-cased name.
    pub async fn tags_for_works(
        &self,
        workspace: WorkspaceId,
        works: &[WorkId],
    ) -> Result<Vec<(WorkId, Tag)>> {
        if works.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|work| work.as_uuid()).collect();
        let rows = sqlx::query(&format!(
            "select wt.work_id, {TAG_COLUMNS} \
             from reader.work_tag wt \
             join reader.tag t on t.workspace_id = wt.workspace_id and t.id = wt.tag_id \
             where wt.workspace_id = $1 and wt.work_id = any($2) \
             order by wt.work_id, lower(t.name), t.id"
        ))
        .bind(workspace.as_uuid())
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    WorkId::from_uuid(row.try_get("work_id")?),
                    tag_from_row(row)?,
                ))
            })
            .collect()
    }

    /// Set the Work's mark, replacing any previous mark and note.
    pub async fn set_mark(
        &self,
        workspace: WorkspaceId,
        work: WorkId,
        mark: WorkMark,
        note: Option<&str>,
    ) -> Result<StoredMark> {
        let row = sqlx::query(
            "insert into reader.work_mark (workspace_id, work_id, mark, note) \
             values ($1, $2, $3, $4) \
             on conflict (workspace_id, work_id) do update \
                 set mark = excluded.mark, note = excluded.note, updated_at = now() \
             returning mark, note, created_at, updated_at",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .bind(mark.as_str())
        .bind(note)
        .fetch_one(&self.pool)
        .await?;
        mark_from_row(&row)
    }

    /// Remove the Work's mark. Whether one existed.
    pub async fn clear_mark(&self, workspace: WorkspaceId, work: WorkId) -> Result<bool> {
        let result =
            sqlx::query("delete from reader.work_mark where workspace_id = $1 and work_id = $2")
                .bind(workspace.as_uuid())
                .bind(work.as_uuid())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Marks on the given Works, ordered by Work.
    pub async fn marks_for_works(
        &self,
        workspace: WorkspaceId,
        works: &[WorkId],
    ) -> Result<Vec<(WorkId, StoredMark)>> {
        if works.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|work| work.as_uuid()).collect();
        let rows = sqlx::query(
            "select work_id, mark, note, created_at, updated_at from reader.work_mark \
             where workspace_id = $1 and work_id = any($2) order by work_id",
        )
        .bind(workspace.as_uuid())
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    WorkId::from_uuid(row.try_get("work_id")?),
                    mark_from_row(row)?,
                ))
            })
            .collect()
    }
}

/// Map a unique violation on the tag name index to `Conflict("tag_exists")`.
fn tag_error(error: sqlx::Error) -> DbError {
    if let sqlx::Error::Database(database) = &error
        && database.is_unique_violation()
    {
        return DbError::Conflict("tag_exists".into());
    }
    DbError::Sqlx(error)
}

fn tag_from_row(row: &PgRow) -> Result<Tag> {
    let kind: String = row.try_get("kind")?;
    Ok(Tag {
        id: TagId::from_uuid(row.try_get("id")?),
        workspace_id: WorkspaceId::from_uuid(row.try_get("workspace_id")?),
        name: row.try_get("name")?,
        color: row.try_get("color")?,
        kind: kind.parse()?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn mark_from_row(row: &PgRow) -> Result<StoredMark> {
    let mark: String = row.try_get("mark")?;
    Ok(StoredMark {
        mark: mark.parse()?,
        note: row.try_get("note")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn name(text: &str) -> TagName {
        TagName::parse(text).expect("name")
    }

    fn color(text: &str) -> TagColor {
        TagColor::parse(text).expect("color")
    }

    fn unique(prefix: &str) -> String {
        format!("{prefix} {}", Uuid::new_v4().simple())
    }

    #[tokio::test]
    async fn should_create_and_list_tag_with_zero_count_when_new() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        assert_eq!(repo.count_tags(workspace).await.expect("count"), 0);
        let tag = repo
            .create_tag(workspace, &name("Methods"), &color("#abc"), TagKind::Topic)
            .await
            .expect("create");
        assert_eq!(tag.color, "#aabbcc");
        assert_eq!(tag.kind, TagKind::Topic);
        let listed = repo.list_tags(workspace).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].tag, tag);
        assert_eq!(listed[0].work_count, 0);
        assert_eq!(repo.count_tags(workspace).await.expect("count"), 1);
        assert_eq!(
            repo.get_tag(workspace, tag.id).await.expect("get"),
            Some(tag)
        );
    }

    #[tokio::test]
    async fn should_conflict_when_tag_name_differs_only_by_case() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let other = test_support::new_workspace(&pool).await;
        repo.create_tag(workspace, &name("Read"), &color("#111"), TagKind::Status)
            .await
            .expect("first");
        let duplicate = repo
            .create_tag(workspace, &name("rEAD"), &color("#222"), TagKind::Topic)
            .await;
        assert!(
            matches!(&duplicate, Err(DbError::Conflict(code)) if code == "tag_exists"),
            "{duplicate:?}"
        );
        repo.create_tag(other, &name("read"), &color("#222"), TagKind::Topic)
            .await
            .expect("same name in another workspace is fine");

        let second = repo
            .create_tag(workspace, &name("Other"), &color("#333"), TagKind::Topic)
            .await
            .expect("second");
        let rename = repo
            .update_tag(workspace, second.id, Some(&name("READ")), None, None)
            .await;
        assert!(matches!(rename, Err(DbError::Conflict(_))), "{rename:?}");
    }

    #[tokio::test]
    async fn should_update_color_and_kind_when_patching_tag() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let tag = repo
            .create_tag(workspace, &name("Topic"), &color("#111111"), TagKind::Topic)
            .await
            .expect("create");
        let updated = repo
            .update_tag(
                workspace,
                tag.id,
                None,
                Some(&color("#FF0000")),
                Some(TagKind::Status),
            )
            .await
            .expect("update")
            .expect("found");
        assert_eq!(updated.name, "Topic");
        assert_eq!(updated.color, "#ff0000");
        assert_eq!(updated.kind, TagKind::Status);
        assert!(updated.updated_at >= tag.updated_at);
        let renamed = repo
            .update_tag(workspace, tag.id, Some(&name("Renamed")), None, None)
            .await
            .expect("rename")
            .expect("found");
        assert_eq!(renamed.name, "Renamed");
        assert_eq!(renamed.color, "#ff0000");
    }

    #[tokio::test]
    async fn should_return_none_when_updating_tag_of_other_workspace() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let other = test_support::new_workspace(&pool).await;
        let tag = repo
            .create_tag(workspace, &name("Mine"), &color("#111"), TagKind::Topic)
            .await
            .expect("create");
        let updated = repo
            .update_tag(other, tag.id, Some(&name("Stolen")), None, None)
            .await
            .expect("update");
        assert_eq!(updated, None);
        assert_eq!(repo.get_tag(other, tag.id).await.expect("get"), None);
        assert!(!repo.delete_tag(other, tag.id).await.expect("delete"));
        assert!(
            repo.get_tag(workspace, tag.id)
                .await
                .expect("get")
                .is_some()
        );
        assert_eq!(
            repo.update_tag(workspace, TagId::new(), None, None, None)
                .await
                .expect("unknown"),
            None
        );
    }

    #[tokio::test]
    async fn should_cascade_assignments_when_deleting_tag() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let work = test_support::new_work(&pool).await;
        let tag = repo
            .create_tag(workspace, &name("Doomed"), &color("#111"), TagKind::Topic)
            .await
            .expect("create");
        repo.assign_tag(workspace, work, tag.id)
            .await
            .expect("assign");
        assert_eq!(
            repo.list_tags(workspace).await.expect("list")[0].work_count,
            1
        );
        assert!(repo.delete_tag(workspace, tag.id).await.expect("delete"));
        assert!(
            repo.tags_for_works(workspace, &[work])
                .await
                .expect("tags")
                .is_empty()
        );
        assert!(!repo.delete_tag(workspace, tag.id).await.expect("again"));
    }

    #[tokio::test]
    async fn should_ignore_duplicate_assignment_when_assigning_twice() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let first = test_support::new_work(&pool).await;
        let second = test_support::new_work(&pool).await;
        let zebra = repo
            .create_tag(workspace, &name("zebra"), &color("#111"), TagKind::Topic)
            .await
            .expect("zebra");
        let apple = repo
            .create_tag(workspace, &name("Apple"), &color("#222"), TagKind::Topic)
            .await
            .expect("apple");
        for _ in 0..2 {
            repo.assign_tag(workspace, first, zebra.id)
                .await
                .expect("assign");
        }
        repo.assign_tag(workspace, first, apple.id)
            .await
            .expect("assign");
        repo.assign_tag(workspace, second, zebra.id)
            .await
            .expect("assign");

        let tags = repo
            .tags_for_works(workspace, &[first, second])
            .await
            .expect("tags");
        assert_eq!(tags.len(), 3);
        let first_names: Vec<&str> = tags
            .iter()
            .filter(|(work, _)| *work == first)
            .map(|(_, tag)| tag.name.as_str())
            .collect();
        assert_eq!(
            first_names,
            vec!["Apple", "zebra"],
            "ordered by lower(name)"
        );
        assert!(
            tags.windows(2).all(|pair| pair[0].0 <= pair[1].0),
            "ordered by work"
        );
        assert!(
            repo.tags_for_works(workspace, &[])
                .await
                .expect("empty")
                .is_empty()
        );

        repo.unassign_tag(workspace, first, zebra.id)
            .await
            .expect("unassign");
        repo.unassign_tag(workspace, first, zebra.id)
            .await
            .expect("unassigning an absent assignment is fine");
        let after = repo
            .tags_for_works(workspace, &[first])
            .await
            .expect("tags");
        assert_eq!(after.len(), 1);
    }

    #[tokio::test]
    async fn should_reject_cross_workspace_assignment_when_tag_belongs_elsewhere() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let other = test_support::new_workspace(&pool).await;
        let work = test_support::new_work(&pool).await;
        let tag = repo
            .create_tag(workspace, &name("Private"), &color("#111"), TagKind::Topic)
            .await
            .expect("create");
        let result = sqlx::query(
            "insert into reader.work_tag (workspace_id, work_id, tag_id) values ($1, $2, $3)",
        )
        .bind(other.as_uuid())
        .bind(work.as_uuid())
        .bind(tag.id.as_uuid())
        .execute(&pool)
        .await;
        let Err(sqlx::Error::Database(error)) = result else {
            panic!("cross-workspace assignment must fail: {result:?}");
        };
        assert!(error.is_foreign_key_violation(), "{error}");
        // The repository path is a no-op failure too: it surfaces the same violation.
        assert!(repo.assign_tag(other, work, tag.id).await.is_err());
    }

    #[tokio::test]
    async fn should_replace_mark_and_note_when_setting_again() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let work = test_support::new_work(&pool).await;
        let first = repo
            .set_mark(workspace, work, WorkMark::Pinned, Some("key paper"))
            .await
            .expect("pin");
        assert_eq!(first.mark, WorkMark::Pinned);
        assert_eq!(first.note.as_deref(), Some("key paper"));
        let second = repo
            .set_mark(workspace, work, WorkMark::Hidden, None)
            .await
            .expect("hide");
        assert_eq!(second.mark, WorkMark::Hidden);
        assert_eq!(second.note, None);
        assert_eq!(second.created_at, first.created_at);
        assert!(second.updated_at >= first.updated_at);
        let marks = repo
            .marks_for_works(workspace, &[work])
            .await
            .expect("marks");
        assert_eq!(marks, vec![(work, second)]);
        assert!(
            repo.marks_for_works(workspace, &[])
                .await
                .expect("empty")
                .is_empty()
        );
        assert!(
            repo.marks_for_works(test_support::new_workspace(&pool).await, &[work])
                .await
                .expect("other workspace")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn should_report_false_when_clearing_absent_mark() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CurationRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let work = test_support::new_work(&pool).await;
        assert!(!repo.clear_mark(workspace, work).await.expect("absent"));
        repo.set_mark(workspace, work, WorkMark::Downplayed, None)
            .await
            .expect("set");
        assert!(repo.clear_mark(workspace, work).await.expect("clear"));
        assert!(!repo.clear_mark(workspace, work).await.expect("again"));
    }

    #[tokio::test]
    async fn should_violate_check_when_mark_value_is_unknown() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let workspace = test_support::new_workspace(&pool).await;
        let work = test_support::new_work(&pool).await;
        let bad_mark = sqlx::query(
            "insert into reader.work_mark (workspace_id, work_id, mark) values ($1, $2, 'starred')",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .execute(&pool)
        .await;
        let Err(sqlx::Error::Database(error)) = bad_mark else {
            panic!("unknown mark must fail: {bad_mark:?}");
        };
        assert!(error.is_check_violation(), "{error}");

        let long_note = sqlx::query(
            "insert into reader.work_mark (workspace_id, work_id, mark, note) \
             values ($1, $2, 'pinned', repeat('x', 2001))",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .execute(&pool)
        .await;
        assert!(matches!(long_note, Err(sqlx::Error::Database(ref e)) if e.is_check_violation()));

        let bad_color = sqlx::query(
            "insert into reader.tag (workspace_id, name, color) values ($1, $2, '#FFF')",
        )
        .bind(workspace.as_uuid())
        .bind(unique("color"))
        .execute(&pool)
        .await;
        assert!(matches!(bad_color, Err(sqlx::Error::Database(ref e)) if e.is_check_violation()));
    }
}
