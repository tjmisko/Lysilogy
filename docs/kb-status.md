# Knowledge base branch: status and instructions

Last updated: 2026-09-16. Keep this to one page and update it in the same PR as any change it
describes. The [phase plan](knowledge-base-phases.md) holds the design order and file ownership;
its session log is an archive. The [design](knowledge-base-plan.md) defines targets.

## What this branch is

`integration/knowledge-base` carries every knowledge-base commit, from the plan (#66) through the
grading tool (#139) and home paging (#140). `main` is the pre-KB reader at `ade1486`. Work here,
target PRs here, and merge back to `main` only when asked.

## What works today

- **The reader** with 10,951 corpus papers, paged at 250 per page.
- **Figure and table detection** on every paper, measured at O1 0.99 and O2 0.89 on 50 graded regions.
- **Grading mode**: confirm, correct or add figure/table truth inside the reader; a stratified queue of the 1,000 eval-tier papers; a collector that rescores the detector from your grades.
- **Foundations** with no user-facing surface yet: paper identity by content hash, KB types, name and title normalizers, provider cache and budgets, evaluation harness, corpus tooling.

Nothing cross-paper exists yet: no Works, Persons, citations, acquisition, lists or graph.

## How to run it

Build once from a worktree of this branch:

```sh
cargo build && (cd web && npm install && npm run build)
python3 scripts/eval/graded-objects.py queue --selection ~/Corpora/arxiv/selection.json
./target/debug/lysilogy --library ~/Corpora/arxiv/pdf --data ~/.cache/lysilogy/arxiv-kb-data \
  --notes ~/.cache/lysilogy/kb-inspection-notes serve --bind 127.0.0.1:7321 --web web/dist
```

Open `http://127.0.0.1:7321`.

**Grade a paper.** Type `:` then `grade` to open the next queued paper, or press `G` in any
paper's PDF view. `j`/`k` move between detected objects, `y` accept, `n` reject, `e` drag a
corrected box, `a` add a missed object, `c` mark the paper complete, `N` save and open the next
paper, `?` show the keys, `Esc` leave. Grades autosave to `papers/<id>/objects-grades.json` in the
data root. Two minutes per paper is typical.

**Score the detector.** After grading a batch:

```sh
python3 scripts/eval/graded-objects.py export
python3 scripts/eval/graded-objects.py measure --build --executable target/debug/examples/object_metrics
python3 scripts/eval/graded-objects.py report
```

`report` lists every unmatched detection and missed object by page and label; that is the
worklist for improving the detector. `measure` republishes O1/O2; `./target/debug/lysilogy eval
objects --check` regenerates the scorecard. Commit `eval/truth/k1-graded/` and
`eval/inputs/objects/figure-table.json` as data.

## Numbers

| Measure | Value |
| --- | --- |
| Hard gates passing | 1 of 5 (G5) |
| Objectives at target | 3 of 30 (O1, O2, O30) |
| Graded truth papers | 9 (seeded from the V5 LaTeX release); grade more to grow it |
| Corpus | 10,951 PDFs indexed; 1,000 eval-tier papers with sources |

## What is next

| Milestone | Issues | Done when |
| --- | --- | --- |
| M1 Vertical slice | #33 → #25 → #39 → #38 (exact-key matcher only) → #40 → #42 → #44 | Open a paper, follow a reference to a Work page listing the local papers that cite it; `kb rebuild` reproduces the same IDs. |
| M2 Objects and scale | #26 #27 #28 #30 #31 #29 #32, #21 #22 #23 | Figures tab with equations and statements; 10k scan and search targets measured. |
| Later | Phase C and D issues in the phase plan | Acquisition, citations, lists, graph. |

Immediate steps: fix the 16 rusqlite integer-type compile errors on `feat/e2.1-kb-store` and add
an unavailable-KB startup state so a bad database never blocks the reader (#33); merge #25 with
#103 as the recorded miss; grade papers as time allows.

## Unfinished branches

| Branch | Issue | State |
| --- | --- | --- |
| `feat/e1.2-bibliography` | #25, draft #94 | Done on its cohort; title accuracy 30/35 tracked in #103. |
| `feat/e2.1-kb-store` | #33, no PR | Written, never compiled; 16 shallow errors. |
| `feat/e8.3-formal-tranche` | #132, no PR | V6 statement/proof truth; superseded in spirit by reader grading. |
| `feat/e8.4-reference-truth`, `feat/e8.5-person-labels` | #71 #72, drafts #93 #95 | Provider truth builders; build small when #46 or #62 needs them. |

## Rules in force

1. #97's 500-paper coverage is a reported number, not a gate.
2. A detector merges with a complete graded cohort of a handful of papers and every miss recorded with a follow-up.
3. One review per PR. Receipts and audits are for acceptance reports only. Truth additions are data PRs.
4. Provider truth is built small by the issue that consumes it.
5. Milestones replace the phase barrier.

Data contract for grading: [`eval/graded-objects-contract.md`](../eval/graded-objects-contract.md).
