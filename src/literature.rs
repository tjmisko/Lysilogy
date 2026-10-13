//! The `lysilogy lit` flow for one paper: front-matter sourcing, hints from the
//! catalog, and the call into [`lysilogy_ingest::literature`].
//!
//! Nothing here writes file artifacts.

pub mod evidence;

use std::{fmt, path::Path, sync::Arc};

use lysilogy_core::{DocumentFormat, ScannedFile, first_author_family, front_matter};
use lysilogy_ingest::identify::PaperHints;
use lysilogy_ingest::literature::{
    LiteratureContext, LiteratureInput, LiteratureSummary, run_literature_flow,
};
use lysilogy_ingest::openalex::{JsonTransport, OpenAlexClient, ProviderFuture, ReqwestTransport};
use lysilogy_ingest::scan::hash_file;
use serde_json::Value;

use crate::{
    AppState, Error, Result,
    domain::PaperId,
    source_index::{self, ReadingIndex},
};

/// How many leading pages are read for identifiers.
const FRONT_PAGES: u32 = 2;

/// A shareable, type-erased transport so `AppState` stays `Clone`.
#[derive(Clone)]
pub(crate) struct SharedTransport(Arc<dyn JsonTransport>);

impl JsonTransport for SharedTransport {
    fn get_json<'a>(
        &'a self,
        url: &'a str,
    ) -> ProviderFuture<'a, std::result::Result<Value, lysilogy_ingest::openalex::ProviderError>>
    {
        self.0.get_json(url)
    }
}

/// The `OpenAlex` client held by the application.
#[derive(Clone)]
pub struct LiteratureClient(Arc<OpenAlexClient<SharedTransport>>);

impl fmt::Debug for LiteratureClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LiteratureClient")
    }
}

impl LiteratureClient {
    /// A live client; the contact email comes from the environment, if set.
    pub fn from_environment() -> Result<Self> {
        Ok(Self::new(ReqwestTransport::from_environment()?))
    }

    pub(crate) fn client(&self) -> &OpenAlexClient<SharedTransport> {
        &self.0
    }

    /// A client over any transport, for tests.
    #[must_use]
    pub fn new(transport: impl JsonTransport + 'static) -> Self {
        Self(Arc::new(OpenAlexClient::new(SharedTransport(Arc::new(
            transport,
        )))))
    }
}

impl AppState {
    /// Link one paper to a Work and ingest what it cites.
    pub async fn run_literature_flow(&self, id: &PaperId) -> Result<LiteratureSummary> {
        let graph = self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "the literature graph needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })?;
        let (source, overview) = {
            let found = self
                .catalog
                .read()
                .await
                .get(id)
                .map(|entry| (entry.source_path.clone(), entry.overview.clone()));
            found.ok_or_else(|| Error::PaperNotFound(id.to_string()))?
        };
        let (sha256, bytes) = hash_file(&source)
            .await
            .map_err(|error| Error::io(&source, error))?;
        let file = ScannedFile {
            relative_path: overview.relative_path.clone(),
            sha256,
            bytes,
            format: DocumentFormat::Pdf,
        };
        let front_matter = self.front_matter_text(id).await?;
        let first_author = overview
            .metadata
            .authors
            .first()
            .and_then(|author| first_author_family(author));
        let hints = PaperHints {
            title: Some(overview.metadata.title.as_str()).filter(|title| !title.trim().is_empty()),
            year: overview.metadata.year.map(i32::from),
            first_author: first_author.as_deref(),
        };
        let context = LiteratureContext {
            works: &graph.works,
            documents: &graph.documents,
            citations: &graph.citations,
            records: &graph.source_records,
            client: &self.literature.0,
        };
        Ok(run_literature_flow(
            &context,
            LiteratureInput {
                file,
                front_matter: &front_matter,
                hints,
            },
        )
        .await?)
    }

    /// The paper's front matter: pages 1-2 cut at the references heading.
    ///
    /// Tries a cached reading index, then a cached extraction, then a bounded
    /// `pdftotext` call. Never writes artifacts.
    pub async fn front_matter_text(&self, id: &PaperId) -> Result<String> {
        let source = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.source_path.clone())
            .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
        let pages = self.front_pages(id, &source).await?;
        let borrowed: Vec<&str> = pages.iter().map(String::as_str).collect();
        Ok(front_matter(&borrowed))
    }

    async fn front_pages(&self, id: &PaperId, source: &Path) -> Result<Vec<String>> {
        let directory = self.store.paper_dir(id);
        if let Some(cached) = source_index::load_cached(source, &directory).await? {
            let pages = index_front_pages(&cached.index);
            if has_first_page(&pages) {
                return Ok(pages.to_vec());
            }
        }
        if let Some(extraction) = self.store.load_extraction(id).await? {
            let mut pages = [String::new(), String::new()];
            for page in &extraction.pages {
                if let Some(slot) = usize::try_from(page.number)
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .and_then(|index| pages.get_mut(index))
                {
                    slot.clone_from(&page.text);
                }
            }
            if has_first_page(&pages) {
                return Ok(pages.to_vec());
            }
        }
        let text = self
            .extractor
            .extract_front_text(source, FRONT_PAGES)
            .await?;
        Ok(vec![text])
    }
}

fn has_first_page(pages: &[String; 2]) -> bool {
    !pages[0].trim().is_empty()
}

/// Text of pages 1 and 2, sliced from the index by its UTF-16 page bounds.
/// Paragraph breaks survive, so a references heading stays on its own line.
fn index_front_pages(index: &ReadingIndex) -> [String; 2] {
    let mut pages = [String::new(), String::new()];
    for page in &index.pages {
        let Some(slot) = usize::try_from(page.number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|position| pages.get_mut(position))
        else {
            continue;
        };
        *slot = utf16_slice(&index.text, page.start, page.end);
    }
    pages
}

fn utf16_slice(text: &str, start: usize, end: usize) -> String {
    let mut position = 0;
    let mut slice = String::new();
    for character in text.chars() {
        if position >= end {
            break;
        }
        if position >= start {
            slice.push(character);
        }
        position += character.len_utf16();
    }
    slice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::test_support::cache_fixture_with_pages;

    #[tokio::test]
    async fn should_prefer_reading_index_text_when_cached() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        let data = root.path().join("data");
        std::fs::create_dir_all(&library).unwrap();
        // Not a real PDF: any attempt to run pdftotext on it would fail.
        let source = library.join("Someone - 2020 - A Paper.pdf");
        std::fs::write(&source, b"not really a pdf").unwrap();

        let state = AppState::new(&library, &data).await.unwrap();
        let id = state.library().await.papers[0].id.clone();
        let directory = state.store.paper_dir(&id);
        cache_fixture_with_pages(
            &source,
            &directory,
            &[
                (1, "A Title\n\nDOI: 10.1257/aer.103.6.2121"),
                (2, "More text.\n\nReferences\n\n[1] 10.9999/after.heading"),
                (3, "ignored page"),
            ],
        )
        .await;

        let text = state.front_matter_text(&id).await.unwrap();
        assert!(text.contains("10.1257/aer.103.6.2121"), "{text}");
        assert!(!text.contains("10.9999"), "cut at the references heading");
        assert!(!text.contains("ignored"), "pages past 2 are excluded");
    }

    #[test]
    fn should_slice_by_utf16_units_when_text_has_astral_characters() {
        // The emoji is two UTF-16 units, one char.
        assert_eq!(utf16_slice("a\u{1F600}bc", 3, 5), "bc");
        assert_eq!(utf16_slice("abc", 1, 99), "bc");
        assert_eq!(utf16_slice("abc", 2, 1), "");
    }

    #[tokio::test]
    async fn should_fall_back_to_pdftotext_when_no_cache_exists() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        let data = root.path().join("data");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("Paper.pdf"), b"not really a pdf").unwrap();
        let program = root.path().join("pdftotext");
        std::fs::write(&program, "#!/bin/sh\necho 'arXiv:1606.06565v2'\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        let state = AppState::with_services(
            &library,
            &data,
            crate::extract::PdfExtractor::with_programs(program, "pdfinfo"),
            crate::analysis::AnalysisService::default(),
        )
        .await
        .unwrap();
        let id = state.library().await.papers[0].id.clone();
        let text = state.front_matter_text(&id).await.unwrap();
        assert!(text.contains("arXiv:1606.06565v2"), "{text}");
        assert!(
            !state
                .store
                .paper_dir(&id)
                .join("reading-index.json")
                .exists(),
            "front matter sourcing must not write artifacts"
        );
    }

    #[tokio::test]
    async fn should_require_a_graph_when_running_the_literature_flow() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("Paper.pdf"), b"x").unwrap();
        let state = AppState::new(&library, root.path().join("data"))
            .await
            .unwrap();
        let id = state.library().await.papers[0].id.clone();
        let error = state.run_literature_flow(&id).await.unwrap_err();
        assert!(
            matches!(&error, Error::InvalidRequest(message) if message.contains("LYSILOGY_DATABASE_URL")),
            "{error}"
        );
    }
}
