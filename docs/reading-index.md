# Local source reading index

`GET /api/papers/{id}/reading-index` builds an index from the library PDF without running an
analysis or model. `?refresh=true` rebuilds it and resumes OCR beyond an earlier processing
budget; already completed OCR pages are reused. The source PDF, saved highlights, citation
anchors, and analysis are not modified.

The response contains canonical `text`, ordered `tokens`, `pages`, text `objects` (`word`,
`WORD`, `sentence`, `paragraph`), `figures`, and explicit `gaps`. All ranges are half-open UTF-16
offsets into `text`, matching JavaScript string indexing. Every token retains its original
page, PDF-point rectangles, and `native` or `ocr` provenance. Page dimensions use the same
coordinate system. OCR page confidence is the mean score of retained Tesseract words;
native pages have no invented confidence score.

Poppler's literal word boundaries and block/line order provide the native layout. The index
normalizes common ligatures, soft hyphens, and line-wrap hyphenation, keeping coordinate maps
for both pieces of a joined word. Common scientific compound prefixes retain their hyphens.
Paragraphs use blocks, line gaps, indentation, and column discontinuities. Headings, captions,
lists, and small bottom-page footnotes stay separate; folios and rotated repository stamps
are excluded. Clear lowercase prose continuations across a page break can share a paragraph,
but a footnote/header boundary prevents an unsafe join. Unicode word/sentence boundaries and
Vim punctuation groups distinguish `iw` from `iW`.

Pages with sparse or unusable native text receive a local fallback via `pdftoppm` and
`tesseract -l eng --psm 3`. The rendered image has a maximum dimension of 1800 pixels. OCR
words below confidence 25 are discarded, and mean confidence below 0.75 is flagged. OCR
replaces a page's unusable native layer only when it recovers more words, keeping provenance
explicit. Image-only and mixed native/scanned PDFs are supported. Missing tools, unreadable
pages, confidence limitations, and exhausted budgets appear in `gaps`.

Resource limits: source PDF 512 MiB, native extraction 400 pages, canonical text 4 MiB,
subprocess output 48 MiB for native layout / 8 MiB for OCR, 35 seconds per subprocess,
12 newly OCR-processed pages and a 100-second OCR launch budget per build, and 180 seconds
for a complete build. Subprocesses are killed when their future is dropped. One extraction
lock limits concurrent index/extraction work. Index and per-page OCR caches are keyed by
source size and modification time; index schema changes also invalidate the index cache.

Figure metadata matches numbered figure mentions to caption candidates, including references
from other pages. References retain their own source rectangles. A figure's `rect` is a
candidate region above its caption inferred from neighboring prose, or `null` when ambiguous.
This is not verified figure segmentation: the UI must identify candidates and retain access
to the original page. Captions without conventional numbered labels, unlabeled subfigures,
floating figures with ambiguous boundaries, equations, unusual reading order, handwritten
text, and languages outside the installed English OCR model remain limitations. Geometry
and text-object heuristics are evidence for navigation, not independent quotation validation.

Tests cover a real paper abstract as one complete paragraph, columns/indentation/footnotes,
page continuation, ligature and UTF-16 offset fidelity, figure mentions across pages,
OCR confidence/coordinate conversion, an actual image-only PDF and mixed PDF, bounded
subprocess output, and cache invalidation when the PDF changes.
