# Extraction-agent spike, rung 1: deepseek-v4.1-flash

Status: 2026-10-16. Work order:
[../tasks/HANDOFF-extraction-spike.md](../tasks/HANDOFF-extraction-spike.md).
Raw records, ledger and metrics: `eval/spike/runs/deepseek-v4.1-flash-2026-10-16/`.

## Summary

One cheap model pass, a self-check pass, and application-side admission
filled the paper-artifacts schema on 19 of 20 papers for **$1.47 total
(7.3¢ per paper, max 10.8¢) and about 4 minutes per paper**. Identity,
abstract, figures, tables, algorithms and bibliography counts are at or near
perfect on every completed paper. Numbered equations are the one weak field
(61% recall), and the loss is confined to the three most equation-dense papers,
where a single whole-paper JSON answer runs out of room. One of the three hit the output limit
outright and is the only failed paper.

Recommendation: **split, don't escalate.** Adopt the agent path for the
fields it wins, and move equations (and the object inventory generally) into
their own chunked pass before re-running the same cohort. Escalating the model
does not address the failure, which is output length, not reading accuracy:
where the model listed an equation it was right 98.5% of the time.

## What ran

- `lysilogy extract --pdf <pdf> --model deepseek/deepseek-v4.1-flash --passes 2`
  per paper, 4 at a time, `LYSILOGY_MODEL_BUDGET_USD=5`, 25¢ per-paper cap.
- Input: page-marked text from our own `pdftotext` pipeline plus a 120 dpi
  colour PNG of every page. Nothing from the old corpus indexes.
- Pass 1 fills the schema. Pass 2 sees the draft and the paper and may only
  correct or flag; each correction needs a quote that is in the source
  (equation corrections may rest on the page image and are counted apart).
- Admission: every quote is aligned against the page text; verbatim is
  admitted, a close match is replaced with the source span, the rest go to one
  bounded repair call, then are removed.

## Deviations from the work order

1. **Cohort.** The handoff's selection rule (exact harness inventory for
   figure, table and equation) and its relaxation (`inventory_known`) select
   **0** of the 725 aligned papers: every source trips the harness's
   unknown-macro guard (`\email`, `\color`, `\argmin`, `\fi`, `cases`, ...).
   The sample is instead the first 20 by stem that are aligned, 8–25 pages,
   and have a complete front-matter oracle (`eval/spike/sample.json`).
2. **Object truth is hand-verified**, not harness-certified: all 278 pages
   were checked against a text-layer draft (`eval/spike/inventory/`). Totals:
   107 figures, 44 tables, 3 algorithms, 745 numbered equations. The
   uncertified source-parser counts agree fully on 16 of 20 papers.
   Adjudicating every model/truth page disagreement against the text layer
   found 11 errors in my hand truth (misread sheet positions); the model was
   right in each, and the files record the corrections.
3. **Reference truth** is the source parser's bibliography count, which is
   uncertified; only counts are compared, not fields.
4. **Year** has no oracle and is not scored.

## Results (final = after self-check and admission)

| Field | Truth | Pass 1 | Final |
| --- | --- | --- | --- |
| Title, normalized exact | front matter | 19/20 | 19/20 (19/19 completed) |
| Authors, ordered exact | front matter | 18/20 | 19/20 (19/19 completed) |
| Abstract, word F1 ≥ 0.95 | front matter | 19/20 | 19/20 (19/19 completed) |
| Figures, label recall / precision | inventory | 98.1% / 98.1% | **100% / 100%** |
| Tables, label recall / precision | inventory | 97.7% / 100% | 97.7% / 100% |
| Algorithms, label recall | inventory | 1/3 | 3/3 |
| Equations, label recall / precision | inventory | 57.6% / 92.3% | **61.5% / 98.5%** |
| Object pages correct (matched objects) | inventory | — | 610/611 |
| Exact object count per paper | inventory | fig 20/20, tab 19/20, eq 17/20 | same |
| Bibliography count exact | source parser | — | 19/19 completed |

All identity misses are the failed paper (1912.01121, see below). The one
missed table is the appendix "TABLE 1" in 1908.00951, a boxed algorithm
captioned as a table.

### Where equations are lost

On 17 papers equation recall is 100%. The other three:

| Paper | Truth | Listed | Cause |
| --- | --- | --- | --- |
| 1912.01121 | 122 | 0 | pass 1 output cut off at 131,072 tokens; paper failed |
| 1909.06187 | 157 | 22 | model stopped listing; no length stop |
| 1803.07954 | 59 | 29 | model grouped blocks as single objects (`24-28`, `31-34`, ...) |

All three are a whole-paper answer running out of room, the "long JSON"
risk the handoff anticipated. The mitigation it names (split objects into
their own pass) is the next step, ideally chunked by page range.

### Quotes

Pass 1 raw, over 2,005 quote sites: 1,833 verbatim (91.4%), 103 near, 69
missing, 29 on the wrong page. After admission on completed papers:

| Kind | Verbatim | Repaired locally | Repaired by model | Withdrawn | Dropped |
| --- | --- | --- | --- | --- | --- |
| evidence | 801 | 48 | 14 | 4 | 23 |
| citation context | 529 | 30 | 3 | 1 | 9 |
| supercut source | 192 | 4 | 3 | 0 | 2 |
| relation | 163 | 5 | 3 | 1 | 1 |
| caption | 135 | 13 | 2 | 1 | 1 |
| abstract | 14 | 5 | 0 | 0 | 0 |

Every admitted quote is verbatim by construction. 96.7% of supercut source
segments were verbatim as written; 1% were removed.

### Self-check (pass 2)

134 corrections: 71 applied on text evidence, 41 applied on image evidence
(equations), 19 rejected (evidence not in the source), 3 flags. Measured
gains: figures 105→107, algorithms 1→3, matched equations 429→458 with
precision 92.3%→98.5%, one author list fixed. It costs 42% of the spend
($0.61 of $1.47), almost all reasoning tokens (30.6k of 31.5k output per
call). On this evidence it pays for itself on objects; on identity it
changed one paper.

### Cost and latency

| Stage | Calls | Mean in | Mean out (reasoning) | Mean latency | Spend |
| --- | --- | --- | --- | --- | --- |
| pass 1 | 20 | 25.7k | 37.5k (14.6k) | 87 s | $0.68 |
| pass 2 | 19 | 39.0k | 31.5k (30.6k) | 86 s | $0.61 |
| repair | 17 | 5.6k | 12.5k (12.1k) | 73 s | $0.17 |

Every call's cost came from OpenRouter's reported `usage.cost`. Wall time per
paper: mean 233 s, max 354 s. Page images are cheap at this model's prices;
output tokens dominate.

## Decision gates

1. **Per-field accuracy:** above for automatic fields. Claims, method,
   limitations, supercut coverage and relation agreement await the human
   rubric (`rubric.md` in the run directory, 5 papers).
2. **Cost and time:** 7.3¢ and about 4 minutes per paper, well inside the 25¢ cap.
3. **Self-check value:** yes for objects, marginal for identity (see above).
4. **Versus the deterministic pipeline, per dollar: not yet measured.** It
   needs the deterministic stages run on these 20 PDFs and scored with the
   same scorer.

Against the suggested starting bar: identity and abstract ≥95% — met
(19/19 completed, 19/20 overall). Object counts and labels ≥90% — met for
figures, tables and algorithms; not met for equations (61.5% recall). Supercut
quotes verbatim — met after admission. Fabricated citations — not directly
measured; bibliography counts match the source parser exactly on 19/19.

## Observations for the product

- The read signal is uncalibrated: 14 of 19 papers came back `read` despite
  the prompt saying most papers are not. It needs a reference distribution or
  a comparative judgment, not a per-paper absolute call.
- Type labels: 10 method, 7 theory, 2 empirical.
- Average per paper: 14.5 supercut segments, 6.9 claims, 9 relations,
  29.6 citations, 9.8 notation entries, 8.2 glossary terms.

## Next

1. Grade the rubric (5 papers) to close gate 1.
2. Split the object inventory into a chunked pass (page ranges) and re-run
   the same cohort at rung 1; re-score equations.
3. Run and score the deterministic pipeline on the same 20 PDFs (gate 4).
4. Only then decide on escalation (Claude with plan, Gemini) for any field
   still below the bar.
