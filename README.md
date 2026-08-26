# Lysilogos

Lysilogos turns a vault of scientific PDFs into a keyboard-first reading path for intelligent outsiders to the field.

The top bar deliberately increases detail in four steps: **Abstract** gives a generated one-sentence TL;DR, the authors' source-validated abstract, and a short contextual supplement; **Overview** maps the paper into conceptual tiles sized by argumentative weight and lays that map over coordinate-aligned PDF pages; **Glossary** teaches the load-bearing technical vocabulary to hold before reading; and **Text** provides both calm reconstructed Markdown and the source PDF. Reception, field history, and later interpretation appear only as cited notes tied to exact source records. Lysilogos independently follows each link to a successful public destination and records when it did so; the interface separately warns that reachability does not prove the source's semantic support. Opening an overview tile gives its contextual digest, key quotations, and page links. AI citations are prehighlighted only after deterministic text checks, and reader highlights use the same stable sentence/token anchors.

The current demo has been exercised against the local `local-articles/Articles` corpus (118 PDFs) and includes a mapped copy of Dijkstra's “GOTO Statements Considered Harmful.”

## Run it

Prerequisites:

- Rust 1.88 or newer
- Node.js 22 or newer
- Poppler's `pdftotext` and `pdfinfo`
- An authenticated `codex` or `claude` CLI for model-backed analysis (optional)

Build the web app, map one paper with the network-free analyzer, and serve everything:

```sh
cd web
npm install
npm run build
cd ..

cargo run -- analyze "GOTO Statements" --provider heuristic
cargo run -- serve
```

Open <http://127.0.0.1:7319>. The default vault is `local-articles/Articles`; both roots are configurable:

```sh
cargo run -- \
  --library /path/to/Articles \
  --data /path/to/lysilogos-data \
  serve
```

For frontend development, run `cargo run -- serve` and `npm run dev` from `web/` in separate terminals. Vite proxies `/api` to the Rust server.

For a normal local redeploy, rebuild both halves, stop the current foreground server with `Ctrl-C`, and launch the release binary again:

```sh
cd /home/tjmisko/Projects/Lysilogos/web
npm run build
cd ..
cargo build --release
./target/release/lysilogos serve
```

If the binary is already managed by a user service, replace the final line with `systemctl --user restart lysilogos.service`. Opening an older map lazily regenerates its coordinate extraction and validates existing quotes. To replace dashed legacy section extents with exact start/end spans, rerun that paper with `--force`; after evaluating the new prompt on a few papers, `ingest --provider codex --force` migrates the full mapped library.

## Map the vault

Discovery is recursive and incremental. Stable paper IDs are derived from paths, while generated material stays outside the source vault.

```sh
# Inventory the vault
cargo run -- scan

# Convert one PDF to portable Markdown on standard output
cargo run -- convert "title fragment"

# Evaluate the first few prompts before committing to a full run
cargo run -- ingest --provider codex --limit 3

# Map every paper not already ready; one failure does not discard other results
cargo run -- ingest --provider codex

# Alternative local CLI or network-free structural fallback
cargo run -- ingest --provider claude
cargo run -- ingest --provider heuristic

# Re-run one paper after changing a prompt or analyzer
cargo run -- analyze "title fragment" --provider codex --force
```

Lysilogos invokes the local command-line tools rather than an API. Codex runs ephemerally with live search, a read-only sandbox, and a JSON output schema; Claude runs in plan permission mode with paper-reading and web-research tools. The extracted paper is explicitly treated as untrusted input. Both commands have bounded runtime, validated structured output, and stage-specific failure reporting. External context uses a second, application-owned gate: every note must map to exact source records, and every cited URL must pass bounded DNS, redirect, public-address, and HTTP-success checks before either the note or source is persisted. One failed citation withholds the complete note.

The heuristic provider is intentionally conservative. It supplies an immediate offline atlas and clearly labels itself; use a model-backed provider for interpretive reading and field context.

## Keyboard model

Press `?` in the app for the complete, contextual key guide.

| Key | Action |
| --- | --- |
| `h j k l` or arrow keys | Move through tiles, panels, pages, or a visual text selection |
| `g g` / `G` | First / last tile |
| `Enter` or `o` | Open the focused section digest |
| `d` | Toggle the digest |
| `g` | Open the technical Glossary after a short single-key delay |
| `m` | Toggle Overview / reconstructed Text |
| `p` | Toggle Overview / source PDF |
| `[` / `]` | Previous / next paper, or PDF page |
| `/` | Search the active view |
| `v` | Begin keyboard text selection in a digest, or enter sentence marking in the source map |
| `o` | Swap the moving end of a visual selection |
| `c` | Clarify the selection in paper context |
| `y` | Copy the selection |
| `Space` | Persist the selected source sentence range as a reader highlight |
| `H` | Toggle AI-cited prehighlights |
| `U` | Toggle reader-created highlights |
| `I` | Invert every PDF rendering between dark ink and true image colors |
| `F1` | Toggle the library from anywhere |
| `F10` | Open the fuzzy article switcher |
| `f` | Toggle mapped-only filtering while the library is open |
| `Esc` | Return to Overview or close the top panel |

Native pointer selection also works: select text in a digest, then choose **Clarify selection**. In the page map, `v` enters a coordinate-backed evidence cursor; a second `v` starts a same-page sentence range, movement extends it, `Space` stores it, and `c` sends the exact sentence text into clarification. Same-page ranges keep one compact, relocation-resistant token anchor; cross-page notes should be saved as separate highlights.

## Plain-text artifacts

Generated files live beneath the selected data root:

```text
.lysilogos/
└── papers/
    └── <stable-paper-id>/
        ├── source.txt        # UTF-8 extraction, form-feed page boundaries
        ├── source.md         # best-effort full-document Markdown with page markers
        ├── layout.json       # PDF points, stable page-local tokens, and sentence segments
        ├── extraction.json   # extraction schema and normalized metadata
        ├── analysis.json     # typed, versioned application model
        ├── digest.md         # portable human-readable digest
        ├── highlights.jsonl  # canonical one-highlight-per-line records (AI and reader)
        ├── highlights.md     # generated human-readable highlight projection
        └── *.schema.json     # exact local-CLI output contracts
```

Writes are atomic. `analysis.json` keeps contextual notes, their source-ID mappings, exact source titles/authors/years, final working URLs, and link-check timestamps; `digest.md` renders the same citations for use outside the app. `highlights.jsonl` is the canonical plaintext-first highlight record: each line is a complete typed highlight with provenance, exact quoted text, PDF page, page-local token range, sentence IDs, and PDF-point rectangles. It is diffable and scriptable without a database; `highlights.md` is regenerated for ordinary reading. The original PDFs are never modified, and generated artifacts can be read, searched, versioned, or reused without running the frontend.

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

`npm run smoke` uses the real Dijkstra analysis, Markdown conversion, and PDF through an in-process browser route, so it works even in environments that block loopback networking. It verifies the four-level top-bar progression, abstract provenance, exact contextual sources and link-check scope, overview navigation, the mapped-only filter, F1/F10 switching, contextual digest, keyboard selection/clarification, the full Glossary, reconstructed Text, PDF rendering, and capital-`I` inversion.

The implementation map and fault boundaries are described in [docs/architecture.md](docs/architecture.md).
