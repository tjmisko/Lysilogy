# Markdown notes

Press **E** or use **Notes** to open a paper's Markdown buffer alongside the PDF. Opening notes temporarily collapses the library sidebar to leave room for reading; closing notes restores it. The editor wraps long lines and highlights headings, emphasis, links, lists, quotes, and code.

The buffer opens in Vim **Normal** mode. Use `i`, `a`, or `o` to start writing; **Esc** returns to Normal. The status line shows the current mode. Motions, counts, operators, visual selection, text objects such as `iw`, named registers, macros, and substitutions work inside the buffer. For example, `ciw` changes a word, `qa` starts recording macro `a`, `q` stops recording, and `@a` replays it. Undo uses `u`; redo uses **Ctrl-R**.

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

Validation: `cargo test notes::` exercises storage, create-on-open templates, existing-file preservation, API conflicts, traversal/symlink rejection, UTF-8 and size limits, and atomic updates. `npm --prefix web run smoke:notes` exercises the actual Vim editor, search and editing commands, pane focus, save/quit failure handling, old-backend HTML failures and retry, and local-time template creation with synthetic PDF and API fixtures. `npm --prefix web run test:api` checks response validation and timestamp formatting.
