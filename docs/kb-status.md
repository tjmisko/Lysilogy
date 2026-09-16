# Knowledge base: current status

One page, kept current. The [phase plan](knowledge-base-phases.md) holds the design order and
file ownership; its session log is an archive. Update this file in the same PR as any change it
describes.

Last updated: 2026-09-16.

## Branch layout (2026-09-16)

`main` is the pre-knowledge-base reader at `ade1486`, the merge of PR #10. Every KB commit from
the plan (#66) through the grading tool (#139) and home paging (#140) lives on
`integration/knowledge-base`. Create KB worktrees from that branch and target PRs at it. The
KB returns to `main` by an ordinary merge when the user asks.

## Where things stand

| Area | State |
| --- | --- |
| Merged (on integration) | Grading tool #137/#139, home paging and preview cache #140, plan decisions #138, A1 foundations (#19 #20 #24 #34 #63 #68 #69), name/title normalizers (#35 #36), figure/table detector fixes, LaTeX truth releases V1–V5 (11 papers), evaluation harness, 10,951-PDF corpus with native indexes. |
| Scorecard | 1/5 gates (G5), 3/30 objectives (O1 0.99, O2 0.89 on 50 regions; O30). Everything else unavailable. |
| Branches | #25 bibliography (draft #94, done on its cohort, title accuracy 30/35 tracked in #103); #33 SQLite store (no PR, 16 shallow compile errors, never compiled); #132 formal truth V6 (no PR); #71/#72 provider truth builders (drafts #93/#95). |
| Environment | crates.io, Crossref and OpenAlex reachable; 111 GiB free. The handoff's network/disk blockers were Codex-sandbox artifacts. |

## Milestones

| Milestone | Issues | Done when |
| --- | --- | --- |
| M0 Grading tool | #137 | A paper can be graded in the reader and `graded-objects.py measure` republishes O1/O2. |
| M1 Vertical slice | #33 → #25 → #39 → #38 (exact-key matcher only) → #40 → #42 → #44 | Open a paper, follow a reference to a Work page listing the local papers that cite it; `kb rebuild` reproduces the same IDs (G4). O8–O10 and O15 measured. |
| M2 Objects and scale | #26 #27 #28 #30 #31 #29 #32, #21 #22 #23 | Figures tab with equations and statements; 10k scan/home/search targets measured. |
| Later | Phases C and D issues in the phase plan's dependency order | Unchanged. |

## Rules in force (2026-09-15)

1. #97's 500-paper coverage is a reported number, not a gate. Nothing waits on it.
2. A detector merges with a complete reviewed cohort of a handful of papers and every metric
   miss recorded with a follow-up. Graded truth from the reader is the active K1 visual source.
3. One review per PR. Receipts, hash tables and independent audits are for phase and system
   acceptance reports only. Truth additions are data PRs.
4. Provider truth (K2, K4, K5, K7) is built small by the issue that consumes it.
5. Milestones replace the phase barrier. Phase A is not "complete" and does not need to be.

## Next actions

1. Grade papers with the reader (M0 is merged on the integration branch); run `export`, `measure`, `report`.
2. Fix the 16 compile errors on `feat/e2.1-kb-store`, add the unavailable-KB startup state, run
   its tests, open the PR.
3. Merge #25 with #103 as the recorded miss.
4. Commit or branch the PDF-preview changes sitting dirty in the main checkout before M2's UI work.
