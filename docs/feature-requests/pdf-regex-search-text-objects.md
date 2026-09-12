# Feature request: source-anchored PDF regex search and Vim text objects

## Problem and target workflow

`/` currently opens library search while reading a paper. Reading requires searching inside the
current source and selecting meaningful text without switching to reconstructed Markdown.

Target workflow: `/optimization`, Enter, `n`, `n`, `v a p`, `y`. This searches the entire paper,
visits subsequent matches, selects the paragraph containing the active match, and copies its text.
The selected text and highlight must describe the same source passage.

## Scope and sequencing

1. **Source index:** build a reusable page/token/text-object index, preserving PDF coordinates,
   original page numbers, reading order, and source text. Normalize ligatures, line wrapping, and
   discretionary hyphenation with offset maps; preserve scientific punctuation and formula text.
   Keep columns, headings, captions, footnotes, lists, and paragraphs distinct.
2. **Search:** `/` opens an inline regex prompt in paper reading; Enter accepts; `n`/`N` navigate
   next/previous matches across the whole paper. Show count/current match, invalid-pattern and
   no-match feedback, wraparound, and a visible source highlight. Search must not freeze the UI
   on pathological regexes. Library search remains available on home/the library itself.
3. **Selection state:** normal, search, visual, and text-object/operator-pending modes must have
   explicit transitions. Search locations, pointer selections, and existing source highlights can
   establish the cursor. `v` begins visual selection, motions extend it, and `y` copies exactly the
   selected source text. Escape/`q` unwinds a local mode before leaving the reader.
4. **Text objects:** support word/WORD, sentence, and paragraph inner/around forms (`iw`/`aw`,
   `iW`/`aW`, `is`/`as`, `ip`/`ap`), including the requested `vap` workflow. Paragraphs should be
   determined from document structure, not merely PDF.js item boundaries or one visual line.
5. **OCR and preprocessing:** detect pages with missing or unusable native text. Run a bounded,
   cached local OCR fallback with coordinates and explicit provenance. Do not OCR text-rich pages
   unnecessarily or silently mix unreliable OCR with verified native quotations. Mixed image/text
   documents must be supported; report unavailable OCR and low-confidence text honestly.
6. **Integration:** reuse the same coordinates for searching, highlighting, selecting/copying,
   passage questions, and section readers. In a scoped section, an outside match must explicitly
   open the full-paper reader instead of exposing unrelated content inside the section crop.

## Acceptance criteria

- The exact target workflow works without pointer use in a paged or continuous PDF reader.
- Matches stay correct in one/two-page layouts, vertical/horizontal scrolling, fit width/height,
  zoom changes, and page navigation; the selected match becomes visible.
- Invalid patterns and zero matches are nonfatal; slow regexes have a bounded worker lifetime.
- Text objects respect multi-column order, wrapped/hyphenated paragraphs, headings, captions,
  footnotes, lists, and page breaks, with tests explaining their structural heuristics.
- Native-text, scanned, and mixed PDFs have indexed searchable text or an actionable extraction
  limitation. OCR-derived matches and clipboard text retain their page/source provenance.
- `y` copies the selected text, preserving useful paragraph boundaries without headers/folios.
  A clipboard denial leaves a manual-copy fallback.
- Typing in search, CodeMirror notes, and question inputs never triggers global reading shortcuts.
- Existing map marking, source questions, source crops, and saved citations continue to work.
- Search, index generation, and OCR have cancellation, finite document/resource limits, and caching
  keyed to the source; old results cannot replace a newly selected paper.

## Validation corpus

Use synthetic fixtures for pathological regexes and exact geometry, plus real single/two-column
papers with figures, captions, footnotes, equations, and long paragraphs. Include an image-only
page and a mixed native/scanned PDF. Browser tests must assert the copied paragraph and the actual
highlight location, not only the presence of a search box.

## Relationship to figure handling and notes

The object index should also recognize figure captions/references so a section can surface figures
located outside its crop. Notes remain a separate editable Markdown buffer; source navigation must
not consume CodeMirror keystrokes or turn OCR/model output into silently edited source text.
