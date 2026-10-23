//! Saved model calls, so a retried run does not pay for answers it already has.
//!
//! A call is keyed by everything that shapes its answer: the backend, model,
//! effort, output cap, schema and every part of the prompt, page text and
//! images included. Change any of them and the key changes, so a stale answer
//! is never replayed. Only calls that returned a usable answer are saved.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    analysis::openrouter::{ContentPart, ReasoningEffort},
};

/// What a replayed call needs from the original.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Checkpoint {
    pub stage: String,
    pub content: String,
    pub provider: Option<String>,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
}

/// Everything that shapes one call's answer.
pub struct CallInputs<'a> {
    pub backend: &'a str,
    pub model: &'a str,
    pub reasoning: ReasoningEffort,
    pub max_tokens: u32,
    pub providers: &'a [String],
    pub schema: &'a str,
    pub parts: &'a [ContentPart],
}

impl CallInputs<'_> {
    /// A hex digest that differs whenever any input does.
    #[must_use]
    pub fn key(&self) -> String {
        let mut hasher = Sha256::new();
        // Length-prefixed so adjacent fields cannot run into each other.
        let mut field = |tag: u8, bytes: &[u8]| {
            hasher.update([tag]);
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        };
        field(b'b', self.backend.as_bytes());
        field(b'm', self.model.as_bytes());
        field(b'e', self.reasoning.label().as_bytes());
        field(b'x', &self.max_tokens.to_le_bytes());
        for provider in self.providers {
            field(b'o', provider.as_bytes());
        }
        field(b's', self.schema.as_bytes());
        for part in self.parts {
            match part {
                ContentPart::Text(text) => field(b't', text.as_bytes()),
                ContentPart::Png(bytes) => field(b'p', bytes),
            }
        }
        format!("{:x}", hasher.finalize())
    }
}

fn path(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("{key}.json"))
}

/// The saved answer for `key`; `None` when there is none or it is unreadable.
pub async fn load(directory: &Path, key: &str) -> Option<Checkpoint> {
    let bytes = tokio::fs::read(path(directory, key)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Save an answer. A failed save loses only the saving, never the run.
pub async fn save(directory: &Path, key: &str, checkpoint: &Checkpoint) {
    let result = async {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|error| Error::io(directory, error))?;
        crate::store::write_atomic(&path(directory, key), &serde_json::to_vec(checkpoint)?).await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(directory = %directory.display(), %error, "could not save a call checkpoint");
    }
}

/// Remove every saved answer, once the run they belong to has finished.
///
/// # Errors
/// Fails when the directory exists and cannot be removed.
pub async fn clear(directory: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(directory).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(directory, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(parts: &'a [ContentPart], providers: &'a [String]) -> CallInputs<'a> {
        CallInputs {
            backend: "openrouter",
            model: "model-a",
            reasoning: ReasoningEffort::Low,
            max_tokens: 1000,
            providers,
            schema: "{}",
            parts,
        }
    }

    fn checkpoint(content: &str) -> Checkpoint {
        Checkpoint {
            stage: "core".to_owned(),
            content: content.to_owned(),
            provider: Some("host".to_owned()),
            prompt_tokens: 10,
            cached_tokens: 2,
            completion_tokens: 5,
            reasoning_tokens: 1,
        }
    }

    #[test]
    fn should_change_the_key_when_any_input_changes() {
        let parts = [ContentPart::Text("page one".to_owned())];
        let none: Vec<String> = Vec::new();
        let base = inputs(&parts, &none).key();
        assert_eq!(base, inputs(&parts, &none).key());

        let other_parts = [ContentPart::Text("page two".to_owned())];
        let png_parts = [ContentPart::Png(b"page one".to_vec())];
        let providers = vec!["host".to_owned()];
        let variants = [
            inputs(&other_parts, &none).key(),
            inputs(&png_parts, &none).key(),
            inputs(&parts, &providers).key(),
            CallInputs {
                backend: "codex",
                ..inputs(&parts, &none)
            }
            .key(),
            CallInputs {
                model: "model-b",
                ..inputs(&parts, &none)
            }
            .key(),
            CallInputs {
                reasoning: ReasoningEffort::High,
                ..inputs(&parts, &none)
            }
            .key(),
            CallInputs {
                max_tokens: 2000,
                ..inputs(&parts, &none)
            }
            .key(),
            CallInputs {
                schema: "{\"a\":1}",
                ..inputs(&parts, &none)
            }
            .key(),
        ];
        for (index, variant) in variants.iter().enumerate() {
            assert_ne!(&base, variant, "variant {index} should change the key");
        }
    }

    #[test]
    fn should_not_confuse_where_one_part_ends_and_the_next_begins() {
        let none: Vec<String> = Vec::new();
        let split = [
            ContentPart::Text("ab".to_owned()),
            ContentPart::Text("c".to_owned()),
        ];
        let moved = [
            ContentPart::Text("a".to_owned()),
            ContentPart::Text("bc".to_owned()),
        ];
        assert_ne!(inputs(&split, &none).key(), inputs(&moved, &none).key());
    }

    #[tokio::test]
    async fn should_round_trip_a_saved_call() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let nested = directory.path().join("checkpoints");
        save(&nested, "abc", &checkpoint("{\"a\":1}")).await;
        let loaded = load(&nested, "abc").await.expect("saved");
        assert_eq!(loaded.content, "{\"a\":1}");
        assert_eq!(loaded.completion_tokens, 5);
    }

    #[tokio::test]
    async fn should_load_nothing_when_the_file_is_missing_or_corrupt() {
        let directory = tempfile::tempdir().expect("temporary directory");
        assert!(load(directory.path(), "missing").await.is_none());
        std::fs::write(directory.path().join("bad.json"), "{not json").expect("write");
        assert!(load(directory.path(), "bad").await.is_none());
    }

    #[tokio::test]
    async fn should_clear_every_checkpoint_and_tolerate_a_missing_directory() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let nested = directory.path().join("checkpoints");
        clear(&nested).await.expect("missing directory is fine");
        save(&nested, "abc", &checkpoint("{}")).await;
        clear(&nested).await.expect("cleared");
        assert!(load(&nested, "abc").await.is_none());
    }
}
