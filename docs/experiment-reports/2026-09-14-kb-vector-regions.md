# Bounded vector support for figure regions

Issue #123 recovers the painted body of the t-SNE figure in 2409.03655v1. Its region
IoU improves from 0.18438363161698818 to 0.9301568442382464. The previous region covered
native legend text; the new optional supplement joins qualifying painted components
to that native body. Original native anchors, caption identity, image/table behavior,
truth labels, matching and objective targets stay unchanged.

The measured implementation is `d033b515628fcdc79ad0de0c97f46da28286b183`, detector 6
and graphics 5. The four frozen cohorts contain 117 overlapping outcomes, representing
50 unique objects across nine papers. Every paper is retained, including negative-only
papers; no favorable subset is selected. The sole changed unique outcome is
`53ac4c3af9951930 / object:fig:tsne`, shared by V3 and V4. No outcome regresses.

| Frozen cohort | Objects | O1 before → after | Median IoU before → after |
|---|---:|---|---|
| V1 | 15 | 1 → 1 | 0.9107793204006069 → unchanged |
| V2 | 23 | 1 → 1 | 0.9169720168893188 → unchanged |
| V3 | 29 | 1 → 1 | 0.9169720168893188 → 0.9176618036504631 |
| V4 | 50 | 0.98989898989899 → unchanged | 0.8863525877384802 → 0.8910021250829322 |

V4 remains 49 TP / 0 FP / 1 FN with 0 unknown truth regions. The two prior zero regions
remain: `object:timeToEvent` and 2207.03024v1 Figure 3 (`object:fig:spherical_interp`).
No caption miss is repaired by this work. The earlier split-caption implementation
from #125 is integrated ordinarily and remains the measured detector 5 baseline.
The owning-CLI before-run at 7a224ab is a separate detector 4 baseline; it is not
misrepresented as a fresh detector 5 execution. The approximately 500-paper coverage
requirement in #97 remains open.

## Painted support and ownership

The initial generated experiment compared trace bounds with actual compositing.
A group, path, frame, clip or white-background bound is not painted support. The
implementation uses final full-page renderer pixels for a conservatively supported
trace subset. A separate renderer mode permits finite Normal nonisolated/knockout
groups and proved closed rectangular clips. Images, masks, unknown operations and
unsupported state retain explicit unavailable evidence. The existing image path
finishes before optional vector work receives the remaining graphics budget.

The candidate policy was fixed from synthetic fixtures before corpus measurement:
144 dpi, a five-level RGB contrast threshold, full-page eight-connected components,
compact components at most one native font height, a 1.5-font-height connection gap,
at least six components per cluster, and exactly one complete caption window.
Components are formed before any native-glyph or ownership exclusion. Whole native
glyphs, prose, captions and observed table grids are barriers; component groups,
the combined group envelope and the final native-body union may not bridge them.
Complete joined captions use the ownership introduced in #125.

Known native gaps, missing/invalid token rectangles and ambiguous owners withhold
the supplement. The page cap is 4,194,304 pixels / 4,096 complete components. At most
16,777,216 ownership checks are allowed across all page captions, including the
component graph. Rendering uses the existing bounded command/cancellation path and
hash-bound resource wrapper: 768 MiB address space, 5 CPU seconds, at most 5 wall
seconds per raster and the remaining 30-second/64-MiB paper budget. Raw rasters,
component records, statuses, PDF/tool/native/source bindings and exact Cargo-selected
executables are retained. This is a candidate heuristic; raster ink does not prove
semantic ownership when text is silently absent from the native inventory.

Six unique pages have complete raster evidence, with 13,077 independently reviewable
components in total. The motivating paper's pages 4/5 have 2,201 / 2,860 components;
four additional complete pages do not change any measured outcome. All 17 other
retained trace pages remain unsupported: 16 contain images and one has no curved
paint. One existing page has a prior tool failure and no trace. No page gap, image
placement, native geometry or unsupported state is invented or removed.

## Failed generation and review corrections

The first full graphics 4 run at 0978f86 produced no changed outcomes or rasters.
It rejected every attempted page at an identity `set_default_colorspaces` preamble.
The same exact preamble existed in all 17 already frozen generated traces, while the
initial compact Rust trace fixtures omitted it. The entire first run, normal CLI
check and review remain preserved as a no-benefit integration failure.

Graphics 5 accepts only one self-closing declaration as the first command directly
under the page, with exactly DeviceGray/DeviceRGB/DeviceCMYK and output intent None.
Substitutions, missing/extra/duplicate attributes, nonempty forms and repeated,
late or nested declarations reject. The version change prevents reuse of graphics 4
unsupported cache entries. Candidate parameters and the strict image parser remain
unchanged; the correction adds no general color-profile or output-intent support.

Independent review also corrected an empty unproved clip, a known native token
without rectangles, and a per-caption work calculation that exceeded the stated
page budget. A measurement-runner correction now requires complete ordered paper
identities and strict pairings, including a trailing negative-only paper. Cohort
artifacts are registered before comparison assertions so a failure retains them.
All corresponding synthetic controls, prior Clippy/format failures and corrected
receipts are preserved.

The 14-PDF/42-command compositing probe and 17-PDF/51-command raster probe remain
separate generated evidence. They retain very thin/near-white misses and an outlined
prose false candidate when native text silently omits that prose. No policy parameter
was retuned to this paper or to measured IoU.

## Validation and cost

Current source passes formatting, strict all-target Clippy, full Rust and own CLI
compilation. Provider-budget synthetic checks, bibliography/scale checks and isolated
G5 pass with 427 Rust / 584 Python / 85 Node tests. The namespace contains only loopback
and no model CLI. The existing TypeScript 5.9.3 installation supplies 132 byte-verified
local files, with no download. Full logs, source copies, generated provider observations
and inputs, Python bytecode and portable CLI/helper executables are retained before
cleanup. Fixed V1/V2/V3/V4 releases replay through the actual collectors unchanged.

The owning aftercheck creates a fresh available 50-case result with the complete
current values. Only O2 improves through the normal baseline ratchet; previous
adjustment history, other metric baselines and targets remain unchanged.

The first full run took 155.367839295 seconds / 279,008 KiB peak child RSS; the corrected
run took 162.145825350 seconds / 278,592 KiB. The current full Rust/CLI portion took
14.975522744 seconds, remaining gates 21.710750361 seconds, and aftercheck 0.360617181
seconds. These include build/replay work as applicable and are not a controlled
incremental-cost or production 10k benchmark. External provider/model-service calls
and billed cost are zero; agent reasoning cost is unknown. No native-index rebuild,
truth publication, target change, corpus download or deposited-code execution occurs.
The independent actual audit rehashed 1,075 bindings and reconstructed all 13,077
components across 11,766,552 pixels, with exact component order, bounds and counts.
The independent owning-CLI audit verified 539 bindings and the complete baseline
history. [Machine evidence](../../eval/evidence/vector-regions-123.json) binds these
reviews and every outcome. Final PR review remains a separate step. Phase A/Wave A2
and overall application acceptance remain open.
