//! The evaluation set and how far the owner has got through it.

use serde::{Deserialize, Serialize};

use super::{load_analysis_json, load_review, snapshot_from_analysis, storage::load_paper_reviews};
use crate::{
    Result,
    domain::{PaperId, PaperOverview},
    store::ArtifactStore,
};

/// The evaluation set, compiled in so progress works without the repository.
pub const MANIFEST_JSON: &str = include_str!("../../eval/review/papers.json");

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub papers: Vec<ManifestPaper>,
}

/// The parts of a manifest entry the server needs; the panel reads the rest
/// (neighbours, identifiers) from the manifest JSON itself.
#[derive(Clone, Debug, Deserialize)]
pub struct ManifestPaper {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub year: Option<u16>,
    pub filename: String,
}

impl Manifest {
    pub fn bundled() -> Result<Self> {
        Ok(serde_json::from_str(MANIFEST_JSON)?)
    }
}

/// One evaluation-set paper and where it stands.
#[derive(Clone, Debug, Serialize)]
pub struct ProgressRow {
    pub manifest_id: String,
    pub title: String,
    pub field: String,
    pub year: Option<u16>,
    pub filename: String,
    /// The library paper that matched, if any.
    pub paper_id: Option<PaperId>,
    pub analysed: bool,
    /// A review exists for the paper's current output.
    pub reviewed: bool,
    /// Reviews exist, but only for earlier output.
    pub reviewed_earlier: bool,
}

/// The manifest entry a library paper corresponds to: same file name, else
/// the same title once case, punctuation and spacing are ignored.
#[must_use]
pub fn manifest_match<'a>(
    manifest: &'a Manifest,
    paper: &PaperOverview,
) -> Option<&'a ManifestPaper> {
    let file_name = paper
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    manifest
        .papers
        .iter()
        .find(|entry| entry.filename.to_lowercase() == file_name)
        .or_else(|| {
            let title = normalize_title(&paper.metadata.title);
            (!title.is_empty()).then_some(())?;
            manifest
                .papers
                .iter()
                .find(|entry| normalize_title(&entry.title) == title)
        })
}

/// Progress across the evaluation set for the papers now in the library.
pub async fn progress(
    store: &ArtifactStore,
    manifest: &Manifest,
    papers: &[PaperOverview],
) -> Result<Vec<ProgressRow>> {
    let mut rows = Vec::with_capacity(manifest.papers.len());
    for entry in &manifest.papers {
        let paper = papers.iter().find(|paper| {
            manifest_match(manifest, paper).is_some_and(|found| found.id == entry.id)
        });
        let mut row = ProgressRow {
            manifest_id: entry.id.clone(),
            title: entry.title.clone(),
            field: entry.field.clone(),
            year: entry.year,
            filename: entry.filename.clone(),
            paper_id: paper.map(|paper| paper.id.clone()),
            analysed: false,
            reviewed: false,
            reviewed_earlier: false,
        };
        if let Some(paper) = paper {
            if let Some(analysis) = load_analysis_json(store, &paper.id).await? {
                row.analysed = true;
                let snapshot = snapshot_from_analysis(&analysis);
                row.reviewed = load_review(store, &paper.id, &snapshot.snapshot_id)
                    .await?
                    .is_some();
            }
            row.reviewed_earlier =
                !row.reviewed && !load_paper_reviews(store, &paper.id).await?.is_empty();
        }
        rows.push(row);
    }
    Ok(rows)
}

fn normalize_title(title: &str) -> String {
    title
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_lowercase().next().unwrap_or(character)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
