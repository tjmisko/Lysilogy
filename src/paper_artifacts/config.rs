//! The experiment configuration: which calls to make, over what input, with
//! which model and effort, and how to verify. A run is data, so experiments
//! are config files rather than code edits.
//!
//! ```json
//! {
//!   "input": {"format": "raw", "images": false, "priors": true},
//!   "calls": [
//!     {"name": "identity", "fields": ["identity"], "effort": "low", "model": "cheap", "scope": "front"},
//!     {"name": "core", "fields": ["sections", "references", "citations", "relations"]}
//!   ],
//!   "verify": {"self_check": "full", "repair": "model"},
//!   "models": {"cheap": "deepseek/deepseek-v4.1-flash", "strong": "anthropic/claude-opus-4"}
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::{
    Error, Result,
    analysis::openrouter::{DEFAULT_MODEL, ReasoningEffort},
};

/// The whole experiment.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    #[serde(default)]
    pub input: Input,
    pub calls: Vec<Call>,
    #[serde(default)]
    pub verify: Verify,
    #[serde(default)]
    pub models: Models,
}

/// How the paper is rendered for the model.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub format: InputFormat,
    /// Include a page image after each page's text.
    #[serde(default)]
    pub images: bool,
    /// Prepend the deterministic front matter (metadata) when it is present.
    #[serde(default)]
    pub priors: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            format: InputFormat::Raw,
            images: false,
            priors: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InputFormat {
    #[default]
    Raw,
    Markdown,
}

/// One model call and the fields it fills.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    /// A label for the record, e.g. `core` or `sections`.
    pub name: String,
    /// The field units to ask for, e.g. `["identity"]` or `["references", "citations"]`.
    pub fields: Vec<String>,
    #[serde(default)]
    pub effort: Effort,
    /// A tier name in `models`, or a literal model id.
    #[serde(default)]
    pub model: Option<String>,
    /// The most output tokens for this call; defaults by field kind.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Send the current draft of this call's fields with the paper.
    #[serde(default)]
    pub include_draft: bool,
    /// `all`, `front`, or `firstN`, e.g. `first3`.
    #[serde(default)]
    pub scope: Option<String>,
    /// Names of calls that must finish before this one starts.
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Off,
    #[default]
    Low,
    Medium,
    High,
}

impl Effort {
    #[must_use]
    pub const fn reasoning(self) -> ReasoningEffort {
        match self {
            Self::Off => ReasoningEffort::Off,
            Self::Low => ReasoningEffort::Low,
            Self::Medium => ReasoningEffort::Medium,
            Self::High => ReasoningEffort::High,
        }
    }
}

/// How the run checks its own work.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verify {
    #[serde(default)]
    pub self_check: SelfCheck,
    #[serde(default)]
    pub repair: Repair,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SelfCheck {
    None,
    #[default]
    Full,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Repair {
    None,
    Local,
    #[default]
    Model,
}

/// Named model ids the calls can refer to.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    #[serde(default)]
    pub cheap: Option<String>,
    #[serde(default)]
    pub strong: Option<String>,
}

impl RunConfig {
    /// Parse and validate a configuration.
    ///
    /// # Errors
    /// Fails when the JSON is malformed, a call has no fields, or a field name
    /// is not one a call can fill.
    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = serde_json::from_str(text)
            .map_err(|error| Error::InvalidRequest(format!("invalid run config: {error}")))?;
        let mut names = BTreeSet::new();
        for call in &config.calls {
            if !names.insert(call.name.as_str()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate call name '{}'",
                    call.name
                )));
            }
        }
        for call in &config.calls {
            if call.fields.is_empty() {
                return Err(Error::InvalidRequest(format!(
                    "call '{}' has no fields",
                    call.name
                )));
            }
            for field in &call.fields {
                if !super::run::is_unit(field) {
                    return Err(Error::InvalidRequest(format!(
                        "call '{}': unknown field '{field}'",
                        call.name
                    )));
                }
            }
            for require in &call.requires {
                if require == &call.name {
                    return Err(Error::InvalidRequest(format!(
                        "call '{}' requires itself",
                        call.name
                    )));
                }
                if !names.contains(require.as_str()) {
                    return Err(Error::InvalidRequest(format!(
                        "call '{}' requires unknown call '{require}'",
                        call.name
                    )));
                }
            }
        }
        if config.has_cycle() {
            return Err(Error::InvalidRequest(
                "the call graph has a cycle".to_owned(),
            ));
        }
        Ok(config)
    }

    /// The direct dependencies of each call, including the producers of the
    /// fields an `include_draft` call re-reads.
    #[must_use]
    pub fn dependencies(&self) -> BTreeMap<&str, Vec<&str>> {
        let mut dependencies = BTreeMap::new();
        for call in &self.calls {
            let mut deps: Vec<&str> = call.requires.iter().map(String::as_str).collect();
            if call.include_draft {
                for other in &self.calls {
                    if other.name != call.name
                        && call.fields.iter().any(|field| other.fields.contains(field))
                    {
                        deps.push(other.name.as_str());
                    }
                }
            }
            deps.sort_unstable();
            deps.dedup();
            dependencies.insert(call.name.as_str(), deps);
        }
        dependencies
    }

    fn has_cycle(&self) -> bool {
        let dependencies = self.dependencies();
        let mut indegree: BTreeMap<&str, usize> = self
            .calls
            .iter()
            .map(|call| (call.name.as_str(), 0))
            .collect();
        let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, deps) in &dependencies {
            for dep in deps {
                *indegree.entry(name).or_default() += 1;
                edges.entry(dep).or_default().push(name);
            }
        }
        let mut ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(name, _)| *name)
            .collect();
        let mut seen = 0;
        while let Some(name) = ready.pop() {
            seen += 1;
            for next in edges.get(name).into_iter().flatten() {
                if let Some(degree) = indegree.get_mut(next) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.push(next);
                    }
                }
            }
        }
        seen != self.calls.len()
    }

    /// The model id for one call, resolving tier names.
    #[must_use]
    pub fn model_for(&self, call: &Call) -> String {
        match call.model.as_deref() {
            Some("strong") => self
                .models
                .strong
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            Some("cheap") | None => self
                .models
                .cheap
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            Some(literal) => literal.to_owned(),
        }
    }
}

/// The number of leading pages a scope keeps; `None` keeps the whole paper.
#[must_use]
pub fn scope_pages(scope: Option<&str>) -> Option<usize> {
    match scope {
        None | Some("all") => None,
        Some("front") => Some(1),
        Some(name) => name
            .strip_prefix("first")
            .and_then(|count| count.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
        "input": {"format": "markdown", "priors": true},
        "calls": [
            {"name": "identity", "fields": ["identity"], "scope": "front", "effort": "off"},
            {"name": "graph", "fields": ["references", "citations"], "model": "strong"}
        ],
        "verify": {"self_check": "none", "repair": "local"},
        "models": {"cheap": "cheap/model", "strong": "strong/model"}
    }"#;

    #[test]
    fn should_read_a_full_config_and_resolve_tiers() {
        let config = RunConfig::parse(CONFIG).expect("config");
        assert_eq!(config.calls.len(), 2);
        assert_eq!(config.calls[0].effort, Effort::Off);
        assert_eq!(config.model_for(&config.calls[0]), "cheap/model");
        assert_eq!(config.model_for(&config.calls[1]), "strong/model");
        assert_eq!(config.verify.self_check, SelfCheck::None);
        assert_eq!(config.verify.repair, Repair::Local);
        assert_eq!(config.input.format, InputFormat::Markdown);
    }

    #[test]
    fn should_default_to_raw_low_effort_and_full_verification() {
        let config = RunConfig::parse(r#"{"calls":[{"name":"core","fields":["sections"]}]}"#)
            .expect("config");
        assert_eq!(config.input.format, InputFormat::Raw);
        assert_eq!(config.calls[0].effort, Effort::Low);
        assert_eq!(config.verify.self_check, SelfCheck::Full);
        assert_eq!(config.verify.repair, Repair::Model);
        assert_eq!(config.model_for(&config.calls[0]), DEFAULT_MODEL);
    }

    #[test]
    fn should_reject_an_unknown_field() {
        assert!(RunConfig::parse(r#"{"calls":[{"name":"x","fields":["vibes"]}]}"#).is_err());
        assert!(RunConfig::parse(r#"{"calls":[]}"#).is_ok());
        assert!(RunConfig::parse(r#"{"calls":[{"name":"x","fields":[]}]}"#).is_err());
    }

    #[test]
    fn should_map_scope_names_to_page_limits() {
        assert_eq!(scope_pages(None), None);
        assert_eq!(scope_pages(Some("all")), None);
        assert_eq!(scope_pages(Some("front")), Some(1));
        assert_eq!(scope_pages(Some("first3")), Some(3));
    }

    #[test]
    fn should_order_explicit_and_include_draft_dependencies() {
        let config = RunConfig::parse(
            r#"{"calls":[
                {"name":"core","fields":["sections","card"]},
                {"name":"refine","fields":["sections"],"include_draft":true},
                {"name":"glossary","fields":["glossary"],"requires":["core"]}
            ]}"#,
        )
        .expect("config");
        let dependencies = config.dependencies();
        assert!(dependencies["core"].is_empty());
        assert_eq!(dependencies["refine"], vec!["core"]);
        assert_eq!(dependencies["glossary"], vec!["core"]);
    }

    #[test]
    fn should_reject_unknown_self_and_cyclic_requires() {
        assert!(
            RunConfig::parse(
                r#"{"calls":[{"name":"a","fields":["sections"],"requires":["missing"]}]}"#
            )
            .is_err()
        );
        assert!(
            RunConfig::parse(
                r#"{"calls":[{"name":"a","fields":["sections"],"requires":["a"]}]}"#
            )
            .is_err()
        );
        assert!(
            RunConfig::parse(
                r#"{"calls":[
                    {"name":"a","fields":["sections"],"requires":["b"]},
                    {"name":"b","fields":["card"],"requires":["a"]}
                ]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn should_reject_a_duplicate_call_name() {
        assert!(
            RunConfig::parse(
                r#"{"calls":[
                    {"name":"a","fields":["sections"]},
                    {"name":"a","fields":["card"]}
                ]}"#
            )
            .is_err()
        );
    }
}
