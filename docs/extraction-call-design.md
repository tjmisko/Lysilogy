# Extraction call design and experiment program

Status: 2026-10-19. Companion to
[experiment-reports/2026-10-19-core-only-barebones.md](experiment-reports/2026-10-19-core-only-barebones.md)
and [experiment-reports/2026-10-16-extraction-spike.md](experiment-reports/2026-10-16-extraction-spike.md).

## Purpose

Decide the cheapest call structure that never gets the basics wrong. The
competitor is pasting a paper into ChatGPT/Claude's web UI: fast, cheap on a
plan, and it hands back a summary. We are allowed to be slower and to cost
more, because we offer the identity record, the reading map, the citation
graph, and the artifacts. We are not allowed to be wrong about identity,
abstract, sectioning, or references — those are the trust surface.

This document fixes the floors, the axes we can turn, the experiments that
isolate them, and the rule for choosing a configuration from the evidence.

## What the evidence already says

- One whole-paper call with images at the model default (`rung 1`) is good on
  short fields: title 19/20, authors 19/20, figures 100%, tables 98%, quotes
  91% verbatim. It fails on long fields: equations 61% (output length), and
  the read signal is uncalibrated (14/19 `read`).
- Core-only, text-only, `--reasoning low`, one call for nine fields: 1.8¢ a
  paper. Title 19/20, authors 17/20, abstract 19/20, but quotes fell to 53%
  verbatim, sections were abandoned or malformed, and one paper looped to the
  output ceiling. Two variables moved together (text-only and low reasoning),
  so that comparison is confounded.
- Input sent to the model today: raw `pdftotext` text, page-marked, no
  markdown, no deterministic structure, no images outside the object pass. The
  markdown renderer and the deterministic front-matter/abstract/structure
  extractors are unused.

## Quality floors

Accuracy, not mechanism, is the requirement. Determinism is one means; a model
check is another; the floors are what matter.

| Field | Floor | Notes |
| --- | --- | --- |
| identity (title, authors) | **100%** | a single miss is a trust failure |
| abstract | verbatim-correct on every paper with an abstract | |
| sections | **≥99%** quality, evaluated partly by human rubric | product-critical |
| references | **≥99%**: if it is printed, it enters the graph | |
| citations / relations | raise from a measured baseline | graph input |
| pass-1 quote verbatim | ≥90% | below this, repair dominates cost |
| cost / latency | minimise, after the floors are met | |

A configuration is only eligible if **every** paper meets the floor, not just
the average. We report worst-case and interval, not only the mean.

### Identity: 100% by cross-checking, not by one perfect parser

Deterministic parsing has a long, annoying tail of formats, so it will not be
100% alone. The candidate design is multi-source with disagreement handling:

1. PDF metadata title/authors/year, when present.
2. A deterministic first-page parse.
3. The model's reading of the first page.
4. Reconcile: agreement is accepted; disagreement triggers a tie-break call or
   a flag, and is counted as a defect either way.

This turns "is the model right?" into "do the sources agree?", which is
measurable and can be driven to zero on the sample.

### Sections: a fuzzy floor needs proxies

Section quality is not a single number. Clear losers: units shorter than a
page, generic model-sounding titles, and titles that do not match their unit's
content. Measured proxies:

- coverage: share of pages inside some unit (gaps are defects);
- unit length distribution (no sub-page units, none over the preferred max);
- title/content agreement, judged by a model or embedding rubric;
- title genericity: does the title merely repeat a printed heading?
- boundary accuracy against a small hand-labeled set;
- a human rubric on 5–10 papers per configuration.

### References

Count-exact against the source parser, plus per-entry presence ("if printed, in
the graph") on a hand-checked subset.

## Axes

Each axis lists the levels worth testing. Input and provider are global per
run; effort, model, and scope are per call.

**A. Input representation**
- raw `pdftotext` (today)
- markdown (`markdown::render_source`)
- markdown + deterministic priors (metadata, abstract locator, layout headings)
- text + page images
- native PDF file part, where a provider accepts it

**B. Decomposition**
- one combined call (today)
- fan-out: one call per core field
- grouped: e.g. identity+abstract, sections, references+citations+relations
- hierarchical: sections first, then per-section citations/relations

**C. Model**
- cheap (deepseek-v4.1-flash) vs strong (Claude Opus / GPT-class)
- one model for everything vs per-field tiers
- the "one-shot bundle" hypothesis: a single strong call over text+images
  beats a cheap decomposed pipeline on both quality and total cost

**D. Effort and budget**
- reasoning effort `off`/`low`/`medium`/`high`, per call
- max output tokens per call

**E. Verification**
- self-check `none`/`full`/`targeted`
- repair `none`/`local`/`model`
- validate corrections against the schema (currently not done)
- deterministic gates: identity agreement, abstract presence, section coverage

**F. Routing and caching**
- provider pinned vs not
- prefix stability, measured by `cached_tokens`

## E0 — measurement harness

Before any sweep: the scorer must produce, per paper and per field, a value
with a confidence interval, plus cost, calls, reasoning tokens, and cache hit
rate. Specifically: identity exact, abstract F1 and presence, section coverage
and unit-length defects, reference count exact, citation/relation recall,
pass-1 verbatim rate, and worst-case flags. Fixes needed first: validate
self-check corrections, and make identity scoring robust to text-layer
small-caps artifacts.

## E1–E6 — staged experiments

One axis changes per stage; hold the rest at the current best.

- **E1 input.** Combined call, effort fixed at `medium`. Raw vs markdown vs
  markdown+priors vs raw+images. Separates input from reasoning.
- **E2 effort.** Best E1 input, combined call. `off`/`low`/`medium`/`high`.
- **E3 decomposition.** Best E1/E2. Combined vs fan-out vs grouped vs
  hierarchical.
- **E4 per-field tier.** From E3, give cheap effort/model to fields that
  saturate, strong to identity and sections.
- **E5 verification.** Repair and self-check modes, correction validation,
  deterministic gates.
- **E6 model.** Cross the best cheap pipeline against a strong model, as
  combined and as fan-out, text and images. This is the "maybe Opus one-shots
  it better and cheaper" question, and it may reset E1–E5 for the strong model.

Run sweeps on a hard subset (the ~8 worst papers) and validate winners on all
20 plus a held-out set, so the sample is not overfit.

## The config data structure

Experiments are configuration files, not code edits:

```json
{
  "input":    {"format": "markdown", "priors": true, "images": false},
  "calls": [
    {"stage": "identity", "fields": ["identity"], "effort": "off",  "model": "cheap",  "scope": "front"},
    {"stage": "abstract", "fields": ["abstract"], "effort": "off",  "model": "cheap",  "scope": "first3"},
    {"stage": "sections", "fields": ["sections"], "effort": "high", "model": "strong"},
    {"stage": "graph",    "fields": ["references", "citations", "relations"], "effort": "low", "model": "cheap"}
  ],
  "verify":   {"self_check": "targeted", "repair": "local", "validate": true},
  "provider": ["DeepSeek"]
}
```

The runner executes the call list and writes the standard record, so the
existing scorer and the reasoning-trace tooling apply unchanged.

## Decision procedure

1. Keep configurations where every paper meets every floor (interval lower
   bound above the floor, worst case clean).
2. Among them, pick the lowest mean `$/paper`.
3. Tiebreak on fewest `calls/paper`, then lowest wall time, then lowest
   variance.
4. **Merge fields into one call only when each field independently saturates.**
   This is what "minimum calls" means in practice: fields that hit their floor
   on a cheap call collapse together; only fields that do not saturate get
   their own call and budget. Sections is the field we already know will not
   collapse.
5. Choose global axes (input, provider, and whether the whole pipeline is cheap
   or strong) once, from E1/E5/E6; choose per-call axes (effort, model, scope)
   per field from E2/E4.

## Open questions

- Does markdown/priors materially raise sections and identity, or is that an
  effort/model problem?
- Can a strong model one-shot the record well enough that decomposition is
  unnecessary, and is that cheaper per paper than many cheap calls?
- What is the actual citation/relation baseline to raise from?
- How much of identity's tail is PDF metadata being blank versus first-page
  layout variety?
- Does prefix caching actually fire when the provider is pinned, and how much
  does it move `$/paper`?

## Next

Build E0 (per-field metric harness) and the config-driven runner, then run E1
as a two-run probe.
