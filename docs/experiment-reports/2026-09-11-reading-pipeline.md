# Reading pipeline implementation and validation — 2026-09-11

The three core feature phases are implemented. This report records software validation and one
cached-source extraction pilot. It does **not** establish a winner in a model-quality experiment:
live model access was blocked, and no blind, repeated context comparison was completed.

## Delivered modules

| Phase | Implementation | Independent boundary |
| --- | --- | --- |
| Abstract | `src/abstracts/`, dedicated repair/boundary schemas, `abstract.json`, selective CLI/API/UI refresh | Verifier checks original source lines without invoking the locator; a fresh model can adjudicate uncertain boundaries but cannot waive mismatched or contaminated source text |
| Historical context | `src/analysis/context/`, separate evidence/writing/review schemas and caches, before/after roles, assessment artifact and UI | Writer uses frozen evidence; separate review inspects support, passages, chronology, and usefulness; public URL checks remain a separate application gate |
| Section reading | `sectionScope`, `usePdfDocument`, `SectionFocus`, page snapshot transitions | Page scope derives from validated anchors or a bounded legacy range; PDF source and native selection remain independent of generated digest text |

Abstract normalization permits whitespace, common ligatures, and soft-hyphen cleanup. It preserves
case and scientific hyphens; it does not accept paraphrase as authored text. Structured abstract
subheadings remain included. Target-title identity, repeated running matter, conventional section
headings, and layout gaps help locate labeled and unlabeled candidates. Unknown reading order or
boundaries remain reviewable rather than silently accepted. A previous accepted result can survive
a failed refresh only when its source fingerprint and provenance checks remain valid.

Context defaults to Terra medium-effort evidence gathering, Astra high-effort writing, and Terra
high-effort independent review for Codex. `LYSILOGY_CONTEXT_MODEL` configures the writer separately.
Claude retains its configured model with the same effort/tool routing. The reviewer receives no
writer reasoning. Published claims require mapped source excerpts/locations and complete positive
review, then successful URL checks. Missing reviews, partial support, chronology failures, and
generic or duplicated claims are withheld. Metrics expose numerators, denominators, and unknowns;
they describe model-assessed support rather than ground truth.

Abstract/context refreshes use the existing per-paper job reservation and write the analysis and
digest projections without touching saved highlights. Context-only process failure preserves the
previous analysis. Initial full analysis can save a useful section map with an explicit context
research gap. Stage keys include source/prompt, schema, provider, profile, and effective model;
force refresh bypasses matching cache entries.

The focused reader shows only the selected section's original pages in a continuous left column,
with independent digest scrolling on the right. It retains a whole-paper locator, native text
selection, citation saving, clarification, source zoom and inversion, and navigation into the full
PDF. Closing restores map columns, scroll, focus, and the prior library state. Shared PDF loading,
lazy page rendering, cancellable canvas-snapshot transitions, mobile tabs, and reduced motion are
implemented. The full Text reader retains single-page and two-page spread modes.

## Checks completed

| Check | Result and scope |
| --- | --- |
| Rust formatting and all-target Clippy with warnings denied | Passed |
| `cargo test --all-targets` | 86 tests passed |
| `cargo build --release` | Passed |
| Frontend typecheck, lint, and production build | Passed |
| `npm run test:section-scope` | 2 tests passed: anchor/legacy scope, boundary inclusion, invalid and out-of-document ranges |
| `npm run smoke:pipeline` | Passed using a synthetic four-page PDF and API fixtures |
| `npm run smoke:reader-tools` | Passed using self-contained browser fixtures |
| Desktop and narrow-screen visual inspection | Focused source/digest layout and mobile tabs inspected |

Rust coverage includes labeled, inline, structured, page-spanning, adjacent-article, unlabeled,
missing, and OCR-needed abstract cases; rejection of truncation, contamination, and invented text;
independent boundary admission; context mapping, review, and support failures; stage cache reuse
and invalidation; and preservation of saved highlight bytes during component refresh.

A stub CLI verifies that context research precedes writing, writing precedes review, only research
and review get web tools, the stronger writer profile is applied, matching stages are reused,
and failed semantic review withholds a claim. These checks verify orchestration and admission,
not actual research quality.

The pipeline browser suite checks distinct cited historical cards, selective refresh requests,
scoped pages and original numbers, one shared PDF fetch, native selection controls, source keyboard
scrolling without digest scrolling, region changes, map restoration, navigation to the current
page in full Text, two-page spreads, mobile panes, and reduced motion. Focus restoration is checked
after its scheduled animation frame. Existing reader-tools smoke covers both cut formats, print
layout, export, references, linking, search/comparison, retries, and keyboard/mobile behavior.

The corpus-based `npm run smoke` was not run because its default fixture needs an original
reading-library PDF. No protected library directory was opened for these checks.

## Cached-source Goodhart pilot

The existing workspace extraction for *Categorizing Variants of Goodhart's Law* contains an
unlabeled opening abstract. Using that cached `source.txt`, layout, and metadata, the locator found
the 180-word block ending immediately before “Varieties of Goodhart-like Phenomena.” The candidate
matched the complete cached source block exactly after whitespace splitting. This is provenance
agreement against cached extraction, not an independent PDF/OCR accuracy score.

The result remains `needs_review`, with `start_boundary_unconfirmed`. The independent model review
could not run: the sandbox network proxy denied the Codex endpoint `ab.chatgpt.com`. A fresh arXiv
PDF download was also denied by the network allowlist, so this pilot used the existing extraction
and did not read the original library PDF. No accepted live-model abstract or historical context
result is claimed.

## Remaining evaluation and extensions

- A held-out, human-reviewed abstract corpus with exact-match, token precision/recall, boundary,
  contamination, and abstention measurements remains to be assembled and evaluated.
- General two-column reconstruction, a detailed typographic repair log, and OCR are outside this
  implementation. Existing raw extraction and layout hints are the current inputs.
- A frozen-dossier, blind comparison across multiple papers and repeated generations is still
  needed to measure writing usefulness, retrieval quality, semantic fidelity, cost, and latency.
  The stronger context default is a configuration choice, not an empirically selected winner.
- Context assessment stores passage, chronology, and usefulness verdicts; the broader plan's
  separate numerical prose scorecard, bibliometric provider, and event-date data model are not
  implemented. No citation-count-based influence ranking is presented.
- The job tracker shows the context branch and component refresh progress, not separate live
  research/writer/reviewer subtask states. Changed input invalidates dependent stage caches on the
  next context run; abstract-only refresh deliberately preserves the current displayed context.
- Scoped pages render near the viewport but remain mounted after rendering. Broader long-document
  memory and device usability measurements remain follow-up work.

The original sequencing and extended acceptance criteria are retained in the
[reading pipeline plan](../reading-pipeline-plan.md).
