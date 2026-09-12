# Local source reading index

`GET /api/papers/{id}/reading-index` builds an index from the library PDF without running an
analysis or model. `?refresh=true` rebuilds it and resumes OCR beyond an earlier processing
budget; already completed OCR pages are reused. The source PDF, saved highlights, citation
anchors, and analysis are not modified.

## Background work and cache lifetime

Opening a full PDF or a focused section schedules indexing after the PDF has loaded, using
browser idle time (at most a 1.5-second scheduling delay, or a 500 ms fallback). Home-page
thumbnails do not trigger it. Warming does not open search controls or display errors; an
explicit search retries a failed warmup. Searching while warming shares the same request.

The server coalesces builds per paper and runs them independently of the HTTP connection.
Leaving the reader or closing the tab does not cancel a started build. Pending background
jobs yield to foreground extraction at the shared extraction lock. When available, `/usr/bin/nice`
lowers the priority of background Poppler and Tesseract children by 10. Running work is not
preempted; the existing subprocess and build limits still apply. Successful cache reads bypass
the extraction lock, and completed jobs release their in-memory server results.

Disk indexes have **no time-based expiry**. They are reused until the PDF's size/modification
time or the index schema changes, or a refresh is requested. Per-page OCR caches also survive
restarts. A source change detected during a build prevents publishing that build's result.

HTTP responses use `Cache-Control: private, max-age=2592000, must-revalidate` (30 days) and an
ETag. Each reader opening conditionally validates its index, so unchanged PDFs return 304
without transferring the body; source/schema changes and explicit refreshes produce a new
validator. The browser can retain the response across reloads. Within a tab, a shared LRU holds
up to 16 parsed indexes and approximately 64 MiB; oversized results remain usable by their
current reader without being retained in the shared cache. Navigation does not cancel shared
requests. Their five-minute deadline frees stalled client requests without canceling a detached
server build, whose eventual result remains on disk.

Both priorities use the same URL. The reader sends `X-Reading-Priority: background` for warmup
and `interactive` for a newly requested search; the optional `?priority=background` query is
also supported. Priority affects scheduling, not the representation, so responses do not vary
their HTTP cache key on that header. A new interactive HTTP request can promote an already
queued same-paper job; readers sharing an existing warmup simply await that request.

## Source representation

The response contains canonical `text`, ordered `tokens`, `pages`, text `objects` (`word`,
`WORD`, `sentence`, `paragraph`), `figures`, and explicit `gaps`. All ranges are half-open UTF-16
offsets into `text`, matching JavaScript string indexing. Every token retains its original
page, PDF-point rectangles, and `native` or `ocr` provenance. Page dimensions use the same
coordinate system. OCR page confidence is the mean score of retained Tesseract words;
native pages have no invented confidence score.

Poppler's literal word boundaries and block/line order provide the native layout. The index
normalizes common ligatures, soft hyphens, and line-wrap hyphenation, keeping coordinate maps
for both pieces of a joined word. Common scientific compound prefixes retain their hyphens.
Paragraphs use blocks, local column margins and line spacing, first-line indentation, and column discontinuities.
This recognizes small recurring indents even when Poppler places several paragraphs in one block,
while preserving hanging definitions and double-spaced prose. Line-leading hyphens and en/em dashes
continue unfinished prose when the surrounding layout agrees; list lead-ins, repeated bullets, and
hanging list items stay separate. Index schema 3 invalidates older cached boundaries automatically;
it keeps the same response fields. Headings, captions,
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
Cache/job tests also cover conditional HTTP responses, refresh validators, legacy cache reuse,
canceled clients, shared builds, queue priority, and cached reads while extraction is busy.
`npm run test:reading-index-cache` checks client deduplication, bounded memory, validators, and
failure recovery; `npm run smoke:source-search` exercises quiet warming, leaving and returning
during a build, conditional reuse on a later visit, and retry after background failure.

## Proposed boundary audit

An agent-audited Markdown view would be useful for ambiguous layouts, but is not implemented by
this deterministic index. A freely rewritten transcription cannot safely drive PDF selection:
even a small omission or paraphrase would break word-to-coordinate alignment. The existing
Markdown reconstruction also has no exact token mapping.

The proposed audit should accept the immutable source tokens, page images, and candidate paragraph
boundaries, and return only split/join proposals expressed as token IDs. Each proposal should carry
the source/index revision and evidence for the boundary. A separate validator must reject reordered,
overlapping, missing, or invented tokens and require every accepted paragraph to reference an exact
contiguous source span. Render Markdown from those validated spans, preserving bidirectional
token-to-Markdown mappings. Uncertain boundaries should retain the deterministic result and an
explicit unresolved status. This keeps yanking faithful to the PDF while allowing an agent to improve
paragraph membership without rewriting the authors' words.
