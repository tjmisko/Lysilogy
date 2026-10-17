//! Bibliographic field extraction through the local `claude` and `codex` CLIs.
//!
//! [`SidecarFieldExtractor`] adapts [`LocalCliAnalyzer`] to the ingest crate's
//! [`FieldExtractor`] trait. Calls run on the cheap model (`fast`), and each
//! call gets its own directory because the analyzer writes its schema and
//! output files next to the process.

use std::path::PathBuf;

use lysilogy_ingest::field_model::{
    FIELD_SCHEMA, FieldDraft, FieldExtractor, FieldExtractorError, FieldFuture, FieldRequest,
    field_prompt,
};
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    analysis::{LocalCliAnalyzer, ReaderToolRequest, structured_fast_model_key},
    domain::AnalysisProvider,
};

/// Length of the prompt-hash prefix that names a call's directory.
const CALL_DIRECTORY_HASH_CHARS: usize = 16;

/// A field extractor that shells out to a local model CLI.
#[derive(Clone, Debug)]
pub struct SidecarFieldExtractor {
    analyzer: LocalCliAnalyzer,
    provider: AnalysisProvider,
    scratch: PathBuf,
    model_key: String,
}

impl SidecarFieldExtractor {
    /// An extractor for `provider`, using `scratch/<hash>/` for each call.
    ///
    /// # Errors
    /// `InvalidRequest` for the heuristic provider, which has no model.
    pub fn new(
        provider: AnalysisProvider,
        analyzer: LocalCliAnalyzer,
        scratch: PathBuf,
    ) -> Result<Self> {
        if provider == AnalysisProvider::Heuristic {
            return Err(Error::InvalidRequest(
                "field extraction needs a claude or codex sidecar".to_owned(),
            ));
        }
        Ok(Self {
            analyzer,
            provider,
            scratch,
            model_key: structured_fast_model_key(provider),
        })
    }
}

impl FieldExtractor for SidecarFieldExtractor {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn extract<'a>(&'a self, request: &'a FieldRequest) -> FieldFuture<'a> {
        Box::pin(async move {
            let prompt = field_prompt(request);
            let hash = format!("{:x}", Sha256::digest(prompt.as_bytes()));
            let directory = self.scratch.join(&hash[..CALL_DIRECTORY_HASH_CHARS]);
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| {
                    FieldExtractorError::Failed(format!(
                        "could not create {}: {error}",
                        directory.display()
                    ))
                })?;
            self.analyzer
                .reader_tool::<FieldDraft>(
                    self.provider,
                    &directory,
                    ReaderToolRequest {
                        schema: FIELD_SCHEMA,
                        prompt: &prompt,
                        live_web: false,
                        fast: true,
                        purpose: "fields",
                    },
                )
                .await
                .map_err(extractor_error)
        })
    }
}

fn extractor_error(error: Error) -> FieldExtractorError {
    match error {
        Error::ProgramUnavailable(program) => FieldExtractorError::Unavailable(program),
        Error::InvalidAnalysis(_) | Error::Json(_) => {
            FieldExtractorError::Malformed(error.to_string())
        }
        other => FieldExtractorError::Failed(other.to_string()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use lysilogy_ingest::field_model::FieldRequestKind;
    use std::os::unix::fs::PermissionsExt;

    fn request() -> FieldRequest {
        FieldRequest {
            kind: FieldRequestKind::BibliographyEntries,
            items: vec!["Ada Adams. A study of things. 2019.".to_owned()],
        }
    }

    fn program(directory: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("executable");
        path
    }

    #[test]
    fn should_reject_heuristic_provider_when_building_field_extractor() {
        let error = SidecarFieldExtractor::new(
            AnalysisProvider::Heuristic,
            LocalCliAnalyzer::default(),
            PathBuf::from("fields"),
        )
        .expect_err("heuristic has no model");
        assert!(matches!(error, Error::InvalidRequest(_)), "{error}");
    }

    #[test]
    fn should_key_fast_sidecar_as_claude_haiku() {
        let claude = SidecarFieldExtractor::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::default(),
            PathBuf::from("fields"),
        )
        .unwrap();
        assert_eq!(claude.model_key(), "claude:haiku");
        let codex = SidecarFieldExtractor::new(
            AnalysisProvider::Codex,
            LocalCliAnalyzer::default(),
            PathBuf::from("fields"),
        )
        .unwrap();
        assert!(
            codex.model_key().starts_with("codex:"),
            "{}",
            codex.model_key()
        );
        assert_ne!(codex.model_key(), "codex:gpt-5.6-terra");
    }

    #[tokio::test]
    async fn should_parse_field_draft_and_run_haiku_when_claude_sidecar_answers() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(
            dir.path(),
            "claude",
            r#"echo "$@" >> "$(dirname "$0")/arguments"
cat <<'JSON'
{"structured_output":{"items":[{"index":0,"title":"A study of things","authors":["Ada Adams"],"year":"2019"}]}}
JSON"#,
        );
        let scratch = dir.path().join("fields");
        let extractor = SidecarFieldExtractor::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            scratch.clone(),
        )
        .unwrap();
        let draft = extractor.extract(&request()).await.expect("draft");
        assert_eq!(draft.items.len(), 1);
        assert_eq!(draft.items[0].year, "2019");
        let arguments = std::fs::read_to_string(dir.path().join("arguments")).expect("arguments");
        assert!(arguments.contains("--model haiku"), "{arguments}");
        assert!(scratch.is_dir());
    }

    #[tokio::test]
    async fn should_report_malformed_when_field_output_is_not_items() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(
            dir.path(),
            "claude",
            r#"echo '{"structured_output":{"relation":"extends"}}'"#,
        );
        let extractor = SidecarFieldExtractor::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            dir.path().join("fields"),
        )
        .unwrap();
        let error = extractor.extract(&request()).await.expect_err("malformed");
        assert!(
            matches!(error, FieldExtractorError::Malformed(_)),
            "{error}"
        );
    }

    #[tokio::test]
    async fn should_report_unavailable_when_sidecar_program_missing() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let extractor = SidecarFieldExtractor::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", dir.path().join("no-such-claude")),
            dir.path().join("fields"),
        )
        .unwrap();
        let error = extractor.extract(&request()).await.expect_err("missing");
        assert!(
            matches!(error, FieldExtractorError::Unavailable(_)),
            "{error}"
        );
    }
}
