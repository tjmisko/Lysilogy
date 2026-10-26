//! Persistence for the append-only reading history and per-paper reading position.
//!
//! Events arrive in client batches, idempotent on `(session_id, client_seq)`. Reads are the
//! recent timeline and the per-paper and per-Work aggregates that the history service
//! derives its summaries and the knowledge profile from.

use chrono::{DateTime, Utc};
use lysilogy_core::{ListId, WorkId, WorkspaceId};
use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::Result;

/// Most events accepted in one batch.
pub const HISTORY_BATCH_MAX: usize = 200;
/// Largest page of timeline rows.
pub const HISTORY_PAGE_MAX: i64 = 500;
/// Page size when the caller does not choose one.
pub const HISTORY_PAGE_DEFAULT: i64 = 100;
/// Longest `paper_id` accepted.
pub const PAPER_ID_MAX: usize = 200;
/// Largest serialized `detail` accepted, in bytes.
pub const DETAIL_MAX_BYTES: usize = 16 * 1024;

/// Every event kind the history records.
pub const HISTORY_KINDS: [&str; 16] = [
    "paper_opened",
    "paper_left",
    "view_changed",
    "section_focused",
    "figure_viewed",
    "page_reached",
    "search_run",
    "question_asked",
    "link_followed",
    "digest_opened",
    "graph_opened",
    "work_inspected",
    "list_opened",
    "list_changed",
    "list_generated",
    "fetch_requested",
];

/// Views a reading position can be in.
pub const POSITION_VIEWS: [&str; 4] = ["pdf", "markdown", "overview", "section"];

/// Why a batch, filter or position was rejected before touching the database.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HistoryInvalid {
    #[error("a batch holds at most {HISTORY_BATCH_MAX} events, got {0}")]
    BatchTooLarge(usize),
    #[error("event {index}: unknown kind {kind:?}")]
    UnknownKind { index: usize, kind: String },
    #[error("event {index}: dwell_ms must not be negative")]
    NegativeDwell { index: usize },
    #[error("event {index}: client_seq must not be negative")]
    NegativeSeq { index: usize },
    #[error("event {index}: paper_id must be 1 to {PAPER_ID_MAX} characters")]
    BadPaperId { index: usize },
    #[error("event {index}: detail must be a JSON object under {DETAIL_MAX_BYTES} bytes")]
    BadDetail { index: usize },
    #[error("unknown kind {0:?}")]
    UnknownFilterKind(String),
    #[error("unknown view {0:?}")]
    UnknownView(String),
    #[error("page, offset and section fields must not be negative or empty")]
    BadPosition,
    #[error("scroll_ratio must be between 0 and 1")]
    BadScrollRatio,
}

impl HistoryInvalid {
    /// The API error code; all of these are `400`s.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BatchTooLarge(_) => "invalid_history_batch",
            Self::UnknownKind { .. } | Self::UnknownFilterKind(_) => "invalid_history_kind",
            Self::NegativeDwell { .. } => "invalid_history_dwell",
            Self::NegativeSeq { .. } => "invalid_history_seq",
            Self::BadPaperId { .. } => "invalid_history_paper",
            Self::BadDetail { .. } => "invalid_history_detail",
            Self::UnknownView(_) | Self::BadPosition | Self::BadScrollRatio => "invalid_position",
        }
    }
}

/// An event as the client reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEvent {
    pub occurred_at: DateTime<Utc>,
    pub session_id: Uuid,
    pub kind: String,
    pub paper_id: Option<String>,
    pub work_id: Option<WorkId>,
    pub list_id: Option<ListId>,
    pub dwell_ms: Option<i32>,
    pub detail: Value,
    pub client_seq: i32,
}

/// A stored event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryEvent {
    pub id: i64,
    pub occurred_at: DateTime<Utc>,
    pub session_id: Uuid,
    pub kind: String,
    pub paper_id: Option<String>,
    pub work_id: Option<WorkId>,
    pub list_id: Option<ListId>,
    pub dwell_ms: Option<i32>,
    pub detail: Value,
    pub client_seq: i32,
    /// Display title of `work_id`, when set and known.
    pub work_title: Option<String>,
}

/// How many events of a batch were stored and how many were already there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BatchOutcome {
    pub accepted: usize,
    pub duplicates: usize,
}

/// Position in the timeline: events strictly older than this come next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryCursor {
    pub occurred_at: DateTime<Utc>,
    /// Breaks ties between events at the same instant; `i64::MAX` includes them all.
    pub id: i64,
}

/// Filters for the recent timeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    pub before: Option<HistoryCursor>,
    /// Empty means every kind.
    pub kinds: Vec<String>,
    /// Case-insensitive substring over the detail's query, title and target, and the
    /// Work's title.
    pub text: Option<String>,
    /// Clamped to `1..=HISTORY_PAGE_MAX`; `None` means `HISTORY_PAGE_DEFAULT`.
    pub limit: Option<i64>,
}

/// Visit and dwell totals for one paper.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaperDwell {
    pub paper_id: String,
    pub work_id: Option<WorkId>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub visits: i64,
    /// Sum of `paper_left` dwell.
    pub dwell_ms: i64,
}

/// Dwell inside one section of a paper.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SectionDwell {
    pub section_id: String,
    pub visits: i64,
    pub dwell_ms: i64,
}

/// Everything the history knows about one paper.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaperAggregate {
    pub paper_id: String,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    pub visits: i64,
    pub dwell_ms: i64,
    pub sections: Vec<SectionDwell>,
}

/// Per-Work engagement used by the knowledge profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkDwell {
    pub work_id: WorkId,
    pub opens: i64,
    pub inspects: i64,
    /// Sum of `paper_left` dwell.
    pub dwell_ms: i64,
    pub last_seen: DateTime<Utc>,
}

/// A Work that was inspected but never opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InspectedWork {
    pub work_id: WorkId,
    pub inspects: i64,
    pub last_inspected: DateTime<Utc>,
}

/// A reading position as the client reports it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadingPosition {
    pub paper_id: String,
    pub view: String,
    pub page: Option<i32>,
    pub offset_utf16: Option<i32>,
    pub section_id: Option<String>,
    pub scroll_ratio: Option<f32>,
    pub updated_at: DateTime<Utc>,
}

/// Check a batch before storing it. `index` in errors is the position in the batch.
pub fn validate_events(events: &[NewEvent]) -> std::result::Result<(), HistoryInvalid> {
    if events.len() > HISTORY_BATCH_MAX {
        return Err(HistoryInvalid::BatchTooLarge(events.len()));
    }
    for (index, event) in events.iter().enumerate() {
        if !HISTORY_KINDS.contains(&event.kind.as_str()) {
            return Err(HistoryInvalid::UnknownKind {
                index,
                kind: event.kind.clone(),
            });
        }
        if event.dwell_ms.is_some_and(|dwell| dwell < 0) {
            return Err(HistoryInvalid::NegativeDwell { index });
        }
        if event.client_seq < 0 {
            return Err(HistoryInvalid::NegativeSeq { index });
        }
        if let Some(paper) = &event.paper_id {
            let length = paper.chars().count();
            if length == 0 || length > PAPER_ID_MAX || paper.contains('\0') {
                return Err(HistoryInvalid::BadPaperId { index });
            }
        }
        if !event.detail.is_object()
            || event.detail.to_string().len() > DETAIL_MAX_BYTES
            || json_contains_nul(&event.detail)
        {
            return Err(HistoryInvalid::BadDetail { index });
        }
    }
    Ok(())
}

/// Postgres `jsonb` rejects `\u0000` in keys and strings.
fn json_contains_nul(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains('\0'),
        serde_json::Value::Array(items) => items.iter().any(json_contains_nul),
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(key, item)| key.contains('\0') || json_contains_nul(item)),
        _ => false,
    }
}

/// Check the kinds a timeline filter names.
pub fn validate_kinds(kinds: &[String]) -> std::result::Result<(), HistoryInvalid> {
    kinds
        .iter()
        .find(|kind| !HISTORY_KINDS.contains(&kind.as_str()))
        .map_or(Ok(()), |kind| {
            Err(HistoryInvalid::UnknownFilterKind(kind.clone()))
        })
}

/// Check a position before storing it.
pub fn validate_position(position: &ReadingPosition) -> std::result::Result<(), HistoryInvalid> {
    if !POSITION_VIEWS.contains(&position.view.as_str()) {
        return Err(HistoryInvalid::UnknownView(position.view.clone()));
    }
    let negative = position.page.is_some_and(|page| page < 0)
        || position.offset_utf16.is_some_and(|offset| offset < 0)
        || position.section_id.as_deref().is_some_and(str::is_empty)
        || position.paper_id.is_empty()
        || position.paper_id.chars().count() > PAPER_ID_MAX
        || position.paper_id.contains('\0')
        || position
            .section_id
            .as_deref()
            .is_some_and(|id| id.contains('\0'));
    if negative {
        return Err(HistoryInvalid::BadPosition);
    }
    if position
        .scroll_ratio
        .is_some_and(|ratio| !(0.0..=1.0).contains(&ratio))
    {
        return Err(HistoryInvalid::BadScrollRatio);
    }
    Ok(())
}

/// Escape `%`, `_` and `\` for use inside a `LIKE` pattern.
fn like_pattern(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('%');
    for character in text.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('%');
    escaped
}

const EVENT_COLUMNS: &str = "e.id, e.occurred_at, e.session_id, e.kind, e.paper_id, e.work_id, \
    e.list_id, e.dwell_ms, e.detail, e.client_seq, w.display_title as work_title";

/// Reads and writes of history events and reading positions.
#[derive(Clone)]
pub struct HistoryRepository {
    pool: PgPool,
}

impl HistoryRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Store a batch in one transaction. Events already stored under the same
    /// `(session_id, client_seq)` count as duplicates and are left untouched. A `work_id`
    /// or `list_id` that no longer exists (or a list of another workspace) is stored as
    /// null rather than failing the batch. A `work_id` merged into another Work is stored
    /// as the surviving Work.
    pub async fn insert_batch(
        &self,
        workspace: WorkspaceId,
        events: &[NewEvent],
    ) -> Result<BatchOutcome> {
        if events.is_empty() {
            return Ok(BatchOutcome::default());
        }
        let mut tx = self.pool.begin().await?;
        let mut outcome = BatchOutcome::default();
        for event in events {
            let inserted = sqlx::query(
                "insert into reader.history_event \
                 (workspace_id, occurred_at, session_id, kind, paper_id, work_id, list_id, \
                  dwell_ms, detail, client_seq) \
                 select $1, $2, $3, $4, $5, \
                        coalesce( \
                            (select new_work_id from graph.work_redirect where old_work_id = $6), \
                            (select id from graph.work where id = $6)), \
                        (select id from reader.reading_list where id = $7 and workspace_id = $1), \
                        $8, $9, $10 \
                 on conflict (session_id, client_seq) do nothing",
            )
            .bind(workspace.as_uuid())
            .bind(event.occurred_at)
            .bind(event.session_id)
            .bind(&event.kind)
            .bind(event.paper_id.as_deref())
            .bind(event.work_id.map(WorkId::as_uuid))
            .bind(event.list_id.map(ListId::as_uuid))
            .bind(event.dwell_ms)
            .bind(&event.detail)
            .bind(event.client_seq)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted == 0 {
                outcome.duplicates += 1;
            } else {
                outcome.accepted += 1;
            }
        }
        tx.commit().await?;
        Ok(outcome)
    }

    /// Give a paper's stored events that have no Work this Work, for events recorded before
    /// the client knew it (or before the paper was linked). Returns the rows changed.
    pub async fn attach_work_to_paper(
        &self,
        workspace: WorkspaceId,
        paper_id: &str,
        work: WorkId,
    ) -> Result<u64> {
        let changed = sqlx::query(
            "with target as ( \
                 select coalesce( \
                     (select new_work_id from graph.work_redirect where old_work_id = $3), \
                     (select id from graph.work where id = $3)) as id) \
             update reader.history_event e set work_id = target.id from target \
             where target.id is not null and e.workspace_id = $1 and e.paper_id = $2 \
               and e.work_id is null",
        )
        .bind(workspace.as_uuid())
        .bind(paper_id)
        .bind(work.as_uuid())
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(changed)
    }

    /// Newest-first timeline for a workspace.
    pub async fn recent(
        &self,
        workspace: WorkspaceId,
        query: &HistoryQuery,
    ) -> Result<Vec<HistoryEvent>> {
        let limit = query
            .limit
            .unwrap_or(HISTORY_PAGE_DEFAULT)
            .clamp(1, HISTORY_PAGE_MAX);
        let pattern = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(like_pattern);
        let rows = sqlx::query(&format!(
            "select {EVENT_COLUMNS} \
             from reader.history_event e \
             left join graph.work w on w.id = e.work_id \
             where e.workspace_id = $1 \
               and ($2::timestamptz is null or (e.occurred_at, e.id) < ($2, $3)) \
               and (cardinality($4::text[]) = 0 or e.kind = any($4)) \
               and ($5::text is null \
                    or e.detail->>'query' ilike $5 \
                    or e.detail->>'title' ilike $5 \
                    or e.detail->>'target' ilike $5 \
                    or w.display_title ilike $5) \
             order by e.occurred_at desc, e.id desc \
             limit $6"
        ))
        .bind(workspace.as_uuid())
        .bind(query.before.map(|cursor| cursor.occurred_at))
        .bind(query.before.map_or(0, |cursor| cursor.id))
        .bind(&query.kinds)
        .bind(pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// Every event at or after `since`, oldest first (for export).
    pub async fn since(
        &self,
        workspace: WorkspaceId,
        since: DateTime<Utc>,
    ) -> Result<Vec<HistoryEvent>> {
        let rows = sqlx::query(&format!(
            "select {EVENT_COLUMNS} \
             from reader.history_event e \
             left join graph.work w on w.id = e.work_id \
             where e.workspace_id = $1 and e.occurred_at >= $2 \
             order by e.occurred_at, e.id"
        ))
        .bind(workspace.as_uuid())
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// Whether a stored `paper_left` event for `work` has at least `dwell_ms` of dwell.
    pub async fn has_paper_left_dwelling(
        &self,
        workspace: WorkspaceId,
        work: WorkId,
        dwell_ms: i32,
    ) -> Result<bool> {
        let found = sqlx::query_scalar(
            "select exists (select 1 from reader.history_event \
             where workspace_id = $1 and work_id = $2 and kind = 'paper_left' \
               and dwell_ms >= $3)",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .bind(dwell_ms)
        .fetch_one(&self.pool)
        .await?;
        Ok(found)
    }

    /// Delete events older than `before`; returns how many.
    pub async fn prune_before(&self, workspace: WorkspaceId, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            "delete from reader.history_event where workspace_id = $1 and occurred_at < $2",
        )
        .bind(workspace.as_uuid())
        .bind(before)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Visit counts, dwell and per-section dwell for one paper. Totals are zero and the
    /// timestamps `None` when the paper has no history.
    pub async fn paper_aggregate(
        &self,
        workspace: WorkspaceId,
        paper_id: &str,
    ) -> Result<PaperAggregate> {
        let row = sqlx::query(
            "select min(occurred_at) as first_seen, max(occurred_at) as last_seen, \
                    count(*) filter (where kind = 'paper_opened') as visits, \
                    coalesce(sum(dwell_ms) filter (where kind = 'paper_left'), 0)::bigint \
                        as dwell_ms \
             from reader.history_event where workspace_id = $1 and paper_id = $2",
        )
        .bind(workspace.as_uuid())
        .bind(paper_id)
        .fetch_one(&self.pool)
        .await?;
        let section_rows = sqlx::query(
            "select detail->>'section_id' as section_id, count(*) as visits, \
                    coalesce(sum(dwell_ms), 0)::bigint as dwell_ms \
             from reader.history_event \
             where workspace_id = $1 and paper_id = $2 and kind = 'section_focused' \
               and detail->>'section_id' is not null \
             group by 1 order by dwell_ms desc, section_id",
        )
        .bind(workspace.as_uuid())
        .bind(paper_id)
        .fetch_all(&self.pool)
        .await?;
        let sections = section_rows
            .iter()
            .map(|row| {
                Ok(SectionDwell {
                    section_id: row.try_get("section_id")?,
                    visits: row.try_get("visits")?,
                    dwell_ms: row.try_get("dwell_ms")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PaperAggregate {
            paper_id: paper_id.to_owned(),
            first_seen: row.try_get("first_seen")?,
            last_seen: row.try_get("last_seen")?,
            visits: row.try_get("visits")?,
            dwell_ms: row.try_get("dwell_ms")?,
            sections,
        })
    }

    /// Dwell per opened paper, most recently seen first. T6 filters "barely read" from this.
    pub async fn paper_dwell(&self, workspace: WorkspaceId) -> Result<Vec<PaperDwell>> {
        let rows = sqlx::query(
            "select paper_id, (array_agg(work_id order by occurred_at desc) \
                        filter (where work_id is not null))[1] as work_id, \
                    min(occurred_at) as first_seen, max(occurred_at) as last_seen, \
                    count(*) filter (where kind = 'paper_opened') as visits, \
                    coalesce(sum(dwell_ms) filter (where kind = 'paper_left'), 0)::bigint \
                        as dwell_ms \
             from reader.history_event \
             where workspace_id = $1 and paper_id is not null \
             group by paper_id \
             having count(*) filter (where kind = 'paper_opened') > 0 \
             order by max(occurred_at) desc, paper_id",
        )
        .bind(workspace.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(PaperDwell {
                    paper_id: row.try_get("paper_id")?,
                    work_id: row
                        .try_get::<Option<Uuid>, _>("work_id")?
                        .map(WorkId::from_uuid),
                    first_seen: row.try_get("first_seen")?,
                    last_seen: row.try_get("last_seen")?,
                    visits: row.try_get("visits")?,
                    dwell_ms: row.try_get("dwell_ms")?,
                })
            })
            .collect()
    }

    /// Engagement per Work with any opens, inspections or dwell, ordered by Work ID.
    pub async fn work_dwell(&self, workspace: WorkspaceId) -> Result<Vec<WorkDwell>> {
        let rows = sqlx::query(
            "select work_id, \
                    count(*) filter (where kind = 'paper_opened') as opens, \
                    count(*) filter (where kind = 'work_inspected') as inspects, \
                    coalesce(sum(dwell_ms) filter (where kind = 'paper_left'), 0)::bigint \
                        as dwell_ms, \
                    max(occurred_at) as last_seen \
             from reader.history_event \
             where workspace_id = $1 and work_id is not null \
             group by work_id order by work_id",
        )
        .bind(workspace.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(work_dwell_from_row).collect()
    }

    /// Total `paper_left` dwell for one Work, in milliseconds (the D17 nudge check).
    pub async fn work_dwell_ms(&self, workspace: WorkspaceId, work: WorkId) -> Result<i64> {
        let total = sqlx::query_scalar::<_, i64>(
            "select coalesce(sum(dwell_ms), 0)::bigint from reader.history_event \
             where workspace_id = $1 and work_id = $2 and kind = 'paper_left'",
        )
        .bind(workspace.as_uuid())
        .bind(work.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        Ok(total)
    }

    /// Works that were inspected but never opened, most recently inspected first. A Work
    /// counts as opened only when a `paper_opened` event carries its `work_id`, so clients
    /// (T8, T10) must send `work_id` on `paper_opened` whenever the paper is linked.
    pub async fn inspected_never_opened(
        &self,
        workspace: WorkspaceId,
    ) -> Result<Vec<InspectedWork>> {
        let rows = sqlx::query(
            "select work_id, count(*) as inspects, max(occurred_at) as last_inspected \
             from reader.history_event \
             where workspace_id = $1 and kind = 'work_inspected' and work_id is not null \
             group by work_id \
             having not exists ( \
                 select 1 from reader.history_event o \
                 where o.workspace_id = $1 and o.kind = 'paper_opened' \
                   and o.work_id = reader.history_event.work_id) \
             order by max(occurred_at) desc, work_id",
        )
        .bind(workspace.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(InspectedWork {
                    work_id: WorkId::from_uuid(row.try_get("work_id")?),
                    inspects: row.try_get("inspects")?,
                    last_inspected: row.try_get("last_inspected")?,
                })
            })
            .collect()
    }

    /// Save a reading position, replacing the previous one for the paper.
    pub async fn upsert_position(
        &self,
        workspace: WorkspaceId,
        position: &ReadingPosition,
    ) -> Result<ReadingPosition> {
        let row = sqlx::query(
            "insert into reader.reading_position \
             (workspace_id, paper_id, view, page, offset_utf16, section_id, scroll_ratio) \
             values ($1, $2, $3, $4, $5, $6, $7) \
             on conflict (workspace_id, paper_id) do update set \
                 view = excluded.view, page = excluded.page, \
                 offset_utf16 = excluded.offset_utf16, section_id = excluded.section_id, \
                 scroll_ratio = excluded.scroll_ratio, updated_at = now() \
             returning paper_id, view, page, offset_utf16, section_id, scroll_ratio, updated_at",
        )
        .bind(workspace.as_uuid())
        .bind(&position.paper_id)
        .bind(&position.view)
        .bind(position.page)
        .bind(position.offset_utf16)
        .bind(position.section_id.as_deref())
        .bind(position.scroll_ratio)
        .fetch_one(&self.pool)
        .await?;
        position_from_row(&row)
    }

    /// The saved position for a paper, if any.
    pub async fn position(
        &self,
        workspace: WorkspaceId,
        paper_id: &str,
    ) -> Result<Option<ReadingPosition>> {
        let row = sqlx::query(
            "select paper_id, view, page, offset_utf16, section_id, scroll_ratio, updated_at \
             from reader.reading_position where workspace_id = $1 and paper_id = $2",
        )
        .bind(workspace.as_uuid())
        .bind(paper_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(position_from_row).transpose()
    }
}

/// Point history rows at `into` after Work `from` was merged into it.
pub async fn repoint_work(conn: &mut PgConnection, from: WorkId, into: WorkId) -> Result<u64> {
    let result = sqlx::query("update reader.history_event set work_id = $2 where work_id = $1")
        .bind(from.as_uuid())
        .bind(into.as_uuid())
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

fn event_from_row(row: &PgRow) -> Result<HistoryEvent> {
    Ok(HistoryEvent {
        id: row.try_get("id")?,
        occurred_at: row.try_get("occurred_at")?,
        session_id: row.try_get("session_id")?,
        kind: row.try_get("kind")?,
        paper_id: row.try_get("paper_id")?,
        work_id: row
            .try_get::<Option<Uuid>, _>("work_id")?
            .map(WorkId::from_uuid),
        list_id: row
            .try_get::<Option<Uuid>, _>("list_id")?
            .map(ListId::from_uuid),
        dwell_ms: row.try_get("dwell_ms")?,
        detail: row.try_get("detail")?,
        client_seq: row.try_get("client_seq")?,
        work_title: row.try_get("work_title")?,
    })
}

fn work_dwell_from_row(row: &PgRow) -> Result<WorkDwell> {
    Ok(WorkDwell {
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        opens: row.try_get("opens")?,
        inspects: row.try_get("inspects")?,
        dwell_ms: row.try_get("dwell_ms")?,
        last_seen: row.try_get("last_seen")?,
    })
}

fn position_from_row(row: &PgRow) -> Result<ReadingPosition> {
    Ok(ReadingPosition {
        paper_id: row.try_get("paper_id")?,
        view: row.try_get("view")?,
        page: row.try_get("page")?,
        offset_utf16: row.try_get("offset_utf16")?,
        section_id: row.try_get("section_id")?,
        scroll_ratio: row.try_get("scroll_ratio")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use chrono::Duration;
    use serde_json::json;

    struct Fixture {
        repo: HistoryRepository,
        pool: PgPool,
        workspace: WorkspaceId,
    }

    async fn fixture() -> Option<Fixture> {
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return None;
        };
        let workspace = test_support::new_workspace(&pool).await;
        Some(Fixture {
            repo: HistoryRepository::new(pool.clone()),
            pool,
            workspace,
        })
    }

    fn event(session: Uuid, seq: i32, kind: &str) -> NewEvent {
        NewEvent {
            occurred_at: Utc::now(),
            session_id: session,
            kind: kind.to_owned(),
            paper_id: None,
            work_id: None,
            list_id: None,
            dwell_ms: None,
            detail: json!({}),
            client_seq: seq,
        }
    }

    fn paper_event(
        session: Uuid,
        seq: i32,
        kind: &str,
        paper: &str,
        dwell: Option<i32>,
    ) -> NewEvent {
        NewEvent {
            paper_id: Some(paper.to_owned()),
            dwell_ms: dwell,
            ..event(session, seq, kind)
        }
    }

    #[test]
    fn should_accept_batch_when_events_are_valid() {
        let session = Uuid::new_v4();
        let events: Vec<NewEvent> = HISTORY_KINDS
            .iter()
            .zip(0..)
            .map(|(kind, seq)| event(session, seq, kind))
            .collect();
        assert_eq!(validate_events(&events), Ok(()));
        assert_eq!(validate_events(&[]), Ok(()));
    }

    #[test]
    fn should_reject_batch_when_over_the_limit() {
        let session = Uuid::new_v4();
        let events: Vec<NewEvent> = (0..201)
            .map(|seq| event(session, seq, "page_reached"))
            .collect();
        assert_eq!(
            validate_events(&events),
            Err(HistoryInvalid::BatchTooLarge(201))
        );
        assert_eq!(validate_events(&events[..200]), Ok(()));
    }

    #[test]
    fn should_reject_event_when_kind_is_unknown() {
        let session = Uuid::new_v4();
        let events = [
            event(session, 0, "paper_opened"),
            event(session, 1, "Paper_Opened"),
        ];
        let error = validate_events(&events).expect_err("unknown kind");
        assert_eq!(error.code(), "invalid_history_kind");
        assert!(matches!(
            error,
            HistoryInvalid::UnknownKind { index: 1, .. }
        ));
    }

    #[test]
    fn should_reject_event_when_dwell_is_negative() {
        let mut bad = event(Uuid::new_v4(), 0, "paper_left");
        bad.dwell_ms = Some(-1);
        assert_eq!(
            validate_events(&[bad.clone()]),
            Err(HistoryInvalid::NegativeDwell { index: 0 })
        );
        bad.dwell_ms = Some(0);
        assert_eq!(validate_events(&[bad]), Ok(()));
    }

    #[test]
    fn should_reject_event_when_seq_paper_or_detail_is_malformed() {
        let session = Uuid::new_v4();
        let mut bad = event(session, -1, "page_reached");
        assert_eq!(
            validate_events(&[bad.clone()]),
            Err(HistoryInvalid::NegativeSeq { index: 0 })
        );
        bad.client_seq = 0;
        bad.paper_id = Some(String::new());
        assert_eq!(
            validate_events(&[bad.clone()]),
            Err(HistoryInvalid::BadPaperId { index: 0 })
        );
        bad.paper_id = Some("p".repeat(201));
        assert_eq!(
            validate_events(&[bad.clone()]),
            Err(HistoryInvalid::BadPaperId { index: 0 })
        );
        bad.paper_id = Some("\u{00e9}".repeat(200));
        assert_eq!(validate_events(&[bad.clone()]), Ok(()));
        bad.detail = json!([1]);
        assert_eq!(
            validate_events(&[bad.clone()]),
            Err(HistoryInvalid::BadDetail { index: 0 })
        );
        bad.detail = json!({"query": "x".repeat(DETAIL_MAX_BYTES)});
        assert_eq!(
            validate_events(&[bad]),
            Err(HistoryInvalid::BadDetail { index: 0 })
        );
    }

    #[test]
    fn should_reject_filter_when_kind_is_unknown() {
        assert_eq!(validate_kinds(&[]), Ok(()));
        assert_eq!(validate_kinds(&["search_run".to_owned()]), Ok(()));
        assert_eq!(
            validate_kinds(&["search_run".to_owned(), "nope".to_owned()]),
            Err(HistoryInvalid::UnknownFilterKind("nope".to_owned()))
        );
    }

    fn position(view: &str) -> ReadingPosition {
        ReadingPosition {
            paper_id: "paper".to_owned(),
            view: view.to_owned(),
            page: Some(3),
            offset_utf16: None,
            section_id: None,
            scroll_ratio: Some(0.5),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn should_reject_position_when_out_of_range() {
        assert_eq!(validate_position(&position("pdf")), Ok(()));
        assert_eq!(
            validate_position(&position("epub")),
            Err(HistoryInvalid::UnknownView("epub".to_owned()))
        );
        let mut bad = position("pdf");
        bad.page = Some(-1);
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadPosition));
        bad = position("pdf");
        bad.section_id = Some(String::new());
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadPosition));
        bad = position("pdf");
        bad.scroll_ratio = Some(1.5);
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadScrollRatio));
        bad.scroll_ratio = Some(f32::NAN);
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadScrollRatio));
    }

    #[test]
    fn should_reject_event_when_detail_contains_a_nul() {
        let mut bad = event(Uuid::new_v4(), 0, "search_run");
        bad.detail = json!({"query": "a\u{0}b"});
        assert_eq!(
            validate_events(&[bad.clone()]),
            Err(HistoryInvalid::BadDetail { index: 0 })
        );
        bad.detail = json!({"nested": [{"k\u{0}": 1}]});
        assert_eq!(
            validate_events(&[bad]),
            Err(HistoryInvalid::BadDetail { index: 0 })
        );
    }

    #[test]
    fn should_reject_position_when_paper_or_section_contains_a_nul() {
        let mut bad = position("pdf");
        bad.paper_id = "a\u{0}b".to_owned();
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadPosition));
        bad = position("pdf");
        bad.section_id = Some("s\u{0}".to_owned());
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadPosition));
        bad = position("pdf");
        bad.paper_id = "p".repeat(PAPER_ID_MAX + 1);
        assert_eq!(validate_position(&bad), Err(HistoryInvalid::BadPosition));
    }

    #[test]
    fn should_escape_wildcards_when_building_like_pattern() {
        assert_eq!(like_pattern("50%_a\\"), "%50\\%\\_a\\\\%");
        assert_eq!(like_pattern("caf\u{00e9}"), "%caf\u{00e9}%");
    }

    #[tokio::test]
    async fn should_count_duplicates_when_batch_is_resent() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let batch = vec![
            event(session, 0, "paper_opened"),
            event(session, 1, "page_reached"),
            event(session, 1, "page_reached"),
        ];
        let first = fixture
            .repo
            .insert_batch(fixture.workspace, &batch)
            .await
            .expect("first");
        assert_eq!(
            first,
            BatchOutcome {
                accepted: 2,
                duplicates: 1
            }
        );

        let mut next = batch.clone();
        next.push(event(session, 2, "paper_left"));
        let second = fixture
            .repo
            .insert_batch(fixture.workspace, &next)
            .await
            .expect("second");
        assert_eq!(
            second,
            BatchOutcome {
                accepted: 1,
                duplicates: 3
            }
        );

        let empty = fixture
            .repo
            .insert_batch(fixture.workspace, &[])
            .await
            .expect("empty");
        assert_eq!(empty, BatchOutcome::default());
    }

    #[tokio::test]
    async fn should_attach_work_only_to_that_papers_workless_events_when_backfilling() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let work = test_support::new_work(&fixture.pool).await;
        let kept = test_support::new_work(&fixture.pool).await;
        let session = Uuid::new_v4();
        let mut already = paper_event(session, 2, "page_reached", "paper-a", None);
        already.work_id = Some(kept);
        let events = [
            paper_event(session, 0, "paper_opened", "paper-a", None),
            paper_event(session, 1, "paper_left", "paper-a", Some(5_000)),
            already,
            paper_event(session, 3, "paper_opened", "paper-b", None),
        ];
        fixture
            .repo
            .insert_batch(fixture.workspace, &events)
            .await
            .expect("insert");
        let other_workspace = test_support::new_workspace(&fixture.pool).await;
        fixture
            .repo
            .insert_batch(
                other_workspace,
                &[paper_event(
                    Uuid::new_v4(),
                    0,
                    "paper_opened",
                    "paper-a",
                    None,
                )],
            )
            .await
            .expect("insert other");

        let changed = fixture
            .repo
            .attach_work_to_paper(fixture.workspace, "paper-a", work)
            .await
            .expect("attach");
        assert_eq!(changed, 2, "only the two workless paper-a events");
        let missing = fixture
            .repo
            .attach_work_to_paper(fixture.workspace, "paper-b", WorkId::new())
            .await
            .expect("attach missing");
        assert_eq!(missing, 0, "a Work that does not exist changes nothing");

        let rows = fixture
            .repo
            .recent(fixture.workspace, &HistoryQuery::default())
            .await
            .expect("recent");
        let work_of = |seq: i32| {
            rows.iter()
                .find(|row| row.client_seq == seq)
                .and_then(|row| row.work_id)
        };
        assert_eq!(work_of(0), Some(work));
        assert_eq!(work_of(1), Some(work));
        assert_eq!(work_of(2), Some(kept), "a known Work is not overwritten");
        assert_eq!(work_of(3), None, "another paper is untouched");
        let other_rows = fixture
            .repo
            .recent(other_workspace, &HistoryQuery::default())
            .await
            .expect("recent other");
        assert_eq!(
            other_rows[0].work_id, None,
            "another workspace is untouched"
        );
    }

    #[tokio::test]
    async fn should_store_null_work_when_work_does_not_exist() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let mut stale = event(session, 0, "work_inspected");
        stale.work_id = Some(WorkId::new());
        stale.list_id = Some(ListId::new());
        let outcome = fixture
            .repo
            .insert_batch(fixture.workspace, &[stale])
            .await
            .expect("insert");
        assert_eq!(outcome.accepted, 1);
        let rows = fixture
            .repo
            .recent(fixture.workspace, &HistoryQuery::default())
            .await
            .expect("recent");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].work_id, None);
        assert_eq!(rows[0].list_id, None);
    }

    #[tokio::test]
    async fn should_page_newest_first_when_using_a_cursor() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let now = Utc::now();
        let batch: Vec<NewEvent> = (0..5)
            .map(|seq| NewEvent {
                occurred_at: now - Duration::minutes(i64::from(5 - seq)),
                ..event(session, seq, "page_reached")
            })
            .collect();
        fixture
            .repo
            .insert_batch(fixture.workspace, &batch)
            .await
            .expect("insert");
        let first = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    limit: Some(2),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("page one");
        assert_eq!(
            first.iter().map(|e| e.client_seq).collect::<Vec<_>>(),
            vec![4, 3]
        );
        let last = first.last().expect("last");
        let rest = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    before: Some(HistoryCursor {
                        occurred_at: last.occurred_at,
                        id: last.id,
                    }),
                    limit: Some(10),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("page two");
        assert_eq!(
            rest.iter().map(|e| e.client_seq).collect::<Vec<_>>(),
            vec![2, 1, 0]
        );
        let clamped = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    limit: Some(-4),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("clamped");
        assert_eq!(clamped.len(), 1);
    }

    #[tokio::test]
    async fn should_break_ties_by_id_when_events_share_an_instant() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let at = Utc::now();
        let batch: Vec<NewEvent> = (0..3)
            .map(|seq| NewEvent {
                occurred_at: at,
                ..event(session, seq, "page_reached")
            })
            .collect();
        fixture
            .repo
            .insert_batch(fixture.workspace, &batch)
            .await
            .expect("insert");
        let all = fixture
            .repo
            .recent(fixture.workspace, &HistoryQuery::default())
            .await
            .expect("all");
        let after_first = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    before: Some(HistoryCursor {
                        occurred_at: all[0].occurred_at,
                        id: all[0].id,
                    }),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("after");
        assert_eq!(all.len(), 3);
        assert_eq!(after_first.len(), 2);
    }

    #[tokio::test]
    async fn should_filter_by_kind_and_text_when_querying_timeline() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let mut search = event(session, 0, "search_run");
        search.detail = json!({"query": "Caf\u{00e9} 100% sure_thing"});
        let mut other = event(session, 1, "search_run");
        other.detail = json!({"query": "100 percent surexthing"});
        let mut link = event(session, 2, "link_followed");
        link.detail = json!({"target": "cafe-society"});
        let work = test_support::new_work(&fixture.pool).await;
        sqlx::query("update graph.work set display_title = 'Zebra Migration' where id = $1")
            .bind(work.as_uuid())
            .execute(&fixture.pool)
            .await
            .expect("title");
        let mut inspected = event(session, 3, "work_inspected");
        inspected.work_id = Some(work);
        fixture
            .repo
            .insert_batch(fixture.workspace, &[search, other, link, inspected])
            .await
            .expect("insert");

        let seqs = |rows: Vec<HistoryEvent>| rows.iter().map(|e| e.client_seq).collect::<Vec<_>>();
        let by_kind = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    kinds: vec!["search_run".to_owned()],
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("kind");
        assert_eq!(by_kind.len(), 2);

        let literal = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    text: Some("100% sure_".to_owned()),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("literal wildcards");
        assert_eq!(seqs(literal), vec![0]);

        let unicode = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    text: Some("CAF\u{00e9}".to_owned()),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("unicode");
        assert_eq!(seqs(unicode), vec![0]);

        let by_title = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    text: Some("zebra".to_owned()),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("title");
        assert_eq!(seqs(by_title.clone()), vec![3]);
        assert_eq!(by_title[0].work_title.as_deref(), Some("Zebra Migration"));

        let blank = fixture
            .repo
            .recent(
                fixture.workspace,
                &HistoryQuery {
                    text: Some("   ".to_owned()),
                    ..HistoryQuery::default()
                },
            )
            .await
            .expect("blank");
        assert_eq!(blank.len(), 4);
    }

    #[tokio::test]
    async fn should_not_leak_events_when_querying_another_workspace() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let other = test_support::new_workspace(&fixture.pool).await;
        fixture
            .repo
            .insert_batch(
                fixture.workspace,
                &[event(Uuid::new_v4(), 0, "digest_opened")],
            )
            .await
            .expect("insert");
        let rows = fixture
            .repo
            .recent(other, &HistoryQuery::default())
            .await
            .expect("recent");
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn should_total_visits_dwell_and_sections_when_aggregating_a_paper() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let mut intro = paper_event(session, 2, "section_focused", "p1", Some(4000));
        intro.detail = json!({"section_id": "intro"});
        let mut intro_again = paper_event(session, 3, "section_focused", "p1", Some(1000));
        intro_again.detail = json!({"section_id": "intro"});
        let mut methods = paper_event(session, 4, "section_focused", "p1", Some(9000));
        methods.detail = json!({"section_id": "methods"});
        let batch = vec![
            paper_event(session, 0, "paper_opened", "p1", None),
            paper_event(session, 1, "paper_left", "p1", Some(60_000)),
            intro,
            intro_again,
            methods,
            paper_event(session, 5, "paper_opened", "p1", None),
            paper_event(session, 6, "paper_left", "p1", Some(30_000)),
            paper_event(session, 7, "paper_opened", "p2", None),
        ];
        fixture
            .repo
            .insert_batch(fixture.workspace, &batch)
            .await
            .expect("insert");
        let aggregate = fixture
            .repo
            .paper_aggregate(fixture.workspace, "p1")
            .await
            .expect("aggregate");
        assert_eq!(aggregate.visits, 2);
        assert_eq!(aggregate.dwell_ms, 90_000);
        assert!(aggregate.first_seen.is_some());
        assert_eq!(
            aggregate.sections,
            vec![
                SectionDwell {
                    section_id: "methods".to_owned(),
                    visits: 1,
                    dwell_ms: 9000
                },
                SectionDwell {
                    section_id: "intro".to_owned(),
                    visits: 2,
                    dwell_ms: 5000
                },
            ]
        );

        let unknown = fixture
            .repo
            .paper_aggregate(fixture.workspace, "missing")
            .await
            .expect("unknown");
        assert_eq!(unknown.visits, 0);
        assert_eq!(unknown.dwell_ms, 0);
        assert_eq!(unknown.first_seen, None);
        assert!(unknown.sections.is_empty());

        let totals = fixture
            .repo
            .paper_dwell(fixture.workspace)
            .await
            .expect("totals");
        let by_id = |id: &str| totals.iter().find(|p| p.paper_id == id).expect("paper");
        assert_eq!(totals.len(), 2);
        assert_eq!(by_id("p1").dwell_ms, 90_000);
        assert_eq!(by_id("p2").dwell_ms, 0);
        assert_eq!(by_id("p2").visits, 1);
    }

    #[tokio::test]
    async fn should_aggregate_work_dwell_and_find_inspected_only_works_when_some_works_were_never_opened()
     {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let opened = test_support::new_work(&fixture.pool).await;
        let peeked = test_support::new_work(&fixture.pool).await;
        let with_work = |work: WorkId, mut base: NewEvent| {
            base.work_id = Some(work);
            base
        };
        let batch = vec![
            with_work(opened, paper_event(session, 0, "paper_opened", "p1", None)),
            with_work(
                opened,
                paper_event(session, 1, "paper_left", "p1", Some(200_000)),
            ),
            with_work(opened, event(session, 2, "work_inspected")),
            with_work(peeked, event(session, 3, "work_inspected")),
            with_work(peeked, event(session, 4, "work_inspected")),
        ];
        fixture
            .repo
            .insert_batch(fixture.workspace, &batch)
            .await
            .expect("insert");
        let dwell = fixture
            .repo
            .work_dwell(fixture.workspace)
            .await
            .expect("dwell");
        let opened_row = dwell.iter().find(|w| w.work_id == opened).expect("opened");
        assert_eq!(
            (opened_row.opens, opened_row.inspects, opened_row.dwell_ms),
            (1, 1, 200_000)
        );
        let peeked_row = dwell.iter().find(|w| w.work_id == peeked).expect("peeked");
        assert_eq!((peeked_row.opens, peeked_row.dwell_ms), (0, 0));
        assert_eq!(
            fixture
                .repo
                .work_dwell_ms(fixture.workspace, opened)
                .await
                .expect("single"),
            200_000
        );
        assert_eq!(
            fixture
                .repo
                .work_dwell_ms(fixture.workspace, peeked)
                .await
                .expect("zero"),
            0
        );

        let inspected = fixture
            .repo
            .inspected_never_opened(fixture.workspace)
            .await
            .expect("inspected");
        assert_eq!(inspected.len(), 1);
        assert_eq!(inspected[0].work_id, peeked);
        assert_eq!(inspected[0].inspects, 2);
    }

    #[tokio::test]
    async fn should_repoint_events_when_works_merge() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let from = test_support::new_work(&fixture.pool).await;
        let into = test_support::new_work(&fixture.pool).await;
        let mut peek = event(Uuid::new_v4(), 0, "work_inspected");
        peek.work_id = Some(from);
        fixture
            .repo
            .insert_batch(fixture.workspace, &[peek])
            .await
            .expect("insert");
        let mut conn = fixture.pool.acquire().await.expect("conn");
        let moved = repoint_work(&mut conn, from, into).await.expect("repoint");
        assert_eq!(moved, 1);
        let dwell = fixture
            .repo
            .work_dwell(fixture.workspace)
            .await
            .expect("dwell");
        assert_eq!(dwell.len(), 1);
        assert_eq!(dwell[0].work_id, into);
    }

    #[tokio::test]
    async fn should_export_and_prune_by_time_when_cutoff_given() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let session = Uuid::new_v4();
        let now = Utc::now();
        let old = NewEvent {
            occurred_at: now - Duration::days(10),
            ..event(session, 0, "digest_opened")
        };
        let fresh = NewEvent {
            occurred_at: now,
            ..event(session, 1, "digest_opened")
        };
        fixture
            .repo
            .insert_batch(fixture.workspace, &[old, fresh])
            .await
            .expect("insert");
        let cutoff = now - Duration::days(1);
        let exported = fixture
            .repo
            .since(fixture.workspace, cutoff)
            .await
            .expect("since");
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].client_seq, 1);
        let pruned = fixture
            .repo
            .prune_before(fixture.workspace, cutoff)
            .await
            .expect("prune");
        assert_eq!(pruned, 1);
        let left = fixture
            .repo
            .recent(fixture.workspace, &HistoryQuery::default())
            .await
            .expect("left");
        assert_eq!(left.len(), 1);
    }

    #[tokio::test]
    async fn should_replace_position_when_upserting_twice() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        assert_eq!(
            fixture
                .repo
                .position(fixture.workspace, "paper")
                .await
                .expect("none"),
            None
        );
        let first = fixture
            .repo
            .upsert_position(fixture.workspace, &position("pdf"))
            .await
            .expect("first");
        assert_eq!(first.page, Some(3));
        let mut moved = position("section");
        moved.page = None;
        moved.section_id = Some("m\u{00e9}thodes".to_owned());
        moved.offset_utf16 = Some(120);
        fixture
            .repo
            .upsert_position(fixture.workspace, &moved)
            .await
            .expect("second");
        let stored = fixture
            .repo
            .position(fixture.workspace, "paper")
            .await
            .expect("get")
            .expect("present");
        assert_eq!(stored.view, "section");
        assert_eq!(stored.page, None);
        assert_eq!(stored.section_id.as_deref(), Some("m\u{00e9}thodes"));
        assert_eq!(stored.offset_utf16, Some(120));
        assert!(stored.updated_at >= first.updated_at);

        let other = test_support::new_workspace(&fixture.pool).await;
        assert_eq!(
            fixture.repo.position(other, "paper").await.expect("other"),
            None
        );
    }

    #[tokio::test]
    async fn should_enforce_check_constraints_when_inserting_raw_rows() {
        let _guard = test_support::lock().await;
        let Some(fixture) = fixture().await else {
            return;
        };
        let bad_kind = sqlx::query(
            "insert into reader.history_event (workspace_id, occurred_at, session_id, kind, client_seq) \
             values ($1, now(), $2, 'bogus', 0)",
        )
        .bind(fixture.workspace.as_uuid())
        .bind(Uuid::new_v4())
        .execute(&fixture.pool)
        .await;
        assert!(bad_kind.is_err());
        let bad_ratio = sqlx::query(
            "insert into reader.reading_position (workspace_id, paper_id, view, scroll_ratio) \
             values ($1, 'p', 'pdf', 2)",
        )
        .bind(fixture.workspace.as_uuid())
        .execute(&fixture.pool)
        .await;
        assert!(bad_ratio.is_err());
    }
}
