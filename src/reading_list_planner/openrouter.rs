//! The `OpenRouter` planner: strict JSON-schema completions through the spend ledger.
//!
//! It is never exercised live by the test suite; the tests cover the request it builds,
//! the schemas, and how answers and failures map to [`PlannerError`].

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    FILL_SCHEMA, FillDraft, FillRequest, PLAN_SCHEMA, PlanDraft, PlanRequest, PlannerError,
    PlannerFuture, PlannerReply, ReadingListPlanner, prompts,
};
use crate::Error;
use crate::analysis::openrouter::{self, ChatRequest, ContentPart, DEFAULT_MODEL, ReasoningEffort};
use crate::analysis::spend::{SpendLedger, default_ledger_path};

/// The whole call may take this long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// The most output tokens one pass may produce, reasoning included.
const MAX_TOKENS: u32 = 12_000;
/// Ledger stage names.
const PLAN_STAGE: &str = "reading_list_plan";
const FILL_STAGE: &str = "reading_list_fill";

/// A planner that asks an `OpenRouter` model.
#[derive(Clone, Debug)]
pub struct OpenRouterPlanner {
    client: reqwest::Client,
    ledger: SpendLedger,
    model: String,
    plan_schema: Value,
    fill_schema: Value,
}

impl OpenRouterPlanner {
    /// A planner for `model` billing `ledger`.
    ///
    /// # Errors
    /// Fails when the HTTP client cannot be built or a bundled schema is not JSON.
    pub fn new(model: &str, ledger: SpendLedger) -> crate::Result<Self> {
        Ok(Self {
            client: openrouter::client(REQUEST_TIMEOUT)?,
            ledger,
            model: model.to_owned(),
            plan_schema: serde_json::from_str(PLAN_SCHEMA)?,
            fill_schema: serde_json::from_str(FILL_SCHEMA)?,
        })
    }

    /// A planner on the default model whose ledger allows `budget_usd` more spend than
    /// the ledger file already holds, so earlier runs do not use up this run's budget.
    ///
    /// # Errors
    /// As [`Self::new`].
    pub fn for_budget(budget_usd: f64) -> crate::Result<Self> {
        let path = default_ledger_path();
        let spent = SpendLedger::new(path.clone(), f64::MAX).total_usd();
        Self::new(DEFAULT_MODEL, SpendLedger::new(path, spent + budget_usd))
    }

    async fn ask<T: DeserializeOwned>(
        &self,
        prompt: String,
        schema: &Value,
        stage: &str,
    ) -> Result<PlannerReply<T>, PlannerError> {
        let parts = [ContentPart::Text(prompt)];
        let request = ChatRequest {
            model: &self.model,
            parts: &parts,
            schema,
            stage,
            max_tokens: Some(MAX_TOKENS),
            reasoning: Some(ReasoningEffort::Low),
            provider_order: &[],
        };
        let completion = openrouter::complete(&self.client, &self.ledger, &request)
            .await
            .map_err(planner_error)?;
        let draft = parse_draft(&completion.parsed.content)?;
        Ok(PlannerReply {
            draft,
            cost_usd: completion.parsed.cost_usd,
        })
    }
}

/// Parse a model answer into a draft.
///
/// # Errors
/// `Malformed` when the text is not JSON of the draft's shape.
pub fn parse_draft<T: DeserializeOwned>(content: &str) -> Result<T, PlannerError> {
    serde_json::from_str(content.trim()).map_err(|error| PlannerError::Malformed(error.to_string()))
}

/// Map a client failure to the planner's vocabulary.
#[must_use]
pub fn planner_error(error: Error) -> PlannerError {
    match error {
        Error::ProgramUnavailable(program) => PlannerError::Unavailable(program),
        Error::BudgetExceeded { .. } => PlannerError::BudgetExceeded(error.to_string()),
        Error::InvalidAnalysis(_) | Error::Json(_) => PlannerError::Malformed(error.to_string()),
        other => PlannerError::Failed(other.to_string()),
    }
}

impl ReadingListPlanner for OpenRouterPlanner {
    fn model_key(&self) -> &str {
        &self.model
    }

    fn funding(&self) -> &'static str {
        "openrouter"
    }

    fn plan<'a>(&'a self, request: &'a PlanRequest) -> PlannerFuture<'a, PlanDraft> {
        Box::pin(async move {
            self.ask(prompts::plan_prompt(request), &self.plan_schema, PLAN_STAGE)
                .await
        })
    }

    fn fill<'a>(&'a self, request: &'a FillRequest) -> PlannerFuture<'a, FillDraft> {
        Box::pin(async move {
            self.ask(prompts::fill_prompt(request), &self.fill_schema, FILL_STAGE)
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::openrouter::request_body;
    use crate::reading_list_planner::Level;

    /// `OpenRouter` strict mode: every object lists all its properties as required and
    /// allows no others.
    fn assert_strict(schema: &Value, path: &str) {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            assert_eq!(
                schema.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "{path} must forbid extra properties"
            );
            let mut required: Vec<&str> = schema["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{path} needs required"))
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let mut listed: Vec<&str> = properties.keys().map(String::as_str).collect();
            required.sort_unstable();
            listed.sort_unstable();
            assert_eq!(required, listed, "{path} must require every property");
            for (name, child) in properties {
                assert_strict(child, &format!("{path}.{name}"));
            }
        }
        if let Some(items) = schema.get("items") {
            assert_strict(items, &format!("{path}[]"));
        }
    }

    #[test]
    fn should_ship_strict_schemas_when_building_the_planner() {
        for (name, text) in [("plan", PLAN_SCHEMA), ("fill", FILL_SCHEMA)] {
            let schema: Value = serde_json::from_str(text).expect("schema is JSON");
            assert_strict(&schema, name);
        }
    }

    #[test]
    fn should_send_a_strict_json_schema_request_when_asking_the_model() {
        let schema: Value = serde_json::from_str(PLAN_SCHEMA).expect("schema");
        let parts = [ContentPart::Text("prompt".to_owned())];
        let body = request_body(&ChatRequest {
            model: DEFAULT_MODEL,
            parts: &parts,
            schema: &schema,
            stage: PLAN_STAGE,
            max_tokens: Some(MAX_TOKENS),
            reasoning: Some(ReasoningEffort::Low),
            provider_order: &[],
        });
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["messages"][0]["content"], "prompt");
    }

    #[test]
    fn should_parse_a_plan_when_the_model_answers_with_the_schema_shape() {
        let draft: PlanDraft = parse_draft(
            r#"{"topics":[{"label":"RL","summary":"s","claims":[{"title":"T","authors":["A"],
            "year":null,"venue":null,"doi":null,"arxiv":null,"role":"core","rationale":"r",
            "context_links":[]}],"graph_works":[]}],"topic_edges":[],"assumed_known":[]}"#,
        )
        .expect("plan");
        assert_eq!(draft.topics[0].claims[0].title, "T");
        assert_eq!(draft.topics[0].claims[0].year, None);
    }

    #[test]
    fn should_report_malformed_when_the_model_answers_with_prose() {
        let error = parse_draft::<PlanDraft>("Sure! Here is a list.").expect_err("prose");
        assert!(matches!(error, PlannerError::Malformed(_)), "{error}");
        let error = parse_draft::<FillDraft>(r#"{"picks": 3}"#).expect_err("wrong shape");
        assert!(matches!(error, PlannerError::Malformed(_)), "{error}");
    }

    #[test]
    fn should_map_client_failures_when_a_call_does_not_complete() {
        assert!(matches!(
            planner_error(Error::ProgramUnavailable("OPENROUTER_API_KEY".to_owned())),
            PlannerError::Unavailable(_)
        ));
        assert!(matches!(
            planner_error(Error::BudgetExceeded {
                budget_usd: 0.1,
                spent_usd: 0.2
            }),
            PlannerError::BudgetExceeded(_)
        ));
        assert!(matches!(
            planner_error(Error::InvalidAnalysis("bad".to_owned())),
            PlannerError::Malformed(_)
        ));
        assert!(matches!(
            planner_error(Error::Task("503".to_owned())),
            PlannerError::Failed(_)
        ));
    }

    #[test]
    fn should_render_the_goal_and_instructions_when_building_the_plan_prompt() {
        let prompt = prompts::plan_prompt(&PlanRequest {
            goal: "learn RLHF reward modelling".to_owned(),
            size: 20,
            level: Level::Intro,
            anchors: Vec::new(),
            graph_context: Vec::new(),
            profile: Vec::new(),
            existing_lists: Vec::new(),
        });
        assert!(prompt.contains("learn RLHF reward modelling"));
        assert!(prompt.contains("\"level\": \"intro\""));
        assert!(prompt.contains("assumed_known"));
    }
}
