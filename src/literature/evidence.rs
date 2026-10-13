//! Local evidence for one paper: the adapter from the reading index to the
//! ingest crate's plain-data [`SourceText`], and the `AppState` entry point
//! that persists and resolves the paper's bibliography.
//!
//! Offsets stay UTF-16 code units of `index.text`; the adapter copies fields
//! and never re-derives anything.

use lysilogy_ingest::bibliography::{
    self, PageRect, SourcePage, SourceParagraph, SourceText, SourceToken, TextSpan,
};
use lysilogy_ingest::labels::{
    LabelContext, LabelOptions, LabelSummary, RelationshipLabeler, label_edges,
};
use lysilogy_ingest::literature::LiteratureSummary;
use lysilogy_ingest::local_evidence::{
    LocalEvidenceContext, LocalEvidenceInput, LocalEvidenceSummary, run_local_evidence,
};
use lysilogy_ingest::scan::hash_file;
use sha2::{Digest, Sha256};

use crate::{
    AppState, Error, Result,
    domain::PaperId,
    source_index::{ReadingIndex, TextRange},
};

/// The warning reported when a paper has no Work to hang local evidence on.
pub const LOCAL_EVIDENCE_REQUIRES_WORK: &str = "local_evidence_requires_work";

/// Everything needed to find a paper's local evidence in the graph.
struct LocalEvidenceTarget {
    citing_work: lysilogy_core::WorkId,
    document_sha256: String,
    document_version: lysilogy_core::DocumentVersionId,
}

impl AppState {
    fn graph_for_literature(&self) -> Result<&crate::graph::GraphState> {
        self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "the literature graph needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })
    }

    /// The paper's Work and current document version, or `None` without a Work.
    async fn local_evidence_target(
        &self,
        id: &PaperId,
        summary: &LiteratureSummary,
    ) -> Result<Option<LocalEvidenceTarget>> {
        let Some(citing_work) = summary.work_id else {
            return Ok(None);
        };
        let graph = self.graph_for_literature()?;
        let source = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.source_path.clone())
            .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
        let (document_sha256, _) = hash_file(&source)
            .await
            .map_err(|error| Error::io(&source, error))?;
        let document_version = graph
            .documents
            .version_for_sha(summary.document_id, &document_sha256)
            .await
            .map_err(lysilogy_ingest::IngestError::from)?
            .ok_or_else(|| {
                Error::Literature(format!(
                    "no document version recorded for {id}; run lit first"
                ))
            })?;
        Ok(Some(LocalEvidenceTarget {
            citing_work,
            document_sha256,
            document_version,
        }))
    }

    /// Label the paper's local citation edges with `labeler`. Returns `None`
    /// (the caller reports [`LOCAL_EVIDENCE_REQUIRES_WORK`]) when the paper
    /// has no Work. Run [`Self::run_local_evidence`] first.
    pub async fn label_local_edges(
        &self,
        id: &PaperId,
        summary: &LiteratureSummary,
        labeler: &dyn RelationshipLabeler,
        options: LabelOptions,
    ) -> Result<Option<LabelSummary>> {
        let Some(target) = self.local_evidence_target(id, summary).await? else {
            return Ok(None);
        };
        let graph = self.graph_for_literature()?;
        let context = LabelContext {
            works: &graph.works,
            records: &graph.source_records,
            evidence: &graph.evidence,
            relationships: &graph.relationships,
        };
        let labels = label_edges(
            &context,
            target.citing_work,
            target.document_version,
            labeler,
            options,
        )
        .await?;
        Ok(Some(labels))
    }

    /// Persist and resolve the paper's local bibliography, and connect it to
    /// its in-text mentions. Returns `None` (the caller reports
    /// [`LOCAL_EVIDENCE_REQUIRES_WORK`]) when the paper has no Work.
    ///
    /// Builds the reading index if it is not cached; that index is a
    /// rebuildable cache artifact.
    pub async fn run_local_evidence(
        &self,
        id: &PaperId,
        summary: &LiteratureSummary,
    ) -> Result<Option<LocalEvidenceSummary>> {
        let Some(target) = self.local_evidence_target(id, summary).await? else {
            return Ok(None);
        };
        let LocalEvidenceTarget {
            citing_work,
            document_sha256,
            document_version,
        } = target;
        let graph = self.graph_for_literature()?;
        let index = self.reading_index(id, false).await?;
        let index_text_sha256 = format!("{:x}", Sha256::digest(index.text.as_bytes()));
        let index_schema_version = index.schema_version;
        let bibliography = tokio::task::spawn_blocking(move || {
            bibliography::extract(&source_text_from_index(&index))
        })
        .await
        .map_err(|error| Error::Task(error.to_string()))?;

        let context = LocalEvidenceContext {
            works: &graph.works,
            citations: &graph.citations,
            claims: &graph.claims,
            records: &graph.source_records,
            evidence: &graph.evidence,
        };
        let evidence = run_local_evidence(
            &context,
            LocalEvidenceInput {
                citing_work,
                document_version,
                document_sha256: &document_sha256,
                index_schema_version,
                index_text_sha256: &index_text_sha256,
                bibliography: &bibliography,
            },
        )
        .await?;
        Ok(Some(evidence))
    }
}

const fn span(range: &TextRange) -> TextSpan {
    TextSpan {
        start: range.start,
        end: range.end,
    }
}

pub fn source_text_from_index(index: &ReadingIndex) -> SourceText {
    SourceText {
        text: index.text.clone(),
        pages: index
            .pages
            .iter()
            .map(|page| SourcePage {
                number: page.number,
                start: page.start,
                end: page.end,
            })
            .collect(),
        tokens: index
            .tokens
            .iter()
            .map(|token| SourceToken {
                start: token.start,
                end: token.end,
                page: token.page,
                rects: token
                    .rects
                    .iter()
                    .map(|rect| PageRect {
                        x_min: rect.x_min,
                        y_min: rect.y_min,
                        x_max: rect.x_max,
                        y_max: rect.y_max,
                    })
                    .collect(),
                text: token.text.clone(),
            })
            .collect(),
        paragraphs: index
            .objects
            .paragraph
            .iter()
            .map(|paragraph| SourceParagraph {
                start: paragraph.start,
                end: paragraph.end,
                kind: paragraph.kind.clone(),
                spans: paragraph.spans.iter().map(span).collect(),
            })
            .collect(),
        sentences: index.objects.sentence.iter().map(span).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::test_support;

    #[test]
    fn should_copy_offsets_and_rects_when_converting_reading_index() {
        let index = test_support::reading_index_fixture();
        let source = source_text_from_index(&index);
        assert_eq!(source.text, index.text);
        assert_eq!(source.pages.len(), index.pages.len());
        for (copied, original) in source.pages.iter().zip(&index.pages) {
            assert_eq!(
                (copied.number, copied.start, copied.end),
                (original.number, original.start, original.end)
            );
        }
        assert_eq!(source.tokens.len(), index.tokens.len());
        for (copied, original) in source.tokens.iter().zip(&index.tokens) {
            assert_eq!(
                (copied.start, copied.end, copied.page, &copied.text),
                (original.start, original.end, original.page, &original.text)
            );
            assert_eq!(copied.rects.len(), original.rects.len());
            for (copied, original) in copied.rects.iter().zip(&original.rects) {
                let bits = |values: [f32; 4]| values.map(f32::to_bits);
                assert_eq!(
                    bits([copied.x_min, copied.y_min, copied.x_max, copied.y_max]),
                    bits([
                        original.x_min,
                        original.y_min,
                        original.x_max,
                        original.y_max
                    ])
                );
            }
        }
        assert_eq!(source.paragraphs.len(), index.objects.paragraph.len());
        assert_eq!(source.paragraphs[0].kind, "heading");
        assert_eq!(source.paragraphs[1].spans.len(), 2);
        assert_eq!(source.paragraphs[1].spans[1].start, 20);
        assert_eq!(source.sentences.len(), index.objects.sentence.len());
        assert_eq!(source.sentences[1].end, index.objects.sentence[1].end);
        // Astral text keeps UTF-16 offsets: the token after the emoji starts at 3.
        assert_eq!(source.tokens[1].start, 3);
    }

    use crate::source_index::test_support::cache_fixture_with_blocks;
    use lysilogy_core::{DocumentFormat, ScannedFile, Visibility, WorkKind, WorkLinkMethod};
    use lysilogy_ingest::literature::LiteratureStatus;

    fn unique() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("x{}x{nanos}", std::process::id())
    }

    async fn test_graph() -> Option<crate::graph::GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(
            crate::graph::GraphState::connect(&url)
                .await
                .expect("connect"),
        )
    }

    fn linked_summary(
        document_id: lysilogy_core::DocumentId,
        work_id: Option<lysilogy_core::WorkId>,
    ) -> LiteratureSummary {
        LiteratureSummary {
            status: LiteratureStatus::Linked,
            document_id,
            work_id,
            link_method: None,
            link_detail: None,
            openalex_id: None,
            references: 0,
            new_stub_works: 0,
            hydrated_works: 0,
            publisher: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[tokio::test]
    async fn should_run_local_evidence_from_cached_reading_index_when_paper_is_linked() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = unique();
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        let data = root.path().join("data");
        std::fs::create_dir_all(&library).unwrap();
        let file_name = format!("Someone - 2020 - Paper {marker}.pdf");
        let source = library.join(&file_name);
        std::fs::write(&source, format!("not really a pdf {marker}")).unwrap();

        let state = AppState::new(&library, &data)
            .await
            .unwrap()
            .with_graph(graph);
        let id = state.library().await.papers[0].id.clone();
        cache_fixture_with_blocks(
            &source,
            &state.store.paper_dir(&id),
            &[
                (
                    "Earlier work [1] shows this. Later work [2] agrees.",
                    "body",
                    1,
                ),
                ("References", "heading", 2),
                (
                    &format!(
                        "[1] Adams. First cached evidence {marker}. 2019.\n\
                         [2] Baker. Second cached evidence {marker}. 2020."
                    ),
                    "body",
                    2,
                ),
            ],
        )
        .await;

        let graph = state.graph.as_ref().unwrap();
        let (sha256, bytes) = hash_file(&source).await.unwrap();
        let workspace = graph.documents.ensure_default_workspace().await.unwrap();
        graph
            .documents
            .apply_partial_scan(
                workspace,
                &[ScannedFile {
                    relative_path: file_name.clone(),
                    sha256,
                    bytes,
                    format: DocumentFormat::Pdf,
                }],
            )
            .await
            .unwrap();
        let document = graph
            .documents
            .find_by_path(workspace, &file_name)
            .await
            .unwrap()
            .unwrap();
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .unwrap();
        graph
            .documents
            .set_work_link(document.id, work.id, WorkLinkMethod::Manual, None, "test")
            .await
            .unwrap();

        let summary = linked_summary(document.id, Some(work.id));
        let evidence = state
            .run_local_evidence(&id, &summary)
            .await
            .unwrap()
            .expect("evidence");
        assert_eq!(evidence.entries, 2, "{evidence:?}");
        assert_eq!(evidence.mentions, 2, "{evidence:?}");
        assert_eq!(evidence.local_edges, 2);
        assert_eq!(evidence.resolutions.local_stub, 2);

        let again = state
            .run_local_evidence(&id, &summary)
            .await
            .unwrap()
            .expect("evidence");
        assert_eq!(again.resolutions.reused, 2, "{again:?}");
        assert_eq!(again.mentions, 2);
        assert_eq!(graph.citations.outgoing(work.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn should_skip_local_evidence_when_paper_has_no_work() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("Paper.pdf"), b"x").unwrap();
        let state = AppState::new(&library, root.path().join("data"))
            .await
            .unwrap();
        let id = state.library().await.papers[0].id.clone();
        let summary = linked_summary(lysilogy_core::DocumentId::new(), None);
        let result = state.run_local_evidence(&id, &summary).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn should_return_an_empty_source_when_the_index_is_empty() {
        let index = test_support::reading_index_fixture_empty();
        let source = source_text_from_index(&index);
        assert!(source.text.is_empty());
        assert!(source.tokens.is_empty() && source.pages.is_empty());
        assert!(source.paragraphs.is_empty() && source.sentences.is_empty());
    }
}
