# Corpus availability and explicit recovery — 2026-09-13

Issue #88 repairs the first complete metadata harvest's failure at unavailable public PDF
`1801.00600`. Selection now qualifies each candidate against a preserved complete public GCS
inventory before filling its deterministic category/year quota. Exact 1,000 eval and 10,000
scale targets remain unchanged. Missing, invalid or oversized latest objects have explicit
exclusion reasons; replacements come from the same stratum ranking. Version, generation, size
and MD5 are frozen in the selection before downloads start.

The original run harvested 514,249 metadata records and froze 10,951 unique selected papers,
then exited before any manifest row, PDF or source was admitted. Its selection hash was
`196b6f49520e10f0e33ef306313ec0908f9cb23ad9cca680184c059484ad71e8`; the exact selection file hash was
`c0a206cab1d997c793bd5853d1e25c24cb5f281cb30992c3e1b0f8b9445c28bb`.
An independent read-only checkpoint at 07:46:49 UTC confirmed these counts and zero artifacts.
The parent agent's completed process receipt measured 45:13.98 and peak RSS 3,732,208 KiB, with
$0 model cost. The peak covers the whole original run; it does not isolate selection.

Fresh rate-coordinated OAI and exact-prefix GCS probes reported by the parent confirmed the
original ID and observed `created=2020-06-30`, and no public PDF object. The repair preserves
that observed metadata even when the ID suggests an earlier year. It does not infer new dates.

## Recovery and durability

Ordinary resume preserves both legacy and new frozen selections. Explicit
`recover-selection --reason …` applies only to legacy metadata-only selections with zero
manifest rows and no artifacts, receipts, partials or other entries in artifact directories.
It archives the exact original selection bytes and failure reason without replacement,
checks that both the metadata hash and original deterministic selection agree, and retains
complete consulted inventory snapshots across interruption. It records unavailable original
members alongside any additional excluded candidates. A complete replacement is durably staged
before atomic publication; retries reuse it, including after a crash immediately after publish.
Insufficient available capacity leaves the original selection intact. Any admitted artifact
prevents recovery.

Verification checks quotas, inventory fingerprints, pinned object identity, exclusion reasons
and recovery archive provenance. Source URLs still use the frozen PDF version. Destination
hosts, explicit proxy opt-in, origin TLS, redirect refusal, shared arXiv pacing and disk floor
are unchanged. Metadata hashing now streams canonical records to avoid allocating another full
metadata JSON array; no revised live RSS improvement is claimed without measurement.

## Validation and scorecard

The fresh before measurements ran on `26b4adc`; final integrated measurements ran on `9ba9fe5` after merging docs-only main `0030399`.
Current main was integrated with an ordinary merge because the standing rebase approval
restriction remains in effect. The independent reviewer cleared source `96b702d` and ran all
75 corpus tests; subsequent commits contain only documentation, retained evidence and the docs-only main merge.

| Check | Before | After |
| --- | ---: | ---: |
| Offline corpus tests | 52 pass | 75 pass |
| G5 isolated Rust / Python / Node tests | 295 / 80 / 85 pass | 305 / 103 / 85 pass |
| O30 budget violations | 0 / 10,000 | 0 / 10,000 |
| `eval scale --check` | pass | pass |
| `eval tests --check` | pass | pass |

Formatting, strict Clippy with all targets/features, and all-target Rust tests pass. The Rust
test increase includes the separately merged title normalizer. Both scale measurements used
clean trees; both G5 runs truthfully report `dirty=true` after scale generated the scorecard.
Only that generated scorecard was restored afterward. Tests used fixtures, no network and no
models; G5 verified the isolated network namespace and absent model CLIs.

The retained [receipt](../../eval/evidence/corpus-availability.json) includes exact source hashes,
full isolated test outputs, command results, scorecard observations and original live failure
provenance. G5 and O30 remain the available passing gate/objective for this change; K0-dependent
metrics remain unavailable until the corpus and their collectors are complete. No gate or
objective target changed, and no additional follow-up was opened for this repair.

## Live verification

Independent source review and all offline gates passed before the parent agent launched the
explicit real-corpus recovery at unchanged source `013911a`, timed with `/usr/bin/time -v`.
The reason is `Selected PDF 1801.00600 is absent from the public GCS bucket (verified 2026-09-13).`
Its log is `~/.cache/lysilogy/arxiv-corpus-recovery.log`. Recovery is in progress; no completed
recovery or download success is claimed yet. The parent will next download one eval paper to
exercise its PDF and source, and the completed receipt will be recorded before merge. Complete
K0 verification remains outstanding.
