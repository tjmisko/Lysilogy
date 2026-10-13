//! Link local documents to Works by detecting their bibliographic identifiers.

use lysilogy_core::{
    DetectedIdentifier, DocumentId, FieldKind, FieldState, IdentifierScheme, IdentifierStatus,
    Work, WorkKind, extract_identifiers,
};
use lysilogy_db::document::DocumentRepository;
use lysilogy_db::work::WorkRepository;

use crate::Result;

/// Optional bibliographic hints taken from a paper's own extracted metadata.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaperHints<'a> {
    pub title: Option<&'a str>,
    pub year: Option<i32>,
}

/// Extract identifiers from a document's text, resolve or create its Work, link
/// the two, and record any supplied title/year hints. Returns `None` when the
/// text carries no usable identifier.
pub async fn link_document_work(
    works: &WorkRepository,
    documents: &DocumentRepository,
    document_id: DocumentId,
    text: &str,
    hints: PaperHints<'_>,
) -> Result<Option<Work>> {
    let detected = extract_identifiers(text);
    let Some(primary) = primary_identifier(&detected) else {
        return Ok(None);
    };

    let (work, _created) = works
        .resolve_or_create(primary.scheme, &primary.value, WorkKind::Article, None)
        .await?;
    let work_id = work.id;

    for other in &detected {
        if other.scheme == primary.scheme && other.value == primary.value {
            continue;
        }
        works
            .add_identifier(
                work_id,
                other.scheme,
                &other.value,
                None,
                IdentifierStatus::Active,
            )
            .await?;
    }

    if let Some(title) = hints.title.filter(|title| !title.trim().is_empty()) {
        works
            .add_claim(
                work_id,
                FieldKind::Title,
                &serde_json::json!(title),
                FieldState::Known,
                None,
                None,
            )
            .await?;
        works.set_display_title(work_id, title).await?;
    }
    if let Some(year) = hints.year {
        works
            .add_claim(
                work_id,
                FieldKind::Year,
                &serde_json::json!(year),
                FieldState::Known,
                None,
                None,
            )
            .await?;
        works.set_display_year(work_id, year).await?;
    }

    documents.set_work(document_id, Some(work_id)).await?;
    Ok(works.get(work_id).await?)
}

/// Prefer a DOI, then an arXiv ID, then whatever was found first.
fn primary_identifier(detected: &[DetectedIdentifier]) -> Option<DetectedIdentifier> {
    detected
        .iter()
        .find(|identifier| identifier.scheme == IdentifierScheme::Doi)
        .or_else(|| {
            detected
                .iter()
                .find(|identifier| identifier.scheme == IdentifierScheme::Arxiv)
        })
        .or_else(|| detected.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lysilogy_core::{DocumentFormat, ScannedFile};
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

    async fn scanned_document(
        documents: &DocumentRepository,
    ) -> (lysilogy_core::WorkspaceId, DocumentId) {
        let workspace = documents
            .create_workspace(&format!("test-{}", unique()))
            .await
            .expect("workspace");
        let marker = unique();
        let path = format!("fixtures/{marker}.pdf");
        documents
            .apply_scan(
                workspace,
                &[ScannedFile {
                    relative_path: path.clone(),
                    sha256: format!("sha-{marker}"),
                    bytes: 1,
                    format: DocumentFormat::Pdf,
                }],
            )
            .await
            .expect("scan");
        let id = documents
            .find_by_path(workspace, &path)
            .await
            .expect("find")
            .expect("present")
            .id;
        (workspace, id)
    }

    #[tokio::test]
    async fn links_document_to_work_by_doi() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let documents = DocumentRepository::new(pool);
        let (workspace, document_id) = scanned_document(&documents).await;

        let doi = format!("10.5555/{}", unique());
        let text = format!("A paper. DOI: {doi}. arXiv:2104.01511v3");
        let work = link_document_work(
            &works,
            &documents,
            document_id,
            &text,
            PaperHints {
                title: Some("A Study"),
                year: Some(2020),
            },
        )
        .await
        .expect("link")
        .expect("work");

        assert_eq!(work.display_title.as_deref(), Some("A Study"));
        assert_eq!(work.display_year, Some(2020));
        assert_eq!(
            work.title_state,
            lysilogy_core::FieldState::Known
        );

        // Both identifiers belong to one Work.
        assert!(works
            .find_by_identifier(IdentifierScheme::Doi, &doi)
            .await
            .expect("find")
            .is_some());
        assert!(works
            .find_by_identifier(IdentifierScheme::Arxiv, "2104.01511")
            .await
            .expect("find")
            .is_some());

        // The document now points at the Work, and re-linking reuses it.
        let relinked = link_document_work(
            &works,
            &documents,
            document_id,
            &text,
            PaperHints::default(),
        )
        .await
        .expect("relink")
        .expect("work");
        assert_eq!(relinked.id, work.id);
        let document = documents
            .list_documents(workspace)
            .await
            .expect("list")
            .into_iter()
            .find(|document| document.id == document_id)
            .expect("document");
        assert_eq!(document.work_id, Some(work.id));
    }

    #[tokio::test]
    async fn link_returns_none_without_an_identifier() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let works = WorkRepository::new(pool.clone());
        let documents = DocumentRepository::new(pool);
        let (_workspace, document_id) = scanned_document(&documents).await;

        let linked = link_document_work(
            &works,
            &documents,
            document_id,
            "This paper cites nothing identifiable.",
            PaperHints::default(),
        )
        .await
        .expect("link");
        assert!(linked.is_none());
    }
}
