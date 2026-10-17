# Handoff: extraction agent spike

Status: 2026-10-16. This is the executable plan for the spike that tests the
architecture hypothesis in [../product-roadmap.md](../product-roadmap.md): that
one agent, after basic resource extraction, can fill the paper data model more
accurately and cheaply than the deterministic pipeline.

Read [../product-roadmap.md](../product-roadmap.md) first. This document is the
work order.

## Objective

For a fixed sample of papers, have a model fill a `paper-artifacts` schema from
the paper's page text and page images, check its own work, and measure accuracy
and cost. Decide from that measurement whether to replace the deterministic
extraction pipeline.

## Decision gates

The spike is done when we can answer, with numbers:

1. What is the per-field accuracy of the agent output against a ground-truth
   oracle and against a human rubric?
2. What does it cost per paper, and how long does it take?
3. Does the second (self-check) pass improve accuracy enough to justify its
   cost?
4. Is the agent better than the existing deterministic pipeline on the same
   artifacts, per dollar?

Outcomes:

- **Near-perfect on cheap model** (see thresholds): stop, adopt the agent path,
  retire the deterministic extraction stages.
- **Not near-perfect**: escalate the model (ladder below) and re-run the same
  sample. Re-decide.
- **Cost or accuracy still unacceptable after escalation**: keep the agent for
  the parts it wins and reconsider the rest.

Suggested initial thresholds, to be revised once we see pass 1: identity and
abstract ≥ 95% normalized exact; object counts and labels ≥ 90%; every supercut
quote verbatim in the source; no fabricated citations. Treat these as a starting
bar, not a hard gate.

## Preconditions

Nothing else is built until these exist.

### P1. Paper-artifacts schema

`prompts/paper-artifacts.schema.json` covering the paper-alone Tier-1 surface
from the roadmap. Every item carries `evidence` (verbatim quote + page) and
`confidence`. Proposed top-level fields:

- `identity`: title, subtitle, authors (ordered), year, venue, identifiers,
  preprint/published
- `type`: survey | method | empirical | theory | dataset | position |
  replication | resource
- `abstract`
- `card`: thesis, position {predecessors, targets, replications}, headline
  results, read_signal {read | skim | glance | summary_enough, reason}
- `supercut`: ordered segments, each `{kind: source|connector, text, page}`
- `question`
- `claims`: statement, support, strength, evidence refs
- `method`: setting, data, procedure, assumptions
- `key_numbers`
- `limitations`
- `open_questions`
- `objects`: figures, tables, equations, each `{kind, label, page, caption,
  latex, plain_language, why_it_matters}`
- `notation`, `glossary`
- `references`: key, title, authors, year, venue, doi
- `citations`: key, page, context
- `relations`: cited key, relation, plain-language why, quote, page

### P2. Multimodal OpenRouter call with a model parameter

`AgentRequest`/`run_openrouter` currently sends one text message to one
hardcoded model. Add:

- a `model` parameter carried through the request,
- multimodal user content: text parts plus `image_url` data URLs for page
  images,
- spend recording unchanged (`src/analysis/spend.rs`).

Keep it a direct chat-completions call; do not build a tool loop for the spike.

### P3. Basic resource extraction for an arbitrary PDF

Given a PDF path, produce:

- page-marked text (page number + text; the reading index or `source.md`),
- page images (PNG) at a fixed DPI via `pdftoppm` (reuse the rasterization in
  `src/source_index/ocr.rs`).

The spike must not depend on the old KB corpus indexes. Build the page text and
images from the PDF with our own pipeline so the input matches runtime.

### P4. `lysilogy extract` command

```
lysilogy extract <paper-or-pdf> --model <openrouter-model> [--passes 1|2] [--out <path>]
```

- Resolve a library paper or accept `--pdf <path>`.
- Run P3, build the prompt with the schema, call P2.
- Pass 1 writes the JSON. Pass 2 sends the paper plus pass 1 and may only
  correct or flag, requiring a quote for each correction.
- Write the result and a run report (model, tokens, cost, latency, pass count).

### P5. Oracle front matter for identity and abstract

The LaTeX harness currently exposes objects, entries, links and coverage, not
`\title`, `\author` or `\begin{abstract}`. Add a small source-side extractor
(the harness already has the TeX reader) that returns the title, ordered
authors, and abstract for a paper. This is the oracle for the identity and
abstract metrics.

### P6. Sample selection and benchmark runner

Select and freeze the sample, run the agent, compute the metrics, emit a report.

## Sample

Twenty papers with exact object inventory from the LaTeX oracle.

Selection procedure:

1. Run `python3 -B scripts/truth/latex/corpus_audit.py --json <out>` over the
   corpus (defaults: `~/.cache/lysilogy/arxiv-kb-data/paper-identities.json`,
   its `papers/`, and `~/Corpora/arxiv/source`).
2. Keep papers with `status == "aligned"` where `kind_coverage` is `eligible`
   for `figure`, `table` and `equation` (exact counts), with a stable ordering.
3. Require 8–25 pages so vision cost stays bounded.
4. Take the first 20. If fewer than 20 qualify, relax to `inventory_known` with
   non-empty expected counts and record which metrics lose exact truth.
5. Write a frozen manifest: `eval/spike/sample.json` with, per paper, the
   identity hash, arXiv stem, PDF path (`~/Corpora/arxiv/pdf/...`), source path,
   oracle eligibility and expected counts.

Record the actual eligible-kind distribution in the report; do not silently
substitute a different cohort.

## Run procedure

1. Set the budget: `export LYSILOGY_MODEL_BUDGET_USD=5` (20 × $0.25).
2. Confirm `OPENROUTER_API_KEY` is present. The spike must not touch
   `ANTHROPIC_API_KEY`/`OPENAI_API_KEY`.
3. For each paper in the manifest: `lysilogy extract --pdf <pdf> --model
   deepseek/deepseek-v4.1-flash --passes 2 --out eval/spike/results/<stem>.json`.
4. Run the benchmark runner; it reads the result files, the oracle, and the
   spend ledger, and writes the report.

Budget: $0.25 per paper for the first model, $5 per 20-paper run. If a paper
exceeds its share, record partial output and continue; do not raise the cap
mid-run.

## Metrics and report

Report at `docs/experiment-reports/2026-10-16-extraction-spike.md`, plus raw
results under `eval/spike/`.

| Field group | Ground truth | Metric |
| --- | --- | --- |
| title, authors, year | oracle front matter (P5) | normalized exact / F1 |
| abstract | oracle abstract | normalized exact |
| figures, tables, equations | oracle inventory | count error, label/number match |
| references | oracle entries | count error, key + field accuracy |
| citations / relations | oracle links + human spot-check | key match, relation agreement |
| supercut | source text | quote validity rate (automatable), human coverage score |
| claims, method, limitations | human rubric on a subsample | rubric score |
| cost, latency | spend ledger | $/paper, seconds/paper |
| self-check | pass 1 vs pass 2 | corrected/flagged counts, accuracy delta |

Also record: tokens by pass, model calls, failures and why, and per-paper notes
on what the agent missed.

## Escalation ladder

Run the same sample at each rung; do not change the cohort.

1. `deepseek/deepseek-v4.1-flash` via OpenRouter (cheapest vision path). Start
   here.
2. Claude with plan. Wrinkle: the plan path is the `claude --print` harness,
   which is text-only unless page images are exposed. Either grant a read-only
   image tool scoped to the page-image directory (loosens the current `--tools
   ""`) or use the Claude Agent SDK, which carries native image blocks. Decide
   and record which.
3. Gemini via OpenRouter.

## Risks and wrinkles

- **Vision cost.** Full-page images dominate token cost. Prefer page text for
  everything except equations and figures, and only send images for pages with
  detected float/equation candidates.
- **Long JSON.** A whole-paper schema may push output limits. If it does, split
  into two passes (identity+abstract+card+supercut, then objects+claims+
  references+relations) and measure both.
- **Schema compliance.** `json_schema` strict mode can reject; capture the raw
  error and count it as a failure, not a silent drop.
- **Oracle limits.** Identity/abstract oracle needs P5. Relation types have no
  oracle; they are human-spot-checked.
- **Old corpus indexes.** Do not use them for the spike input; rebuild from the
  PDF with P3.

## Definition of done

- `prompts/paper-artifacts.schema.json` committed.
- `lysilogy extract` runs on one paper and on the frozen 20.
- Raw results and the dated report exist, with cost, latency, per-field
  accuracy, and pass-1 vs pass-2 delta.
- A written recommendation: adopt, escalate, or split, with the numbers behind
  it.

## Out of scope

- Graph-level aggregation (lineages, debate clusters, position across papers).
- The user-aim model, session log, and resonance pass.
- Triage signals from external providers.
- Retiring or deleting the deterministic pipeline; that follows the decision.
