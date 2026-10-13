//! Turn provider responses into graph edges.

use lysilogy_core::{IdentifierScheme, IdentifierStatus, SourceRecordId, WorkId, WorkKind};
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::work::WorkRepository;

use crate::Result;
use crate::openalex::OpenAlexWork;

/// Ingest one `OpenAlex` work's outbound references.
///
/// The citing Work receives the work's `OpenAlex` (and DOI, when present)
/// identifier; every referenced `OpenAlex` ID is resolved to a stub Work and
/// asserted as a citation edge from the source record. Returns the number of
/// edges. Re-running with the same source record changes no row counts.
pub async fn ingest_openalex_references(
    works: &WorkRepository,
    citations: &CitationRepository,
    citing_work_id: WorkId,
    work: &OpenAlexWork,
    source_record_id: SourceRecordId,
) -> Result<usize> {
    works
        .add_identifier(
            citing_work_id,
            IdentifierScheme::Openalex,
            &work.openalex_id,
            Some(source_record_id),
            IdentifierStatus::Active,
        )
        .await?;
    if let Some(doi) = &work.doi {
        works
            .add_identifier(
                citing_work_id,
                IdentifierScheme::Doi,
                doi,
                Some(source_record_id),
                IdentifierStatus::Active,
            )
            .await?;
    }

    let mut edges = 0;
    for cited_identifier in &work.referenced_works {
        let (cited, _created) = works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                cited_identifier,
                WorkKind::Article,
                Some(source_record_id),
            )
            .await?;
        if cited.id == citing_work_id {
            continue;
        }
        citations
            .add_assertion(citing_work_id, cited.id, Some(source_record_id))
            .await?;
        edges += 1;
    }
    Ok(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lysilogy_core::Coverage;
    use lysilogy_db::source_record::SourceRecordRepository;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique() -> String {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}-{sequence}", std::process::id())
    }

    async fn pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = lysilogy_db::connect(&url).await.expect("connect");
        lysilogy_db::migrate(&pool).await.expect("migrate");
        Some(pool)
    }

    #[tokio::test]
    async fn ingesting_references_twice_is_stable() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let citations = CitationRepository::new(pool.clone());
        let records = SourceRecordRepository::new(pool);

        let marker = unique();
        let citing_doi = format!("10.7777/{marker}");
        let source = records
            .upsert(
                "openalex_work",
                &format!("doi:{citing_doi}"),
                Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("source");

        let (citing, _) = works
            .resolve_or_create(IdentifierScheme::Doi, &citing_doi, WorkKind::Article, None)
            .await
            .expect("citing work");

        let work = OpenAlexWork {
            openalex_id: format!("W9{marker}"),
            doi: Some(citing_doi),
            title: Some("A Citing Paper".to_owned()),
            year: Some(2021),
            referenced_works: vec![format!("W8{marker}"), format!("W7{marker}")],
        };

        let first = ingest_openalex_references(&works, &citations, citing.id, &work, source)
            .await
            .expect("first ingest");
        assert_eq!(first, 2);

        let second = ingest_openalex_references(&works, &citations, citing.id, &work, source)
            .await
            .expect("second ingest");
        assert_eq!(second, 2);

        let outgoing = citations.outgoing(citing.id).await.expect("outgoing");
        assert_eq!(outgoing.len(), 2, "re-ingest must not duplicate edges");
    }
}
