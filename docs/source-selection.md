# Selecting PDF text

Click the source text or use `/` to search for a passage, then press `v` to enter
Visual selection. Search accepts regular expressions; `n` and `N` move between
matches. Selection works in paged or continuous reading, with either scrolling
direction, and within a selected section.

| Visual key | Motion or action |
| --- | --- |
| `h`, `l`, Left, Right | Move one character backward or forward. Emoji and combining characters stay intact. |
| `j`, `k`, Down, Up | Move to the following or preceding source line. |
| `w`, `b` | Move to the next or previous word start. |
| `W`, `B` | Move between starts of whitespace-delimited words. |
| `e`, `E` | Move to a word end; `E` uses whitespace-delimited words. |
| `ge`, `gE` | Move backward to a word end. |
| `(`, `)` | Move to the previous or next sentence start. |
| `iw`, `aw` | Select a word, or include its adjacent space. |
| `iW`, `aW` | Select a whitespace-delimited word, or include its adjacent space. |
| `is`, `as` | Select a sentence, or include its surrounding whitespace. |
| `ip`, `ap` | Select a paragraph, or include its surrounding whitespace. |
| `y` | Copy exactly the selected source text. |
| `o` | Swap the anchor and moving end of the selection. |
| `q`, Esc | Cancel the current selection or pending text-object command. |

Visual motions include the characters at both ends of the selection. For example,
starting on the `a` in `anti-noise`, `ve` selects `anti`, while `vE` selects
`anti-noise`. Sentence motions move to sentence starts; paragraph objects can
contain several sentences. `ip` excludes the separator between paragraphs, while
`ap` includes adjacent paragraph whitespace.
Motions accept counts such as `3e`, `2)`, and `5j` (up to 999). Counts on text-object
commands are not supported. Vertical motions continue through column and page
boundaries and work when the cursor is on paragraph whitespace. Inside Visual mode,
`E` belongs to WORD selection; outside it, `E` retains the Notes shortcut.

Highlights follow the selected characters within each word. Adjacent selected
words share a continuous line highlight that covers their spacing; separate lines
and columns retain separate rectangles. Native PDF text positions refine the
highlight when available. OCR and unavailable native character mappings use the
index's estimated geometry. Copying always uses the selected range in the source
index, preserving complete Unicode characters.

The reader keeps selected sections bounded. A search match outside the section
offers **Open match in full paper**. If clipboard access is denied, the exact text
remains available in a field for manual copying.

`npm --prefix web run smoke:source-search` checks these behaviors using synthetic
PDFs, including native character geometry, Unicode mappings, multi-sentence
paragraphs, and adjacent columns. Backend paragraph detection and the source index
format are described in [reading-index.md](reading-index.md).
