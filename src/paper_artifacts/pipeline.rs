//! The identity of the pipeline that produced a record.
//!
//! A human grade is only useful for tuning if it can be tied to exactly what
//! produced the graded output. The pipeline version is readable at a glance
//! (model and provider) and ends in a digest of every prompt, schema and plan
//! setting the run used, so any edit to them gives a new version.

use sha2::{Digest as _, Sha256};

/// How many hex digits of the digest the version keeps.
const DIGEST_CHARS: usize = 12;

/// What a run was made of.
pub struct PipelineInputs<'a> {
    /// The model that answered the plan calls, or the plan harness label.
    pub model: &'a str,
    /// The stronger model used for judgment calls, when one is configured.
    pub judgment_model: Option<&'a str>,
    /// Providers in the order they were asked; empty means routed freely.
    pub providers: &'a [String],
    /// Every prompt, instruction, schema and plan setting text the run used,
    /// in a stable order.
    pub texts: &'a [String],
}

/// A version string such as `deepseek/deepseek-v4.1-flash@deepinfra#3f9a1c2b7d4e`.
#[must_use]
pub fn pipeline_version(inputs: &PipelineInputs<'_>) -> String {
    let mut hasher = Sha256::new();
    // Length-prefixed so adjacent texts cannot run into each other.
    let mut field = |tag: u8, bytes: &[u8]| {
        hasher.update([tag]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    field(b'm', inputs.model.as_bytes());
    if let Some(judge) = inputs.judgment_model {
        field(b'j', judge.as_bytes());
    }
    for provider in inputs.providers {
        field(b'p', provider.as_bytes());
    }
    for text in inputs.texts {
        field(b't', text.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    let models = inputs.judgment_model.map_or_else(
        || inputs.model.to_owned(),
        |judge| format!("{}+{judge}", inputs.model),
    );
    let providers = if inputs.providers.is_empty() {
        "auto".to_owned()
    } else {
        inputs.providers.join(",")
    };
    format!("{models}@{providers}#{}", &digest[..DIGEST_CHARS])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(model: &str, providers: &[&str], texts: &[&str]) -> String {
        let providers = providers
            .iter()
            .map(|p| (*p).to_owned())
            .collect::<Vec<_>>();
        let texts = texts.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
        pipeline_version(&PipelineInputs {
            model,
            judgment_model: None,
            providers: &providers,
            texts: &texts,
        })
    }

    #[test]
    fn should_name_the_model_and_provider_and_end_in_a_digest() {
        let version = version("m/x", &["deepinfra"], &["prompt"]);
        let (label, digest) = version.split_once('#').expect("digest");
        assert_eq!(label, "m/x@deepinfra");
        assert_eq!(digest.len(), DIGEST_CHARS);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn should_change_when_any_prompt_model_or_provider_changes() {
        let base = version("m/x", &["deepinfra"], &["prompt", "schema"]);
        assert_eq!(base, version("m/x", &["deepinfra"], &["prompt", "schema"]));
        assert_ne!(base, version("m/x", &["deepinfra"], &["prompt!", "schema"]));
        assert_ne!(base, version("m/x", &["deepinfra"], &["promptschema"]));
        assert_ne!(base, version("m/y", &["deepinfra"], &["prompt", "schema"]));
        assert_ne!(base, version("m/x", &[], &["prompt", "schema"]));
        assert!(version("m/x", &[], &[]).contains("@auto#"));
    }

    #[test]
    fn should_show_a_judgment_model_beside_the_main_model() {
        let version = pipeline_version(&PipelineInputs {
            model: "m/x",
            judgment_model: Some("m/strong"),
            providers: &[],
            texts: &[],
        });
        assert!(version.starts_with("m/x+m/strong@auto#"), "{version}");
    }
}
