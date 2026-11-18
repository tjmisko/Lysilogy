//! Relationship labeling: why a paper cites each work.
//!
//! Two adapters to the ingest crate's [`RelationshipLabeler`] trait, chosen
//! by the app's provider setting ([`labeler_for`]):
//!
//! - [`OpenRouterLabeler`] calls `OpenRouter` with the analysis settings the
//!   core pipeline uses (model, providers, spend ledger) and its own per-paper
//!   label budget.
//! - [`SidecarLabeler`] adapts [`LocalCliAnalyzer`] to the local `claude` and
//!   `codex` CLIs. Each call runs in its own directory because the analyzer
//!   writes its schema and output files next to the process.

use std::path::PathBuf;
use std::sync::Mutex;

use lysilogy_ingest::labels::{
    LABEL_SCHEMA, LabelDraft, LabelFuture, LabelRequest, LabelerError, RelationshipLabeler,
    label_cache_key, label_prompt,
};

use crate::{
    Error, Result,
    analysis::{
        LocalCliAnalyzer, ReaderToolRequest,
        openrouter::{self, ChatRequest, ContentPart, ReasoningEffort},
        spend::SpendLedger,
        structured_model_key,
    },
    config::AnalysisConfig,
    domain::AnalysisProvider,
};

/// The ledger stage label calls are billed to.
pub const LABEL_STAGE: &str = "labels.edge";

/// Output ceiling for one label, reasoning included.
const LABEL_MAX_TOKENS: u32 = 4_000;

/// How long one label call may take.
const LABEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The labeler for `provider`, or `None` for the offline reader, which has
/// no model.
///
/// # Errors
/// When the `OpenRouter` HTTP client cannot be built.
pub fn labeler_for(
    provider: AnalysisProvider,
    settings: &AnalysisConfig,
    local_cli: &LocalCliAnalyzer,
    scratch: PathBuf,
) -> Result<Option<Box<dyn RelationshipLabeler>>> {
    Ok(match provider {
        AnalysisProvider::Heuristic => None,
        AnalysisProvider::OpenRouter => Some(Box::new(OpenRouterLabeler::new(
            settings,
            SpendLedger::with_defaults(),
        )?)),
        AnalysisProvider::Claude | AnalysisProvider::Codex => Some(Box::new(SidecarLabeler::new(
            provider,
            local_cli.clone(),
            scratch,
        )?)),
    })
}

/// A labeler that calls `OpenRouter` directly.
#[derive(Debug)]
pub struct OpenRouterLabeler {
    client: reqwest::Client,
    ledger: SpendLedger,
    model: String,
    providers: Vec<String>,
    budget_usd: f64,
    spent_usd: Mutex<f64>,
    model_key: String,
}

impl OpenRouterLabeler {
    /// A labeler on `settings.label_model()` with `settings.label_budget_usd`
    /// for this labeler's lifetime (one paper's run).
    ///
    /// # Errors
    /// When the HTTP client cannot be built.
    pub fn new(settings: &AnalysisConfig, ledger: SpendLedger) -> Result<Self> {
        let model = settings.label_model().to_owned();
        Ok(Self {
            client: openrouter::client(LABEL_TIMEOUT)?,
            ledger,
            model_key: format!("openrouter:{model}"),
            model,
            providers: settings.providers.clone(),
            budget_usd: settings.label_budget_usd,
            spent_usd: Mutex::new(0.0),
        })
    }

    /// What this labeler has spent so far.
    #[must_use]
    pub fn spent_usd(&self) -> f64 {
        *self
            .spent_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn reserve(&self) -> std::result::Result<(), LabelerError> {
        let spent = self.spent_usd();
        if spent >= self.budget_usd {
            return Err(LabelerError::Budget(format!(
                "${spent:.4} spent of the ${:.2} label budget",
                self.budget_usd
            )));
        }
        Ok(())
    }

    fn charge(&self, cost_usd: f64) {
        *self
            .spent_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += cost_usd;
    }
}

/// Parse a label answer.
///
/// # Errors
/// `Malformed` when the content is not a label.
pub fn parse_label_answer(content: &str) -> std::result::Result<LabelDraft, LabelerError> {
    let trimmed = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    serde_json::from_str(trimmed).map_err(|error| LabelerError::Malformed(error.to_string()))
}

impl RelationshipLabeler for OpenRouterLabeler {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn label<'a>(&'a self, request: &'a LabelRequest) -> LabelFuture<'a> {
        Box::pin(async move {
            self.reserve()?;
            let schema: serde_json::Value = serde_json::from_str(LABEL_SCHEMA)
                .map_err(|error| LabelerError::Failed(error.to_string()))?;
            let parts = [ContentPart::Text(label_prompt(request))];
            let completion = openrouter::complete(
                &self.client,
                &self.ledger,
                &ChatRequest {
                    model: &self.model,
                    parts: &parts,
                    schema: &schema,
                    stage: LABEL_STAGE,
                    max_tokens: Some(LABEL_MAX_TOKENS),
                    reasoning: Some(ReasoningEffort::Low),
                    provider_order: &self.providers,
                },
            )
            .await
            .map_err(labeler_error)?;
            self.charge(completion.parsed.cost_usd);
            parse_label_answer(&completion.parsed.content)
        })
    }
}

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
        Error::BudgetExceeded { .. } => LabelerError::Budget(error.to_string()),
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
    fn should_send_the_edge_label_brief_from_the_prompts_folder() {
        // The ingest crate embeds the brief; this keeps it the file reviewers edit.
        assert_eq!(
            include_str!("../prompts/edge-label.md"),
            lysilogy_ingest::labels::LABEL_BRIEF
        );
        assert!(label_prompt(&request()).starts_with(lysilogy_ingest::labels::LABEL_BRIEF.trim_end()));
    }

    #[test]
    fn should_parse_a_label_answer_with_or_without_a_code_fence() {
        let answer = r#"{"passage_index":0,"quote":"extend the cited method","engagement":"substantive","relation":"extends","reason":"Builds on it."}"#;
        let draft = parse_label_answer(answer).expect("label");
        assert_eq!(draft.engagement.as_deref(), Some("substantive"));
        let fenced = format!("```json\n{answer}\n```");
        assert_eq!(parse_label_answer(&fenced).expect("fenced"), draft);
        assert!(matches!(
            parse_label_answer(r#"{"relation":"extends"}"#),
            Err(LabelerError::Malformed(_))
        ));
    }

    #[test]
    fn should_choose_the_labeler_by_provider_and_name_the_label_model() {
        let settings = AnalysisConfig {
            judgment_model: Some("strong/model".to_owned()),
            ..AnalysisConfig::default()
        };
        let local = LocalCliAnalyzer::default();
        let scratch = PathBuf::from("labels");
        assert!(
            labeler_for(
                AnalysisProvider::Heuristic,
                &settings,
                &local,
                scratch.clone()
            )
            .unwrap()
            .is_none()
        );
        let openrouter = labeler_for(
            AnalysisProvider::OpenRouter,
            &settings,
            &local,
            scratch.clone(),
        )
        .unwrap()
        .expect("openrouter labeler");
        assert_eq!(openrouter.model_key(), "openrouter:strong/model");
        let claude = labeler_for(AnalysisProvider::Claude, &settings, &local, scratch)
            .unwrap()
            .expect("sidecar");
        assert_eq!(claude.model_key(), "claude:default");
    }

    #[tokio::test]
    async fn should_leave_edges_for_later_once_the_label_budget_is_spent() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let settings = AnalysisConfig {
            label_budget_usd: 0.01,
            ..AnalysisConfig::default()
        };
        let labeler = OpenRouterLabeler::new(
            &settings,
            SpendLedger::new(dir.path().join("spend.jsonl"), 1.0),
        )
        .unwrap();
        labeler.charge(0.02);
        let error = labeler.label(&request()).await.expect_err("over budget");
        assert!(matches!(error, LabelerError::Budget(_)), "{error}");
        assert!((labeler.spent_usd() - 0.02).abs() < f64::EPSILON);

        let spent_ledger = OpenRouterLabeler::new(
            &AnalysisConfig::default(),
            SpendLedger::new(dir.path().join("spent.jsonl"), 0.0),
        )
        .unwrap();
        let error = spent_ledger
            .label(&request())
            .await
            .expect_err("ledger spent");
        assert!(
            matches!(
                error,
                LabelerError::Budget(_) | LabelerError::Unavailable(_)
            ),
            "a spent ledger or a missing key stops the call before any request: {error}"
        );
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
