//! The pinned copy of the output a review graded.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Top-level analysis keys copied into a snapshot, matched by prefix:
/// `abstract` covers `abstract_extraction` and `abstract_agreement`,
/// `context` covers the context node's `context` and the older
/// `context_notes`/`context_sources`, and `section` covers the logical units
/// (`sections`), whose `outline_entries` index into `outline`. The pipeline's
/// `quality_warnings` are pinned so a grade can be read next to them. The
/// glossary and claims are left out because the review does not grade them.
/// `component_versions` records which pipeline last refreshed each part.
const REVIEWED_PREFIXES: &[&str] = &[
    "thesis",
    "author_abstract",
    "abstract",
    "outsider_brief",
    "context",
    "outline",
    "section",
    "quality_warnings",
    "component_versions",
];

/// What a review graded: the reviewed fields as they were, plus the
/// identifiers that tell one analysis run from another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSnapshot {
    /// A content hash of the reviewed fields, `generated_at` and
    /// `pipeline_version`. Re-analysis that changes any of them yields a new
    /// snapshot and therefore a new, empty review.
    pub snapshot_id: String,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub pipeline_version: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

impl ReviewSnapshot {
    /// The snapshot's copy of a reviewed field, if the analysis had it.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Value> {
        self.fields.get(key).filter(|value| !value.is_null())
    }
}

/// Copies the reviewed fields out of an analysis read as untyped JSON.
#[must_use]
pub fn snapshot_from_analysis(analysis: &Value) -> ReviewSnapshot {
    let fields = analysis
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter(|(key, _)| is_reviewed_key(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Map<_, _>>()
        })
        .unwrap_or_default();
    let generated_at = scalar_text(analysis.get("generated_at"));
    let pipeline_version = scalar_text(analysis.get("pipeline_version")).or_else(|| {
        scalar_text(
            analysis
                .get("pipeline")
                .and_then(|pipeline| pipeline.get("version")),
        )
    });
    let provider = scalar_text(analysis.get("provider"));
    let snapshot_id = snapshot_id(
        &fields,
        generated_at.as_deref(),
        pipeline_version.as_deref(),
    );
    ReviewSnapshot {
        snapshot_id,
        generated_at,
        pipeline_version,
        provider,
        fields,
    }
}

fn is_reviewed_key(key: &str) -> bool {
    REVIEWED_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

fn scalar_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn snapshot_id(
    fields: &Map<String, Value>,
    generated_at: Option<&str>,
    pipeline_version: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(generated_at.unwrap_or_default().as_bytes());
    hasher.update([0]);
    hasher.update(pipeline_version.unwrap_or_default().as_bytes());
    hasher.update([0]);
    // Serialising a Value is deterministic for a given parsed document.
    hasher.update(Value::Object(fields.clone()).to_string().as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(8)
        .fold(String::from("s-"), |mut id, byte| {
            let _ = write!(id, "{byte:02x}");
            id
        })
}
