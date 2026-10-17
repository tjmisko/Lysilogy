# Agent harnesses, plans, and model routing

Status: 2026-10-16. This is the reference for how Lysilogy spends model quota and
which model runs which task. The task list lives in [tasks/](tasks/README.md).

## Plan and auth facts (verified 2026-10-16)

| Access path | How it authenticates | What it spends |
| --- | --- | --- |
| Lysilogy `claude` provider → `claude --print` | OAuth in `~/.claude/.credentials.json` | **Claude plan** (`subscriptionType: "pro"`) |
| Lysilogy `codex` provider → `codex exec` | ChatGPT login in `~/.codex/auth.json` | **ChatGPT plan** |
| Lysilogy `openrouter` provider | `OPENROUTER_API_KEY` over HTTPS | **OpenRouter credits**, metered |
| Raw Anthropic Messages API | `ANTHROPIC_API_KEY` | **Metered credits** |
| Raw OpenAI Responses API | `OPENAI_API_KEY` | **Metered credits** |

Verify at any time:

```sh
claude auth status     # currently: authMethod "claude.ai", subscriptionType "pro"
codex login status     # currently: "Logged in using ChatGPT"
```

Key facts and caveats:

- Normal `claude` mode prefers the plan even when `ANTHROPIC_API_KEY` is set.
  `claude --bare` is the exception: it forces the API key and never reads OAuth
  or the keychain. We never pass `--bare`.
- There is **no official plan-backed HTTP API**. Plan quota flows through the
  vendor harness (CLI, Claude Agent SDK, Codex `app-server`). Pointing the OAuth
  tokens at `api.anthropic.com` or OpenAI directly is unsupported and a
  ToS/account risk; it is a user-chosen experiment, not a dependency.
- `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are set in this environment. They must
  not be reachable from the default path, or a harness could silently fall back
  to metered usage. Child environments should have them stripped unless a run
  explicitly opts in.

## Tool control under the plan harnesses

Plan access means the vendor agent loop, but not uncontrolled tools.

**Claude Code** (`claude --print ...`):

- `--tools` replaces the built-in tool list; `--allowedTools` narrows it.
- `--mcp-config` + `--strict-mcp-config` loads only our MCP servers.
- `--permission-mode`, `--settings`, `--agents` shape permissions.
- `--system-prompt` / `--append-system-prompt` control the system prompt.
- `--json-schema` + `--output-format stream-json` carry structured output and
  usage events.

With `--tools ""` and a strict MCP config, the only callable tools are ours.

**Codex** (`codex exec ...`):

- `--sandbox read-only` constrains shell execution.
- `-c key=value` overrides config; `--disable <feature>` turns features off.
- `--output-schema` + `--json` give structured output and events.
- External tools come from MCP servers managed by `codex mcp`.

**SDK-shaped options:** the Claude Agent SDK and the Codex `app-server` are
programmatic versions of the same plan-backed engines with permission hooks.

What cannot be replaced is the vendor system prompt and its decisions about
*when* to call a tool. We control which tools exist and validate every call, so
structured-tools-only harnesses are feasible; see
[tasks/06](tasks/06-analysis-agent-harness.md).

## Spending policy

1. **Default: custom agents on OpenRouter.** Stages run our own harness against
   an OpenRouter model chosen per task. This is currently cheaper than
   first-party APIs and keeps prompts/tools/spend under our control.
2. **Plan harnesses are available** when we want to spend plan quota or need a
   specific model. They use direct tool control via MCP.
3. **Metered first-party Anthropic/OpenAI is opt-in only** and never a default.
4. Every model call records its `funding` (`openrouter`, `plan`, `metered`) in
   the JSONL spend ledger.

This decision is revisitable. If plan quota should be spent first again, the
policy is one line, but the default today is OpenRouter custom agents.

## Model routing

The hypothesis: most Lysilogy model calls are bounded, grounded transformation
or classification over short text with a strict JSON schema. Those are cheap
tasks, and the cheapest capable model should do them. Only synthesis and
judgment need more firepower.

Prices below are USD per million tokens from the local OpenRouter cache
(`~/.pi/agent/openrouter-pricing.json`, fetched 2026-10-16). They change; the
cache and the OpenRouter models endpoint are the sources of truth.

### Candidate cheap models

| Model | in/M | out/M | cache/M | Notes |
| --- | --- | --- | --- | --- |
| `ibm-granite/granite-4.0-h-micro` | 0.017 | 0.112 | 0.017 | very cheap general text |
| `openai/gpt-oss-20b` | 0.018 | 0.090 | 0.009 | cheap, tool-aware family |
| `inclusionai/ling-3.0-flash` | 0.021 | 0.063 | 0.004 | cheapest with cheap cache |
| `nex-agi/nex-n2.5-mini` | 0.025 | 0.100 | 0.003 | cheap, low cache cost |
| `inference-net/schematron-v2-turbo` | 0.030 | 0.150 | 0.030 | schema/extraction-oriented |
| `inception/mercury-2.5` | 0.040 | 0.150 | 0.004 | diffusion-style, low cache |
| `google/gemma-3-4b-it` | 0.050 | 0.100 | 0.050 | small general |
| `qwen/qwen3.7-flash` | 0.030 | 0.130 | 0.006 | small Qwen flash |
| `deepseek/deepseek-v4-flash` | 0.091 | 0.182 | 0.018 | stronger cheap DeepSeek |
| `deepseek/deepseek-v4.1-flash` | 0.036 | 0.405 | 0.005 | current default; good cache |

### Candidate mid and strong models

| Model | in/M | out/M | Notes |
| --- | --- | --- | --- |
| `openai/gpt-5.6-luna` | 0.200 | 1.200 | workhorse mid-tier |
| `deepseek/deepseek-v4-pro` | 0.267 | 0.533 | strong, very low output |
| `z-ai/glm-4.7-flash` | 0.061 | 0.400 | cheap with strong output |
| `qwen/qwen3.6-flash` | 0.188 | 1.125 | mid |
| `anthropic/claude-haiku-5.5` | 0.100 | 0.500 | Haiku via OpenRouter |
| `openai/gpt-5.6-terra` | 2.000 | 12.000 | strong analysis |
| `anthropic/claude-sonnet-5.5` | 2.000 | 10.000 | strong analysis |
| `openai/gpt-6-astra` | 10.000 | 50.000 | strongest, use sparingly |

`:batch` variants are cheaper for latency-tolerant work; `:free` variants exist
but are not reliable defaults.

### Task tiers

| Tier | Nature of the task | Lysilogy stages | Suggested class |
| --- | --- | --- | --- |
| 0 | Deterministic | parsing, splitting, span resolution, ranking | no model |
| 1 | Bounded fuzzy text: classify, select, normalize, repair short spans | bibliography field verification, front-matter fields, landing-page link pick, edge-relation labeling, mention→entry suggestions, OCR word repair proposals | cheapest capable (granite micro, gpt-oss-20b, ling-flash, nex-mini, schematron) |
| 2 | Structured extraction over longer text | abstract repair/boundaries, sectioning, orientation, supercut, reference connection, clarification | cheap-to-mid (deepseek-v4-flash/pro, qwen flash, gpt-5.6-luna) |
| 3 | Synthesis, judgment, web research | context evidence/writer/review, revision, learning-ramp experiments | mid-to-strong (gpt-5.6-terra, sonnet-5.5, deepseek-v4-pro) |

Tier 1 is where the "straightforward fuzzy text manipulation" claim is safest:
short inputs, strict schemas, and deterministic grounding catch most failures.
Two caveats:

- Capability matters more than price for some cheap models: verify structured
  output and tool-calling reliability per candidate, not just the price.
- Grounding and validation are what make cheap models safe. Without exact-span
  or exact-quote admission, a cheap model's errors reach the archive.

### What to measure before committing

1. A routing benchmark: for each tier, run the real prompt/schema on a fixed
   sample across 3–5 candidate models and score grounded acceptance rate, not
   prose quality.
2. Support matrix for `response_format: json_schema` and tool calls.
3. Cost per stage from the spend ledger, so a model choice can be justified by
   measured acceptance × price.

## References

- [tasks/06 analysis agent harness](tasks/06-analysis-agent-harness.md)
- [tasks/02 bibliography integrity](tasks/02-bibliography-integrity.md)
- [Spend ledger](../src/analysis/spend.rs)
- [OpenRouter request path](../src/analysis/local_cli.rs)
