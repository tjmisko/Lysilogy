# Masked figure regions from decoded opacity

Issue [#111](https://github.com/tjmisko/Lysilogy/issues/111) recovers the three masked figure bodies left by #106. Figures 4, 5 and 6 of [arXiv:2210.11141v1](https://arxiv.org/abs/2210.11141v1) move from zero overlap to IoUs **0.983288, 0.977261 and 0.989286**. Expanded-cohort median region IoU rises from **0.907601 to 0.916972**, retaining all 23 annotated objects. Detection remains 23 true positives, zero false positives and zero false negatives.

The clean baseline is `fa841d403a0836fa14e5927e0829e97436a01cde`. Corrected measurements use `d596880d8fc211145143fcf4e467c1d686d402b9`; final integration and all gates use `431e4f1a9914a8366b4d756a3fbcb71a97f4579b`, including main `7d0bffa` and #107. Every measured implementation file remains byte-identical after that integration, and both refreshed complete per-paper metric records match the corrected measurement. Only generated current-metric JSON conflicted; the measured generation was preserved and then refreshed. The standing rebase restriction is handled by an ordinary merge.

## Evidence and supported behavior

The existing strict XML trace parser continues to own page framing, drawing state, transforms and clip scope. An additional trusted MuPDF Device adapter records complete decoded mask opacity, its attached image mask and independently verified base-image opacity. Only one active mask, matching image dimensions and exact transforms, and an opaque base image can establish support. Extra clips, unknown transparency, interpolation, color-key/decode effects and unproved states remain unsupported.

Every nonzero opacity value, including partial alpha, contributes its full pixel cell. The candidate is the tight outer rectangle of those cells, transformed into native page coordinates. This does not claim that holes or every interior pixel are painted. An empty mask yields no image. No mask frame alone is treated as support, and neither figure composition nor caption matching selects a result using truth IoU.

The fixed embedded script runs through a seekable, private unnamed regular file under `~/.cache/lysilogy`. Descriptor-based ancestor traversal rejects symlinks; `O_TMPFILE` creates no replaceable filename. The owning handle remains open through the child lifetime, with exact script bytes checked before and after. The existing `prlimit` utility enforces a 768 MiB address-space and eight-second CPU limit; the mask command also has a ten-second wall deadline within the existing thirty-second paper budget. Output is bounded to 16 MiB per page and 64 MiB per paper, with explicit pixel, axis and event limits. Missing platform/tool support retains fallback behavior. No new dependency is introduced.

Graphics derivation advances from version 2 to 3. Runtime/script hashes, raw receipt hashes, per-page status and admission counts enter source-backed provenance. Detector version remains 3; native reading indexes, their schema, source generations, anchors and the identity registry are unchanged.

## Measurements and preserved failures

| Frozen cohort | Objects | Before O1 | Final O1 | Before median O2 | Final median O2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| K1 limited v1, two papers | 15 | 1.0 | 1.0 | 0.9107793204 | 0.9107793204 |
| K1 limited v2, three papers | 23 | 1.0 | 1.0 | 0.9076006900 | 0.9169720169 |

| Printed object | Before IoU | First iteration | Corrected IoU |
| --- | ---: | ---: | ---: |
| Figure 4 | 0 | 0 | 0.9832876694 |
| Figure 5 | 0 | 0 | 0.9772608530 |
| Figure 6 | 0 | 0.9892855432 | 0.9892855432 |

Every other region value and both complete older paper records remain exact. Unknown truth regions remain zero. Zero-IoU regions decrease from four to one; that unchanged earlier figure remains in the full denominator. The frozen truth versions, labels, matching and scoring are unchanged.

The first measured iteration (`eb551e7`) recovered only Figure 6. A retained, Cargo-bound reproduction showed that the default `serde_json` binary64 parser shifted three valid page-3 transform decimals by one ULP. Enabling its existing `float_roundtrip` feature preserves exact values; a regression also rejects a neighboring unequal value even when it would narrow to the same native f32. No tolerance or coordinate narrowing was added. This feature changes JSON decoding globally, so all-target tests, native commitments and integrated gates cover that scope. Stored baseline fields remain unchanged except the measured O2 improvement.

The initial pipe-stdin compatibility probe also remains retained: MuPDF requires a seekable script. The corrected unnamed-file probe passes with an authored synthetic PDF. Two earlier strict-Clippy cleanup failures, their passing focused tests and all logs remain separate from the successful final checks. Original observations and all 24 first-generation files remain immutable.

## Validation and portability

Six new Rust tests cover the independently authored 43-case opacity packet, receipt/event integrity, resource limits, private seekable files, symlinked ancestors and exact decimal boundaries. The packet includes empty, partial, irregular and disconnected support, signed/quarter-turn transforms, crop traps, mismatched attachments, malformed arrays and neighboring figure/table events. Its expectations are preserved unchanged. A Node harness executes the exact embedded script against those synthetic pixels, and Python collector regressions bind script/runtime/receipt provenance.

Independent review replayed the exact Cargo-selected tests and recomputed all 76 matches and IoUs across both measured generations, all three native commitments and **2,062,628 opacity cells** for the three recovered masks, including XML, attachment and base-opacity bindings.

Final integrated formatting, strict all-target/all-feature Clippy, all-target Rust tests, both current-source K1 collectors, `lysilogy eval objects --check`, the provider collector, `eval scale --check` and isolated `eval tests --check` pass. G5 records **368 Rust / 424 Python / 85 Node tests**, with network disabled and model CLIs absent. O30 stays zero violations over 10,000 synthetic references. No web source changed.

The corrected measured build, both collectors and objects check took **41.373 seconds**, with peak child RSS **279,472 KiB**. Final integrated gates plus current-source measurement refresh took **51.288 seconds**. Network calls, model calls and external API cost were **0 / 0 / $0**; agent inference cost is not available. These are local derivation/verification timings, not corpus indexing or enrichment metrics.

[Machine evidence](../../eval/evidence/mask-regions.json) binds source generations, fixture provenance, every measurement, failures, exact executables, raw logs, masks, native commitments, reviews and portable aliases. Its **316 external artifacts total 432,706,017 bytes**, all under `~/.cache/lysilogy`; none requires retaining this worktree. Final measured/integrated CLI and object-metrics binaries, regression binaries and decimal probes are retained, with probe-library aliases and raw Cargo records.

Key receipts under `~/.cache/lysilogy/mask-regions/` are `before-fa841d4/receipt.json`, `after-eb551e7-first-iteration/receipt.json`, `after-d596880-second-iteration/receipt.json`, `final-gates-431e4f1/receipt.json` and `release-431e4f1/archive.json`.

## Remaining scope

The earlier zero-IoU figure and unsupported clip/transparency/resource cases remain explicit. The measured cohorts contain two and three papers; broader K1 coverage remains open in #97. The local scorecard has **1/5 hard gates and 3/30 objectives at target** (O1, O2, O30). Held bibliography/K2 metrics and historical scale misses #21/#22 remain separate; this change does not re-attest the 10k timing run. No hard gate or objective target was lowered, and this report does not establish complete system acceptance.
