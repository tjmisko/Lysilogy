# Lysilogos architecture

## Shape of the system

```text
PDF vault
   │ recursive, read-only discovery
   ▼
Paper catalog ──► Poppler extraction ──► source.txt + extraction.json
                                            │
                         ┌──────────────────┼──────────────────┐
                         ▼                  ▼                  ▼
                    codex exec          claude -p        heuristic
                         └──────────────────┼──────────────────┘
                                            ▼
                                  validated AnalysisDraft
                                            │ normalize IDs,
                                            │ pages, dimensions
                                            ▼
                                  analysis.json + digest.md
                                            │
                                            ▼
                                     Axum JSON API
                                            │
                                            ▼
                           React atlas / digest / Gloss / PDF
```

The Rust backend owns discovery, extraction, subprocess isolation, validation, persistence, and state transitions. React owns interaction and presentation. Neither frontend code nor a model process receives an arbitrary filesystem path from the browser.

## Domain model

The public model lives in `src/domain.rs`. Its enums keep states and semantic distinctions explicit:

- `ProcessingStatus` is a tagged state machine from discovery through ready or a structured failure.
- `ProcessingStage` identifies where a fault occurred.
- `AnalysisProvider` makes provenance visible throughout the API and interface.
- `SectionKind` describes conventional paper structure; `SectionFamily` groups those kinds into the atlas color language.
- `QuoteSignificance` and `EvidenceStrength` prevent key quotations and claims from becoming untyped strings.
- `PageSpan::normalized` and analysis normalization keep model output inside the actual document.

The local CLI produces an internal `AnalysisDraft`, not a persisted `PaperAnalysis`. Normalization supplies stable section IDs, repairs cross-references, clamps tile sizes, validates page numbers, and records provider/time metadata before anything is stored.

## Analysis boundary

Model-backed analysis is deliberately a subprocess boundary:

1. Extracted UTF-8 text is written to the paper's artifact directory.
2. A JSON Schema is written beside it.
3. The analyzer starts in that directory with read-only or plan permissions.
4. The prompt points at `source.txt`; the PDF text itself is marked as untrusted data.
5. Standard input carries the prompt, standard output carries structured JSON, and standard error is bounded before reporting.
6. A 20-minute timeout kills abandoned work.
7. JSON is parsed and normalized before an atomic save.

This makes the future switch to a different local agent—or an explicitly configured API adapter—an implementation detail behind `AnalysisService`.

The heuristic provider follows the same typed output path. It identifies printed headings where reliable, falls back to conceptual chunks, scores thesis-like sentences, assigns semantic families, extracts bounded quotations, and builds a small technical gloss. It never presents itself as model interpretation.

## Fault behavior

Failures are paper-local and observable. A paper moves through:

```text
discovered → queued → extracting → analyzing → ready
                  └──────────── failed(stage, message, retryable)
```

Extraction and analysis artifacts are cached independently. A failed analysis retains valid extraction; an ingest run continues to later papers and reports an aggregate failure only after preserving successful results. Schema changes invalidate stale extraction caches. Writes use a temporary sibling followed by an atomic rename.

Expected faults have dedicated errors: missing PDF tools, unreadable vaults, empty image-only extractions, failed local commands, timeout, invalid model output, duplicate processing, unsafe paper IDs, oversized extraction, and missing frontend assets.

## Reader interaction

The atlas is a CSS grid whose tile spans come from analysis rather than PDF page dimensions. That is the core product distinction: area expresses conceptual weight, while color expresses argumentative role.

Focus is the single source of truth for mouse, touch, and keyboard navigation. The digest exposes real selectable DOM text. Keyboard visual mode stores an anchor and a moving character offset over that same text, so `v`, movement, `o`, `y`, and `c` parallel Vim without breaking native browser selection.

PDF.js renders one page at a time. The default CSS filter produces light paper ink on a dark surface. `i` removes that filter, which is the reliable way to inspect figures, heatmaps, and photographs without color distortion.

## Deliberate MVP boundaries

- Scanned, image-only papers report an empty extraction instead of silently inventing OCR text. OCR is the next extraction adapter.
- Ingest is incremental but command-triggered. A filesystem watcher can later call the same refresh/queue path without changing the model.
- The PDF view renders a page rather than virtualizing a continuous document. The later native-reader experiment can reuse the API and portable artifacts.
- Analysis is single-paper and sequential during batch ingest to keep local CLI resource use predictable. Parallelism belongs behind an explicit concurrency limit.
