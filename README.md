# Lysilogos

Lysilogos turns a vault of scientific PDFs into a keyboard-first visual atlas for readers who are intelligent outsiders to the field.

The main view maps a paper into conceptual tiles sized by argumentative weight, then lays the verified map directly over coordinate-aligned PDF pages. Focus or hover gives the short reading; opening a tile gives its contextual digest, key quotations, and page links. AI citations are prehighlighted only after deterministic text checks. Reader highlights use the same stable sentence/token anchors. A searchable **Gloss** explains technical language. A reconstructed Markdown tab provides a calm, selectable reading surface, while every PDF canvas defaults to dark rendering and can reveal figures in its original colors.

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

Lysilogos invokes the local command-line tools rather than an API. Analysis sessions are persisted so reader feedback can continue the same Codex or Claude conversation. Codex uses a workspace-write sandbox and Claude receives read/search/edit tools, but both are explicitly restricted to editing one generated file: `analysis-tasklist.md`. The paper, current atlas, and schemas remain read-only context. JSONL/session metadata and the final schema-validated answer are captured separately, with bounded runtime and stage-specific failures. Clarification requests remain ephemeral and read-only.

Every analysis and feedback retry creates a live Markdown tasklist. Open the queue with `q` to watch the agent mark work active and complete; progress is calculated directly from those checkboxes. If a saved session cannot be resumed, the retry falls back to a fresh agent with `source.txt`, `analysis.json`, the tasklist, and the complete feedback available in its working directory.

The heuristic provider is intentionally conservative. It supplies an immediate offline atlas and clearly labels itself; use a model-backed provider for interpretive reading and field context.

## Keyboard model

Press `?` in the app for the complete, contextual key guide.

| Key | Action |
| --- | --- |
| `h j k l` or arrow keys | Move through tiles, panels, pages, or a visual text selection |
| `g g` / `G` | First / last tile |
| `Enter` or `o` | Open the focused section digest |
| `d` | Toggle the digest |
| `g` | Open Gloss after a short single-key delay |
| `m` | Toggle atlas / reconstructed Markdown |
| `p` | Toggle atlas / PDF |
| `2` | Toggle one-page / two-page PDF view |
| `[` / `]` | Previous / next paper, or PDF page |
| `Ctrl-d` / `Ctrl-u` | Page forward / back in PDF; half-screen down / up in text views |
| `PageDown` / `PageUp` | Page forward / back in PDF; full-screen down / up in text views |
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
| `:` | Open the command menu (`:analyze`, `:queue`, `:feedback`, `:spread`, and more) |
| `q` | Toggle the processing queue and live agent tasklists |
| `Esc` | Leave the current mode or close the top panel |

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
        ├── analysis-tasklist.md # live checklist edited by the local coding agent
        ├── job.json          # typed queue/progress state for the latest run
        ├── agent-session.json # resumable Codex or Claude session identity
        ├── feedback.jsonl    # one plaintext-friendly reader feedback record per line
        └── *.schema.json     # exact local-CLI output contracts
```

Writes are atomic. `highlights.jsonl` is the canonical plaintext-first record: each line is a complete typed highlight with provenance, exact quoted text, PDF page, page-local token range, sentence IDs, and PDF-point rectangles. It is diffable and scriptable without a database; `highlights.md` is regenerated for ordinary reading. The original PDFs are never modified, and generated artifacts can be read, searched, versioned, or reused without running the frontend.

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

`npm run smoke` uses the real Dijkstra analysis, Markdown conversion, and PDF through an in-process browser route, so it works even in environments that block loopback networking. It verifies atlas navigation, the mapped-only filter, F1/F10 switching, the colon command menu, live tasklist progress, feedback retries, contextual digest, keyboard selection/clarification, Gloss, Markdown reading, one/two-page PDF rendering and paging, and capital-`I` inversion.

The implementation map and fault boundaries are described in [docs/architecture.md](docs/architecture.md).
