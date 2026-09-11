# Lysilogy architecture

## Shape of the system

```text
Public PDF URL ──► bounded, address-pinned import ──► origin.json
                                                      │ atomic PDF
PDF vault ◄────────────────────────────────────────────┘
   │ recursive discovery
   ▼
Paper catalog ──► Poppler raw + bbox extraction ──► source.txt + source.md + layout.json
                                                        │
                               abstract locate → repair → verify
                                                        │
                                      deterministic context prefetch
                                                        │
                               ┌────────────────┬────────┴────────┐
                               ▼                ▼                 ▼
                        fast orientation   structure/evidence   research
                               │                │                 ▼
                               │                │            context writer
                               │                │                 ▼
                               │                │          independent review
                               └────────────────┼─────────────────┘
                                                ▼
                                      merged AnalysisDraft
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

Remote imports cross a separate application-owned trust boundary. The backend accepts a public
HTTP(S) URL, disables environment proxies, resolves and pins its destination, repeats that check
for every redirect, and rejects credentials, nonstandard ports, and any DNS answer containing a
private or reserved address. Downloads are streamed to a hidden sibling file, capped at 100 MiB,
checked for a PDF header, synced, and renamed into the vault only when complete. The original URL,
final URL, byte count, and import time remain in the paper's `origin.json`. Serving the resulting
local file through `/api/papers/{id}/source` avoids dependence on browser CORS and remote range
request support.

## Domain model

The public model lives in `src/domain.rs`. Its enums keep states and semantic distinctions explicit:

- `ProcessingStatus` is a tagged state machine from discovery through ready or a structured failure.
- `ProcessingStage` identifies where a fault occurred.
- `AnalysisProvider` makes provenance visible throughout the API and interface.
- `SectionKind` describes conventional paper structure; `SectionFamily` groups those kinds into the Overview's color language.
- `QuoteSignificance` and `EvidenceStrength` prevent key quotations and claims from becoming untyped strings.
- `DocumentLayout`, `LayoutToken`, and `LayoutSentence` preserve PDF-point geometry with stable page-local coordinates.
- `CitationStatus` distinguishes exact, normalized, ambiguous, missing, and legacy-unverified evidence.
- `ContextNote` assigns a before/after/legacy role and maps each claim to source IDs. `ContextSource` preserves bibliographic metadata, an inspected excerpt/location, the claimed relationship, final checked URL, and verification time. `ContextAssessment` records independent review verdicts, citation metrics, and gaps.
- `AbstractResult` preserves a candidate's source lines/pages, fingerprint, accepted or unresolved status, and check report independently of generated orientation.
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
2. `src/abstracts/` locates source lines, retains structured subheadings, and conservatively removes running matter. A dedicated model pass proposes source boundaries even when a deterministic candidate exists. The independent verifier checks the whole span, narrow typographic normalization, and contamination without invoking the locator. A fresh model review can resolve uncertain boundaries but cannot waive provenance failures. `abstract.json` records the result.
3. Prefetch writes `analysis-context.json` with the accepted abstract, metadata, heading candidates, page-marked structural text, and bounded orientation excerpts. Orientation, structure/evidence, and historical context start concurrently with strict schemas and disjoint prompts. Context itself sequences evidence research, dossier-only writing, and independent review.
4. Orientation and clarification use Codex Luna at low effort. Structure, abstract repair, evidence gathering, and revision use Terra at medium effort. Context writing uses Astra at high effort (`LYSILOGY_CONTEXT_MODEL` overrides it); context review uses Terra at high effort. Claude retains its configured model with the same effort and tool partition.
5. All model subprocesses are read-only. Structure gets local read/search tools only when needed; context research and independent context review get live web tools. The writer sees frozen evidence. PDF and retrieved text are explicitly untrusted data.
6. Each successful stage immediately writes a typed cache artifact. Keys include source/prompt, schema, provider, profile, and effective model. An ordinary retry reuses matching stages; a forced run invalidates them. Malformed cached JSON is ignored.
7. The backend, not a model, updates `job.json` and its Markdown tasklist projection. Multiple analysis tasks may be active at once.
8. Codex JSONL or Claude's JSON envelope supplies a session ID only for the structural branch. Standard input carries prompts, final messages are captured separately, and stderr is bounded before reporting.
9. A 20-minute per-process timeout kills abandoned work. The three drafts are merged, normalized, source-mapped, and link-checked before an atomic final save.

Codex calls use `codex exec --json --output-last-message`; feedback prefers `codex exec resume <SESSION_ID>`. Claude uses its persisted print-mode structural session and `--resume`. A resume failure is safe to retry fresh because `source.txt`, `analysis.json`, and reader feedback are durable context. Clarification is deliberately outside the analysis session and runs ephemerally against prefetched passage surroundings.

`src/analysis/context/` owns the evidence, writing, review, and admission contracts. It permits up
to eight sources and six atomic claims, with the writer asked for at most three per temporal role.
Sources must include a short inspected excerpt, its location, and the relationship it supports.
The reviewer receives proposed claims and the dossier without the writer's reasoning and checks
passage provenance, complete support, chronology, and usefulness. Missing/duplicate mappings or
reviews, partial support, unsupported relationships, and duplicated/generic prose withhold the
claim. Runtime metrics count proposed/cited/published claims, proposed/supported links, fully
supported claims, and unassessed claims. They report model assessment, not semantic ground truth.

Source-link verification remains application-owned. Only cited HTTP(S) URLs on default ports are considered. For the initial URL and every redirect, the backend resolves DNS itself, rejects credentials and localhost, requires every returned address to be public, disables proxies, pins the request to a previously checked address, and accepts only a 2xx response. Redirect depth and the complete link check are time-bounded. A note survives only when every source it cites succeeds; unchecked and unreferenced records are discarded.

This deterministic gate proves a narrower fact than ground-truth interpretation: the exact link resolved to a public successful response at the recorded time. It cannot establish that the publication semantically entails the generated note. The Abstract view and `digest.md` therefore show the exact source, the analyzer's narrow account of what it supports, the check timestamp, and this limitation so readers can inspect the evidence themselves.

This makes the future switch to a different local agent—or an explicitly configured API adapter—an implementation detail behind `AnalysisService`.

`analysis/sectioning.rs` and `prompts/sectioning.md` define the map's reading units: coherent topics
of 1–5 occupied pages, with a preference for 1–2. The prompt supplies a page-aware planning budget
and keeps parent topics with their short dependent subheadings. Verified source-token endpoints
provide fractional page-size estimates. A prevalence of short units, repeated parent/variant titles,
or overly long units triggers at most one additional structural call using the original source,
first draft, and diagnostic feedback. The final report retains unresolved warnings rather than
pretending size checks prove semantic coherence. Initial analysis and feedback prompts share the
policy. The structure cache includes the actual prompt, schema, source/layout, and model profile.

`refresh-structure` and `/api/papers/{id}/structure/refresh` run only that structural path, including
claim/glossary references and source validation. The existing abstract, thesis, prerequisites, and
external context are preserved. Publication uses the normal highlight synchronization under its
write lock so reader-owned marks survive while AI marks follow new section IDs. A model or validation
failure leaves the previous analysis in place. This operation uses the existing per-paper job
reservation and is also available as `:refresh-structure` in the reader.

The heuristic provider follows the same typed output path. It identifies printed headings where reliable, falls back to conceptual chunks, scores thesis-like sentences, assigns semantic families, extracts bounded quotations, and builds a small technical gloss. It never presents itself as model interpretation.

## Prompt experiment boundary

Learning-ramp experiments are deliberately separate from canonical analysis. A catalog entry names
one prompt dimension and exactly two variant instructions. The backend randomizes their `A`/`B`
positions, runs both against the same extracted paper with the same model, effort, shared prompt,
tools, and JSON Schema, and stores a typed run beneath the paper's `experiments/` directory.
Neither arm can overwrite `analysis.json`, `digest.md`, or highlights.

The API redacts variant IDs and labels until a judgment is stored. The interface records overall,
early-traction, and rigor preferences plus confidence and a note, then reveals which prompt produced
each arm. This makes the artifact useful both for immediate qualitative inspection and later
aggregation across paper field and reader-rated distance.

Markdown conversion is a separate, model-free derivative of extraction. It rejoins wrapped lines, infers common headings and labels, neutralizes embedded HTML, and emits explicit PDF page markers. Text opens the source PDF by default; pressing `m` explicitly requests the reconstruction, which lazily extracts an unmapped paper if necessary but does not start a model analysis. Pressing `m` again returns to the PDF. The reconstruction offers both a safe rendered view and the exact `.md` source.

## Fault behavior

Failures are paper-local and observable. A paper moves through:

```text
discovered → queued → extracting → analyzing → ready
                  └──────────── failed(stage, message, retryable)
```

Extraction and analysis artifacts are cached independently. A failed analysis retains valid extraction; an ingest run continues to later papers and reports an aggregate failure only after preserving successful results. Schema changes invalidate stale extraction caches. Writes use a temporary sibling followed by an atomic rename.

`job.json` survives server restarts. A job that was active when the process stopped is converted to a retryable interrupted failure on the next boot instead of being shown as permanently running. Typed backend state is the progress source of truth; `analysis-tasklist.md` is a generated, human-readable projection and external edits are ignored.

Expected faults have dedicated errors: missing PDF tools, unreadable vaults, empty image-only extractions, failed local commands, timeout, invalid model output, duplicate processing, unsafe paper IDs, oversized extraction, and missing frontend assets.

## Reader interaction

The top-level reading ladder is Abstract → Overview → Glossary → Text. Abstract separates generated
orientation from the source-verified authored abstract. It shows explicit unresolved extraction
states and distinct **Before the paper** and **After the paper** cards with claim-level citations,
inspected excerpts, and an evidence assessment. Legacy cited notes remain readable in a separate
card and are never inferred to be historical before/after context. The heuristic supplement remains
limited to the paper. Abstract/context refresh actions have independent CLI/API/job entry points;
they preserve section maps and never rewrite highlights. A failed context-only refresh leaves the
previous analysis intact. Overview owns the argument map, Glossary is the pre-reading curriculum,
and Text owns reconstructed Markdown and full-paper PDF views.

Within that ramp, Overview leads with a CSS page grid containing every PDF page. Its column count is an explicit integer from one to ten: `+` zooms in by removing a column and `-` zooms out by adding one. Within a page, source-anchor token progress is projected onto the horizontal axis. A section transition three quarters through the page therefore lands three quarters across its page cell. This intentionally abstract orientation distinguishes structural segmentation from coordinate highlights. A secondary CSS grid retains the analysis-provided tile spans, where area expresses conceptual weight and color expresses argumentative role.

Focus is the single source of truth for mouse, touch, and keyboard navigation. Arrow keys mirror `h/j/k/l` in every spatial list. The digest exposes real selectable DOM text; its visual mode stores an anchor and a moving semantic-fragment cursor, so `v`, movement, `o`, `y`, and `c` parallel Vim without breaking native browser selection. The source map provides the same workflow over deterministic sentence segments: `Space` writes a same-page token range to `highlights.jsonl`, and `c` hands its exact text to the contextual clarifier. `F1` owns the library rail, while `F10` opens a focused fuzzy switcher that searches titles, authors, and years.

Opening a region mounts `SectionFocus`: a continuous column of only its original PDF pages on the
left and an independently scrollable digest on the right. The pure `sectionScope` helper validates
anchor endpoints before using them, otherwise uses a clamped legacy page range. Valid token spans
produce coordinate section marks; fallback ranges do not imply precise text boundaries. A compact
whole-paper locator keeps the selected pages in context. Source links scroll within the subset or
open the matching page in the full Text reader.

`usePdfDocument` shares a reference-counted PDF.js loading task across map and reader surfaces.
Scoped pages reserve their final dimensions and render canvases/text layers near the viewport.
Temporary page snapshots animate measured map rectangles into the source column; the actual text
layer stays at its final size. Transitions can cancel, and reduced motion skips them. The hidden
map remains mounted and inert, preserving its columns and scroll for close; focus and prior library
state are restored. Narrow screens switch between Source/Digest tabs. Keyboard scrolling targets
the active pane, with brackets switching sections.

PDF.js renders either a focused page or an aligned two-page spread in Text and lazy page thumbnails in Overview. Its official text-layer builder supplies native multi-line and cross-page selection over the canvas; Lysilogy records the selected text, PDF.js item offsets, pages, and rectangles converted back into PDF points. The resulting action bar can copy the passage or seed contextual clarification. A spread is one paging unit for `h/l`, arrow keys, Ctrl-u/d, and PageUp/PageDown. Page cells preserve each PDF page's exact aspect ratio and boundary; section overlays use stable token order only to estimate reading progress along the abstract horizontal axis. Evidence and reader highlights remain coordinate-aligned because they identify literal source lines rather than conceptual regions. The default CSS filter produces light paper ink on a dark surface. Capital `I` toggles that filter everywhere, which is the reliable way to inspect figures, heatmaps, and photographs without color distortion.

Highlights deliberately avoid a database. `highlights.jsonl` is canonical and atomically rewritten in stable ID order, one complete JSON object per line. Reader records survive reanalysis; AI records are regenerated from currently verified key quotes. `highlights.md` is a disposable human-readable projection. This gives tools and people a plain-text interface while retaining enough typed geometry for lossless rendering.

## Deliberate MVP boundaries

- Scanned, image-only papers report an empty extraction instead of silently inventing OCR text. OCR is the next extraction adapter.
- Ingest is incremental but command-triggered. A filesystem watcher can later call the same refresh/queue path without changing the model.
- The full-paper Text view pages through one page or one two-page spread. The focused section view is continuous and renders nearby pages lazily; it does not unload every offscreen page after rendering.
- Abstract extraction uses existing raw reading order and layout boundary hints. General two-column reconstruction and OCR remain outside this module. Uncertain unlabeled candidates require independent review.
- Runtime citation review is model judgment. A held-out abstract corpus and a blind, repeated context-quality comparison remain outstanding; passing orchestration tests does not establish generation quality.
- Batch ingest remains sequential across papers to keep local CLI resource use predictable; the three scoped stages inside one paper run concurrently.
