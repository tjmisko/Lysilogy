# Ground-truth kit

This directory holds the source papers and the prompt needed to build the
identity/abstract/sections oracle for the spike with a strong model (e.g. GPT
Sol or Claude). The model's output is spot-checked by hand and then promoted
into `eval/spike/sample.json`.

## Layout

- `inputs/` — one PDF per paper, named by stem (see `manifest.json`).
- `prompt.md` — the instructions to give the model, one paper at a time.
- `oracle.schema.json` — the exact JSON shape the model must return.
- `manifest.json` — the papers, their stem, page count, and the fields to fill.
- `oracles/` — where the returned JSON is saved, `<stem>.json`.

## Procedure

1. For each entry in `manifest.json`, give the model the matching
   `inputs/<stem>.pdf` plus `prompt.md`.
2. Save the model's JSON to `oracles/<stem>.json`.
3. Spot-check against the first page of the PDF: title, author order, and that
   the abstract is verbatim. Fix or reject as needed.
4. Tell the spike runner to use it: the entry becomes
   `"oracle": <the JSON>` in `eval/spike/sample.json`.

Papers already carrying a LaTeX-derived or hand-checked oracle do not need this
kit; it exists for the local and image-only papers.
