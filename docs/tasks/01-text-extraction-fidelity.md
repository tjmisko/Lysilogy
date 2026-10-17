# 01 · Text extraction fidelity

Status: proposed, 2026-10-16. Priority P0.

## Problem

The whole product rests on extracted text being faithful, but extraction loss is
currently invisible. Every stage — abstract, sectioning, bibliography, labels,
reader highlights — trusts `source.txt`, `layout.json` and the reading index as
if they were the paper. When extraction mangles or drops text, the failure shows
up later as a wrong quote, a missing reference, a broken section boundary, or an
untitled Work, and nothing warns the user.

The user's requirement: **we should only ever fail if the source is truly
unreadable.** That means lossy but readable inputs — bad scans, warped pages,
two-column layouts, unusual punctuation — must be detected, reported and, where
possible, repaired with evidence.

## What can go wrong

Known and suspected mangling classes, by pipeline stage:

| Stage | Failure mode | Where |
| --- | --- | --- |
| Native extraction | Split letters (`W ord`), line-end hyphen misjoins, ligatures, soft hyphens, wrong reading order in columns/rotated text | [`layout.rs`](../../src/layout.rs) token repair, [`frontmatter::readable_pages`](../../src/frontmatter.rs) |
| Page reconstruction | Body/float interleaving, tables and captions spliced into prose, footnotes in the wrong place, dropped pages | [`extract.rs`](../../src/extract.rs), [`source_index/paragraphs.rs`](../../src/source_index/paragraphs.rs) |
| OCR fallback | Low-confidence words kept or dropped, warped text, non-English text, only 12 new pages per build | [`source_index/ocr.rs`](../../src/source_index/ocr.rs) |
| Sections | sampled structure text loses middle passages; heading detection misses unusual headings | [`prefetch.rs`](../../src/analysis/prefetch.rs) |
| Abstract | wrong boundary, body leakage, missing unlabeled abstract | [`abstracts/locate.rs`](../../src/abstracts/locate.rs), [`abstracts/verify.rs`](../../src/abstracts/verify.rs) |
| Bibliography | reference list not found, entries split mid-title, whole body treated as bibliography, venue parsed as title/authors; PDFs with no reference list at all | [`bibliography/`](../../crates/lysilogy-ingest/src/bibliography/) — see [02](02-bibliography-integrity.md) |
| Metadata | printed title/subtitle/authors/year not found; subtitle dropped; author order corrupted by multi-column grids | [`frontmatter.rs`](../../crates/lysilogy-ingest/src/front_matter.rs), field passes — see [03](03-work-identity-and-hydration.md) |

This list is a starting hypothesis, not a complete inventory. The first job of
this task is to produce the real inventory from evidence.

## Evidence that the problem is real

- A `local_bibliography` source record in the dev database holds 53 "entries",
  two of them longer than 1,500 characters, containing method and experiment
  prose — the reference list was never found, and the paper body was indexed as
  a bibliography.
- The same record splits "Preference-based policy learning. Machine" from
  "learning and knowledge discovery in databases, pages 12–27, 2011.", which
  creates a second, mistitled citation and an untitled Work.
- 8 of the 362 untitled Works come from these mis-split entries; their author
  claims contain the venue (`["Conference on Learning Representations"]`,
  `["Information Processing Systems", "pages 4565–4573"]`).
- The reading index documents its own limits: two-column reconstruction,
  unlabeled subfigures, equations, handwritten text, and non-English OCR are not
  solved ([reading index](../reading-index.md)).

## Goal

Extraction is adversarially audited, loss is measured and visible, and a bounded
model repair pass can fix mangled text without ever inventing or rewriting the
authors' words.

## Scope

1. **Adversarial audit corpus and harness.**
   - Collect large, old, two-column, scanned, warped, rotated, ligature-heavy,
     unusual-script and reference-heavy papers, plus synthetic pages with known
     text.
   - Run the deterministic pipeline and a strong model reader over the same
     pages; diff by category (dropped words, merged/split paragraphs, wrong
     order, bad OCR, missed headings, missed references, missed metadata).
   - Emit a per-paper extraction report with counts and locations.

2. **Mangle detection in production.**
   - Cheap, deterministic signals: empty or sparse pages, low OCR confidence,
     abnormal token geometry, paragraph/page ratios, unmatched bibliography,
     missing printed metadata on page 1, reference heading not found.
   - Persist a per-paper extraction status (`clean | degraded | unreadable`) with
     reasons; the UI must show it. A degraded paper should not silently look
     like a clean one.

3. **Bounded repair agent.**
   - A model reads the page text or image and proposes **token-ID-level** edits
     only: split, join, reorder within a line, repair a hyphen/ligature, relabel
     a heading boundary. No free-text rewrite, no new tokens.
   - A deterministic validator rejects proposals that add, drop, reorder or
     overlap tokens, or that cannot be tied to the source revision. Uncertain or
     rejected proposals keep the deterministic result and are recorded.
   - Applies to paragraphs, headings, reference-list boundaries and front
     matter. This generalizes the boundary-audit sketch in
     [reading index → Proposed boundary audit](../reading-index.md).

4. **Metadata completeness.**
   - Title, subtitle, year and every listed author must be extracted when
     printed in the paper.
   - Fallback chain: deterministic front matter → bounded model read of the
     first pages → external registry by DOI/arXiv/OpenAlex → explicit unknown
     state. Never an untitled Work when the title is printed.

## Non-goals

- Replacing Poppler or Tesseract.
- Freely rewritten Markdown or paraphrased text.
- Semantic interpretation of the paper; that is the analysis pipeline.

## Acceptance

- An audit report over the corpus lists every loss class with counts; each class
  is fixed, detected, or explicitly declared unrecoverable.
- Degraded and unreadable extractions are visible; no paper with a mangled
  bibliography or missing printed metadata is shown as fully mapped.
- Repair proposals are token-anchored and validated; a poisoned proposal cannot
  change source text.
- A rerun on an unchanged PDF is a no-op; a changed PDF invalidates the derived
  artifacts.

## Open questions

1. Which model reads pages for repair, and does it need page images or is text
   enough? What is the per-paper budget?
2. How do we validate a repair when there is no ground truth — cross-model
   agreement, raster round-trip, or checksum of unchanged tokens only?
3. Do we keep two extraction variants (raw vs repaired) and show the reader
   which one an anchor points into?
4. How much of the audit corpus can live in-repo versus a private fixture set?
