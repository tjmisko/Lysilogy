# Reviewing reader output

This folder holds what you need to grade Lysilogy's output by hand on a small
set of papers and to use those grades to tune prompts:

- `rubric.json`: what to grade and how (the review panel and the summary both
  read it; it is compiled into the server).
- `papers.json`: the evaluation set: four AI papers and five classic economics
  papers, each with three to five neighbours (predecessors, successors,
  critiques, replies) so you can judge the "why it matters" text against the
  papers around it.

Every identifier and URL in `papers.json` was written without network access
and is marked `"verified": false`. Check an entry before relying on it, and
flip the flag when you have.

## The loop

1. **Fetch the papers.**

   ```sh
   python3 scripts/review/fetch_papers.py --target ~/Papers/review-set --neighbours
   ```

   The script tries each open-access URL, keeps a file only if it is a PDF, and
   ends with a table of `fetched`, `present`, `failed` and `needs-manual`
   rows. Paywalled papers (Akerlof 1970, several neighbours) are
   `needs-manual`: download them through a library and save them under the
   file name the table prints. Papers go to `<target>/<filename>`, neighbours
   to `<target>/neighbours/<paper id>/<neighbour id>.pdf`. `--dry-run` shows
   the plan; `--only card1990` limits it to one paper.

2. **Add the folder to the library** (the home page's folder list) and analyse
   each paper as usual.

3. **Grade.** Open a paper and press `F2` (or `:review`). The panel shows each
   component's output next to its rubric dimensions. Keys:

   | key | does |
   | --- | --- |
   | `j` / `k` | next / previous dimension |
   | `[` / `]` | previous / next component |
   | `0`–`4` | score |
   | `y` / `n`, space | pass / fail, toggle |
   | `1`–`9` | pick a choice |
   | digits, `+` / `-` | set or adjust a count |
   | Backspace | clear the grade |
   | `x` | mark the component not produced |
   | `r` | the component's hard reject (the context's false claim) |
   | `e` | jump to the component's notes |
   | `i` | flag an issue; text selected in the output becomes its quote |
   | `Esc` / `q` | close the issue form, leave a text field, then the panel |

   Grades save as you go. The strip at the top shows which evaluation-set
   papers are in the library, analysed and reviewed; click one to open it.
   Papers outside the set can be reviewed too; they just do not appear in the
   strip.

4. **Compare.** `lysilogy review` prints the summary: per pipeline version, the
   mean of every score dimension, pass rates, choice counts, count totals,
   not-produced and hard-reject counts, and issues by component and
   severity. `lysilogy review --json` prints the same as JSON, and
   `lysilogy review --export reviews.json` writes every review, the summary
   and every flagged issue (with the output it refers to) to one file. The
   same data is at `GET /api/reviews` and `GET /api/reviews/summary`.

## What the rubric measures, and why

Ten minutes a paper is the budget, so the rubric grades only what the first
two views show and what you said matters, in this order:

- **Abstract** is pass/fail, because "almost the abstract" is a bug, not a
  grade. When it fails you say how (truncated, extra text, wrong block,
  garbled, missing), which points at the fix. The panel names the oracle
  abstract under `eval/spike/ground-truth/oracles` when one exists
  (`amodei2016`, `kaplan2020`, `autor2013`).
- **Thesis** gets one quick score; it is the line most readers see first.
- **Why it matters** is the highest-value component and gets the most
  dimensions: accuracy, specificity ("could this be pasted onto another
  paper?"), orientation value, the read-alongside papers, provenance and
  economy. A false claim about the field is a hard reject, recorded
  separately from the scores so it cannot be averaged away. The panel shows
  the evaluation set's neighbours under the output for comparison.
- **Outline** counts errors against the printed headings (missing, spurious,
  wrong level or page) and adds one overall score. Counts are cheaper to give
  and easier to compare than a feeling about structure.
- **Reading units** (the logical segmentation) grades whether each unit
  answers one question, whether boundaries fall where the argument turns,
  granularity (too many / about right / too few), whether titles carry a
  claim rather than a genre label, and whether summaries are faithful and
  specific. This is the part models have found hardest, so it is split
  finely enough to show which of those fails.
- **Overall** and free notes, then **issue flags**: component, severity, an
  optional tag, a note and the quoted text. Flags are the raw material for
  regression cases: the export puts each one next to the exact output it was
  about.

"Not produced" is gradable on purpose. If the reader shows no read-alongside
list, mark the component and score what the absence costs (usually a 0 on
that dimension). Not-produced rates are reported separately in the summary.

## How reviews are stored

Each review is `papers/<paper id>/reviews/<snapshot id>.json` in the data
folder, next to the paper's `analysis.json`. It contains a copy of the fields
it graded (every top-level analysis key starting with `thesis`,
`author_abstract`, `abstract`, `outsider_brief`, `context`, `read_alongside`,
`neighbo`, `surrounding`, `outline`, `logical`, `section` or `reading_unit`),
the analysis's `generated_at` and `pipeline_version` (or `pipeline.version`)
when present, and a snapshot id hashed from those. Analysing the paper again
produces a new snapshot and a new, empty review; the old review keeps the text
it graded and shows under "Earlier reviews" in the panel. A save sent against
output that has since been replaced is refused (HTTP 409) rather than attached
to the wrong text. Summaries group reviews by `pipeline_version`; analyses
that record none are grouped as `unversioned`.

## What the panel reads from an analysis

The snapshot copies these top-level fields of `analysis.json` verbatim
(`REVIEWED_PREFIXES` in `src/review/snapshot.rs`), and the panel reads them with
the reader model's types (`ReviewedFields` in `web/src/lib/review.ts`):

- abstract: `author_abstract`, `abstract_extraction`, `abstract_agreement`
  (token F1 against the deterministic extraction); thesis: `thesis`.
- context: the context node's `context` (lead, before, contribution, after,
  each claim with its `basis` of `paper`, `dossier` or `background`, plus
  `read_alongside`, `reception_note` and `counts`). Analyses made before the
  node show `outsider_brief`, the older `context_notes` and `context_sources`.
- outline: `outline`, the printed headings with number, level, page and role.
- logical units: `sections`; each unit's `outline_entries` index into
  `outline`, and the panel shows the headings it groups. Units made without a
  usable outline have none and are labelled page-span units.
- `quality_warnings`: shown above the component whose field they concern.

A missing field renders as "not produced". The shared fixture
`tests/fixtures/reader/analysis-current.json` exercises every field; tests on
both sides load it.

## Files

- `eval/review/rubric.json`, `eval/review/papers.json`: this folder.
- `scripts/review/fetch_papers.py` and its tests
  (`python3 -B -m unittest scripts/review/test_fetch_papers.py`).
- `src/review.rs`, `src/review/`: rubric validation, snapshots, storage,
  progress, summary and export. `src/api/review.rs`: the routes.
- `web/src/components/ReviewPanel.tsx`, `web/src/lib/review.ts` and
  `web/scripts/review.test.mjs` (`npm run test:review`).
