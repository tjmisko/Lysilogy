# Reading-index page isolation (#98)

The reading index now retains usable pages when Poppler returns invalid native word coordinates on another page. The actual failed eval paper, `pdf/2308.05883v2.pdf`, now has 47 indexed pages: **45 native and two explicitly OCR pages**, with both original native failure gaps retained. All 1,749 previously persisted indexes and all 10,928 prior canonical identity records remain unchanged.

## Behavior and compatibility

Three combining circumflex glyphs on pages 30 and 34 previously caused the strict native parser to reject the entire 47-page paper. The repair validates all document/page boundaries and finite positive page dimensions before admitting page bodies. Invalid framing still rejects the whole document. A malformed word or line with reliable page framing withholds that entire native page, including otherwise valid words on it. Coordinates are never swapped, synthesized or silently dropped.

The isolated page retains its number, dimensions and original error. Existing bounded local OCR may replace it with OCR-labeled text; failure, empty output or an exhausted budget leaves it unavailable. Native failure gaps survive successful OCR. The existing limits remain 12 OCR pages / 100 seconds, 35 seconds per external command, and 180 seconds per index build.

Word validation accepts only Poppler's four exact numeric coordinate attributes. This prevents an unrelated attribute from shadowing a coordinate or becoming extracted word text through the older decoder. Unknown word attributes conservatively withhold the page. The reader permits Poppler's existing raw control glyphs and does not execute document declarations or fetch external resources. Existing unused line-box coordinates retain their prior policy.

Saved citation-anchor parsing remains strict and retains its token numbering. Page construction was factored without changing that parser's behavior. A native-failure page list prevents joining prose across a newly unavailable page, while preserving the established output of previously successful sparse-page indexes. Reading-index schema 6 and existing cache generations remain valid; no blanket invalidation or canonical-registry rebuild was performed.

## Tests and scorecard

Twelve new offline tests cover coordinate reversal, nonfinite/missing/invalid/duplicate values, unsafe framing and dimensions, malformed local nesting, exact valid-fixture preservation, Unicode/control glyphs, attribute shadowing, page/block mapping, OCR success/failure/empty output, and interrupted prose. Three existing real native fixtures compare exactly with the strict verbatim parser.

Before-change main `941c4f3` was clean. Its offline CLI build took 95.8173 seconds; `eval objects`, `bibliography` and `scale --check` passed. The final source was normally merged with main through `a322185` (without rebase), producing validation head `e5ac219`.

Final formatting, strict all-target/all-feature Clippy, all-target Rust tests, and the same affected eval suites pass. G5 independently runs with networking disabled and model CLIs absent: **317 Rust, 254 Python and 85 Node tests pass**. Web source was unchanged.

| Measurement | Before | After | Meaning |
| --- | ---: | ---: | --- |
| G5 | not rerun in this issue's before check | pass | Full final isolated suite |
| O30 | 0 / 10,000 | 0 / 10,000 | Production provider-budget simulation |
| Owning object/bibliography measurements | unavailable | unavailable | This branch has no owning collector; unavailability is not a pass |
| Actual failed paper | no persisted index | 47 pages | 45 native, 2 OCR, 2 original native gaps |

The current local scorecard has G5 passing and O30 at target (1/5 gates and 1/30 objectives available at target). The limited K1 truth release now exists, while detector/bibliography measurements are handled by their owning implementations. This repair makes no detector-accuracy or provider-identity claim. Historical O25/O26 misses remain 15.503406 seconds, 1757.1 ms render, and 163.4 ms search p95, tracked by #21/#22. Those timings were not rerun or re-attested, and no gate or target changed. No new objective miss or follow-up was introduced by this repair.

## Actual PDF verification

The source PDF SHA256 is `6470900fce80751fc31cc97401933fba56e5f2ad8848a353a7e34d6f16cc0647`; its original diagnostic bbox SHA256 is `1cb13f4c3d8c444bc474e1e441e4301df6f5b6c03e00856f79131e7c237edd82`. Original diagnostic and failed-index receipts remain frozen.

A separate read-only probe confirms strict whole-document rejection on the original bbox bytes and exactly two isolated failures, pages 30/34. All **45 valid page layouts match exactly**, including words, ordering, geometry and sentence/token identities, against the strict parser with only the rejected page bodies absent. That comparison string was never persisted or used for indexing. The probe took 0.4660 seconds; its exact reviewed source copies, Cargo-selected libraries, executable and result are fingerprinted. Their source hashes still match the final branch.

The production `k1_index` helper was selected from Cargo's compiler-artifact output and frozen with its source/build receipts. Its one-paper request used the existing `e723f251047f05c0` identity and dedicated external corpus data root. The actual PDF was processed **once**:

| Result | Measured value |
| --- | --- |
| Paper processing | 3.272503362 seconds |
| Full helper, including catalog scan | 10.924505562 seconds |
| Maximum resident memory | 47,344 KiB |
| Persisted pages / tokens | 47 / 18,962 |
| Page provenance | 45 native; pages 30 and 34 OCR |
| Retained gaps | Native ordered-finite-coordinate failures on pages 30 and 34 |
| Network / model calls / cost | 0 / 0 / $0 |

New index SHA256: `8b3e01363ad7383cd8019f71d2e9818e8bc447bef799b0ece82b2c6bd71e85b8`. The schema-6 generation is `1789302227502573966-27-0`. Both failed pages contain only OCR-labeled tokens; their original native errors remain inspectable.

The ordinary catalog scan added 23 PDFs downloaded since the prior scan, yielding 10,951 identity rows. Every one of the previous 10,928 rows and all registry-level metadata are identical; none were removed or reassigned. The first audit required whole-registry equality and stopped on these additions. A separate exact row/hash audit then confirmed preservation without rerunning the PDF. All 1,749 existing index hashes match their retained before manifest (5,151,173,020 bytes). Independent review repeated that full hash comparison and verified the additions and exact layout probe.

This new derived index does not modify the 999 successful eval / 750 scale index receipts, retroactively admit any truth labels, or certify OCR math as exact native transcription. No corpus PDF or deposited source was copied into the repository, library or `.lysilogy`.

## Retained evidence

The committed [reading-page-isolation evidence](../../eval/evidence/reading-page-isolation.json) binds before/after results, final source and log hashes, G5 counts, the compatibility probe, actual retry and independent review. Raw receipts live under `~/.cache/lysilogy/page-failure-development/`; the standalone helper and exact source/build bundle are at `~/.cache/lysilogy/k1-native-page-isolation-0843453/`. The helper remains available after the worktree is removed.

Independent source review cleared `5cc0199`; the subsequent merges add reviewed K1 tooling and phase notes without changing this implementation. Independent final integration/live review cleared the actual run at `0843453`, including all old index hashes, registry rows, the Cargo-selected helper and 45 valid-page comparison. Final report and latest warm-gate evidence receive a separate review before merge.
