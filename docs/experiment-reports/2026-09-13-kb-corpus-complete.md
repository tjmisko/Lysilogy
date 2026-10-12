# Complete K0 corpus verification — 2026-09-13

The separately stored arXiv corpus now meets its download and verification targets. The final
production `corpus.py run` completed with exit 0 and no verification problems. It validated the
frozen selection, availability evidence, versioned receipts, and SHA-256, MD5 and byte length of
every admitted artifact.

| Tier | Selected | Verified PDFs | Verified sources |
| --- | ---: | ---: | ---: |
| Eval | 1,000 | 1,000 | 1,000 |
| Scale | 10,000 | 10,000 | 49 |
| Unique union | 10,951 | 10,951 | 1,000 |

The tiers overlap by 49 papers. Sources are required only for eval papers. Artifact size is
46,892,682,976 bytes (43.67 GiB), excluding metadata, inventory and receipt files. Everything
remains under `~/Corpora/arxiv/`; no PDF or source archive was copied into the repository,
`local-articles`, or `.lysilogy`. The mapped index has its own data root at
`~/.cache/lysilogy/arxiv-kb-data/`.

The priority eval download took 1:15:30 with 241,008 KiB peak RSS. The subsequent complete resume
and final verification took 1:45:40 with 303,808 KiB peak RSS. These are resumed stage timings,
not total corpus construction time: the earlier harvest and availability recovery are reported
in [the recovery report](2026-09-13-corpus-availability.md). Both stages made zero model calls;
model cost was $0. Downloads used the approved public GCS and canonical arXiv source endpoints,
with the existing single-connection pacing, retry handling, origin validation and 20 GiB disk
floor. The independent metadata audit observed 82.25 GiB free after download completion.

## Frozen evidence

The final run used permanent-main corpus implementation SHA-256
`82991d0ad9e4c1beb277402836426a03dff6a4ed2a886bdc4524b80ef8812930`.
Its retained log is `~/.cache/lysilogy/arxiv-corpus-full-resume.log`, SHA-256
`965846a9a2197c37ae6d1d3d4259c2d1985def78ce1aaf8fb51f2dd565e33308`.
The log includes the complete final verification counts and `/usr/bin/time -v` receipt.
The preceding eval log is `~/.cache/lysilogy/arxiv-corpus-eval-priority.log`, SHA-256
`de67b52a2aedcb96c717e4a380aa5319d8c70949750b43b4cd2944a79eb96ccc`.

| Frozen input | SHA-256 |
| --- | --- |
| Metadata records | `4e85f3319ae397687d8bd80796846cc2596cde872843089469fd40d5ad25e2d6` |
| Selection identity | `172d18c2eeb8a640ead81f55261619800e2728553c7b25f87a191393d25b3e5f` |
| Exact selection file | `5c2a5f7c556fadf947ac129040e00c9c4d68399ec067fddff5649179a0d1a686` |
| Complete manifest | `b89786b72a4f12720b24c54650d36a1d2e7ac212deb6b405ee80b60c9ad4c1e0` |

Separately, the root agent checked all selected IDs and tier/stratum memberships against the
manifest, every version and artifact path, all 11,951 sidecars against their manifest receipts,
file sizes, expected HTTPS hosts and the disk floor. All 1,000 source receipts use `/src/`.
This independent metadata audit took 0.665 seconds and made no network or model calls. Its
receipt is `~/.cache/lysilogy/arxiv-complete-corpus-metadata-review.json`, SHA-256
`2525f2047d5397950e98a83e4169bf0e3cb0ff52757d96462bc809e0fc5d4ff7`;
the script is retained beside it as `verify-complete-corpus-metadata.py`. The production run
performed the full content rehash; the independent audit checked metadata and sidecar agreement.

## Remaining system work

K0 completion does not complete Phase A or the 10k app acceptance scenario. Native index
construction, independent K1 and provider truth, actual detector metrics, and later phases
remain in progress. The full eval index has 999 successful papers and one explicit coordinate
failure tracked by [#98](https://github.com/tjmisko/Lysilogy/issues/98); no failed paper is counted
as native indexing success. Two subsequent 250-paper scale batches succeeded, with the third
running when this report was written. Batch preparation is not an O25/O27 benchmark.

The scorecard remains G5 passing and O30 at target: 1/5 gates and 1/30 objectives. Existing
O25/O26 misses retain #21/#22. No target or baseline changed. Corpus construction processes have
finished; do not restart downloads or repeat selection recovery.
