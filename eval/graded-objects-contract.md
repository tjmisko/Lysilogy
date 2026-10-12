# Graded object truth

Human-graded figure/table truth built inside the reader. It replaces the LaTeX-derived,
byte-replayed K1 visual releases as the active owner of O1/O2, while keeping the same matching
arithmetic. Adding a paper is a matter of grading it in the reader and rerunning one script; no
per-paper review protocol, receipts or codec changes are required.

Design decisions (2026-09-15):

- Truth is reviewed data. A grades file is plain JSON that one person writes through the
  reader UI. It is versioned by Git like any other data file after export.
- Grading is detector-informed. The grader sees what the detector found and confirms, corrects,
  rejects or adds. Adding missed objects is a first-class action so misses are not invisible.
- Scoring reuses `evaluate_paper` from `scripts/eval/object-metrics.py` unchanged, so O1/O2
  are computed by the same code as every historical release.

## Files

| File | Written by | Purpose |
| --- | --- | --- |
| `<data>/papers/<paper_id>/objects-grades.json` | reader (`PUT /api/papers/{id}/objects/grades`) | One grader's verdicts on the detector output for one paper |
| `<data>/grading-queue.json` | `scripts/eval/graded-objects.py queue` | Stratified order in which to grade papers |
| `eval/truth/k1-graded/papers/<paper_id>.json` | `scripts/eval/graded-objects.py export` | Derived truth child, one per complete paper |
| `eval/truth/k1-graded/objects.json` | `scripts/eval/graded-objects.py export` | Release manifest with cohorts and denominators |
| `eval/inputs/objects/figure-table.json` | `scripts/eval/graded-objects.py measure` | Harness input owning O1 and O2 |
| `eval/inputs/evidence/graded-objects.json` | `scripts/eval/graded-objects.py measure` | Per-paper outcomes for the iteration loop |

## Grades file

`<data>/papers/<paper_id>/objects-grades.json`, schema 1.

```json
{
  "schema_version": 1,
  "paper_id": "e51f6f61ec6d4107",
  "index_sha256": "f40e9c89…",
  "objects_generation": "3b1c…",
  "grader": "tjmisko",
  "updated_at": "2026-09-15T20:11:00-07:00",
  "revision": "9a0c…",
  "complete": false,
  "verdicts": {
    "fig-3": { "verdict": "correct", "region": null, "note": "" },
    "tab-1": { "verdict": "region", "region": { "x_min": 60, "y_min": 100, "x_max": 540, "y_max": 300 }, "note": "" },
    "fig-9": { "verdict": "reject", "region": null, "note": "prose reference, not a caption" }
  },
  "additions": [
    { "id": "add-1", "kind": "figure", "printed_label": "4", "page": 3,
      "region": { "x_min": 72, "y_min": 80, "x_max": 300, "y_max": 260 },
      "caption": { "start": 12345, "end": 12480 }, "note": "" }
  ]
}
```

Fields:

- `paper_id` must equal the path's paper id.
- `index_sha256` is the SHA-256 of the paper's `reading-index.json` bytes. It equals
  `ObjectsArtifact.reading_index_generation` with its surrounding quotes removed. The server
  rejects a save whose value differs from the currently cached index (`409 grades_stale_index`).
- `objects_generation` is `ObjectsArtifact.figure_detector_generation` at grading time. It is
  informational: derived truth does not depend on detector object ids surviving a detector change.
- `grader` is free text; the server fills it from `$USER` when empty.
- `updated_at` and `revision` are set by the server. `revision` is the SHA-256 of the stored
  bytes. A `PUT` carries the revision it loaded (or `null` for a new file); a mismatch is
  `409 grades_conflict` and returns the current document.
- `complete` means every page has been reviewed and every detector object has a verdict. Only
  complete papers enter the O1/O2 cohorts. The server does not enforce the verdict coverage; the
  export step does.
- `verdicts` is keyed by detector object id from `objects.json`. `verdict` is one of:
  - `correct`: kind, printed label, caption and region are right. Truth region is the
    detector's `region`; a `correct` verdict on an object with `region: null` is rejected by
    export (grade it `region` and draw the box instead).
  - `region`: kind, label and caption are right; `region` supplies the corrected box.
  - `reject`: no truth object. Use for prose misread as captions, duplicate captions, and wrong
    labels. If the object exists with a different label, reject it and add it.
- `additions` are objects the detector missed. `kind` is `figure` or `table`. `printed_label`
  is the number as printed (`4`, `IV`, `S1`, `2.3`). `caption` is a half-open UTF-16 span in the
  reading-index text, normally proposed by the UI from the index's caption paragraphs on that
  page and confirmed by the grader. An addition with `caption: null` makes the paper
  unexportable; the export step reports it.

Limits enforced by the server: body ≤ 1 MiB; ≤ 2,000 verdicts and ≤ 2,000 additions; notes
≤ 2,000 characters; `printed_label` 1–32 characters; regions finite with `x_min < x_max` and
`y_min < y_max`; addition ids unique and 1–64 characters; `caption.start < caption.end`.

## Endpoints

- `GET /api/papers/{id}/objects/grades` → the grades document, or `404 grades_not_found`.
- `PUT /api/papers/{id}/objects/grades` with the document (`revision` as loaded) → stored
  document with new `revision` and `updated_at`.
- `GET /api/grading/queue?limit=200` → `{ "summary": { "complete": n, "partial": n, "ungraded": n },
  "papers": [ { "paper_id", "title", "stratum", "status": "complete" | "partial" | "ungraded",
  "position": 0 } ] }`. Order follows `<data>/grading-queue.json` when present, else catalog
  order. `stratum` is null without a queue file.
- `GET /api/grading/next?after={paper_id}` → `{ "paper_id", "title" }` for the first paper after
  `after` (or from the start) whose status is not `complete`; `404 grading_queue_exhausted`.

## Queue file

`<data>/grading-queue.json`, schema 1:

```json
{ "schema_version": 1, "seed": 1, "tier": "eval",
  "papers": [ { "paper_id": "e51f…", "arxiv_id": "2104.01511", "stratum": "cs" } ] }
```

Built from the corpus `selection.json`: papers in the requested tier, grouped by the field of
their primary category (`math`, `cs`, `physics`, `stat`, `econ`, `other`), shuffled with the seed
inside each group, then interleaved round-robin so any prefix is roughly stratified.

## Truth derivation

`graded-objects.py export` reads every grades file with `complete: true`, loads the paper's
`objects.json` through the production bridge (same detector build the measurement will use) and
emits `eval/truth/k1-graded/papers/<paper_id>.json`:

```json
{
  "paper_id": "e51f6f61ec6d4107", "arxiv_id": "2104.01511", "arxiv_version": 1,
  "pdf_sha256": "f99c…", "index": { "path": "papers/e51f6f61ec6d4107/reading-index.json", "sha256": "f40e…" },
  "provenance": { "source": "graded", "grader": "tjmisko", "graded_at": "…", "grades_sha256": "…", "objects_generation": "…" },
  "metric_eligibility": { "O1": true, "O2": true },
  "counts": { "figure": 10, "table": 5 },
  "reviewed_absent_kinds": [],
  "objects": [
    { "id": "graded:fig-3", "kind": "figure", "printed_label": "3",
      "spans": [ { "start": 9092, "end": 9262 } ],
      "region": [ { "page": 2, "rect": { "x_min": 332.25, "y_min": 261.75, "x_max": 540.75, "y_max": 338.25 } } ] }
  ]
}
```

Rules:

- `correct` → object copied with `spans = [anchor]`, `printed_label` from the detector label,
  `region = [{page, rect: detector region}]`.
- `region` → as above with the corrected rect.
- `reject` → omitted.
- addition → `id = "graded:" + addition.id`, `spans = [caption]`, `region = [{page, rect}]`.
- A detector object without a verdict, a `correct` verdict without a region, or an addition
  without a caption makes the paper ineligible; export lists the reason and skips it.
- `arxiv_id`/`arxiv_version`/`pdf_sha256` come from `<data>/paper-identities.json`.
- Children imported from an earlier release (`graded-objects.py seed --from eval/truth/k1-limited-v5`)
  keep `provenance.source = "k1-limited-v5"` and are kept unless a grades file for the same
  paper supersedes them.

`eval/truth/k1-graded/objects.json`:

```json
{ "schema_version": 1, "truth_set": "K1", "origin": "graded", "version": "k1-graded",
  "layout": "k1-graded-v1", "build_date": "…",
  "papers": [ { "paper_id", "path": "papers/<id>.json", "sha256", "bytes", "counts", "metric_eligibility", "provenance_source" } ],
  "coverage": { "cohort_papers": { "O1": [...], "O2": [...] },
                "denominators": { "O1": { "truth_objects": n }, "O2": { "all_annotated_truth_objects": n } } } }
```

## Measurement

`graded-objects.py measure` builds `examples/object_metrics.rs`, requests predictions for every
cohort paper, and calls `object_metrics.evaluate_paper(paper, artifact, index)` per paper. O1 is
micro F1 over the summed counts; O2 is the median of every truth region's IoU, misses counting
0.0. It writes the observation with per-paper outcomes to `eval/inputs/evidence/graded-objects.json`
and publishes `eval/inputs/objects/figure-table.json` with `collector: "graded-objects-v1"`. If
the existing input has a different collector, its bytes are frozen under
`eval/evidence/object-metrics-history/` first, so the last LaTeX-replayed measurement stays on
record. `lysilogy eval objects --check` then reads the new input.

`graded-objects.py report` prints, per paper, the unmatched predictions and missed truth objects
with page and label, which is the list to work from when improving the detector.
