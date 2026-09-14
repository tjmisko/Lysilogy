# Native figure regions below tables

Issue #121 prevents a preceding table's cells from expanding a later figure's
native-text region. Detector version4 uses the already observed table grid as a
boundary for native labels. Image placements retain their existing ownership and
filtering. Native indexes, graphics parsing, frozen truth and scoring are unchanged.

The independently reviewed measurement at `734372ab96f1d9ac9da765fad2390e8431888975`
improves two regions without any per-object regression:

| Object | Previous IoU | Current IoU |
|---|---:|---:|
| Historical `object:shapPlot` | 0.5255890483769016 | 0.8820197414368678 |
| 2409.03655v1 Figure3, privacy/emotion trade-off | 0.16254643935124297 | 0.8793852892966821 |
| 2409.03655v1 Figure4, t-SNE | 0.18438363161698818 | 0.18438363161698818 |

All15/23/29 outcomes in frozen v1/v2/v3 were measured. O1 remains1 in every cohort;
O2 medians remain0.9107793204006069 /0.9169720168893188 /0.9169720168893188.
Every cohort retains the historical `object:timeToEvent` zero. No unknown truth
regions, changed captions, favorable subset or target adjustment is hidden by the
aggregate. The three cohorts overlap: these are29 unique objects, not67 independent
cases. [Complete comparisons and receipt bindings](../../eval/evidence/visual-regions-121.json)
retain every old/new outcome and zero.

## Diagnosis and scope

Original2409.03655 pages4/5 contain vector paths and no image placements. Their
nonisolated groups cause the existing graphics collector to report unsupported
trace state. Permitting those groups alone would still supply no vector geometry;
a group bbox is not painted support. Page5 also contains a knockout legend group.
The current t-SNE prediction follows native legend text and misses the scatter plot.
[Follow-up #123](https://github.com/tjmisko/Lysilogy/issues/123) tracks bounded vector
support, including clipping/transparency/resource controls. Overall O2 already
meets its target; the remaining per-region limitation is explicit.

The Figure3 failure has a separate actionable cause: its native seed included
short numeric paragraphs from Table1 above it. Reusing that table's observed grid
removes those cells. No enclosing frame, margin expansion or synthetic plot body
is used. The graphics collector version remains3; only the derived figure detector
and its collector version check advance to4.

## Validation and review

Five new offline Rust controls cover an earlier table grid, a neighboring column,
a caption without a grid, unavailable figure geometry, and an owned image beginning
inside the table's padding margin. The last control addresses independent review's
first finding: the initial boundary incorrectly affected image filtering. It now
applies only to native labels and native padding. Collector tests also reject the
historical detector3 generation.

Formatting, strict Clippy, all386Rust tests,41focused collector tests and isolated
G5 pass. G5 executes386Rust/537Python/85Node tests with external networking isolated
and model CLIs absent. Own-worktree CLI objects checks pass after measurement.
The offline provider-budget fixture was generated in this new worktree before its
scale check; O30 remains0/10,000. Current scorecard remains1/5 gates and3/30 objectives.
Missing suites remain unavailable, not passing system acceptance.

The before CLI check explicitly reused a byte-identical archived binary whose
Cargo inputs and Rust sources match the pre-change checkout. Its archive mode0644
first prevented execution; that failed attempt remains recorded. An owned755 copy
then passed. Current gates and the after check use a freshly built own-worktree CLI.
Before cohort observations reuse the exact retained #115 measurement at70eebb5;
they are not relabeled as new runs. The historical files and all native/graphics
bindings were rehashed and preserved.

Actual measurement took85.867978seconds with275744KiB peak child RSS, zero external
network/model calls and zero external service cost. Agent reasoning cost is unknown.
The wall time includes Cargo checks and truth replay; it is not a controlled detector
performance comparison or a10k performance claim. Repeated table scans remain a
resource consideration beyond this four-paper cohort.

Independent source, runner and actual reviews are bound in the machine evidence.
The runner's first review found a mutable-baseline reread; it now compares verified
frozen copies and checks all originals again on completion. The actual reviewer
independently recomputed every old/new per-paper record from complete predictions.
The source is unchanged after that measurement. Ordinary main integration is used
under the standing no-rebase-without-approval rule; final gates and PR review precede
the merge commit. No PDFs or LaTeX sources are committed.
