//! Landing-page link picking through the local `claude` and `codex` CLIs.
//!
//! [`SidecarLinkPicker`] adapts [`LocalCliAnalyzer`] to the ingest crate's
//! [`LandingLinkPicker`] trait, mirroring `SidecarLabeler`. Each call runs in
//! its own directory because the analyzer writes its schema and output files
//! next to the process.

use std::path::PathBuf;

use lysilogy_ingest::acquire::{
    LINK_PICK_SCHEMA, LandingLinkPicker, LinkPick, LinkPickRequest, PickFuture, PickerError,
    link_pick_cache_key, link_pick_prompt,
};

use crate::{
    Error, Result,
    analysis::{LocalCliAnalyzer, ReaderToolRequest, structured_model_key},
    domain::AnalysisProvider,
};

/// Length of the cache-key prefix that names a call's directory.
const CALL_DIRECTORY_HASH_CHARS: usize = 16;

/// A link picker that shells out to a local model CLI.
#[derive(Clone, Debug)]
pub struct SidecarLinkPicker {
    analyzer: LocalCliAnalyzer,
    provider: AnalysisProvider,
    scratch: PathBuf,
    model_key: String,
}

impl SidecarLinkPicker {
    /// A picker for `provider`, using `scratch/<hash>/` for each call.
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
                "link picking needs a claude or codex sidecar".to_owned(),
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

impl LandingLinkPicker for SidecarLinkPicker {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn pick<'a>(&'a self, request: &'a LinkPickRequest) -> PickFuture<'a> {
        Box::pin(async move {
            let key = link_pick_cache_key(request);
            let directory = self.scratch.join(&key[..CALL_DIRECTORY_HASH_CHARS]);
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| {
                    PickerError::Failed(format!(
                        "could not create {}: {error}",
                        directory.display()
                    ))
                })?;
            let prompt = link_pick_prompt(request);
            self.analyzer
                .reader_tool::<LinkPick>(
                    self.provider,
                    &directory,
                    ReaderToolRequest {
                        schema: LINK_PICK_SCHEMA,
                        prompt: &prompt,
                        live_web: false,
                        fast: false,
                    },
                )
                .await
                .map_err(picker_error)
        })
    }
}

fn picker_error(error: Error) -> PickerError {
    match error {
        Error::ProgramUnavailable(program) => PickerError::Unavailable(program),
        Error::InvalidAnalysis(_) | Error::Json(_) => PickerError::Malformed(error.to_string()),
        other => PickerError::Failed(other.to_string()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use lysilogy_ingest::acquire::PageLink;
    use std::os::unix::fs::PermissionsExt;

    fn request() -> LinkPickRequest {
        LinkPickRequest {
            work_title: Some("A Paper".to_owned()),
            work_year: Some(2020),
            page_url: "https://example.org/article".to_owned(),
            links: vec![
                PageLink {
                    href: "https://example.org/supplement".to_owned(),
                    text: "Supplement".to_owned(),
                },
                PageLink {
                    href: "https://example.org/full.pdf".to_owned(),
                    text: "Full text".to_owned(),
                },
            ],
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
    fn should_reject_heuristic_provider_when_building_link_picker() {
        let error = SidecarLinkPicker::new(
            AnalysisProvider::Heuristic,
            LocalCliAnalyzer::default(),
            PathBuf::from("picks"),
        )
        .expect_err("heuristic has no model");
        assert!(matches!(error, Error::InvalidRequest(_)), "{error}");
    }

    #[tokio::test]
    async fn should_parse_link_pick_when_claude_sidecar_returns_structured_output() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(
            dir.path(),
            "claude",
            r#"cat <<'JSON'
{"structured_output":{"pdf_link_index":1,"reason":"The second link is the article PDF."}}
JSON"#,
        );
        let scratch = dir.path().join("picks");
        let picker = SidecarLinkPicker::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            scratch.clone(),
        )
        .unwrap();
        let request = request();
        let pick = picker.pick(&request).await.expect("pick");
        assert_eq!(pick.pdf_link_index, Some(1));
        let key = link_pick_cache_key(&request);
        assert!(
            scratch
                .join(&key[..CALL_DIRECTORY_HASH_CHARS])
                .join("output.schema.json")
                .is_file(),
            "each call works in its own hashed directory"
        );
    }

    #[tokio::test]
    async fn should_report_malformed_when_sidecar_output_is_not_a_pick() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let claude = program(
            dir.path(),
            "claude",
            r#"echo '{"structured_output":{"pdf_link_index":"first","reason":"wrong type"}}'"#,
        );
        let picker = SidecarLinkPicker::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands("codex-not-used", &claude),
            dir.path().join("picks"),
        )
        .unwrap();
        let error = picker.pick(&request()).await.expect_err("malformed");
        assert!(matches!(error, PickerError::Malformed(_)), "{error}");
    }

    #[tokio::test]
    async fn should_report_unavailable_when_sidecar_program_missing() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let picker = SidecarLinkPicker::new(
            AnalysisProvider::Claude,
            LocalCliAnalyzer::with_commands(
                "codex-not-used",
                dir.path().join("no-such-claude-program"),
            ),
            dir.path().join("picks"),
        )
        .unwrap();
        let error = picker.pick(&request()).await.expect_err("missing");
        assert!(matches!(error, PickerError::Unavailable(_)), "{error}");
    }
}
