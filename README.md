# Lysilogy

Lysilogy turns a vault of scientific PDFs into a keyboard-first reading path for intelligent
outsiders to a field. It preserves source PDFs and adds imported papers only on request. Every
generated artifact is plain, diffable text stored outside the source library.

**Lysilogos** is the agent who cuts papers and follows their references inside Lysilogy.

The current demo has been exercised against a local corpus of 118 PDFs and ships with a mapped copy
of Dijkstra's "GOTO Statements Considered Harmful."

## The reading ladder

The top bar is a monotonic ramp — each level is strictly more detailed than the one before it.

| Level | What it gives you |
| --- | --- |
| **Abstract** | A generated one-sentence thesis, the authors' own abstract, and at most two externally sourced context notes on field history, reception, or later interpretation. |
| **Overview** | A resizable grid of every PDF page, with section transitions projected across each page cell, plus a secondary tile map where area expresses conceptual weight and color expresses argumentative role. |
| **Glossary** | The load-bearing technical vocabulary to hold in your head before reading. |
| **Text** | The selectable source PDF by default; reconstructed Markdown is available on request. |

Two provenance rules hold everywhere:

- The authors' abstract is retained only when its normalized text is actually present in the
  extraction. Generated orientation stays visibly separate from the authors' words.
- Every context note maps to exact source records, and every cited URL must pass bounded DNS,
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
```

Opening an older map lazily regenerates its coordinate extraction and revalidates existing quotes.
To replace dashed legacy section extents with exact start/end spans, rerun that paper with
`--force`; `ingest --provider codex --force` migrates the whole mapped library.

### How analysis runs

Lysilogy drives local command-line tools rather than an API. There are five distinct prompt
templates: three for an initial analysis (orientation, structure/evidence, and external context),
one for feedback revision, and one for passage clarification. Initial analysis prefetches paper
metadata, headings, page-marked text, opening/closing context, and a deterministic authored abstract
once, then starts its three scoped calls concurrently. Only the external-context branch gets live
web tools; the structure branch receives local read tools only when a very large paper had to be
sampled.

For Codex, the small orientation and clarification jobs use `gpt-5.6-luna` at low effort. Structural
analysis, sourced context, and revision use `gpt-5.6-terra` at medium effort. Claude receives the
same scoped tools and low/medium effort split while retaining its configured model. Abstract
extraction normally costs no model call; when deterministic extraction finds no abstract, the fast
orientation call may return an exact candidate, which still has to pass source-text verification.

Each initial branch writes a typed stage artifact as soon as it succeeds. A retry reuses matching
stages and reruns only missing or malformed ones; `--force` deliberately invalidates this stage
cache. Only the structural call retains a resumable session for feedback. If that session cannot be
resumed, revision falls back to a fresh read-only call with `source.txt`, `analysis.json`, and the
feedback already present. Clarification stays ephemeral and uses prefetched local passage context.

The backend owns `analysis-tasklist.md` and its typed `job.json` state; model processes are read-only
and never edit progress. Press `q` to watch the three initial branches run in parallel.

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

Open **Lysilogos** in the toolbar, or use `:supercut` and `:references`.

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
| `Enter` or `o` | Open the focused section digest |
| `d` | Toggle the digest |
| `g` | Open the Glossary after a short single-key delay |
| `m` | Toggle source PDF / requested Markdown reconstruction |
| `p` | Toggle Overview / source PDF |
| `2` | Toggle one-page / two-page PDF view |
| `+` / `-` | One fewer / one more page column in Overview (up to 10) |
| `[` / `]` | Previous / next paper, or PDF page |
| `Ctrl-d` / `Ctrl-u` | Page forward / back in PDF; half-screen in text views |
| `PageDown` / `PageUp` | Page forward / back in PDF; full-screen in text views |
| `/` | Search the active view |
| `v` | Start keyboard selection in a digest, or sentence marking in the source map |
| `o` | Swap the moving end of a visual selection |
| `c` | Clarify the selection in paper context |
| `y` | Copy the selection |
| `Space` | Persist the selected source sentence range as a reader highlight |
| `H` / `U` | Toggle AI-cited prehighlights / reader highlights |
| `I` | Invert every PDF rendering between dark ink and true image colors |
| `F1` | Toggle the library from anywhere |
| `F10` | Open the fuzzy article switcher |
| `f` | Filter to mapped papers while the library is open |
| `:` | Command menu (`:analyze`, `:queue`, `:feedback`, `:spread`, and more) |
| `:experiment` | Open the blind learning-ramp A/B prompt lab. |
| `q` | Toggle the processing queue and live analysis tasklists |
| `Esc` | Return to Overview, or close the top panel |

Pointer selection works too: select text in a digest, then choose **Clarify selection**. In the page
map, `v` enters a coordinate-backed evidence cursor; a second `v` starts a same-page sentence range,
movement extends it, `Space` stores it, and `c` sends the exact sentence text into clarification.
Same-page ranges keep one compact, relocation-resistant token anchor; save cross-page notes as
separate highlights.

PDF mode renders a PDF.js text layer over the page image. Drag over a passage—even across lines or
the two-page spread—to copy it or open **Ask about this** with the passage and its first page already
filled in. Selections retain PDF-page coordinates and text-item offsets for future persistent marks.
Image-only pages report that OCR is required instead of presenting an inert selection surface.

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
npm run smoke
npm run smoke:reader-tools
```

`npm run smoke` drives the real Dijkstra analysis, Markdown conversion, and PDF through an
in-process browser route, so it works even where loopback networking is blocked. It covers the
four-level top bar, abstract provenance, contextual sources and link-check scope, the all-page
Overview grid, horizontal section progress and integer-column zoom, the mapped-only filter, F1/F10
switching, the command menu, live tasklist progress, feedback retries, keyboard selection and
clarification, the Glossary, reconstructed Text, selectable one/two-page PDF paging, and capital-`I`
inversion.

`smoke:reader-tools` uses self-contained browser fixtures to exercise both cut formats, six-page
PDF output, Markdown export, saved citations, paper linking, search, comparison, retry, keyboard focus,
and mobile layout. Backend tests use local stub CLIs to exercise task execution and source validation
without model calls. For the original corpus-based smoke test in a worktree, set
`LYSILOGY_SMOKE_FIXTURE_ROOT` to the checkout containing the cached paper. Set
`LYSILOGY_SMOKE_PDF` as well if its PDF has moved from the default fixture path.

The implementation map and fault boundaries are in [docs/architecture.md](docs/architecture.md).
The proposed delivery phases for abstract fidelity, cited before/after context, and a focused
section reader are in [the reading pipeline plan](docs/reading-pipeline-plan.md).
