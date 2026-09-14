//! Deterministic, paper-local objects derived from a particular reading index.
//! Enrichment belongs in a separate artifact and never changes these source facts.

use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    domain::{PaperId, TextRect},
    source_index::{
        FIGURE_DETECTOR_VERSION, Figure, IndexDocument, detect_figures, detect_figures_with_images,
        graphics,
    },
    store::write_atomic,
};

pub const OBJECTS_FILE: &str = "objects.json";
pub const ENRICHMENT_FILE: &str = "objects-enrichment.json";
pub const SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectsArtifact {
    pub schema_version: u16,
    pub paper_id: PaperId,
    /// Opaque fingerprint (`IndexDocument.etag`) of the exact persisted index.
    /// A rebuilt index has a new generation even when the PDF is unchanged.
    pub reading_index_generation: String,
    /// Current derived figure/table behavior, independent of native schema/version.
    #[serde(default)]
    pub figure_detector_version: u16,
    /// Fingerprint of the exact native generation and the derived detector version.
    #[serde(default)]
    pub figure_detector_generation: String,
    /// Optional PDF placements, independently versioned from immutable native text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics: Option<graphics::GraphicsEvidence>,
    pub objects: Vec<PaperObject>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperObject {
    /// Stable within this paper; independent of cache generation and model output.
    pub id: String,
    #[serde(flatten)]
    pub kind: PaperObjectKind,
    pub label: String,
    pub page: u32,
    /// Authored caption, statement, equation, or raw bibliography entry.
    pub text: String,
    /// Coordinates of `text`, distinct from surrounding diagram/table membership.
    pub anchor: ReadingIndexAnchor,
    /// Disjoint source members of the object (such as diagram labels and cells).
    /// Empty means no membership beyond the authored text has been identified.
    pub member_anchors: Vec<ReadingIndexAnchor>,
    /// Conservative detector candidate in PDF points, not verified segmentation.
    pub region: Option<TextRect>,
    /// Preserve the detector's confidence label without inventing a numeric score.
    pub confidence: String,
    pub mentions: Vec<ObjectMention>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaperObjectKind {
    Figure,
    Table,
    Equation,
    Statement { statement_kind: StatementKind },
    Proof { statement_id: Option<String> },
    Algorithm,
    BibEntry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementKind {
    Theorem,
    Lemma,
    Proposition,
    Corollary,
    Definition,
    Remark,
}

/// Half-open UTF-16 code-unit offsets into the complete reading-index text.
///
/// These are never layout token IDs, byte offsets, or page-relative offsets.
/// The containing artifact supplies the generation needed to resolve them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadingIndexAnchor {
    pub page: u32,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectMention {
    pub anchor: ReadingIndexAnchor,
    pub rects: Vec<TextRect>,
}

impl ObjectsArtifact {
    /// Recompute current deterministic objects from immutable native coordinates.
    /// The historical `ReadingIndex.figures` cache is never authoritative here.
    #[must_use]
    pub fn from_reading_index(paper_id: &PaperId, document: &IndexDocument) -> Self {
        Self::from_figures(paper_id, document, &detect_figures(&document.index))
    }

    fn from_figures(paper_id: &PaperId, document: &IndexDocument, figures: &[Figure]) -> Self {
        let mut ids = BTreeSet::new();
        let objects = figures
            .iter()
            .map(|figure| {
                let mut object = PaperObject::from_figure(figure);
                let base_id = object.id.clone();
                let mut occurrence = 1_u32;
                while !ids.insert(object.id.clone()) {
                    occurrence += 1;
                    object.id = format!("{base_id}-{occurrence}");
                }
                object
            })
            .collect();
        Self {
            schema_version: SCHEMA_VERSION,
            paper_id: paper_id.clone(),
            reading_index_generation: document.etag.clone(),
            figure_detector_version: FIGURE_DETECTOR_VERSION,
            figure_detector_generation: figure_detector_generation(&document.etag),
            graphics: None,
            objects,
        }
    }
}

#[must_use]
pub fn figure_detector_generation(native_generation: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "figures:{FIGURE_DETECTOR_VERSION}:{native_generation}"
        ))
    )
}

#[must_use]
pub fn source_detector_generation(native_generation: &str, graphics_generation: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "figures:{FIGURE_DETECTOR_VERSION}:{native_generation}:graphics:{graphics_generation}"
        ))
    )
}

pub struct SourceObjects {
    pub artifact: ObjectsArtifact,
    /// Raw trace bytes are available to explicit experiments, never persisted by
    /// the product in its object cache or native reading index.
    pub traces: Vec<(u32, Vec<u8>)>,
    pub mask_receipts: Vec<(u32, Vec<u8>)>,
}

async fn derive_source(
    source: &Path,
    paper_id: &PaperId,
    document: &IndexDocument,
    prepared: graphics::PreparedGraphics,
) -> Result<SourceObjects> {
    let pages = detect_figures(&document.index)
        .iter()
        .map(|figure| figure.page)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let graphics = graphics::collect(source, document, &pages, prepared).await?;
    let mut artifact = ObjectsArtifact::from_figures(
        paper_id,
        document,
        &detect_figures_with_images(&document.index, &graphics.evidence.pages),
    );
    artifact.figure_detector_generation =
        source_detector_generation(&document.etag, &graphics.evidence.generation);
    artifact.graphics = Some(graphics.evidence);
    Ok(SourceObjects {
        artifact,
        traces: graphics.traces,
        mask_receipts: graphics.mask_receipts,
    })
}

/// The shared current production path. Derive into separate artifacts; never
/// rewrite the native index, its old figures, registry, or source PDF.
pub async fn from_source(
    source: &Path,
    paper_id: &PaperId,
    document: &IndexDocument,
) -> Result<SourceObjects> {
    let prepared = graphics::prepare(source, document).await?;
    derive_source(source, paper_id, document, prepared).await
}

/// The source-aware cache invalidates native-only and previous tool generations.
pub async fn load_or_build_from_source(
    source: &Path,
    directory: &Path,
    paper_id: &PaperId,
    document: &IndexDocument,
) -> Result<ObjectsArtifact> {
    let prepared = graphics::prepare(source, document).await?;
    let path = directory.join(OBJECTS_FILE);
    if let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(cached) = serde_json::from_slice::<ObjectsArtifact>(&bytes)
        && cached.schema_version == SCHEMA_VERSION
        && cached.paper_id == *paper_id
        && cached.reading_index_generation == document.etag
        && cached.figure_detector_version == FIGURE_DETECTOR_VERSION
        && let Some(evidence) = &cached.graphics
        && evidence.reusable_for(&prepared)
        && evidence
            .pages
            .iter()
            .map(|page| page.page)
            .collect::<Vec<_>>()
            == detect_figures(&document.index)
                .iter()
                .map(|figure| figure.page)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        && cached.figure_detector_generation
            == source_detector_generation(&document.etag, &evidence.generation)
    {
        return Ok(cached);
    }
    let artifact = derive_source(source, paper_id, document, prepared)
        .await?
        .artifact;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| Error::io(directory, error))?;
    write_atomic(&path, &serde_json::to_vec(&artifact)?).await?;
    Ok(artifact)
}

impl PaperObject {
    fn from_figure(figure: &Figure) -> Self {
        let table = figure.kind == "table" || figure.id.starts_with("table-");
        let id = if table {
            figure
                .id
                .strip_prefix("table-")
                .map(|label| format!("tab-{label}"))
        } else {
            figure
                .id
                .strip_prefix("figure-")
                .map(|label| format!("fig-{label}"))
        }
        .unwrap_or_else(|| figure.id.clone());
        Self {
            id,
            kind: if table {
                PaperObjectKind::Table
            } else {
                PaperObjectKind::Figure
            },
            label: figure.label.clone(),
            page: figure.page,
            text: figure.caption.clone(),
            anchor: ReadingIndexAnchor {
                page: figure.page,
                start: figure.start,
                end: figure.end,
            },
            member_anchors: figure
                .spans
                .iter()
                .map(|span| ReadingIndexAnchor {
                    page: figure.page,
                    start: span.start,
                    end: span.end,
                })
                .collect(),
            region: figure.rect,
            confidence: figure.confidence.clone(),
            mentions: figure
                .references
                .iter()
                .map(|reference| ObjectMention {
                    anchor: ReadingIndexAnchor {
                        page: reference.page,
                        start: reference.start,
                        end: reference.end,
                    },
                    rects: reference.rects.clone(),
                })
                .collect(),
        }
    }
}

/// Reuse only artifacts derived from this exact paper, index generation, and
/// object schema. Stale or malformed cache data is replaced atomically.
pub async fn load_or_build(
    directory: &Path,
    paper_id: &PaperId,
    document: &IndexDocument,
) -> Result<ObjectsArtifact> {
    let path = directory.join(OBJECTS_FILE);
    if let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(cached) = serde_json::from_slice::<ObjectsArtifact>(&bytes)
        && cached.schema_version == SCHEMA_VERSION
        && cached.paper_id == *paper_id
        && cached.reading_index_generation == document.etag
        && cached.figure_detector_version == FIGURE_DETECTOR_VERSION
        && cached.figure_detector_generation == figure_detector_generation(&document.etag)
    {
        return Ok(cached);
    }
    let artifact = ObjectsArtifact::from_reading_index(paper_id, document);
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| Error::io(directory, error))?;
    write_atomic(&path, &serde_json::to_vec(&artifact)?).await?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::source_index::{ReadingIndex, SCHEMA_VERSION as INDEX_SCHEMA_VERSION};

    fn paper_id() -> PaperId {
        PaperId::from_relative_path(Path::new("fixture.pdf"))
    }

    fn document() -> IndexDocument {
        let mut index: ReadingIndex = serde_json::from_value(json!({
            "schema_version": INDEX_SCHEMA_VERSION,
            "text": "😀 Figure 3. A caption. Diagram. See Figure 3.",
            "pages": [], "tokens": [],
            "objects": {"word": [], "WORD": [], "sentence": [], "paragraph": []},
            "figures": [{
                "id": "figure-3", "kind": "figure", "label": "Figure 3", "page": 1,
                "caption": "Figure 3. A caption.", "start": 3, "end": 23,
                "spans": [{"start": 3, "end": 23}, {"start": 24, "end": 32}],
                "rect": {"x_min": 1.0, "y_min": 2.0, "x_max": 100.0, "y_max": 110.0},
                "confidence": "candidate", "references": [{
                    "page": 1, "start": 37, "end": 45,
                    "rects": [{"x_min": 1.0, "y_min": 120.0, "x_max": 70.0, "y_max": 130.0}]
                }]
            }], "gaps": []
        }))
        .unwrap();
        index.pages = serde_json::from_value(json!([{"number":1,"width":600.0,"height":800.0,"start":0,"end":46,"provenance":"native","confidence":null}])).unwrap();
        index.objects.paragraph = serde_json::from_value(json!([
            {"start":3,"end":23,"kind":"caption"}, {"start":24,"end":32,"kind":"float"}, {"start":33,"end":46,"kind":"body"}
        ])).unwrap();
        for (start, end, text, x, y) in [
            (3, 9, "Figure", 10.0, 200.0),
            (10, 12, "3.", 45.0, 200.0),
            (13, 14, "A", 60.0, 200.0),
            (15, 23, "caption.", 70.0, 200.0),
            (24, 32, "Diagram.", 10.0, 100.0),
            (33, 36, "See", 10.0, 300.0),
            (37, 43, "Figure", 40.0, 300.0),
            (44, 46, "3.", 80.0, 300.0),
        ] {
            index.tokens.push(serde_json::from_value(json!({"start":start,"end":end,"text":text,"page":1,"rects":[{"x_min":x,"x_max":x+30.0,"y_min":y,"y_max":y+10.0}],"provenance":"native"})).unwrap());
        }
        IndexDocument {
            index,
            etag: "\"generation-1\"".into(),
        }
    }

    #[tokio::test]
    async fn should_replace_native_only_objects_when_the_source_factory_supplies_a_new_generation()
    {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("synthetic.pdf");
        // No caption pages means no native command; this test exercises source
        // identity and cache invalidation independently of installed tools.
        tokio::fs::write(&source, b"independently specified source")
            .await
            .unwrap();
        let mut document = document();
        document.index.objects.paragraph.clear();
        let id = paper_id();
        let original = load_or_build(directory.path(), &id, &document)
            .await
            .unwrap();
        let native_before = serde_json::to_vec(&document.index).unwrap();
        let current = load_or_build_from_source(&source, directory.path(), &id, &document)
            .await
            .unwrap();
        assert!(current.graphics.is_some());
        assert!(current.objects.is_empty());
        assert_ne!(
            current.figure_detector_generation,
            original.figure_detector_generation
        );
        assert_eq!(serde_json::to_vec(&document.index).unwrap(), native_before);
        let reused = load_or_build_from_source(&source, directory.path(), &id, &document)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&current).unwrap(),
            serde_json::to_value(reused).unwrap()
        );
        tokio::fs::write(&source, b"independently changed source")
            .await
            .unwrap();
        let changed = load_or_build_from_source(&source, directory.path(), &id, &document)
            .await
            .unwrap();
        assert_ne!(
            current.figure_detector_generation,
            changed.figure_detector_generation
        );
        assert_eq!(
            changed.reading_index_generation,
            current.reading_index_generation
        );
    }

    #[test]
    fn should_preserve_detector_evidence_when_wrapping_figures_and_tables() {
        let mut document = document();
        let mut table = document.index.figures[0].clone();
        table.id = "table-2".into();
        table.kind = "table".into();
        table.label = "Table 2".into();
        table.rect = None;
        document.index.figures.push(table);
        let artifact =
            ObjectsArtifact::from_figures(&paper_id(), &document, &document.index.figures);
        assert_eq!(artifact.objects.len(), 2);
        let figure = &artifact.objects[0];
        assert_eq!(figure.id, "fig-3");
        assert_eq!(figure.kind, PaperObjectKind::Figure);
        assert_eq!(figure.label, "Figure 3");
        assert_eq!(figure.text, document.index.figures[0].caption);
        assert_eq!(figure.confidence, "candidate");
        assert_eq!(
            figure.anchor,
            ReadingIndexAnchor {
                page: 1,
                start: 3,
                end: 23
            }
        );
        assert_eq!(
            figure.member_anchors,
            vec![
                ReadingIndexAnchor {
                    page: 1,
                    start: 3,
                    end: 23
                },
                ReadingIndexAnchor {
                    page: 1,
                    start: 24,
                    end: 32
                },
            ]
        );
        assert_eq!(figure.region, document.index.figures[0].rect);
        assert_eq!(figure.mentions[0].anchor.start, 37);
        assert_eq!(figure.mentions[0].anchor.end, 45);
        assert_eq!(
            figure.mentions[0].rects,
            document.index.figures[0].references[0].rects
        );
        assert_eq!(artifact.objects[1].id, "tab-2");
        assert_eq!(artifact.objects[1].kind, PaperObjectKind::Table);
        assert!(artifact.objects[1].region.is_none());
    }

    #[test]
    fn should_keep_object_ids_stable_when_unchanged_index_is_rebuilt() {
        let original = document();
        let mut rebuilt = original.clone();
        rebuilt.etag = "\"generation-2\"".into();
        let first = ObjectsArtifact::from_reading_index(&paper_id(), &original);
        let second = ObjectsArtifact::from_reading_index(&paper_id(), &rebuilt);
        assert_ne!(
            first.reading_index_generation,
            second.reading_index_generation
        );
        assert_eq!(
            serde_json::to_value(first.objects).unwrap(),
            serde_json::to_value(second.objects).unwrap()
        );
    }

    #[tokio::test]
    async fn should_keep_object_ids_stable_when_native_pdf_index_is_rebuilt() {
        use crate::source_index::{
            BuildPriority, load_or_build_priority,
            test_support::{has_pdftotext, native_pdf},
        };

        if !has_pdftotext() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("fixture.pdf");
        tokio::fs::write(
            &source,
            native_pdf("Figure 3. A small diagram is described by this generated native caption."),
        )
        .await
        .unwrap();
        let artifacts = directory.path().join("artifacts");
        let original =
            load_or_build_priority(&source, &artifacts, false, BuildPriority::Interactive)
                .await
                .unwrap();
        let rebuilt = load_or_build_priority(&source, &artifacts, true, BuildPriority::Interactive)
            .await
            .unwrap();
        let first = ObjectsArtifact::from_reading_index(&paper_id(), &original);
        let second = ObjectsArtifact::from_reading_index(&paper_id(), &rebuilt);
        assert_eq!(
            first.objects.first().map(|object| object.id.as_str()),
            Some("fig-3")
        );
        assert_ne!(
            first.reading_index_generation,
            second.reading_index_generation
        );
        assert_eq!(
            serde_json::to_value(first.objects).unwrap(),
            serde_json::to_value(second.objects).unwrap()
        );
    }

    #[test]
    fn should_disambiguate_ids_when_printed_labels_repeat() {
        let mut document = document();
        document
            .index
            .figures
            .push(document.index.figures[0].clone());
        let first = ObjectsArtifact::from_figures(&paper_id(), &document, &document.index.figures);
        assert_eq!(first.objects[0].id, "fig-3");
        assert_eq!(first.objects[1].id, "fig-3-2");
        document.etag = "\"rebuilt\"".into();
        let second = ObjectsArtifact::from_figures(&paper_id(), &document, &document.index.figures);
        assert_eq!(first.objects[1].id, second.objects[1].id);
    }

    #[test]
    fn should_preserve_legacy_tables_when_kind_field_is_empty() {
        let mut document = document();
        document.index.figures[0].kind.clear();
        document.index.figures[0].id = "table-2".into();
        let artifact =
            ObjectsArtifact::from_figures(&paper_id(), &document, &document.index.figures);
        assert_eq!(artifact.objects[0].kind, PaperObjectKind::Table);
        assert_eq!(artifact.objects[0].id, "tab-2");
    }

    #[test]
    fn should_serialize_every_kind_when_objects_have_reading_index_anchors() {
        let template = ObjectsArtifact::from_reading_index(&paper_id(), &document());
        let mut kinds = vec![
            json!({"kind": "figure"}),
            json!({"kind": "table"}),
            json!({"kind": "equation"}),
            json!({"kind": "proof", "statement_id": "thm-2.1"}),
            json!({"kind": "proof", "statement_id": null}),
            json!({"kind": "algorithm"}),
            json!({"kind": "bib_entry"}),
        ];
        for statement in [
            "theorem",
            "lemma",
            "proposition",
            "corollary",
            "definition",
            "remark",
        ] {
            kinds.push(json!({"kind": "statement", "statement_kind": statement}));
        }
        for kind in kinds {
            let mut expected = serde_json::to_value(&template.objects[0]).unwrap();
            expected
                .as_object_mut()
                .unwrap()
                .extend(kind.as_object().unwrap().clone());
            let object: PaperObject = serde_json::from_value(expected.clone()).unwrap();
            let encoded = serde_json::to_string(&object).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), expected);
            let decoded: PaperObject = serde_json::from_str(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
        }
        assert!(serde_json::from_value::<PaperObjectKind>(json!({"kind": "statement"})).is_err());
        assert!(serde_json::from_value::<PaperObjectKind>(json!({"kind": "invented"})).is_err());
    }

    #[tokio::test]
    async fn should_invalidate_objects_when_reading_index_generation_changes() {
        let directory = tempfile::tempdir().unwrap();
        let id = paper_id();
        let mut document = document();
        let first = load_or_build(directory.path(), &id, &document)
            .await
            .unwrap();
        document.etag = "\"generation-2\"".into();
        document.index.text = document.index.text.replace("caption.", "revised.");
        document
            .index
            .tokens
            .iter_mut()
            .find(|t| t.start == 15)
            .unwrap()
            .text = "revised.".into();
        let second = load_or_build(directory.path(), &id, &document)
            .await
            .unwrap();
        assert_ne!(
            first.reading_index_generation,
            second.reading_index_generation
        );
        assert_eq!(first.objects[0].id, second.objects[0].id);
        assert_eq!(second.objects[0].text, "Figure 3. A revised.");
        let persisted: ObjectsArtifact = serde_json::from_slice(
            &tokio::fs::read(directory.path().join(OBJECTS_FILE))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            persisted.reading_index_generation,
            second.reading_index_generation
        );
    }

    #[tokio::test]
    async fn should_refresh_legacy_detector_cache_when_native_generation_is_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let id = paper_id();
        let mut document = document();
        document.index.figures[0].caption = "Stale embedded prediction".into();
        let native = serde_json::to_vec(&document.index).unwrap();
        let mut old =
            serde_json::to_value(ObjectsArtifact::from_reading_index(&id, &document)).unwrap();
        old.as_object_mut()
            .unwrap()
            .remove("figure_detector_version");
        old.as_object_mut()
            .unwrap()
            .remove("figure_detector_generation");
        old["objects"][0]["text"] = json!("Stale derived object");
        tokio::fs::write(
            directory.path().join(OBJECTS_FILE),
            serde_json::to_vec(&old).unwrap(),
        )
        .await
        .unwrap();
        let rebuilt = load_or_build(directory.path(), &id, &document)
            .await
            .unwrap();
        assert_eq!(rebuilt.objects[0].text, "Figure 3. A caption.");
        assert_eq!(rebuilt.reading_index_generation, document.etag);
        assert_eq!(rebuilt.figure_detector_version, FIGURE_DETECTOR_VERSION);
        assert_eq!(
            rebuilt.figure_detector_generation,
            figure_detector_generation(&document.etag)
        );
        assert_eq!(serde_json::to_vec(&document.index).unwrap(), native);
        assert!(!directory.path().join("reading-index.json").exists());
    }

    #[tokio::test]
    async fn should_reuse_cached_objects_when_generation_and_schema_match() {
        let directory = tempfile::tempdir().unwrap();
        let id = paper_id();
        let document = document();
        let artifact = load_or_build(directory.path(), &id, &document)
            .await
            .unwrap();
        let mut saved = serde_json::to_value(&artifact).unwrap();
        saved["cache_fixture_marker"] = json!(true);
        let bytes = serde_json::to_vec_pretty(&saved).unwrap();
        let path = directory.path().join(OBJECTS_FILE);
        tokio::fs::write(&path, &bytes).await.unwrap();
        let reused = load_or_build(directory.path(), &id, &document)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(reused).unwrap(),
            serde_json::to_value(artifact).unwrap()
        );
        assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    }

    #[tokio::test]
    async fn should_rebuild_objects_when_cache_is_corrupt_stale_or_for_another_paper() {
        let directory = tempfile::tempdir().unwrap();
        let id = paper_id();
        let document = document();
        let artifact = ObjectsArtifact::from_reading_index(&id, &document);
        let mut old_schema = serde_json::to_value(&artifact).unwrap();
        old_schema["schema_version"] = json!(0);
        let mut other_paper = serde_json::to_value(&artifact).unwrap();
        other_paper["paper_id"] = json!("0000000000000000");
        for bytes in [
            b"{broken".to_vec(),
            serde_json::to_vec(&old_schema).unwrap(),
            serde_json::to_vec(&other_paper).unwrap(),
        ] {
            tokio::fs::write(directory.path().join(OBJECTS_FILE), bytes)
                .await
                .unwrap();
            let rebuilt = load_or_build(directory.path(), &id, &document)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(rebuilt).unwrap(),
                serde_json::to_value(&artifact).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn should_preserve_enrichment_when_deterministic_objects_are_rebuilt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(ENRICHMENT_FILE);
        let enrichment = b"separate enrichment fixture";
        tokio::fs::write(&path, enrichment).await.unwrap();
        load_or_build(directory.path(), &paper_id(), &document())
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), enrichment);
    }
}
