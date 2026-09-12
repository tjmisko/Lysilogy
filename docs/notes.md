# Markdown notes

Press **E** or use **Notes** to open a paper's Markdown buffer alongside the PDF. Opening notes temporarily collapses the library sidebar to leave room for reading; closing notes restores it. The editor supports configurable line wrapping and highlights headings, emphasis, links, lists, quotes, and code.

The buffer opens in Vim **Normal** mode. Use `i`, `a`, or `o` to start writing; **Esc** returns to Normal. The status line shows the current mode. Motions, counts, operators, visual selection, text objects such as `iw`, named registers, macros, and substitutions work inside the buffer. For example, `ciw` changes a word, `qa` starts recording macro `a`, `q` stops recording, and `@a` replays it. Undo uses `u`; redo uses **U** or **Ctrl-R**. `U` works even without a Vimrc file.

| Command | Action |
| --- | --- |
| `:w` | Save the Markdown file. **Save** and **Ctrl/Cmd-S** are aliases. |
| `:q` | Close notes; offer Save, Discard, or Keep editing if there are unsaved changes. |
| `:q!` | Close and explicitly discard the unsaved draft. |
| `:wq` or `:x` | Save changes, then close only after a successful save. `:x` does not write a clean buffer. |
| `:w!` or `:wq!` | Explicitly replace an external edit with this draft; `:wq!` then closes after success. |
| `/pattern`, `?pattern` | Search forward or backward within notes. |
| `n`, `N` | Repeat the search or reverse its direction. |
| `*`, `#` | Search forward or backward for the word under the cursor. |
| `:%s/old/new/g` | Replace all matches throughout the buffer. |
| **Ctrl-W**, then `h` | Focus the PDF to the left. |
| **Ctrl-W**, then `l` | Focus notes from the PDF. |
| **Ctrl-W**, then `w` | Switch between the PDF and notes. |

Inside the editor, **Esc** cancels Insert, Visual, search, Ex, or an unfinished operator. Repeated Esc in Normal mode keeps notes open. Use `:q` or the close button to leave. Esc on the unsaved-changes prompt cancels closing and returns focus to the buffer. Editor keys stay local: Normal-mode `q` records macros, `/` searches notes, and `:` opens Vim's command line. In Insert mode, `q` and `/` are ordinary text.

From the PDF, `:home` in the app's command menu returns to the paper library grid. Unsaved notes keep their Save, Discard, or Keep editing prompt. Use **Ctrl-W**, then `h` to focus the PDF before opening that menu from notes.

The default directory is `./Notes`, relative to the server's working directory. Running from this repository puts it at `/home/tjmisko/Projects/Lysilogy/Notes`. This is separate from the user's home-directory notes. Configure another directory with the global `--notes` option:

```sh
cargo run -- --library local-articles --data .lysilogy --notes ./paper-notes serve
```

A paper named `Author - Paper.pdf` uses `Author - Paper.md`. Subdirectories are preserved, so identically named PDFs in separate library folders have separate notes. Opening a missing note creates its Markdown file immediately with the reader's local date and time, a `paper` tag, and a link to the source PDF:

```markdown
---
date: 2026-09-11
time: 15:49
tags:
  - paper
---

## Sources
- [Author - Paper](<file:///path/to/Author%20-%20Paper.pdf>)
```

Reopening a file leaves its contents untouched, including an existing empty file. Notes survive analysis refreshes and remain ordinary UTF-8 files editable in other programs.

New-note defaults can be changed in the optional `./lysilogy.config.json`, or a JSON file passed with `--config`. For example:

```json
{
  "notes": {
    "date_format": "YYYY-MM-DD",
    "time_format": "HH:mm",
    "tags": ["paper", "reading"],
    "template": "---\ndate: {{date}}\ntime: {{time}}\ntags:\n{{tags}}\n---\n\n## Sources\n- [{{source_title}}](<{{source_url}}>)\n"
  }
}
```

The template supports `{{date}}`, `{{time}}`, `{{tags}}`, `{{source_title}}`, and `{{source_url}}`. Configuration changes apply to newly created notes. Restart the backend after changing configuration.

Leaving with unsaved changes offers **Save and close**, **Discard changes**, or **Keep editing**. Saves compare the file's SHA-256 content revision with the version opened by the editor. An observed external edit causes a conflict instead of replacing a stale version. **Compare with disk** shows the current file while retaining the draft; accepting the disk version explicitly discards that draft. Saves use a temporary file and atomic replacement; new-file creation cannot overwrite a file another writer created first. Existing-file checks are optimistic, not an exclusive lock on external editors.

An explicit forced write (`:w!` or `:wq!`) reads the latest disk revision before writing the current draft. A further external edit during that operation can still cause a conflict. Failed writes keep the draft and notes panel open, including when the command requested saving and quitting.

Opening uses `POST /api/papers/{id}/notes/open` with `{created_at: "2026-09-11T15:49:00-07:00"}`. This idempotently creates a missing note or returns the existing one. The timestamp preserves the browser's local offset. `GET /api/papers/{id}/notes` only reads and is used for comparing external changes; `PUT` to that route saves edits. All return `{filename, text, revision}`. On a read, `revision: null` denotes a missing file; send the last revision with every save. Conflicts return HTTP 409. Notes are limited to 2 MiB of UTF-8 text. The server accepts paths only from the paper catalog, rejects traversal and symlinks, and opens descendants relative to directory file descriptors with `O_NOFOLLOW`.

If the reader shows **Notes unavailable** and says the API returned a web page, an older backend or misdirected `/api` proxy is serving the app shell instead of the notes endpoint. Restart the backend with the current build, check the frontend API proxy, then use **Retry**. The reader also rejects malformed JSON or incompatible note responses with an actionable message; it never displays the returned HTML or a raw JSON parser error.

The buffer uses CodeMirror 6 with `@replit/codemirror-vim` and a small Markdown decoration extension. Dependency versions are pinned in `web/package-lock.json`; the [Vim archive provenance and build instructions](../web/vendor/README.md) document the exact upstream release. File commands operate on the current paper's Markdown note. The optional full Markdown language package is not required by this implementation.

## Vimrc

Notes load `.vimrc` beside `lysilogy.config.json` each time the buffer opens. Without an
application config, `.vimrc` is relative to the server's working directory. The checked-in
[file](../.vimrc) ports these mappings from `linux-config/common/.config/nvim/lua/goose/remap.lua`:

| Keys | Behavior |
| --- | --- |
| `U` | Redo, keeping Ctrl-R available. |
| Ctrl-D / Ctrl-U, PageDown / PageUp | Move half a screen and center the cursor. Page keys also work in Insert mode. |
| `n`, `N`, `*` | Center after a search motion. |
| `;`, `,` | Reverse their usual character-find repeat bindings. |
| Home | Move to the first nonblank character. |
| Space, `p` in Visual mode | Replace the selection while preserving the paste register. |

The file also sets absolute/relative line numbers, four-space indentation, and `nowrap`.
Edit it directly and run **`:source`** in the note to reload it. `:source .vimrc` or the
configured path also works. Reload preserves unsaved text, undo/redo history, and registers;
removed remaps return to defaults. Fetch failures keep the existing configuration. Unsupported
lines appear in an expandable **Vimrc warnings** message and do not disable the editor.
Normal/Visual mapping prefixes wait up to one second for another key, then fall back to their
ordinary motion or shorter mapping. Escape cancels a pending prefix. Interactive remaps and
`:unmap` / `:mapclear` use the same configuration owner; `:source` replaces those session edits
with the current file.

To choose another file, add this to the existing application config and restart the backend:

```json
"vim": { "vimrc": "/path/to/notes.vimrc" }
```

Relative paths resolve beside the config file. Vimrc contents are fetched afresh on opening
or `:source`; editing the file itself does not require restarting. A missing file is optional
and uses the built-in bindings. The read-only API `GET /api/notes/vimrc` returns
`{path,text,exists}` with `Cache-Control: no-store`; files must be regular UTF-8 files of at
most 64 KiB. `:source` reloads only the configured file, not arbitrary browser-supplied paths.

The loader implements a **Vimscript subset**, not the Neovim Lua runtime:

- `map` / `noremap`, mode-specific `n`, `i`, `v`, `x`, and `o` variants, unmapping, and map clearing.
  `noremap` prevents recursive expansion, so swaps and `n` → `nzz` work. `<silent>` and
  `<buffer>` are accepted; `<Leader>`, `<LocalLeader>`, standard key notation, and `<Nop>` work.
- Scalar `let` assignments, string concatenation, `execute` to generate configuration,
  `if` / `elseif` / `else` / `endif`, comparisons, Boolean operators, `exists()`, and
  `has('lysilogy')` guards. Variables are evaluated within one file load.
- `command[!] Name ex-command` defines an argument-free command alias invoked later in the
  buffer. Alias recursion is bounded. Existing `:w`, `:wq`, `:q!`, and other file commands
  retain their save/conflict behavior.
- `set` / `setlocal` support `number` (`nu`), `relativenumber` (`rnu`), `wrap`, `expandtab`
  (`et`), `tabstop` (`ts`), `shiftwidth` (`sw`), `textwidth` (`tw`), `pcre`, and
  `insertModeEscKeysTimeout`. Boolean negation, inversion, and `&` default resets work.
  `:set` also applies these options interactively; all settings belong to the notes editor.

For example:

```vim
let mapleader = " "
nnoremap U <C-r>
inoremap jk <Esc>
let g:save_key = '<leader>w'
if has('lysilogy') && !exists('g:disable_save_key')
  execute 'nnoremap ' . g:save_key . ' :write<CR>'
  command! SaveAndQuit wq
endif
```

Lua callbacks/plugins, Vim functions, autocommands, loops, expression mappings, shell commands,
and nested file sourcing are outside this subset. Unsupported script blocks are skipped as
blocks so their interior does not become accidental configuration. Neovim-only filesystem,
buffer/tab, quickfix, timestamp-function, and Markdown-fence clipboard mappings from the
original Lua config are not imported. PDF-reader keybindings are separate from the notes Vimrc.
Operator-pending multikey mappings that begin with a complete native motion still use the
engine's matching behavior, where that native motion can take precedence. Insert mappings
use the engine's escape-sequence timeout, configurable with `insertModeEscKeysTimeout`.

`npm --prefix web run test:vimrc` verifies the compiler; `npm --prefix web run smoke:notes-vimrc`
exercises actual remaps, scripts, live reload, and save/quit in the browser, including the
checked-in Vimrc. Backend tests cover missing files, reload, path resolution, and size/type limits.

Validation: `cargo test notes::` exercises storage, create-on-open templates, existing-file preservation, API conflicts, traversal/symlink rejection, UTF-8 and size limits, and atomic updates. `npm --prefix web run smoke:notes` exercises the actual Vim editor, search and editing commands, pane focus, save/quit failure handling, old-backend HTML failures and retry, and local-time template creation with synthetic PDF and API fixtures. `npm --prefix web run test:api` checks response validation and timestamp formatting.
