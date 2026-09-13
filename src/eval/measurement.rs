//! Evidence interchange for suite collectors. Only local, content-addressed inputs are read.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceFile {
    pub path: String,
    pub sha256: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Sample {
    Value {
        value: f64,
    },
    Ratio {
        numerator: f64,
        denominator: f64,
    },
    F1 {
        true_positive: f64,
        false_positive: f64,
        false_negative: f64,
    },
    Mean {
        values: Vec<f64>,
    },
    Median {
        values: Vec<f64>,
    },
    P95 {
        values: Vec<f64>,
    },
}

impl Sample {
    pub fn calculate(&self) -> Option<f64> {
        let value = match self {
            Self::Value { value } => *value,
            Self::Ratio {
                numerator,
                denominator,
            } => {
                if !numerator.is_finite()
                    || !denominator.is_finite()
                    || *numerator < 0.0
                    || *denominator <= 0.0
                {
                    return None;
                }
                numerator / denominator
            }
            Self::F1 {
                true_positive,
                false_positive,
                false_negative,
            } => {
                if [true_positive, false_positive, false_negative]
                    .iter()
                    .any(|n| !n.is_finite() || **n < 0.0)
                {
                    return None;
                }
                let denominator = 2.0 * true_positive + false_positive + false_negative;
                if denominator <= 0.0 {
                    return None;
                }
                2.0 * true_positive / denominator
            }
            Self::Mean { values } | Self::Median { values } | Self::P95 { values } => {
                if values.is_empty() || values.iter().any(|n| !n.is_finite() || *n < 0.0) {
                    return None;
                }
                match self {
                    Self::Mean { .. } => values.iter().sum::<f64>() / count(values.len()),
                    Self::Median { .. } => {
                        let mut sorted = values.clone();
                        sorted.sort_by(f64::total_cmp);
                        f64::midpoint(sorted[(sorted.len() - 1) / 2], sorted[sorted.len() / 2])
                    }
                    _ => {
                        let mut sorted = values.clone();
                        sorted.sort_by(f64::total_cmp);
                        sorted[(95 * sorted.len()).div_ceil(100) - 1]
                    }
                }
            }
        };
        value.is_finite().then_some(value)
    }
}

#[allow(clippy::cast_precision_loss)] // Measurement sample counts fit f64's integer precision.
const fn count(value: usize) -> f64 {
    value as f64
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub sample: Sample,
    /// Actual evaluated case count; empty truth sets cannot establish zero-error gates.
    pub cases: u64,
    pub evidence: Vec<EvidenceFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub schema_version: u32,
    pub suite: String,
    pub collector: String,
    pub implementation: Vec<EvidenceFile>,
    pub truth_sets: BTreeMap<String, EvidenceFile>,
    pub metrics: BTreeMap<String, Observation>,
    /// None means not measured, not a zero-cost model run.
    pub cost_usd: Option<f64>,
    pub wall_seconds: f64,
}

pub fn safe_file(root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    let path = Path::new(relative);
    if !path.components().all(|part| matches!(part, Component::Normal(name) if name != ".secrets" && name != ".env" && !name.to_string_lossy().starts_with(".env."))) {
        return Err(Error::InvalidRequest(format!("evidence path must be a safe repository-relative file: {relative}")));
    }
    let canonical = root
        .join(path)
        .canonicalize()
        .map_err(|error| Error::io(root.join(path), error))?;
    if !canonical.starts_with(root) {
        return Err(Error::InvalidRequest(format!(
            "evidence escapes repository: {relative}"
        )));
    }
    // Recheck the canonical relative path to reject symlinks into forbidden names.
    let relative = canonical
        .strip_prefix(root)
        .map_err(|error| Error::InvalidRequest(error.to_string()))?;
    if relative.components().any(|part| matches!(part,Component::Normal(name) if name == ".secrets" || name == ".env" || name.to_string_lossy().starts_with(".env."))) {
        return Err(Error::InvalidRequest("evidence resolves to a forbidden file".into()));
    }
    Ok(canonical)
}

pub fn digest(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|error| Error::io(path, error))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn verify(root: &Path, evidence: &EvidenceFile) -> Result<bool> {
    if evidence.version.trim().is_empty() {
        return Ok(false);
    }
    let path = match safe_file(root, &evidence.path) {
        Ok(path) => path,
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    Ok(digest(&path)? == evidence.sha256)
}
