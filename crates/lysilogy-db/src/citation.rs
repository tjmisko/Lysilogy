//! Persistence for citation evidence and the canonical edges it forms.
//!
//! An assertion is one source's evidence that A cites B. Canonical edges group
//! the accepted assertions for a `(citing, cited)` pair.

use std::collections::HashMap;

use lysilogy_core::{AssertionId, EntryId, SourceRecordId, Work, WorkId};
use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

use crate::work::{WORK_COLUMNS, work_from_row};
use crate::{DbError, Result};

/// Which kinds of evidence support a canonical edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeSources {
    /// Some assertion comes from a provider record.
    pub provider: bool,
    /// Some assertion comes from a local bibliography entry.
    pub local: bool,
    /// In-text mentions stored across the edge's assertions.
    pub mention_count: usize,
}

/// Reads and writes citation assertions and their grouped works.
#[derive(Clone)]
pub struct CitationRepository {
    pool: PgPool,
}

impl CitationRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record one source's evidence for `citing → cited`, reusing any existing
    /// assertion with the same evidence source.
    pub async fn add_assertion(
        &self,
        citing: WorkId,
        cited: WorkId,
        source_record_id: Option<SourceRecordId>,
    ) -> Result<AssertionId> {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "select id from graph.citation_assertion \
             where citing_work_id = $1 and cited_work_id = $2 \
               and source_record_id is not distinct from $3 \
             limit 1",
        )
        .bind(citing.as_uuid())
        .bind(cited.as_uuid())
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .fetch_optional(&self.pool)
        .await?;
        if let Some(id) = existing {
            return Ok(AssertionId::from_uuid(id));
        }

        let inserted: Option<Uuid> = sqlx::query_scalar(
            "insert into graph.citation_assertion \
                 (citing_work_id, cited_work_id, source_record_id, status) \
             values ($1, $2, $3, 'accepted') \
             on conflict do nothing \
             returning id",
        )
        .bind(citing.as_uuid())
        .bind(cited.as_uuid())
        .bind(source_record_id.map(SourceRecordId::as_uuid))
        .fetch_optional(&self.pool)
        .await?;
        inserted
            .map(AssertionId::from_uuid)
            .ok_or_else(|| DbError::Conflict("citation assertion lost a concurrent insert".into()))
    }

    /// Record that a local bibliography entry shows `citing → cited`. The
    /// assertion has status `asserted` and is reused when it already exists.
    pub async fn add_entry_assertion(
        &self,
        citing: WorkId,
        cited: WorkId,
        entry: EntryId,
    ) -> Result<AssertionId> {
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "insert into graph.citation_assertion \
                 (citing_work_id, cited_work_id, entry_id, status) \
             values ($1, $2, $3, 'asserted') \
             on conflict (citing_work_id, cited_work_id, entry_id) \
                 where entry_id is not null do nothing \
             returning id",
        )
        .bind(citing.as_uuid())
        .bind(cited.as_uuid())
        .bind(entry.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        if let Some(id) = inserted {
            return Ok(AssertionId::from_uuid(id));
        }
        let existing: Uuid = sqlx::query_scalar(
            "select id from graph.citation_assertion \
             where citing_work_id = $1 and cited_work_id = $2 and entry_id = $3",
        )
        .bind(citing.as_uuid())
        .bind(cited.as_uuid())
        .bind(entry.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        Ok(AssertionId::from_uuid(existing))
    }

    /// Evidence summary for each requested `(citing, cited)` edge. Edges with no
    /// live assertion are omitted.
    pub async fn edge_sources(
        &self,
        pairs: &[(WorkId, WorkId)],
    ) -> Result<HashMap<(WorkId, WorkId), EdgeSources>> {
        if pairs.is_empty() {
            return Ok(HashMap::new());
        }
        let citing: Vec<Uuid> = pairs.iter().map(|pair| pair.0.as_uuid()).collect();
        let cited: Vec<Uuid> = pairs.iter().map(|pair| pair.1.as_uuid()).collect();
        let rows = sqlx::query(
            "select p.citing, p.cited, \
                    bool_or(a.source_record_id is not null) as provider, \
                    bool_or(a.entry_id is not null) as local, \
                    count(m.id) as mentions \
             from (select distinct * from unnest($1::uuid[], $2::uuid[]) as t(citing, cited)) p \
             join graph.citation_assertion a \
               on a.citing_work_id = p.citing and a.cited_work_id = p.cited \
              and a.status in ('asserted', 'accepted') \
             left join graph.citation_mention m on m.assertion_id = a.id \
             group by p.citing, p.cited",
        )
        .bind(&citing)
        .bind(&cited)
        .fetch_all(&self.pool)
        .await?;
        let mut sources = HashMap::with_capacity(rows.len());
        for row in &rows {
            let key = (
                WorkId::from_uuid(row.try_get("citing")?),
                WorkId::from_uuid(row.try_get("cited")?),
            );
            let mentions: i64 = row.try_get("mentions")?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mention_count = mentions as usize;
            sources.insert(
                key,
                EdgeSources {
                    provider: row.try_get("provider")?,
                    local: row.try_get("local")?,
                    mention_count,
                },
            );
        }
        Ok(sources)
    }

    /// Works cited by `citing`, one row per canonical edge.
    pub async fn outgoing(&self, citing: WorkId) -> Result<Vec<Work>> {
        let rows = sqlx::query(&format!(
            "select distinct {WORK_COLUMNS} from graph.work w \
             join graph.citation_assertion a on a.cited_work_id = w.id \
             where a.citing_work_id = $1 and a.status in ('asserted', 'accepted') \
             order by w.display_year nulls last, w.id",
        ))
        .bind(citing.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(work_from_row).collect()
    }

    /// Works that cite `cited`, one row per canonical edge.
    pub async fn incoming(&self, cited: WorkId) -> Result<Vec<Work>> {
        let rows = sqlx::query(&format!(
            "select distinct {WORK_COLUMNS} from graph.work w \
             join graph.citation_assertion a on a.citing_work_id = w.id \
             where a.cited_work_id = $1 and a.status in ('asserted', 'accepted') \
             order by w.display_year nulls last, w.id",
        ))
        .bind(cited.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(work_from_row).collect()
    }

    /// Distinct `(citing, cited)` pairs of live (`asserted` or `accepted`) assertions where
    /// both Works are in `works`, ordered by pair. Self pairs cannot occur.
    pub async fn edges_among(&self, works: &[WorkId]) -> Result<Vec<(WorkId, WorkId)>> {
        if works.len() < 2 {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|work| work.as_uuid()).collect();
        let rows = sqlx::query(
            "select distinct citing_work_id, cited_work_id from graph.citation_assertion \
             where citing_work_id = any($1) and cited_work_id = any($1) \
               and citing_work_id <> cited_work_id and status in ('asserted', 'accepted') \
             order by citing_work_id, cited_work_id",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    WorkId::from_uuid(row.try_get("citing_work_id")?),
                    WorkId::from_uuid(row.try_get("cited_work_id")?),
                ))
            })
            .collect()
    }

    /// How many distinct Works each given Work cites through live assertions. Works that
    /// cite nothing are absent from the map.
    pub async fn outgoing_counts(&self, works: &[WorkId]) -> Result<HashMap<WorkId, i64>> {
        if works.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|work| work.as_uuid()).collect();
        let rows = sqlx::query(
            "select citing_work_id, count(distinct cited_work_id) as cited_count \
             from graph.citation_assertion \
             where citing_work_id = any($1) and status in ('asserted', 'accepted') \
             group by citing_work_id",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    WorkId::from_uuid(row.try_get("citing_work_id")?),
                    row.try_get("cited_count")?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_record::SourceRecordRepository;
    use crate::test_support;
    use crate::work::WorkRepository;
    use lysilogy_core::{Coverage, Visibility, WorkKind};

    #[tokio::test]
    async fn assertions_are_idempotent_and_query_both_directions() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let citations = CitationRepository::new(pool.clone());
        let records = SourceRecordRepository::new(pool);

        let source = records
            .upsert(
                "test",
                &format!("k-{}", Uuid::new_v4()),
                Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("source");
        let citing = works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("citing");
        let cited = works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("cited");

        let first = citations
            .add_assertion(citing.id, cited.id, Some(source))
            .await
            .expect("first");
        let second = citations
            .add_assertion(citing.id, cited.id, Some(source))
            .await
            .expect("second");
        assert_eq!(first, second, "same evidence reuses one assertion");

        let outgoing = citations.outgoing(citing.id).await.expect("outgoing");
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].id, cited.id);

        let incoming = citations.incoming(cited.id).await.expect("incoming");
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].id, citing.id);
    }

    #[tokio::test]
    async fn should_reuse_assertion_when_entry_edge_added_twice() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let citations = CitationRepository::new(pool.clone());
        let version = test_support::new_version(&pool).await;
        let entry = test_support::new_entries(&pool, version, 1).await[0];
        let citing = test_support::new_work(&pool).await;
        let cited = test_support::new_work(&pool).await;
        let first = citations
            .add_entry_assertion(citing, cited, entry)
            .await
            .expect("first");
        let second = citations
            .add_entry_assertion(citing, cited, entry)
            .await
            .expect("second");
        assert_eq!(first, second);
        let status: String =
            sqlx::query_scalar("select status from graph.citation_assertion where id = $1")
                .bind(first.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("status");
        assert_eq!(status, "asserted");
    }

    #[tokio::test]
    async fn should_report_provider_and_local_sources_when_edge_has_both() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let citations = CitationRepository::new(pool.clone());
        let citing = test_support::new_work(&pool).await;
        let both = test_support::new_work(&pool).await;
        let provider_only = test_support::new_work(&pool).await;
        let local_only = test_support::new_work(&pool).await;
        let unrelated = test_support::new_work(&pool).await;
        let provider = test_support::new_record(&pool).await;
        let version = test_support::new_version(&pool).await;
        let entries = test_support::new_entries(&pool, version, 2).await;

        citations
            .add_assertion(citing, both, Some(provider))
            .await
            .expect("provider both");
        citations
            .add_entry_assertion(citing, both, entries[0])
            .await
            .expect("local both");
        citations
            .add_assertion(citing, provider_only, Some(provider))
            .await
            .expect("provider");
        citations
            .add_entry_assertion(citing, local_only, entries[1])
            .await
            .expect("local");

        let sources = citations
            .edge_sources(&[
                (citing, both),
                (citing, provider_only),
                (citing, local_only),
                (citing, unrelated),
                (citing, both),
            ])
            .await
            .expect("sources");
        assert_eq!(
            sources[&(citing, both)],
            EdgeSources {
                provider: true,
                local: true,
                mention_count: 0
            }
        );
        assert_eq!(
            sources[&(citing, provider_only)],
            EdgeSources {
                provider: true,
                local: false,
                mention_count: 0
            }
        );
        assert_eq!(
            sources[&(citing, local_only)],
            EdgeSources {
                provider: false,
                local: true,
                mention_count: 0
            }
        );
        assert!(!sources.contains_key(&(citing, unrelated)));
        assert_eq!(sources.len(), 3);
        assert!(citations.edge_sources(&[]).await.expect("empty").is_empty());
    }

    #[tokio::test]
    async fn should_count_mentions_per_edge_when_grouping_sources() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let citations = CitationRepository::new(pool.clone());
        let evidence = crate::evidence::EvidenceRepository::new(pool.clone());
        let citing = test_support::new_work(&pool).await;
        let cited_a = test_support::new_work(&pool).await;
        let cited_b = test_support::new_work(&pool).await;
        let provider = test_support::new_record(&pool).await;
        let version = test_support::new_version(&pool).await;
        let entries = test_support::new_entries(&pool, version, 2).await;

        citations
            .add_assertion(citing, cited_a, Some(provider))
            .await
            .expect("provider");
        let assertion_a = citations
            .add_entry_assertion(citing, cited_a, entries[0])
            .await
            .expect("a");
        let assertion_b = citations
            .add_entry_assertion(citing, cited_b, entries[1])
            .await
            .expect("b");
        for (assertion, entry, count) in
            [(assertion_a, entries[0], 3), (assertion_b, entries[1], 1)]
        {
            for index in 0..count {
                evidence
                    .insert_mention(&crate::evidence::NewMention {
                        assertion,
                        version,
                        entry,
                        style: lysilogy_core::CitationStyle::AuthorYear,
                        marker_text: "(Smith, 2020)".into(),
                        page: 1,
                        marker_start: 100 * (index + 1),
                        marker_end: 100 * (index + 1) + 13,
                        passage_start: 100 * (index + 1) - 10,
                        passage_end: 100 * (index + 1) + 30,
                        passage: "passage".into(),
                        rects: None,
                    })
                    .await
                    .expect("mention");
            }
        }
        let sources = citations
            .edge_sources(&[(citing, cited_a), (citing, cited_b)])
            .await
            .expect("sources");
        assert_eq!(sources[&(citing, cited_a)].mention_count, 3);
        assert_eq!(sources[&(citing, cited_b)].mention_count, 1);
    }

    #[tokio::test]
    async fn should_return_edges_among_set_only_when_querying() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = CitationRepository::new(pool.clone());
        let a = test_support::new_work(&pool).await;
        let b = test_support::new_work(&pool).await;
        let c = test_support::new_work(&pool).await;
        let outside = test_support::new_work(&pool).await;
        test_support::cite(&pool, a, b).await;
        test_support::cite(&pool, a, b).await; // duplicate assertion, one edge
        repo.add_assertion(b, c, None).await.expect("accepted edge");
        test_support::cite(&pool, a, outside).await;
        test_support::cite(&pool, outside, c).await;
        let rejected = test_support::cite(&pool, c, a).await;
        sqlx::query("update graph.citation_assertion set status = 'rejected' where id = $1")
            .bind(rejected)
            .execute(&pool)
            .await
            .expect("reject");

        let mut expected = vec![(a, b), (b, c)];
        expected.sort();
        assert_eq!(repo.edges_among(&[a, b, c]).await.expect("edges"), expected);
        assert!(repo.edges_among(&[a]).await.expect("single").is_empty());
        assert!(repo.edges_among(&[]).await.expect("empty").is_empty());
        assert_eq!(
            repo.edges_among(&[a, b, c, a, b])
                .await
                .expect("repeated ids"),
            expected
        );

        let counts = repo
            .outgoing_counts(&[a, b, c, outside])
            .await
            .expect("counts");
        assert_eq!(counts.get(&a), Some(&2));
        assert_eq!(counts.get(&b), Some(&1));
        assert_eq!(counts.get(&c), None, "a rejected edge does not count");
        assert_eq!(counts.get(&outside), Some(&1));
        assert!(repo.outgoing_counts(&[]).await.expect("none").is_empty());
    }
}
