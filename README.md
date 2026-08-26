# Lysilogy

Lysilogy turns a vault of scientific PDFs into a keyboard-first reading path for intelligent
outsiders to a field. It reads the vault and never writes to it: every artifact it generates is
plain, diffable text stored outside the source library.

The current demo has been exercised against a local corpus of 118 PDFs and ships with a mapped copy
of Dijkstra's "GOTO Statements Considered Harmful."

## The reading ladder

The top bar is a monotonic ramp — each level is strictly more detailed than the one before it.

| Level | What it gives you |
| --- | --- |
| **Abstract** | A generated one-sentence thesis, the authors' own abstract, and at most two externally sourced context notes on field history, reception, or later interpretation. |
| **Overview** | A resizable grid of every PDF page, with section transitions projected across each page cell, plus a secondary tile map where area expresses conceptual weight and color expresses argumentative role. |
| **Glossary** | The load-bearing technical vocabulary to hold in your head before reading. |
| **Text** | Calm reconstructed Markdown alongside the source PDF. |

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

For frontend work, run `cargo run -- serve` and `npm run dev` (from `web/`) in separate terminals;
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
| `--library` | `LYSILOGY_LIBRARY` | `local-articles/Articles` |
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

Lysilogy drives the local command-line tools rather than an API. Each run starts an agent in the
paper's artifact directory with the paper, the current analysis, and the output schema as read-only
context; the agent may edit exactly one generated file, `analysis-tasklist.md`. Codex gets live
search in a workspace-write sandbox; Claude gets paper-reading, tasklist-editing, and web-research
tools. Sessions are persisted so reader feedback can continue the same conversation, and a session
that cannot be resumed falls back to a fresh agent with `source.txt`, `analysis.json`, the tasklist,
and the full feedback already in its working directory. Clarification requests stay ephemeral and
read-only.

Every analysis and feedback retry writes a live Markdown tasklist. Press `q` to watch the agent mark
work active and complete — progress is computed directly from those checkboxes.

The heuristic provider is deliberately conservative. It gives you an immediate offline Overview and
labels itself plainly; use a model-backed provider for interpretive reading and field context.

## Keyboard model

Press `?` in the app for the complete, contextual guide.

| Key | Action |
| --- | --- |
| `h j k l` or arrows | Move through tiles, panels, pages, or a visual text selection |
| `g g` / `G` | First / last tile |
| `Enter` or `o` | Open the focused section digest |
| `d` | Toggle the digest |
| `g` | Open the Glossary after a short single-key delay |
| `m` | Toggle Overview / reconstructed Text |
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
| `q` | Toggle the processing queue and live agent tasklists |
| `Esc` | Return to Overview, or close the top panel |

Pointer selection works too: select text in a digest, then choose **Clarify selection**. In the page
map, `v` enters a coordinate-backed evidence cursor; a second `v` starts a same-page sentence range,
movement extends it, `Space` stores it, and `c` sends the exact sentence text into clarification.
Same-page ranges keep one compact, relocation-resistant token anchor; save cross-page notes as
separate highlights.

## Artifacts on disk

Everything generated lives beneath the data root:

```text
.lysilogy/
└── papers/
    └── <stable-paper-id>/
        ├── source.txt           # UTF-8 extraction, form-feed page boundaries
        ├── source.md            # full-document Markdown with page markers
        ├── layout.json          # PDF points, stable page-local tokens, sentence segments
        ├── extraction.json      # extraction schema and normalized metadata
        ├── analysis.json        # typed, versioned application model
        ├── digest.md            # portable human-readable digest
        ├── highlights.jsonl     # canonical one-highlight-per-line records (AI and reader)
        ├── highlights.md        # generated human-readable projection
        ├── analysis-tasklist.md # live checklist edited by the local coding agent
        ├── job.json             # typed queue/progress state for the latest run
        ├── agent-session.json   # resumable Codex or Claude session identity
        ├── feedback.jsonl       # one reader feedback record per line
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
```

`npm run smoke` drives the real Dijkstra analysis, Markdown conversion, and PDF through an
in-process browser route, so it works even where loopback networking is blocked. It covers the
four-level top bar, abstract provenance, contextual sources and link-check scope, the all-page
Overview grid, horizontal section progress and integer-column zoom, the mapped-only filter, F1/F10
switching, the command menu, live tasklist progress, feedback retries, keyboard selection and
clarification, the Glossary, reconstructed Text, one/two-page PDF paging, and capital-`I` inversion.

The implementation map and fault boundaries are in [docs/architecture.md](docs/architecture.md).
