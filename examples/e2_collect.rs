//! E2 collection: real citations written by a model from memory.
//!
//! Sends the reading-list planner's real plan-pass prompt (size 15, level survey, an empty
//! knowledge profile and empty graph context) for each goal, through the repo's `OpenRouter`
//! client and a scratch spend ledger, and saves the raw answer and usage per goal.
//!
//! `cargo run --example e2_collect -- --goals goals.json --out raw/deepseek --ledger ledger.jsonl --budget 0.12`
//! `cargo run --example e2_collect -- --goals goals.json --dump-prompts prompts-dir` writes
//! each goal's instructions and input for a plan-backed harness and calls no model.

use std::path::PathBuf;
use std::time::Instant;

use lysilogy::analysis::openrouter::{
    self, ChatRequest, ContentPart, DEFAULT_MODEL, ReasoningEffort,
};
use lysilogy::analysis::spend::SpendLedger;
use lysilogy::reading_list_planner::openrouter::parse_draft;
use lysilogy::reading_list_planner::{
    Level, PLAN_PROMPT, PLAN_SCHEMA, PlanDraft, PlanRequest, prompts,
};
use serde_json::{Value, json};

type Failure = Box<dyn std::error::Error>;

const MAX_TOKENS: u32 = 12_000;

/// The planner's own parse of an answer, as a claim count or the error text.
fn parse_plan(content: &str) -> Result<usize, String> {
    parse_draft::<PlanDraft>(content)
        .map(|draft| draft.topics.iter().map(|topic| topic.claims.len()).sum())
        .map_err(|error| error.to_string())
}

fn request_for(goal: &str) -> PlanRequest {
    PlanRequest {
        goal: goal.to_owned(),
        size: 15,
        level: Level::Survey,
        anchors: Vec::new(),
        graph_context: Vec::new(),
        profile: Vec::new(),
        existing_lists: Vec::new(),
    }
}

#[tokio::main]
#[allow(clippy::too_many_lines)] // two modes read top to bottom
async fn main() -> Result<(), Failure> {
    let mut goals_path = None;
    let mut out = None;
    let mut dump = None;
    let mut ledger_path = None;
    let mut budget = 0.12_f64;
    let mut model = DEFAULT_MODEL.to_owned();
    let mut only = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or("flag needs a value");
        match flag.as_str() {
            "--goals" => goals_path = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--dump-prompts" => dump = Some(PathBuf::from(value()?)),
            "--ledger" => ledger_path = Some(PathBuf::from(value()?)),
            "--budget" => budget = value()?.parse()?,
            "--model" => model = value()?,
            "--only" => only = Some(value()?),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let goals: Vec<Value> = serde_json::from_slice(&std::fs::read(goals_path.ok_or("--goals")?)?)?;
    let goals: Vec<(String, String)> = goals
        .iter()
        .filter_map(|goal| {
            Some((
                goal["id"].as_str()?.to_owned(),
                goal["goal"].as_str()?.to_owned(),
            ))
        })
        .filter(|(id, _)| only.as_ref().is_none_or(|only| only == id))
        .collect();

    if let Some(dir) = dump {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("instructions.md"), PLAN_PROMPT.trim_end())?;
        std::fs::write(dir.join("schema.json"), PLAN_SCHEMA)?;
        for (id, goal) in &goals {
            let full = prompts::plan_prompt(&request_for(goal));
            let input = full
                .strip_prefix(PLAN_PROMPT.trim_end())
                .ok_or("prompt does not start with the instructions")?
                .trim_start();
            std::fs::write(dir.join(format!("{id}.input.md")), input)?;
        }
        return Ok(());
    }

    let out = out.ok_or("--out")?;
    std::fs::create_dir_all(&out)?;
    let ledger = SpendLedger::new(ledger_path.ok_or("--ledger")?, budget);
    let client = openrouter::client(std::time::Duration::from_secs(300))?;
    let schema: Value = serde_json::from_str(PLAN_SCHEMA)?;
    for (id, goal) in &goals {
        let parts = [ContentPart::Text(prompts::plan_prompt(&request_for(goal)))];
        let request = ChatRequest {
            model: &model,
            parts: &parts,
            schema: &schema,
            stage: "reading_list_plan",
            max_tokens: Some(MAX_TOKENS),
            reasoning: Some(ReasoningEffort::Low),
            provider_order: &[],
        };
        let started = Instant::now();
        let result = openrouter::complete(&client, &ledger, &request).await;
        let record = match result {
            Ok(completion) => {
                let parsed = completion.parsed;
                let parse = parse_plan(&parsed.content);
                eprintln!(
                    "{id}: {} chars, {} prompt / {} completion tokens, ${:.5}, finish {:?}, parse {}",
                    parsed.content.len(),
                    parsed.prompt_tokens,
                    parsed.completion_tokens,
                    parsed.cost_usd,
                    parsed.finish_reason,
                    parse
                        .as_ref()
                        .map_or_else(Clone::clone, |count| format!("ok, {count} claims")),
                );
                json!({
                    "goal_id": id, "model": model, "ok": true,
                    "content": parsed.content,
                    "parse_error": parse.err(),
                    "usage": {
                        "prompt_tokens": parsed.prompt_tokens,
                        "completion_tokens": parsed.completion_tokens,
                        "reasoning_tokens": parsed.reasoning_tokens,
                        "cached_tokens": parsed.cached_tokens,
                        "cost_usd": parsed.cost_usd,
                        "cost_source": format!("{:?}", parsed.cost_source),
                        "provider": parsed.provider,
                        "finish_reason": parsed.finish_reason,
                        "latency_s": started.elapsed().as_secs_f64(),
                    },
                })
            }
            Err(error) => {
                eprintln!("{id}: FAILED {error}");
                json!({"goal_id": id, "model": model, "ok": false, "error": error.to_string()})
            }
        };
        std::fs::write(
            out.join(format!("{id}.json")),
            serde_json::to_string_pretty(&record)? + "\n",
        )?;
        if ledger.ensure_available().is_err() {
            eprintln!("budget spent ({:.5}); stopping", ledger.total_usd());
            break;
        }
    }
    eprintln!("ledger total ${:.5}", ledger.total_usd());
    Ok(())
}
