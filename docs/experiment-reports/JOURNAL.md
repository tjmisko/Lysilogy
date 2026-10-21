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

## E3 — 30-paper cohort with audited sections

Combined core, low effort, cap fixed, Relace, 20 arXiv + 10 local papers. 28 of
30 completed; `1908.00951v4` and `kaplan2020` still hit the 65,536 output cap.

- Identity: title 27/30 (.90), authors ordered 25/30 (.83), set 27/30 (.90),
matchable 28/30 (.93), abstract 24/26 (.92). All misses are the two cap failures
plus one title (`1803.07954v4`).
- Sections: coverage 0.894, but against the audited maps (10 local papers) the
mean unit IoU is 0.728 with a +3.7 unit-count error (122 predicted vs 85 truth)
and 0.42 exact boundaries. The model over-segments.
- Cost: $0.0769 total, $0.0026/paper.

The section oracle is the first real gradient for sections. The remaining work
is granularity calibration against the audited maps, and chunking long papers
past the cap.

The fixed 65,536 core cap is replaced by an adaptive one: roughly 3,000 tokens
per page plus 8,192 overhead, floor 16,384, ceiling 131,072. Both capped papers
completed on the rerun (`eval/spike/runs/e3b-capfix-2026-10-20/`). The cap is a
ceiling, not a spend: actual core output was ~24k tokens on each.

## E4 — adaptive cap (rejected)

The page-scaled core cap was tested on all 30 (`eval/spike/runs/e4-30-2026-10-20/`).
It is worse: 26 of 30 completed against E3's 28, with three new truncations on
short and medium papers (`dijkstra1968` at a 20,192 cap, `1912.06889v2` at
35,192, `1911.01515v4` at 65,192).

Core output is **not** proportional to page count. Successful completions sit
near 20-30k tokens regardless of length: a 55-page paper used 29,683 and a
48-page paper 29,666, while short papers that completed used 19-27k. The
failures were reasoning loops that stopped exactly at their cap, not long
records. A lower cap on a short paper only converts a loop into a failed paper.

Recalibrate: the cap should be a fixed ceiling (65,536 worked), or at most a
floor of 65,536 scaling up for long papers, never below it. Cost estimation
should use observed usage (~25-30k per core call), not the cap.

Separately, cost is not stable: E4 core produced 821k completion tokens for
$0.4457 and E3 produced 820k for $0.0327, both reported by Relace. The 13x gap
is provider pricing, not token count, and any cost model must expect that.

## E6 — fixes verified, providers are the bottleneck

The four follow-ups landed and the targeted misses are fixed: the cite-as title
on `1803.07954v4` resolved, `Tworzydło` now joins, and the 65,536 floor restored
`dijkstra1968`. But both reruns were dominated by provider failures, not
quality. Pinned to Relace the whole cohort returned 429 (upstream shared pool);
unpinned, six papers got "no message content" and two looped, finishing 22 of 30
at $1.18. Extraction is no longer the bottleneck; provider selection is.

Watch item: `autor2013` returned the running-head title `The China Syndrome`
without its subtitle, and `1908.00951v4` returned empty authors. Both completed.

## E7 — robust routing

The provider policy now sorts by price, requires parameter support, allows
fallbacks, caps price (`prompt 0.5`, `completion 2.0` USD/M), prefers 20 tok/s
and 60 s latency, and skips low-bit quantization. Relace was rate-limited, so
routing fell back to Together and finished 28 of 30 (E6: 22 of 30).

Identity is now 28/30 exact and 28/30 matchable; the only misses are the two
loop failures (`0812.5080v5`, `amodei2016`). On completed papers identity is
28/28 both ways. Section over-segmentation improved to +1.4 units (99 predicted
vs 85 truth, IoU 0.669), from +3.7 in E3.

Cost rose to $1.67 for the cohort because Together is roughly ten times Relace.
The fallback buys completion, not cheapness; the cost model should assume a
cheap host when available and a ~$1.2/M fallback when not.

## E8/E9 — loops are stochastic; retry and routing fix them

Decomposition does not fix loops. `fanout.json` on the two loopers completed,
but the full-30 fanout moved the loops to tiny fields — `identity`, `abstract`,
`card` — and finished 25 of 30. The loop follows the model, not the task size.

A bounded retry (two attempts on a length finish) plus robust routing does fix
them: the combined core finishes 29 of 30 with identity 29/30 exact and
matchable, and abstract 26/26 (1.00). The one miss (`shannon1948`) is a provider
stream error truncated mid-JSON, not a loop; the retry covers length at present,
not provider errors.

Sections are now the weak and unstable field. E7 measured IoU 0.669 with +1.4
over-segmentation; E9 measured 0.586 with -0.9 under-segmentation on different
hosts. Coverage swings with them (0.83 vs 0.70). Section quality is
provider-sensitive, and page coverage is not a stable metric.

## Cohort change

The sample grows from 20 to 30: the frozen 20 (bulk arXiv) are kept, and 10
papers from the authoritative local library `Projects/Lysilogy/local-articles`
are added (`scripts/spike/add_local_sample.py`, stems `amodei2016` through
`box1976`). The local papers have a hand-checked identity oracle and no object
truth, so object metrics skip them. Sections no longer carry `kind`/`family`:
the taxonomy was Procrustean and the segmentation is the work.

Next arms: validate E2-low on all 30, and try a hierarchical sections pass
(deterministic heading candidates, model boundaries) against the one-shot.

## Image-only inputs

One clean image-only PDF exists in the local library (Putnam 1975); Borges is
corrupt. To test the dimension, `scripts/spike/build_image_only.py` rasterizes
three local text papers at 150 DPI and the set is `eval/spike/image-only.json`
with `eval/spike/configs/image-only.json`.

Measured: a 120 DPI letter page is a 1314x1020 PNG (~320 KB); tesseract takes
~2.9 s/page and reads the scanned page well. At DeepSeek flash pricing the image
tokens are roughly $0.001-0.003/paper, so wall time, not tokens, is the cost.
The pipeline currently refuses a PDF with no text layer (`EmptyExtraction`), so
the next code step is the OCR fallback: render each page (already done for
images), run tesseract to fill `page.text`, and send `input.images` when the
vision model should see the page. Ground truth for these comes from the
strong-model kit in `eval/spike/ground-truth/`.

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
