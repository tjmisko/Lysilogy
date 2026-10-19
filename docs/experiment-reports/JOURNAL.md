# Extraction spike journal

Status: 2026-10-19. A running index of the spike: each experiment, the one
question it asked, what it changed, what it taught, and where the numbers live.
The linked reports carry the detail; this file is the timeline.

| # | Question | Change held | Verdict | Record |
| --- | --- | --- | --- | --- |
| [E0](#e0--core-only-barebones) | What is the baseline? | built-in core, text-only, low effort | identity done, sections the weak field | [report](2026-10-19-core-only-barebones.md) |
| [E1](#e1--input-raw-vs-markdownpriors) | Does input format matter? | raw vs markdown+priors | raw wins, and the run was capped | [report](2026-10-19-e1-input.md) |
| [E2](#e2--output-cap-and-sections-hard-eight) | Can sections pass the floor? | output cap fixed, effort/decomposition | combined@low, coverage 0.97 | `eval/spike/runs/e2-*` |

## E0 — core-only barebones

Built-in combined core pass, text-only, `--reasoning low`, `--passes 2`, 20
papers. Identity 19/20 after the fold fixes; sections 0.80 coverage, 7/20 below
95%, 8 malformed units. Cost $0.3612; verification (self-check + repair) was 61%
of spend. Showed the scorer needed correction validation and the input/effort
axes needed a controlled comparison.

## E1 — input: raw vs markdown+priors

Config-driven combined core, low effort, 20 papers. Raw beat
markdown+priors on sections (0.782 vs 0.707) and the graph, and cost less.
**Confounded**: the plan runner capped the core call at `FEATURE_MAX_TOKENS`
(24,576), the built-in pass used 65,536, and every identity miss was one of the
4 truncated calls. Provider routing was also uncontrolled. The input conclusion
is therefore still open; the cap and the routing were the real variables.

## E2 — output cap and sections (hard eight)

The cap fix gives core-field plan calls `CORE_MAX_TOKENS`, and the run is pinned
to one cheap host. Three arms on the eight lowest-coverage E0 papers:

| arm | ok | coverage | <95% | malformed | identity exact | matchable | cost |
| --- | --- | --- | --- | --- | --- | --- | --- |
| E2 low | 7 | 0.972 | 1 | 0 | 7/8 | 8/8 | $0.0164 |
| E2 medium | 7 | 0.828 | 2 | 0 | 6/8 | 7/8 | $0.0143 |
| E2 fanout | 6 | 0.893 | 4 | 0 | 8/8 | 8/8 | $0.0351 |

**Combined @ low wins.** With the output cap corrected, low effort reaches the
floor where high effort and fan-out did not, at a fraction of the cost. The
design's assumption that sections needs a dedicated high-effort call is not
supported by this sample. Caveats: coverage is `last_page / pages`, the sample
is the E0 worst eight, and E0's own row on this set is circular. Validate on all
30 before adopting.

## Cohort change

The sample grows from 20 to 30: the frozen 20 (bulk arXiv) are kept, and 10
papers from the authoritative local library `Projects/Lysilogy/local-articles`
are added (`scripts/spike/add_local_sample.py`, stems `amodei2016` through
`box1976`). The local papers have a hand-checked identity oracle and no object
truth, so object metrics skip them. Sections no longer carry `kind`/`family`:
the taxonomy was Procrustean and the segmentation is the work.

Next arms: validate E2-low on all 30, and try a hierarchical sections pass
(deterministic heading candidates, model boundaries) against the one-shot.

## Cross-cutting learnings

- The malformed `sections` bug is fixed by correction validation (8 → 0).
- Identity is effectively exact once a call completes; the residual misses are
  text-layer artifacts, not prompt problems.
- Provider routing is a first-order cost lever: the same model spans roughly
  100× in list output price across hosts, and the cheap hosts served the run
  fine. Pre-detect via `/models/{id}/endpoints` (pricing, latency, throughput,
  uptime, caching, parameter support).
- Deferred: a loose identity matcher (casefold + edit distance, authors as a
  set). See the E1 report's "Deferred" section.
