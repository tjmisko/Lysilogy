//! Persistence for reading lists.
//!
//! A list is one graph of Work, topic and claim nodes with typed edges, owned by one
//! workspace. Reading status lives with the Work ([`crate::reading_state`]), never on the
//! node.
//!
//! Every write that depends on the list's contents runs under a row lock on the list, so
//! limits, uniqueness and edge validation hold under concurrent requests. Refusals are
//! [`DbError::Conflict`] carrying a [`ListError::code`]; a missing list is
//! [`DbError::not_found`].

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use lysilogy_core::{
    EdgeKind, EdgeRejection, EdgeSource, GenerationId, LIST_EDGES_MAX, LIST_TOPICS_MAX,
    LIST_WORKS_MAX, ListEdge, ListError, ListId, ListOrigin, ListReview, ListTitle, NodeId,
    NodeKind, NodeSource, RAMP_PATH_MAX, TopicLabel, WorkId, WorkspaceId, validate_edge,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

/// Most provider candidates a claim keeps.
pub const CLAIM_CANDIDATES_MAX: usize = 5;
/// Most context links a node carries (D16).
pub const CONTEXT_LINKS_MAX: usize = 5;

const LIST_COLUMNS: &str = "l.id, l.workspace_id, l.title, l.description, l.origin, \
    l.seed_work_id, l.goal, l.archived_at, l.created_at, l.updated_at";
const NODE_COLUMNS: &str = "n.id, n.list_id, n.kind, n.work_id, n.label, n.summary, n.claim, \
    n.claim_state, n.candidates, n.position, n.note, n.rationale, n.context_links, n.source, \
    n.review, n.via_work_id, n.path_work_ids, n.generation_id, n.added_at, n.updated_at";
const EDGE_COLUMNS: &str =
    "e.list_id, e.from_node, e.to_node, e.kind, e.source, e.review, e.created_at";

fn conflict(error: ListError) -> DbError {
    DbError::Conflict(error.code().to_owned())
}

/// A reading list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct List {
    pub id: ListId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub description: Option<String>,
    pub origin: ListOrigin,
    pub seed_work_id: Option<WorkId>,
    pub goal: Option<String>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A list with node counts and the reading status of its Works.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListSummary {
    #[serde(flatten)]
    pub list: List,
    /// Work and claim nodes.
    pub item_count: i64,
    pub topic_count: i64,
    pub claim_count: i64,
    pub proposed_count: i64,
    pub unread: i64,
    pub reading: i64,
    pub read: i64,
}

/// A node of a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListNode {
    pub id: NodeId,
    pub list_id: ListId,
    pub kind: NodeKind,
    pub work_id: Option<WorkId>,
    pub label: Option<String>,
    pub summary: Option<String>,
    /// `{title, authors[], year, venue, doi, arxiv, raw}` for claim nodes.
    pub claim: Option<Value>,
    pub claim_state: Option<lysilogy_core::ClaimState>,
    pub candidates: Value,
    pub position: i32,
    pub note: Option<String>,
    pub rationale: Option<String>,
    pub context_links: Value,
    pub source: NodeSource,
    pub review: ListReview,
    pub via_work_id: Option<WorkId>,
    /// Seed or root through this Work, inclusive, when it was reached by walking citations.
    pub path: Option<Vec<WorkId>>,
    pub generation_id: Option<GenerationId>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A stored edge of a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StoredEdge {
    pub list_id: ListId,
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub source: EdgeSource,
    pub review: ListReview,
    pub created_at: DateTime<Utc>,
}

/// A list to create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewList {
    pub title: ListTitle,
    pub description: Option<String>,
    pub origin: ListOrigin,
    /// Recorded on the list only; add the seed Work as a node separately.
    pub seed_work_id: Option<WorkId>,
    pub goal: Option<String>,
}

impl NewList {
    /// A manual list with just a title.
    #[must_use]
    pub const fn manual(title: ListTitle) -> Self {
        Self {
            title,
            description: None,
            origin: ListOrigin::Manual,
            seed_work_id: None,
            goal: None,
        }
    }
}

/// A partial change to a list. `description: Some(None)` clears it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPatch {
    pub title: Option<ListTitle>,
    pub description: Option<Option<String>>,
    pub archived: Option<bool>,
}

/// Which lists a query returns by archive state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveScope {
    Active,
    Archived,
    All,
}

/// What a new node is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewNodeContent {
    Work {
        work_id: WorkId,
        /// The member this Work was reached from, which must cite it.
        via: Option<WorkId>,
        /// Topic nodes to file the Work under (`covers` edges).
        topics: Vec<NodeId>,
    },
    Topic {
        label: TopicLabel,
        summary: Option<String>,
    },
    Claim {
        claim: Value,
        state: lysilogy_core::ClaimState,
        candidates: Value,
    },
}

/// A node to add; it is appended after the last position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNode {
    pub content: NewNodeContent,
    pub note: Option<String>,
    pub rationale: Option<String>,
    pub context_links: Value,
    pub source: NodeSource,
    pub review: ListReview,
    pub generation_id: Option<GenerationId>,
}

#[allow(clippy::missing_const_for_fn)]
impl NewNode {
    fn with(content: NewNodeContent) -> Self {
        Self {
            content,
            note: None,
            rationale: None,
            context_links: Value::Array(Vec::new()),
            source: NodeSource::User,
            review: ListReview::Accepted,
            generation_id: None,
        }
    }

    /// A user-added Work node.
    #[must_use]
    pub fn work(work_id: WorkId) -> Self {
        Self::with(NewNodeContent::Work {
            work_id,
            via: None,
            topics: Vec::new(),
        })
    }

    /// A user-added Work node reached from another member.
    #[must_use]
    pub fn work_via(work_id: WorkId, via: WorkId) -> Self {
        Self::with(NewNodeContent::Work {
            work_id,
            via: Some(via),
            topics: Vec::new(),
        })
    }

    /// A user-added topic node.
    #[must_use]
    pub fn topic(label: TopicLabel) -> Self {
        Self::with(NewNodeContent::Topic {
            label,
            summary: None,
        })
    }

    /// A user-added claim node.
    #[must_use]
    pub fn claim(claim: Value, state: lysilogy_core::ClaimState) -> Self {
        Self::with(NewNodeContent::Claim {
            claim,
            state,
            candidates: Value::Array(Vec::new()),
        })
    }
}

/// A partial change to a node. `Some(None)` clears a nullable field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodePatch {
    pub note: Option<Option<String>>,
    /// Topic nodes only.
    pub label: Option<TopicLabel>,
    /// Topic nodes only.
    pub summary: Option<Option<String>>,
    /// Accept a `proposed` node.
    pub accept: bool,
}

/// An edge to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub source: EdgeSource,
    pub review: ListReview,
}

/// Everything removing a node deleted, enough to undo it with
/// [`ListRepository::restore_node`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RemovedNode {
    pub node: ListNode,
    /// Edges that touched the node.
    pub edges: Vec<StoredEdge>,
    /// The node was the list's seed Work, which removal cleared (D14).
    pub was_seed: bool,
}

/// One list a Work is in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Membership {
    pub list_id: ListId,
    pub title: String,
    pub archived: bool,
    pub node_id: NodeId,
    /// Labels of the topics that cover the Work.
    pub topics: Vec<String>,
    pub note: Option<String>,
}

/// Reads and writes reading lists.
#[derive(Clone)]
pub struct ListRepository {
    pool: PgPool,
}

impl ListRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a list with no nodes.
    pub async fn create(&self, workspace: WorkspaceId, new: NewList) -> Result<List> {
        let row = sqlx::query(&format!(
            "insert into reader.reading_list as l \
                 (workspace_id, title, description, origin, seed_work_id, goal) \
             values ($1, $2, $3, $4, $5, $6) returning {LIST_COLUMNS}"
        ))
        .bind(workspace.as_uuid())
        .bind(new.title.as_str())
        .bind(new.description)
        .bind(new.origin.as_str())
        .bind(new.seed_work_id.map(WorkId::as_uuid))
        .bind(new.goal)
        .fetch_one(&self.pool)
        .await?;
        list_from_row(&row)
    }

    /// The workspace's lists, most recently updated first, optionally only those holding
    /// one Work.
    pub async fn list(
        &self,
        workspace: WorkspaceId,
        work: Option<WorkId>,
        scope: ArchiveScope,
    ) -> Result<Vec<ListSummary>> {
        let rows = sqlx::query(&format!(
            "select {LIST_COLUMNS}, \
                    count(n.id) filter (where n.kind in ('work', 'claim')) as item_count, \
                    count(n.id) filter (where n.kind = 'topic') as topic_count, \
                    count(n.id) filter (where n.kind = 'claim') as claim_count, \
                    count(n.id) filter (where n.review = 'proposed') as proposed_count, \
                    count(n.id) filter (where n.kind = 'work' \
                        and coalesce(s.status, 'unread') = 'unread') as unread, \
                    count(n.id) filter (where n.kind = 'work' and s.status = 'reading') as reading, \
                    count(n.id) filter (where n.kind = 'work' and s.status = 'read') as read \
             from reader.reading_list l \
             left join reader.list_node n on n.list_id = l.id \
             left join reader.work_reading_state s \
                 on s.workspace_id = l.workspace_id and s.work_id = n.work_id \
             where l.workspace_id = $1 \
               and ($2::uuid is null or exists ( \
                   select 1 from reader.list_node m \
                   where m.list_id = l.id and m.kind = 'work' and m.work_id = $2)) \
               and case $3 when 'active' then l.archived_at is null \
                           when 'archived' then l.archived_at is not null else true end \
             group by l.id \
             order by l.updated_at desc, l.id"
        ))
        .bind(workspace.as_uuid())
        .bind(work.map(WorkId::as_uuid))
        .bind(match scope {
            ArchiveScope::Active => "active",
            ArchiveScope::Archived => "archived",
            ArchiveScope::All => "all",
        })
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ListSummary {
                    list: list_from_row(row)?,
                    item_count: row.try_get("item_count")?,
                    topic_count: row.try_get("topic_count")?,
                    claim_count: row.try_get("claim_count")?,
                    proposed_count: row.try_get("proposed_count")?,
                    unread: row.try_get("unread")?,
                    reading: row.try_get("reading")?,
                    read: row.try_get("read")?,
                })
            })
            .collect()
    }

    /// One list of the workspace.
    pub async fn get(&self, workspace: WorkspaceId, list: ListId) -> Result<Option<List>> {
        let row = sqlx::query(&format!(
            "select {LIST_COLUMNS} from reader.reading_list l \
             where l.workspace_id = $1 and l.id = $2"
        ))
        .bind(workspace.as_uuid())
        .bind(list.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(list_from_row).transpose()
    }

    /// The workspace that owns a list, if the list exists.
    pub async fn workspace_of(&self, list: ListId) -> Result<Option<WorkspaceId>> {
        let workspace: Option<Uuid> =
            sqlx::query_scalar("select workspace_id from reader.reading_list where id = $1")
                .bind(list.as_uuid())
                .fetch_optional(&self.pool)
                .await?;
        Ok(workspace.map(WorkspaceId::from_uuid))
    }

    /// Active lists whose title contains every word of `query` (case-insensitive), titles
    /// that start with it first, then most recently updated. A blank query returns the
    /// most recent lists.
    pub async fn search_by_title(
        &self,
        workspace: WorkspaceId,
        query: &str,
        limit: i64,
    ) -> Result<Vec<List>> {
        let words: Vec<String> = query.split_whitespace().map(like_pattern).collect();
        let prefix = like_pattern(&query.split_whitespace().collect::<Vec<_>>().join(" "));
        let rows = sqlx::query(&format!(
            "select {LIST_COLUMNS} from reader.reading_list l \
             where l.workspace_id = $1 and l.archived_at is null \
               and not exists (select 1 from unnest($2::text[]) as w(pattern) \
                               where l.title not ilike w.pattern) \
             order by (l.title ilike $3 || '%') desc, l.updated_at desc, l.id \
             limit $4"
        ))
        .bind(workspace.as_uuid())
        .bind(
            words
                .iter()
                .map(|word| format!("%{word}%"))
                .collect::<Vec<_>>(),
        )
        .bind(prefix)
        .bind(limit.max(0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(list_from_row).collect()
    }

    /// Retitle, describe, archive or unarchive a list; `None` when it is not in the
    /// workspace.
    pub async fn update(
        &self,
        workspace: WorkspaceId,
        list: ListId,
        patch: ListPatch,
    ) -> Result<Option<List>> {
        let (set_description, description) = patch
            .description
            .map_or((false, None), |description| (true, description));
        let row = sqlx::query(&format!(
            "update reader.reading_list as l set \
                 title = coalesce($3, l.title), \
                 description = case when $4 then $5 else l.description end, \
                 archived_at = case when $6::boolean is null then l.archived_at \
                     when $6 then coalesce(l.archived_at, now()) else null end, \
                 updated_at = now() \
             where l.workspace_id = $1 and l.id = $2 returning {LIST_COLUMNS}"
        ))
        .bind(workspace.as_uuid())
        .bind(list.as_uuid())
        .bind(patch.title.as_ref().map(ListTitle::as_str))
        .bind(set_description)
        .bind(description)
        .bind(patch.archived)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(list_from_row).transpose()
    }

    /// Delete a list and, by cascade, its nodes and edges. Whether a list was removed.
    pub async fn delete(&self, workspace: WorkspaceId, list: ListId) -> Result<bool> {
        let result =
            sqlx::query("delete from reader.reading_list where workspace_id = $1 and id = $2")
                .bind(workspace.as_uuid())
                .bind(list.as_uuid())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// A list's nodes by position.
    pub async fn nodes(&self, list: ListId) -> Result<Vec<ListNode>> {
        let rows = sqlx::query(&format!(
            "select {NODE_COLUMNS} from reader.list_node n where n.list_id = $1 \
             order by n.position, n.id"
        ))
        .bind(list.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(node_from_row).collect()
    }

    /// One node of a list.
    pub async fn node(&self, list: ListId, node: NodeId) -> Result<Option<ListNode>> {
        let row = sqlx::query(&format!(
            "select {NODE_COLUMNS} from reader.list_node n where n.list_id = $1 and n.id = $2"
        ))
        .bind(list.as_uuid())
        .bind(node.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(node_from_row).transpose()
    }

    /// The node holding a Work, if the list has one.
    pub async fn node_for_work(&self, list: ListId, work: WorkId) -> Result<Option<ListNode>> {
        let row = sqlx::query(&format!(
            "select {NODE_COLUMNS} from reader.list_node n \
             where n.list_id = $1 and n.kind = 'work' and n.work_id = $2"
        ))
        .bind(list.as_uuid())
        .bind(work.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(node_from_row).transpose()
    }

    /// A list's stored edges, ordered for stable output.
    pub async fn edges(&self, list: ListId) -> Result<Vec<StoredEdge>> {
        let rows = sqlx::query(&format!(
            "select {EDGE_COLUMNS} from reader.list_edge e where e.list_id = $1 \
             order by e.created_at, e.from_node, e.to_node, e.kind"
        ))
        .bind(list.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(edge_from_row).collect()
    }

    /// Append a node. Checks, in order, the list's limits, that a Work or topic label is new
    /// (`item_exists`), and for a Work with `via` that the member exists (`via_not_in_list`)
    /// and cites it (`no_citation_edge`) within [`RAMP_PATH_MAX`] (`ramp_too_deep`).
    pub async fn add_node(&self, list: ListId, new: NewNode) -> Result<ListNode> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let node = insert_new_node(&mut transaction, list, new).await?;
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(node)
    }

    /// Append many nodes in order, all or nothing: the first refusal undoes the rest.
    pub async fn add_nodes(&self, list: ListId, news: Vec<NewNode>) -> Result<Vec<ListNode>> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let mut added = Vec::with_capacity(news.len());
        for new in news {
            added.push(insert_new_node(&mut transaction, list, new).await?);
        }
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(added)
    }

    /// Change a node; `None` when it does not exist in the list. A label or summary on a
    /// non-topic node is refused with `invalid_node_kind`, and a label another topic holds
    /// with `item_exists`.
    pub async fn update_node(
        &self,
        list: ListId,
        node: NodeId,
        patch: NodePatch,
    ) -> Result<Option<ListNode>> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let Some(current) = fetch_node(&mut transaction, list, node).await? else {
            return Ok(None);
        };
        if (patch.label.is_some() || patch.summary.is_some()) && current.kind != NodeKind::Topic {
            return Err(DbError::Conflict("invalid_node_kind".into()));
        }
        if let Some(label) = &patch.label {
            let taken: bool = sqlx::query_scalar(
                "select exists (select 1 from reader.list_node \
                 where list_id = $1 and kind = 'topic' and lower(label) = lower($2) and id <> $3)",
            )
            .bind(list.as_uuid())
            .bind(label.as_str())
            .bind(node.as_uuid())
            .fetch_one(&mut *transaction)
            .await?;
            if taken {
                return Err(conflict(ListError::ItemExists));
            }
        }
        let (note_given, new_note) = patch.note.map_or((false, None), |text| {
            (true, text.filter(|text| !text.is_empty()))
        });
        let (set_summary, summary) = patch.summary.map_or((false, None), |summary| {
            (true, summary.filter(|text| !text.is_empty()))
        });
        let row = sqlx::query(&format!(
            "update reader.list_node as n set \
                 note = case when $3 then $4 else n.note end, \
                 label = coalesce($5, n.label), \
                 summary = case when $6 then $7 else n.summary end, \
                 review = case when $8 then 'accepted' else n.review end, \
                 updated_at = now() \
             where n.list_id = $1 and n.id = $2 returning {NODE_COLUMNS}"
        ))
        .bind(list.as_uuid())
        .bind(node.as_uuid())
        .bind(note_given)
        .bind(new_note)
        .bind(patch.label.as_ref().map(TopicLabel::as_str))
        .bind(set_summary)
        .bind(summary)
        .bind(patch.accept)
        .fetch_one(&mut *transaction)
        .await?;
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        node_from_row(&row).map(Some)
    }

    /// Turn a claim node into a Work node in place (keeping its position, note and
    /// rationale). `None` when the node does not exist; `not_a_claim` when it is not a
    /// claim; `item_exists` when the Work is already a member.
    pub async fn resolve_claim(
        &self,
        list: ListId,
        node: NodeId,
        work: WorkId,
    ) -> Result<Option<ListNode>> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let Some(current) = fetch_node(&mut transaction, list, node).await? else {
            return Ok(None);
        };
        if current.kind != NodeKind::Claim {
            return Err(DbError::Conflict("not_a_claim".into()));
        }
        if work_in_list(&mut transaction, list, work).await? {
            return Err(conflict(ListError::ItemExists));
        }
        let row = sqlx::query(&format!(
            "update reader.list_node as n set kind = 'work', work_id = $3, claim = null, \
                 claim_state = null, candidates = '[]', review = 'accepted', updated_at = now() \
             where n.list_id = $1 and n.id = $2 returning {NODE_COLUMNS}"
        ))
        .bind(list.as_uuid())
        .bind(node.as_uuid())
        .bind(work.as_uuid())
        .fetch_one(&mut *transaction)
        .await?;
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        node_from_row(&row).map(Some)
    }

    /// Delete a node and, by cascade, its edges, leaving a gap in positions. The result
    /// carries what was deleted so the caller can undo; `None` when the node does not exist.
    /// Removing the seed Work clears the list's seed (D14).
    pub async fn remove_node(&self, list: ListId, node: NodeId) -> Result<Option<RemovedNode>> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let Some(removed) = fetch_node(&mut transaction, list, node).await? else {
            return Ok(None);
        };
        let edge_rows = sqlx::query(&format!(
            "select {EDGE_COLUMNS} from reader.list_edge e \
             where e.from_node = $1 or e.to_node = $1 \
             order by e.created_at, e.from_node, e.to_node, e.kind"
        ))
        .bind(node.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
        let edges = edge_rows
            .iter()
            .map(edge_from_row)
            .collect::<Result<Vec<_>>>()?;
        sqlx::query("delete from reader.list_node where id = $1")
            .bind(node.as_uuid())
            .execute(&mut *transaction)
            .await?;
        let cleared = match removed.work_id {
            Some(work) => {
                sqlx::query(
                    "update reader.reading_list set seed_work_id = null \
                     where id = $1 and seed_work_id = $2",
                )
                .bind(list.as_uuid())
                .bind(work.as_uuid())
                .execute(&mut *transaction)
                .await?
                .rows_affected()
                    > 0
            }
            None => false,
        };
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(Some(RemovedNode {
            node: removed,
            edges,
            was_seed: cleared,
        }))
    }

    /// Undo [`Self::remove_node`]: reinsert the node under its ID at its old position (the
    /// end when taken), the edges whose other end still exists, and the seed link. Refuses
    /// with `item_exists` when the Work or topic label has been re-added meanwhile.
    pub async fn restore_node(&self, list: ListId, removed: &RemovedNode) -> Result<ListNode> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let node = &removed.node;
        if let Some(work) = node.work_id
            && work_in_list(&mut transaction, list, work).await?
        {
            return Err(conflict(ListError::ItemExists));
        }
        if let Some(label) = &node.label {
            let taken: bool = sqlx::query_scalar(
                "select exists (select 1 from reader.list_node \
                 where list_id = $1 and kind = 'topic' and lower(label) = lower($2))",
            )
            .bind(list.as_uuid())
            .bind(label)
            .fetch_one(&mut *transaction)
            .await?;
            if taken {
                return Err(conflict(ListError::ItemExists));
            }
        }
        let position_taken: bool = sqlx::query_scalar(
            "select exists (select 1 from reader.list_node where list_id = $1 and position = $2)",
        )
        .bind(list.as_uuid())
        .bind(node.position)
        .fetch_one(&mut *transaction)
        .await?;
        let position = if position_taken {
            next_position(&mut transaction, list).await?
        } else {
            node.position
        };
        let row = sqlx::query(&format!(
            "insert into reader.list_node as n \
                 (id, list_id, kind, work_id, label, summary, claim, claim_state, candidates, \
                  position, note, rationale, context_links, source, review, via_work_id, \
                  path_work_ids, generation_id, added_at) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                     $17, $18, $19) returning {NODE_COLUMNS}"
        ))
        .bind(node.id.as_uuid())
        .bind(list.as_uuid())
        .bind(node.kind.as_str())
        .bind(node.work_id.map(WorkId::as_uuid))
        .bind(&node.label)
        .bind(&node.summary)
        .bind(&node.claim)
        .bind(node.claim_state.map(lysilogy_core::ClaimState::as_str))
        .bind(&node.candidates)
        .bind(position)
        .bind(&node.note)
        .bind(&node.rationale)
        .bind(&node.context_links)
        .bind(node.source.as_str())
        .bind(node.review.as_str())
        .bind(node.via_work_id.map(WorkId::as_uuid))
        .bind(
            node.path
                .as_ref()
                .map(|path| path.iter().map(|work| work.as_uuid()).collect::<Vec<_>>()),
        )
        .bind(node.generation_id.map(GenerationId::as_uuid))
        .bind(node.added_at)
        .fetch_one(&mut *transaction)
        .await?;
        for edge in &removed.edges {
            sqlx::query(
                "insert into reader.list_edge (list_id, from_node, to_node, kind, source, review, \
                     created_at) \
                 select $1, $2, $3, $4, $5, $6, $7 \
                 where exists (select 1 from reader.list_node where id = $2 and list_id = $1) \
                   and exists (select 1 from reader.list_node where id = $3 and list_id = $1) \
                 on conflict do nothing",
            )
            .bind(list.as_uuid())
            .bind(edge.from.as_uuid())
            .bind(edge.to.as_uuid())
            .bind(edge.kind.as_str())
            .bind(edge.source.as_str())
            .bind(edge.review.as_str())
            .bind(edge.created_at)
            .execute(&mut *transaction)
            .await?;
        }
        if let (true, Some(work)) = (removed.was_seed, node.work_id) {
            sqlx::query(
                "update reader.reading_list set seed_work_id = $2 \
                 where id = $1 and seed_work_id is null",
            )
            .bind(list.as_uuid())
            .bind(work.as_uuid())
            .execute(&mut *transaction)
            .await?;
        }
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        node_from_row(&row)
    }

    /// Add an edge. Storing an edge that exists is a no-op returning it. The outer error is
    /// a database failure, a missing list, or `edges_full`; the inner one is the core
    /// refusal (unknown endpoint, wrong kind for the endpoints, or a cycle with its nodes).
    pub async fn add_edge(
        &self,
        list: ListId,
        new: NewEdge,
    ) -> Result<std::result::Result<StoredEdge, EdgeRejection>> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let edge = match insert_edge(&mut transaction, list, new).await? {
            Ok(edge) => edge,
            Err(rejection) => return Ok(Err(rejection)),
        };
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(Ok(edge))
    }

    /// Remove an edge. Whether one was removed.
    pub async fn remove_edge(
        &self,
        list: ListId,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let result = sqlx::query(
            "delete from reader.list_edge \
             where list_id = $1 and from_node = $2 and to_node = $3 and kind = $4",
        )
        .bind(list.as_uuid())
        .bind(from.as_uuid())
        .bind(to.as_uuid())
        .bind(kind.as_str())
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(true)
    }

    /// Accept a `proposed` edge. Whether the edge exists.
    pub async fn accept_edge(
        &self,
        list: ListId,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    ) -> Result<bool> {
        let result = sqlx::query(
            "update reader.list_edge set review = 'accepted' \
             where list_id = $1 and from_node = $2 and to_node = $3 and kind = $4",
        )
        .bind(list.as_uuid())
        .bind(from.as_uuid())
        .bind(to.as_uuid())
        .bind(kind.as_str())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Rewrite positions to `0..n` following `order`, in one transaction. A list that is not
    /// exactly the list's nodes is refused with `invalid_order` and nothing changes.
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    pub async fn reorder(&self, list: ListId, order: &[NodeId]) -> Result<()> {
        let ids: Vec<Uuid> = order.iter().map(|node| node.as_uuid()).collect();
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let total: i64 =
            sqlx::query_scalar("select count(*) from reader.list_node where list_id = $1")
                .bind(list.as_uuid())
                .fetch_one(&mut *transaction)
                .await?;
        let result = sqlx::query(
            "update reader.list_node n set position = (o.ordinal - 1)::int, updated_at = now() \
             from unnest($2::uuid[]) with ordinality as o(node_id, ordinal) \
             where n.list_id = $1 and n.id = o.node_id",
        )
        .bind(list.as_uuid())
        .bind(&ids)
        .execute(&mut *transaction)
        .await?;
        let distinct = ids.iter().collect::<HashSet<_>>().len();
        if result.rows_affected() as usize != ids.len()
            || ids.len() != distinct
            || ids.len() as i64 != total
        {
            return Err(conflict(ListError::InvalidOrder));
        }
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// The lists of the workspace that hold a Work (D5), most recently updated first.
    pub async fn memberships_by_work(
        &self,
        workspace: WorkspaceId,
        work: WorkId,
        scope: ArchiveScope,
    ) -> Result<Vec<Membership>> {
        let rows = sqlx::query(
            "select l.id as list_id, l.title, l.archived_at is not null as archived, \
                    n.id as node_id, n.note, \
                    coalesce((select array_agg(t.label order by t.position) \
                              from reader.list_edge e \
                              join reader.list_node t on t.id = e.from_node \
                              where e.to_node = n.id and e.kind = 'covers'), '{}') as topics \
             from reader.list_node n \
             join reader.reading_list l on l.id = n.list_id \
             where l.workspace_id = $1 and n.kind = 'work' and n.work_id = $2 \
               and case $3 when 'active' then l.archived_at is null \
                           when 'archived' then l.archived_at is not null else true end \
             order by l.updated_at desc, l.id",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .bind(match scope {
            ArchiveScope::Active => "active",
            ArchiveScope::Archived => "archived",
            ArchiveScope::All => "all",
        })
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(Membership {
                    list_id: ListId::from_uuid(row.try_get("list_id")?),
                    title: row.try_get("title")?,
                    archived: row.try_get("archived")?,
                    node_id: NodeId::from_uuid(row.try_get("node_id")?),
                    topics: row.try_get("topics")?,
                    note: row.try_get("note")?,
                })
            })
            .collect()
    }
}

/// Escape `LIKE` metacharacters so user text matches literally.
fn like_pattern(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) async fn lock_list(transaction: &mut PgConnection, list: ListId) -> Result<()> {
    let locked: Option<Uuid> =
        sqlx::query_scalar("select id from reader.reading_list where id = $1 for update")
            .bind(list.as_uuid())
            .fetch_optional(&mut *transaction)
            .await?;
    if locked.is_none() {
        return Err(DbError::not_found());
    }
    Ok(())
}

pub(crate) async fn touch_list(transaction: &mut PgConnection, list: ListId) -> Result<()> {
    sqlx::query("update reader.reading_list set updated_at = now() where id = $1")
        .bind(list.as_uuid())
        .execute(&mut *transaction)
        .await?;
    Ok(())
}

async fn fetch_node(
    transaction: &mut PgConnection,
    list: ListId,
    node: NodeId,
) -> Result<Option<ListNode>> {
    let row = sqlx::query(&format!(
        "select {NODE_COLUMNS} from reader.list_node n where n.list_id = $1 and n.id = $2"
    ))
    .bind(list.as_uuid())
    .bind(node.as_uuid())
    .fetch_optional(&mut *transaction)
    .await?;
    row.as_ref().map(node_from_row).transpose()
}

async fn work_in_list(transaction: &mut PgConnection, list: ListId, work: WorkId) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "select exists (select 1 from reader.list_node \
         where list_id = $1 and kind = 'work' and work_id = $2)",
    )
    .bind(list.as_uuid())
    .bind(work.as_uuid())
    .fetch_one(&mut *transaction)
    .await?)
}

async fn next_position(transaction: &mut PgConnection, list: ListId) -> Result<i32> {
    Ok(sqlx::query_scalar(
        "select coalesce(max(position), -1) + 1 from reader.list_node where list_id = $1",
    )
    .bind(list.as_uuid())
    .fetch_one(&mut *transaction)
    .await?)
}

fn json_array_within(value: &Value, max: usize) -> bool {
    value.as_array().is_some_and(|items| items.len() <= max)
}

/// Insert one node (and a Work's `covers` edges) under the list lock.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]
pub(crate) async fn insert_new_node(
    transaction: &mut PgConnection,
    list: ListId,
    new: NewNode,
) -> Result<ListNode> {
    if !json_array_within(&new.context_links, CONTEXT_LINKS_MAX) {
        return Err(DbError::Conflict("invalid_context_links".into()));
    }
    let (works_and_claims, topics): (i64, i64) = sqlx::query_as(
        "select count(*) filter (where kind in ('work', 'claim')), \
                count(*) filter (where kind = 'topic') \
         from reader.list_node where list_id = $1",
    )
    .bind(list.as_uuid())
    .fetch_one(&mut *transaction)
    .await?;

    let mut work_id = None;
    let mut label = None;
    let mut summary = None;
    let mut claim = None;
    let mut claim_state = None;
    let mut candidates = Value::Array(Vec::new());
    let mut via_work_id = None;
    let mut path: Option<Vec<Uuid>> = None;
    let mut cover_topics = Vec::new();
    match new.content {
        NewNodeContent::Work {
            work_id: work,
            via,
            topics: cover,
        } => {
            if works_and_claims as usize >= LIST_WORKS_MAX {
                return Err(conflict(ListError::Full));
            }
            if work_in_list(transaction, list, work).await? {
                return Err(conflict(ListError::ItemExists));
            }
            if let Some(via) = via {
                path = Some(path_via(transaction, list, via, work).await?);
                via_work_id = Some(via);
            }
            work_id = Some(work);
            cover_topics = cover;
        }
        NewNodeContent::Topic {
            label: topic,
            summary: text,
        } => {
            if topics as usize >= LIST_TOPICS_MAX {
                return Err(conflict(ListError::TopicsFull));
            }
            let taken: bool = sqlx::query_scalar(
                "select exists (select 1 from reader.list_node \
                 where list_id = $1 and kind = 'topic' and lower(label) = lower($2))",
            )
            .bind(list.as_uuid())
            .bind(topic.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            if taken {
                return Err(conflict(ListError::ItemExists));
            }
            label = Some(topic.as_str().to_owned());
            summary = text.filter(|text| !text.is_empty());
        }
        NewNodeContent::Claim {
            claim: value,
            state,
            candidates: found,
        } => {
            if works_and_claims as usize >= LIST_WORKS_MAX {
                return Err(conflict(ListError::Full));
            }
            if !json_array_within(&found, CLAIM_CANDIDATES_MAX) || !value.is_object() {
                return Err(DbError::Conflict("invalid_claim".into()));
            }
            claim = Some(value);
            claim_state = Some(state);
            candidates = found;
        }
    }

    let kind = match (work_id, &label) {
        (Some(_), _) => NodeKind::Work,
        (None, Some(_)) => NodeKind::Topic,
        (None, None) => NodeKind::Claim,
    };
    let position = next_position(transaction, list).await?;
    let row = sqlx::query(&format!(
        "insert into reader.list_node as n \
             (list_id, kind, work_id, label, summary, claim, claim_state, candidates, position, \
              note, rationale, context_links, source, review, via_work_id, path_work_ids, \
              generation_id) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) \
         returning {NODE_COLUMNS}"
    ))
    .bind(list.as_uuid())
    .bind(kind.as_str())
    .bind(work_id.map(WorkId::as_uuid))
    .bind(label)
    .bind(summary)
    .bind(claim)
    .bind(claim_state.map(lysilogy_core::ClaimState::as_str))
    .bind(candidates)
    .bind(position)
    .bind(new.note.filter(|note| !note.is_empty()))
    .bind(new.rationale.filter(|text| !text.is_empty()))
    .bind(new.context_links)
    .bind(new.source.as_str())
    .bind(new.review.as_str())
    .bind(via_work_id.map(WorkId::as_uuid))
    .bind(path)
    .bind(new.generation_id.map(GenerationId::as_uuid))
    .fetch_one(&mut *transaction)
    .await?;
    let node = node_from_row(&row)?;

    for topic in cover_topics {
        let edge = NewEdge {
            from: topic,
            to: node.id,
            kind: EdgeKind::Covers,
            source: EdgeSource::User,
            review: new.review,
        };
        if let Err(rejection) = insert_edge(transaction, list, edge).await? {
            return Err(conflict(rejection.list_error()));
        }
    }
    Ok(node)
}

/// The citation path ending at `work`, extending the path of member `via`.
async fn path_via(
    transaction: &mut PgConnection,
    list: ListId,
    via: WorkId,
    work: WorkId,
) -> Result<Vec<Uuid>> {
    let via_path: Option<Option<Vec<Uuid>>> = sqlx::query_scalar(
        "select path_work_ids from reader.list_node \
         where list_id = $1 and kind = 'work' and work_id = $2",
    )
    .bind(list.as_uuid())
    .bind(via.as_uuid())
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(via_path) = via_path else {
        return Err(conflict(ListError::ViaNotInList));
    };
    let cites: bool = sqlx::query_scalar(
        "select exists (select 1 from graph.citation_assertion \
         where citing_work_id = $1 and cited_work_id = $2 \
           and status in ('asserted', 'accepted'))",
    )
    .bind(via.as_uuid())
    .bind(work.as_uuid())
    .fetch_one(&mut *transaction)
    .await?;
    if !cites {
        return Err(conflict(ListError::NoCitationEdge));
    }
    let mut path = via_path.unwrap_or_else(|| vec![via.as_uuid()]);
    if path.len() + 1 > RAMP_PATH_MAX {
        return Err(conflict(ListError::TooDeep));
    }
    path.push(work.as_uuid());
    Ok(path)
}

/// Validate and store an edge under the list lock.
pub(crate) async fn insert_edge(
    transaction: &mut PgConnection,
    list: ListId,
    new: NewEdge,
) -> Result<std::result::Result<StoredEdge, EdgeRejection>> {
    let kind_rows = sqlx::query("select id, kind from reader.list_node where list_id = $1")
        .bind(list.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
    let mut kinds: HashMap<NodeId, NodeKind> = HashMap::with_capacity(kind_rows.len());
    for row in &kind_rows {
        let kind: String = row.try_get("kind")?;
        kinds.insert(NodeId::from_uuid(row.try_get("id")?), kind.parse()?);
    }
    let existing_rows = sqlx::query(&format!(
        "select {EDGE_COLUMNS} from reader.list_edge e where e.list_id = $1"
    ))
    .bind(list.as_uuid())
    .fetch_all(&mut *transaction)
    .await?;
    let existing = existing_rows
        .iter()
        .map(edge_from_row)
        .collect::<Result<Vec<_>>>()?;
    let stored: Vec<ListEdge> = existing
        .iter()
        .map(|edge| ListEdge {
            from: edge.from,
            to: edge.to,
            kind: edge.kind,
        })
        .collect();
    let candidate = ListEdge {
        from: new.from,
        to: new.to,
        kind: new.kind,
    };
    if let Err(rejection) = validate_edge(&kinds, &stored, candidate) {
        return Ok(Err(rejection));
    }
    if let Some(found) = existing
        .into_iter()
        .find(|edge| edge.from == new.from && edge.to == new.to && edge.kind == new.kind)
    {
        return Ok(Ok(found));
    }
    if stored.len() >= LIST_EDGES_MAX {
        return Err(conflict(ListError::EdgesFull));
    }
    let row = sqlx::query(&format!(
        "insert into reader.list_edge as e (list_id, from_node, to_node, kind, source, review) \
         values ($1, $2, $3, $4, $5, $6) returning {EDGE_COLUMNS}"
    ))
    .bind(list.as_uuid())
    .bind(new.from.as_uuid())
    .bind(new.to.as_uuid())
    .bind(new.kind.as_str())
    .bind(new.source.as_str())
    .bind(new.review.as_str())
    .fetch_one(&mut *transaction)
    .await?;
    edge_from_row(&row).map(Ok)
}

fn list_from_row(row: &PgRow) -> Result<List> {
    let origin: String = row.try_get("origin")?;
    Ok(List {
        id: ListId::from_uuid(row.try_get("id")?),
        workspace_id: WorkspaceId::from_uuid(row.try_get("workspace_id")?),
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        origin: origin.parse()?,
        seed_work_id: row
            .try_get::<Option<Uuid>, _>("seed_work_id")?
            .map(WorkId::from_uuid),
        goal: row.try_get("goal")?,
        archived_at: row.try_get("archived_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn node_from_row(row: &PgRow) -> Result<ListNode> {
    let kind: String = row.try_get("kind")?;
    let claim_state: Option<String> = row.try_get("claim_state")?;
    let source: String = row.try_get("source")?;
    let review: String = row.try_get("review")?;
    let path: Option<Vec<Uuid>> = row.try_get("path_work_ids")?;
    Ok(ListNode {
        id: NodeId::from_uuid(row.try_get("id")?),
        list_id: ListId::from_uuid(row.try_get("list_id")?),
        kind: kind.parse()?,
        work_id: row
            .try_get::<Option<Uuid>, _>("work_id")?
            .map(WorkId::from_uuid),
        label: row.try_get("label")?,
        summary: row.try_get("summary")?,
        claim: row.try_get("claim")?,
        claim_state: claim_state.map(|text| text.parse()).transpose()?,
        candidates: row.try_get("candidates")?,
        position: row.try_get("position")?,
        note: row.try_get("note")?,
        rationale: row.try_get("rationale")?,
        context_links: row.try_get("context_links")?,
        source: source.parse()?,
        review: review.parse()?,
        via_work_id: row
            .try_get::<Option<Uuid>, _>("via_work_id")?
            .map(WorkId::from_uuid),
        path: path.map(|ids| ids.into_iter().map(WorkId::from_uuid).collect()),
        generation_id: row
            .try_get::<Option<Uuid>, _>("generation_id")?
            .map(GenerationId::from_uuid),
        added_at: row.try_get("added_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn edge_from_row(row: &PgRow) -> Result<StoredEdge> {
    let kind: String = row.try_get("kind")?;
    let source: String = row.try_get("source")?;
    let review: String = row.try_get("review")?;
    Ok(StoredEdge {
        list_id: ListId::from_uuid(row.try_get("list_id")?),
        from: NodeId::from_uuid(row.try_get("from_node")?),
        to: NodeId::from_uuid(row.try_get("to_node")?),
        kind: kind.parse()?,
        source: source.parse()?,
        review: review.parse()?,
        created_at: row.try_get("created_at")?,
    })
}

#[cfg(test)]
#[allow(clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::reading_state::ReadingStateRepository;
    use crate::test_support;
    use crate::work::WorkRepository;
    use lysilogy_core::{ClaimState, ReadingStatus};
    use serde_json::json;

    struct Fixture {
        pool: PgPool,
        repo: ListRepository,
        workspace: WorkspaceId,
    }

    async fn fixture() -> Option<Fixture> {
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return None;
        };
        let workspace = test_support::new_workspace(&pool).await;
        Some(Fixture {
            repo: ListRepository::new(pool.clone()),
            pool,
            workspace,
        })
    }

    fn title(text: &str) -> ListTitle {
        ListTitle::parse(text).expect("title")
    }

    fn label(text: &str) -> TopicLabel {
        TopicLabel::parse(text).expect("label")
    }

    fn conflict_code<T: std::fmt::Debug>(result: Result<T>) -> String {
        match result {
            Err(DbError::Conflict(code)) => code,
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    fn new_edge(from: NodeId, to: NodeId, kind: EdgeKind) -> NewEdge {
        NewEdge {
            from,
            to,
            kind,
            source: EdgeSource::User,
            review: ListReview::Accepted,
        }
    }

    fn positions(nodes: &[ListNode]) -> Vec<i32> {
        nodes.iter().map(|node| node.position).collect()
    }

    fn node_ids(nodes: &[ListNode]) -> Vec<NodeId> {
        nodes.iter().map(|node| node.id).collect()
    }

    fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
        items.sort();
        items
    }

    fn summary_ids(summaries: &[ListSummary]) -> Vec<ListId> {
        summaries.iter().map(|summary| summary.list.id).collect()
    }

    /// A work node carrying a topic filing.
    fn work_in_topics(work: WorkId, topics: Vec<NodeId>) -> NewNode {
        NewNode {
            content: NewNodeContent::Work {
                work_id: work,
                via: None,
                topics,
            },
            ..NewNode::work(work)
        }
    }

    impl Fixture {
        async fn list(&self, name: &str) -> List {
            self.repo
                .create(self.workspace, NewList::manual(title(name)))
                .await
                .expect("create")
        }

        async fn work(&self) -> WorkId {
            test_support::new_work(&self.pool).await
        }

        async fn add_work(&self, list: ListId) -> ListNode {
            let work = self.work().await;
            self.repo
                .add_node(list, NewNode::work(work))
                .await
                .expect("add work")
        }

        async fn add_topic(&self, list: ListId, name: &str) -> ListNode {
            self.repo
                .add_node(list, NewNode::topic(label(name)))
                .await
                .expect("add topic")
        }

        async fn archive(&self, list: ListId) {
            self.repo
                .update(
                    self.workspace,
                    list,
                    ListPatch {
                        archived: Some(true),
                        ..ListPatch::default()
                    },
                )
                .await
                .expect("archive");
        }

        async fn edge(&self, list: ListId, from: NodeId, to: NodeId, kind: EdgeKind) {
            self.repo
                .add_edge(list, new_edge(from, to, kind))
                .await
                .expect("db")
                .expect("edge accepted");
        }

        async fn rejection(
            &self,
            list: ListId,
            from: NodeId,
            to: NodeId,
            kind: EdgeKind,
        ) -> EdgeRejection {
            self.repo
                .add_edge(list, new_edge(from, to, kind))
                .await
                .expect("db")
                .expect_err("rejected")
        }

        /// Insert `count` claim nodes directly, to reach limits without `count` round trips.
        async fn bulk_claims(&self, list: ListId, count: i32) {
            let start: i32 = sqlx::query_scalar(
                "select coalesce(max(position), -1) + 1 from reader.list_node where list_id = $1",
            )
            .bind(list.as_uuid())
            .fetch_one(&self.pool)
            .await
            .expect("start");
            sqlx::query(
                "insert into reader.list_node (list_id, kind, claim, claim_state, position, source) \
                 select $1, 'claim', '{}'::jsonb, 'unverified', g, 'user' \
                 from generate_series($2::int, $3::int) g",
            )
            .bind(list.as_uuid())
            .bind(start)
            .bind(start + count - 1)
            .execute(&self.pool)
            .await
            .expect("bulk claims");
        }
    }

    // ---- lists ----

    #[tokio::test]
    async fn should_create_an_empty_list_with_origin_and_seed_when_creating() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let seed = fixture.work().await;
        let list = fixture
            .repo
            .create(
                fixture.workspace,
                NewList {
                    title: title("  Reward   modelling "),
                    description: Some("why".into()),
                    origin: ListOrigin::Generated,
                    seed_work_id: Some(seed),
                    goal: Some("learn RLHF".into()),
                },
            )
            .await
            .expect("create");
        assert_eq!(list.title, "Reward modelling");
        assert_eq!(list.origin, ListOrigin::Generated);
        assert_eq!(list.seed_work_id, Some(seed));
        assert_eq!(list.goal.as_deref(), Some("learn RLHF"));
        assert_eq!(list.archived_at, None);
        assert!(fixture.repo.nodes(list.id).await.expect("nodes").is_empty());
        let fetched = fixture
            .repo
            .get(fixture.workspace, list.id)
            .await
            .expect("get");
        assert_eq!(fetched, Some(list));
    }

    #[tokio::test]
    async fn should_summarize_lists_by_update_time_with_status_counts() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let older = fixture.list("Older").await;
        let newer = fixture.list("Newer").await;
        let reading = fixture.add_work(older.id).await;
        let read = fixture.add_work(older.id).await;
        fixture.add_work(older.id).await;
        fixture.add_topic(older.id, "Topic").await;
        fixture
            .repo
            .add_node(
                older.id,
                NewNode::claim(json!({"title": "Maybe real"}), ClaimState::Unverified),
            )
            .await
            .expect("claim");
        let mut proposed = NewNode::work(fixture.work().await);
        proposed.review = ListReview::Proposed;
        fixture
            .repo
            .add_node(older.id, proposed)
            .await
            .expect("proposed");
        let states = ReadingStateRepository::new(fixture.pool.clone());
        states
            .set(
                fixture.workspace,
                reading.work_id.expect("work"),
                ReadingStatus::Reading,
            )
            .await
            .expect("state");
        states
            .set(
                fixture.workspace,
                read.work_id.expect("work"),
                ReadingStatus::Read,
            )
            .await
            .expect("state");

        let summaries = fixture
            .repo
            .list(fixture.workspace, None, ArchiveScope::Active)
            .await
            .expect("list");
        assert_eq!(
            summary_ids(&summaries),
            vec![older.id, newer.id],
            "older was touched last"
        );
        let counts = &summaries[0];
        assert_eq!(counts.item_count, 5, "4 works + 1 claim, no topic");
        assert_eq!(counts.topic_count, 1);
        assert_eq!(counts.claim_count, 1);
        assert_eq!(counts.proposed_count, 1);
        assert_eq!((counts.unread, counts.reading, counts.read), (2, 1, 1));
        assert_eq!(summaries[1].item_count, 0);
    }

    #[tokio::test]
    async fn should_filter_lists_by_work_and_archive_scope_when_listing() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let holding = fixture.list("Holding").await;
        let archived = fixture.list("Archived holding").await;
        let empty = fixture.list("Empty").await;
        let shared = fixture.work().await;
        for list in [holding.id, archived.id] {
            fixture
                .repo
                .add_node(list, NewNode::work(shared))
                .await
                .expect("add");
        }
        fixture.archive(archived.id).await;

        let active = fixture
            .repo
            .list(fixture.workspace, Some(shared), ArchiveScope::Active)
            .await
            .expect("active");
        assert_eq!(summary_ids(&active), vec![holding.id]);
        let only_archived = fixture
            .repo
            .list(fixture.workspace, None, ArchiveScope::Archived)
            .await
            .expect("archived");
        assert_eq!(summary_ids(&only_archived), vec![archived.id]);
        let all = fixture
            .repo
            .list(fixture.workspace, None, ArchiveScope::All)
            .await
            .expect("all");
        assert_eq!(
            sorted(summary_ids(&all)),
            sorted(vec![holding.id, archived.id, empty.id])
        );
        let none = fixture
            .repo
            .list(fixture.workspace, Some(WorkId::new()), ArchiveScope::All)
            .await
            .expect("none");
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn should_patch_archive_and_description_without_losing_other_fields_when_updating() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Original").await;
        let described = fixture
            .repo
            .update(
                fixture.workspace,
                list.id,
                ListPatch {
                    description: Some(Some("a description".into())),
                    ..ListPatch::default()
                },
            )
            .await
            .expect("update")
            .expect("list");
        assert_eq!(described.title, "Original");
        assert_eq!(described.description.as_deref(), Some("a description"));

        let archived = fixture
            .repo
            .update(
                fixture.workspace,
                list.id,
                ListPatch {
                    archived: Some(true),
                    title: Some(title("Renamed")),
                    ..ListPatch::default()
                },
            )
            .await
            .expect("update")
            .expect("list");
        let stamp = archived.archived_at.expect("archived");
        assert_eq!(archived.title, "Renamed");
        assert_eq!(archived.description.as_deref(), Some("a description"));

        // Archiving again keeps the first stamp; clearing the description works; a patch
        // that sets nothing changes nothing.
        let again = fixture
            .repo
            .update(
                fixture.workspace,
                list.id,
                ListPatch {
                    archived: Some(true),
                    description: Some(None),
                    ..ListPatch::default()
                },
            )
            .await
            .expect("update")
            .expect("list");
        assert_eq!(again.archived_at, Some(stamp));
        assert_eq!(again.description, None);
        let untouched = fixture
            .repo
            .update(fixture.workspace, list.id, ListPatch::default())
            .await
            .expect("update")
            .expect("list");
        assert_eq!(untouched.archived_at, Some(stamp));
        let restored = fixture
            .repo
            .update(
                fixture.workspace,
                list.id,
                ListPatch {
                    archived: Some(false),
                    ..ListPatch::default()
                },
            )
            .await
            .expect("update")
            .expect("list");
        assert_eq!(restored.archived_at, None);
    }

    #[tokio::test]
    async fn should_return_none_when_list_belongs_to_another_workspace() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Mine").await;
        let other = test_support::new_workspace(&fixture.pool).await;
        assert_eq!(fixture.repo.get(other, list.id).await.expect("get"), None);
        assert_eq!(
            fixture
                .repo
                .update(other, list.id, ListPatch::default())
                .await
                .expect("update"),
            None
        );
        assert!(!fixture.repo.delete(other, list.id).await.expect("delete"));
        assert!(
            fixture
                .repo
                .list(other, None, ArchiveScope::All)
                .await
                .expect("list")
                .is_empty()
        );
        assert!(
            fixture
                .repo
                .search_by_title(other, "Mine", 10)
                .await
                .expect("search")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn should_name_the_owning_workspace_when_asked_for_a_lists_workspace() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Owned").await;
        assert_eq!(
            fixture.repo.workspace_of(list.id).await.expect("owner"),
            Some(fixture.workspace)
        );
        assert_eq!(
            fixture
                .repo
                .workspace_of(ListId::new())
                .await
                .expect("owner"),
            None
        );
    }

    #[tokio::test]
    async fn should_cascade_nodes_and_edges_when_deleting_a_list() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Doomed").await;
        let topic = fixture.add_topic(list.id, "T").await;
        let work = fixture.add_work(list.id).await;
        fixture
            .edge(list.id, topic.id, work.id, EdgeKind::Covers)
            .await;
        assert!(
            fixture
                .repo
                .delete(fixture.workspace, list.id)
                .await
                .expect("delete")
        );
        assert!(
            !fixture
                .repo
                .delete(fixture.workspace, list.id)
                .await
                .expect("again")
        );
        assert!(fixture.repo.nodes(list.id).await.expect("nodes").is_empty());
        assert!(fixture.repo.edges(list.id).await.expect("edges").is_empty());
        // The Work itself survives.
        let kept = WorkRepository::new(fixture.pool.clone())
            .get(work.work_id.expect("work"))
            .await
            .expect("get");
        assert!(kept.is_some());
    }

    #[tokio::test]
    async fn should_find_lists_by_title_words_with_prefix_matches_first_when_searching() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let contains = fixture.list("Intro to RLHF basics").await;
        let prefix = fixture.list("RLHF reading").await;
        let literal = fixture.list("100% reward_model").await;
        fixture.list("Graph theory").await;
        let accents = fixture.list("Théorie des jeux").await;
        let archived = fixture.list("RLHF archive").await;
        fixture.archive(archived.id).await;

        let search = |query: &'static str| {
            let repo = fixture.repo.clone();
            let workspace = fixture.workspace;
            async move {
                repo.search_by_title(workspace, query, 10)
                    .await
                    .expect("search")
                    .into_iter()
                    .map(|list| list.id)
                    .collect::<Vec<_>>()
            }
        };
        assert_eq!(search("rlhf").await, vec![prefix.id, contains.id]);
        assert_eq!(search("basics RLHF").await, vec![contains.id]);
        assert_eq!(search("100%").await, vec![literal.id]);
        assert_eq!(search("%").await, vec![literal.id], "percent is literal");
        assert_eq!(search("_").await, vec![literal.id], "underscore is literal");
        // Case folding of non-ASCII letters follows the database locale, so match the case.
        assert_eq!(search("théorie").await, vec![accents.id]);
        assert!(search("nothing like it").await.is_empty());
        assert_eq!(
            search("   ").await.len(),
            5,
            "blank lists recent active lists"
        );
        let limited = fixture
            .repo
            .search_by_title(fixture.workspace, "", 2)
            .await
            .expect("limited");
        assert_eq!(limited.len(), 2);
    }

    // ---- nodes ----

    #[tokio::test]
    async fn should_append_nodes_at_increasing_positions_when_adding() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Order").await;
        fixture.add_work(list.id).await;
        fixture.add_topic(list.id, "Topic").await;
        let claim = fixture
            .repo
            .add_node(
                list.id,
                NewNode::claim(json!({"title": "A claim"}), ClaimState::Ambiguous),
            )
            .await
            .expect("claim");
        assert_eq!(claim.kind, NodeKind::Claim);
        assert_eq!(claim.claim_state, Some(ClaimState::Ambiguous));
        assert_eq!(claim.work_id, None);
        assert_eq!(claim.candidates, json!([]));
        let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
        assert_eq!(positions(&nodes), vec![0, 1, 2]);
        assert_eq!(
            nodes.iter().map(|node| node.kind).collect::<Vec<_>>(),
            vec![NodeKind::Work, NodeKind::Topic, NodeKind::Claim]
        );
        let missing = fixture
            .repo
            .add_node(ListId::new(), NewNode::work(WorkId::new()))
            .await;
        assert!(matches!(missing, Err(ref error) if error.is_not_found()));
    }

    #[tokio::test]
    async fn should_build_path_from_via_when_adding_a_work() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Walk").await;
        let seed = fixture.work().await;
        let child = fixture.work().await;
        let grandchild = fixture.work().await;
        test_support::cite(&fixture.pool, seed, child).await;
        test_support::cite(&fixture.pool, child, grandchild).await;

        let root = fixture
            .repo
            .add_node(list.id, NewNode::work(seed))
            .await
            .expect("seed");
        assert_eq!(root.path, None, "a node nobody walked to has no path");
        let first = fixture
            .repo
            .add_node(list.id, NewNode::work_via(child, seed))
            .await
            .expect("child");
        assert_eq!(first.path, Some(vec![seed, child]));
        assert_eq!(first.via_work_id, Some(seed));
        let second = fixture
            .repo
            .add_node(list.id, NewNode::work_via(grandchild, child))
            .await
            .expect("grandchild");
        assert_eq!(second.path, Some(vec![seed, child, grandchild]));
        assert_eq!(second.position, 2);

        let mut noted = NewNode::work(fixture.work().await);
        noted.note = Some(String::new());
        noted.rationale = Some("because".into());
        noted.source = NodeSource::Agent;
        let stored = fixture.repo.add_node(list.id, noted).await.expect("noted");
        assert_eq!(stored.note, None, "an empty note stores as none");
        assert_eq!(stored.rationale.as_deref(), Some("because"));
        assert_eq!(stored.source, NodeSource::Agent);
    }

    #[tokio::test]
    async fn should_conflict_when_a_work_is_added_twice_or_via_is_invalid() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Conflicts").await;
        let seed = fixture.work().await;
        let child = fixture.work().await;
        let stranger = fixture.work().await;
        test_support::cite(&fixture.pool, seed, child).await;
        fixture
            .repo
            .add_node(list.id, NewNode::work(seed))
            .await
            .expect("seed");

        assert_eq!(
            conflict_code(fixture.repo.add_node(list.id, NewNode::work(seed)).await),
            "item_exists"
        );
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(list.id, NewNode::work_via(child, stranger))
                    .await
            ),
            "via_not_in_list"
        );
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(list.id, NewNode::work_via(stranger, seed))
                    .await
            ),
            "no_citation_edge"
        );
        // The reverse direction does not count: child does not cite seed.
        fixture
            .repo
            .add_node(list.id, NewNode::work(child))
            .await
            .expect("child");
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(list.id, NewNode::work_via(seed, child))
                    .await
            ),
            "item_exists"
        );
        assert_eq!(fixture.repo.nodes(list.id).await.expect("nodes").len(), 2);
    }

    #[tokio::test]
    async fn should_conflict_when_path_would_exceed_nine_works() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Deep").await;
        let mut chain = vec![fixture.work().await];
        for _ in 0..9 {
            let next = fixture.work().await;
            test_support::cite(&fixture.pool, *chain.last().expect("last"), next).await;
            chain.push(next);
        }
        fixture
            .repo
            .add_node(list.id, NewNode::work(chain[0]))
            .await
            .expect("root");
        for pair in chain.windows(2).take(8) {
            fixture
                .repo
                .add_node(list.id, NewNode::work_via(pair[1], pair[0]))
                .await
                .expect("within depth");
        }
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(list.id, NewNode::work_via(chain[9], chain[8]))
                    .await
            ),
            "ramp_too_deep"
        );
    }

    #[tokio::test]
    async fn should_conflict_when_list_holds_the_work_limit() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Full").await;
        fixture.bulk_claims(list.id, 500).await;
        let work = fixture.work().await;
        assert_eq!(
            conflict_code(fixture.repo.add_node(list.id, NewNode::work(work)).await),
            "list_full"
        );
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(list.id, NewNode::claim(json!({}), ClaimState::Pending))
                    .await
            ),
            "list_full"
        );
        // Topics have their own budget.
        fixture.add_topic(list.id, "Still fits").await;
    }

    #[tokio::test]
    async fn should_conflict_when_list_holds_the_topic_limit() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Many topics").await;
        sqlx::query(
            "insert into reader.list_node (list_id, kind, label, position, source) \
             select $1, 'topic', 'topic ' || g, g, 'user' from generate_series(0, 199) g",
        )
        .bind(list.id.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("topics");
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(list.id, NewNode::topic(label("one too many")))
                    .await
            ),
            "topics_full"
        );
        fixture.add_work(list.id).await;
    }

    #[tokio::test]
    async fn should_refuse_a_topic_label_that_differs_only_in_case_when_adding() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Topics").await;
        let other = fixture.list("Other list").await;
        fixture.add_topic(list.id, "Reward Modelling").await;
        for clash in ["reward modelling", "REWARD   MODELLING"] {
            assert_eq!(
                conflict_code(
                    fixture
                        .repo
                        .add_node(list.id, NewNode::topic(label(clash)))
                        .await
                ),
                "item_exists",
                "{clash}"
            );
        }
        fixture.add_topic(list.id, "Réward").await;
        // The same label in another list is fine.
        fixture.add_topic(other.id, "Reward Modelling").await;
    }

    #[tokio::test]
    async fn should_add_covers_edges_when_a_work_is_filed_under_topics() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Filed").await;
        let first = fixture.add_topic(list.id, "First").await;
        let second = fixture.add_topic(list.id, "Second").await;
        let work = fixture.work().await;
        let node = fixture
            .repo
            .add_node(list.id, work_in_topics(work, vec![first.id, second.id]))
            .await
            .expect("add");
        let edges = fixture.repo.edges(list.id).await.expect("edges");
        assert_eq!(edges.len(), 2);
        assert!(
            edges
                .iter()
                .all(|edge| edge.to == node.id && edge.kind == EdgeKind::Covers)
        );

        // A Work node is not a topic: nothing is added, not even the Work.
        let other_work = fixture.work().await;
        let result = fixture
            .repo
            .add_node(list.id, work_in_topics(other_work, vec![node.id]))
            .await;
        assert_eq!(conflict_code(result), "invalid_edge_kind");
        assert_eq!(fixture.repo.nodes(list.id).await.expect("nodes").len(), 3);
    }

    #[tokio::test]
    async fn should_refuse_malformed_claims_and_context_links_when_adding() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Malformed").await;
        let mut too_many_candidates = NewNode::claim(json!({"title": "x"}), ClaimState::Ambiguous);
        too_many_candidates.content = NewNodeContent::Claim {
            claim: json!({"title": "x"}),
            state: ClaimState::Ambiguous,
            candidates: json!([1, 2, 3, 4, 5, 6]),
        };
        assert_eq!(
            conflict_code(fixture.repo.add_node(list.id, too_many_candidates).await),
            "invalid_claim"
        );
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_node(
                        list.id,
                        NewNode::claim(json!("a string"), ClaimState::Pending)
                    )
                    .await
            ),
            "invalid_claim"
        );
        let mut links = NewNode::work(fixture.work().await);
        links.context_links = json!([1, 2, 3, 4, 5, 6]);
        assert_eq!(
            conflict_code(fixture.repo.add_node(list.id, links).await),
            "invalid_context_links"
        );
        let mut not_array = NewNode::work(fixture.work().await);
        not_array.context_links = json!({"work_id": "x"});
        assert_eq!(
            conflict_code(fixture.repo.add_node(list.id, not_array).await),
            "invalid_context_links"
        );
        let mut five = NewNode::work(fixture.work().await);
        five.context_links = json!([1, 2, 3, 4, 5]);
        fixture
            .repo
            .add_node(list.id, five)
            .await
            .expect("five links fit");
        assert_eq!(fixture.repo.nodes(list.id).await.expect("nodes").len(), 1);
    }

    #[tokio::test]
    async fn should_add_nothing_when_any_node_of_a_batch_is_refused() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Batch").await;
        let existing = fixture.work().await;
        fixture
            .repo
            .add_node(list.id, NewNode::work(existing))
            .await
            .expect("existing");
        let fresh = fixture.work().await;
        let result = fixture
            .repo
            .add_nodes(
                list.id,
                vec![
                    NewNode::work(fresh),
                    NewNode::topic(label("Would be added")),
                    NewNode::work(existing),
                ],
            )
            .await;
        assert_eq!(conflict_code(result), "item_exists");
        let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
        assert_eq!(nodes.len(), 1, "the first two additions were undone");

        // A duplicate inside the batch itself is refused too.
        let result = fixture
            .repo
            .add_nodes(list.id, vec![NewNode::work(fresh), NewNode::work(fresh)])
            .await;
        assert_eq!(conflict_code(result), "item_exists");
        assert_eq!(fixture.repo.nodes(list.id).await.expect("nodes").len(), 1);

        let added = fixture
            .repo
            .add_nodes(
                list.id,
                vec![NewNode::work(fresh), NewNode::topic(label("Kept"))],
            )
            .await
            .expect("batch");
        assert_eq!(positions(&added), vec![1, 2]);
        assert!(
            fixture
                .repo
                .add_nodes(list.id, Vec::new())
                .await
                .expect("empty batch")
                .is_empty()
        );
        let missing = fixture
            .repo
            .add_nodes(ListId::new(), vec![NewNode::work(fresh)])
            .await;
        assert!(matches!(missing, Err(ref error) if error.is_not_found()));
    }

    #[tokio::test]
    async fn should_patch_notes_labels_and_review_when_updating_a_node() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Patch").await;
        let mut proposed = NewNode::work(fixture.work().await);
        proposed.review = ListReview::Proposed;
        proposed.note = Some("first".into());
        let work = fixture
            .repo
            .add_node(list.id, proposed)
            .await
            .expect("work");
        let topic = fixture.add_topic(list.id, "Alpha").await;
        let beta = fixture.add_topic(list.id, "Beta").await;
        let patch = |node: NodeId, patch: NodePatch| {
            let repo = fixture.repo.clone();
            async move { repo.update_node(list.id, node, patch).await }
        };

        let accepted = patch(
            work.id,
            NodePatch {
                accept: true,
                ..NodePatch::default()
            },
        )
        .await
        .expect("patch")
        .expect("node");
        assert_eq!(accepted.review, ListReview::Accepted);
        assert_eq!(accepted.note.as_deref(), Some("first"), "other fields stay");
        let cleared = patch(
            work.id,
            NodePatch {
                note: Some(None),
                ..NodePatch::default()
            },
        )
        .await
        .expect("patch")
        .expect("node");
        assert_eq!(cleared.note, None);
        let blank = patch(
            work.id,
            NodePatch {
                note: Some(Some(String::new())),
                ..NodePatch::default()
            },
        )
        .await
        .expect("patch")
        .expect("node");
        assert_eq!(blank.note, None);

        let relabeled = patch(
            topic.id,
            NodePatch {
                label: Some(label("Alpha prime")),
                summary: Some(Some("what it covers".into())),
                ..NodePatch::default()
            },
        )
        .await
        .expect("patch")
        .expect("node");
        assert_eq!(relabeled.label.as_deref(), Some("Alpha prime"));
        assert_eq!(relabeled.summary.as_deref(), Some("what it covers"));
        // A topic may change the case of its own label; a sibling's label is taken.
        patch(
            topic.id,
            NodePatch {
                label: Some(label("ALPHA PRIME")),
                ..NodePatch::default()
            },
        )
        .await
        .expect("own label")
        .expect("node");
        let clash = patch(
            topic.id,
            NodePatch {
                label: Some(label("beta")),
                ..NodePatch::default()
            },
        )
        .await;
        assert_eq!(conflict_code(clash), "item_exists");
        let wrong_kind = patch(
            work.id,
            NodePatch {
                label: Some(label("Nope")),
                ..NodePatch::default()
            },
        )
        .await;
        assert_eq!(conflict_code(wrong_kind), "invalid_node_kind");
        let too_long = patch(
            beta.id,
            NodePatch {
                summary: Some(Some("x".repeat(2001))),
                ..NodePatch::default()
            },
        )
        .await;
        assert!(too_long.is_err(), "the column check refuses a long summary");
        let missing = patch(NodeId::new(), NodePatch::default())
            .await
            .expect("patch");
        assert_eq!(missing, None);
        let other_list = fixture.list("Other").await;
        let foreign = fixture
            .repo
            .update_node(other_list.id, work.id, NodePatch::default())
            .await
            .expect("patch");
        assert_eq!(foreign, None, "a node of another list is not found");
    }

    #[tokio::test]
    async fn should_turn_a_claim_into_a_work_node_in_place_when_resolving() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Resolve").await;
        let present = fixture.add_work(list.id).await;
        let mut claim = NewNode::claim(json!({"title": "Maybe"}), ClaimState::Ambiguous);
        claim.note = Some("keep me".into());
        claim.rationale = Some("because".into());
        let claim = fixture.repo.add_node(list.id, claim).await.expect("claim");
        let topic = fixture.add_topic(list.id, "Topic").await;
        let work = fixture.work().await;

        assert_eq!(
            conflict_code(fixture.repo.resolve_claim(list.id, topic.id, work).await),
            "not_a_claim"
        );
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .resolve_claim(list.id, claim.id, present.work_id.expect("work"))
                    .await
            ),
            "item_exists"
        );
        let resolved = fixture
            .repo
            .resolve_claim(list.id, claim.id, work)
            .await
            .expect("resolve")
            .expect("node");
        assert_eq!(resolved.id, claim.id);
        assert_eq!(resolved.kind, NodeKind::Work);
        assert_eq!(resolved.work_id, Some(work));
        assert_eq!(resolved.claim, None);
        assert_eq!(resolved.claim_state, None);
        assert_eq!(resolved.position, claim.position);
        assert_eq!(resolved.note.as_deref(), Some("keep me"));
        assert_eq!(resolved.rationale.as_deref(), Some("because"));
        assert_eq!(
            fixture
                .repo
                .resolve_claim(list.id, NodeId::new(), work)
                .await
                .expect("resolve"),
            None
        );
        // Resolved, it is an ordinary member found by Work.
        assert!(
            fixture
                .repo
                .node_for_work(list.id, work)
                .await
                .expect("node")
                .is_some()
        );
    }

    #[tokio::test]
    async fn should_return_an_undo_payload_and_cascade_edges_when_removing_a_node() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Remove").await;
        let topic = fixture.add_topic(list.id, "Topic").await;
        let middle = fixture.add_work(list.id).await;
        let tail = fixture.add_work(list.id).await;
        fixture
            .edge(list.id, topic.id, middle.id, EdgeKind::Covers)
            .await;
        fixture
            .edge(list.id, middle.id, tail.id, EdgeKind::Prerequisite)
            .await;
        fixture
            .edge(list.id, topic.id, tail.id, EdgeKind::Covers)
            .await;

        let removed = fixture
            .repo
            .remove_node(list.id, middle.id)
            .await
            .expect("remove")
            .expect("removed");
        assert_eq!(removed.node, middle);
        assert_eq!(removed.edges.len(), 2, "both edges touching the node");
        assert!(!removed.was_seed);
        let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
        assert_eq!(positions(&nodes), vec![0, 2], "removal leaves a gap");
        let edges = fixture.repo.edges(list.id).await.expect("edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, tail.id);
        assert_eq!(
            fixture
                .repo
                .remove_node(list.id, middle.id)
                .await
                .expect("again"),
            None
        );

        let restored = fixture
            .repo
            .restore_node(list.id, &removed)
            .await
            .expect("restore");
        assert_eq!(restored.id, middle.id);
        assert_eq!(restored.position, 1, "the old slot was free");
        assert_eq!(fixture.repo.edges(list.id).await.expect("edges").len(), 3);
        assert_eq!(
            conflict_code(fixture.repo.restore_node(list.id, &removed).await),
            "item_exists",
            "restoring twice would duplicate the Work"
        );
    }

    #[tokio::test]
    async fn should_clear_the_seed_and_restore_it_when_removing_and_undoing_the_seed_node() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let seed = fixture.work().await;
        let list = fixture
            .repo
            .create(
                fixture.workspace,
                NewList {
                    seed_work_id: Some(seed),
                    origin: ListOrigin::Walk,
                    ..NewList::manual(title("Seeded"))
                },
            )
            .await
            .expect("create");
        let node = fixture
            .repo
            .add_node(list.id, NewNode::work(seed))
            .await
            .expect("seed");
        let other = fixture.add_work(list.id).await;
        let seed_of = |fixture: &Fixture| {
            let repo = fixture.repo.clone();
            let workspace = fixture.workspace;
            async move {
                repo.get(workspace, list.id)
                    .await
                    .expect("get")
                    .expect("list")
                    .seed_work_id
            }
        };

        let removed = fixture
            .repo
            .remove_node(list.id, node.id)
            .await
            .expect("remove")
            .expect("removed");
        assert!(removed.was_seed);
        assert_eq!(seed_of(&fixture).await, None);

        let restored = fixture
            .repo
            .restore_node(list.id, &removed)
            .await
            .expect("restore");
        assert_eq!(restored.work_id, Some(seed));
        assert_eq!(seed_of(&fixture).await, Some(seed));

        // Removing a non-seed node leaves the seed alone.
        let removed_other = fixture
            .repo
            .remove_node(list.id, other.id)
            .await
            .expect("remove")
            .expect("removed");
        assert!(!removed_other.was_seed);
        assert_eq!(seed_of(&fixture).await, Some(seed));
    }

    #[tokio::test]
    async fn should_place_a_restored_node_last_when_its_position_was_taken() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Taken").await;
        let first = fixture.add_work(list.id).await;
        fixture.add_work(list.id).await;
        let removed = fixture
            .repo
            .remove_node(list.id, first.id)
            .await
            .expect("remove")
            .expect("removed");
        let squatter = fixture.add_work(list.id).await;
        assert_eq!(squatter.position, 2, "appends go after the last position");
        // Take the old slot by hand, then undo.
        sqlx::query("update reader.list_node set position = 0 where id = $1")
            .bind(squatter.id.as_uuid())
            .execute(&fixture.pool)
            .await
            .expect("move");
        let restored = fixture
            .repo
            .restore_node(list.id, &removed)
            .await
            .expect("restore");
        assert_eq!(restored.position, 2, "one past the highest position");
    }

    // ---- edges ----

    #[tokio::test]
    async fn should_store_edges_idempotently_and_remove_them_when_asked() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Edges").await;
        let topic = fixture.add_topic(list.id, "Topic").await;
        let work = fixture.add_work(list.id).await;
        let first = fixture
            .repo
            .add_edge(list.id, new_edge(topic.id, work.id, EdgeKind::Covers))
            .await
            .expect("db")
            .expect("edge");
        let again = fixture
            .repo
            .add_edge(
                list.id,
                NewEdge {
                    source: EdgeSource::Agent,
                    ..new_edge(topic.id, work.id, EdgeKind::Covers)
                },
            )
            .await
            .expect("db")
            .expect("edge");
        assert_eq!(again, first, "the stored edge is returned unchanged");
        assert_eq!(fixture.repo.edges(list.id).await.expect("edges").len(), 1);

        assert!(
            fixture
                .repo
                .remove_edge(list.id, topic.id, work.id, EdgeKind::Covers)
                .await
                .expect("remove")
        );
        assert!(
            !fixture
                .repo
                .remove_edge(list.id, topic.id, work.id, EdgeKind::Covers)
                .await
                .expect("again")
        );
        let missing = fixture
            .repo
            .add_edge(ListId::new(), new_edge(topic.id, work.id, EdgeKind::Covers))
            .await;
        assert!(matches!(missing, Err(ref error) if error.is_not_found()));
    }

    #[tokio::test]
    async fn should_reject_edges_with_wrong_kinds_unknown_nodes_and_cycles() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Rejections").await;
        let other_list = fixture.list("Elsewhere").await;
        let topic = fixture.add_topic(list.id, "Topic").await;
        let a = fixture.add_work(list.id).await;
        let b = fixture.add_work(list.id).await;
        let c = fixture.add_work(list.id).await;
        let foreign = fixture.add_work(other_list.id).await;
        let claim = fixture
            .repo
            .add_node(list.id, NewNode::claim(json!({}), ClaimState::Unverified))
            .await
            .expect("claim");

        assert_eq!(
            fixture
                .rejection(list.id, a.id, b.id, EdgeKind::Covers)
                .await,
            EdgeRejection::InvalidKind
        );
        assert_eq!(
            fixture
                .rejection(list.id, topic.id, a.id, EdgeKind::Subtopic)
                .await,
            EdgeRejection::InvalidKind
        );
        assert_eq!(
            fixture
                .rejection(list.id, a.id, claim.id, EdgeKind::Prerequisite)
                .await,
            EdgeRejection::InvalidKind
        );
        assert_eq!(
            fixture
                .rejection(list.id, a.id, foreign.id, EdgeKind::Prerequisite)
                .await,
            EdgeRejection::UnknownNode,
            "a node of another list is not an endpoint"
        );
        assert_eq!(
            fixture
                .rejection(list.id, a.id, a.id, EdgeKind::Prerequisite)
                .await,
            EdgeRejection::Cycle(vec![a.id])
        );

        fixture
            .edge(list.id, a.id, b.id, EdgeKind::Prerequisite)
            .await;
        fixture
            .edge(list.id, b.id, c.id, EdgeKind::Prerequisite)
            .await;
        let cycle = fixture
            .rejection(list.id, c.id, a.id, EdgeKind::Prerequisite)
            .await;
        assert_eq!(cycle.cycle(), Some(&[c.id, a.id, b.id][..]));
        assert_eq!(fixture.repo.edges(list.id).await.expect("edges").len(), 2);
    }

    #[tokio::test]
    async fn should_conflict_when_a_list_holds_the_edge_limit() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Edge limit").await;
        fixture.bulk_claims(list.id, 100).await;
        sqlx::query(
            "insert into reader.list_edge (list_id, from_node, to_node, kind, source) \
             select $1, a.id, b.id, 'prerequisite', 'user' \
             from reader.list_node a, reader.list_node b \
             where a.list_id = $1 and b.list_id = $1 and a.id <> b.id limit 4000",
        )
        .bind(list.id.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("edges");
        let topic = fixture.add_topic(list.id, "Topic").await;
        let work = fixture.add_work(list.id).await;
        assert_eq!(
            conflict_code(
                fixture
                    .repo
                    .add_edge(list.id, new_edge(topic.id, work.id, EdgeKind::Covers))
                    .await
            ),
            "edges_full"
        );
    }

    #[tokio::test]
    async fn should_accept_a_proposed_edge_when_asked() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Review edges").await;
        let a = fixture.add_work(list.id).await;
        let b = fixture.add_work(list.id).await;
        fixture
            .repo
            .add_edge(
                list.id,
                NewEdge {
                    source: EdgeSource::Agent,
                    review: ListReview::Proposed,
                    ..new_edge(a.id, b.id, EdgeKind::Prerequisite)
                },
            )
            .await
            .expect("db")
            .expect("edge");
        assert!(
            fixture
                .repo
                .accept_edge(list.id, a.id, b.id, EdgeKind::Prerequisite)
                .await
                .expect("accept")
        );
        let edges = fixture.repo.edges(list.id).await.expect("edges");
        assert_eq!(edges[0].review, ListReview::Accepted);
        assert_eq!(edges[0].source, EdgeSource::Agent);
        assert!(
            !fixture
                .repo
                .accept_edge(list.id, b.id, a.id, EdgeKind::Prerequisite)
                .await
                .expect("missing")
        );
    }

    // ---- order ----

    #[tokio::test]
    async fn should_rewrite_dense_positions_when_reordering() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Reorder").await;
        let a = fixture.add_work(list.id).await;
        let b = fixture.add_topic(list.id, "B").await;
        let c = fixture.add_work(list.id).await;
        let d = fixture.add_work(list.id).await;
        // Open a gap first: reorder must close it.
        fixture
            .repo
            .remove_node(list.id, c.id)
            .await
            .expect("remove");
        assert_eq!(
            positions(&fixture.repo.nodes(list.id).await.expect("nodes")),
            vec![0, 1, 3]
        );

        fixture
            .repo
            .reorder(list.id, &[d.id, a.id, b.id])
            .await
            .expect("reorder");
        let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
        assert_eq!(node_ids(&nodes), vec![d.id, a.id, b.id]);
        assert_eq!(positions(&nodes), vec![0, 1, 2]);

        // The same order again, and a later append, keep positions dense.
        fixture
            .repo
            .reorder(list.id, &[d.id, a.id, b.id])
            .await
            .expect("same");
        let appended = fixture.add_work(list.id).await;
        assert_eq!(appended.position, 3);
    }

    #[tokio::test]
    async fn should_refuse_and_change_nothing_when_the_order_is_not_a_permutation() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Bad order").await;
        let other = fixture.list("Other").await;
        let a = fixture.add_work(list.id).await;
        let b = fixture.add_work(list.id).await;
        let foreign = fixture.add_work(other.id).await;
        let attempts: Vec<(&str, Vec<NodeId>)> = vec![
            ("missing one", vec![b.id]),
            ("duplicate", vec![b.id, b.id]),
            ("extra", vec![b.id, a.id, NodeId::new()]),
            ("foreign", vec![b.id, foreign.id]),
            ("empty", vec![]),
        ];
        for (label, order) in attempts {
            assert_eq!(
                conflict_code(fixture.repo.reorder(list.id, &order).await),
                "invalid_order",
                "{label}"
            );
            let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
            assert_eq!(
                node_ids(&nodes),
                vec![a.id, b.id],
                "{label} must not reorder"
            );
        }
        let other_nodes = fixture.repo.nodes(other.id).await.expect("nodes");
        assert_eq!(other_nodes[0].position, 0, "another list is untouched");
        let missing = fixture.repo.reorder(ListId::new(), &[]).await;
        assert!(matches!(missing, Err(ref error) if error.is_not_found()));
        // An empty list accepts the empty order.
        let empty = fixture.list("Empty").await;
        fixture.repo.reorder(empty.id, &[]).await.expect("empty");
    }

    // ---- memberships ----

    #[tokio::test]
    async fn should_report_lists_with_topics_and_notes_when_asking_for_memberships() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let work = fixture.work().await;
        let plain = fixture.list("Plain").await;
        let filed = fixture.list("Filed").await;
        let archived = fixture.list("Archived").await;
        let absent = fixture.list("Absent").await;
        let mut with_note = NewNode::work(work);
        with_note.note = Some("why here".into());
        fixture
            .repo
            .add_node(plain.id, with_note)
            .await
            .expect("plain");
        let second = fixture.add_topic(filed.id, "Second").await;
        let first = fixture.add_topic(filed.id, "First").await;
        let node = fixture
            .repo
            .add_node(filed.id, work_in_topics(work, vec![first.id, second.id]))
            .await
            .expect("filed");
        fixture
            .repo
            .add_node(archived.id, NewNode::work(work))
            .await
            .expect("archived");
        fixture.archive(archived.id).await;
        fixture.add_work(absent.id).await;

        let active = fixture
            .repo
            .memberships_by_work(fixture.workspace, work, ArchiveScope::Active)
            .await
            .expect("memberships");
        assert_eq!(
            active
                .iter()
                .map(|membership| membership.list_id)
                .collect::<Vec<_>>(),
            vec![filed.id, plain.id],
            "most recently updated list first"
        );
        assert_eq!(active[0].node_id, node.id);
        assert_eq!(
            active[0].topics,
            vec!["Second".to_owned(), "First".to_owned()]
        );
        assert_eq!(active[1].topics, Vec::<String>::new());
        assert_eq!(active[1].note.as_deref(), Some("why here"));
        let all = fixture
            .repo
            .memberships_by_work(fixture.workspace, work, ArchiveScope::All)
            .await
            .expect("memberships");
        assert_eq!(all.len(), 3);
        assert_eq!(
            all.iter().filter(|membership| membership.archived).count(),
            1
        );
        let nothing = fixture
            .repo
            .memberships_by_work(fixture.workspace, WorkId::new(), ArchiveScope::All)
            .await
            .expect("memberships");
        assert!(nothing.is_empty());
        let other = test_support::new_workspace(&fixture.pool).await;
        assert!(
            fixture
                .repo
                .memberships_by_work(other, work, ArchiveScope::All)
                .await
                .expect("memberships")
                .is_empty()
        );
    }

    // ---- schema checks ----

    #[tokio::test]
    async fn should_refuse_malformed_rows_when_the_schema_checks_run() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let list = fixture.list("Checks").await;
        let work = fixture.work().await;
        let node_insert = |kind: &'static str,
                           work_id: Option<Uuid>,
                           label: Option<&'static str>,
                           claim_state: Option<&'static str>,
                           position: i32| {
            let pool = fixture.pool.clone();
            let list_id = list.id.as_uuid();
            async move {
                sqlx::query(
                    "insert into reader.list_node \
                         (list_id, kind, work_id, label, claim, claim_state, position, source) \
                     values ($1, $2, $3, $4, case when $2 = 'claim' then '{}'::jsonb end, \
                             $5, $6, 'user')",
                )
                .bind(list_id)
                .bind(kind)
                .bind(work_id)
                .bind(label)
                .bind(claim_state)
                .bind(position)
                .execute(&pool)
                .await
            }
        };
        assert!(
            node_insert("work", None, None, None, 0).await.is_err(),
            "work needs a work"
        );
        assert!(
            node_insert("topic", None, None, None, 0).await.is_err(),
            "topic needs a label"
        );
        assert!(
            node_insert("claim", None, None, None, 0).await.is_err(),
            "claim needs a state"
        );
        assert!(
            node_insert("topic", Some(work.as_uuid()), Some("x"), None, 0)
                .await
                .is_err(),
            "a topic is not a Work"
        );
        assert!(
            node_insert("work", Some(work.as_uuid()), None, None, -1)
                .await
                .is_err()
        );
        assert!(
            node_insert("claim", None, None, Some("guessed"), 0)
                .await
                .is_err()
        );
        assert!(node_insert("note", None, None, None, 0).await.is_err());
        node_insert("work", Some(work.as_uuid()), None, None, 0)
            .await
            .expect("valid work");
        assert!(
            node_insert("work", Some(work.as_uuid()), None, None, 1)
                .await
                .is_err(),
            "a Work is unique per list"
        );
        assert!(
            node_insert("topic", None, Some("a topic"), None, 0)
                .await
                .is_err(),
            "a position is unique per list"
        );
        let padded = sqlx::query(
            "insert into reader.reading_list (workspace_id, title, origin) \
             values ($1, ' x ', 'manual')",
        )
        .bind(fixture.workspace.as_uuid())
        .execute(&fixture.pool)
        .await;
        assert!(padded.is_err(), "a title with edge spaces is refused");
        let origin = sqlx::query(
            "insert into reader.reading_list (workspace_id, title, origin) \
             values ($1, 'x', 'import')",
        )
        .bind(fixture.workspace.as_uuid())
        .execute(&fixture.pool)
        .await;
        assert!(origin.is_err(), "an unknown origin is refused");
        let self_edge = sqlx::query(
            "insert into reader.list_edge (list_id, from_node, to_node, kind, source) \
             select $1, id, id, 'prerequisite', 'user' from reader.list_node where list_id = $1",
        )
        .bind(list.id.as_uuid())
        .execute(&fixture.pool)
        .await;
        assert!(self_edge.is_err(), "a self edge is refused");
    }

    #[tokio::test]
    async fn should_set_seed_to_null_and_keep_nodes_when_the_seed_work_is_deleted() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let seed = fixture.work().await;
        let list = fixture
            .repo
            .create(
                fixture.workspace,
                NewList {
                    seed_work_id: Some(seed),
                    ..NewList::manual(title("Seed gone"))
                },
            )
            .await
            .expect("create");
        let kept = fixture.add_work(list.id).await;
        sqlx::query("delete from graph.work where id = $1")
            .bind(seed.as_uuid())
            .execute(&fixture.pool)
            .await
            .expect("delete work");
        let after = fixture
            .repo
            .get(fixture.workspace, list.id)
            .await
            .expect("get")
            .expect("list");
        assert_eq!(after.seed_work_id, None);
        assert!(
            fixture
                .repo
                .node(list.id, kept.id)
                .await
                .expect("node")
                .is_some()
        );
    }

    // ---- migration 0011 ----

    type MigratedNode = (
        Uuid,
        String,
        Uuid,
        i32,
        Option<Uuid>,
        Option<Vec<Uuid>>,
        Option<String>,
        String,
        String,
    );

    /// One ramp item: ramp, work, position, via, path, status, note, `status_changed_at`.
    type RampItem = (
        Uuid,
        WorkId,
        i32,
        Option<WorkId>,
        Vec<WorkId>,
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
    );

    /// Replay the ramp-to-list data move inside a transaction that is rolled back, against
    /// ramp tables recreated from migration 0008.
    #[tokio::test]
    async fn should_keep_ids_positions_paths_and_notes_and_take_max_status_when_migrating_ramps() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let ramp_ddl = include_str!("../migrations/0008_ramps.sql")
            .split_once("-- Ledger")
            .expect("ledger marker")
            .0;
        let move_sql = include_str!("../migrations/0011_reading_lists.sql")
            .split_once("-- Data move from Ramp E")
            .and_then(|(_, rest)| rest.split_once('\n'))
            .expect("data move marker")
            .1;
        let (w1, w2, w3, w4) = (
            fixture.work().await,
            fixture.work().await,
            fixture.work().await,
            fixture.work().await,
        );
        let (r1, r2, r3) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let workspace = fixture.workspace.as_uuid();
        let other_workspace = test_support::new_workspace(&fixture.pool).await.as_uuid();

        let mut tx = fixture.pool.begin().await.expect("begin");
        sqlx::raw_sql(ramp_ddl)
            .execute(&mut *tx)
            .await
            .expect("ramp tables");
        for (id, owner, seed, name) in [
            (r1, workspace, w1, "Ramp one"),
            (r2, workspace, w2, "Ramp two"),
            (r3, other_workspace, w2, "Other workspace"),
        ] {
            sqlx::query(
                "insert into reader.ramp (id, workspace_id, seed_work_id, title) \
                 values ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(owner)
            .bind(seed.as_uuid())
            .bind(name)
            .execute(&mut *tx)
            .await
            .expect("ramp");
        }
        let items: Vec<RampItem> = vec![
            (
                r1,
                w1,
                0,
                None,
                vec![w1],
                "reading",
                None,
                Some("2026-03-01T00:00:00Z"),
            ),
            (
                r1,
                w2,
                3,
                Some(w1),
                vec![w1, w2],
                "read",
                Some("kept note"),
                Some("2026-04-01T00:00:00Z"),
            ),
            (r1, w3, 7, Some(w2), vec![w1, w2, w3], "unread", None, None),
            (
                r2,
                w2,
                0,
                None,
                vec![w2],
                "reading",
                None,
                Some("2026-05-01T00:00:00Z"),
            ),
            (
                r2,
                w4,
                1,
                Some(w2),
                vec![w2, w4],
                "reading",
                Some("n"),
                Some("2026-02-01T00:00:00Z"),
            ),
            (r3, w2, 0, None, vec![w2], "unread", None, None),
        ];
        for (ramp, work, position, via, path, status, note, changed) in &items {
            sqlx::query(
                "insert into reader.ramp_item \
                     (ramp_id, work_id, position, via_work_id, path_work_ids, status, note, \
                      status_changed_at) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz)",
            )
            .bind(ramp)
            .bind(work.as_uuid())
            .bind(position)
            .bind(via.map(WorkId::as_uuid))
            .bind(path.iter().map(|id| id.as_uuid()).collect::<Vec<_>>())
            .bind(status)
            .bind(note)
            .bind(changed)
            .execute(&mut *tx)
            .await
            .expect("item");
        }

        sqlx::raw_sql(move_sql)
            .execute(&mut *tx)
            .await
            .expect("data move");

        let lists: Vec<(Uuid, String, String, Option<Uuid>)> = sqlx::query_as(
            "select id, title, origin, seed_work_id from reader.reading_list \
             where id = any($1) order by title",
        )
        .bind(vec![r1, r2, r3])
        .fetch_all(&mut *tx)
        .await
        .expect("lists");
        assert_eq!(
            lists,
            vec![
                (
                    r3,
                    "Other workspace".into(),
                    "walk".into(),
                    Some(w2.as_uuid())
                ),
                (r1, "Ramp one".into(), "walk".into(), Some(w1.as_uuid())),
                (r2, "Ramp two".into(), "walk".into(), Some(w2.as_uuid())),
            ]
        );

        let nodes: Vec<MigratedNode> = sqlx::query_as(
            "select list_id, kind, work_id, position, via_work_id, path_work_ids, note, \
                    source, review \
             from reader.list_node where list_id = any($1) order by list_id, position",
        )
        .bind(vec![r1, r2])
        .fetch_all(&mut *tx)
        .await
        .expect("nodes");
        let mut expected: Vec<MigratedNode> = items
            .iter()
            .filter(|item| item.0 != r3)
            .map(|(ramp, work, position, via, path, _, note, _)| {
                (
                    *ramp,
                    "work".to_owned(),
                    work.as_uuid(),
                    *position,
                    via.map(WorkId::as_uuid),
                    Some(path.iter().map(|id| id.as_uuid()).collect::<Vec<_>>()),
                    note.map(str::to_owned),
                    "walk".to_owned(),
                    "accepted".to_owned(),
                )
            })
            .collect();
        expected.sort_by_key(|row| (row.0, row.3));
        assert_eq!(nodes, expected, "positions, via, paths and notes survive");

        let states: Vec<(Uuid, String, DateTime<Utc>)> = sqlx::query_as(
            "select work_id, status, changed_at from reader.work_reading_state \
             where workspace_id = $1",
        )
        .bind(workspace)
        .fetch_all(&mut *tx)
        .await
        .expect("states");
        let state = |work: WorkId| {
            states
                .iter()
                .find(|(id, _, _)| *id == work.as_uuid())
                .map(|(_, status, at)| (status.as_str(), at.to_rfc3339()))
        };
        assert_eq!(
            state(w1),
            Some(("reading", "2026-03-01T00:00:00+00:00".into()))
        );
        assert_eq!(
            state(w2),
            Some(("read", "2026-05-01T00:00:00+00:00".into())),
            "read beats reading, stamped with the latest change"
        );
        assert_eq!(
            state(w4),
            Some(("reading", "2026-02-01T00:00:00+00:00".into()))
        );
        assert_eq!(state(w3).map(|(status, _)| status), Some("unread"));
        let other_states: i64 = sqlx::query_scalar(
            "select count(*) from reader.work_reading_state where workspace_id = $1",
        )
        .bind(other_workspace)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
        assert_eq!(other_states, 1, "statuses are per workspace");

        let dropped: Option<String> = sqlx::query_scalar(
            "select coalesce(to_regclass('reader.ramp')::text, \
                             to_regclass('reader.ramp_item')::text)",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("regclass");
        assert_eq!(dropped, None, "the ramp tables are gone");
        tx.rollback().await.expect("rollback");
    }

    // ---- merges ----

    #[tokio::test]
    async fn should_repoint_nodes_paths_and_seed_when_merging_works() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let works = WorkRepository::new(fixture.pool.clone());
        let from = fixture.work().await;
        let into = fixture.work().await;
        let middle = fixture.work().await;
        let leaf = fixture.work().await;
        test_support::cite(&fixture.pool, from, middle).await;
        test_support::cite(&fixture.pool, middle, leaf).await;
        let list = fixture
            .repo
            .create(
                fixture.workspace,
                NewList {
                    seed_work_id: Some(from),
                    origin: ListOrigin::Walk,
                    ..NewList::manual(title("Merge me"))
                },
            )
            .await
            .expect("create");
        fixture
            .repo
            .add_node(list.id, NewNode::work(from))
            .await
            .expect("seed");
        fixture
            .repo
            .add_node(list.id, NewNode::work_via(middle, from))
            .await
            .expect("middle");
        fixture
            .repo
            .add_node(list.id, NewNode::work_via(leaf, middle))
            .await
            .expect("leaf");

        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");

        let after = fixture
            .repo
            .get(fixture.workspace, list.id)
            .await
            .expect("get")
            .expect("list");
        assert_eq!(after.seed_work_id, Some(into));
        let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
        let by_work = |work: WorkId| {
            nodes
                .iter()
                .find(|node| node.work_id == Some(work))
                .expect("node")
        };
        assert!(nodes.iter().all(|node| node.work_id != Some(from)));
        assert_eq!(by_work(into).position, 0);
        assert_eq!(by_work(middle).path, Some(vec![into, middle]));
        assert_eq!(by_work(middle).via_work_id, Some(into));
        assert_eq!(by_work(leaf).path, Some(vec![into, middle, leaf]));
        assert_eq!(by_work(leaf).via_work_id, Some(middle));
    }

    #[tokio::test]
    async fn should_fold_node_note_edges_state_and_history_into_the_survivor_when_merging() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let works = WorkRepository::new(fixture.pool.clone());
        let states = ReadingStateRepository::new(fixture.pool.clone());
        let from = fixture.work().await;
        let into = fixture.work().await;
        let list = fixture.list("Both").await;
        let only_from_list = fixture.list("Only from").await;

        let topic = fixture.add_topic(list.id, "Topic").await;
        let before = fixture.add_work(list.id).await;
        let mut noted_from = NewNode::work(from);
        noted_from.note = Some("lost?".into());
        let from_node = fixture
            .repo
            .add_node(list.id, noted_from)
            .await
            .expect("from");
        let mut noted_into = NewNode::work(into);
        noted_into.note = Some("survivor".into());
        let into_node = fixture
            .repo
            .add_node(list.id, noted_into)
            .await
            .expect("into");
        let after = fixture.add_work(list.id).await;
        // from has: a cover that duplicates the survivor's, a prerequisite in and out, and a
        // prerequisite to the survivor (which would become a self edge).
        fixture
            .edge(list.id, topic.id, from_node.id, EdgeKind::Covers)
            .await;
        fixture
            .edge(list.id, topic.id, into_node.id, EdgeKind::Covers)
            .await;
        fixture
            .edge(list.id, before.id, from_node.id, EdgeKind::Prerequisite)
            .await;
        fixture
            .edge(list.id, from_node.id, after.id, EdgeKind::Prerequisite)
            .await;
        fixture
            .edge(list.id, from_node.id, into_node.id, EdgeKind::Prerequisite)
            .await;
        // A list holding only `from` simply repoints.
        let lone = fixture
            .repo
            .add_node(only_from_list.id, NewNode::work(from))
            .await
            .expect("lone");

        states
            .set(fixture.workspace, from, ReadingStatus::Read)
            .await
            .expect("state");
        states
            .set(fixture.workspace, into, ReadingStatus::Reading)
            .await
            .expect("state");
        let other_workspace = test_support::new_workspace(&fixture.pool).await;
        states
            .set(other_workspace, from, ReadingStatus::Reading)
            .await
            .expect("state");
        states
            .set(other_workspace, into, ReadingStatus::Read)
            .await
            .expect("state");
        let session = Uuid::new_v4();
        sqlx::query(
            "insert into reader.history_event \
                 (workspace_id, occurred_at, session_id, kind, work_id, client_seq) \
             values ($1, now(), $2, 'work_inspected', $3, 1)",
        )
        .bind(fixture.workspace.as_uuid())
        .bind(session)
        .bind(from.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("history");

        works
            .merge_work(from, into, "duplicate", "test")
            .await
            .expect("merge");

        let nodes = fixture.repo.nodes(list.id).await.expect("nodes");
        assert_eq!(nodes.len(), 4, "the merged node is folded away");
        assert!(nodes.iter().all(|node| node.work_id != Some(from)));
        let survivor = fixture
            .repo
            .node(list.id, into_node.id)
            .await
            .expect("node")
            .expect("survivor");
        assert_eq!(survivor.note.as_deref(), Some("survivor\n\nlost?"));
        assert_eq!(
            fixture
                .repo
                .node(list.id, from_node.id)
                .await
                .expect("node"),
            None
        );
        let edges: Vec<(NodeId, NodeId, EdgeKind)> = fixture
            .repo
            .edges(list.id)
            .await
            .expect("edges")
            .into_iter()
            .map(|edge| (edge.from, edge.to, edge.kind))
            .collect();
        let has = |from: NodeId, to: NodeId, kind: EdgeKind| edges.contains(&(from, to, kind));
        assert!(has(topic.id, into_node.id, EdgeKind::Covers));
        assert!(has(before.id, into_node.id, EdgeKind::Prerequisite));
        assert!(has(into_node.id, after.id, EdgeKind::Prerequisite));
        assert_eq!(
            edges.len(),
            3,
            "duplicates and the self edge are dropped: {edges:?}"
        );

        let lone_after = fixture
            .repo
            .node(only_from_list.id, lone.id)
            .await
            .expect("node")
            .expect("lone");
        assert_eq!(lone_after.work_id, Some(into));

        assert_eq!(
            states.get(fixture.workspace, into).await.expect("get"),
            ReadingStatus::Read
        );
        assert_eq!(
            states.get(fixture.workspace, from).await.expect("get"),
            ReadingStatus::Unread
        );
        assert_eq!(
            states.get(other_workspace, into).await.expect("get"),
            ReadingStatus::Read,
            "the survivor keeps a higher status"
        );
        let history: i64 = sqlx::query_scalar(
            "select count(*) from reader.history_event where work_id = $1 and session_id = $2",
        )
        .bind(into.as_uuid())
        .bind(session)
        .fetch_one(&fixture.pool)
        .await
        .expect("history");
        assert_eq!(history, 1);
    }

    #[tokio::test]
    async fn should_keep_the_survivor_note_when_the_merged_node_has_none_and_cap_long_notes() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let works = WorkRepository::new(fixture.pool.clone());
        let list = fixture.list("Notes").await;
        let (from, into) = (fixture.work().await, fixture.work().await);
        fixture
            .repo
            .add_node(list.id, NewNode::work(from))
            .await
            .expect("from");
        let mut survivor = NewNode::work(into);
        survivor.note = Some("only mine".into());
        let into_node = fixture
            .repo
            .add_node(list.id, survivor)
            .await
            .expect("into");
        works
            .merge_work(from, into, "dup", "test")
            .await
            .expect("merge");
        let kept = fixture
            .repo
            .node(list.id, into_node.id)
            .await
            .expect("node")
            .expect("node");
        assert_eq!(kept.note.as_deref(), Some("only mine"));

        let (from, into) = (fixture.work().await, fixture.work().await);
        let mut long_from = NewNode::work(from);
        long_from.note = Some("é".repeat(1500));
        fixture
            .repo
            .add_node(list.id, long_from)
            .await
            .expect("from");
        let mut long_into = NewNode::work(into);
        long_into.note = Some("ü".repeat(1500));
        let into_node = fixture
            .repo
            .add_node(list.id, long_into)
            .await
            .expect("into");
        works
            .merge_work(from, into, "dup", "test")
            .await
            .expect("merge");
        let merged = fixture
            .repo
            .node(list.id, into_node.id)
            .await
            .expect("node")
            .expect("node");
        assert_eq!(merged.note.expect("note").chars().count(), 2000);
    }
}
