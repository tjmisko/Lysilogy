# E1.2 backend bibliography parsing and reference links

Bibliography entries now come from the deterministic backend and are persisted as schema-2
`bib_entry` objects. The reader consumes their reference links only when the paper ID and exact
reading-index generation agree. The original implementation, collector, measured parser corrections and final structured-DOI
boundary fix have passed separate independent source reviews. **The PR remains draft: genuine
K2 deposited-reference measurements are still required for O9 before this detector merges.**

## Current integration with figure and graphics derivation

Source `347de697a87c72ea1d6d3ff41ee0eff66f2e78cb` normally merges main through `d51ff95`,
including #101/PR104. Schema 2 retains bibliography fields, unresolved citations and sentence
anchors inside the common object assembly. The asynchronous source-backed API/cache and the
native-only fixture factory share that assembly; current figure geometry, graphics generation,
source/tool fingerprints and the compact native commitment remain intact. Two new offline tests
cover bibliography stability under changed figure regions and source-aware cache reuse/schema-1
upgrade, without invoking native commands or changing an index. Separate root source and compiled
reviews cleared the integration before real remeasurement.

The same frozen K1 bibliography still gives **O8=1.0 (35/0/0)** and **O10 precision/recall=1.0
(50/0/0)**. Known K1 fields remain **title 30/35, first author 35/35, year 35/35**; genuine K2
is absent, so complete O9 remains unavailable. The merged graphics results are preserved exactly:
**O1=1.0 (15/0/0), O2=0.9107793204006069** over all 15 original regions. No truth, scoring rule,
canonical PDF/source/index or registry changed. The original bibliography observations and all
historical parser/figure baselines below remain immutable.

All integration gates pass at that source: formatting, strict all-target/all-feature Clippy,
**388 Rust tests**, 14 bibliography-collector tests, 28 figure-collector tests, frontend
typecheck/lint/build, 20 paper-link tests and the production-artifact Playwright smoke. Both new
screenshots were inspected. Isolated G5 passes **388 Rust / 296 Python / 92 Node**, with loopback
only, external networking rejected and model CLIs absent. Bibliography, objects, scale and tests
`--check` pass; O30 remains 0/10,000 simulated admissions. The local scorecard is **1/5 hard gates
and 5/30 objectives at target** (O1, O2, O8, O10, O30), not phase/system acceptance.

Targeted tests/Clippy took 29.928 s, actual bibliography 0.823 s, actual source-backed objects
7.764 s, and the final Rust/frontend/eval sequence 37.451 s (isolated G5 7.747 s). These are
separate scopes, with zero network/model calls and $0. Historical scale timings were not repeated.
[Integration evidence](../../eval/evidence/bibliography-graphics-integration.json) binds 193 source
fingerprints, exact new observations/results, 71 retained artifacts, executable bytes and review
receipts under `~/.cache/lysilogy/bibliography-development/integration-release-347de69/`.
The draft still waits for genuine K2/O9. #103 retains the five title misses; #97 retains broader
independent coverage. The earlier #101 aggregate objective misses are resolved by its merged fix.

## Real K1 measurements and corrections

The unchanged limited K1 release now supplies one independently annotated, complete bibliography
from arXiv **2503.05828v1**: **35 entries**, **105 known fields**, and **40 citation groups / 50
occurrence-target pairs**. Its bibliography truth SHA-256 is
`a821d3070768175f2de7022928a6a19f6ef8cddce9edef726448447e24fc7c82`;
the exact frozen reading-index SHA-256 is
`da7f49d6a539b86a4aba82b3c85c6078d4eb2166fc01a577b55b5817185728e2`.
No PDF, source archive, reading index, label, matching rule, denominator or target changed.
This small, manually selected cohort measures current behavior; it does not establish broad
corpus generalization. Coverage expansion remains tracked by #97.

| Measure | First real run `779f0c2` | Line correction `aae7369` | Marker correction `d497304` | Final parser `2608bc8` |
| --- | --- | --- | --- | --- |
| O8 exact entry F1 | 0.289855 (10 TP / 24 FP / 25 FN) | 0.916667 (33 / 4 / 2) | 1.0 (35 / 0 / 0) | 1.0 (35 / 0 / 0) |
| O10 precision / recall | 0.261905 / 0.22 (11 TP / 31 FP / 39 FN) | 0.738462 / 0.96 (48 / 17 / 2) | 1.0 / 1.0 (50 / 0 / 0) | 1.0 / 1.0 (50 / 0 / 0) |
| Known K1 title accuracy | 7/35 | 27/35 | 29/35 | 30/35 |
| Known K1 first-author / year accuracy | 10/35 / 10/35 | 33/35 / 33/35 | 35/35 / 35/35 | 35/35 / 35/35 |
| Complete O9 | unavailable | unavailable | unavailable | unavailable: genuine K2 absent |

The first actual baseline is immutable under
`~/.cache/lysilogy/bibliography-k1-first-measurement/`. Every later measured checkpoint and
failed gate attempt remains under `~/.cache/lysilogy/bibliography-development/`, with source,
truth, index, Cargo-selected executable, command and observation fingerprints. The final
bibliography evaluation at `2608bc8` retains the first real O8/O10 values in `baseline_before`;
the evaluation harness advances its rolling baseline after each run. Earlier pre-truth receipts
below remain historical evidence.

The corrections address general representation failures. Bibliography text classified as a
heading is retained within the bibliography region; captions remain excluded. Printed keys at
physical line or page starts split entries even when the reading index joins those lines with
spaces. The additional boundaries use token geometry and exact byte-to-UTF-16 conversion,
with raised inline tokens, missing or invalid rectangles, and astral text covered by tests.
Wrapped author/year/title members remain attached to their numbered entry. Explicit
surname/initial/year entries following a complete numbered entry preserve mixed conventions.
Dotted publication-year continuations are not assumed to be new keys; an independently present
adjacent dotted-key sequence, including lookahead at the first entry, can justify four-digit
printed keys. Bracketed four-digit keys remain explicit.

A square-bracket citation convention provides evidence against interpreting an unrelated bare
superscript or parenthesized number as another citation. Such candidates now remain unlinked
`ambiguous_marker` observations with exact occurrences and candidate IDs unless a connected known
citation or explicit bibliographic cue supports the mixed form. “See [1] and (2)” is supported;
“See (2)” and “[1] confirms (2)” alone remain ambiguous. Tests retain pure superscript support
and cover equation/list numbers, footnotes, mathematical indices, and frontend withholding.
Question/exclamation punctuation now terminates a title while remaining part of that title.

The **five remaining known K1 title misses** are line-break hyphens: `Feed- back`, `Substi- tutes`,
`Intel- ligent`, `Re- inforcement`, and `Compet- itive`. Their measured 30/35 title accuracy is
0.857143 versus the planned 0.90 component target. [Follow-up #103](https://github.com/tjmisko/Lysilogy/issues/103)
records the exact independent source IDs and next idea: retain physical break evidence and
conservative spelling alternatives while preserving real compounds and mathematical minus
signs. Ambiguous printed hyphens have not been silently removed. This remains a K1 diagnostic;
no full O9 value is claimed without deposited K2 data.

Full checks at `2608bc8` pass: formatting, strict all-target/all-feature Clippy, **355 Rust tests**,
frontend typecheck/lint/build, **20 paper-link tests**, and the Playwright reader smoke using the
production Rust artifact. G5 passes **355 Rust / 292 Python / 92 Node**, with external networking
disabled and model CLIs absent. Eleven new Rust tests and one new frontend test extend the original
bibliography checks. Both retained screenshots were inspected. The full gate sequence took
**69.149 seconds**; the real bibliography collector took **0.817 seconds**, the integrated figure
collector **5.288 seconds**, and isolated G5 **7.994 seconds**. Network/model calls: **0**; cost: **$0**.

The figure collector was refreshed against unchanged K1 after integrating #96: O1 remains
0.769231 (10 TP / 1 FP / 5 FN), O2 remains 0.244851 over all 15 independently annotated objects.
Those historical misses were tracked by #101 and are resolved in the current integration above.
O30 remained zero violations over 10,000 simulated admissions.
Bibliography, objects, scale and tests `--check` pass with honest unavailable metrics; the local
scorecard at that checkpoint was **1/5 hard gates passing and 3/30 objectives at target** (O8, O10, O30).
Historical O25/O26 misses remain with #21/#22. No native extraction or expensive scale timing was
repeated for this parser.

The [K1 iteration evidence](../../eval/evidence/bibliography-k1-iterations.json) and
[derived final bibliography observations](../../eval/evidence/bibliography-k1-observations.json)
retain inspectable source matching and field disagreements. Main's docs-only checkpoint `9dc3f27`
was normally merged as `e8601d0`; all 189 compiled, collector and frontend source fingerprints
remain identical to `2608bc8`. All required warm integration checks also pass with the same metrics and G5 counts; that
sequence took **40.332 seconds**, with **0.537 seconds** for bibliography, **4.863 seconds** for
figures, and **7.896 seconds** inside isolated G5. Both screenshots are byte-identical to the
inspected prior run. Its 50 retained artifacts include 16 gate logs, 15 isolated G5 logs, exact
results, source/executable bindings, and screenshots.

The final provenance correction is `e3539d2`. Real field diagnostics showed that
`10.48550/arXiv.2102. 04906` (or a newline at that point) could become the explicit but incomplete
`10.48550/arXiv.2102`. The structured `10.48550/arxiv.` namespace now requires a complete arXiv
suffix under the existing local grammar; incomplete suffixes stay missing while raw entry text
and citation anchors remain intact. Complete modern, versioned and legacy suffixes and unrelated
generic DOI prefixes have positive regressions. Seven fixture cases and nine independent
production probes verify this boundary without provider calls. This guard withholds evidence;
it does not join uncertain fragments or infer a provider identity.

All required checks were refreshed at that exact source: **356 Rust / 292 Python / 92 Node**
under G5, **35 bibliography tests**, 20 paper-link tests, full frontend gates and the same
Playwright smoke. The two screenshots remain byte-identical to the inspected copies. The final
sequence took **69.996 seconds**; bibliography **0.808 seconds**, figures **5.604 seconds**,
and isolated G5 **7.888 seconds**, with zero network/model calls and $0 cost. O8/O10, all K1 field
denominators/diagnostics, O1/O2 and O30 remain unchanged. The current evidence contains this final
50-artifact receipt plus all earlier iterations and three separate independent review receipts.
The measured iteration adds **12 Rust tests and one frontend test** overall. Genuine K2/O9 still
blocks merging; the report does not claim system acceptance.

## Behavior and decisions

Each entry retains its raw text and disjoint source members, printed key, authors, title, year,
venue, DOI and arXiv ID. Field confidence is categorical (`explicit`, `heuristic`, `missing`),
not a calibrated probability. Printed author strings, year suffixes and arXiv versions survive;
conflicting or unsupported identifier evidence stays missing while its raw source remains.

Numeric lists/ranges, geometry-confirmed superscripts and author-year suffixes resolve locally
only when the destination is unique. Unresolved records retain the reason and candidates.
Ranges expand at most 30 steps. Each occurrence has exact UTF-16 offsets and token rectangles,
separate from its sentence context; repeated citations in a sentence and surrogate-pair boundaries
have regressions. Shared name parsing supplies a family key only when every interpretation agrees.
This change does not merge Works or Persons.

The frontend retries a mismatched index/object pair once and withholds stale reference links.
Figure/table and native-annotation behavior survives absent, failed or stale objects. A narrow
frontend bibliography-section mask remains solely to exclude figure/table text inside references;
entry splitting and citation resolution live in the backend. Appendix links remain active.
Native annotations preserve their existing precedence without collapsing grouped destinations.

Independent review corrected four parser cases: prose following a line-wrapped DOI, unnumbered
continuations, overlapping alphanumeric/author-year matches, and a block that finishes one entry
before starting another. A separate frontend regression restored bibliography masking during
object failure. Each correction has a targeted regression and the corrected source was cleared.

## Collector and provenance

The [O8–O10 collector contract](../../eval/bibliography-contract.md) consumes independent K1
LaTeX/PDF alignment and genuine K2 deposited reference labels. It runs the production Rust builder
against external reading-index caches. Source PDFs, LaTeX archives, full indexes and raw object
artifacts remain outside the repository.

O8 matches exact non-whitespace UTF-16 entry membership one-to-one. O9 includes every known field
on every truth entry, counting missing segmentation predictions wrong; unknown labels are excluded
with counts. O10 compares exact occurrence/target pairs and retains wrong, missed and duplicate
pairs. Disjoint page geometry for one occurrence is counted once. K2 structured-only references
are excluded from O9 to prevent labels from supplying their own input; case IDs and deposited
snapshot/JSON-pointer provenance are retained. Each O9 component requires known labels from both
real populations. The 14 synthetic collector tests verify arithmetic and provenance only.

The collector uses the executable reported by Cargo's JSON compiler-artifact output, rather than
assuming a target-directory location. Its executable SHA-256 is retained and verified before
publication. Review added a custom-target/stale-default regression and executable-replacement
rejection. The actual Cargo-selected executable and production field/title modes passed the final
integration check.

## Historical validation before K1 publication

Initial integrated checkpoint `b779eb1f750e0389fd55a53e8635dfab271e8975` normally merges main `cc329dd`, including
corpus fixes #88/#91. Its parser, frontend and collector source is unchanged from the independently
cleared `97067fc`. Formatting, strict all-target/all-feature Clippy, and all **329 Rust tests** pass.
The additions include 23 bibliography tests and a backend API evidence test. Frontend typecheck,
lint and production build pass, as do 19 paper-link, 9 API and 10 reading-index-cache tests.

The fixture-backed Playwright reader smoke passed using artifacts generated by the production
Rust builder; its HTTP transport is mocked. It exercises grouped references, native precedence,
stale and failed artifacts, cancellation, appendix destinations, crops, navigation history and
keyboard isolation. The healthy fixture exposes six reference, four figure, one table and two
additional native hints. The final screenshots were inspected and retained outside the worktree.
This is issue-level reader verification; final live-system and 10k arXiv verification remain ahead.

At that checkpoint, G5 passes **329 Rust, 124 Python and 90 Node tests** in an isolated network namespace with no model
CLIs. It took **7.650 seconds**; the complete final gate sequence took **64.534 seconds**. Model
calls: **0**. Cost: **$0**. Bibliography, objects, scale and tests evaluations all pass `--check`.
The original before/after JSON, 28 source fingerprints, command log hashes, G5 evidence/counts,
executable fingerprint and screenshot hashes are retained in
[bibliography-parser.json](../../eval/evidence/bibliography-parser.json). Raw logs and screenshots
are also retained under `~/.cache/lysilogy/bibliography-e1.2/`. Actual commit/dirty flags remain
intact; generated scorecard output accounts for dirty state during evaluation. No source changed
during the final gates.

The final fixture follow-up is `6291b7e5c64cdc4c87762ab2097f4e51499d449d`. Review found that the
browser test helper inherited Cargo directory/target settings while executing a fixed binary path.
It now owns the worktree target directory and selects Cargo's reported executable by exact example
name, kind and source path. Its regression models directory and native-target-triple overrides from
both environment and configuration, including stale native binaries and unrelated build artifacts.
Production Rust and the Python collector are unchanged from the earlier validated checkpoint.

The affected frontend typecheck/lint/build, focused fixture regression, production-artifact paper
links and Playwright smoke were refreshed successfully. Bibliography, objects, scale and tests
`--check` also pass after refreshing O30. Final G5 is **329 Rust / 124 Python / 91 Node**, taking
**7.386 seconds**; the affected-check sequence took **32.858 seconds**, with zero model calls and
$0 cost. [Supplemental evidence](../../eval/evidence/bibliography-fixture-target.json) retains the
final 29 source fingerprints, exact results/logs, G5 receipt/counts and screenshots, plus the
historical passing directory-only checkpoint. The earlier full Rust/Clippy receipts remain valid
for their unchanged source. This follow-up is independently cleared; the K1/K2 measurement hold
is unchanged.

| Measure | Before | After | Status |
| --- | --- | --- | --- |
| O8 segmentation F1 | unavailable | unavailable | K1 #70 required |
| O9 title / first author / year | unavailable | unavailable | genuine K1 and K2 #70/#71 required |
| O10 marker precision / recall | unavailable | unavailable | K1 #70 required |
| Object-suite metrics including G3 | unavailable | unavailable | owning truth/collectors pending |
| G5 offline tests | passing baseline | pass | no regression |
| O30 provider budget violations | 0 / 10,000 baseline | 0 / 10,000 fresh simulation | at target |

At that pre-truth checkpoint, the local scorecard was **1/5 hard gates passing and 1/30 objectives at target**. Unavailable
metrics do not count as passes, and no synthetic fixture has been published as K1/K2 or as an
O8–O10 measurement. No target or baseline was relaxed. The existing historical O25/O26 misses
remain tracked by #21/#22; their expensive synthetic 10k timing was not rerun or re-attested for
this parser change. Their absence in this worktree's fresh results is not an objective regression.

The pre-truth hold above is superseded by the real K1 results at the start of this report.
Next: retain the #25 draft while #71 builds genuine K2 deposited reference inputs, then measure
all O9 components, address remaining reviewed defects, and assess the existing merge gate.
The K1 title diagnostic miss now has follow-up #103; #97 owns broader independent coverage.
