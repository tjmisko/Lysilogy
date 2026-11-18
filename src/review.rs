//! Human review of reader output.
//!
//! The owner grades what the Abstract and Overview views show for a paper
//! against `eval/review/rubric.json`. Each review is a plain JSON file stored
//! beside the paper's other artifacts, one per (paper, reviewed snapshot). A
//! review carries a copy of the fields it graded, so its grades stay attached
//! to that exact output after the paper is analysed again, and aggregates can
//! be grouped by the pipeline version that produced it.
//!
//! The analysis is read as untyped JSON on purpose: the reader model is still
//! gaining fields (outline, logical sections, read-alongside lists, pipeline
//! version), and a review must keep working whatever shape it finds.

mod progress;
mod rubric;
mod snapshot;
mod storage;
mod summary;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use self::{
    progress::{MANIFEST_JSON, Manifest, ManifestPaper, ProgressRow, manifest_match, progress},
    rubric::{DimensionKind, GENERAL_COMPONENT, RUBRIC_JSON, Rubric, RubricComponent},
    snapshot::{ReviewSnapshot, snapshot_from_analysis},
    storage::{load_all_reviews, load_analysis_json, load_paper_reviews, load_review, save_review},
    summary::{
        ComponentSummary, DimensionSummary, ExportedIssue, GroupSummary, ReviewExport, Summary,
        UNVERSIONED, export, render_summary, summarize,
    },
};
use crate::{Error, Result, domain::PaperId, store::ArtifactStore};

/// Bumped when a stored review's shape changes incompatibly.
pub const REVIEW_SCHEMA_VERSION: u16 = 1;

/// One recorded grade. The rubric dimension's kind decides which variant is
/// valid: scores and counts are numbers, pass/fail is a flag, a choice is the
/// option id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GradeValue {
    Flag(bool),
    Number(u32),
    Choice(String),
}

/// The grades for one rubric component.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentGrade {
    /// The reader showed nothing for this component. That is gradable: the
    /// dimensions may still be scored for what the absence costs the reader.
    #[serde(default)]
    pub not_produced: bool,
    /// The component's hard reject applies (for example a false claim in the
    /// context), whatever its dimension scores.
    #[serde(default)]
    pub hard_reject: bool,
    #[serde(default)]
    pub values: BTreeMap<String, GradeValue>,
    #[serde(default)]
    pub notes: String,
}

impl ComponentGrade {
    /// Whether the reviewer has recorded anything for this component.
    #[must_use]
    pub fn is_graded(&self) -> bool {
        self.not_produced || self.hard_reject || !self.values.is_empty()
    }
}

/// A concrete problem the reviewer wants a prompt change to fix. Flags are
/// exported as regression cases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueFlag {
    /// Client-chosen, stable within a review so edits replace the same flag.
    pub id: String,
    /// A rubric component id, or `general`.
    pub component: String,
    pub severity: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub note: String,
    /// The offending text, copied from the reviewed output.
    #[serde(default)]
    pub quote: Option<String>,
}

/// A stored review of one paper's output at one snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub schema_version: u16,
    pub rubric_id: String,
    pub paper_id: PaperId,
    pub paper_title: String,
    /// The evaluation-set entry this paper matched, if any.
    #[serde(default)]
    pub manifest_id: Option<String>,
    pub snapshot: ReviewSnapshot,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub components: BTreeMap<String, ComponentGrade>,
    #[serde(default)]
    pub overall: Option<u32>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub issues: Vec<IssueFlag>,
}

/// What the review panel sends on each autosave.
///
/// `snapshot_id` names the output the reviewer was looking at; a save against
/// output that has since been replaced is refused rather than attached to the
/// wrong text.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReviewInput {
    pub snapshot_id: String,
    #[serde(default)]
    pub components: BTreeMap<String, ComponentGrade>,
    #[serde(default)]
    pub overall: Option<u32>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub issues: Vec<IssueFlag>,
}

/// Identifies the paper a review belongs to.
#[derive(Clone, Debug)]
pub struct ReviewSubject {
    pub paper_id: PaperId,
    pub paper_title: String,
    pub manifest_id: Option<String>,
}

impl Review {
    /// Builds the stored review from a validated input, keeping the creation
    /// time of an earlier save of the same snapshot.
    #[must_use]
    pub fn from_input(
        rubric: &Rubric,
        subject: ReviewSubject,
        snapshot: ReviewSnapshot,
        input: ReviewInput,
        previous: Option<&Self>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut components = input.components;
        components.retain(|_, grade| grade.is_graded() || !grade.notes.trim().is_empty());
        Self {
            schema_version: REVIEW_SCHEMA_VERSION,
            rubric_id: rubric.id.clone(),
            paper_id: subject.paper_id,
            paper_title: subject.paper_title,
            manifest_id: subject.manifest_id,
            snapshot,
            created_at: previous.map_or(now, |review| review.created_at),
            updated_at: now,
            components,
            overall: input.overall,
            notes: input.notes.trim().to_owned(),
            issues: input
                .issues
                .into_iter()
                .map(|issue| IssueFlag {
                    note: issue.note.trim().to_owned(),
                    quote: issue
                        .quote
                        .map(|quote| quote.trim().to_owned())
                        .filter(|quote| !quote.is_empty()),
                    ..issue
                })
                .collect(),
        }
    }

    /// The pipeline version that produced the reviewed output, or
    /// [`UNVERSIONED`] for analyses written before versions were recorded.
    #[must_use]
    pub fn pipeline_version(&self) -> &str {
        self.snapshot
            .pipeline_version
            .as_deref()
            .unwrap_or(UNVERSIONED)
    }
}

/// `lysilogy review`: the summary as a table (or JSON), or, with an export
/// path, every review, the summary and the flagged issues written as one JSON
/// file. Returns the text to print.
pub async fn run_cli(
    store: &ArtifactStore,
    export_path: Option<&std::path::Path>,
    json: bool,
) -> Result<String> {
    let rubric = Rubric::bundled()?;
    let reviews = load_all_reviews(store).await?;
    if let Some(path) = export_path {
        let count = reviews.len();
        let exported = export(
            &rubric,
            serde_json::from_str(RUBRIC_JSON)?,
            reviews,
            Utc::now(),
        );
        let issues = exported.issues.len();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| Error::io(parent, error))?;
        }
        let mut bytes = serde_json::to_vec_pretty(&exported)?;
        bytes.push(b'\n');
        tokio::fs::write(path, bytes)
            .await
            .map_err(|error| Error::io(path, error))?;
        return Ok(format!(
            "Exported {count} reviews and {issues} flagged issues to {}\n",
            path.display()
        ));
    }
    let summary = summarize(&rubric, &reviews);
    if json {
        let mut text = serde_json::to_string_pretty(&summary)?;
        text.push('\n');
        return Ok(text);
    }
    Ok(render_summary(&rubric, &summary))
}
