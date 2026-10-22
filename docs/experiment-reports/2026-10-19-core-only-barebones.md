# Extraction spike, core-only barebones: deepseek-v4.1-flash

Status: 2026-10-19. Preceded by
[2026-10-16-extraction-spike.md](2026-10-16-extraction-spike.md).
Raw records, ledger and metrics:
`eval/spike/runs/barebones-low-2026-10-19/`.

## Summary

The core pass alone (no optional features), text-only, `--reasoning low`,
`--passes 2`, 20 papers: **$0.3612 total, 1.8¢ per paper, max 4.5¢**. 19 of 20
papers completed. Identity holds: title 19/20, abstract 19/20, authors 17/20.

But only 39% of the spend is the core answer. The self-check is 31% and quote
repair is 30%. The headline regression is **verbatim quote discipline**: 53%
of pass-1 quotes were verbatim, against 91% in the image run.

## What ran

`lysilogy extract --pdf <pdf> --passes 2 --features "" --reasoning low
--paper-budget-usd 0.10`, 4 papers at a time. Core fields only: identity,
type, abstract, card, question, sections, references, citations, relations.

Two variables changed against the previous run: the core pass is **text-only**
(no page images) and reasoning is **low**. The two are not yet separated.

## Cost and time

| Stage | Spend | Share |
| --- | --- | --- |
| `extract.pass1` (core) | $0.142 | 39% |
| `extract.pass2` (self-check) | $0.113 | 31% |
| `extract.repair` | $0.107 | 30% |

Mean wall 314 s, max 428 s. Reasoning medians: pass 1 16.6k, self-check 20.0k,
repair 7.2k. The self-check reasons *more* than the core pass.

## Identity

| Field | Final |
| --- | --- |
| Title exact | 19/20 |
| Authors ordered exact | 17/20 (F1 0.883) |
| Abstract match (F1 ≥ .95) | 19/20 (F1 0.943) |

The author misses are text-layer artifacts, not model errors: small-caps
surnames arrive as `Martin B ROSSARD` and split ligatures as `LÜTFÜOĞLU`, which
the scorer's family-name key cannot match. The one identity failure is the
paper whose core call was cut off (1910.00671).

## Quotes

Pass 1 raw over 926 quote sites:

| Verbatim | Near | Missing | Wrong page |
| --- | --- | --- | --- |
| 491 (53.0%) | 32 | 403 | 11 |

Admission afterwards: 494 verbatim, 32 repaired locally, **272 repaired by
model**, 80 dropped, 55 withdrawn. The image run for comparison: 1661 sites,
1510 verbatim (90.9%), 77 missing, 14 repaired by model.

The repair loop is what consumes 30% of the budget, and the two `confused`
flags are both repair stages: 1910.06658 ×2.17 and 1911.01515 ×3.11 the cohort
median.

## Sections

The reading map is the weakest core field, in two ways.

- **Empty or truncated.** 1912.01121 returned no sections from pass 1.
  1809.05643 (25 pages) returned one section ending on page 3; 1903.02210
  (8 pages) returned one ending on page 2. Four papers cover under 70% of
  their pages.
- **Self-check breaks the shape.** On 1912.01121 the self-check issued
  `replace /sections` with `{title, page, summary, confidence}`, which is not
  the schema (`title, kind, family, pages, source_span, summary`). Corrections
  are applied without validation, so the final record is unusable by the app.

## Failure

1910.00671 hit the 65,536 output ceiling with only 18,410 reasoning tokens;
the raw output is hundreds of lines of tabs and newlines. A formatting loop,
not a content overflow. It is the same paper that regressed in the earlier
run.

## Coverage gaps

Citations and relations are frequently empty even with a full bibliography:
1909.01112 (21 references, 0 citations), 1911.08525 (18, 0), 2001.11673
(25, 0, 0 relations), 1912.01121 (14, 1, 0).

## Takeaways

1. **Core-only is cheap and the identity fields survive.** 1.8¢ a paper.
2. **Cleanup dominates.** Self-check plus repair is 61% of spend; only 39% is
   the answer.
3. **Quote discipline collapsed.** 53% vs 91% verbatim. Isolate text-only from
   `low` before drawing a conclusion, but the repair cost is real either way.
4. **One call splits the output budget.** The model abandons the section map
   while writing references and long citation excerpts. Asking for each core
   field on its own is the obvious remedy.
5. **Corrections need schema validation.** The self-check can write a shape
   the app cannot read.
6. **Guard against degenerate output.** Whitespace loops should fail fast, not
   consume the whole output ceiling.

## Next

A fan-out core mode: one request per core field over a shared paper prefix, so
each field has its own output budget and a provider prefix cache can serve the
repeated paper. The abstract call reads only the first three pages. Measure
against this run, including cached-token counts and the per-stage reasoning
profile.
