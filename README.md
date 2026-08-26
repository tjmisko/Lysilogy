# Lysilogos

Lysilogos turns a vault of scientific PDFs into a keyboard-first visual atlas for readers who are intelligent outsiders to the field.

The main view maps a paper into conceptual tiles sized by argumentative weight. Focus or hover gives the short reading; opening a tile gives its contextual digest, key quotations, and page links. A searchable **Gloss** explains technical language, while the PDF reader defaults to dark rendering and can reveal figures in their original colors.

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

## Map the vault

Discovery is recursive and incremental. Stable paper IDs are derived from paths, while generated material stays outside the source vault.

```sh
# Inventory the vault
cargo run -- scan

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

Lysilogos invokes the local command-line tools rather than an API. Codex runs ephemerally with a read-only sandbox and a JSON output schema; Claude runs in plan permission mode with only read/search tools. The extracted paper is explicitly treated as untrusted input. Both commands have bounded runtime, validated structured output, and stage-specific failure reporting.

The heuristic provider is intentionally conservative. It supplies an immediate offline atlas and clearly labels itself; use a model-backed provider for interpretive reading and field context.

## Keyboard model

Press `?` in the app for the complete, contextual key guide.

| Key | Action |
| --- | --- |
| `h j k l` | Move through tiles, panels, or a visual text selection |
| `g g` / `G` | First / last tile |
| `Enter` or `o` | Open the focused section digest |
| `d` | Toggle the digest |
| `g` | Open Gloss after a short single-key delay |
| `p` | Toggle atlas / PDF |
| `[` / `]` | Previous / next paper, or PDF page |
| `/` | Search the active view |
| `v` | Begin keyboard text selection in a digest |
| `o` | Swap the moving end of a visual selection |
| `c` | Clarify the selection in paper context |
| `y` | Copy the selection |
| `i` | Invert the PDF between dark ink and true image colors |
| `Esc` | Leave the current mode or close the top panel |

Native pointer selection also works: select text in a digest, then choose **Clarify selection**. This avoids replacing browser text semantics with a custom editor while still making the entire clarification flow keyboard-accessible.

## Plain-text artifacts

Generated files live beneath the selected data root:

```text
.lysilogos/
└── papers/
    └── <stable-paper-id>/
        ├── source.txt        # UTF-8 extraction, form-feed page boundaries
        ├── extraction.json   # extraction schema and normalized metadata
        ├── analysis.json     # typed, versioned application model
        ├── digest.md         # portable human-readable digest
        └── *.schema.json     # exact local-CLI output contracts
```

Writes are atomic. The original PDFs are never modified, and generated artifacts can be read, searched, versioned, or reused without running the frontend.

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

`npm run smoke` uses the real Dijkstra analysis and PDF through an in-process browser route, so it works even in environments that block loopback networking. It verifies atlas navigation, contextual digest, keyboard selection/clarification, Gloss, and PDF rendering.

The implementation map and fault boundaries are described in [docs/architecture.md](docs/architecture.md).
