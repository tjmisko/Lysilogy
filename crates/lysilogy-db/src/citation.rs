//! Persistence for citation evidence and the canonical edges it forms.
//!
//! An assertion is one source's evidence that A cites B. Canonical edges group
//! the accepted assertions for a `(citing, cited)` pair.

use lysilogy_core::{AssertionId, SourceRecordId, Work, WorkId};
use sqlx::PgPool;
use uuid::Uuid;

use crate::work::{work_from_row, WORK_COLUMNS};
use crate::{DbError, Result};

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
        inserted.map(AssertionId::from_uuid).ok_or_else(|| {
            DbError::Conflict("citation assertion lost a concurrent insert".into())
        })
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
}
