# Lysilogy knowledge-base guide and status report

**The KB system is incomplete. Phase A / Wave A2 is still in progress.** The foundations, corpus tooling, evaluation harness and several extraction improvements have merged. The cross-paper database, resolution pipeline, acquisition workflow, reading lists and graph experience have not been delivered as an integrated system.

This handoff covers the work since the KB plan merged in PR #66, through implementation PR #133 and the retained unfinished branches. It also inventories older reader branches without attributing them to the KB buildout. Repository/GitHub inventory was captured at **2026-09-16 04:34:30 UTC**—September 15 locally—against main **`4972d63f10efdc2a89d01901b1adc0624408e868`**. Documentation commits after this snapshot do not change its implementation baseline.

## Read the report

| File | What it answers |
| --- | --- |
| [Implementation status](implementation-status.md) | What works on main, what exists only on branches, what is missing, and the architectural problems found |
| [Evaluation and corpus](evaluation-and-corpus.md) | Corpus contents, truth coverage, metrics, test evidence, failed experiments and limits on each result |
| [Worktrees and branches](worktrees-and-branches.md) | Every registered worktree, every local/cached remote branch, exact heads, dirty files and open PRs |
| [Issue and PR register](issue-register.md) | Every issue and PR in the repository snapshot, phase/wave, blockers, merge commits and open follow-ups |
| [Operations and resume guide](operations-and-resume.md) | Where data lives, safe inspection commands, validation workflow, blockers and precise continuation order |

## Current position

| Area | Verified status and practical limit |
| --- | --- |
| Merged KB work | 32 implementation/follow-up issues closed; all seven original A1 foundations merged. Last implementation: [#131](https://github.com/tjmisko/Lysilogy/issues/131) via [PR #133](https://github.com/tjmisko/Lysilogy/pull/133), merge `6059dcfc0ec7d7c13df3a535c4d96f1f63028da7`. Closure counts include fixes and experiments, not 32 complete product features. |
| Work in progress | Five unfinished KB worktrees; drafts [#93](https://github.com/tjmisko/Lysilogy/pull/93), [#94](https://github.com/tjmisko/Lysilogy/pull/94), [#95](https://github.com/tjmisko/Lysilogy/pull/95). SQLite #33 and formal truth #132 have no PR. |
| Committed scorecard | **1/5 hard gates pass; 3/30 objectives at target:** G5 and O1/O2/O30. Other gates/objectives are unavailable, not passed. O1/O2 are measured on a small selected cohort; O30 uses recorded fixtures. |
| Corpus preparation | 10,951 unique PDFs plus 1,000 version-matched source artifacts downloaded; all native indexes audited. The production 10k application scenario remains unverified. |
| Truth generation | 724/1,000 frozen papers parsed; **zero automatically admitted complete metric cohorts**. Main contains a limited 11-paper V5 release; the 12-paper V6 release exists only on #132. Approximately 500-paper stratified coverage remains open in #97. |
| Closest unfinished change | #132's corrected collector completed successfully with all 50 visual outcomes unchanged. Its separate final collector audit, CLI after-check and final integration/review remain. |
| Storage | **11.99 GiB free** at capture, below the **20 GiB floor**. Further substantial builds/downloads require space recovery; none was started for this report. |
| Provider/registry access | Four hosts were approved, but the last effective check on September 14 still denied them. Terminal-side application and a fresh effective check remain unverified. This handoff did not re-probe those hosts. |
| Implementation activity | Paused for the user's strategy/architecture discussion. This handoff does not adopt the proposed phase-order change or authorize three additional hosts. |

## How to interpret “working”

- **Merged and measured:** code is on main, and the linked report names its actual test or measurement. Coverage is limited to that report's input set and source revision.
- **Implemented on a branch:** code exists but is not delivered on main. It may still lack dependencies, real truth data, independent review or final integration checks.
- **Planned:** an issue/design exists. Types or offline fixtures do not establish the finished behavior.
- **Unavailable:** the required measurement has not been established. This is neither a passing result nor a measured failure.

Fresh checks for this handoff cover repository/GitHub inventory, filesystem paths, report consistency, links and preservation of the ten protected PDF-preview files. Rust/frontend/model suites and the live application scenario were **not rerun** for a documentation change. Their retained results are dated and attributed in the evidence report.

## What is not done

Phase A exit checks have not passed. No phase-completion report or final KB system acceptance report has been written. There is no verified end-to-end path from a real paper through object enrichment, reference acquisition, stable Work/Person resolution, an AI reading list, graph navigation and deterministic rebuild. No production 10k scenario has passed.

Final acceptance requires all five hard gates, at least **24/30 objectives**, measured follow-ups for misses, the running-app Playwright scenario, the scale scenario and `docs/experiment-reports/<date>-kb-system.md`. Use `lysilogy eval all --check --require-complete` for completeness; ordinary `--check` can succeed while measurements remain unavailable.

## Canonical planning and previous artifacts

The [phase plan and session log](../../knowledge-base-phases.md) remain the progress authority. The [design plan](../../knowledge-base-plan.md), [architecture](../../architecture.md), [scorecard](../../kb-scorecard.md) and [GitHub project](https://github.com/users/tjmisko/projects/12) provide the underlying specifications. This dated report consolidates them; it does not silently revise scope or acceptance targets.

The requested dark-mode technical explanation is retained locally at [kb-technical-explanation.html](/home/tjmisko/.cache/lysilogy/artifacts/kb-technical-explanation.html). It explains the seven strategy/architecture findings with code examples. Static checks passed; a browser preview could not run because the environment rejected a local socket. The earlier strategy review is [2026-09-14-kb-strategy-review.md](/home/tjmisko/.cache/lysilogy/2026-09-14-kb-strategy-review.md). These cache links work on this machine; the Markdown handoff and linked repository reports are versioned.
