# 06 · Analysis agent harness

Status: proposed, 2026-10-16. Priority P1. Depends on 01–03.

## Problem

Every model-backed analysis stage currently runs through the vendor agent
harnesses: Codex and Claude are spawned as CLIs with a schema file, prompt on
stdin, session resume, and a tool partition chosen by profile. That was the
fastest way to get real work done, but it has costs:

- The vendor agent loop decides how tools are used; we cannot give a stage
  exactly the structured tools it should have.
- Planning is entangled with the harness rather than a stage we can turn on and
  off.
- Provider-specific behavior leaks into the pipeline (session files, JSON
  envelopes, model/effort routing).
- The spending policy is implicit: it is not obvious whether a call burns plan
  quota or metered credits, and the user wants plan first, DeepSeek second, and
  metered first-party APIs almost never.

The user's requirement: a custom extractor harness per agent, with access only
to structured tools, and planning available sometimes but not by default. Also:
spend the Claude and Codex plans first, prefer DeepSeek over metered
Anthropic/OpenAI, and use the plans outside the vendor harnesses if possible.

## Findings from the 2026-10-16 auth spike

- **The current `claude` provider spends the Claude plan.**
  `claude auth status` → `authMethod: "claude.ai"`, `subscriptionType: "pro"`.
  Spawning `claude --print` uses Pro quota even though `ANTHROPIC_API_KEY` is set
  in the environment. Note that `claude --bare` is the opposite: it forces
  `ANTHROPIC_API_KEY` and explicitly never reads OAuth or the keychain.
- **The current `codex` provider spends the ChatGPT plan.**
  `codex login status` → "Logged in using ChatGPT".
- **OpenRouter/DeepSeek is metered but separate** from Anthropic/OpenAI credits.
  It is the cheap volume path and is the default provider today.
- **Direct first-party API calls are metered.** `ANTHROPIC_API_KEY` and
  `OPENAI_API_KEY` are set, so raw adapters are technically available, but using
  them spends metered credits. That is the path to avoid by default.
- **Plan-backed HTTP does not exist officially.** Plan credentials live in
  `~/.claude/.credentials.json` and `~/.codex/auth.json` (OAuth). Pointing those
  tokens at `api.anthropic.com` or OpenAI directly is unsupported and a
  ToS/account risk; it stays an explicit user-chosen spike, never a dependency.

## Tool control under the plan harnesses

Plan quota means the vendor loop, but not uncontrolled tools:

- Claude Code headless supports `--tools` (replace the built-in tool list),
  `--allowedTools`, `--permission-mode`, `--append-system-prompt`,
  `--system-prompt`, `--settings`, `--agents`, `--json-schema`,
  `--output-format stream-json`, and `--mcp-config` with
  `--strict-mcp-config`. With `--tools ""` plus a strict MCP config, the only
  tools the model can call are ours.
- Codex `exec` supports `--sandbox read-only`, `-c` config overrides,
  `--disable <feature>`, `--output-schema`, `--json`, and external MCP servers
  through `codex mcp`. Same shape: restrict built-ins, attach our tools.
- The Claude Agent SDK and the Codex `app-server` are programmatic versions of
  the same plan-backed engines if permission hooks are needed.

What we cannot replace is the vendor system prompt and agent loop. The harness
still decides when to call a tool; we control which tools exist and validate
every call. So: **fork/exec, or the SDK/app-server, is the price of plan quota.**

## Spending policy

1. **Default: custom agents on OpenRouter.** Each stage runs our own harness
   against an OpenRouter model chosen per task. This is currently the cheaper
   path and keeps prompts, tools and spend under our control. See
   [agent harnesses and model routing](../agent-harnesses.md).
2. **Plan harnesses are available** — Claude Pro and ChatGPT through their
   CLIs/SDKs, with direct tool control through MCP — for when plan quota should
   be spent first.
3. **Metered first-party Anthropic/OpenAI is opt-in only** and never a default.

This ordering is revisitable; changing it is one policy, not a rewrite. Every
stage artifact must say which of the three paid for it.

## Goal

A provider-and-agent layer where each workstream runs its own harness with a
typed tool surface, optional planning, our own context/session handling, and
per-stage spend attribution under the policy above.

## Scope

1. **Provider adapters.**
   - An OpenRouter raw adapter with function/tool calling is the default; model
     choice is per task and per tier, not one global model.
   - Plan harness adapters (`claude`, `codex`, or their SDKs) with MCP-only tool
     surfaces for plan-backed runs.
   - No metered first-party Anthropic/OpenAI adapter in the default path.
   - Model routing per stage (cheap / mid / strong) is explicit and recorded
     with each artifact.

2. **Structured tool protocol.**
   - Tools are typed and deterministic: search the reading index, read a token
     span, propose a paragraph split/join, propose a bibliography entry repair,
     read a dossier, write a grounded claim. No shell, no arbitrary file reads,
     no raw web unless a stage is explicitly granted it.
   - For plan harnesses the tools are exposed as one MCP server per harness; for
     OpenRouter they are JSON tool definitions.
   - Every tool call and result is recorded in the stage artifact so a
     generation can be replayed and audited.
   - The application validates tool inputs and outputs; a model cannot bypass a
     validator by calling a tool.

3. **Per-agent harnesses.**
   - One harness definition per workstream: extraction repair, bibliography
     verification, field extraction, edge labeling, orientation, structure,
     context evidence/writer/review, clarification, experiment arm.
   - Each names its provider preference, model profile, tools, schemas,
     call/budget limits and cache identity.
   - Retire the one-size-fits-all `LocalCliAnalyzer` surface.

4. **Planning.**
   - An optional typed planning stage that produces a plan artifact consumed by
     the execution stage. Off by default; enabled per workstream or per run.
   - Planning never gets write tools; it observes and proposes.

5. **Sessions and context.**
   - Our own message/history management or cached artifact handoff instead of
     vendor session files, so resumption is provider-independent.
   - Long stages read artifacts, not a growing conversation.

6. **Spend and observability.**
   - Extend the JSONL spend ledger with a `funding` field (`plan`, `openrouter`,
     `metered`) per record; cap per harness and report per run.
   - A manager/report shows plan-vs-DeepSeek usage so "spend the plan down
     first" is a measurable fact, not an intention.

## Non-goals

- Rewriting every stage at once; start with one harness (bibliography
  verification is the natural first) and move stages one at a time.
- A general-purpose plugin system for tools.

## Acceptance

- A spike report: feasibility, cost and risk of (a) OpenRouter custom agents,
  (b) plan harnesses with MCP-only tools, (c) plan OAuth against vendor APIs,
  for both Anthropic and OpenAI.
- A routing benchmark scores candidate OpenRouter models per tier by grounded
  acceptance rate and cost; see
  [agent harnesses](../agent-harnesses.md#what-to-measure-before-committing).
- One new harness — bibliography verification ([02](02-bibliography-integrity.md))
  — runs with structured tools only, no shell, and records tool calls and
  funding source.
- The default path provably does not use `ANTHROPIC_API_KEY`/`OPENAI_API_KEY`;
  enabling metered first-party calls is an explicit switch.
- A plan-mode stage exists and is demonstrably off by default.
- Stage artifacts pin the provider, model, profile and tool set; old harnesses
  still pass their tests until retired.

## Open questions

1. Which tasks can move to the cheapest tier without a measurable drop in
   grounded acceptance?
2. Preference between the two plans when a plan-backed run is wanted: Claude
   Pro first or ChatGPT plan first, and does it depend on the stage?
3. Do we strip first-party API keys from the child environment, or merely never
   pass `--bare`?
4. Is "plan mode" a stage that plans the analysis, or a per-agent reasoning
   budget? Both may exist.
5. Do we keep a CLI fallback forever for plan quota, or move to the
   SDK/app-server once tool control is proven?
