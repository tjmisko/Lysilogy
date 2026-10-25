//! Reading history: the service shared by the API and the CLI.
//!
//! Events arrive in client batches and are appended (see `lysilogy_db::history`). This
//! module derives from them: the timeline grouped by session, per-paper summaries, Works
//! inspected but never opened, papers opened but barely read, the D17 reading nudge, and
//! the D16 knowledge profile that list generation shares with its tests. The derivations
//! are pure functions; [`HistoryService`] only fetches and writes.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
};

use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use lysilogy_core::{ListId, ReadingStatus, WorkId, WorkspaceId};
use lysilogy_db::{
    DbError,
    history::{
        HISTORY_PAGE_DEFAULT, HISTORY_PAGE_MAX, HistoryCursor, HistoryEvent, HistoryInvalid,
        HistoryQuery, HistoryRepository, InspectedWork, NewEvent, PaperAggregate, PaperDwell,
        ReadingPosition, SectionDwell, validate_events, validate_kinds, validate_position,
    },
    list::{ArchiveScope, ListRepository},
    reading_state::ReadingStateRepository,
    work::WorkRepository,
};
use serde::Serialize;

use crate::{Error, graph::GraphState};

/// Active dwell, in milliseconds, on one visit that nudges an unread Work to `reading`
/// (D17).
pub const NUDGE_DWELL_MS: i32 = 3 * 60 * 1000;
/// A paper opened with less total dwell than this counts as barely read.
pub const BARELY_READ_MS: i64 = 2 * 60 * 1000;
/// Most Works in the knowledge profile (D16).
pub const PROFILE_MAX_WORKS: usize = 120;
/// Works that are neither read nor reading only enter the profile when seen this recently.
pub const PROFILE_WINDOW_DAYS: i64 = 180;
/// Half-life of a Work's score, in days.
const RECENCY_HALF_LIFE_DAYS: f64 = 90.0;
/// Longest `--since` or `--before` age the CLI accepts, in days (about 100 years).
const MAX_AGE_DAYS: i64 = 36_500;

/// Why a history operation failed.
#[derive(Debug, thiserror::Error)]
pub enum HistoryServiceError {
    /// `paper_not_found` or `work_not_found`.
    #[error("{}", not_found_message(.0))]
    NotFound(&'static str),
    /// The input was refused: a code such as `invalid_history_kind` and a message.
    #[error("{1}")]
    Invalid(&'static str, String),
    /// The graph database is missing or a query failed.
    #[error("{0}")]
    Graph(Error),
}

fn not_found_message(code: &str) -> &'static str {
    match code {
        "paper_not_found" => "No history for that paper.",
        _ => "No such work.",
    }
}

impl HistoryServiceError {
    /// The machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound(code) | Self::Invalid(code, _) => code,
            Self::Graph(_) => "graph_error",
        }
    }
}

impl From<HistoryServiceError> for Error {
    fn from(error: HistoryServiceError) -> Self {
        match error {
            HistoryServiceError::Graph(error) => error,
            other => Self::InvalidRequest(other.to_string()),
        }
    }
}

impl From<HistoryInvalid> for HistoryServiceError {
    fn from(error: HistoryInvalid) -> Self {
        Self::Invalid(error.code(), error.to_string())
    }
}

fn database(error: &DbError) -> HistoryServiceError {
    HistoryServiceError::Graph(Error::Literature(error.to_string()))
}

fn invalid(code: &'static str, message: &str) -> HistoryServiceError {
    HistoryServiceError::Invalid(code, message.to_owned())
}

// ---------------------------------------------------------------------------------------
// Pure derivations
// ---------------------------------------------------------------------------------------

/// The Works whose `paper_left` events in a batch reach the nudge threshold (D17),
/// resolved by the caller through redirects. Sorted and distinct.
#[must_use]
pub fn nudge_candidates(events: &[NewEvent]) -> Vec<WorkId> {
    let mut works: Vec<WorkId> = events
        .iter()
        .filter(|event| event.kind == "paper_left")
        .filter(|event| event.dwell_ms.is_some_and(|dwell| dwell >= NUDGE_DWELL_MS))
        .filter_map(|event| event.work_id)
        .collect();
    works.sort();
    works.dedup();
    works
}

/// Papers opened but with total dwell under [`BARELY_READ_MS`], in the input's order.
#[must_use]
pub fn barely_read(papers: &[PaperDwell]) -> Vec<PaperDwell> {
    papers
        .iter()
        .filter(|paper| paper.visits > 0 && paper.dwell_ms < BARELY_READ_MS)
        .cloned()
        .collect()
}

/// One browsing session in the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionGroup {
    pub session_id: uuid::Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Newest first, as fetched.
    pub events: Vec<HistoryEvent>,
}

/// Group a newest-first event list by session. Groups appear in the order their newest
/// event appears, so the timeline stays newest-first.
#[must_use]
pub fn group_sessions(events: Vec<HistoryEvent>) -> Vec<SessionGroup> {
    let mut groups: Vec<SessionGroup> = Vec::new();
    let mut index: HashMap<uuid::Uuid, usize> = HashMap::new();
    for event in events {
        let Some(&at) = index.get(&event.session_id) else {
            index.insert(event.session_id, groups.len());
            groups.push(SessionGroup {
                session_id: event.session_id,
                started_at: event.occurred_at,
                ended_at: event.occurred_at,
                events: vec![event],
            });
            continue;
        };
        let group = &mut groups[at];
        group.started_at = group.started_at.min(event.occurred_at);
        group.ended_at = group.ended_at.max(event.occurred_at);
        group.events.push(event);
    }
    groups
}

/// The cursor string for the next timeline page: a microsecond RFC 3339 time and the row ID.
#[must_use]
pub fn encode_cursor(cursor: HistoryCursor) -> String {
    format!(
        "{}_{}",
        cursor
            .occurred_at
            .to_rfc3339_opts(SecondsFormat::Micros, true),
        cursor.id
    )
}

/// Parse a timeline cursor: `<rfc3339>` (every event at that instant is older) or
/// `<rfc3339>_<id>`.
///
/// # Errors
/// `Invalid("invalid_history_cursor", …)`.
pub fn parse_cursor(text: &str) -> Result<HistoryCursor, HistoryServiceError> {
    let bad = || {
        invalid(
            "invalid_history_cursor",
            "The cursor is an RFC 3339 time, optionally followed by _ and an event number.",
        )
    };
    let (time, id) = match text.rsplit_once('_') {
        Some((time, id)) => (time, id.parse::<i64>().map_err(|_| bad())?),
        None => (text, i64::MAX),
    };
    let occurred_at = DateTime::parse_from_rfc3339(time.trim())
        .map_err(|_| bad())?
        .with_timezone(&Utc);
    Ok(HistoryCursor { occurred_at, id })
}

/// Parse a point in time: an age such as `7d`, `12h`, `90m` or `2w` before `now`, an RFC 3339
/// time, or a `YYYY-MM-DD` date (midnight UTC).
///
/// # Errors
/// `Invalid("invalid_history_time", …)`.
pub fn parse_when(text: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, HistoryServiceError> {
    let bad = || {
        invalid(
            "invalid_history_time",
            "Use an age like 7d, 12h, 90m or 2w, a date like 2026-10-01, or an RFC 3339 time.",
        )
    };
    let text = text.trim();
    if let Some(unit) = text.chars().last().filter(char::is_ascii_alphabetic) {
        let digits = &text[..text.len() - 1];
        let amount: i64 = digits.parse().map_err(|_| bad())?;
        if amount < 0 {
            return Err(bad());
        }
        let per_unit = match unit {
            'w' => 7 * 24 * 60,
            'd' => 24 * 60,
            'h' => 60,
            'm' => 1,
            _ => return Err(bad()),
        };
        let minutes = amount.checked_mul(per_unit).ok_or_else(bad)?;
        if minutes > MAX_AGE_DAYS * 24 * 60 {
            return Err(bad());
        }
        return Ok(now - Duration::minutes(minutes));
    }
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Ok(time.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| bad())?;
    let midnight = date.and_hms_opt(0, 0, 0).ok_or_else(bad)?;
    Ok(Utc.from_utc_datetime(&midnight))
}

/// One event as a JSON line.
#[must_use]
pub fn export_jsonl(events: &[HistoryEvent]) -> String {
    let mut out = String::new();
    for event in events {
        // Serializing plain strings, numbers and a JSON value cannot fail.
        if let Ok(line) = serde_json::to_string(event) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// What the history knows about one paper.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaperSummary {
    pub paper_id: String,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    pub visits: i64,
    /// Total `paper_left` dwell, in milliseconds.
    pub dwell_ms: i64,
    /// Sections by dwell, most first.
    pub sections: Vec<SectionDwell>,
    /// Opened at least once, with less than [`BARELY_READ_MS`] of dwell.
    pub barely_read: bool,
    pub position: Option<ReadingPosition>,
}

impl PaperSummary {
    /// Combine a paper's aggregate with its saved position.
    #[must_use]
    pub fn new(aggregate: PaperAggregate, position: Option<ReadingPosition>) -> Self {
        Self {
            barely_read: aggregate.visits > 0 && aggregate.dwell_ms < BARELY_READ_MS,
            paper_id: aggregate.paper_id,
            first_seen: aggregate.first_seen,
            last_seen: aggregate.last_seen,
            visits: aggregate.visits,
            dwell_ms: aggregate.dwell_ms,
            sections: aggregate.sections,
            position,
        }
    }

    /// Nothing is recorded for the paper.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.first_seen.is_none() && self.position.is_none()
    }
}

/// A Work's engagement, as the knowledge profile reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngagementInput {
    pub work_id: WorkId,
    pub dwell_ms: i64,
    pub last_seen: DateTime<Utc>,
}

/// A stored reading status, as the knowledge profile reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusInput {
    pub work_id: WorkId,
    pub status: ReadingStatus,
    pub changed_at: DateTime<Utc>,
}

/// A list and its Work members, as the knowledge profile reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileList {
    pub id: ListId,
    pub title: String,
    pub works: Vec<WorkId>,
}

/// A list a profile Work is in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileListRef {
    pub id: ListId,
    pub title: String,
}

/// One Work the reader is taken to know.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProfileEntry {
    pub work_id: WorkId,
    /// Filled by [`HistoryService::knowledge_profile`]; the pure ranking has no titles.
    pub title: Option<String>,
    pub year: Option<i32>,
    pub status: ReadingStatus,
    pub dwell_ms: i64,
    /// The latest of the Work's last visit and its last status change.
    pub last_active: DateTime<Utc>,
    pub score: f64,
    pub lists: Vec<ProfileListRef>,
}

/// The ranked knowledge profile.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Profile {
    /// Best first, at most [`PROFILE_MAX_WORKS`].
    pub entries: Vec<ProfileEntry>,
    /// Qualifying Works the cap cut off.
    pub omitted: usize,
}

/// The weight a status carries; Works with no status, or `unread`, that were dwelt on
/// count 0.3.
const fn status_weight(status: ReadingStatus, dwell_ms: i64) -> f64 {
    match status {
        ReadingStatus::Read => 1.0,
        ReadingStatus::Reading => 0.6,
        ReadingStatus::Unread => {
            if dwell_ms > 0 {
                0.3
            } else {
                0.0
            }
        }
    }
}

/// Rank what the reader knows (D16).
///
/// A Work's score is
/// `status_weight × ln(1 + dwell_minutes) × 0.5^(age_days / 90)`, where `age_days` counts
/// from the later of its last visit and last status change. Works marked `read` or
/// `reading` always qualify, even with no dwell (their score is then 0); other Works need
/// dwell and a visit within [`PROFILE_WINDOW_DAYS`]. Ties break by Work ID, so the result
/// is the same under any input order. The top [`PROFILE_MAX_WORKS`] are kept.
#[must_use]
#[allow(clippy::cast_precision_loss)] // seconds and milliseconds of dwell fit f64 exactly
pub fn knowledge_profile(
    engagement: &[EngagementInput],
    states: &[StatusInput],
    lists: &[ProfileList],
    now: DateTime<Utc>,
) -> Profile {
    struct Acc {
        dwell_ms: i64,
        last_seen: Option<DateTime<Utc>>,
        status: ReadingStatus,
        changed_at: Option<DateTime<Utc>>,
    }
    let mut accs: BTreeMap<WorkId, Acc> = BTreeMap::new();
    let fresh = |_: WorkId| Acc {
        dwell_ms: 0,
        last_seen: None,
        status: ReadingStatus::Unread,
        changed_at: None,
    };
    for input in engagement {
        let acc = accs
            .entry(input.work_id)
            .or_insert_with(|| fresh(input.work_id));
        acc.dwell_ms += input.dwell_ms.max(0);
        acc.last_seen = acc.last_seen.max(Some(input.last_seen));
    }
    for input in states {
        let acc = accs
            .entry(input.work_id)
            .or_insert_with(|| fresh(input.work_id));
        acc.status = input.status;
        acc.changed_at = acc.changed_at.max(Some(input.changed_at));
    }
    let mut memberships: HashMap<WorkId, Vec<ProfileListRef>> = HashMap::new();
    for list in lists {
        let mut seen: HashSet<WorkId> = HashSet::new();
        for work in &list.works {
            if seen.insert(*work) {
                memberships.entry(*work).or_default().push(ProfileListRef {
                    id: list.id,
                    title: list.title.clone(),
                });
            }
        }
    }
    let window = Duration::days(PROFILE_WINDOW_DAYS);
    let mut entries: Vec<ProfileEntry> = Vec::new();
    for (work_id, acc) in accs {
        let Some(last_active) = acc.last_seen.max(acc.changed_at) else {
            continue;
        };
        let known = matches!(acc.status, ReadingStatus::Read | ReadingStatus::Reading);
        if !known && (acc.dwell_ms <= 0 || now - last_active > window) {
            continue;
        }
        let age_days = ((now - last_active).num_seconds().max(0)) as f64 / 86_400.0;
        let dwell_minutes = acc.dwell_ms as f64 / 60_000.0;
        let score = status_weight(acc.status, acc.dwell_ms)
            * dwell_minutes.ln_1p()
            * 0.5_f64.powf(age_days / RECENCY_HALF_LIFE_DAYS);
        let mut member_of = memberships.remove(&work_id).unwrap_or_default();
        member_of.sort_by_key(|list| list.id);
        entries.push(ProfileEntry {
            work_id,
            title: None,
            year: None,
            status: acc.status,
            dwell_ms: acc.dwell_ms,
            last_active,
            score,
            lists: member_of,
        });
    }
    entries.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.work_id.cmp(&b.work_id))
    });
    let omitted = entries.len().saturating_sub(PROFILE_MAX_WORKS);
    entries.truncate(PROFILE_MAX_WORKS);
    Profile { entries, omitted }
}

// ---------------------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------------------

/// What a batch ingest did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IngestOutcome {
    pub accepted: usize,
    pub duplicates: usize,
    /// Works the D17 nudge moved from `unread` to `reading`.
    pub nudged: Vec<WorkId>,
}

/// One page of the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Timeline {
    pub sessions: Vec<SessionGroup>,
    /// Pass as `before` for the next page; `None` when this was the last.
    pub next_before: Option<String>,
}

/// A Work inspected but never opened, with its title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InspectedEntry {
    pub work_id: WorkId,
    pub title: Option<String>,
    pub inspects: i64,
    pub last_inspected: DateTime<Utc>,
}

/// The history service for the default workspace.
#[derive(Clone)]
pub struct HistoryService {
    history: HistoryRepository,
    reading: ReadingStateRepository,
    lists: ListRepository,
    works: WorkRepository,
    workspace: WorkspaceId,
}

impl HistoryService {
    /// Build the service over the graph's pool and resolve the default workspace.
    ///
    /// # Errors
    /// `Graph` when the workspace cannot be resolved.
    pub async fn connect(graph: &GraphState) -> Result<Self, HistoryServiceError> {
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .map_err(|error| database(&error))?;
        let pool = graph.works.pool().clone();
        Ok(Self {
            history: HistoryRepository::new(pool.clone()),
            reading: ReadingStateRepository::new(pool.clone()),
            lists: ListRepository::new(pool),
            works: graph.works.clone(),
            workspace,
        })
    }

    /// Store a batch, then apply the D17 nudge for visits that reached the threshold. A
    /// batch that stored nothing (a replay) never nudges.
    ///
    /// # Errors
    /// `Invalid` for a rejected batch; `Graph` for a database failure.
    pub async fn ingest(&self, events: &[NewEvent]) -> Result<IngestOutcome, HistoryServiceError> {
        validate_events(events)?;
        let outcome = self
            .history
            .insert_batch(self.workspace, events)
            .await
            .map_err(|error| database(&error))?;
        let mut nudged = Vec::new();
        if outcome.accepted == 0 {
            return Ok(IngestOutcome {
                accepted: 0,
                duplicates: outcome.duplicates,
                nudged,
            });
        }
        let mut resolved: Vec<WorkId> = Vec::new();
        for work in nudge_candidates(events) {
            let work = self
                .works
                .resolve_redirect(work)
                .await
                .map_err(|error| database(&error))?;
            resolved.push(work);
        }
        resolved.sort();
        resolved.dedup();
        for work in resolved {
            let exists = self
                .works
                .get(work)
                .await
                .map_err(|error| database(&error))?
                .is_some();
            if !exists {
                continue;
            }
            let changed = self
                .reading
                .nudge_to_reading(self.workspace, work)
                .await
                .map_err(|error| database(&error))?;
            if changed {
                nudged.push(work);
            }
        }
        Ok(IngestOutcome {
            accepted: outcome.accepted,
            duplicates: outcome.duplicates,
            nudged,
        })
    }

    /// The newest events, grouped by session.
    ///
    /// # Errors
    /// `Invalid` for an unknown kind; `Graph` for a database failure.
    pub async fn timeline(&self, query: &HistoryQuery) -> Result<Timeline, HistoryServiceError> {
        validate_kinds(&query.kinds)?;
        let limit = query
            .limit
            .unwrap_or(HISTORY_PAGE_DEFAULT)
            .clamp(1, HISTORY_PAGE_MAX);
        let events = self
            .history
            .recent(self.workspace, query)
            .await
            .map_err(|error| database(&error))?;
        #[allow(clippy::cast_possible_wrap)] // a page holds at most HISTORY_PAGE_MAX rows
        let full = events.len() as i64 >= limit;
        let next_before = if full {
            events.last().map(|event| {
                encode_cursor(HistoryCursor {
                    occurred_at: event.occurred_at,
                    id: event.id,
                })
            })
        } else {
            None
        };
        Ok(Timeline {
            sessions: group_sessions(events),
            next_before,
        })
    }

    /// Everything known about one paper.
    ///
    /// # Errors
    /// `NotFound("paper_not_found")` when the paper has neither events nor a position.
    pub async fn paper_summary(&self, paper_id: &str) -> Result<PaperSummary, HistoryServiceError> {
        let aggregate = self
            .history
            .paper_aggregate(self.workspace, paper_id)
            .await
            .map_err(|error| database(&error))?;
        let position = self
            .history
            .position(self.workspace, paper_id)
            .await
            .map_err(|error| database(&error))?;
        let summary = PaperSummary::new(aggregate, position);
        if summary.is_empty() {
            return Err(HistoryServiceError::NotFound("paper_not_found"));
        }
        Ok(summary)
    }

    /// Save where the reader got to in a paper.
    ///
    /// # Errors
    /// `Invalid("invalid_position", …)` for a bad view, field or ratio.
    pub async fn save_position(
        &self,
        position: &ReadingPosition,
    ) -> Result<ReadingPosition, HistoryServiceError> {
        validate_position(position)?;
        self.history
            .upsert_position(self.workspace, position)
            .await
            .map_err(|error| database(&error))
    }

    /// Works that were inspected but never opened, most recently inspected first.
    ///
    /// # Errors
    /// `Graph` for a database failure.
    pub async fn inspected_never_opened(&self) -> Result<Vec<InspectedEntry>, HistoryServiceError> {
        let inspected: Vec<InspectedWork> = self
            .history
            .inspected_never_opened(self.workspace)
            .await
            .map_err(|error| database(&error))?;
        let ids: Vec<WorkId> = inspected.iter().map(|work| work.work_id).collect();
        let titles = self.titles(&ids).await?;
        Ok(inspected
            .into_iter()
            .map(|work| InspectedEntry {
                title: titles.get(&work.work_id).cloned().flatten(),
                work_id: work.work_id,
                inspects: work.inspects,
                last_inspected: work.last_inspected,
            })
            .collect())
    }

    /// Papers opened but barely read, most recently seen first.
    ///
    /// # Errors
    /// `Graph` for a database failure.
    pub async fn barely_read(&self) -> Result<Vec<PaperDwell>, HistoryServiceError> {
        let papers = self
            .history
            .paper_dwell(self.workspace)
            .await
            .map_err(|error| database(&error))?;
        Ok(barely_read(&papers))
    }

    /// The knowledge profile (D16) as of `now`, with titles and years filled in.
    ///
    /// # Errors
    /// `Graph` for a database failure.
    pub async fn knowledge_profile(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Profile, HistoryServiceError> {
        let dwell = self
            .history
            .work_dwell(self.workspace)
            .await
            .map_err(|error| database(&error))?;
        let engagement: Vec<EngagementInput> = dwell
            .iter()
            .map(|work| EngagementInput {
                work_id: work.work_id,
                dwell_ms: work.dwell_ms,
                last_seen: work.last_seen,
            })
            .collect();
        let summaries = self
            .lists
            .list(self.workspace, None, ArchiveScope::Active)
            .await
            .map_err(|error| database(&error))?;
        let mut lists = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let nodes = self
                .lists
                .nodes(summary.list.id)
                .await
                .map_err(|error| database(&error))?;
            lists.push(ProfileList {
                id: summary.list.id,
                title: summary.list.title,
                works: nodes.iter().filter_map(|node| node.work_id).collect(),
            });
        }
        let mut wanted: Vec<WorkId> = engagement.iter().map(|input| input.work_id).collect();
        wanted.extend(lists.iter().flat_map(|list| list.works.iter().copied()));
        wanted.sort();
        wanted.dedup();
        let states: Vec<StatusInput> = self
            .reading
            .for_works(self.workspace, &wanted)
            .await
            .map_err(|error| database(&error))?
            .into_values()
            .map(|state| StatusInput {
                work_id: state.work_id,
                status: state.status,
                changed_at: state.changed_at,
            })
            .collect();
        let mut profile = knowledge_profile(&engagement, &states, &lists, now);
        let ids: Vec<WorkId> = profile.entries.iter().map(|entry| entry.work_id).collect();
        let works = self
            .works
            .get_many(&ids)
            .await
            .map_err(|error| database(&error))?;
        let by_id: HashMap<WorkId, _> = works.into_iter().map(|work| (work.id, work)).collect();
        for entry in &mut profile.entries {
            if let Some(work) = by_id.get(&entry.work_id) {
                entry.title.clone_from(&work.display_title);
                entry.year = work.display_year;
            }
        }
        Ok(profile)
    }

    /// Every event at or after `since`, as JSON lines, oldest first.
    ///
    /// # Errors
    /// `Graph` for a database failure.
    pub async fn export(
        &self,
        since: DateTime<Utc>,
    ) -> Result<(String, usize), HistoryServiceError> {
        let events = self
            .history
            .since(self.workspace, since)
            .await
            .map_err(|error| database(&error))?;
        Ok((export_jsonl(&events), events.len()))
    }

    /// Delete events older than `before`; returns how many.
    ///
    /// # Errors
    /// `Graph` for a database failure.
    pub async fn prune(&self, before: DateTime<Utc>) -> Result<u64, HistoryServiceError> {
        self.history
            .prune_before(self.workspace, before)
            .await
            .map_err(|error| database(&error))
    }

    async fn titles(
        &self,
        ids: &[WorkId],
    ) -> Result<HashMap<WorkId, Option<String>>, HistoryServiceError> {
        Ok(self
            .works
            .get_many(ids)
            .await
            .map_err(|error| database(&error))?
            .into_iter()
            .map(|work| (work.id, work.display_title))
            .collect())
    }
}

// ---------------------------------------------------------------------------------------
// CLI handlers (wired into `main.rs` by the generation task)
// ---------------------------------------------------------------------------------------

/// Arguments of `lysilogy history`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryShowArgs {
    /// An age (`7d`), date or RFC 3339 time; the default is `7d`.
    pub since: Option<String>,
    pub paper: Option<String>,
    pub kinds: Vec<String>,
}

/// Render `lysilogy history`: a paper's summary, or the timeline since a time.
///
/// # Errors
/// Any [`HistoryServiceError`].
pub async fn cli_history(
    service: &HistoryService,
    args: &HistoryShowArgs,
    now: DateTime<Utc>,
) -> Result<String, HistoryServiceError> {
    if let Some(paper) = args.paper.as_deref() {
        let summary = service.paper_summary(paper).await?;
        return Ok(render_paper_summary(&summary));
    }
    let since = parse_when(args.since.as_deref().unwrap_or("7d"), now)?;
    let query = HistoryQuery {
        before: None,
        kinds: args.kinds.clone(),
        text: None,
        limit: Some(HISTORY_PAGE_MAX),
    };
    let timeline = service.timeline(&query).await?;
    Ok(render_timeline(&timeline.sessions, since))
}

/// `lysilogy history export --since`: the JSON lines and how many events they hold.
///
/// # Errors
/// Any [`HistoryServiceError`].
pub async fn cli_history_export(
    service: &HistoryService,
    since: &str,
    now: DateTime<Utc>,
) -> Result<(String, usize), HistoryServiceError> {
    service.export(parse_when(since, now)?).await
}

/// `lysilogy history prune --before`: how many events were deleted.
///
/// # Errors
/// Any [`HistoryServiceError`].
pub async fn cli_history_prune(
    service: &HistoryService,
    before: &str,
    now: DateTime<Utc>,
) -> Result<u64, HistoryServiceError> {
    service.prune(parse_when(before, now)?).await
}

#[allow(clippy::cast_precision_loss)] // display only
fn minutes(dwell_ms: i64) -> f64 {
    dwell_ms as f64 / 60_000.0
}

/// A paper summary as lines of text.
#[must_use]
pub fn render_paper_summary(summary: &PaperSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}: {} visit(s), {:.1} min{}",
        summary.paper_id,
        summary.visits,
        minutes(summary.dwell_ms),
        if summary.barely_read {
            " (barely read)"
        } else {
            ""
        }
    );
    if let (Some(first), Some(last)) = (summary.first_seen, summary.last_seen) {
        let _ = writeln!(out, "first seen {first}, last seen {last}");
    }
    for section in &summary.sections {
        let _ = writeln!(
            out,
            "  {}: {} visit(s), {:.1} min",
            section.section_id,
            section.visits,
            minutes(section.dwell_ms)
        );
    }
    out
}

/// Timeline sessions as lines of text, events at or after `since` only.
#[must_use]
pub fn render_timeline(sessions: &[SessionGroup], since: DateTime<Utc>) -> String {
    let mut out = String::new();
    for session in sessions {
        let events: Vec<&HistoryEvent> = session
            .events
            .iter()
            .filter(|event| event.occurred_at >= since)
            .collect();
        if events.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "session {} from {}",
            session.session_id, session.started_at
        );
        for event in events {
            let subject = event
                .work_title
                .as_deref()
                .or(event.paper_id.as_deref())
                .unwrap_or("");
            let dwell = event.dwell_ms.map_or_else(String::new, |ms| {
                format!(" {:.1} min", minutes(i64::from(ms)))
            });
            let _ = writeln!(
                out,
                "  {} {}{dwell} {subject}",
                event.occurred_at.format("%Y-%m-%d %H:%M"),
                event.kind
            );
        }
    }
    if out.is_empty() {
        out.push_str("No history in that range.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use lysilogy_core::IdentifierScheme;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::expansion::fixtures::{marker, new_work, test_graph};

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn at(days_ago: i64, now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::days(days_ago)
    }

    fn work(n: u128) -> WorkId {
        WorkId::from_uuid(Uuid::from_u128(n))
    }

    fn engagement(n: u128, minutes: i64, seen: DateTime<Utc>) -> EngagementInput {
        EngagementInput {
            work_id: work(n),
            dwell_ms: minutes * 60_000,
            last_seen: seen,
        }
    }

    fn status(n: u128, status: ReadingStatus, changed: DateTime<Utc>) -> StatusInput {
        StatusInput {
            work_id: work(n),
            status,
            changed_at: changed,
        }
    }

    fn new_event(kind: &str, dwell_ms: Option<i32>, work_id: Option<WorkId>) -> NewEvent {
        NewEvent {
            occurred_at: Utc::now(),
            session_id: Uuid::new_v4(),
            kind: kind.to_owned(),
            paper_id: Some("paper-a".to_owned()),
            work_id,
            list_id: None,
            dwell_ms,
            detail: json!({}),
            client_seq: 0,
        }
    }

    fn stored(session: u128, second: i64, id: i64) -> HistoryEvent {
        HistoryEvent {
            id,
            occurred_at: Utc.timestamp_opt(1_800_000_000 + second, 0).unwrap(),
            session_id: Uuid::from_u128(session),
            kind: "paper_opened".to_owned(),
            paper_id: Some("p".to_owned()),
            work_id: None,
            list_id: None,
            dwell_ms: None,
            detail: json!({}),
            client_seq: 0,
            work_title: None,
        }
    }

    #[test]
    fn should_nudge_only_paper_left_at_the_threshold_when_selecting_candidates() {
        let w = work(1);
        let events = [
            new_event("paper_left", Some(NUDGE_DWELL_MS), Some(w)),
            new_event("paper_left", Some(NUDGE_DWELL_MS + 1), Some(w)),
            new_event("paper_left", Some(NUDGE_DWELL_MS - 1), Some(work(2))),
            new_event("paper_opened", Some(NUDGE_DWELL_MS * 2), Some(work(3))),
            new_event("section_focused", Some(NUDGE_DWELL_MS * 2), Some(work(4))),
            new_event("paper_left", Some(NUDGE_DWELL_MS * 2), None),
            new_event("paper_left", None, Some(work(5))),
        ];
        assert_eq!(nudge_candidates(&events), vec![w]);
        assert!(nudge_candidates(&[]).is_empty());
    }

    #[test]
    fn should_keep_only_opened_papers_under_two_minutes_when_finding_barely_read() {
        let now = Utc::now();
        let paper = |id: &str, visits: i64, dwell_ms: i64| PaperDwell {
            paper_id: id.to_owned(),
            work_id: None,
            first_seen: now,
            last_seen: now,
            visits,
            dwell_ms,
        };
        let papers = [
            paper("short", 1, BARELY_READ_MS - 1),
            paper("edge", 1, BARELY_READ_MS),
            paper("none", 2, 0),
            paper("never-opened", 0, 0),
        ];
        let ids: Vec<String> = barely_read(&papers)
            .into_iter()
            .map(|paper| paper.paper_id)
            .collect();
        assert_eq!(ids, vec!["short", "none"]);
    }

    #[test]
    fn should_group_events_by_session_in_newest_first_order_when_grouping() {
        let events = vec![
            stored(1, 50, 5),
            stored(2, 40, 4),
            stored(1, 30, 3),
            stored(2, 20, 2),
            stored(1, 10, 1),
        ];
        let groups = group_sessions(events);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].session_id, Uuid::from_u128(1));
        assert_eq!(groups[0].events.len(), 3);
        assert_eq!(groups[0].started_at.timestamp(), 1_800_000_010);
        assert_eq!(groups[0].ended_at.timestamp(), 1_800_000_050);
        assert_eq!(
            groups[1].events.iter().map(|e| e.id).collect::<Vec<_>>(),
            [4, 2]
        );
        assert!(group_sessions(Vec::new()).is_empty());
    }

    #[test]
    fn should_round_trip_a_cursor_when_encoding_and_parsing() {
        let cursor = HistoryCursor {
            occurred_at: Utc.timestamp_micros(1_800_000_000_123_456).unwrap(),
            id: 42,
        };
        assert_eq!(parse_cursor(&encode_cursor(cursor)).unwrap(), cursor);
        let bare = parse_cursor("2026-10-24T10:00:00Z").unwrap();
        assert_eq!(bare.id, i64::MAX);
    }

    #[test]
    fn should_reject_malformed_cursors_when_parsing() {
        for text in [
            "",
            "yesterday",
            "2026-10-24T10:00:00Z_x",
            "_5",
            "2026-13-01T00:00:00Z",
        ] {
            let error = parse_cursor(text).expect_err(text);
            assert_eq!(error.code(), "invalid_history_cursor", "{text}");
        }
    }

    #[test]
    fn should_parse_ages_dates_and_timestamps_when_parsing_a_time() {
        let now = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
        assert_eq!(parse_when("7d", now).unwrap(), now - Duration::days(7));
        assert_eq!(parse_when(" 2w ", now).unwrap(), now - Duration::days(14));
        assert_eq!(parse_when("90m", now).unwrap(), now - Duration::minutes(90));
        assert_eq!(parse_when("12h", now).unwrap(), now - Duration::hours(12));
        assert_eq!(parse_when("0d", now).unwrap(), now);
        assert_eq!(
            parse_when("2026-10-01", now).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(
            parse_when("2026-10-01T00:00:00+02:00", now).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 30, 22, 0, 0).unwrap()
        );
    }

    #[test]
    fn should_reject_bad_times_when_parsing() {
        let now = Utc::now();
        for text in [
            "",
            "d",
            "-3d",
            "7x",
            "1.5d",
            "tomorrow",
            "99999999999999999d",
            "2026-02-30",
        ] {
            let error = parse_when(text, now).expect_err(text);
            assert_eq!(error.code(), "invalid_history_time", "{text}");
        }
    }

    #[test]
    fn should_write_one_json_line_per_event_when_exporting() {
        let mut event = stored(1, 0, 1);
        event.work_title = Some("Naïve \"quoted\"\nTitle ✓".to_owned());
        let text = export_jsonl(&[event, stored(1, 1, 2)]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["work_title"], "Naïve \"quoted\"\nTitle ✓");
        assert!(text.ends_with('\n'));
        assert_eq!(export_jsonl(&[]), "");
    }

    #[test]
    fn should_flag_barely_read_only_when_visited_with_little_dwell() {
        let aggregate = |visits, dwell_ms| PaperAggregate {
            paper_id: "p".to_owned(),
            first_seen: Some(Utc::now()),
            last_seen: Some(Utc::now()),
            visits,
            dwell_ms,
            sections: Vec::new(),
        };
        assert!(PaperSummary::new(aggregate(1, 1_000), None).barely_read);
        assert!(!PaperSummary::new(aggregate(1, BARELY_READ_MS), None).barely_read);
        assert!(!PaperSummary::new(aggregate(0, 0), None).barely_read);
    }

    #[test]
    fn should_rank_by_status_weight_dwell_and_recency_when_building_a_profile() {
        let now = Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap();
        let engaged = [
            engagement(1, 10, now),
            engagement(2, 10, now),
            engagement(3, 10, now),
            engagement(4, 10, at(90, now)),
        ];
        let states = [
            status(1, ReadingStatus::Read, now),
            status(2, ReadingStatus::Reading, now),
            status(4, ReadingStatus::Read, at(90, now)),
        ];
        let profile = knowledge_profile(&engaged, &states, &[], now);
        let order: Vec<WorkId> = profile.entries.iter().map(|e| e.work_id).collect();
        assert_eq!(order, vec![work(1), work(2), work(4), work(3)]);
        let by = |n: u128| {
            profile
                .entries
                .iter()
                .find(|e| e.work_id == work(n))
                .unwrap()
        };
        let base = 11.0_f64.ln();
        assert_close(by(1).score, base);
        assert_close(by(2).score, 0.6 * base);
        assert_close(by(3).score, 0.3 * base);
        assert_close(by(4).score, 0.5 * base); // one half-life old
    }

    #[test]
    fn should_include_read_works_without_dwell_but_not_untouched_ones_when_building_a_profile() {
        let now = Utc::now();
        let states = [
            status(1, ReadingStatus::Read, now),
            status(2, ReadingStatus::Unread, now),
        ];
        let engaged = [engagement(3, 0, now)];
        let profile = knowledge_profile(&engaged, &states, &[], now);
        let ids: Vec<WorkId> = profile.entries.iter().map(|e| e.work_id).collect();
        assert_eq!(ids, vec![work(1)]);
        assert_close(profile.entries[0].score, 0.0);
    }

    #[test]
    fn should_drop_stale_dwell_only_works_but_keep_stale_read_ones_when_building_a_profile() {
        let now = Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap();
        let engaged = [
            engagement(1, 30, at(PROFILE_WINDOW_DAYS + 1, now)),
            engagement(2, 30, at(PROFILE_WINDOW_DAYS, now)),
            engagement(3, 30, at(400, now)),
        ];
        let states = [status(3, ReadingStatus::Read, at(400, now))];
        let profile = knowledge_profile(&engaged, &states, &[], now);
        let ids: Vec<WorkId> = profile.entries.iter().map(|e| e.work_id).collect();
        assert_eq!(ids, vec![work(2), work(3)]);
    }

    #[test]
    fn should_cap_at_120_and_break_ties_by_id_when_building_a_profile() {
        let now = Utc::now();
        let engaged: Vec<EngagementInput> =
            (1..=130).rev().map(|n| engagement(n, 5, now)).collect();
        let states: Vec<StatusInput> = (1..=130)
            .map(|n| status(n, ReadingStatus::Read, now))
            .collect();
        let profile = knowledge_profile(&engaged, &states, &[], now);
        assert_eq!(profile.entries.len(), PROFILE_MAX_WORKS);
        assert_eq!(profile.omitted, 10);
        let expected: Vec<WorkId> = (1..=120).map(work).collect();
        let ids: Vec<WorkId> = profile.entries.iter().map(|e| e.work_id).collect();
        assert_eq!(
            ids, expected,
            "equal scores keep the lowest IDs, in ID order"
        );
    }

    #[test]
    fn should_give_the_same_profile_under_any_input_order_when_building_a_profile() {
        let now = Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap();
        let engaged = vec![
            engagement(1, 3, at(2, now)),
            engagement(2, 3, at(2, now)),
            engagement(3, 9, at(40, now)),
            engagement(1, 4, at(1, now)),
        ];
        let states = vec![
            status(2, ReadingStatus::Reading, at(2, now)),
            status(3, ReadingStatus::Read, at(40, now)),
        ];
        let forward = knowledge_profile(&engaged, &states, &[], now);
        let mut reversed_engaged = engaged;
        reversed_engaged.reverse();
        let mut reversed_states = states;
        reversed_states.reverse();
        let backward = knowledge_profile(&reversed_engaged, &reversed_states, &[], now);
        assert_eq!(forward, backward);
        let first = forward
            .entries
            .iter()
            .find(|e| e.work_id == work(1))
            .unwrap();
        assert_eq!(first.dwell_ms, 7 * 60_000, "dwell rows for one Work add up");
        assert_eq!(first.last_active, at(1, now));
    }

    #[test]
    fn should_name_each_list_once_when_a_work_repeats_in_a_list() {
        let now = Utc::now();
        let a = ListId::from_uuid(Uuid::from_u128(10));
        let b = ListId::from_uuid(Uuid::from_u128(11));
        let lists = [
            ProfileList {
                id: b,
                title: "B".to_owned(),
                works: vec![work(1)],
            },
            ProfileList {
                id: a,
                title: "A".to_owned(),
                works: vec![work(1), work(1), work(2)],
            },
        ];
        let states = [status(1, ReadingStatus::Read, now)];
        let profile = knowledge_profile(&[], &states, &lists, now);
        let titles: Vec<&str> = profile.entries[0]
            .lists
            .iter()
            .map(|l| l.title.as_str())
            .collect();
        assert_eq!(titles, ["A", "B"]);
    }

    #[test]
    fn should_produce_a_finite_score_when_dwell_is_negative_or_the_future() {
        let now = Utc::now();
        let engaged = [EngagementInput {
            work_id: work(1),
            dwell_ms: -5_000,
            last_seen: now + Duration::days(3),
        }];
        let states = [status(1, ReadingStatus::Read, now)];
        let profile = knowledge_profile(&engaged, &states, &[], now);
        assert!(profile.entries[0].score.is_finite());
        assert_eq!(profile.entries[0].dwell_ms, 0);
    }

    #[test]
    fn should_render_a_readable_timeline_when_there_are_no_events() {
        assert_eq!(
            render_timeline(&[], Utc::now()),
            "No history in that range.\n"
        );
    }

    // ---- database-backed -------------------------------------------------------------

    struct Fixture {
        service: HistoryService,
        graph: GraphState,
        marker: String,
    }

    impl Fixture {
        async fn new() -> Option<Self> {
            let graph = test_graph().await?;
            let service = HistoryService::connect(&graph).await.expect("service");
            Some(Self {
                service,
                graph,
                marker: marker(),
            })
        }

        async fn work(&self, label: &str) -> WorkId {
            let id = new_work(
                &self.graph,
                IdentifierScheme::Doi,
                &format!("10.9999/{}-{label}", self.marker),
            )
            .await;
            self.graph
                .works
                .set_display_title(id, &format!("Title {label} {}", self.marker))
                .await
                .expect("title");
            id
        }

        fn event(&self, session: Uuid, seq: i32, kind: &str) -> NewEvent {
            NewEvent {
                occurred_at: Utc::now(),
                session_id: session,
                kind: kind.to_owned(),
                paper_id: Some(format!("paper-{}", self.marker)),
                work_id: None,
                list_id: None,
                dwell_ms: None,
                detail: json!({}),
                client_seq: seq,
            }
        }
    }

    #[tokio::test]
    async fn should_move_an_unread_work_to_reading_once_when_a_visit_reaches_three_minutes() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let target = f.work("nudge").await;
        let short = f.work("short").await;
        let session = Uuid::new_v4();
        let mut long_visit = f.event(session, 0, "paper_left");
        long_visit.work_id = Some(target);
        long_visit.dwell_ms = Some(NUDGE_DWELL_MS);
        let mut short_visit = f.event(session, 1, "paper_left");
        short_visit.work_id = Some(short);
        short_visit.dwell_ms = Some(NUDGE_DWELL_MS - 1);
        let outcome = f
            .service
            .ingest(&[long_visit.clone(), short_visit])
            .await
            .expect("ingest");
        assert_eq!((outcome.accepted, outcome.duplicates), (2, 0));
        assert_eq!(outcome.nudged, vec![target]);
        let state = |work| f.service.reading.get(f.service.workspace, work);
        assert_eq!(state(target).await.unwrap(), ReadingStatus::Reading);
        assert_eq!(state(short).await.unwrap(), ReadingStatus::Unread);

        // A replay stores nothing and must not undo a later manual change.
        f.service
            .reading
            .set(f.service.workspace, target, ReadingStatus::Unread)
            .await
            .unwrap();
        let replay = f.service.ingest(&[long_visit]).await.expect("replay");
        assert_eq!((replay.accepted, replay.duplicates), (0, 1));
        assert!(replay.nudged.is_empty());
        assert_eq!(state(target).await.unwrap(), ReadingStatus::Unread);
    }

    #[tokio::test]
    async fn should_never_downgrade_or_auto_read_when_nudging() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let read = f.work("read").await;
        f.service
            .reading
            .set(f.service.workspace, read, ReadingStatus::Read)
            .await
            .unwrap();
        let mut left = f.event(Uuid::new_v4(), 0, "paper_left");
        left.work_id = Some(read);
        left.dwell_ms = Some(NUDGE_DWELL_MS * 10);
        let outcome = f.service.ingest(&[left]).await.expect("ingest");
        assert!(outcome.nudged.is_empty());
        let status = f
            .service
            .reading
            .get(f.service.workspace, read)
            .await
            .unwrap();
        assert_eq!(status, ReadingStatus::Read);
    }

    #[tokio::test]
    async fn should_nudge_the_surviving_work_when_the_event_names_a_merged_one() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let from = f.work("from").await;
        let into = f.work("into").await;
        f.graph
            .works
            .merge_work(from, into, "dup", "test")
            .await
            .expect("merge");
        let mut left = f.event(Uuid::new_v4(), 0, "paper_left");
        left.work_id = Some(from);
        left.dwell_ms = Some(NUDGE_DWELL_MS);
        let outcome = f.service.ingest(&[left]).await.expect("ingest");
        assert_eq!(outcome.nudged, vec![into]);
    }

    #[tokio::test]
    async fn should_reject_a_bad_batch_before_storing_anything_when_ingesting() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let good = f.event(session, 0, "paper_opened");
        let bad_kind = f.event(session, 1, "teleported");
        let error = f
            .service
            .ingest(&[good.clone(), bad_kind])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_history_kind");
        let mut negative = f.event(session, 2, "paper_left");
        negative.dwell_ms = Some(-1);
        assert_eq!(
            f.service.ingest(&[negative]).await.unwrap_err().code(),
            "invalid_history_dwell"
        );
        let many: Vec<NewEvent> = (0..=200)
            .map(|seq| f.event(session, seq, "paper_opened"))
            .collect();
        assert_eq!(
            f.service.ingest(&many).await.unwrap_err().code(),
            "invalid_history_batch"
        );
        let stored = f.service.ingest(&[good]).await.expect("good alone");
        assert_eq!(stored.accepted, 1, "the rejected batches stored nothing");
        let empty = f.service.ingest(&[]).await.expect("empty");
        assert_eq!((empty.accepted, empty.duplicates), (0, 0));
    }

    #[tokio::test]
    async fn should_sum_dwell_per_paper_and_section_when_summarizing() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let mut events = vec![
            f.event(session, 0, "paper_opened"),
            f.event(session, 10, "paper_opened"),
        ];
        for (seq, ms) in [(1, 30_000), (11, 45_000)] {
            let mut left = f.event(session, seq, "paper_left");
            left.dwell_ms = Some(ms);
            events.push(left);
        }
        for (seq, section, ms) in [
            (2, "intro", 5_000),
            (3, "intro", 7_000),
            (4, "méthode", 9_000),
        ] {
            let mut focus = f.event(session, seq, "section_focused");
            focus.dwell_ms = Some(ms);
            focus.detail = json!({ "section_id": section });
            events.push(focus);
        }
        f.service.ingest(&events).await.expect("ingest");
        let paper = format!("paper-{}", f.marker);
        let summary = f.service.paper_summary(&paper).await.expect("summary");
        assert_eq!(summary.visits, 2);
        assert_eq!(summary.dwell_ms, 75_000);
        assert!(summary.barely_read);
        let sections: Vec<(&str, i64, i64)> = summary
            .sections
            .iter()
            .map(|s| (s.section_id.as_str(), s.visits, s.dwell_ms))
            .collect();
        assert_eq!(sections, vec![("intro", 2, 12_000), ("méthode", 1, 9_000)]);
        assert!(summary.position.is_none());

        let missing = f.service.paper_summary(&format!("none-{}", f.marker)).await;
        assert_eq!(missing.unwrap_err().code(), "paper_not_found");
    }

    #[tokio::test]
    async fn should_list_inspected_works_that_were_never_opened_when_deriving() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let peeked = f.work("peeked").await;
        let opened = f.work("opened").await;
        let session = Uuid::new_v4();
        let mut inspect_a = f.event(session, 0, "work_inspected");
        inspect_a.work_id = Some(peeked);
        inspect_a.paper_id = None;
        let mut inspect_b = f.event(session, 1, "work_inspected");
        inspect_b.work_id = Some(opened);
        inspect_b.paper_id = None;
        let mut open_b = f.event(session, 2, "paper_opened");
        open_b.work_id = Some(opened);
        f.service
            .ingest(&[inspect_a, inspect_b, open_b])
            .await
            .expect("ingest");
        let never = f.service.inspected_never_opened().await.expect("derive");
        let mine: Vec<&InspectedEntry> = never
            .iter()
            .filter(|entry| entry.work_id == peeked || entry.work_id == opened)
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].work_id, peeked);
        assert_eq!(mine[0].inspects, 1);
        assert_eq!(
            mine[0].title.as_deref(),
            Some(format!("Title peeked {}", f.marker).as_str())
        );
    }

    #[tokio::test]
    async fn should_report_opened_papers_with_little_dwell_when_finding_barely_read() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let mut left = f.event(session, 1, "paper_left");
        left.dwell_ms = Some(60_000);
        f.service
            .ingest(&[f.event(session, 0, "paper_opened"), left])
            .await
            .expect("ingest");
        let papers = f.service.barely_read().await.expect("barely read");
        assert!(
            papers
                .iter()
                .any(|p| p.paper_id == format!("paper-{}", f.marker))
        );
    }

    #[tokio::test]
    async fn should_save_and_reject_positions_when_updating() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let paper = format!("pos-{}", f.marker);
        let position = ReadingPosition {
            paper_id: paper.clone(),
            view: "pdf".to_owned(),
            page: Some(7),
            offset_utf16: None,
            section_id: None,
            scroll_ratio: Some(0.25),
            updated_at: Utc::now(),
        };
        f.service.save_position(&position).await.expect("save");
        let summary = f.service.paper_summary(&paper).await.expect("summary");
        assert_eq!(summary.position.as_ref().and_then(|p| p.page), Some(7));
        assert!(
            summary.first_seen.is_none(),
            "a position alone is not a visit"
        );
        assert!(!summary.barely_read);
        let bad_view = ReadingPosition {
            view: "scroll".to_owned(),
            ..position.clone()
        };
        assert_eq!(
            f.service.save_position(&bad_view).await.unwrap_err().code(),
            "invalid_position"
        );
        let bad_ratio = ReadingPosition {
            scroll_ratio: Some(1.5),
            ..position
        };
        assert_eq!(
            f.service
                .save_position(&bad_ratio)
                .await
                .unwrap_err()
                .code(),
            "invalid_position"
        );
    }

    #[tokio::test]
    async fn should_page_the_timeline_with_a_cursor_when_events_exceed_the_limit() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let base = Utc::now();
        let events: Vec<NewEvent> = (0..5)
            .map(|seq| {
                let mut event = f.event(session, seq, "search_run");
                event.occurred_at = base + Duration::seconds(i64::from(seq));
                event.detail = json!({ "query": format!("q-{}-{seq}", f.marker) });
                event
            })
            .collect();
        f.service.ingest(&events).await.expect("ingest");
        let query = HistoryQuery {
            kinds: vec!["search_run".to_owned()],
            text: Some(format!("q-{}-", f.marker)),
            limit: Some(2),
            ..HistoryQuery::default()
        };
        let first = f.service.timeline(&query).await.expect("page 1");
        assert_eq!(first.sessions.len(), 1);
        assert_eq!(first.sessions[0].events.len(), 2);
        let cursor = parse_cursor(first.next_before.as_deref().expect("more")).expect("cursor");
        let second = f
            .service
            .timeline(&HistoryQuery {
                before: Some(cursor),
                ..query.clone()
            })
            .await
            .expect("page 2");
        let seqs: Vec<i32> = second.sessions[0]
            .events
            .iter()
            .map(|e| e.client_seq)
            .collect();
        assert_eq!(seqs, vec![2, 1]);
        let last = f
            .service
            .timeline(&HistoryQuery {
                before: Some(parse_cursor(second.next_before.as_deref().unwrap()).unwrap()),
                ..query
            })
            .await
            .expect("page 3");
        assert_eq!(last.sessions[0].events.len(), 1);
        assert!(last.next_before.is_none());
    }

    #[tokio::test]
    async fn should_reject_unknown_kind_filters_when_reading_the_timeline() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let query = HistoryQuery {
            kinds: vec!["nope".to_owned()],
            ..HistoryQuery::default()
        };
        assert_eq!(
            f.service.timeline(&query).await.unwrap_err().code(),
            "invalid_history_kind"
        );
    }

    #[tokio::test]
    async fn should_export_since_a_time_and_prune_before_one_when_managing_history() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let session = Uuid::new_v4();
        let now = Utc::now();
        let mut old = f.event(session, 0, "search_run");
        old.occurred_at = now - Duration::days(400);
        old.detail = json!({ "query": format!("old-{}", f.marker) });
        let mut recent = f.event(session, 1, "search_run");
        recent.detail = json!({ "query": format!("new-{}", f.marker) });
        f.service.ingest(&[old, recent]).await.expect("ingest");

        let (everything, _) = cli_history_export(&f.service, "500d", now)
            .await
            .expect("export");
        assert!(everything.contains(&format!("old-{}", f.marker)));
        assert!(everything.contains(&format!("new-{}", f.marker)));
        let (recent_only, count) = cli_history_export(&f.service, "30d", now)
            .await
            .expect("export");
        assert!(!recent_only.contains(&format!("old-{}", f.marker)));
        assert!(recent_only.contains(&format!("new-{}", f.marker)));
        assert_eq!(count, recent_only.lines().count());

        // Prune the old row only; other tests' rows are all recent.
        let deleted = cli_history_prune(&f.service, "300d", now)
            .await
            .expect("prune");
        assert!(deleted >= 1);
        let (after, _) = cli_history_export(&f.service, "500d", now)
            .await
            .expect("export");
        assert!(!after.contains(&format!("old-{}", f.marker)));
        assert!(after.contains(&format!("new-{}", f.marker)));
    }

    #[tokio::test]
    async fn should_build_a_profile_from_stored_history_when_fetching() {
        let Some(f) = Fixture::new().await else {
            return;
        };
        let read = f.work("profile-read").await;
        let session = Uuid::new_v4();
        let mut left = f.event(session, 0, "paper_left");
        left.work_id = Some(read);
        left.dwell_ms = Some(10_000 * 60_000);
        f.service.ingest(&[left]).await.expect("ingest");
        f.service
            .reading
            .set(f.service.workspace, read, ReadingStatus::Read)
            .await
            .unwrap();
        let profile = f
            .service
            .knowledge_profile(Utc::now())
            .await
            .expect("profile");
        let entry = profile
            .entries
            .iter()
            .find(|entry| entry.work_id == read)
            .expect("the read Work is in the profile");
        assert_eq!(entry.status, ReadingStatus::Read);
        assert!(entry.dwell_ms >= 10_000 * 60_000);
        assert_eq!(
            entry.title.as_deref(),
            Some(format!("Title profile-read {}", f.marker).as_str())
        );
        assert!(entry.score > 0.0);
    }
}
