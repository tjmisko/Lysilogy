//! Small, explicit application settings; paper-note contents remain ordinary Markdown.
use crate::{Error, Result, notes::NoteTemplateConfig};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub notes: NoteTemplateConfig,
    pub vim: VimConfig,
    pub analysis: AnalysisConfig,
}

/// How the model reader runs a paper through the core pipeline.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AnalysisConfig {
    /// The `OpenRouter` model that answers the core calls.
    pub model: String,
    /// A stronger model for judgment calls such as the self-check; `None`
    /// uses `model` for everything.
    pub judgment_model: Option<String>,
    /// `OpenRouter` providers to try, in order. The extraction experiments
    /// found unpinned routing flaky and about ten times the cost; `DeepInfra`
    /// was the stable choice. Empty lets `OpenRouter` choose.
    pub providers: Vec<String>,
    /// No further model call starts once one paper has spent this much.
    /// Generous enough that a normal paper always reaches its self-check.
    pub paper_budget_usd: f64,
    /// The model that labels why a paper cites each work, over `OpenRouter`;
    /// `None` uses `judgment_model`, then `model`. Labels are short judgment
    /// calls, so the stronger model costs little per edge.
    pub label_model: Option<String>,
    /// No further label call starts once one paper's labelling has spent
    /// this much; the rest are left for a later run, which resumes from the
    /// cache.
    pub label_budget_usd: f64,
    /// Most label calls for one paper in one run (cache hits are free).
    pub label_limit: usize,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            model: crate::analysis::openrouter::DEFAULT_MODEL.to_owned(),
            judgment_model: None,
            providers: vec!["deepinfra".to_owned()],
            paper_budget_usd: 0.50,
            label_model: None,
            label_budget_usd: 0.10,
            label_limit: 40,
        }
    }
}

impl AnalysisConfig {
    fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "analysis.model must not be empty".into(),
            ));
        }
        if self
            .judgment_model
            .as_deref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err(Error::InvalidRequest(
                "analysis.judgment_model must not be empty; omit it to use analysis.model".into(),
            ));
        }
        if self
            .providers
            .iter()
            .any(|provider| provider.trim().is_empty())
        {
            return Err(Error::InvalidRequest(
                "analysis.providers must not contain an empty name".into(),
            ));
        }
        if !self.paper_budget_usd.is_finite() || self.paper_budget_usd <= 0.0 {
            return Err(Error::InvalidRequest(
                "analysis.paper_budget_usd must be a positive number".into(),
            ));
        }
        if self
            .label_model
            .as_deref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err(Error::InvalidRequest(
                "analysis.label_model must not be empty; omit it to use the judgment model".into(),
            ));
        }
        if !self.label_budget_usd.is_finite() || self.label_budget_usd < 0.0 {
            return Err(Error::InvalidRequest(
                "analysis.label_budget_usd must be zero or a positive number".into(),
            ));
        }
        Ok(())
    }

    /// The model that writes edge labels over `OpenRouter`.
    #[must_use]
    pub fn label_model(&self) -> &str {
        self.label_model
            .as_deref()
            .or(self.judgment_model.as_deref())
            .unwrap_or(&self.model)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VimConfig {
    /// Optional Vimrc, resolved beside an existing application config file.
    pub vimrc: PathBuf,
}

impl Default for VimConfig {
    fn default() -> Self {
        Self {
            vimrc: PathBuf::from(".vimrc"),
        }
    }
}

impl AppConfig {
    /// The conventional config is optional; explicitly requested files must exist.
    pub async fn load(path: &Path, required: bool) -> Result<Self> {
        let file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(Error::io(path, error)),
        };
        let mut bytes = Vec::new();
        file.take(65_537)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| Error::io(path, error))?;
        if bytes.len() > 65_536 {
            return Err(Error::InvalidRequest(format!(
                "configuration {} exceeds 64 KiB",
                path.display()
            )));
        }
        let mut config: Self = serde_json::from_slice(&bytes).map_err(|error| {
            Error::InvalidRequest(format!("configuration {}: {error}", path.display()))
        })?;
        config.analysis.validate()?;
        if config.vim.vimrc.as_os_str().is_empty() {
            return Err(Error::InvalidRequest("vim.vimrc must not be empty".into()));
        }
        if config.vim.vimrc.is_relative() {
            config.vim.vimrc = path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&config.vim.vimrc);
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn config_defaults_and_explicit_overrides_are_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let defaults = AppConfig::load(&path, false).await.unwrap();
        assert_eq!(defaults.notes.date_format, "YYYY-MM-DD");
        assert_eq!(defaults.notes.time_format, "HH:mm");
        assert_eq!(defaults.notes.tags, ["paper"]);
        assert_eq!(defaults.vim.vimrc, Path::new(".vimrc"));
        assert!(AppConfig::load(&path, true).await.is_err());
        tokio::fs::write(
            &path,
            br#"{"notes":{"tags":["paper","ai"],"date_format":"DD/MM/YYYY"}}"#,
        )
        .await
        .unwrap();
        let config = AppConfig::load(&path, true).await.unwrap();
        assert_eq!(config.notes.tags, ["paper", "ai"]);
        assert_eq!(config.notes.date_format, "DD/MM/YYYY");
        assert_eq!(config.notes.time_format, "HH:mm");
        assert_eq!(config.vim.vimrc, dir.path().join(".vimrc"));
        tokio::fs::write(&path, br#"{"unknown_setting":true}"#)
            .await
            .unwrap();
        assert!(
            AppConfig::load(&path, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("unknown_setting")
        );
    }

    #[tokio::test]
    async fn should_default_the_analysis_to_a_pinned_provider_and_a_budget_that_reaches_the_self_check()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let defaults = AppConfig::load(&path, false).await.unwrap().analysis;
        assert_eq!(defaults.model, crate::analysis::openrouter::DEFAULT_MODEL);
        assert_eq!(defaults.judgment_model, None);
        assert_eq!(defaults.providers, ["deepinfra"]);
        assert!(defaults.paper_budget_usd >= 0.5);
        assert_eq!(
            defaults.label_model(),
            defaults.model,
            "labels default to the core model"
        );
        assert!(defaults.label_budget_usd > 0.0 && defaults.label_limit > 0);

        tokio::fs::write(
            &path,
            br#"{"analysis":{"judgment_model":"anthropic/claude-opus-4","paper_budget_usd":1.25}}"#,
        )
        .await
        .unwrap();
        let config = AppConfig::load(&path, true).await.unwrap().analysis;
        assert_eq!(
            config.judgment_model.as_deref(),
            Some("anthropic/claude-opus-4")
        );
        assert!((config.paper_budget_usd - 1.25).abs() < f64::EPSILON);
        assert_eq!(
            config.label_model(),
            "anthropic/claude-opus-4",
            "labels use the judgment model when one is set"
        );
        assert_eq!(
            config.providers,
            ["deepinfra"],
            "unset fields keep defaults"
        );

        for invalid in [
            r#"{"analysis":{"paper_budget_usd":0}}"#,
            r#"{"analysis":{"model":" "}}"#,
            r#"{"analysis":{"providers":[""]}}"#,
            r#"{"analysis":{"judgment_model":""}}"#,
            r#"{"analysis":{"budget":1}}"#,
            r#"{"analysis":{"label_model":" "}}"#,
            r#"{"analysis":{"label_budget_usd":-1}}"#,
        ] {
            tokio::fs::write(&path, invalid).await.unwrap();
            assert!(AppConfig::load(&path, true).await.is_err(), "{invalid}");
        }
    }

    #[tokio::test]
    async fn should_load_the_committed_config() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("lysilogy.config.json");
        let config = AppConfig::load(&path, true).await.unwrap();
        assert_eq!(config.analysis, AnalysisConfig::default());
    }

    #[tokio::test]
    async fn vimrc_paths_resolve_beside_config_and_absolute_paths_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        tokio::fs::write(&path, br#"{"vim":{"vimrc":"keymaps/notes.vim"}}"#)
            .await
            .unwrap();
        let config = AppConfig::load(&path, true).await.unwrap();
        assert_eq!(config.vim.vimrc, dir.path().join("keymaps/notes.vim"));
        let absolute = dir.path().join("shared.vim");
        tokio::fs::write(
            &path,
            serde_json::json!({"vim": {"vimrc": absolute}}).to_string(),
        )
        .await
        .unwrap();
        assert_eq!(
            AppConfig::load(&path, true).await.unwrap().vim.vimrc,
            absolute
        );
        tokio::fs::write(&path, br#"{"vim":{"vimrc":""}}"#)
            .await
            .unwrap();
        assert!(AppConfig::load(&path, true).await.is_err());
    }
}
