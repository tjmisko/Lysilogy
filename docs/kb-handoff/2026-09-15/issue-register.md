# Issue and pull-request register

[Back to guide](README.md)

Snapshot: **2026-09-16T04:34:30.832246+00:00**, main `4972d63f10efdc2a89d01901b1adc0624408e868`. This register covers every issue and PR returned by the repository inventory: **94 issues** (35 closed, 59 open), plus **41 PRs**. Counts include earlier reader work and epic trackers. KB implementation/follow-up issues account for **80 issues**, **32 closed** and **48 open**; issue closure is not system acceptance.

## Current scheduling state

Phase **A / Wave A2** remains active. All seven original A1 foundations merged. A3 (#37), all B–D waves, phase exit reports and final system verification remain unfinished. Closed prerequisite issues make some later work dependency-ready, but the current phase ordering still applies. The proposed parallel product lane has not been adopted.

The blocker column reflects explicitly listed issue dependencies, not every operational restriction. In particular, the truth-before-detector merge rule, missing provider datasets, disk floor and the strategic implementation pause still apply even when all listed blockers are closed.

## KB implementation and follow-up issues

### A1

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#19](https://github.com/tjmisko/Lysilogy/issues/19) [E0.1] Synthetic 10k vault and scale benchmarks | closed | Merged via [#82](https://github.com/tjmisko/Lysilogy/pull/82). | None listed |
| [#20](https://github.com/tjmisko/Lysilogy/issues/20) [E0.2] Content-hash paper identity | closed | Merged via [#80](https://github.com/tjmisko/Lysilogy/pull/80). | None listed |
| [#24](https://github.com/tjmisko/Lysilogy/issues/24) [E1.1] PaperObject types and objects artifact | closed | Merged via [#77](https://github.com/tjmisko/Lysilogy/pull/77). | None listed |
| [#34](https://github.com/tjmisko/Lysilogy/issues/34) [E2.2] KB domain types | closed | Merged via [#74](https://github.com/tjmisko/Lysilogy/pull/74). | None listed |
| [#63](https://github.com/tjmisko/Lysilogy/issues/63) [E7.1] Provider cache and rate budgets | closed | Merged via [#78](https://github.com/tjmisko/Lysilogy/pull/78). | None listed |
| [#68](https://github.com/tjmisko/Lysilogy/issues/68) [E8.1] Evaluation harness, scorecard, and ratchet | closed | Merged via [#76](https://github.com/tjmisko/Lysilogy/pull/76). | None listed |
| [#69](https://github.com/tjmisko/Lysilogy/issues/69) [E8.2] arXiv research corpus | closed | Merged via [#75](https://github.com/tjmisko/Lysilogy/pull/75). | None listed |

### A2

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#25](https://github.com/tjmisko/Lysilogy/issues/25) [E1.2] Backend bibliography extraction and parsing | open | Branch implemented; draft #94; genuine K2/full O9 missing. | [#24](https://github.com/tjmisko/Lysilogy/issues/24) closed |
| [#33](https://github.com/tjmisko/Lysilogy/issues/33) [E2.1] SQLite store, migrations, and rebuild | open | Branch implemented; no PR; Rust dependencies/build/gates blocked. | None listed |
| [#35](https://github.com/tjmisko/Lysilogy/issues/35) [E2.3] Person name parser and normalizer | closed | Merged via [#87](https://github.com/tjmisko/Lysilogy/pull/87). | [#34](https://github.com/tjmisko/Lysilogy/issues/34) closed |
| [#36](https://github.com/tjmisko/Lysilogy/issues/36) [E2.4] Title normalizer and fuzzy index | closed | Merged via [#89](https://github.com/tjmisko/Lysilogy/pull/89). | [#34](https://github.com/tjmisko/Lysilogy/issues/34) closed |
| [#70](https://github.com/tjmisko/Lysilogy/issues/70) [E8.3] arXiv LaTeX object truth | closed | Merged via [#99](https://github.com/tjmisko/Lysilogy/pull/99). | [#68](https://github.com/tjmisko/Lysilogy/issues/68) closed |
| [#71](https://github.com/tjmisko/Lysilogy/issues/71) [E8.4] Reference, acquisition, and read-next truth | open | Offline builders; draft #93; real K2/K5/K7 incomplete. | [#68](https://github.com/tjmisko/Lysilogy/issues/68) closed |
| [#72](https://github.com/tjmisko/Lysilogy/issues/72) [E8.5] Person silver labels | open | Offline builder; stacked draft #95; real K4 absent. | [#68](https://github.com/tjmisko/Lysilogy/issues/68) closed |
| [#88](https://github.com/tjmisko/Lysilogy/issues/88) [E8.2 follow-up] Preserve corpus quotas when public PDF objects are missing | closed | Merged via [#90](https://github.com/tjmisko/Lysilogy/pull/90). | [#69](https://github.com/tjmisko/Lysilogy/issues/69) closed, [#85](https://github.com/tjmisko/Lysilogy/issues/85) closed |
| [#91](https://github.com/tjmisko/Lysilogy/issues/91) fix: use the canonical arXiv source endpoint | closed | Merged via [#92](https://github.com/tjmisko/Lysilogy/pull/92). | [#69](https://github.com/tjmisko/Lysilogy/issues/69) closed, [#85](https://github.com/tjmisko/Lysilogy/issues/85) closed |
| [#96](https://github.com/tjmisko/Lysilogy/issues/96) eval: measure figure and table detection against independent K1 | closed | Merged via [#102](https://github.com/tjmisko/Lysilogy/pull/102). | [#70](https://github.com/tjmisko/Lysilogy/issues/70) closed |
| [#97](https://github.com/tjmisko/Lysilogy/issues/97) eval: expand K1 to the planned stratified paper coverage | open | Open coverage shortfall; small reviewed releases only, not approximately 500 papers. | [#70](https://github.com/tjmisko/Lysilogy/issues/70) closed |
| [#98](https://github.com/tjmisko/Lysilogy/issues/98) fix: retain readable pages when native PDF coordinates fail | closed | Merged via [#100](https://github.com/tjmisko/Lysilogy/pull/100). | [#19](https://github.com/tjmisko/Lysilogy/issues/19) closed, [#20](https://github.com/tjmisko/Lysilogy/issues/20) closed |
| [#101](https://github.com/tjmisko/Lysilogy/issues/101) fix(objects): recover Roman tables and improve measured full-body regions | closed | Merged via [#104](https://github.com/tjmisko/Lysilogy/pull/104). | [#96](https://github.com/tjmisko/Lysilogy/issues/96) closed |
| [#103](https://github.com/tjmisko/Lysilogy/issues/103) fix(objects): resolve bibliography title line-break hyphens with retained evidence | open | Known five title misses; follow-up queued after #25. | [#25](https://github.com/tjmisko/Lysilogy/issues/25) open |
| [#105](https://github.com/tjmisko/Lysilogy/issues/105) eval: publish reviewed manual K1 cohorts with immutable replay | closed | Merged via [#108](https://github.com/tjmisko/Lysilogy/pull/108). | [#70](https://github.com/tjmisko/Lysilogy/issues/70) closed |
| [#106](https://github.com/tjmisko/Lysilogy/issues/106) [E1.1] Recover missing regions and reject duplicate captions on expanded K1 | closed | Merged via [#112](https://github.com/tjmisko/Lysilogy/pull/112). | [#105](https://github.com/tjmisko/Lysilogy/issues/105) closed |
| [#107](https://github.com/tjmisko/Lysilogy/issues/107) eval: preserve source objects with ambiguous duplicate labels | closed | Merged via [#113](https://github.com/tjmisko/Lysilogy/pull/113). | [#105](https://github.com/tjmisko/Lysilogy/issues/105) closed |
| [#109](https://github.com/tjmisko/Lysilogy/issues/109) fix(eval): preserve literal BibTeX field syntax (E8.3) | closed | Merged via [#119](https://github.com/tjmisko/Lysilogy/pull/119). | [#107](https://github.com/tjmisko/Lysilogy/issues/107) closed |
| [#110](https://github.com/tjmisko/Lysilogy/issues/110) feat(eval): test confined source-layout reconstruction (E8.3) | closed | Merged via [#116](https://github.com/tjmisko/Lysilogy/pull/116). | [#105](https://github.com/tjmisko/Lysilogy/issues/105) closed |
| [#111](https://github.com/tjmisko/Lysilogy/issues/111) [E1.1] Resolve remaining mask-clipped figure regions with bounded evidence | closed | Merged via [#114](https://github.com/tjmisko/Lysilogy/pull/114). | [#106](https://github.com/tjmisko/Lysilogy/issues/106) closed |
| [#115](https://github.com/tjmisko/Lysilogy/issues/115) eval: publish reconciled visual pilot with attached table notes (E8.3) | closed | Merged via [#122](https://github.com/tjmisko/Lysilogy/pull/122). | [#105](https://github.com/tjmisko/Lysilogy/issues/105) closed, [#111](https://github.com/tjmisko/Lysilogy/issues/111) closed |
| [#117](https://github.com/tjmisko/Lysilogy/issues/117) Publish fixed visual-only pilot cohorts without formal metric admission | closed | Merged via [#126](https://github.com/tjmisko/Lysilogy/pull/126). | [#115](https://github.com/tjmisko/Lysilogy/issues/115) closed |
| [#118](https://github.com/tjmisko/Lysilogy/issues/118) fix: retain usable native output with bounded diagnostic drainage | closed | Merged via [#120](https://github.com/tjmisko/Lysilogy/pull/120). | [#98](https://github.com/tjmisko/Lysilogy/issues/98) closed |
| [#121](https://github.com/tjmisko/Lysilogy/issues/121) fix: investigate two weak visual-tranche figure regions (E1.1) | closed | Merged via [#124](https://github.com/tjmisko/Lysilogy/pull/124). | [#115](https://github.com/tjmisko/Lysilogy/issues/115) closed |
| [#123](https://github.com/tjmisko/Lysilogy/issues/123) feat: measure bounded vector support for figure regions (E1.1) | closed | Merged via [#128](https://github.com/tjmisko/Lysilogy/pull/128). | [#121](https://github.com/tjmisko/Lysilogy/issues/121) closed |
| [#125](https://github.com/tjmisko/Lysilogy/issues/125) Recover split and interleaved captions in the visual-only K1 cohort (E1.1) | closed | Merged via [#127](https://github.com/tjmisko/Lysilogy/pull/127). | [#117](https://github.com/tjmisko/Lysilogy/issues/117) closed |
| [#129](https://github.com/tjmisko/Lysilogy/issues/129) feat: add bounded per-paper K1 release replay and collection (E8.3) | closed | Merged via [#130](https://github.com/tjmisko/Lysilogy/pull/130). | [#123](https://github.com/tjmisko/Lysilogy/issues/123) closed |
| [#131](https://github.com/tjmisko/Lysilogy/issues/131) feat: publish complete numbered-equation K1 cohorts (E8.3) | closed | Merged via [#133](https://github.com/tjmisko/Lysilogy/pull/133). | [#129](https://github.com/tjmisko/Lysilogy/issues/129) closed |
| [#132](https://github.com/tjmisko/Lysilogy/issues/132) feat: publish complete formal K1 cohort (E8.3) | open | Branch + successful retry evidence; no PR; independent audit/final integration pending. | [#131](https://github.com/tjmisko/Lysilogy/issues/131) closed |
| [#134](https://github.com/tjmisko/Lysilogy/issues/134) feat: add the complete 2301 numbered-equation cohort (E8.3) | open | Reviewed proposal; queued after #132; no code/worktree. | [#132](https://github.com/tjmisko/Lysilogy/issues/132) open |
| [#135](https://github.com/tjmisko/Lysilogy/issues/135) feat: add the complete 2011 statement cohort (E8.3) | open | Reviewed proposal; queued after #132 and #134; no code/worktree. | [#132](https://github.com/tjmisko/Lysilogy/issues/132) open, [#134](https://github.com/tjmisko/Lysilogy/issues/134) open |

### A3

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#37](https://github.com/tjmisko/Lysilogy/issues/37) [E2.5] Resolution gold set and evaluation | open | Planned; no implementation branch in this inventory. | [#25](https://github.com/tjmisko/Lysilogy/issues/25) open |

### B1

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#21](https://github.com/tjmisko/Lysilogy/issues/21) [E0.3] Incremental catalog scan | open | Planned; no implementation branch in this inventory. | [#19](https://github.com/tjmisko/Lysilogy/issues/19) closed |
| [#22](https://github.com/tjmisko/Lysilogy/issues/22) [E0.4] Home grid virtualization and server-side search | open | Planned; no implementation branch in this inventory. | [#19](https://github.com/tjmisko/Lysilogy/issues/19) closed |
| [#23](https://github.com/tjmisko/Lysilogy/issues/23) [E0.5] Bounded concurrent batch ingest | open | Planned; no implementation branch in this inventory. | [#19](https://github.com/tjmisko/Lysilogy/issues/19) closed |
| [#26](https://github.com/tjmisko/Lysilogy/issues/26) [E1.3] Numbered equation detection | open | Planned; no implementation branch in this inventory. | [#24](https://github.com/tjmisko/Lysilogy/issues/24) closed |
| [#27](https://github.com/tjmisko/Lysilogy/issues/27) [E1.4] Theorem-like statements and proofs | open | Planned; no implementation branch in this inventory. | [#24](https://github.com/tjmisko/Lysilogy/issues/24) closed |
| [#28](https://github.com/tjmisko/Lysilogy/issues/28) [E1.5] Algorithm and listing detection | open | Planned; no implementation branch in this inventory. | [#24](https://github.com/tjmisko/Lysilogy/issues/24) closed |
| [#30](https://github.com/tjmisko/Lysilogy/issues/30) [E1.7] Object enrichment: ranking and quoted context | open | Planned; no implementation branch in this inventory. | [#24](https://github.com/tjmisko/Lysilogy/issues/24) closed |
| [#31](https://github.com/tjmisko/Lysilogy/issues/31) [E1.8] Figures tab replaces the Glossary slot | open | Planned; no implementation branch in this inventory. | [#24](https://github.com/tjmisko/Lysilogy/issues/24) closed |
| [#38](https://github.com/tjmisko/Lysilogy/issues/38) [E2.6] Resolution engine | open | Planned; no implementation branch in this inventory. | [#33](https://github.com/tjmisko/Lysilogy/issues/33) open |
| [#39](https://github.com/tjmisko/Lysilogy/issues/39) [E2.7] Decision log and stable identities | open | Planned; no implementation branch in this inventory. | [#33](https://github.com/tjmisko/Lysilogy/issues/33) open |

### B2

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#29](https://github.com/tjmisko/Lysilogy/issues/29) [E1.6] Link hints for new object kinds | open | Planned; no implementation branch in this inventory. | [#26](https://github.com/tjmisko/Lysilogy/issues/26) open |
| [#40](https://github.com/tjmisko/Lysilogy/issues/40) [E2.8] Ingest local papers | open | Planned; no implementation branch in this inventory. | [#20](https://github.com/tjmisko/Lysilogy/issues/20) closed |
| [#41](https://github.com/tjmisko/Lysilogy/issues/41) [E2.9] Ingest provider snapshots | open | Planned; no implementation branch in this inventory. | [#38](https://github.com/tjmisko/Lysilogy/issues/38) open |

### B3

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#42](https://github.com/tjmisko/Lysilogy/issues/42) [E2.10] KB API | open | Planned; no implementation branch in this inventory. | [#40](https://github.com/tjmisko/Lysilogy/issues/40) open |

### C1

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#44](https://github.com/tjmisko/Lysilogy/issues/44) [E2.12] Routes plus Work and Person pages | open | Planned; no implementation branch in this inventory. | [#42](https://github.com/tjmisko/Lysilogy/issues/42) open |
| [#45](https://github.com/tjmisko/Lysilogy/issues/45) [E3.1] Acquisition state and global jobs | open | Planned; no implementation branch in this inventory. | [#33](https://github.com/tjmisko/Lysilogy/issues/33) open |
| [#52](https://github.com/tjmisko/Lysilogy/issues/52) [E4.1] CSL-JSON, BibTeX, and RIS | open | Planned; no implementation branch in this inventory. | [#34](https://github.com/tjmisko/Lysilogy/issues/34) closed |
| [#55](https://github.com/tjmisko/Lysilogy/issues/55) [E5.1] Reading list model and API | open | Planned; no implementation branch in this inventory. | [#33](https://github.com/tjmisko/Lysilogy/issues/33) open |
| [#59](https://github.com/tjmisko/Lysilogy/issues/59) [E6.1] Neighborhood and metrics backend | open | Planned; no implementation branch in this inventory. | [#42](https://github.com/tjmisko/Lysilogy/issues/42) open |
| [#73](https://github.com/tjmisko/Lysilogy/issues/73) [E8.6] Reading-list evaluation | open | Planned; no implementation branch in this inventory. | [#68](https://github.com/tjmisko/Lysilogy/issues/68) closed |

### C2

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#46](https://github.com/tjmisko/Lysilogy/issues/46) [E3.2] Deterministic identifier resolution | open | Planned; no implementation branch in this inventory. | [#45](https://github.com/tjmisko/Lysilogy/issues/45) open |
| [#48](https://github.com/tjmisko/Lysilogy/issues/48) [E3.4] Agent fallback | open | Planned; no implementation branch in this inventory. | [#45](https://github.com/tjmisko/Lysilogy/issues/45) open |
| [#49](https://github.com/tjmisko/Lysilogy/issues/49) [E3.5] Downloaded PDF verification | open | Planned; no implementation branch in this inventory. | [#45](https://github.com/tjmisko/Lysilogy/issues/45) open |
| [#50](https://github.com/tjmisko/Lysilogy/issues/50) [E3.6] Post-download policy | open | Planned; no implementation branch in this inventory. | [#45](https://github.com/tjmisko/Lysilogy/issues/45) open |
| [#56](https://github.com/tjmisko/Lysilogy/issues/56) [E5.2] Keyboard reading-list view | open | Planned; no implementation branch in this inventory. | [#44](https://github.com/tjmisko/Lysilogy/issues/44) open |

### C3

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#47](https://github.com/tjmisko/Lysilogy/issues/47) [E3.3] Open-access locators | open | Planned; no implementation branch in this inventory. | [#46](https://github.com/tjmisko/Lysilogy/issues/46) open |

### C4

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#51](https://github.com/tjmisko/Lysilogy/issues/51) [E3.7] Fetch UI | open | Planned; no implementation branch in this inventory. | [#42](https://github.com/tjmisko/Lysilogy/issues/42) open |

### D1

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#32](https://github.com/tjmisko/Lysilogy/issues/32) [E1.9] Clickable proofs | open | Planned; no implementation branch in this inventory. | [#27](https://github.com/tjmisko/Lysilogy/issues/27) open |
| [#43](https://github.com/tjmisko/Lysilogy/issues/43) [E2.11] Review queue | open | Planned; no implementation branch in this inventory. | [#42](https://github.com/tjmisko/Lysilogy/issues/42) open |
| [#53](https://github.com/tjmisko/Lysilogy/issues/53) [E4.2] Styled citations | open | Planned; no implementation branch in this inventory. | [#52](https://github.com/tjmisko/Lysilogy/issues/52) open |
| [#54](https://github.com/tjmisko/Lysilogy/issues/54) [E4.3] Copy and export commands | open | Planned; no implementation branch in this inventory. | [#52](https://github.com/tjmisko/Lysilogy/issues/52) open |
| [#57](https://github.com/tjmisko/Lysilogy/issues/57) [E5.3] AI-generated reading lists | open | Planned; no implementation branch in this inventory. | [#45](https://github.com/tjmisko/Lysilogy/issues/45) open |
| [#60](https://github.com/tjmisko/Lysilogy/issues/60) [E6.2] Graph view | open | Planned; no implementation branch in this inventory. | [#44](https://github.com/tjmisko/Lysilogy/issues/44) open |
| [#64](https://github.com/tjmisko/Lysilogy/issues/64) [E7.2] Semantic Scholar recommendations | open | Planned; no implementation branch in this inventory. | [#42](https://github.com/tjmisko/Lysilogy/issues/42) open |
| [#65](https://github.com/tjmisko/Lysilogy/issues/65) [E7.3] Author identity from OpenAlex and ORCID | open | Planned; no implementation branch in this inventory. | [#38](https://github.com/tjmisko/Lysilogy/issues/38) open |

### D2

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#58](https://github.com/tjmisko/Lysilogy/issues/58) [E5.4] List relationship graph | open | Planned; no implementation branch in this inventory. | [#56](https://github.com/tjmisko/Lysilogy/issues/56) open |
| [#61](https://github.com/tjmisko/Lysilogy/issues/61) [E6.3] Keyboard graph navigation | open | Planned; no implementation branch in this inventory. | [#60](https://github.com/tjmisko/Lysilogy/issues/60) open |
| [#62](https://github.com/tjmisko/Lysilogy/issues/62) [E6.4] Communities and read-next suggestions | open | Planned; no implementation branch in this inventory. | [#59](https://github.com/tjmisko/Lysilogy/issues/59) open |

### Other foundation follow-ups

| Issue | GitHub state | Delivery / remaining work | Listed blockers |
| --- | --- | --- | --- |
| [#79](https://github.com/tjmisko/Lysilogy/issues/79) fix: include configured linker in cold G5 builds | closed | Merged via [#81](https://github.com/tjmisko/Lysilogy/pull/81). | See issue / phase notes |
| [#83](https://github.com/tjmisko/Lysilogy/issues/83) fix: release provider request locks at lease end | closed | Merged via [#84](https://github.com/tjmisko/Lysilogy/pull/84). | See issue / phase notes |
| [#85](https://github.com/tjmisko/Lysilogy/issues/85) fix: support explicitly configured proxy transport for corpus builds | closed | Merged via [#86](https://github.com/tjmisko/Lysilogy/pull/86). | See issue / phase notes |

## Epic trackers

All nine epics remain open. They describe groups of work and are not nine additional implemented features.

| Epic | Title | State |
| --- | --- | --- |
| [#11](https://github.com/tjmisko/Lysilogy/issues/11) | [E0] Epic: Library at scale | open |
| [#12](https://github.com/tjmisko/Lysilogy/issues/12) | [E1] Epic: Paper objects | open |
| [#13](https://github.com/tjmisko/Lysilogy/issues/13) | [E2] Epic: Knowledge base core | open |
| [#14](https://github.com/tjmisko/Lysilogy/issues/14) | [E3] Epic: Acquisition | open |
| [#15](https://github.com/tjmisko/Lysilogy/issues/15) | [E4] Epic: Citations | open |
| [#16](https://github.com/tjmisko/Lysilogy/issues/16) | [E5] Epic: Reading lists | open |
| [#17](https://github.com/tjmisko/Lysilogy/issues/17) | [E6] Epic: Graph | open |
| [#18](https://github.com/tjmisko/Lysilogy/issues/18) | [E7] Epic: Integrations | open |
| [#67](https://github.com/tjmisko/Lysilogy/issues/67) | [E8] Epic: Evaluation | open |

## Earlier reader issues

These predate the KB buildout. Their GitHub state is reported without claiming that this handoff revalidated their implementation or closed their remaining criteria.

| Issue | Title | State |
| --- | --- | --- |
| [#1](https://github.com/tjmisko/Lysilogy/issues/1) | Product Name: Lysilogy | closed |
| [#5](https://github.com/tjmisko/Lysilogy/issues/5) | Supercut | closed |
| [#6](https://github.com/tjmisko/Lysilogy/issues/6) | Reference Tools | closed |
| [#8](https://github.com/tjmisko/Lysilogy/issues/8) | Feature: compact reader navigation with wrapping citations and sidebar branding | open |
| [#9](https://github.com/tjmisko/Lysilogy/issues/9) | Feature: PDF regex search, Vim text objects, and OCR-backed source indexing | open |

## Complete PR ledger

Merge hashes are GitHub merge commits, not reviewed branch heads. Rows before #66 are pre-existing reader work. #66 established the KB plan. This handoff’s own PR was not yet created at inventory time and is recorded in the final session checkpoint.

| PR | Title | State | Head branch → base | Merge commit |
| --- | --- | --- | --- | --- |
| [#2](https://github.com/tjmisko/Lysilogy/pull/2) | Stage paper reading from sourced abstract to text | merged | `codex/topbar-reading-views` → `main` | `440a9a50b29e651d0b0d98ee36cca12d5102ff98` |
| [#3](https://github.com/tjmisko/Lysilogy/pull/3) | Make Overview an all-page segmentation grid | merged | `codex/atlas-page-grid` → `main` | `075815e68ca38e92c4704a5e264c279f6495b452` |
| [#4](https://github.com/tjmisko/Lysilogy/pull/4) | refactor: rename Lysilogos to Lysilogy | merged | `refactor/rename-lysilogy` → `main` | `34fb241b6ad3554c88df13cdbffe1760147b5cc1` |
| [#7](https://github.com/tjmisko/Lysilogy/pull/7) | Add Lysilogos Supercuts and reference tools | merged | `feat/lysilogos-supercut-references` → `main` | `75c31a77198e7f0779ec8e9b53099baedddb1f60` |
| [#10](https://github.com/tjmisko/Lysilogy/pull/10) | Add Vimium-style PDF hints for citations, figures, and tables | merged | `feat/paper-link-hints` → `main` | `ade148625f91f07d00d951a5522f7c29b7848ca3` |
| [#66](https://github.com/tjmisko/Lysilogy/pull/66) | docs: plan paper objects, cross-paper knowledge base, and reading lists | merged | `docs/knowledge-base-plan` → `main` | `d2dd6eb095d16fb9d3a72b9fedff8b156ca01031` |
| [#74](https://github.com/tjmisko/Lysilogy/pull/74) | feat: define knowledge base domain types (E2.2) | merged | `feat/e2.2-kb-types` → `main` | `4ff924b2267358bf8e0a1134c2a3c6898d6a7162` |
| [#75](https://github.com/tjmisko/Lysilogy/pull/75) | feat: add reproducible arxiv research corpus tooling (E8.2) | merged | `feat/e8.2-arxiv-corpus` → `main` | `b8baa7b72d67406752886faf4f20d3e8adefd602` |
| [#76](https://github.com/tjmisko/Lysilogy/pull/76) | feat: add evaluation harness, scorecard, and ratchet (E8.1) | merged | `feat/e8.1-eval-harness` → `main` | `a04c0c032b60295da3d9877cfbbf29e487e8702b` |
| [#77](https://github.com/tjmisko/Lysilogy/pull/77) | feat: expose deterministic paper objects (E1.1) | merged | `feat/e1.1-paper-objects` → `main` | `86e7c75f571426dbd5c4a1dede2649c63043b388` |
| [#78](https://github.com/tjmisko/Lysilogy/pull/78) | feat: share provider cache and request budgets (E7.1) | merged | `feat/e7.1-provider-cache` → `main` | `2c21e7748e3358762f82efb16eb7bbd548b87177` |
| [#80](https://github.com/tjmisko/Lysilogy/pull/80) | feat: preserve paper identity across PDF moves (E0.2) | merged | `feat/e0.2-content-hash` → `main` | `94ddaccf9fa7d27db98e4d74a3bf26f64c6c7072` |
| [#81](https://github.com/tjmisko/Lysilogy/pull/81) | fix: allow configured linker in cold isolated tests (E8.1) | merged | `fix/e8.1-isolated-linker` → `main` | `1e9bc5e7e8e4f345073ddcaf1ffb4a07952c0099` |
| [#82](https://github.com/tjmisko/Lysilogy/pull/82) | feat: add isolated synthetic vault scale benchmarks (E0.1) | merged | `feat/e0.1-scale-bench` → `main` | `2683dceb84a958eebe93b5c43196bb8c80c4019f` |
| [#84](https://github.com/tjmisko/Lysilogy/pull/84) | fix: release provider connection locks at lease end (E7.1) | merged | `fix/e7.1-provider-lease-unlock` → `main` | `481e911a6845a0d91a117168757e35fe61739601` |
| [#86](https://github.com/tjmisko/Lysilogy/pull/86) | fix: support explicit corpus proxy transport (E8.2) | merged | `fix/e8.2-corpus-proxy-transport` → `main` | `e12adc3f0537cf0b3909f7f97a14f97b00599fe5` |
| [#87](https://github.com/tjmisko/Lysilogy/pull/87) | feat: parse person names and candidate keys (E2.3) | merged | `feat/e2.3-names` → `main` | `e06694684a985b19b9889e66f9bf03b8a33eb7b7` |
| [#89](https://github.com/tjmisko/Lysilogy/pull/89) | feat: normalize titles and score trigram candidates (E2.4) | merged | `feat/e2.4-titles` → `main` | `0987241e9eb669ccb2def8a9b5e4bbc0bb3ef02a` |
| [#90](https://github.com/tjmisko/Lysilogy/pull/90) | fix: preserve corpus quotas across public PDF gaps (E8.2) | merged | `fix/e8.2-corpus-availability` → `main` | `e60acb9865b045c1414e535864f3483c9a489f1b` |
| [#92](https://github.com/tjmisko/Lysilogy/pull/92) | fix: use canonical arxiv source downloads (E8.2) | merged | `fix/e8.2-source-endpoint` → `main` | `ca7d8d75ebc8957cd800c1567e83cbfcdc81d727` |
| [#93](https://github.com/tjmisko/Lysilogy/pull/93) | feat: prepare frozen reference truth builders | open / draft | `feat/e8.4-reference-truth` → `main` | — |
| [#94](https://github.com/tjmisko/Lysilogy/pull/94) | feat: parse bibliography entries and backend reference links (E1.2) | open / draft | `feat/e1.2-bibliography` → `main` | — |
| [#95](https://github.com/tjmisko/Lysilogy/pull/95) | feat: build deposited-ORCID person truth tooling (E8.5) | open / draft | `feat/e8.5-person-labels` → `feat/e8.4-reference-truth` | — |
| [#99](https://github.com/tjmisko/Lysilogy/pull/99) | feat(eval): add independent K1 truth and a limited reviewed release | merged | `feat/e8.3-latex-truth` → `main` | `0dc68dc5ccf3ab699126e2ca6136b0413b79f1cb` |
| [#100](https://github.com/tjmisko/Lysilogy/pull/100) | fix: isolate native reading-index page failures (E0) | merged | `fix/e0-reading-index-page-failure` → `main` | `46f11e0357a4282e4a94c8dce17765272e5ae834` |
| [#102](https://github.com/tjmisko/Lysilogy/pull/102) | feat(eval): measure independent figure and table baselines (E1.1) | merged | `feat/e1.1-object-metrics` → `main` | `d074dae96d9158c75add4ba9cc6fb27be1adf5bb` |
| [#104](https://github.com/tjmisko/Lysilogy/pull/104) | fix: recover tables and improve measured visual regions (E1.1) | merged | `fix/e1.1-figure-detection` → `main` | `a54de426d6691c013e5ea4b3333e7027d7103335` |
| [#108](https://github.com/tjmisko/Lysilogy/pull/108) | feat(eval): publish a reviewed three-paper K1 truth version (E8.3) | merged | `feat/e8.3-k1-coverage` → `main` | `7ef8fda06b43ec0e4485fcceb3f19ad9487b20f7` |
| [#112](https://github.com/tjmisko/Lysilogy/pull/112) | fix: recover expanded K1 figure and table regions (E1.1) | merged | `fix/e1.1-expanded-k1-regions` → `main` | `e1fe6a38925c90de5189462ead96725439d69f73` |
| [#113](https://github.com/tjmisko/Lysilogy/pull/113) | fix: preserve objects with ambiguous source labels (E8.3) | merged | `fix/e8.3-duplicate-labels` → `main` | `ca1f2c9756f4aa25682ccbe0ca076fbe536a0052` |
| [#114](https://github.com/tjmisko/Lysilogy/pull/114) | fix(objects): derive masked figure regions from opacity (E1.1) | merged | `fix/e1.1-mask-regions` → `main` | `d7d25c39d2c2ec87ea7234068b58e60c4eff5c3b` |
| [#116](https://github.com/tjmisko/Lysilogy/pull/116) | feat: add confined source-layout reconstruction experiment (E8.3) | merged | `feat/e8.3-source-layout-probe` → `main` | `230bf8182881d5cb923785e3896de05f46e3389b` |
| [#119](https://github.com/tjmisko/Lysilogy/pull/119) | fix: preserve complete BibTeX source fields (E8.3) | merged | `fix/e8.3-bibtex-fields` → `main` | `722b6b24aa57fc454cd9f4fb54b14071dfcc235e` |
| [#120](https://github.com/tjmisko/Lysilogy/pull/120) | fix: retain usable native output with bounded diagnostics (E0) | merged | `fix/e0-native-diagnostics` → `main` | `d88f6f0f925a8d410169ef5eb61a59df815d213d` |
| [#122](https://github.com/tjmisko/Lysilogy/pull/122) | feat: publish the reviewed limited visual tranche (E8.3) | merged | `feat/e8.3-visual-tranche` → `main` | `e0d3df18b6d6dc4f2b77c1facb16ff5e8a2a9d3e` |
| [#124](https://github.com/tjmisko/Lysilogy/pull/124) | fix: keep table cells out of native figure regions (E1.1) | merged | `fix/e1.1-visual-tranche-regions` → `main` | `339ded4f2e51d2696d79989a7230ec3898fa9582` |
| [#126](https://github.com/tjmisko/Lysilogy/pull/126) | feat: publish fixed visual-only pilot cohorts (E8.3) | merged | `feat/e8.3-visual-only-pilot` → `main` | `2e02c38a0696b4bc5e59ccc2b31b8ebae5c3b1f4` |
| [#127](https://github.com/tjmisko/Lysilogy/pull/127) | fix: recover complete split table captions (E1.1) | merged | `fix/e1.1-visual-only-captions` → `main` | `4a9c6842aa04c22b1533d8de145fff4b3bac3704` |
| [#128](https://github.com/tjmisko/Lysilogy/pull/128) | feat: recover bounded vector figure regions (E1.1) | merged | `fix/e1.1-vector-regions` → `main` | `ebe976339a5c93771375400bae5935cbf9ae1b3a` |
| [#130](https://github.com/tjmisko/Lysilogy/pull/130) | feat: add bounded per-paper K1 replay and collection (E8.3) | merged | `feat/e8.3-bounded-k1-replay` → `main` | `d9a62af3855c9bbf49dc8171cadf088bf644878d` |
| [#133](https://github.com/tjmisko/Lysilogy/pull/133) | feat(truth): publish two complete numbered-equation inventories (E8.3) | merged | `feat/e8.3-numbered-math-tranche` → `main` | `6059dcfc0ec7d7c13df3a535c4d96f1f63028da7` |

## Open follow-ups and uncovered work

- #97 owns the truth-coverage shortfall. #132, #134 and #135 are bounded contributions to it; none closes the approximately 500-paper requirement.
- #103 tracks the bibliography title line-break misses measured on the limited K1 cohort.
- The historical synthetic O25/O26 misses are assigned to the existing planned fixes #21/#22. A current production scale measurement is still unavailable.
- Most remaining objectives are unavailable because the required implementation/truth/collector does not yet exist. They are not measured failures. Final acceptance still requires measured misses to have explicit follow-up issues and recorded next attempts.
- The three architecture findings (SQLite startup propagation, durable canonical writes, global model limits) are recorded in the [implementation report](implementation-status.md). They have not all been filed as separate follow-up issues or fixed; do not infer issue creation from this report.

## Refresh commands

```sh
gh issue list --repo tjmisko/Lysilogy --state all --limit 300 --json number,title,state,url,body,closedAt
gh pr list --repo tjmisko/Lysilogy --state all --limit 300 --json number,title,state,isDraft,headRefName,baseRefName,url,mergeCommit
```

Read the actual issue body and latest phase notes before assigning work. The live [project](https://github.com/users/tjmisko/projects/12) and GitHub pages can change after this snapshot.
