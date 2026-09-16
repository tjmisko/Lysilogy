//! Human verdicts on detector objects. See `eval/graded-objects-contract.md`.
//!
//! A grades file is one grader's plain-JSON judgement of `objects.json` for one
//! paper. It is reviewed data, not derived state: nothing here recomputes it.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result, domain::TextRect, store::write_atomic};

pub const GRADES_FILE: &str = "objects-grades.json";
pub const QUEUE_FILE: &str = "grading-queue.json";
pub const SCHEMA_VERSION: u16 = 1;
pub const MAX_GRADES_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 2000;
const MAX_NOTE_CHARS: usize = 2000;
const MAX_LABEL_CHARS: usize = 32;
const MAX_ID_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Correct,
    Region,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerdictRecord {
    pub verdict: Verdict,
    #[serde(default)]
    pub region: Option<TextRect>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdditionKind {
    Figure,
    Table,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaptionSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Addition {
    pub id: String,
    pub kind: AdditionKind,
    pub printed_label: String,
    pub page: u32,
    pub region: TextRect,
    #[serde(default)]
    pub caption: Option<CaptionSpan>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectGrades {
    pub schema_version: u16,
    pub paper_id: String,
    /// SHA-256 of the paper's `reading-index.json` bytes (the index etag without quotes).
    pub index_sha256: String,
    #[serde(default)]
    pub objects_generation: String,
    #[serde(default)]
    pub grader: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    /// SHA-256 of the stored bytes; `None` until first saved.
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub verdicts: BTreeMap<String, VerdictRecord>,
    #[serde(default)]
    pub additions: Vec<Addition>,
}

/// Status of one paper in the grading queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradingStatus {
    Complete,
    Partial,
    Ungraded,
}

/// `<data>/grading-queue.json`, written by `scripts/eval/graded-objects.py queue`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GradingQueue {
    pub schema_version: u16,
    #[serde(default)]
    pub papers: Vec<QueuedPaper>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedPaper {
    pub paper_id: String,
    #[serde(default)]
    pub arxiv_id: Option<String>,
    #[serde(default)]
    pub stratum: Option<String>,
}

fn finite_rect(rect: &TextRect) -> bool {
    [rect.x_min, rect.y_min, rect.x_max, rect.y_max]
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
        && rect.x_min < rect.x_max
        && rect.y_min < rect.y_max
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidRequest(message.into())
}

impl ObjectGrades {
    /// Reject structurally invalid grades before they reach disk.
    ///
    /// Coverage rules (every object graded, captions present) belong to the
    /// export step, which also knows the detector output; the server only
    /// guarantees that what it stores is well formed.
    pub fn validate(&self, paper_id: &str) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid("unsupported grades schema version"));
        }
        if self.paper_id != paper_id {
            return Err(invalid("grades paper id does not match the request"));
        }
        if self.index_sha256.len() != 64
            || !self
                .index_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid("index_sha256 must be a 64-character hex digest"));
        }
        if self.verdicts.len() > MAX_ENTRIES || self.additions.len() > MAX_ENTRIES {
            return Err(invalid("too many verdicts or additions"));
        }
        if self.grader.chars().count() > MAX_NOTE_CHARS {
            return Err(invalid("grader is too long"));
        }
        for (id, record) in &self.verdicts {
            if id.is_empty() || id.chars().count() > MAX_ID_CHARS {
                return Err(invalid("verdict object id must be 1–64 characters"));
            }
            if record.note.chars().count() > MAX_NOTE_CHARS {
                return Err(invalid(format!("note for {id} is too long")));
            }
            match (record.verdict, record.region.as_ref()) {
                (Verdict::Region, None) => {
                    return Err(invalid(format!("verdict region for {id} needs a region")));
                }
                (_, Some(rect)) if !finite_rect(rect) => {
                    return Err(invalid(format!("region for {id} is not a finite box")));
                }
                _ => {}
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for addition in &self.additions {
            if addition.id.is_empty() || addition.id.chars().count() > MAX_ID_CHARS {
                return Err(invalid("addition id must be 1–64 characters"));
            }
            if !ids.insert(addition.id.as_str()) {
                return Err(invalid(format!("duplicate addition id {}", addition.id)));
            }
            let label = addition.printed_label.trim();
            if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
                return Err(invalid(format!(
                    "printed_label for {} must be 1–32 characters",
                    addition.id
                )));
            }
            if addition.page == 0 {
                return Err(invalid(format!(
                    "page for {} must be positive",
                    addition.id
                )));
            }
            if !finite_rect(&addition.region) {
                return Err(invalid(format!(
                    "region for {} is not a finite box",
                    addition.id
                )));
            }
            if addition
                .caption
                .is_some_and(|caption| caption.start >= caption.end)
            {
                return Err(invalid(format!("caption for {} is empty", addition.id)));
            }
            if addition.note.chars().count() > MAX_NOTE_CHARS {
                return Err(invalid(format!("note for {} is too long", addition.id)));
            }
        }
        Ok(())
    }
}

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Read a paper's grades; `None` when the paper has never been graded.
pub async fn load(directory: &Path) -> Result<Option<ObjectGrades>> {
    let path = directory.join(GRADES_FILE);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(&path, error)),
    };
    let mut grades: ObjectGrades = serde_json::from_slice(&bytes)?;
    grades.revision = Some(revision(&bytes));
    Ok(Some(grades))
}

/// Outcome of a conditional save.
#[derive(Debug)]
pub enum SaveOutcome {
    Saved(ObjectGrades),
    /// The stored revision differs from the one the client loaded.
    Conflict(Option<ObjectGrades>),
}

/// Persist grades when `expected_revision` matches the stored file.
///
/// The server stamps `updated_at`, fills an empty `grader` from `$USER`, and
/// derives `revision` from the bytes it wrote so the client can save again.
pub async fn save(
    directory: &Path,
    mut grades: ObjectGrades,
    expected_revision: Option<&str>,
) -> Result<SaveOutcome> {
    let current = load(directory).await?;
    if current
        .as_ref()
        .and_then(|stored| stored.revision.as_deref())
        != expected_revision
    {
        return Ok(SaveOutcome::Conflict(current));
    }
    if grades.grader.trim().is_empty() {
        grades.grader = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    }
    grades.updated_at = Some(chrono::Local::now().fixed_offset().to_rfc3339());
    grades.revision = None;
    let mut bytes = serde_json::to_vec_pretty(&grades)?;
    bytes.push(b'\n');
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| Error::io(directory, error))?;
    write_atomic(&directory.join(GRADES_FILE), &bytes).await?;
    grades.revision = Some(revision(&bytes));
    Ok(SaveOutcome::Saved(grades))
}

/// Cheap status without deserializing verdict details.
pub async fn status(directory: &Path) -> Result<GradingStatus> {
    Ok(match load(directory).await? {
        None => GradingStatus::Ungraded,
        Some(grades) if grades.complete => GradingStatus::Complete,
        Some(_) => GradingStatus::Partial,
    })
}

/// Load the optional queue file; absence means catalog order.
pub async fn load_queue(data_root: &Path) -> Result<Option<GradingQueue>> {
    let path = data_root.join(QUEUE_FILE);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(&path, error)),
    };
    let queue: GradingQueue = serde_json::from_slice(&bytes)?;
    if queue.schema_version != 1 {
        return Err(invalid("unsupported grading queue schema version"));
    }
    Ok(Some(queue))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> TextRect {
        TextRect {
            x_min: 10.0,
            y_min: 20.0,
            x_max: 110.0,
            y_max: 220.0,
        }
    }

    fn grades() -> ObjectGrades {
        ObjectGrades {
            schema_version: SCHEMA_VERSION,
            paper_id: "0123456789abcdef".into(),
            index_sha256: "a".repeat(64),
            objects_generation: "gen".into(),
            grader: String::new(),
            updated_at: None,
            revision: None,
            complete: false,
            verdicts: BTreeMap::from([
                (
                    "fig-1".into(),
                    VerdictRecord {
                        verdict: Verdict::Correct,
                        region: None,
                        note: String::new(),
                    },
                ),
                (
                    "tab-1".into(),
                    VerdictRecord {
                        verdict: Verdict::Region,
                        region: Some(rect()),
                        note: "tightened".into(),
                    },
                ),
            ]),
            additions: vec![Addition {
                id: "add-1".into(),
                kind: AdditionKind::Figure,
                printed_label: "IV".into(),
                page: 3,
                region: rect(),
                caption: Some(CaptionSpan { start: 5, end: 40 }),
                note: String::new(),
            }],
        }
    }

    #[test]
    fn should_accept_well_formed_grades_when_validated() {
        grades().validate("0123456789abcdef").unwrap();
    }

    #[test]
    fn should_reject_grades_when_paper_id_region_or_caption_is_malformed() {
        assert!(grades().validate("fedcba9876543210").is_err());
        let mut missing_region = grades();
        missing_region.verdicts.get_mut("tab-1").unwrap().region = None;
        assert!(missing_region.validate("0123456789abcdef").is_err());
        let mut inverted = grades();
        inverted.additions[0].region.x_max = 0.0;
        assert!(inverted.validate("0123456789abcdef").is_err());
        let mut empty_caption = grades();
        empty_caption.additions[0].caption = Some(CaptionSpan { start: 9, end: 9 });
        assert!(empty_caption.validate("0123456789abcdef").is_err());
        let mut duplicate = grades();
        duplicate.additions.push(duplicate.additions[0].clone());
        assert!(duplicate.validate("0123456789abcdef").is_err());
        let mut bad_hash = grades();
        bad_hash.index_sha256 = "xyz".into();
        assert!(bad_hash.validate("0123456789abcdef").is_err());
    }

    #[tokio::test]
    async fn should_stamp_revision_and_detect_conflicts_when_saving() {
        let directory = tempfile::tempdir().unwrap();
        assert!(load(directory.path()).await.unwrap().is_none());
        assert_eq!(
            status(directory.path()).await.unwrap(),
            GradingStatus::Ungraded
        );
        let SaveOutcome::Saved(first) = save(directory.path(), grades(), None).await.unwrap()
        else {
            panic!("first save must succeed");
        };
        assert!(first.revision.is_some());
        assert!(first.updated_at.is_some());
        assert!(!first.grader.is_empty(), "server fills the grader");
        let reloaded = load(directory.path()).await.unwrap().unwrap();
        assert_eq!(reloaded.revision, first.revision);
        assert_eq!(
            status(directory.path()).await.unwrap(),
            GradingStatus::Partial
        );
        let stale = save(directory.path(), grades(), None).await.unwrap();
        assert!(matches!(stale, SaveOutcome::Conflict(Some(_))));
        let mut complete = grades();
        complete.complete = true;
        let SaveOutcome::Saved(second) =
            save(directory.path(), complete, first.revision.as_deref())
                .await
                .unwrap()
        else {
            panic!("save with the current revision must succeed");
        };
        assert_ne!(second.revision, first.revision);
        assert_eq!(
            status(directory.path()).await.unwrap(),
            GradingStatus::Complete
        );
    }

    #[test]
    fn should_round_trip_grades_json_when_serialized() {
        let json = serde_json::to_string(&grades()).unwrap();
        assert!(json.contains("\"verdict\":\"region\""));
        assert!(json.contains("\"kind\":\"figure\""));
        let parsed: ObjectGrades = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, grades());
    }
}
