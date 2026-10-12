# Expanded K1 figure and table regions

Issue [#106](https://github.com/tjmisko/Lysilogy/issues/106) removes the duplicate Figure 4 caption and recovers four missing figure/table bodies in the expanded, independently labeled K1 cohort. Detection F1 rises from 0.978723 to 1.0; median region IoU rises from 0.757008 to 0.907601 across all 23 annotated objects. Both objectives remain above their unchanged targets. Three new masked figures remain unresolved and are tracked in [#111](https://github.com/tjmisko/Lysilogy/issues/111), under the E1 Paper objects epic (#12).

The baseline is `3b44b1e1ae7beb7408d9d169413fba57919c899b`. Final measurements use `7e74e59c2722791e9829cdc71abd060faf9a79ac`; final integrated checks use `fadcceaadae677774fb064cb863547164fba49e9`. That ordinary merge includes main `de0d543` and changes only progress documentation relative to the measured source. Rebase was not used because the standing approval restriction requires the previously agreed ordinary merge alternative.

## Behavior and evidence boundaries

The graphics parser now accepts a bounded subset of opaque, isolated, non-knockout Normal groups and explicitly rectangular clips. It intersects placed image bounds with those clips and checks scope balance, page dimensions, finite transforms, framing, and resource limits. The interpretation follows the installed MuPDF Device documentation and the actual retained trace syntax. Arbitrary masks, unknown drawing state, and unsupported clip shapes remain excluded. Graphics derivation version advances from 1 to 2 and detector version from 2 to 3; native reading indexes, their schema, source generations, and anchors are unchanged.

Table ownership now recognizes repeated numeric rows above a caption, including native header/method columns. It rejects a grid separated from its caption by multiline prose. Overlapping glyph bands keep a heading's superscript on the same printed row, so a heading alone cannot manufacture a two-row table. A narrowly evidenced unfinished prose reference prevents the duplicate caption; the general prose barrier retains its earlier constraints.

An image embedded in a larger native diagram no longer necessarily replaces the diagram's bounds. The native candidate must independently enclose the image, with at least three complete, small, nonnumeric labels and complete labels above and below it. Missing token geometry, Unicode numeric grids, one-sided labels, and neighboring captions cannot establish this cue. Other image-backed paths retain their previous image-boundary behavior.

Neither frozen truth version, its labels, the matching algorithm, nor the scoring denominator changed. The collector's changes enforce the current derivation versions. All native index, PDF, source, trace, and identity-registry inputs were hashed before and after measurement. No corpus PDF or source was copied into the repository or library.

## Same-cohort measurements

| Frozen cohort | Annotated objects | Before O1 | Final O1 | Before median O2 | Final median O2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| K1 limited v1, two papers | 15 | 1.0 | 1.0 | 0.9107793204 | 0.9107793204 |
| K1 limited v2, three papers | 23 | 0.9787234043 | 1.0 | 0.7570080448 | 0.9076006900 |

V2 moves from 23 true positives, one false positive and zero false negatives to **23/0/0**. Both complete older per-paper metric records, and every older region IoU, are exactly unchanged. Unknown truth regions remain zero; all four remaining zero-IoU objects are included in the 23-object median.

The newly included paper, [arXiv:2210.11141v1](https://arxiv.org/abs/2210.11141v1), has these final region results:

| Printed object | Before IoU | Final IoU | Result |
| --- | ---: | ---: | --- |
| Figure 1 | 0.757008 | 0.757008 | Native diagram extent preserved |
| Figure 2 | 0 | 0.883265 | Scoped raster region recovered |
| Figure 3 | 0 | 0.935046 | Scoped raster region recovered |
| Table 1 | 0 | 0.917662 | Grid above caption replaces unrelated heading below |
| Figure 4 | 0 | 0 | Masked region unresolved; duplicate prose caption removed |
| Figure 5 | 0 | 0 | Masked region unresolved |
| Figure 6 | 0 | 0 | Masked region unresolved |
| Table 2 | 0 | 0.945156 | Grid above caption recovered |

The first measured iteration (`7f1bffa`) already reached the final aggregate values, but regressed Figure 1 from 0.757008 to 0.202630 by choosing partial raster inserts over enclosing native labels. That full observation remains frozen. The second iteration's independently tested composition rule restores Figure 1 exactly; it does not choose between old and new predictions using truth IoU. The independent reviewer recomputed all 38 matches and IoUs across the final v1/v2 runs, including this restoration.

## Validation and retained artifacts

Twelve new offline Rust test functions cover scoped/clipped image handling, image-count limits, caption continuations, native grid ownership, superscript row boundaries, intervening prose, and raster/native composition controls. The unchanged original trace packet remains recorded alongside explicit version-2 clip expectations; a separate 19-case adversarial scope packet is included. Collector tests also reject stale detector/graphics versions. Independent compiled replay passed all 27 figure and 12 graphics tests from the exact Cargo-selected retained test executable.

Final integrated validation passed formatting, strict all-target/all-feature Clippy, all-target Rust tests, the provider-budget collector, `lysilogy eval scale --check`, `lysilogy eval objects --check`, and isolated `lysilogy eval tests --check`. G5 passed **362 Rust / 383 Python / 85 Node tests**, with network disabled and model CLIs absent. O30 remains zero budget violations over 10,000 synthetic references. No web source changed.

The initial G5 attempt failed because this new worktree lacked its cached TypeScript dependency link; the other checks and suites passed. Its failed receipt and complete logs remain retained. Linking the existing dependency cache with the identical package-lock hash required no installation or network call. G5 then passed in 7.610 seconds, with source bytes unchanged. The failure is separate from the successful retry in the evidence manifest.

The final measured build plus both actual collectors and objects check took **42.148 seconds**, with peak child RSS **1,011,680 KiB**. The v1 and v2 collector commands took 14.903 and 13.702 seconds respectively. Network calls, model calls, and external API cost were **0 / 0 / $0**. These timings include bounded local derivation and validation, not corpus indexing. Agent inference cost is not available.

[The machine evidence](../../eval/evidence/expanded-k1-regions.json) binds source commits, fixture inputs, every measurement generation, raw Cargo output, predictions, source inventories, independent reviews, and both gate attempts. Its 230 external artifact references total 185,126,901 bytes under `~/.cache/lysilogy/`; none requires the worktree to remain. Exact final measured CLI, object-metrics executable, and compiled regression executable are retained. Baseline and first-iteration executable hashes, source commits, Cargo records and outputs are retained; their executable bytes are not.

Key immutable receipt locations relative to `~/.cache/lysilogy/expanded-k1-regions/` are:

- `before-3b44b1e/receipt.json`: original cohort baselines.
- `after-7f1bffa-first-iteration/receipt.json`: first changed measurements, including the Figure 1 regression.
- `after-7e74e59-second-iteration/receipt.json`: final actual measurement and preservation checks.
- `final-gates-fadccea/receipt.json`: all initial integrated checks and the explicit G5 setup failure.
- `g5-retry-fadccea/receipt.json`: successful isolated G5 with matching cached dependencies.
- `release-fadccea/archive.json`: portable aliases for scorecard, baselines, provider simulation, and all evaluation results.

## Remaining scope

The four zero regions comprise three newly observed mask-clipped figures and one unchanged earlier figure. Follow-up #111 investigates bounded mask evidence; it must not treat an arbitrary mask frame as painted content. Broad K1 coverage remains open in #97. These two- and three-paper cohorts establish the stated measurements, not 500-paper coverage or general performance.

The current worktree scorecard has **1/5 hard gates** and **3/30 objectives** available at target (O1, O2, O30). Other stages remain unavailable here; the held bibliography work and its K2/O9 requirement are separate. Historical scale misses remain recorded in #21/#22 and are not re-attested by this change. No hard gate or objective target was lowered, and this report does not establish complete system acceptance.
