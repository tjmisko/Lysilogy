# Coherent reading units — 2026-09-11

The structural prompt now asks for topic-based reading sessions: 1–5 occupied pages, usually
1–2 pages, with parent introductions, variants, examples, and qualifications kept together.
The ten-page Goodhart paper receives an initial planning budget of roughly 5–7 substantive units.
The model must adjust that guidance for actual topics and preserve source coverage.

## Baseline from the saved Goodhart map

The previous map has 15 regions, including references. Verified token endpoints give these
approximate occupied lengths; they measure reading progress within each PDF page, not physical
ink area or reading time:

| Region | Occupied pages |
| --- | ---: |
| Extremal Goodhart | 0.51 |
| Extremal Goodhart — Model Insufficiency | 0.53 |
| Extremal Goodhart — Change in Regime | 0.66 |
| Combined topic | 1.70 |

That family is now an explicit positive example of one region titled **Extremal Goodhart**, with
the internal distinctions retained in its digest. Other topics follow the same policy rather than
a hard-coded merge for this paper.

## Implementation and checks

The shared prompt applies to initial sectioning and boundary-changing revisions. A deterministic
diagnostic checks repeated parent/variant titles, excessive section counts, a prevalence of short
units, and units over about five pages. Verified token endpoints expose small fragments that happen
to cross a page break. Missing anchors remain unknown rather than receiving invented size precision.
At most one additional model pass consolidates the draft using the original source and diagnostics.
The initial and final reports are saved in `sectioning-report.json`; remaining warnings are advisory.

`refresh-structure` regenerates only the structural projection and AI evidence marks. Tests verify
that abstract/orientation/history fields survive replacement, invalid structure leaves the previous
analysis intact, fragmented output triggers one consolidation call, coherent output needs one call,
and matching versus changed prompt inputs reuse or invalidate the structural cache correctly.

Validation passed: 91 Rust tests; all-target Clippy with warnings denied; frontend typecheck, lint,
production build, and the pipeline browser smoke suite; release backend build. The browser check
includes the new `:refresh-structure` command and its component endpoint request.

## Live evaluation status

The current sandbox still denies `ab.chatgpt.com`, including a fresh connectivity check during this
change. No live result from the revised sectioning prompt is claimed, and the saved Goodhart map
remains the baseline until it is refreshed. Run from an environment with model access:

```sh
./target/release/lysilogy refresh-structure af74f1de6f1d51f4 --provider codex --force
```

Inspect the resulting map for a single Extremal Goodhart region, source coverage, accurate combined
boundaries, and a digest that distinguishes model insufficiency from regime change. Then inspect
the other groups for independent topic coherence; a smaller section count alone is not success.
