# E8.5 person silver-label tooling

The offline K4 builder and its feature boundary are implemented and independently reviewed.
Genuine K4 publication is still pending frozen OpenAlex records and complete K2/K0 membership;
G1.persons and O13 remain unavailable. This is a partial draft, explicitly stacked on E8.4
[#71 / PR #93](https://github.com/tjmisko/Lysilogy/pull/93), whose provenance tooling is reused.
Neither issue can be marked complete from fixture or tooling success.

## Identity and provenance

K4 uses the per-authorship `raw_orcid` as its silver label. OpenAlex's resolved `author.orcid`
propagates the profile's identity onto its associated works; using that alone would reuse the
provider's prior clustering as truth. The official
[OpenAlex ORCID documentation](https://help.openalex.org/data/authors/orcid/) explains this
distinction. Profile-only slots remain excluded and counted.

A mention is work DOI plus zero-based authorship array index. Raw names supply inputs; the
provider's first/middle/last role and resolved display name do not identify slots. Valid but
conflicting raw/profile ORCIDs exclude a slot, and an ORCID repeated across coauthor positions
excludes every affected slot. Across works, a provider profile spanning multiple raw ORCIDs is
diagnostic and does not define the labels. The
[authorship documentation](https://help.openalex.org/data/authorships/) supports the raw-name,
position and incomplete-byline interpretation.

The validator checks canonical ASCII ORCID shape and MOD 11-2 checksum, preserving the original
observed string separately. This does not prove registry assignment; see the
[official ORCID specification](https://support.orcid.org/hc/en-us/articles/360006897674-Structure-of-the-ORCID-Identifier).
Frozen response bytes, original fetch time, exact requested/returned DOI, JSON pointer and
authorship hash bind each observation. Raw provider payloads remain outside the repository.

The feature export contains only opaque mention ID, raw name, work DOI and array index. ORCID
labels, profile IDs, provenance and expected clusters stay in evaluator-only truth. Explicit
ORCID/OpenAlex identifiers embedded in names are excluded, including compatibility-width and
control obfuscation. The CLI verifies K4's content-derived version before projection; the pure
projection remains independent of label values. Oversize outputs are refused before publication.

## Actual offline metadata preparation

The planner read the genuine frozen K0 selection `172d18c2…`; it made no network or model calls.
It observed 10,951 selected metadata rows, with 9,146 missing DOI values, two invalid DOI values,
and 1,803 valid unique DOIs. The resulting 73 requests at 25 DOIs per batch are unexecuted.
This establishes metadata membership only, with no PDF download, mapping or K4 label claim.

The external universe file is
`~/.cache/lysilogy/person-truth/k0-172d18c2-universe-v1.json`, SHA256
`540893f9a21136bc7775f1fc7d61e382472623c778d1eb190669933d6ffaae9d`.
Its request plan is `k0-172d18c2-requests-v1.json`, SHA256
`dd84e0faeceb06bd9c59177920cf02b584480e424392ef9458db80b87e19f591`.
The [retained evidence](../../eval/evidence/person-silver-labels.json) records their source and
content fingerprints. This K0-only plan is preparatory: first build genuine K2, then form the
combined K2/K0 universe and freeze its OpenAlex plan once for reuse by both K4 and K5.

## Validation and scorecard

The clean before-persons evaluation ran at `cc329dd`; final gates ran at reviewed source
`b33e92491bacc2f2a399251562d88bd1aac8f911`, with main `9b6b3d8` integrated through normal merges.
Both persons results record `dirty=false`. Subsequent scale/G5 runs truthfully record
`dirty=true` after the generated scorecard changed; tested implementation bytes stayed fixed.

| Check | Result |
| --- | --- |
| Formatting; strict all-target/all-feature Clippy | Pass |
| All-target Rust tests | 309 pass |
| New offline K4 Python tests | 22 pass |
| Complete truth tooling Python suite | 55 pass |
| G5 isolated Rust / Python / Node tests | 309 / 165 / 85 pass |
| O30 provider-budget violations | 0 / 10,000 requests |
| Persons `eval … --check`, before and after | Pass; G1.persons and O13 unavailable, zero real cases |
| Genuine K4 publication | Pending |

Tests cover ORCID validity and conflict policy, repeated coauthors, distinct middle positions,
same-name/different-ORCID separation, raw/profile provenance, profile-only coverage, incomplete
bylines, immutable receipt reuse, request/source identity, input fingerprints, bounded output,
Unicode identifier exposure and label-free feature projection. CLI regressions prove changed
frozen bytes or stale K4 versions cannot replace earlier labels/features. G5 confirms network
isolation and absent model CLIs. No frontend source changed.

No target or baseline changed. Current available scorecard coverage remains G5 passing and O30
at target; historical O25/O26 misses remain tracked by #21/#22. The evidence embeds exact eval
results, implementation hashes, all isolated test logs and final gate receipts. Passing commands
with unavailable metrics does not meet the system's hard-gate or truth acceptance requirements.

## Remaining acceptance

Runtime host policy still blocks fresh Crossref/OpenAlex responses, and the sanctioned provider
cache contains no genuine responses to reuse. This issue made no new provider probe or bypass.
Provider/model calls and cost for this work are zero; no model-backed verification ran.

After approved provider access, follow the [K4 workflow](../../scripts/truth/PERSONS.md): freeze
genuine K2 and shared OpenAlex inputs, derive K4 with both membership origins, inspect exclusions,
cluster diversity and truncation coverage, and retain the actual silver-label version. Keep
#72 draft until those labels and its independent acceptance review exist. Actual G1/O13
measurement also requires the later person resolver/collector; fixtures cannot supply it.
