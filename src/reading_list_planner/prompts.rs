//! Prompt text of the two model passes: the instructions, then the input as JSON.

use serde::Serialize;

use super::{FILL_PROMPT, FillRequest, PLAN_PROMPT, PlanRequest};

fn with_input(instructions: &str, input: &impl Serialize) -> String {
    let json = serde_json::to_string_pretty(input).unwrap_or_default();
    format!("{}\n\n## Input\n\n{json}\n", instructions.trim_end())
}

/// The plan pass's prompt.
#[must_use]
pub fn plan_prompt(request: &PlanRequest) -> String {
    with_input(PLAN_PROMPT, request)
}

/// The fill pass's prompt.
#[must_use]
pub fn fill_prompt(request: &FillRequest) -> String {
    with_input(FILL_PROMPT, request)
}
