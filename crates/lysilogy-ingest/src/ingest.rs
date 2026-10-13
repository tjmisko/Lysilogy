//! Turn provider responses into graph edges.

use std::collections::HashMap;

use lysilogy_core::{
    FieldKind, FieldState, IdentifierScheme, IdentifierStatus, SourceRecordId, Work, WorkId,
    WorkKind,
};
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;

use crate::Result;
use crate::openalex::{JsonTransport, OpenAlexClient, OpenAlexWork, fetch_summaries_cached};

/// What one reference ingest changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceIngestReport {
    /// Citation edges asserted (re-asserting an existing edge still counts).
    pub edges: usize,
    /// Stub Works created for references not seen before.
    pub new_works: usize,
}

/// Ingest one `OpenAlex` work's outbound references.
///
/// The citing Work receives the work's `OpenAlex` (and DOI, when present)
/// identifier; every referenced `OpenAlex` ID is resolved to a stub Work and
/// asserted as a citation edge from the source record. Returns the edge and new-Work
/// counts. Re-running with the same source record changes no row counts.
pub async fn ingest_openalex_references(
    works: &WorkRepository,
    citations: &CitationRepository,
    citing_work_id: WorkId,
    work: &OpenAlexWork,
    source_record_id: SourceRecordId,
) -> Result<ReferenceIngestReport> {
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
    let mut new_works = 0;
    for cited_identifier in &work.referenced_works {
        let (cited, created) = works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                cited_identifier,
                WorkKind::Article,
                Some(source_record_id),
            )
            .await?;
        if created {
            new_works += 1;
        }
        if cited.id == citing_work_id {
            continue;
        }
        citations
            .add_assertion(citing_work_id, cited.id, Some(source_record_id))
            .await?;
        edges += 1;
    }
    Ok(ReferenceIngestReport { edges, new_works })
}

/// Give untitled Works their title and year from batched `OpenAlex` summaries.
///
/// Only Works whose title is not yet known are considered. Returns how many
/// were given a title.
pub async fn hydrate_stub_works<T: JsonTransport>(
    works: &WorkRepository,
    records: &SourceRecordRepository,
    client: &OpenAlexClient<T>,
    cited: &[Work],
) -> Result<usize> {
    let stub_ids: Vec<WorkId> = cited
        .iter()
        .filter(|work| work.title_state != FieldState::Known)
        .map(|work| work.id)
        .collect();
    if stub_ids.is_empty() {
        return Ok(0);
    }
    let openalex_ids = works.openalex_ids_for(&stub_ids).await?;
    let requested: Vec<String> = openalex_ids
        .iter()
        .map(|(_, id)| id.to_ascii_uppercase())
        .collect();
    let summaries: HashMap<String, _> = fetch_summaries_cached(records, client, &requested)
        .await?
        .into_iter()
        .map(|(record_id, summary)| {
            (
                summary.openalex_id.to_ascii_lowercase(),
                (record_id, summary),
            )
        })
        .collect();

    let mut titled = 0;
    for (work_id, openalex_id) in &openalex_ids {
        let Some((record_id, summary)) = summaries.get(&openalex_id.to_ascii_lowercase()) else {
            continue;
        };
        if let Some(year) = summary.year {
            works
                .add_claim_once(
                    *work_id,
                    FieldKind::Year,
                    &serde_json::json!(year),
                    FieldState::Known,
                    Some(*record_id),
                    None,
                )
                .await?;
            works.fill_display_year_if_unknown(*work_id, year).await?;
        }
        if let Some(doi) = &summary.doi {
            // A conflicting active DOI is not an error here.
            works
                .add_identifier(
                    *work_id,
                    IdentifierScheme::Doi,
                    doi,
                    Some(*record_id),
                    IdentifierStatus::Active,
                )
                .await?;
        }
        let Some(title) = &summary.title else {
            continue;
        };
        works
            .add_claim_once(
                *work_id,
                FieldKind::Title,
                &serde_json::json!(title),
                FieldState::Known,
                Some(*record_id),
                None,
            )
            .await?;
        if works.fill_display_title_if_unknown(*work_id, title).await? {
            titled += 1;
        }
    }
    Ok(titled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openalex::fake::RecordingTransport;
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
        assert_eq!(
            first,
            ReferenceIngestReport {
                edges: 2,
                new_works: 2
            }
        );

        let second = ingest_openalex_references(&works, &citations, citing.id, &work, source)
            .await
            .expect("second ingest");
        assert_eq!(
            second,
            ReferenceIngestReport {
                edges: 2,
                new_works: 0
            }
        );

        let outgoing = citations.outgoing(citing.id).await.expect("outgoing");
        assert_eq!(outgoing.len(), 2, "re-ingest must not duplicate edges");
    }

    #[tokio::test]
    async fn should_count_new_stub_works_when_ingesting_references() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let citations = CitationRepository::new(pool.clone());
        let records = SourceRecordRepository::new(pool);
        let marker = unique();
        let source = records
            .upsert(
                "openalex_work",
                &format!("count-{marker}"),
                Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .expect("source");
        let (citing, _) = works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                &format!("W5{marker}"),
                WorkKind::Article,
                None,
            )
            .await
            .expect("citing");
        // One reference already exists as a Work.
        works
            .resolve_or_create(
                IdentifierScheme::Openalex,
                &format!("W6{marker}"),
                WorkKind::Article,
                None,
            )
            .await
            .expect("existing");
        let work = OpenAlexWork {
            openalex_id: format!("W5{marker}"),
            doi: None,
            title: None,
            year: None,
            referenced_works: vec![
                format!("W6{marker}"),
                format!("W4{marker}"),
                format!("W3{marker}"),
            ],
        };
        let report = ingest_openalex_references(&works, &citations, citing.id, &work, source)
            .await
            .expect("ingest");
        assert_eq!(
            report,
            ReferenceIngestReport {
                edges: 3,
                new_works: 2
            }
        );
    }

    #[tokio::test]
    async fn should_title_stub_works_when_hydrating() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let records = SourceRecordRepository::new(pool);
        let marker = unique();
        let titled_id = format!("W1{marker}");
        let untitled_id = format!("W2{marker}");
        let missing_id = format!("W3{marker}");
        let mut stubs = Vec::new();
        for id in [&titled_id, &untitled_id, &missing_id] {
            let (work, _) = works
                .resolve_or_create(IdentifierScheme::Openalex, id, WorkKind::Article, None)
                .await
                .expect("stub");
            stubs.push(work);
        }
        works
            .set_display_title(stubs[0].id, "Curated Title")
            .await
            .expect("curate");
        let stubs: Vec<Work> = {
            let mut refreshed = Vec::new();
            for stub in &stubs {
                refreshed.push(works.get(stub.id).await.expect("get").expect("exists"));
            }
            refreshed
        };

        let untitled = untitled_id.clone();
        let titled = titled_id.clone();
        let transport = RecordingTransport::new(move |_| {
            serde_json::json!({ "results": [
                {"id": format!("https://openalex.org/{untitled}"), "display_name": "Hydrated Title",
                 "publication_year": 2019, "doi": format!("https://doi.org/10.5555/{untitled}")},
                {"id": format!("https://openalex.org/{titled}"), "display_name": "Provider Title"},
            ]})
        });
        let client = OpenAlexClient::new(transport.clone());

        let count = hydrate_stub_works(&works, &records, &client, &stubs)
            .await
            .expect("hydrate");
        assert_eq!(
            count, 1,
            "only the untitled stub with a provider record is titled"
        );
        assert_eq!(transport.urls().len(), 1);
        assert!(
            !transport.urls()[0].contains(&titled_id),
            "known titles are not requested"
        );

        let hydrated = works.get(stubs[1].id).await.expect("get").expect("exists");
        assert_eq!(hydrated.display_title.as_deref(), Some("Hydrated Title"));
        assert_eq!(hydrated.display_year, Some(2019));
        let unchanged = works.get(stubs[0].id).await.expect("get").expect("exists");
        assert_eq!(unchanged.display_title.as_deref(), Some("Curated Title"));
        let skipped = works.get(stubs[2].id).await.expect("get").expect("exists");
        assert_eq!(skipped.display_title, None);

        let again = hydrate_stub_works(&works, &records, &client, &stubs)
            .await
            .expect("rehydrate");
        assert_eq!(again, 0, "stale Work snapshot must not double count");
    }
}
