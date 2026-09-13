# Lysilogy

Lysilogy turns a vault of scientific PDFs into a keyboard-first reading path for intelligent
outsiders to a field. It preserves source PDFs and adds imported papers only on request. Every
generated artifact is plain, diffable text stored outside the source library.

**Lysilogos** is the agent who cuts papers and follows their references inside Lysilogy.

The current demo has been exercised against a local corpus of 118 PDFs and ships with a mapped copy
of Dijkstra's "GOTO Statements Considered Harmful."

## The reading ladder

Click **Lysilogy** above the sidebar’s Vault heading (or the compact Home control) to return to the library home page. Each card shows a first-page PDF preview
with a compact title, authors, year, and mapping status. Arrows or `h/j/k/l` move between cards;
Enter or `o` opens the selected paper. Home/End select the first/last card, and `/` searches.
From search, Down or Enter focuses a matching card. The selection is restored when returning home.
Previews load near the viewport, with at most two renders running; only the small images are cached.
Filter mapped or unmapped papers, sort, or click anywhere on a card to read it. The empty URL and
`#home` open the grid; `#paper=<id>` remains a direct paper link. Browser Back and Forward work
between the library and papers.

The top bar is a monotonic ramp — each level is strictly more detailed than the one before it.

| Level | What it gives you |
| --- | --- |
| **Abstract** | A generated one-sentence thesis, a source-verified authored abstract, and separate cited accounts of research before the paper and its subsequent influence. |
| **Overview** | A resizable whole-paper page map and conceptual tiles. Opening a region brings its source pages into a scrollable left column beside the contextual digest. |
| **Glossary** | The load-bearing technical vocabulary to hold in your head before reading. |
| **Text** | The selectable source PDF by default; reconstructed Markdown is available on request. |

Two provenance rules hold everywhere:

- Abstract proposals must match an entire source span under narrow typographic normalization and
  pass separate boundary checks. Ambiguous boundaries need independent model review; unresolved
  text is withheld. Generated orientation stays visibly separate from the authors' words.
- Every new context claim maps to inspected excerpts and passes an independent model review of
  support, chronology, and usefulness. Every cited URL must also pass bounded DNS,
  redirect, public-address, and HTTP-success checks before the note or its sources are persisted.
  One failed citation withholds the whole note. The interface states the limit of that guarantee:
  reachability at a recorded time is not evidence that a source semantically supports the claim.

AI citations become prehighlights only after a deterministic unique-match check against the
extracted tokens. Reader highlights use the same stable sentence and token anchors, so both survive
reanalysis.

## Requirements

- Rust 1.88 or newer
- Node.js 22 or newer
- Poppler's `pdftotext` and `pdfinfo`
- Optionally, an authenticated `codex` or `claude` CLI for model-backed analysis

## Quick start

Build the frontend, map one paper with the network-free analyzer, and serve both halves:

```sh
cd web
npm install
npm run build
cd ..

cargo run -- analyze "GOTO Statements" --provider heuristic
cargo run -- serve
```

Open <http://127.0.0.1:7319>.

For frontend work, run `cargo run -- serve --bind 127.0.0.1:7320` and `npm run dev` (from `web/`) in separate terminals;
Vite proxies `/api` to the Rust server. For a local redeploy, rebuild both halves, stop the running
server, and relaunch the release binary:

```sh
cd web && npm run build && cd ..
cargo build --release
./target/release/lysilogy serve
```

## Configuration

| Flag | Environment variable | Default |
| --- | --- | --- |
| `--library` | `LYSILOGY_LIBRARY` | `local-articles` |
| `--data` | `LYSILOGY_DATA` | `.lysilogy` |
| `--notes` | `LYSILOGY_NOTES` | `Notes` |
| `--config` | `LYSILOGY_CONFIG` | Optional `lysilogy.config.json` |
| `--bind` (serve) | — | `127.0.0.1:7319` |
| `--web` (serve) | — | `web/dist` |

```sh
cargo run -- --library /path/to/Articles --data /path/to/lysilogy-data serve
```

## Mapping the vault

Discovery is recursive and incremental. Paper IDs are derived from paths and stay stable; generated
material never lands inside the source vault.

```sh
# Inventory the vault
cargo run -- scan

# Convert one PDF to portable Markdown on stdout
cargo run -- convert "title fragment"

# Evaluate a few prompts before committing to a full run
cargo run -- ingest --provider codex --limit 3

# Run one blind, single-dial learning-ramp A/B experiment
cargo run -- experiment "title fragment" --experiment conceptual-bridge --provider codex

# Run three replications per paper; papers are processed in parallel
cargo run -- experiment "economics title" "philosophy title" "vision title" \
  --experiment conceptual-bridge --provider codex --repeat 3 --concurrency 3

# Aggregate persisted scorecards, preferences, failures, and run coverage
cargo run -- experiment-report --output docs/experiment-reports/latest.md

# Map everything not already ready; one failure does not discard other results
cargo run -- ingest --provider codex

# Alternate local CLI, or the network-free structural fallback
cargo run -- ingest --provider claude
cargo run -- ingest --provider heuristic

# Re-run one paper after changing a prompt or analyzer
cargo run -- analyze "title fragment" --provider codex --force

# Regroup only the map, claims, and glossary into coherent reading units
cargo run -- refresh-structure "title fragment" --provider codex --force
```

Opening an older map lazily regenerates its coordinate extraction and revalidates existing quotes.
To replace dashed legacy section extents with exact start/end spans, rerun that paper with
`--force`; `ingest --provider codex --force` migrates the whole mapped library.

### How analysis runs

Lysilogy drives local command-line tools rather than an API. Initial analysis first runs the
standalone abstract pipeline, then prefetches metadata, headings, page-marked text, and bounded
opening/closing context. Orientation, structure/evidence, and historical context run concurrently.
The context branch sequences three separate calls: evidence research, writing from that frozen
evidence, and independent review. Research and review have web tools; the writer receives only the
dossier. Structure receives local read tools when a very large paper had to be sampled.

For Codex, the small orientation and clarification jobs use `gpt-5.6-luna` at low effort. Structural
analysis, context evidence gathering, and revision use `gpt-5.6-terra` at medium effort. Historical
context writing uses `gpt-6-astra` at high effort, configurable with `LYSILOGY_CONTEXT_MODEL`; a
separate Terra high-effort web pass checks the cited passages, chronology, and usefulness. Claude
receives the same scoped tools and effort levels while retaining its configured model. Abstract
extraction has its own deterministic locator, model review, and independent source/boundary
verifier. The repair pass runs even when deterministic extraction found text. Structured abstract
subheadings are retained; unlabeled candidates need independent front-matter evidence or a separate
boundary review. Source prose uses literal Poppler word boundaries; the merged coordinate index
remains stable for existing citations. Author rows are extracted from the PDF, including names
separated by affiliation markers or affiliation blocks. The result and check report are saved in
`abstract.json`; orientation consumes the accepted result. An uncertain or unsupported proposal is
withheld. `cargo run -- refresh-abstract "title fragment" --provider codex --force` (or
`:refresh-abstract`) refreshes this component without regenerating the map or context.

Context is now researched into a frozen evidence dossier, written as distinct before/after
claims, then independently reviewed. `context-assessment.json` records citation coverage, supported
links, fully supported claims, unassessed claims, and research gaps. These are model-assessed support
metrics; successful URL checks remain a separate deterministic guarantee. Refresh only this component
with `cargo run -- refresh-context "title fragment" --provider codex --force` or `:refresh-context`.
The Abstract view also provides separate refresh buttons for abstract and context. These refreshes
preserve section maps and saved highlights. A failed context-only refresh preserves the existing
analysis; an initial analysis can still save its map with an explicit context research gap.

Each initial branch writes a typed stage artifact as soon as it succeeds. A retry reuses matching
stages and reruns only missing or malformed ones. Keys include the source/prompt, schema, provider,
profile, and effective model; `--force` deliberately invalidates the stage cache. Only the structural
call retains a resumable session for feedback. If that session cannot be
resumed, revision falls back to a fresh read-only call with `source.txt`, `analysis.json`, and the
feedback already present. Clarification stays ephemeral and uses prefetched local passage context.

The backend owns `analysis-tasklist.md` and its typed `job.json` state; model processes are read-only
and never edit progress. Press `Q` to watch analysis progress.

### Reading units in the map

Sectioning favors coherent topics spanning 1–5 pages of content, usually 1–2 pages. Parent topics,
their variants, and short examples stay together; internal distinctions belong in the digest.
For example, Extremal Goodhart, Model Insufficiency, and Change in Regime should form one region
titled **Extremal Goodhart**. A ten-page paper starts with a planning budget of roughly 5–7 main
units, adjusted for actual topics and non-body material. The budget is guidance, not a quota.

The shared instructions live in [`prompts/sectioning.md`](prompts/sectioning.md). After generation,
source-token positions estimate occupied page fractions; short fragments across a page break do
not count as two full pages. Unusually many small units, repeated parent/variant titles, or units
over about five pages can trigger one consolidation pass. `sectioning-report.json` records the
initial and final diagnostics. Remaining size warnings are advisory: semantic coherence and source
coverage take priority over automatic merging or a forced count.

Use `:refresh-structure` or the CLI command above to regenerate only the map, structural claims,
glossary, caveats, and reading path. The authored abstract, thesis, prerequisites, and historical
context are retained. AI evidence marks are rebuilt for the new sections while reader highlights
are preserved. Failed generation or validation leaves the previous analysis available. Structural
cache keys include the actual prompt/policy, schema, source/layout, provider, and model profile.

### Learning-ramp prompt experiments

The separate experiment lane compares two structured learning ramps without replacing the paper's
canonical analysis. Its five initial single-factor tests cover conceptual bridges, glossary order,
the context budget around the essential 10%, concrete downstream use/misuse, and camp-aware
counterarguments. The paper, model, reasoning effort, shared prompt, and output schema stay fixed
within a run; only the named variant instruction changes.

Open `:experiment` in the reader to start or inspect a run. Arms remain labeled only `A` and `B`
until you independently score both ramps, flag hard failures or anti-slop patterns, and record the
comparative preference. The revealed prompt identities and judgments persist beside the paper.
Product intent, hypotheses, guardrails, and promotion criteria are recorded in [the learning-ramp
experiment brief](docs/learning-ramp-experiments.md); editable experiment definitions live in
[`experiments/catalog.json`](experiments/catalog.json).

The CLI accepts several paper IDs or quoted title fragments plus `--repeat`. Different papers run
with bounded parallelism; replications for the same paper remain sequential so concurrent first-use
extraction cannot corrupt its artifacts. Each run still generates its two arms concurrently with
run- and arm-specific schema/output paths.

`experiment-report` converts persisted runs and judgments into Markdown tables for absolute quality,
blind wins, hard rejects, recurring failure tags, and run coverage. It explicitly reports when no
judged evidence exists instead of inferring a winner from incomplete runs.

The heuristic provider is deliberately conservative. It gives you an immediate offline Overview and
labels itself plainly; use a model-backed provider for interpretive reading and field context.

## Supercuts and reference tools

Open Lysilogos with `:supercut` or `:references`.

- **Supercut:** request exactly ten paragraphs or six printable Letter pages. Lysilogos selects a
  coherent path through the question, mechanism, evidence, and qualifications. At least 80% of prose
  words must be exact source text; generated connector sentences are visibly labeled. Every source
  segment links to its PDF page. Download Markdown or use **Print / save PDF**.
- **Saved references:** select a citation in the PDF and choose **Save citation**, or paste a title,
  DOI, or bibliography entry into the References tab. The citation, optional page, and note persist.
- **Find & fetch paper:** Lysilogos searches for the cited paper and returns a candidate with an
  explanation. When it finds a public PDF, the backend imports it through the same bounded public-URL
  checks as **+ URL** and links the imported paper. Unavailable copies remain saved for later.
  You can also link a paper already in your library.
- **Check claim / Explain connection:** after linking the cited paper, ask a question or request a
  short connector. Lysilogos receives both extracted papers. Each saved answer includes verified
  passages from both, a relationship verdict, and a limitation. Exact quotations establish provenance;
  the relationship remains model interpretation.

Saving, removing, and linking references work offline. Generation, search, and comparison require
the selected Codex or Claude reader. Each explicit task starts one model call, with no automatic model
retry; reference search alone enables web tools. Tasks continue when the panel closes. Failures are
saved with a retry action, and an interrupted server marks unfinished tasks failed on restart.

Supercuts enforce case-sensitive, complete-word source matches (allowing extraction whitespace),
reject overlapping/repeated excerpts, and count source words against all prose words. Ten-paragraph
cuts have a 2400-word ceiling. Six-page cuts use up to 420 words and 2800 characters per page; the print
control also checks for page overflow. Source documents are limited to 180 KB of extracted context;
two-paper comparisons are limited to 240 KB combined. Oversized or image-only sources fail visibly
instead of silently using an incomplete paper.

Saved work lives in `papers/<id>/reader-tools.json`; each model task has its own
`reader-jobs/<job-id>/` schema and output directory. Existing analysis, digests, and highlights are
preserved.

## Keyboard model

Press `?` in the app for the complete, contextual guide.

| Key | Action |
| --- | --- |
| `h j k l` or arrows | Move through tiles, panels, pages, or a visual text selection |
| `g g` / `G` | First / last tile |
| `Enter` or `o` | Open the section's source pages beside its digest |
| `d` | Toggle the digest |
| `g` | Open the Glossary after a short single-key delay |
| `m` | Toggle source PDF / requested Markdown reconstruction |
| `p` | Toggle Overview / source PDF |
| `2` | Toggle one-page / two-page PDF view |
| `W` / `H` | Fit the PDF to viewport width / height and reset zoom |
| `gW` / `gH` | Fit visible PDF content width / height with a small margin and reset zoom |
| `P` | Toggle paged / continuous PDF reading |
| `R` | Rotate continuous scrolling: vertical / horizontal |
| `+` / `-` | One fewer / one more page column in Overview (up to 10) |
| `[` / `]` | Previous / next paper, or PDF page |
| `Ctrl-d` / `Ctrl-u` | Page forward / back in PDF; half-screen in text views |
| `PageDown` / `PageUp` | Page forward / back in PDF; full-screen in text views |
| `/` | Regex search in the paper; filter the focused library input |
| `v` | Start source visual selection, digest selection, or map sentence marking |
| `o` | Swap the moving end of a visual selection |
| `c` | Clarify the selection in paper context |
| `y` | Copy the selection |
| `Space` | Persist the selected source sentence range as a reader highlight |
| `H` / `U` in Overview | Toggle AI-cited prehighlights / reader highlights |
| `I` | Invert every PDF rendering between dark ink and true image colors |
| `F1` | Toggle the library from anywhere |
| `F10` | Open the fuzzy article switcher |
| `f` | Show Vimium-style hints for visible citations, figures, tables, and PDF links; filter mapped papers when the library has focus |
| `Ctrl-o` | Return to the reading position before following a link |
| `:` | Command menu (`:analyze`, `:queue`, `:feedback`, `:spread`, and more) |
| `:experiment` | Open the blind learning-ramp A/B prompt lab. |
| `Q` | Toggle the processing queue and live analysis tasklists |
| `q` / `Esc` | Leave the current selection or panel, return to map, then home |
| `T` | Show or pin reader controls; the top mouse edge also reveals them |
| `E` | Open Markdown notes beside the paper |

In the PDF, press `f` and type the yellow hint to follow its link. `Tab` shows the citation or
caption destination in the hint bar; `Enter` follows the inspected hint. `Backspace` edits a
partial hint, and `Esc` or `q` cancels. Grouped citations get separate hints for each destination.
Only visible links within the current page or section crop receive hints. Following a destination
outside the section opens it in the full paper, with its location briefly outlined; `Ctrl-o`
returns to the previous source position. Web links open in a new tab.

Link matching uses the current paper's bibliography and captions: numbered and alphanumeric
references, raised numeric citations, author–year citations, figure/table abbreviations, Roman
numerals, and supplementary labels. Embedded PDF destinations take precedence for individual
links. Ambiguous or unresolved text references are omitted; scanned pages depend on available
OCR or embedded links. Hint keys belong to the PDF reader; notes and editable fields keep their
normal keyboard behavior.

Pointer selection works too: select text in a digest, then choose **Clarify selection**. In the page
map, `v` enters a coordinate-backed evidence cursor; a second `v` starts a same-page sentence range,
movement extends it, `Space` stores it, and `c` sends the exact sentence text into clarification.
Same-page ranges keep one compact, relocation-resistant token anchor; save cross-page notes as
separate highlights.

Overview arrows follow the actual screen rectangles rather than assuming equally sized tiles:
Down chooses the region beneath the current region, and navigation adapts to the chosen page
columns. Title, authors, and year share a compact header where space permits. Large author lists
expand to show every extracted name in a bounded, scrollable list. The authored abstract is
justified, and each verified context note is a bullet with its supporting citations.

Paper details reload saved author metadata and upgrade older extractions on opening. Restart the
Rust backend after backend changes; Vite only reloads the frontend. An older backend can rewrite
the extraction cache using its older schema.

Citation discovery uses a shared provider interface with OpenAlex, Semantic Scholar,
OpenCitations, and Crossref adapters. Fetch a paper's neighborhood using its exact external ID:

```sh
cargo run -- --library local-articles --data .lysilogy citation-graph "title fragment" \
  --identifier doi:10.1234/example --limit 100
```

Replace the example DOI with the paper's DOI. `--provider` and `--direction` narrow the request;
the defaults query all four providers for references and citations. The same operation is exposed
as `POST /api/papers/{id}/citation-graph`; `GET` reads the saved snapshot. Reports preserve provider
identity, retrieval times, citation passages where available, and explicit coverage/error states.
Crossref's unsupported incoming list is reported separately from an empty result.

The saved `citation-graph.json` supplies discovery candidates to subsequent context research;
the researcher and reviewer still verify original passages before publishing historical claims.
Discovery is explicit and does not add network calls to offline analysis. See
[Citation graph sources](docs/citation-graph-sources.md) for credentials, identifier support,
limits, and the document-level verification approach.

PDF mode renders a PDF.js text layer over the page image. Drag over a passage—even across lines or
the two-page spread—to copy it or open **Ask about this** with the passage and its first page already
filled in. Selections retain PDF-page coordinates and text-item offsets for future persistent marks.
Image-only pages can be indexed locally with Tesseract for search and visual text objects; missing tools or unfinished pages are reported as search gaps.
Unanalyzed papers open directly in source reading, with PDF/Text tabs and one Analyze action.
Passage questions work before analysis; extraction runs on demand without creating an analysis.
Analyzed papers display their author, year, and title in the app bar; `Q` opens the queue.

Opening an Overview region animates its source into a reading column with a compact digest on
the right. Both panes fill the space below the app bar and scroll independently. The original PDF
is cropped to the section's verified token bounds, including its first and last partial pages;
surrounding text is excluded from rendering and native selection. Footnotes are retained when a
raised callout belongs to the selected section, including notes below its ending boundary.
Missing geometry offers an
explicit link to the full page. The page picker preserves original PDF numbering, and **Open full
paper** takes the current page into Text. **Map** or `Esc` restores the map's scroll, columns,
keyboard focus, and library state. Escape works from either reading pane, including controls,
selected passages, and inline questions. A selection or inline question closes before its containing reader; a separate open dialog closes first. Section navigation
lives in the digest footer. Reduced-motion
settings skip the transition; narrow screens provide Source/Digest tabs. Overview and both readers
share a PDF document, and scoped page canvases render lazily.
Both readers offer fit width/height and paged/continuous modes. `gW` and `gH` trim blank paper
around the visible text and figures, leaving a small visual margin; `W` and `H` restore the
full page bounds. Press `g` then Shift-W or Shift-H within the glossary's short prefix delay.
A lone `g` still opens the glossary in the full reader. Continuous pages can scroll vertically
or horizontally; the mouse wheel follows the chosen direction. Uppercase `H`, `P`, and `R`
distinguish reader controls from the existing lowercase navigation shortcuts.

In the focused source pane, `j/k` or up/down scroll, `h/l` or left/right move between its pages,
`Ctrl-d/u` and PageDown/PageUp scroll by screen increments, and `[`/`]` switch sections. `+`/`-`
zoom the source. Native selection supports Copy, Save citation, and Ask about this.

Use **+ URL** in the library rail to import a PDF from the public web. Lysilogy downloads it on the
server, checks every redirect and resolved address, rejects non-PDF responses and files over 100
MiB, writes the completed file atomically into the vault, and opens it directly in PDF mode. Direct
browser loading is deliberately avoided, so the source site does not need permissive CORS headers
or byte-range behavior. Private, loopback, link-local, credentialed, and nonstandard-port URLs are
rejected.

## Artifacts on disk

Everything generated lives beneath the data root:

```text
.lysilogy/
└── papers/
    └── <stable-paper-id>/
        ├── source.txt           # UTF-8 extraction, form-feed page boundaries
        ├── source.md            # full-document Markdown with page markers
        ├── origin.json          # original/final URL and byte count for remote imports
        ├── layout.json          # PDF points, stable page-local tokens, sentence segments
        ├── extraction.json      # extraction schema and normalized metadata
        ├── abstract.json        # source span, accepted/unresolved status, independent checks
        ├── context-assessment.json # claim/link support counts, reviews, and research gaps
        ├── sectioning-report.json # initial/final size and parent-topic fragmentation checks
        ├── analysis.json        # typed, versioned application model
        ├── digest.md            # portable human-readable digest
        ├── highlights.jsonl     # canonical one-highlight-per-line records (AI and reader)
        ├── highlights.md        # generated human-readable projection
        ├── analysis-context.json # deterministic prompt context shared by stages
        ├── analysis-*.json      # typed scoped-stage outputs and cache manifest
        ├── analysis-tasklist.md # backend-generated live checklist
        ├── job.json             # typed queue/progress state for the latest run
        ├── agent-session.json   # resumable Codex or Claude session identity
        ├── feedback.jsonl       # one reader feedback record per line
        ├── experiments/         # blind A/B runs and durable human judgments
        └── *.schema.json        # exact local-CLI output contracts
```

Writes are atomic. `analysis.json` holds contextual notes, their source-ID mappings, exact source
titles/authors/years, final working URLs, and link-check timestamps; `digest.md` renders the same
citations for use outside the app. `highlights.jsonl` is the canonical record — each line is a
complete typed highlight with provenance, exact quoted text, PDF page, page-local token range,
sentence IDs, and PDF-point rectangles — diffable and scriptable without a database. The original
PDFs are never modified, and every artifact can be read, searched, versioned, or reused without
running the frontend.

## Quality checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets

cd web
npm run typecheck
npm run lint
npm run build
npm run test:section-scope
npm run smoke:pipeline
npm run smoke:reader-tools
# Optional: requires access to the existing corpus fixture and original PDF
npm run smoke
```

`npm run smoke` drives the real Dijkstra analysis, Markdown conversion, and PDF through an
in-process browser route, so it works even where loopback networking is blocked. It covers the
four-level top bar, abstract provenance, contextual sources and link-check scope, the all-page
Overview grid, horizontal section progress and integer-column zoom, the mapped-only filter, F1/F10
switching, the command menu, live tasklist progress, feedback retries, keyboard selection and
clarification, the Glossary, reconstructed Text, selectable one/two-page PDF paging, and capital-`I`
inversion.

`test:section-scope` checks exact and legacy section page ranges. `smoke:pipeline` uses a synthetic
PDF and API fixtures to check distinct before/after context, component refresh requests, scoped
source pages, independent scrolling, selection, shared PDF loading, map restoration, full-paper
spreads, reduced motion, and mobile layout without reading the library or calling a model.

`smoke:reader-tools` uses self-contained browser fixtures to exercise both cut formats, six-page
PDF output, Markdown export, saved citations, paper linking, search, comparison, retry, keyboard focus,
and mobile layout. Backend tests use local stub CLIs to exercise task execution and source validation
without model calls. For the original corpus-based smoke test in a worktree, set
`LYSILOGY_SMOKE_FIXTURE_ROOT` to the checkout containing the cached paper. Set
`LYSILOGY_SMOKE_PDF` as well if its PDF has moved from the default fixture path.

The implementation map and fault boundaries are in [docs/architecture.md](docs/architecture.md).
The delivery phases for abstract fidelity, cited before/after context, and a focused section reader
are in [the reading pipeline plan](docs/reading-pipeline-plan.md). The implemented scope, checks,
and outstanding model-quality evaluation are recorded in the
[validation report](docs/experiment-reports/2026-09-11-reading-pipeline.md).

### Paper search, figures, and notes

Reader chrome is hidden by default in the full PDF and focused section views. Move to the top
edge or press `T` to reveal/pin it. Reading phase controls stay together, while long author/year/title
metadata wraps. The sidebar holds the app branding. `Q` opens the queue; lowercase `q` and Escape
unwind the current mode before closing the reader. The footer hint is removed.

In a paper, `/` opens regex search over source text. Enter runs the pattern; `n` / `N` move forward /
backward. `/optimization`, Enter, `n`, `n`, `v`, `a`, `p`, `y` searches, selects a paragraph and copies
it. Visual selection supports `iw` / `aw`, `iW` / `aW`, `is` / `as`, and `ip` / `ap`, plus source
cursor placement by clicking. `h/l` select individual characters, `e/E` move to word ends, and
`(`/`)` move to sentence starts. Native PDF character positions keep partial-word highlights precise,
and selected words share continuous line highlights. See [source selection](docs/source-selection.md)
for the complete motion and text-object behavior. Clipboard denial exposes selectable copy text. Out-of-section matches
offer an explicit jump to the full paper, preserving section bounds. Library and Glossary filters
remain local when their inputs are focused.

Opening a PDF quietly warms its search index during browser idle time. Index builds run at
lower priority, are shared with searches, and continue after leaving the paper. Disk indexes
have no expiry; PDF size/modification-time or schema changes invalidate them. The browser also
caches responses for 30 days and conditionally validates them on each opening, while a bounded
in-memory cache avoids repeated parsing. Failed warmups stay quiet and can be retried by searching.
See [reading-index caching and scheduling](docs/reading-index.md#background-work-and-cache-lifetime).

`GET /api/papers/{id}/reading-index` supplies the cached UTF-16 text/geometry index. Native
PDF text, columns, paragraphs, captions and footnotes are processed deterministically; sparse pages
use local `pdftoppm` + `tesseract` when installed. Native and OCR provenance stays visible. Limits are
400 pages, 4 MiB text, and 12 OCR pages per build; unfinished pages have explicit gaps. Regex runs in
a terminable worker with a one-second deadline. This cache leaves existing citation anchors intact.
OCR and paragraph/figure segmentation remain heuristic, especially on unusual scans and layouts.

When a cropped section cites a figure outside its visible regions, **Referenced figures** opens its
source image and caption. Estimated image boundaries always offer **Whole page** as a fallback.
Closing the figure returns to the same section. This is figure-reference detection, not model-verified
image segmentation.

`E` opens an actual CodeMirror 6 Markdown buffer. Notes map the PDF’s relative filename to `.md`
beneath `Notes/` (for example `local-articles/topic/Paper.pdf` → `Notes/topic/Paper.md`). Set another
root with `--notes /path/to/paper-notes`. Opening a missing note creates it from the configured Markdown template, with local date/time, the `paper` tag, and a link to its source PDF. Existing files are preserved.
The buffer opens in Vim Normal mode using [CodeMirror Vim](https://github.com/replit/codemirror-vim).
Motions, text objects, Visual modes, registers, macros, `/`/`?` search, `n`/`N`, substitution,
and undo/redo stay inside the note. Escape returns to Normal or cancels the current prompt;
repeated Escape never closes a focused buffer. `q` records a macro.

**U** and **Ctrl-R** redo; `u` undoes. Notes load the project's [`.vimrc`](.vimrc), with
portable mappings from the user's Neovim configuration. `:source` reloads remaps, options,
variables, conditionals, and command aliases without losing edits or undo history. Configure
another file with `"vim": { "vimrc": "/path/to/notes.vimrc" }` in `lysilogy.config.json`.
See [Vimrc support and its limits](docs/notes.md#vimrc) for the supported scripting subset.

`:w` or Ctrl/Cmd-S saves atomically; `:q` quits with an unsaved-changes prompt; `:wq`, `:x`,
and `ZZ` save and quit only after a successful save. `:q!` and `ZQ` discard the draft. Ordinary
saves detect external changes; explicit `:w!` or `:wq!` accepts the current disk version before
replacing it, with another revision check for edits made during that request. `Ctrl-w h`, `:reader`,
or `Ctrl-w w` focuses the reader; `Ctrl-w l` or `Ctrl-w w` returns to notes from the reader.
See [notes controls and configuration](docs/notes.md) for details.

The requests were recorded before implementation in [navigation issue #8](https://github.com/tjmisko/Lysilogy/issues/8)
and [source-search issue #9](https://github.com/tjmisko/Lysilogy/issues/9).

Note templates are configured in [`lysilogy.config.json`](lysilogy.config.json). The default uses
`YYYY-MM-DD` dates, `HH:mm` times, and `tags: ["paper"]` (rendered as a Markdown/YAML block list).
Edit the tags or template there and restart the backend; use `--config /path/to/settings.json` for
another settings file. Opening notes uses an idempotent `POST /api/papers/{id}/notes/open` operation,
so repeated `E` presses reuse the file. `GET /notes` remains read-only for checking external changes.
If the notes UI reports an unavailable API or an HTML response, restart the Rust backend as well as
updating the frontend. With Vite, the backend command is `cargo run -- serve --bind 127.0.0.1:7320`.
