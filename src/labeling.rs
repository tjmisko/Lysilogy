//! Relationship labeling through the local `claude` and `codex` CLIs.
//!
//! [`SidecarLabeler`] adapts [`LocalCliAnalyzer`] to the ingest crate's
//! [`RelationshipLabeler`] trait. Each call runs in its own directory because
//! the analyzer writes its schema and output files next to the process.

use std::path::PathBuf;

use lysilogy_ingest::labels::{
    LABEL_SCHEMA, LabelDraft, LabelFuture, LabelRequest, LabelerError, RelationshipLabeler,
    label_cache_key, label_prompt,
};

use crate::{
    Error, Result,
    analysis::{LocalCliAnalyzer, ReaderToolRequest, structured_model_key},
    domain::AnalysisProvider,
};

/// Length of the cache-key prefix that names a call's directory.
const CALL_DIRECTORY_HASH_CHARS: usize = 16;

/// A labeler that shells out to a local model CLI.
#[derive(Clone, Debug)]
pub struct SidecarLabeler {
    analyzer: LocalCliAnalyzer,
    provider: AnalysisProvider,
    scratch: PathBuf,
    model_key: String,
}

impl SidecarLabeler {
    /// A labeler for `provider`, using `scratch/<hash>/` for each call.
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
                "relationship labels need a claude or codex sidecar".to_owned(),
            ));
        }
        Ok(Self {
            analyzer,
            provider,
            scratch,
            model_key: structured_model_key(provider),
        })
    }
}

impl RelationshipLabeler for SidecarLabeler {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn label<'a>(&'a self, request: &'a LabelRequest) -> LabelFuture<'a> {
        Box::pin(async move {
            let key = label_cache_key(request);
            let directory = self.scratch.join(&key[..CALL_DIRECTORY_HASH_CHARS]);
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| {
                    LabelerError::Failed(format!(
                        "could not create {}: {error}",
                        directory.display()
                    ))
                })?;
            let prompt = label_prompt(request);
            self.analyzer
                .reader_tool::<LabelDraft>(
                    self.provider,
                    &directory,
                    ReaderToolRequest {
                        schema: LABEL_SCHEMA,
                        prompt: &prompt,
                        live_web: false,
                        fast: false,
                        purpose: "labels",
                    },
                )
                .await
                .map_err(labeler_error)
        })
    }
}

fn labeler_error(error: Error) -> LabelerError {
    match error {
        Error::ProgramUnavailable(program) => LabelerError::Unavailable(program),
        Error::InvalidAnalysis(_) | Error::Json(_) => LabelerError::Malformed(error.to_string()),
        other => LabelerError::Failed(other.to_string()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn request() -> LabelRequest {
        LabelRequest {
            citing_title: Some("Citing".to_owned()),
            passages: vec!["We extend the cited method [1] to new tasks.".to_owned()],
            ..LabelRequest::default()
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
    fn should_reject_heuristic_provider_when_building_sidecar() {
        let error = SidecarLabeler::new(
            AnalysisProvider::Heuristic,
            LocalCliAnalyzer::default(),
            PathBuf::from("labels"),
        )
        .expect_err("heuristic has no model");
        assert!(matches!(error, Error::InvalidRequest(_)), "{error}");
    }

    #[test]
    fn should_key_the_sidecar_by_model_when_provider_chosen() {
        let claude = SidecarLabeler::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::default(),
            PathBuf::from("labels"),
        )
        .unwrap();
        assert_eq!(claude.model_key(), "claude:default");
        let codex = SidecarLabeler::new(
            AnalysisProvider::Codex,
            LocalCliAnalyzer::default(),
            PathBuf::from("labels"),
        )
        .unwrap();
        assert!(
            codex.model_key().starts_with("codex:"),
            "{}",
            codex.model_key()
        );
    }

    #[tokio::test]
    async fn should_parse_label_draft_when_claude_sidecar_returns_structured_output() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(
            dir.path(),
            "claude",
            r#"cat <<'JSON'
{"structured_output":{"relation":"extends","reason":"It builds on the method.","quote":"extend the cited method","passage_index":0}}
JSON"#,
        );
        let scratch = dir.path().join("labels");
        let labeler = SidecarLabeler::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            scratch.clone(),
        )
        .unwrap();
        let request = request();
        let draft = labeler.label(&request).await.expect("draft");
        assert_eq!(draft.relation, "extends");
        assert_eq!(draft.quote, "extend the cited method");
        assert_eq!(draft.passage_index, 0);
        let key = label_cache_key(&request);
        assert!(
            scratch
                .join(&key[..CALL_DIRECTORY_HASH_CHARS])
                .join("output.schema.json")
                .is_file(),
            "each call works in its own hashed directory"
        );
    }

    #[tokio::test]
    async fn should_report_malformed_when_sidecar_output_is_not_a_label() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(
            dir.path(),
            "claude",
            r#"echo '{"structured_output":{"relation":"extends"}}'"#,
        );
        let labeler = SidecarLabeler::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            dir.path().join("labels"),
        )
        .unwrap();
        let error = labeler.label(&request()).await.expect_err("malformed");
        assert!(matches!(error, LabelerError::Malformed(_)), "{error}");
    }

    #[tokio::test]
    async fn should_report_failed_when_sidecar_program_exits_nonzero() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(dir.path(), "claude", "echo broken >&2\nexit 3");
        let labeler = SidecarLabeler::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            dir.path().join("labels"),
        )
        .unwrap();
        let error = labeler.label(&request()).await.expect_err("failure");
        assert!(
            matches!(&error, LabelerError::Failed(text) if text.contains("broken")),
            "{error}"
        );
    }

    #[tokio::test]
    async fn should_report_unavailable_when_sidecar_program_missing() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let labeler = SidecarLabeler::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands(
                "codex-not-used",
                dir.path().join("no-such-claude-program"),
            ),
            dir.path().join("labels"),
        )
        .unwrap();
        let error = labeler.label(&request()).await.expect_err("missing");
        assert!(matches!(error, LabelerError::Unavailable(_)), "{error}");
    }
}
