//! The pinned copy of the output a review graded.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::edge_labels::{
    EdgeLabelBrief, EdgeLabelRun, LabelledEdge, REVIEW_SAMPLE_SIZE, review_sample,
};
use lysilogy_db::relationship::LabelPath;
use lysilogy_ingest::labels::LabelSummary;

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

/// The snapshot field that pins the citation labels a review graded.
///
/// It is not part of the analysis and not hashed into the snapshot id: labels are
/// written after the analysis and change as the owner reviews them, so the
/// first save of a review pins the sample it graded and later saves keep it.
pub const EDGE_LABELS_FIELD: &str = "edge_labels";

/// Pin a sample of the citation labels into `snapshot`, unless one is
/// already pinned.
///
/// The sample is drawn from `current` (the paper's outgoing
/// edges with their labels as the graph holds them now) when given, else
/// from the run's edges; rejected labels are never sampled. The run gives
/// the counts and the versions. Without a run nothing is pinned and the
/// panel shows the component as not produced.
pub fn pin_edge_labels(
    snapshot: &mut ReviewSnapshot,
    run: Option<&EdgeLabelRun>,
    current: Option<&[LabelledEdge]>,
) {
    if snapshot.fields.contains_key(EDGE_LABELS_FIELD) {
        return;
    }
    let Some(run) = run else {
        return;
    };
    let edges = current.unwrap_or(&run.edges);
    let sample: Vec<serde_json::Value> = review_sample(edges, REVIEW_SAMPLE_SIZE)
        .into_iter()
        .map(|edge| {
            let version = edge
                .label
                .as_ref()
                .map(|label| item_label_version(label, run));
            let mut value = serde_json::to_value(&edge).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("label_version".to_owned(), serde_json::json!(version));
            }
            value
        })
        .collect();
    let pinned = serde_json::json!({
        "generated_at": run.generated_at,
        "status": run.status,
        "detail": run.detail,
        "labeler": run.labeler,
        "prompt_version": run.prompt_version,
        "brief_digest": run.brief_digest,
        "label_version": label_version(run.labeler.as_deref(), run.prompt_version, run.brief_digest.as_deref()),
        "from_graph": current.is_some(),
        "counts": run.counts,
        "unresolved_keys": run.core.as_ref().map(|core| core.unresolved.values().sum::<usize>()),
        "left_for_later": run.labels.as_ref().map(LabelSummary::left_unlabelled),
        "rejected_for_grounding": run.labels.as_ref().map(|labels| labels.rejected_for_grounding),
        "sample": sample,
    });
    snapshot.fields.insert(EDGE_LABELS_FIELD.to_owned(), pinned);
}

/// Which labels a pinned sample shows.
///
/// The run it came with and the claim ids in order. The review panel sends the pin it showed, and a first save
/// whose pin differs from the current one is refused as stale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeLabelsPin {
    pub generated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub claim_ids: Vec<String>,
}

impl EdgeLabelsPin {
    /// The pin of a pinned `edge_labels` field.
    #[must_use]
    pub fn of(pinned: &serde_json::Value) -> Self {
        Self {
            generated_at: pinned
                .get("generated_at")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            claim_ids: pinned
                .get("sample")
                .and_then(serde_json::Value::as_array)
                .map(|sample| {
                    sample
                        .iter()
                        .filter_map(|edge| edge.pointer("/label/claim_id")?.as_str())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// The key the edge labeler's grades are compared under: the labeler, its
/// prompt version and, when known, the digest of its brief and schema.
#[must_use]
pub fn label_version(labeler: Option<&str>, prompt_version: u32, brief_digest: Option<&str>) -> String {
    let digest = brief_digest.map(|digest| format!("+{digest}")).unwrap_or_default();
    format!("{}@v{prompt_version}{digest}", labeler.unwrap_or("no labeler"))
}

/// The key one sampled label's verdict is compared under.
///
/// By the path that wrote it: a core relation by the core model and pipeline version, an
/// edge-labeler label by its model and prompt version (with the brief
/// digest when it is the run's version), a person's label as `user`.
#[must_use]
pub fn item_label_version(label: &EdgeLabelBrief, run: &EdgeLabelRun) -> String {
    match label.path.unwrap_or(LabelPath::EdgeLabeler) {
        LabelPath::CoreRelation => format!(
            "core:{}@{}",
            label
                .model
                .as_deref()
                .or(run.core_model.as_deref())
                .unwrap_or("unknown model"),
            run.pipeline_version.as_deref().unwrap_or("unversioned")
        ),
        LabelPath::EdgeLabeler => {
            let version = label
                .prompt_version
                .and_then(|version| u32::try_from(version).ok())
                .unwrap_or(1);
            let digest = (version == run.prompt_version)
                .then_some(run.brief_digest.as_deref())
                .flatten();
            label_version(
                label.model.as_deref().or(run.labeler.as_deref()),
                version,
                digest,
            )
        }
        LabelPath::User => "user".to_owned(),
    }
}

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

/// `value` with every object's keys in sorted order.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical(&object[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
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
    // Keys are sorted before hashing, so the id depends on the content and
    // not on the order the analysis happened to be written in.
    hasher.update(canonical(&Value::Object(fields.clone())).to_string().as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(8)
        .fold(String::from("s-"), |mut id, byte| {
            let _ = write!(id, "{byte:02x}");
            id
        })
}
