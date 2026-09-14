# Bounded native diagnostics recover the batch 28 failure

Issue #118 fixes the recorded extraction failure for arXiv 2106.09069v1. The original PDF made `pdftotext` emit more than the old 64 KiB stderr limit despite exiting successfully. The reviewed production path now creates an 18-page native reading index from those exact PDF bytes. Independent review confirmed the new index and byte-for-byte preservation of all 7,999 previously successful indexes.

The new policy applies explicitly to the native `pdftotext` call. It retains the first 65,536 original diagnostic bytes, drains at most 1,048,576 bytes, and rejects upon observing byte 1,048,577. Reads use a fixed 8,192-byte scratch buffer. At EOF the observed count is exact; overflow records a lower bound with `complete=false`. A typed result carries the retained prefix and count metadata. One bounded tracing event losslessly ASCII-escapes the prefix, including invalid UTF-8 and control bytes. Escaped prefix text is at most 256 KiB. The `k1_index` helper installs a warning sink on stderr, separate from its JSON stdout.

The default command wrapper retains its strict 64 KiB policy, including when its display name is `pdftotext`. OCR, graphics and mask extraction therefore retain their existing limits. The native 35-second timeout, 48 MiB stdout bound, concurrent readers, nonzero-exit errors and owned-child cancellation remain enforced. Diagnostics do not enter reading text, geometry, gaps or the cache schema. No parser, detector, truth, OCR decision, native normalization or cache invalidation changes are included.

| Observation | Original failure | Reviewed retry |
| --- | --- | --- |
| PDF identity | `5ad6cdda040648c1`, PDF SHA256 `4c592541…` | Same original bytes |
| Application result | Stderr-bound failure; no index | Canonical reading index |
| Diagnostic bytes | 169,653 in the retained diagnostic experiment | 169,653 drained; 65,536 retained |
| Native / OCR / unavailable pages | No accepted index | 18 / 0 / 0 |
| Stored tokens / UTF-16 units | No accepted index | 9,493 / 62,185 |
| Page gaps | No accepted index | 0 |
| Existing successful indexes | 7,999 | All 7,999 byte-identical |

The new index SHA256 is `b035196b4fd5d3156145c56243ea741911fca0d66ad344093dbc3f9a638fcaca`. Its diagnostic prefix exactly matches the original retained prefix, SHA256 `953a31281c1eb39bdc88c567ab946fd5335aaff4cd208563c4a5ba0adce03222`. No full-stderr hash is claimed. The earlier XML diagnostic counted 9,520 word elements; the application stores 9,493 tokens after its unchanged normalization and assembly. These counts describe different representations and do not prove semantic completeness.

The application processing interval was 1.092720 seconds; the helper took 3.758814 seconds. Including complete before/after preservation checks, the wrapper took 87.185776 seconds with 32,800 KiB peak child RSS. Each preservation pass rehashed 23,325,711,591 bytes across 7,999 indexes. Registry bytes, the PDF, original ledgers and receipts, and bound source/tool bytes stayed exact. The helper binary and its 78-source Cargo build bundle are retained outside the worktree.

The retry used a reviewed wrapper with a minimal environment and exact resolved `nice`, `pdftotext`, `pdftoppm` and `tesseract` identities. It drained helper stdout and stderr concurrently under 1 MiB and 512 KiB bounds, with a 240-second deadline and 1.5 GiB memory limit. Process-group cleanup covers descendants even after the leader exits. Original batch 28 failure records remain unchanged; successor queue activation is separate work.

Validation includes the clean own-CLI before check, 13 new command tests, the existing stdout-overflow control, and eight generated wrapper controls. Tests cover exact boundaries, nonzero exits with truncated diagnostics, concurrent pipes, stdout overflow, deadline, cancellation, compatibility, descendant cleanup and tool/environment binding. Independent source review replayed the compiled tests without rebuilding. The final integrated run passed all 12 commands: formatting, strict Clippy with all targets/features, all-target Rust tests, Python tests, the CLI build, synthetic provider budgets, objects/bibliography/scale checks, isolated G5 and both immutable truth replays. Raw G5 logs contain 381 Rust, 505 Python and 85 Node tests. Final gates took 42.482783 seconds with 369,808 KiB peak child RSS.

The clean before check and final actual current-v2 collector both have available O1 = 1 and O2 = .9169720168893188 over the same three papers and 23 objects. Every previous per-paper outcome, prediction, truth/input identity and geometry value is exact; both score deltas are zero. The gate-stage objects check initially withheld these values because its incoming native-source fingerprints were stale. The subsequent `--executable` measurement refreshed provenance and produced exactly one fresh available result. It took 27.906131 seconds. The selected CLI and Cargo-selected bridge, raw build logs and full prediction output are archived outside the worktree.

The portable evidence index is [native-diagnostics.json](../../eval/evidence/native-diagnostics.json). It binds original failure/diagnostic evidence, reviewed source and executable identities, raw command/G5 logs, both preserved releases, actual recovery, full preservation ledgers, current metric observations and independent reviews. Earlier unlaunched wrapper findings, a synthetic process-observation race, and the review harness's docs-only HEAD precondition failure remain recorded as preparation/audit history. They are distinct from the successful native retry. Gate-generated Python bytecode was archived before individual cleanup; source bytes remained unchanged.

Experiments made zero network/model calls and incurred $0 external service cost; agent reasoning cost is not measured. Ordinary main integration followed the standing restriction on rebasing. This change demonstrates recovery of the one recorded failure. It does not establish a production 10,000-paper run, O25/O27 success, semantic truth or expanded K1 admission. The approximately 500-paper coverage objective in #97 remains open.
