# Lysilogos architecture

## Shape of the system

```text
PDF vault
   │ recursive, read-only discovery
   ▼
Paper catalog ──► Poppler raw + bbox extraction ──► source.txt + source.md + layout.json
                                                        │
                         ┌──────────────────┼──────────────────┐
                         ▼                  ▼                  ▼
                    codex exec          claude -p        heuristic
                         └──────────────────┼──────────────────┘
                                            ▼
                                  validated AnalysisDraft
                                            │ normalize IDs, pages,
                                            │ dimensions, source mappings
                                            ▼
                                public source-link verifier
                                            ▼
              analysis.json + digest.md + highlights.jsonl
              analysis-tasklist.md + job/session/feedback records
                                            │
                                            ▼
                                     Axum JSON API
                                            │
                                            ▼
                 React Abstract / Overview / Glossary / Text
```

The Rust backend owns discovery, extraction, subprocess isolation, validation, persistence, and state transitions. React owns interaction and presentation. Neither frontend code nor a model process receives an arbitrary filesystem path from the browser.

## Domain model

The public model lives in `src/domain.rs`. Its enums keep states and semantic distinctions explicit:

- `ProcessingStatus` is a tagged state machine from discovery through ready or a structured failure.
- `ProcessingStage` identifies where a fault occurred.
- `AnalysisProvider` makes provenance visible throughout the API and interface.
- `SectionKind` describes conventional paper structure; `SectionFamily` groups those kinds into the Overview's color language.
- `QuoteSignificance` and `EvidenceStrength` prevent key quotations and claims from becoming untyped strings.
- `DocumentLayout`, `LayoutToken`, and `LayoutSentence` preserve PDF-point geometry with stable page-local coordinates.
- `CitationStatus` distinguishes exact, normalized, ambiguous, missing, and legacy-unverified evidence.
- `ContextNote` maps each external-context sentence to one or more source IDs; `ContextSource` preserves the exact title/authors/year record, the narrowly supported point, final checked URL, and verification time.
- `Highlight` carries a typed AI/user origin, semantic kind, immutable text anchor, PDF rectangles, note, and timestamp.
- `AnalysisJob`, `AnalysisTask`, and their enums make queue state and checkbox-derived progress explicit.
- `AgentSession` records the provider-specific resumable session without exposing it through browser commands.
- `FeedbackRecord` preserves each revision request and outcome as an append-friendly JSONL record.
- `PageSpan::normalized` and analysis normalization keep model output inside the actual document.

The local CLI produces an internal `AnalysisDraft`, not a persisted `PaperAnalysis`. Normalization supplies stable section IDs, repairs cross-references, clamps tile sizes, validates page numbers, resolves exact section start/end excerpts, and records provider/time metadata before anything is stored.

Poppler runs twice: raw mode supplies authored reading order for Markdown and model input, while bbox-layout mode supplies exact page dimensions and word rectangles. Bbox words are repaired into stable logical tokens, including common split-letter and line-hyphen artifacts. A deterministic punctuation-aware pass assigns page-local sentence IDs. Analyzer quotations are canonicalized only for matching (case, punctuation, whitespace, and common ligatures); they become trusted anchors only when a unique complete-token match exists. Missing or ambiguous citations remain visible in the digest but never become AI prehighlights.

## Analysis boundary

Model-backed analysis is deliberately a subprocess boundary:

1. Extracted UTF-8 text is written to the paper's artifact directory.
2. A JSON Schema is written beside it.
3. The backend initializes `analysis-tasklist.md`; extraction stages update it before the model starts.
4. The analyzer starts in that directory. Its sandbox can edit the tasklist, and the prompt prohibits edits to every other artifact.
5. The prompt points at `source.txt`; the PDF text itself is marked as untrusted data.
6. The backend polls the tasklist through the queue endpoint, while Codex JSONL or Claude's JSON envelope supplies a resumable session ID.
7. Standard input carries the prompt, the final message is captured separately, and standard error is bounded before reporting.
8. A 20-minute timeout kills abandoned work; JSON is parsed, normalized, and evidence-checked before an atomic save.

Codex analysis uses `codex exec --json --output-last-message`; feedback prefers `codex exec resume <SESSION_ID>`. Claude uses its persisted print-mode session and `--resume`. A resume failure is safe to retry fresh because `source.txt`, `analysis.json`, and the reader feedback are all durable context. Clarification is deliberately kept out of the analysis session and runs ephemerally with read-only tools.

Codex analysis enables live web search; Claude receives explicit web-search and fetch tools. The model contract allows at most two external-context notes and requires every note to name exact source records. Missing, duplicate, or malformed mappings invalidate the complete note rather than silently weakening its citation set.

Source-link verification remains application-owned. Only cited HTTP(S) URLs on default ports are considered. For the initial URL and every redirect, the backend resolves DNS itself, rejects credentials and localhost, requires every returned address to be public, disables proxies, pins the request to a previously checked address, and accepts only a 2xx response. Redirect depth and the complete link check are time-bounded. A note survives only when every source it cites succeeds; unchecked and unreferenced records are discarded.

This deterministic gate proves a narrower fact than ground-truth interpretation: the exact link resolved to a public successful response at the recorded time. It cannot establish that the publication semantically entails the generated note. The Abstract view and `digest.md` therefore show the exact source, the analyzer's narrow account of what it supports, the check timestamp, and this limitation so readers can inspect the evidence themselves.

This makes the future switch to a different local agent—or an explicitly configured API adapter—an implementation detail behind `AnalysisService`.

The heuristic provider follows the same typed output path. It identifies printed headings where reliable, falls back to conceptual chunks, scores thesis-like sentences, assigns semantic families, extracts bounded quotations, and builds a small technical gloss. It never presents itself as model interpretation.

Markdown conversion is a separate, model-free derivative of extraction. It rejoins wrapped lines, infers common headings and labels, neutralizes embedded HTML, and emits explicit PDF page markers. Opening reconstructed Text lazily extracts an unmapped paper if necessary, but does not start a model analysis. The UI offers both a safe rendered view and the exact `.md` source.

## Fault behavior

Failures are paper-local and observable. A paper moves through:

```text
discovered → queued → extracting → analyzing → ready
                  └──────────── failed(stage, message, retryable)
```

Extraction and analysis artifacts are cached independently. A failed analysis retains valid extraction; an ingest run continues to later papers and reports an aggregate failure only after preserving successful results. Schema changes invalidate stale extraction caches. Writes use a temporary sibling followed by an atomic rename.

`job.json` survives server restarts. A job that was active when the process stopped is converted to a retryable interrupted failure on the next boot instead of being shown as permanently running. The Markdown tasklist is the progress source of truth while an agent works; malformed or missing agent edits fall back to the backend's typed task definitions.

Expected faults have dedicated errors: missing PDF tools, unreadable vaults, empty image-only extractions, failed local commands, timeout, invalid model output, duplicate processing, unsafe paper IDs, oversized extraction, and missing frontend assets.

## Reader interaction

The top-level information architecture is a monotonic reading ladder: Abstract → Overview → Glossary → Text. Abstract keeps generated orientation visibly separate from the authors' own words: `thesis` is the one-sentence TL;DR, `author_abstract` is retained only when its normalized text is present in the extraction, and `context_notes` provide at most two high-leverage, externally sourced sentences about field history, reception, or later interpretation. Legacy unsourced model context is withheld until reanalysis; the heuristic supplement remains explicitly limited to what can be inferred from the source paper. Overview owns the argument map, Glossary is a full pre-reading curriculum rather than a utility drawer, and Text owns both reconstructed Markdown and PDF formats.

Within that ramp, Overview leads with a CSS page grid containing every PDF page. Its column count is an explicit integer from one to ten: `+` zooms in by removing a column and `-` zooms out by adding one. Within a page, source-anchor token progress is projected onto the horizontal axis. A section transition three quarters through the page therefore lands three quarters across its page cell. This intentionally abstract orientation distinguishes structural segmentation from coordinate highlights. A secondary CSS grid retains the analysis-provided tile spans, where area expresses conceptual weight and color expresses argumentative role.

Focus is the single source of truth for mouse, touch, and keyboard navigation. Arrow keys mirror `h/j/k/l` in every spatial list. The digest exposes real selectable DOM text; its visual mode stores an anchor and a moving semantic-fragment cursor, so `v`, movement, `o`, `y`, and `c` parallel Vim without breaking native browser selection. The source map provides the same workflow over deterministic sentence segments: `Space` writes a same-page token range to `highlights.jsonl`, and `c` hands its exact text to the contextual clarifier. `F1` owns the library rail, while `F10` opens a focused fuzzy switcher that searches titles, authors, and years.

PDF.js renders either a focused page or an aligned two-page spread in Text and lazy page thumbnails in Overview. A spread is one paging unit for `h/l`, arrow keys, Ctrl-u/d, and PageUp/PageDown. Page cells preserve each PDF page's exact aspect ratio and boundary; section overlays use stable token order only to estimate reading progress along the abstract horizontal axis. Evidence and reader highlights remain coordinate-aligned because they identify literal source lines rather than conceptual regions. The default CSS filter produces light paper ink on a dark surface. Capital `I` toggles that filter everywhere, which is the reliable way to inspect figures, heatmaps, and photographs without color distortion.

Highlights deliberately avoid a database. `highlights.jsonl` is canonical and atomically rewritten in stable ID order, one complete JSON object per line. Reader records survive reanalysis; AI records are regenerated from currently verified key quotes. `highlights.md` is a disposable human-readable projection. This gives tools and people a plain-text interface while retaining enough typed geometry for lossless rendering.

## Deliberate MVP boundaries

- Scanned, image-only papers report an empty extraction instead of silently inventing OCR text. OCR is the next extraction adapter.
- Ingest is incremental but command-triggered. A filesystem watcher can later call the same refresh/queue path without changing the model.
- The PDF view pages through one page or one two-page spread rather than virtualizing a continuous document. The later native-reader experiment can reuse the API and portable artifacts.
- Analysis is single-paper and sequential during batch ingest to keep local CLI resource use predictable. Parallelism belongs behind an explicit concurrency limit.
