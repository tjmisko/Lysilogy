//! Small, explicit application settings; paper-note contents remain ordinary Markdown.
use crate::{Error, Result, notes::NoteTemplateConfig};
use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncReadExt;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub notes: NoteTemplateConfig,
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
        serde_json::from_slice(&bytes).map_err(|error| {
            Error::InvalidRequest(format!("configuration {}: {error}", path.display()))
        })
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
}
