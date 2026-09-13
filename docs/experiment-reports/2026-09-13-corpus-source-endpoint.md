# Canonical arXiv source endpoint — 2026-09-13

Issue #91 changes new corpus source requests to the exact version-pinned
`https://export.arxiv.org/src/<id>v<n>` endpoint. Previously admitted sources with an exact
`https://export.arxiv.org/e-print/<same-id>v<same-version>` receipt remain reusable after byte/hash
verification. Their original URL, fetch time and bytes are preserved, including when a crash
occurred before manifest publication. Different hosts, versions, paths, query strings,
fragments, receipt kinds and mismatching bytes are rejected without replacing existing files.
PDF generation URLs remain exact. Redirect following, destination TLS, the three-host
allowlist, explicit proxy opt-in, shared arXiv pacing and disk floor are unchanged.

The preceding bounded live run admitted PDF `0812.5080v5`, then stopped at the source endpoint's
HTTP 301. It took 4.00 seconds and peaked at 240,944 KiB RSS; model cost was $0. The parent
independently confirmed a redirect to the same approved host's `/src/0812.5080v5` and directly
probed that canonical URL without following redirects. It returned HTTP 200 gzip, 21,993 bytes,
SHA-256 `8b95087c0ab3a43d4f021459374bc52a66a4baae9211174f83984cb12f250c1d`, in 0.45 seconds.
Those probe bytes were not admitted as a corpus source.

## Validation and scorecard

Fresh before measurements ran on `0030399`; final integrated measurements ran on `ebead0c`
after normally merging docs-only main `5de68a4`. No rebase was performed under the standing
approval restriction. Independent review cleared source `ebead0c` and independently ran all
59 offline corpus tests.

| Check | Before | After |
| --- | ---: | ---: |
| Offline corpus tests | 52 pass | 59 pass |
| G5 isolated Rust / Python / Node tests | 305 / 80 / 85 pass | 305 / 87 / 85 pass |
| O30 budget violations | 0 / 10,000 | 0 / 10,000 |
| `eval scale --check` | pass | pass |
| `eval tests --check` | pass | pass |

Formatting, strict Clippy with all targets/features, and all-target Rust tests pass. Both scale
runs used clean trees; G5 truthfully reports `dirty=true` after scale generated the scorecard,
which was subsequently restored. Tests made no network/model calls; G5 verified its network
namespace isolation and absent model CLIs. Seven new tests cover canonical version pinning,
legacy resume and interrupted manifest publication, wrong identity/route/path rejection,
preserved source bytes, invalid receipt kinds/hashes and continued redirect refusal.

The [retained receipt](../../eval/evidence/corpus-source-endpoint.json) contains exact source
hashes, full isolated test outputs, command results, scorecard observations and the original
live source failure/probe evidence. G5/O30 remain unchanged; no gate or objective target was
changed, and K0-dependent objectives remain unavailable until their real corpus and collectors
are complete.

## Live verification boundary

The bounded integrated download succeeded with exit 0 in 3.19 seconds, peaking at 240,608 KiB
RSS with $0 model cost. It rehashed and reused the already-admitted 327,794-byte PDF
`0812.5080v5`, then downloaded and admitted its 21,993-byte canonical source. The source
SHA-256 is `8b95087c0ab3a43d4f021459374bc52a66a4baae9211174f83984cb12f250c1d`,
fetched at `2026-09-13T08:26:54.029223+00:00`. The parent independently verified both receipts'
identities and actual bytes. Selection SHA-256 remains
`172d18c2eeb8a640ead81f55261619800e2728553c7b25f87a191393d25b3e5f`.

Live verification combined this reviewed source with PR #90's schema-2 availability validation.
The first fetch ran at `a975703f07c32f7f94ecf3cfd75a0d29e74bd0bd`; integration commit
`06dc69329ae0064507ba76e8196752bd64883fd5` has the identical Git tree after a message-only
amend to place the coauthor trailer last. All 82 combined offline corpus tests passed. The
retained receipt records the exact combined source hashes, standalone receipt hash and full
live log (SHA-256 `8a4d295852a8daa24801549030691e94c0c1a2b7168bdb10e91a3779c40c244a`).

This verifies one PDF/source pair. Complete K0 verification remains outstanding. The recovered
selection and admitted artifacts remain intact; reselection is forbidden after artifact admission.
