# Markdown notes

Press **E** or use **Notes** to open a paper's Markdown buffer alongside the PDF. Opening notes temporarily collapses the library sidebar to leave room for reading; closing notes restores it. **Save** or **Ctrl/Cmd-S** writes the file. Ordinary typing, including `q` and `/`, stays in the editor. Undo uses **Ctrl/Cmd-Z**; redo uses **Ctrl/Cmd-Shift-Z** or **Ctrl-Y**. The editor wraps long lines and highlights headings, emphasis, links, lists, quotes, and code.

The default directory is `./Notes`, relative to the server's working directory. Running from this repository puts it at `/home/tjmisko/Projects/Lysilogy/Notes`. This is separate from the user's home-directory notes. Configure another directory with the global `--notes` option:

```sh
cargo run -- --library local-articles --data .lysilogy --notes ./paper-notes serve
```

A paper named `Author - Paper.pdf` uses `Author - Paper.md`. Subdirectories are preserved, so identically named PDFs in separate library folders have separate notes. Opening a missing note shows an empty buffer and creates nothing; the first save creates its directory and Markdown file. Notes survive analysis refreshes and remain ordinary UTF-8 files editable in other programs.

Leaving with unsaved changes offers **Save and close**, **Discard changes**, or **Keep editing**. Saves compare the file's SHA-256 content revision with the version opened by the editor. An observed external edit causes a conflict instead of replacing a stale version. **Compare with disk** shows the current file while retaining the draft; accepting the disk version explicitly discards that draft. Saves use a temporary file and atomic replacement; new-file creation cannot overwrite a file another writer created first. Existing-file checks are optimistic, not an exclusive lock on external editors.

The API is `GET /api/papers/{id}/notes` and `PUT` to the same route. Both return `{filename, text, revision}`. `revision: null` denotes a missing file; send the last revision with every save. Conflicts return HTTP 409. Notes are limited to 2 MiB of UTF-8 text. The server accepts paths only from the paper catalog, rejects traversal and symlinks, and opens descendants relative to directory file descriptors with `O_NOFOLLOW`.

The buffer uses the official `@codemirror/state` and `@codemirror/view` packages, with a small Markdown decoration extension. Package versions and npm integrity hashes are pinned in `web/package-lock.json`. Network package downloads were unavailable in the implementation environment, so these official packages were obtained from an existing local installation; the lockfile has normal public npm URLs and no dependency on that local project. The optional full Markdown language package is not required by this implementation.

Validation: `cargo test notes::` exercises storage and API conflicts, lazy creation, traversal/symlink rejection, UTF-8 and size limits, and atomic updates. `npm --prefix web run smoke:notes` exercises the actual CodeMirror editor with synthetic PDF and API fixtures.
