# Extraction spike: what we tried, what we kept, and why

Status: 2026-10-20. A narrative of the spike from hypothesis to the current
best run. The short index lives in [JOURNAL.md](JOURNAL.md); the run records
live under `eval/spike/runs/`. This document is the story: what we measured,
what broke, what we changed, and what we deliberately left alone.

## The hypothesis

From [../product-roadmap.md](../product-roadmap.md) and
[../tasks/HANDOFF-extraction-spike.md](../tasks/HANDOFF-extraction-spike.md):
one agent, given the extracted resources, might fill the paper data model more
accurately and cheaply than the hand-written deterministic pipeline. The spike
was built to answer that with numbers: per-field accuracy, cost per paper, and
whether a self-check pass earns its keep.

`done` was defined as: identity and abstract near-perfect, section maps that
match a hand-audited reading map, exact object counts, verbatim quotes, and a
cost low enough to run over a whole library.

## Measuring before tuning

The first decision was to make the measurement trustworthy before changing the
agent. That turned out to matter more than any prompt.

- **Frozen sample.** Twenty arXiv papers, 8–25 pages, complete front-matter
  oracle, chosen by `scripts/spike/select_sample.py`. Later grown to thirty by
  adding ten papers from the authoritative local library
  `Projects/Lysilogy/local-articles` (`scripts/spike/add_local_sample.py`), so
  the sample reflects the user's own corpus, not only a bulk arXiv slice.
- **Oracles.** Identity/abstract truth comes from the LaTeX front matter when
  available. The local ten use a hand-checked oracle produced with a stronger
  model, then second-pass audited (`eval/spike/ground-truth/`).
- **Metrics.** `scripts/spike/benchmark.py` scores every paper against the
  oracle: identity exact, abstract token-F1, section coverage and unit defects,
  graph counts, cost, cache hit rate, and per-stage reasoning. Wilson 95%
  intervals and a worst-case list keep the averages honest.

Two scoring choices were deliberate and shaped everything after:

1. **Exact versus matchable identity.** Exact is compact string equality.
   Matchable tolerates case, accents, spacing, ligatures and a low edit distance,
   and treats authors as a **set**. The distinction exists because the goal is
   confident later matching, not byte equality. It changed the story: a two-
   column author order difference counts as an exact miss but is fully matchable.
2. **Sections against the audited map.** Early on the only section metric was
   `last_page / pages`, which measures reach, not correctness. After the audited
   maps arrived (`731b85d`), `score_sections_oracle` measures per-unit page
   intersection-over-union, unit-count error, and exact boundaries. The proxy and
   the real metric disagreed more than once; the real one won.

## E0 — the baseline, and the first real bug

Run: `eval/spike/runs/barebones-low-2026-10-19/`. Report:
[2026-10-19-core-only-barebones.md](2026-10-19-core-only-barebones.md).

The built-in combined core pass, raw text, low reasoning, two passes, twenty
papers: 19/20 complete, $0.3612. Identity 19/20 and abstract 19/20, but sections
only 0.80 with **8 malformed units**. The malformed units were a self-check bug:
the model's corrections were applied without validating them against the field
schema, so a "repair" could replace a good section with a malformed one.

Fix (`17f711e`): `schema.rs` plus `corrections::apply_with_schema`, so a
self-check correction must keep the draft valid. Malformed units went 8 → 0 and
stayed there. Lesson: **make the verifier check the schema before it overwrites
anything.**

E0 also showed the cost shape: the self-check and repair were 61% of spend. The
agent was cheap at the core and expensive at the edges.

## E1 — input format, and two confounds

Runs: `eval/spike/runs/e1-raw-2026-10-19/`, `e1-markdown-2026-10-19/`. Report:
[2026-10-19-e1-input.md](2026-10-19-e1-input.md). Configs:
`eval/spike/configs/combined-raw.json`, `combined-markdown-priors.json`.

Question: does raw page text beat reconstructed Markdown with deterministic
priors? The config-driven runner (`lysilogy extract --config`) let us hold
everything else fixed and change only the input.

It looked like raw won (sections 0.782 vs 0.707), but the comparison was
confounded twice:

- The plan runner capped the core call at `FEATURE_MAX_TOKENS` (24,576) while
  the built-in pass used 65,536. Every identity miss was one of four truncated
  calls. The cap was the variable, not the input.
- The `--config` flag collided with the global app-settings `--config`, so the
  plan was never actually parsed until we fixed the scope (`eeb5b87`).

Fixes: bound plan calls by the fields they carry (`9871558`), and record the
output ceiling on each call so this is visible (`9027da2`). Lesson: **a
comparison is only as good as its confound list; when every miss is a truncation,
you are measuring the cap.**

## E2 — sections on the hard eight

Runs: `e2-low`, `e2-medium`, `e2-fanout` (2026-10-19). Configs:
`combined-raw.json`, `combined-raw-medium.json`, `fanout.json`.

With the cap fixed, three arms on the eight worst papers: combined at low effort,
combined at medium, and a per-field fan-out with sections at high effort.
Combined @ low won: coverage 0.972 and only one paper below 95%, at a fraction of
the cost. The design's assumption that sections needs a dedicated high-effort
call was **not** supported. The earlier section weakness was starvation of output
tokens, not effort.

## E3 — the thirty-paper cohort

Run: `eval/spike/runs/e3-30-2026-10-20/`. Config: `combined-raw.json`.

Twenty arXiv plus ten local papers, combined core, low effort, pinned to a cheap
host: 28/30, $0.0769. Identity 27/30 title, authors 25/30 ordered but 27/30 as a
set, abstract 24/26. Sections: coverage 0.894, audited IoU 0.728, but **+3.7
units** over the audited maps. The census was the useful part: 104 audited units
gave the first real target for section quality.

The identity misses were diagnostic, not mysterious:

- small-caps artifacts (`B ROSSARD`) and a dotless `ı` (`Kříž`) were scorer
  gaps, fixed in `fold`/`name_key` (`1159415`);
- `Tworzyd lo` was our own text extractor inserting a space at the `ł` glyph
  (`747661c`);
- the rest were author-order differences on two-column pages, which the
  set/matchable score absorbs.

## E4 — the adaptive cap we rejected

Runs: `e4-30-2026-10-20/`, `e3b-capfix-2026-10-20/`. Commit `7979856`.

Hypothesis: shorter papers need smaller output ceilings. We built
`clamp(pages × 3000 + 8192, …)`. It was **worse**: 26/30 against E3's 28, with
three new truncations on short papers. The data said why — core completion is
**not** proportional to page count. A 55-page paper used 29,683 tokens and a
4-page paper that completed used ~20k; the failures were reasoning loops that
stopped exactly at their cap. Recalibrated to a fixed 65,536 floor that scales up
only for long papers (`b9354e7`). Lesson: **a cost model based on page length was
measuring the wrong variable; reasoning is the variable.**

## E5–E7 — the provider layer

Runs: `e6-30-2026-10-20/`, `e7-30-2026-10-20/`. Script:
`scripts/spike/providers.py`. Commit `39677db`.

Two failures that had nothing to do with the model:

- Pinned to one cheap host, the whole cohort returned `429` from an upstream
  shared pool.
- Unpinned, routing landed on flaky hosts: six "no message content", two loops,
  22/30 at $1.18.

Fix: a routing policy that sorts by price, requires parameter support (so
strict JSON is respected), allows fallbacks, caps price, prefers throughput and
latency, and excludes low-bit quantization. Result: 28/30 at $1.67. The point is
not that it is cheap — the fallback is ~10× the rate-limited host — it is that
**a 429 no longer fails a paper.** The inspector shows per-host quantization and
price before spending (`python3 -B scripts/spike/providers.py`).

A scorer bug surfaced here too: exact identity must imply matchable
(`da043ff`).

## E8–E9 — decomposition does not fix loops; a retry does

Runs: `e8-fanout-30-2026-10-20/` (fan-out), `e9-combined-30-2026-10-20/`
(combined + retry). Commit `37efe47`.

We asked whether decomposition fixes reasoning loops. On the two looping papers
the per-field fan-out completed, which looked like a fix. On all thirty it moved
the loops to tiny fields — `identity`, `abstract`, `card` — and finished 25/30.
The loop follows the model, not the task size.

The fix is a bounded retry: when a call's `finish_reason` is `length`, try it
once more before failing. Loops are nondeterministic; the same paper that looped
completed on a rerun. Combined + retry: 29/30, identity 0.97, abstract 26/26.
E10 later extended the retry to provider errors and dropped answers
(`d0738ea`). Lesson: **retry is the robust primitive for stochastic failures;
decomposition changes which call fails, not whether.**

## E10 — pin the host

Run: `e10-pinned-30-2026-10-20/`.

Pinned to DeepInfra fp8, chosen from the inspector ($0.42/$0.14 per million,
100% uptime, json + reasoning). **30/30**, $0.686. Identity 29/30, and section
IoU jumped to 0.790 with coverage 0.958 — the section metric was noise from
host-hopping. The remaining defect was now clear: **over-segmentation**, 152
units against 85 audited.

## E11–E13 — granularity and the deterministic title

Runs: `e11-oneshot-30-2026-10-20/`, `e12-refine-30-2026-10-20/`,
`e13-titleprior-30-2026-10-20/`. Commit `30f793c`.

Two changes, plus one experiment that did not earn its keep.

- **Granularity as an explicit input.** `sections_guidance` now computes
  `ceil(pages/4)–ceil(pages/2)` units (each ~2–3 pages), not `pages/2` to
  `2·pages/3`, and `prompts/sectioning.md` states the merge preference. Over-
  segmentation collapsed from 152 to 91 units, IoU 0.790 → 0.866.
- **A deterministic title prior.** `likely_title` takes the largest text line on
  page 1, skipping cite-as and accepted-for-publication banners, and is stated
  authoritatively. Identity reached **30/30**, including `1803.07954v4`, whose
  first page prints both a "please cite as" title and the real heading.
- **Multi-turn refine, tested and not adopted.** A second call with
  `include_draft` re-reads its own section map and merges. It added only +0.017
  IoU (0.866 → 0.883) for ~40% more cost and no identity gain. Keep one-shot.

Best run `e13-titleprior-30-2026-10-20`: 30/30, identity 1.00, sections IoU
0.866 with 91 vs 85 units, $0.50 ($0.017/paper).

## Things we considered and did not do

- **Images and native PDF.** The pipeline can now OCR an image-only PDF and send
  page images (`7aa6d04`), and a controlled image-only set exists
  (`eval/spike/image-only.json`, `scripts/spike/build_image_only.py`). We
  measured the cost (a 120 DPI page is ~320 KB, tesseract ~3 s/page, image
  tokens cents per paper) but did not run the full image arm.
- **Effort sweep.** Only low and medium were compared; `off` and `high` were
  not swept once the cap turned out to be the real lever.
- **Model escalation.** The strong-model rung in the handoff — one strong call
  over text plus images — was never run.
- **Hierarchical sections.** Using deterministic heading candidates and having
  the model only merge/split was designed but not built; the granularity prompt
  fixed the count first.
- **Prefix-cache measurement.** Deferred by choice; the `cached_tokens` field is
  recorded and waiting.
- **Object and graph truth.** Object metrics used the hand-verified inventory
  for the original twenty; the local ten have none. Graph counts are reported but
  not scored against a reference — which is exactly where the next phase starts.

## What this says about AI engineering

1. **Measure something you trust first.** Exact versus matchable, and audited
   section IoU, changed conclusions that the raw averages got wrong.
2. **Enumerate confounds before comparing.** E1 was really about a token cap and
   a provider; the input format was almost irrelevant.
3. **Verifiers must validate their own output.** The worst bug (malformed
   sections) was the self-check overwriting good data.
4. **Stochastic failures want retries, not clever decomposition.** Decomposition
   re-homed the loop; a bounded retry removed it.
5. **The provider is part of the model.** Quality, cost, and stability all moved
   with the host, by 10× and more.
6. **Prefer deterministic priors for unambiguous fields.** An authoritative
   page-1 heading beat a prompt rule that the model ignored on one host.
7. **Give the model the number.** "About 2–3 pages per unit" fixed an
   over-segmentation that no amount of style guidance did.

## Where it stands, and what is next

The agent fills identity, abstract, sections and graph for all thirty papers at
~$0.017 each, with identity perfect and section maps close to the audited target.
The unproven half is the graph: references, citations, and relations have counts
but no reference truth. The next phase is to build that truth — near-complete
reference collection, cross-referenced against OpenAlex — and to score citation
and relation recall, not just volume.
