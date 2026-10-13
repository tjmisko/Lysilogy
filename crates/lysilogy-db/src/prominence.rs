//! Persistence for the per-workspace prominence cache (`reader.work_prominence`).
//!
//! The table is derived and safe to truncate: rows are recomputed from the graph
//! whenever they are missing or were written under another signals version.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use lysilogy_core::{
    PROMINENCE_SIGNALS_VERSION, ProminenceSignals, WorkId, WorkspaceId, prominence_score,
};
use serde::Serialize;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;

/// How many Works one signal query covers.
const SIGNAL_CHUNK: usize = 500;

/// A cached score with the signals it was computed from.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct StoredProminence {
    pub work_id: WorkId,
    pub signals: ProminenceSignals,
    pub score: f64,
    pub computed_at: DateTime<Utc>,
}

const SIGNALS_SQL: &str = "\
with target as (select distinct unnest($2::uuid[]) as id)
select t.id, w.display_year,
  (select count(distinct a.citing_work_id) from graph.citation_assertion a
     where a.cited_work_id = t.id and a.status in ('asserted', 'accepted')) as citing,
  (select count(distinct a.cited_work_id) from graph.citation_assertion a
     where a.citing_work_id = t.id and a.status in ('asserted', 'accepted')) as cited,
  (select max(case
                when jsonb_typeof(r.payload->'cited_by_count') = 'number'
                 and (r.payload->>'cited_by_count')::numeric >= 0
                 and (r.payload->>'cited_by_count')::numeric < 9e18
                then trunc((r.payload->>'cited_by_count')::numeric)::bigint
              end)
     from graph.work_identifier i
     join source.source_record r on r.kind = 'openalex_metrics'
                                 and r.source_key = lower(i.value_normalized)
    where i.work_id = t.id and i.scheme = 'openalex' and i.status = 'active') as cited_by_global,
  exists (select 1 from reader.document d where d.workspace_id = $1
            and d.work_id = t.id and d.relative_path is not null) as local_available,
  (select count(*) from graph.citation_mention m
     join graph.citation_assertion a on a.id = m.assertion_id
    where a.cited_work_id = t.id and a.status in ('asserted', 'accepted')) as mentions,
  (select count(*) from graph.relationship_claim c
     join graph.citation_assertion a on a.id = c.assertion_id
    where a.cited_work_id = t.id and a.status in ('asserted', 'accepted')
      and c.superseded_by is null and c.review_state <> 'rejected'
      and c.label in ('uses_method', 'uses_data', 'extends')) as method_citations,
  (select count(*) from graph.relationship_claim c
     join graph.citation_assertion a on a.id = c.assertion_id
    where a.cited_work_id = t.id and a.status in ('asserted', 'accepted')
      and c.superseded_by is null and c.review_state <> 'rejected'
      and c.label = 'background') as background_citations
from target t join graph.work w on w.id = t.id
order by t.id";

const STORED_COLUMNS: &str = "work_id, citing_works_count, cited_works_count, \
    cited_by_global_count, display_year, local_available, mention_count, \
    method_citation_count, background_citation_count, score, computed_at";

/// Computes, caches, and reads prominence signals and scores.
#[derive(Clone)]
pub struct ProminenceRepository {
    pool: PgPool,
}

impl ProminenceRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Compute the raw signals for Works (unknown Works are omitted).
    pub async fn compute_signals(
        &self,
        workspace: WorkspaceId,
        works: &[WorkId],
    ) -> Result<Vec<(WorkId, ProminenceSignals)>> {
        let mut signals = Vec::with_capacity(works.len());
        for chunk in works.chunks(SIGNAL_CHUNK) {
            let ids: Vec<Uuid> = chunk.iter().map(|work| work.as_uuid()).collect();
            let rows = sqlx::query(SIGNALS_SQL)
                .bind(workspace.as_uuid())
                .bind(&ids)
                .fetch_all(&self.pool)
                .await?;
            for row in &rows {
                signals.push((
                    WorkId::from_uuid(row.try_get("id")?),
                    ProminenceSignals {
                        citing_works_count: row.try_get("citing")?,
                        cited_works_count: row.try_get("cited")?,
                        cited_by_global_count: row.try_get("cited_by_global")?,
                        display_year: row.try_get("display_year")?,
                        local_available: row.try_get("local_available")?,
                        mention_count: row.try_get("mentions")?,
                        method_citation_count: row.try_get("method_citations")?,
                        background_citation_count: row.try_get("background_citations")?,
                    },
                ));
            }
        }
        Ok(signals)
    }

    /// Upsert rows at the current signals version.
    #[allow(clippy::cast_possible_truncation)]
    pub async fn store(
        &self,
        workspace: WorkspaceId,
        rows: &[(WorkId, ProminenceSignals, f64)],
    ) -> Result<()> {
        for chunk in rows.chunks(SIGNAL_CHUNK) {
            let work: Vec<Uuid> = chunk.iter().map(|row| row.0.as_uuid()).collect();
            let citing: Vec<i32> = chunk
                .iter()
                .map(|row| row.1.citing_works_count as i32)
                .collect();
            let cited: Vec<i32> = chunk
                .iter()
                .map(|row| row.1.cited_works_count as i32)
                .collect();
            let global: Vec<Option<i64>> = chunk
                .iter()
                .map(|row| row.1.cited_by_global_count)
                .collect();
            let year: Vec<Option<i32>> = chunk.iter().map(|row| row.1.display_year).collect();
            let local: Vec<bool> = chunk.iter().map(|row| row.1.local_available).collect();
            let mentions: Vec<i32> = chunk.iter().map(|row| row.1.mention_count as i32).collect();
            let method: Vec<i32> = chunk
                .iter()
                .map(|row| row.1.method_citation_count as i32)
                .collect();
            let background: Vec<i32> = chunk
                .iter()
                .map(|row| row.1.background_citation_count as i32)
                .collect();
            let score: Vec<f64> = chunk.iter().map(|row| row.2).collect();
            sqlx::query(
                "insert into reader.work_prominence \
                     (workspace_id, work_id, signals_version, citing_works_count, \
                      cited_works_count, cited_by_global_count, display_year, local_available, \
                      mention_count, method_citation_count, background_citation_count, score) \
                 select $1, u.work, $2, u.citing, u.cited, u.global, u.year, u.local, \
                        u.mentions, u.method, u.background, u.score \
                 from unnest($3::uuid[], $4::int[], $5::int[], $6::bigint[], $7::int[], \
                             $8::bool[], $9::int[], $10::int[], $11::int[], $12::float8[]) \
                      as u(work, citing, cited, global, year, local, mentions, method, \
                           background, score) \
                 on conflict (workspace_id, work_id) do update set \
                     signals_version = excluded.signals_version, \
                     citing_works_count = excluded.citing_works_count, \
                     cited_works_count = excluded.cited_works_count, \
                     cited_by_global_count = excluded.cited_by_global_count, \
                     display_year = excluded.display_year, \
                     local_available = excluded.local_available, \
                     mention_count = excluded.mention_count, \
                     method_citation_count = excluded.method_citation_count, \
                     background_citation_count = excluded.background_citation_count, \
                     score = excluded.score, computed_at = now()",
            )
            .bind(workspace.as_uuid())
            .bind(PROMINENCE_SIGNALS_VERSION)
            .bind(&work)
            .bind(&citing)
            .bind(&cited)
            .bind(&global)
            .bind(&year)
            .bind(&local)
            .bind(&mentions)
            .bind(&method)
            .bind(&background)
            .bind(&score)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Cached rows written under the current signals version.
    pub async fn cached(
        &self,
        workspace: WorkspaceId,
        works: &[WorkId],
    ) -> Result<Vec<StoredProminence>> {
        if works.is_empty() {
            return Ok(Vec::new());
        }
        let mut stored = Vec::with_capacity(works.len());
        for chunk in works.chunks(SIGNAL_CHUNK) {
            let ids: Vec<Uuid> = chunk.iter().map(|work| work.as_uuid()).collect();
            let rows = sqlx::query(&format!(
                "select {STORED_COLUMNS} from reader.work_prominence \
                 where workspace_id = $1 and work_id = any($2) and signals_version = $3 \
                 order by work_id"
            ))
            .bind(workspace.as_uuid())
            .bind(&ids)
            .bind(PROMINENCE_SIGNALS_VERSION)
            .fetch_all(&self.pool)
            .await?;
            for row in &rows {
                stored.push(stored_from_row(row)?);
            }
        }
        Ok(stored)
    }

    /// Prominence for each Work: cached rows, plus freshly computed and stored
    /// rows for the missing or stale ones. Unknown Works are omitted.
    pub async fn prominence_for(
        &self,
        workspace: WorkspaceId,
        works: &[WorkId],
        current_year: i32,
    ) -> Result<HashMap<WorkId, StoredProminence>> {
        let mut found: HashMap<WorkId, StoredProminence> = self
            .cached(workspace, works)
            .await?
            .into_iter()
            .map(|row| (row.work_id, row))
            .collect();
        let mut missing: Vec<WorkId> = works
            .iter()
            .copied()
            .filter(|work| !found.contains_key(work))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        if missing.is_empty() {
            return Ok(found);
        }
        let computed = self.compute_signals(workspace, &missing).await?;
        let scored: Vec<(WorkId, ProminenceSignals, f64)> = computed
            .into_iter()
            .map(|(work, signals)| (work, signals, prominence_score(&signals, current_year)))
            .collect();
        self.store(workspace, &scored).await?;
        let stored_ids: Vec<WorkId> = scored.iter().map(|row| row.0).collect();
        for row in self.cached(workspace, &stored_ids).await? {
            found.insert(row.work_id, row);
        }
        Ok(found)
    }

    /// Works that appear in any assertion or have a document, keyset-paged by ID.
    #[allow(clippy::cast_possible_wrap)]
    pub async fn participating_works(
        &self,
        after: Option<WorkId>,
        limit: usize,
    ) -> Result<Vec<WorkId>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "select w.id from graph.work w \
             where ($1::uuid is null or w.id > $1) \
               and (exists (select 1 from graph.citation_assertion a where a.citing_work_id = w.id) \
                 or exists (select 1 from graph.citation_assertion a where a.cited_work_id = w.id) \
                 or exists (select 1 from reader.document d where d.work_id = w.id)) \
             order by w.id limit $2",
        )
        .bind(after.map(WorkId::as_uuid))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(WorkId::from_uuid).collect())
    }
}

fn stored_from_row(row: &PgRow) -> Result<StoredProminence> {
    let count = |name: &str| -> Result<i64> { Ok(i64::from(row.try_get::<i32, _>(name)?)) };
    Ok(StoredProminence {
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        signals: ProminenceSignals {
            citing_works_count: count("citing_works_count")?,
            cited_works_count: count("cited_works_count")?,
            cited_by_global_count: row.try_get("cited_by_global_count")?,
            display_year: row.try_get("display_year")?,
            local_available: row.try_get("local_available")?,
            mention_count: count("mention_count")?,
            method_citation_count: count("method_citation_count")?,
            background_citation_count: count("background_citation_count")?,
        },
        score: row.try_get("score")?,
        computed_at: row.try_get("computed_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_record::SourceRecordRepository;
    use crate::test_support;
    use lysilogy_core::Coverage;
    use serde_json::{Value, json};

    const YEAR: i32 = 2026;

    async fn signals_of(
        repo: &ProminenceRepository,
        workspace: WorkspaceId,
        work: WorkId,
    ) -> ProminenceSignals {
        let computed = repo
            .compute_signals(workspace, &[work])
            .await
            .expect("signals");
        assert_eq!(computed.len(), 1);
        computed[0].1
    }

    async fn set_status(pool: &PgPool, assertion: Uuid, status: &str) {
        sqlx::query("update graph.citation_assertion set status = $2 where id = $1")
            .bind(assertion)
            .bind(status)
            .execute(pool)
            .await
            .expect("status");
    }

    async fn add_mention(pool: &PgPool, assertion: Uuid) {
        sqlx::query("insert into graph.citation_mention (assertion_id) values ($1)")
            .bind(assertion)
            .execute(pool)
            .await
            .expect("mention");
    }

    async fn add_claim(
        pool: &PgPool,
        assertion: Uuid,
        label: &str,
        review_state: &str,
        superseded: bool,
    ) {
        let id: Uuid = sqlx::query_scalar(
            "insert into graph.relationship_claim (assertion_id, label, review_state) \
             values ($1, $2, $3) returning id",
        )
        .bind(assertion)
        .bind(label)
        .bind(review_state)
        .fetch_one(pool)
        .await
        .expect("claim");
        if superseded {
            let newer: Uuid = sqlx::query_scalar(
                "insert into graph.relationship_claim (assertion_id, label, review_state) \
                 values ($1, 'other', 'unreviewed') returning id",
            )
            .bind(assertion)
            .fetch_one(pool)
            .await
            .expect("newer");
            sqlx::query("update graph.relationship_claim set superseded_by = $2 where id = $1")
                .bind(id)
                .bind(newer)
                .execute(pool)
                .await
                .expect("supersede");
        }
    }

    async fn give_openalex_id(pool: &PgPool, work: WorkId) -> String {
        let value = format!("W{}", Uuid::new_v4().as_u128() % 10_u128.pow(15));
        sqlx::query(
            "insert into graph.work_identifier (work_id, scheme, value_normalized) \
             values ($1, 'openalex', $2)",
        )
        .bind(work.as_uuid())
        .bind(&value)
        .execute(pool)
        .await
        .expect("identifier");
        value
    }

    async fn cache_metrics(pool: &PgPool, openalex: &str, payload: &Value) {
        SourceRecordRepository::new(pool.clone())
            .upsert(
                "openalex_metrics",
                &openalex.to_ascii_lowercase(),
                Coverage::Complete,
                payload,
            )
            .await
            .expect("metrics record");
    }

    #[tokio::test]
    async fn should_count_distinct_citing_works_and_ignore_rejected_assertions_when_computing_signals()
     {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let target = test_support::new_work(&pool).await;
        let other = test_support::new_work(&pool).await;
        let first = test_support::new_work(&pool).await;
        let second = test_support::new_work(&pool).await;
        let rejected = test_support::new_work(&pool).await;
        let withdrawn = test_support::new_work(&pool).await;

        test_support::cite(&pool, first, target).await;
        // A second assertion from the same Work does not add a citing Work.
        test_support::cite(&pool, first, target).await;
        let accepted = test_support::cite(&pool, second, target).await;
        set_status(&pool, accepted, "accepted").await;
        let rejected_assertion = test_support::cite(&pool, rejected, target).await;
        set_status(&pool, rejected_assertion, "rejected").await;
        let withdrawn_assertion = test_support::cite(&pool, withdrawn, target).await;
        set_status(&pool, withdrawn_assertion, "withdrawn").await;
        test_support::cite(&pool, target, other).await;

        let signals = signals_of(&repo, workspace, target).await;
        assert_eq!(signals.citing_works_count, 2);
        assert_eq!(signals.cited_works_count, 1);
        assert_eq!(signals.cited_by_global_count, None);
        assert!(!signals.local_available);
        assert_eq!(signals.mention_count, 0);

        let isolated = signals_of(&repo, workspace, test_support::new_work(&pool).await).await;
        assert_eq!(isolated, ProminenceSignals::default());
    }

    #[tokio::test]
    async fn should_read_cited_by_count_from_metrics_record_when_cached() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let cases: [(Value, Option<i64>); 7] = [
            (json!({"cited_by_count": 1204}), Some(1204)),
            (json!({"cited_by_count": 0}), Some(0)),
            (json!({"cited_by_count": "77"}), None),
            (json!({"cited_by_count": -3}), None),
            (json!({"cited_by_count": 12.9}), Some(12)),
            (json!({"cited_by_count": 1e30}), None),
            (json!({"id": "no count"}), None),
        ];
        for (payload, expected) in cases {
            let work = test_support::new_work(&pool).await;
            let openalex = give_openalex_id(&pool, work).await;
            cache_metrics(&pool, &openalex, &payload).await;
            assert_eq!(
                signals_of(&repo, workspace, work)
                    .await
                    .cited_by_global_count,
                expected,
                "{payload}"
            );
        }
        let uncached = test_support::new_work(&pool).await;
        give_openalex_id(&pool, uncached).await;
        assert_eq!(
            signals_of(&repo, workspace, uncached)
                .await
                .cited_by_global_count,
            None
        );
    }

    #[tokio::test]
    async fn should_count_mentions_and_method_and_background_labels_when_computing_signals() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let target = test_support::new_work(&pool).await;
        let citing = test_support::new_work(&pool).await;
        let rejected_citing = test_support::new_work(&pool).await;
        let assertion = test_support::cite(&pool, citing, target).await;
        for _ in 0..3 {
            add_mention(&pool, assertion).await;
        }
        add_claim(&pool, assertion, "uses_method", "unreviewed", false).await;
        add_claim(&pool, assertion, "extends", "accepted", false).await;
        add_claim(&pool, assertion, "background", "unreviewed", false).await;
        add_claim(&pool, assertion, "uses_data", "rejected", false).await;
        add_claim(&pool, assertion, "uses_method", "unreviewed", true).await;
        add_claim(&pool, assertion, "supports", "unreviewed", false).await;
        // Evidence on a rejected assertion counts for nothing.
        let rejected = test_support::cite(&pool, rejected_citing, target).await;
        set_status(&pool, rejected, "rejected").await;
        add_mention(&pool, rejected).await;
        add_claim(&pool, rejected, "uses_method", "unreviewed", false).await;

        let signals = signals_of(&repo, workspace, target).await;
        assert_eq!(signals.mention_count, 3);
        assert_eq!(signals.method_citation_count, 2, "uses_method + extends");
        assert_eq!(signals.background_citation_count, 1);
        assert_eq!(signals.citing_works_count, 1);
    }

    #[tokio::test]
    async fn should_mark_local_available_only_for_present_documents_in_workspace() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let other = test_support::new_workspace(&pool).await;
        let present = test_support::new_work(&pool).await;
        let tombstoned = test_support::new_work(&pool).await;
        let elsewhere = test_support::new_work(&pool).await;
        for (owner, work, path) in [
            (workspace, present, Some("present.pdf")),
            (workspace, tombstoned, None),
            (other, elsewhere, Some("elsewhere.pdf")),
        ] {
            sqlx::query(
                "insert into reader.document (workspace_id, work_id, relative_path) \
                 values ($1, $2, $3)",
            )
            .bind(owner.as_uuid())
            .bind(work.as_uuid())
            .bind(path)
            .execute(&pool)
            .await
            .expect("document");
        }
        assert!(signals_of(&repo, workspace, present).await.local_available);
        assert!(
            !signals_of(&repo, workspace, tombstoned)
                .await
                .local_available
        );
        assert!(
            !signals_of(&repo, workspace, elsewhere)
                .await
                .local_available
        );
        assert!(signals_of(&repo, other, elsewhere).await.local_available);
    }

    #[tokio::test]
    async fn should_recompute_stale_rows_when_signals_version_differs() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let cited = test_support::new_work(&pool).await;
        let citing = test_support::new_work(&pool).await;
        test_support::cite(&pool, citing, cited).await;

        assert!(
            repo.cached(workspace, &[cited])
                .await
                .expect("cold")
                .is_empty()
        );
        let first = repo
            .prominence_for(workspace, &[cited, cited], YEAR)
            .await
            .expect("compute");
        assert_eq!(first.len(), 1);
        assert_eq!(first[&cited].signals.citing_works_count, 1);
        assert!(first[&cited].score > 0.0);
        assert_eq!(
            repo.cached(workspace, &[cited]).await.expect("warm").len(),
            1
        );

        // Corrupt the row under another version: it must be ignored and rewritten.
        sqlx::query(
            "update reader.work_prominence set signals_version = signals_version + 1, \
                 citing_works_count = 99, score = 0.5 where work_id = $1",
        )
        .bind(cited.as_uuid())
        .execute(&pool)
        .await
        .expect("stale");
        assert!(
            repo.cached(workspace, &[cited])
                .await
                .expect("stale")
                .is_empty()
        );
        let again = repo
            .prominence_for(workspace, &[cited], YEAR)
            .await
            .expect("recompute");
        assert_eq!(again[&cited].signals.citing_works_count, 1);
        let version: i16 = sqlx::query_scalar(
            "select signals_version from reader.work_prominence where work_id = $1",
        )
        .bind(cited.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("version");
        assert_eq!(version, PROMINENCE_SIGNALS_VERSION);

        // A cached row is served as is, even when the graph changed since.
        test_support::cite(&pool, test_support::new_work(&pool).await, cited).await;
        let served = repo
            .prominence_for(workspace, &[cited], YEAR)
            .await
            .expect("cached");
        assert_eq!(served[&cited].signals.citing_works_count, 1);

        // Unknown Works are omitted.
        let unknown = WorkId::new();
        let none = repo
            .prominence_for(workspace, &[unknown], YEAR)
            .await
            .expect("unknown");
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn should_return_empty_without_query_when_no_ids_given() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let workspace = test_support::new_workspace(&pool).await;
        // A closed pool would fail any query, so success proves none was issued.
        let closed = ProminenceRepository::new(pool.clone());
        closed.pool.close().await;
        assert!(
            closed
                .compute_signals(workspace, &[])
                .await
                .expect("signals")
                .is_empty()
        );
        assert!(
            closed
                .cached(workspace, &[])
                .await
                .expect("cached")
                .is_empty()
        );
        assert!(closed.store(workspace, &[]).await.is_ok());
        assert!(
            closed
                .prominence_for(workspace, &[], YEAR)
                .await
                .expect("for")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn should_chunk_and_page_when_many_works_participate() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = ProminenceRepository::new(pool.clone());
        let workspace = test_support::new_workspace(&pool).await;
        let hub = test_support::new_work(&pool).await;
        let mut spokes = Vec::new();
        for _ in 0..620 {
            let spoke = test_support::new_work(&pool).await;
            test_support::cite(&pool, spoke, hub).await;
            spokes.push(spoke);
        }
        let mut all = spokes.clone();
        all.push(hub);
        let signals = repo
            .compute_signals(workspace, &all)
            .await
            .expect("signals");
        assert_eq!(signals.len(), 621, "two chunks cover every Work");
        let hub_signals = signals.iter().find(|row| row.0 == hub).expect("hub").1;
        assert_eq!(hub_signals.citing_works_count, 620);

        let scored: Vec<_> = signals
            .iter()
            .map(|(work, row)| (*work, *row, prominence_score(row, YEAR)))
            .collect();
        repo.store(workspace, &scored).await.expect("store");
        assert_eq!(
            repo.cached(workspace, &all).await.expect("cached").len(),
            621
        );

        let mut seen = std::collections::HashSet::new();
        let mut after = None;
        loop {
            let page = repo.participating_works(after, 250).await.expect("page");
            let Some(last) = page.last().copied() else {
                break;
            };
            assert!(page.len() <= 250);
            for work in page {
                assert!(seen.insert(work), "keyset paging never repeats a Work");
            }
            after = Some(last);
        }
        assert!(all.iter().all(|work| seen.contains(work)));
        let isolated = test_support::new_work(&pool).await;
        assert!(!seen.contains(&isolated));
    }
}
