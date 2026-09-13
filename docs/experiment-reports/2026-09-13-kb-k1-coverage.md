# K1 coverage experiments — issue #97

This is an implementation checkpoint, not a completed coverage release. The
approximately 500-paper target remains unmet. No new automatic truth is
published, and the independently reviewed `k1-limited-v1` payloads and build
configuration remain unchanged.

## Fixed comparison population

Every full comparison uses the same frozen 1,000 eval inputs and the original
999 successful native indexes, retaining the original failed index record.
Issue #98's later recovery is separate from this comparison. Inputs SHA-256 is
`2d4504df1208d27dc56ddae5549d1a49465d5db8fff06765215d0351a63cf8e0`;
index-map SHA-256 is
`40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`.
All corpus bytes and large receipts remain external to the repository.

| Automatic source checkpoint | Parsed | Source/index failures | Raw accepted | Defensible new admissions | Wall seconds | Peak RSS KiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Historical `0910c35` | 626 | 374 | 0 | 0 | 1,017.604 | 512,176 |
| Reviewed baseline `9d3bd42` | 622 | 378 | 0 | 0 | 992.856 | 470,192 |
| Exploratory `4ada537` | 617 | 383 | 1 | 0 | 997.138 | 499,824 |

The latest run used the reviewed source and frozen runner, with a 30-second
per-paper limit and 1.5 GiB memory cap. It made no network or model calls and
incurred no external cost. Both owner and independent root audits rehashed all
617 candidate payloads (119,831,173 bytes), validated their source inventories,
PDF/source hashes, PaperIds, index identities, strata and frozen order, and
reproduced the aggregate counts. Seven papers changed parse-success state from
the baseline. Unlaunched predecessor snapshots are retained separately.

The sole raw accepted candidate, `2007.05954`, was an invalid negative-cohort
claim. Its source contains a manually formatted References section and an
enumerated clustering procedure with nine literal `item` commands, of which
eight are active and one is commented out. These were absent from the parsed
bibliography and algorithm inventories. Source review therefore admitted no
new truth. The raw run and its receipts are preserved as a failed exploratory
attempt; its acceptance flags were not relabeled or published as valid metrics.

## Corrections and capability measurements

The reviewed guards now retain manual reference/procedure role evidence,
including descendant reference subsections and compact `Step1` labels, while
keeping complete unrelated kinds separate. Mathematical arrays and aligned
layouts, including clipped link contexts inside them, retain explicit rendering
fidelity exclusions. They cannot certify a flattened text match as exact
mathematical truth. All original source spans and omitted inventories remain
auditable.

The full-ledger capability analysis shows why a small symbol list is
insufficient. Of 617 parsed candidates, 616 have unknown command reasons, 363
have control flow and 316 have uninterpreted local styles. The following are
optimistic inventory-only ceilings, not predicted measurements or admissions:

| Hypothetical capability | Candidate ceiling before other fidelity/role checks | With a positive complete-kind alignment before source guards |
| --- | ---: | ---: |
| Add the diagnostic's finite standard atoms | 1, already the invalid negative candidate | 1 |
| Prove every unknown command, retain all other source/environment guards | 34 | 6 |
| Previous row plus the listed mathematical layout inventories | 59 | 12 |
| Prove every nonstyle source/environment issue, retain local-style exclusions | 301 | 61 |
| Remove all source/environment barriers, an unattainable assumption | 617 | 197 |

No such barriers were removed by this diagnostic. Reaching approximately 500
requires substantial source-program verification and/or recovering whole-paper
failures as well as improving faithful alignment. The most common whole-paper
failure was duplicated object labels (92 papers); unsafe ambiguity must remain
explicit if a later implementation localizes that failure.

A finite AMS math-operator declaration prototype investigates a source
capability affecting 184 frozen candidates. It checks the entire literal body,
preamble scope, load order and known name collisions. In a deterministic
source-only pilot of the first two affected candidates per category, checkpoint
`38f27a0` parsed all 14 papers and marked 18 declarations in eight papers as
verified under its partial proof. Every original object
ID, kind and source span remained identical. This took 2.034 seconds and
158,496 KiB peak RSS, with no PDF alignment, network or model calls. The sample
is targeted, not a representative coverage estimate. Independent review then
showed that this partial proof could miss imported argument consumers or
misread stored package tokens as executed loads. Checkpoint `8a8f58c` therefore
retains literal-definition evidence but withholds every unproved imported
namespace. The 18 historical flags are not defensible admissions. A short
reserved-name list or an incomplete regex catalog cannot supply the missing
proof. No generated toolchain probe has run. The prior `f389393` pilot
and its one heading-scan failure are retained; the correction restricts heading
roles to the document body while preserving unknown preamble semantics.

## Validation and remaining work

Source-capability validation passed 198 offline tests with Python warnings
treated as errors. Tests cover macro argument visibility/multiplicity, literal
and stored roles, original include spans, math fidelity, section hierarchy,
declaration scope/redefinitions, reserved environment names and valid controls.
Independent review has cleared the role and math-layout guards and the
conservative AMS imported-namespace correction at `8a8f58c`. Its 198 pinned
tests and 11 independent probes pass, including both original namespace/load
findings. Stored-load and imported argument-consumer regressions are included.

Main `21ef86b` was merged normally at `8f8105a`. The two collector/test conflicts
were resolved by keeping the immutable release replay and the separately
versioned current detector/native/graphics validation introduced by #101.
Focused checks passed nine verifier and 31 collector tests with warnings treated
as errors. All 74 pinned source-support, immutable truth and blind packet files
remained byte-identical. Scoring and graphics-validation functions are unchanged
from main; no production measurement, native build or pilot parser run occurred.

The clean before checks for the affected objects and bibliography suites passed
using an explicitly rooted, source-equivalent borrowed CLI, with all 72
Rust/Cargo/config inputs compared against `fd024fc`. The producer source and
binary hash are retained, along with the limitation that no standalone Cargo
JSON CLI build receipt existed. A later clean before checkpoint `7c6296e` built
and selected this worktree's CLI from Cargo artifact JSON, then passed both
affected suites in 96.978 seconds (1,015,088 KiB peak child RSS). Its binary
SHA-256 is `31d3f1257fc8b1da24c94425d02f6e16f1954b60f2f2557507d2cefa47457753`.

The mixed-overlay/native-export adapter and exact JSON type correction were
independently cleared at `3154815`. All 212 LaTeX tooling tests pass. Formatting,
strict Clippy, all Rust targets, the offline O30 collector, and objects,
bibliography and scale checks passed. G5 subsequently passed **350 Rust / 353
Python / 85 Node tests**. The initial G5 attempt exposed missing TypeScript in
the fresh worktree; its failure remains retained. Offline npm setup encountered
a read-only default cache and then a sandbox-blocked registry request. The exact
locked TypeScript 5.9.3 tarball was instead verified from the local content cache
and unpacked through bounded regular-file paths without scripts or networking.
Only G5 was rerun after that environment repair. The initial gate attempt took
57.660 seconds (1,234,368 KiB peak child RSS); the successful G5 retry and immutable
truth replay took 11.554 seconds (156,048 KiB). The replay itself took 4.550 seconds
and reproduced both historical payload hashes exactly. The selected CLI binary
remained identical to the before build.

These checks retain O30 = 0/10,000. Objects and bibliography have unchanged
unavailable collector inputs in this worktree; that is not a new performance
measurement or regression. No expanded truth release or completion of issue #97
is claimed.

Further namespace probing is deferred because no material coverage payoff has
been established. The next concrete step is independent annotation of a frozen
seven-paper pilot, one paper per category, within a planned 21-paper tranche.
Issue #97 remains open and the approximately 500-paper target is unchanged.
Another full automatic comparison requires a material, independently reviewed
capability change and the shared execution window.

## Availability-selected annotation pilot

The policy was frozen before selection and independently reviewed. It takes the
first three new available papers per category in the original frozen 1,000-input
array order, with an eight-page cap; the first one per category forms the pilot.
Availability means matching actual source/PDF/original-index bytes and safely
readable TeX source. Parser success, source object counts, detector outputs and
automatic alignment outcomes are absent from the selection criteria. A selected
paper with parsing, ambiguity or annotation problems remains selected.

The selector filled all 21 slots (128 pages) in 9.436 seconds, with 187,936 KiB
peak RSS and no external calls. Its full ledger retains 214 page-cap exclusions,
one unavailable original index, the two already published papers, 21 selections
and 762 later rows whose category quota was already filled. The first seven are:

| Paper | Frozen category/year | Pages |
| --- | --- | ---: |
| 2210.11141v1 | cs.CV / 2022 | 5 |
| 2310.04162v1 | cs.RO / 2023 | 8 |
| 2001.05217v1 | hep-th / 2020 | 7 |
| 2409.03655v1 | cs.LG / 2024 | 6 |
| 2002.03492v1 | econ.TH / 2020 | 5 |
| 2207.03024v1 | stat.ML / 2022 | 5 |
| 2303.07834v2 | math.PR / 2023 | 7 |

Independent review reproduced all 1,000 ledger rows, all 21 selected bindings
and every availability exclusion. Prepared blind packets bind all source members
and actual PDF/index hashes.
Their native export contains only text, pages and word rectangles; embedded
historical detector figures/objects and automatic outcomes are omitted. Primary
and independent annotators have separate output directories and first enumerate
the complete paper without a parser inventory. Independent export verification
confirmed the allowlists against the original bytes, and rendering produced all
43 original pages at 96 dpi in 2.851 seconds with 72,464 KiB peak child RSS.
The first paper was assigned to both reviewers; its labels were read by the
implementation agent only after both inventories froze. Existing reconciled annotation
contracts remain the publication boundary. This pilot will measure annotation
throughput, complete per-kind coverage and demonstrated tooling gaps before
committing to the remaining 14 papers. Selection and packet preparation alone
admit no truth, and the page cap introduces an explicit short-paper bias.

A read-only adapter assessment identified combined visual/math bundle
composition, typed reference roles and blinded native-export provenance as
the first gaps. After both annotations froze, the reviewed adapter permits
separately validated overlays against the same unchanged candidate, retains
figure/table references separately from the O4 denominator, and rehashes the
exact allowlisted native export actually shown to annotators. It rejects changed
candidate fields, conflicting object identities or negative claims, and resealed
export changes including JSON boolean/number substitutions. Existing complete
math-kind and source-provenance checks remain in force.

For `2210.11141v1`, the two frozen annotations and root reconciliation agree on
12 source object occurrences (six figures, two tables, four equations), 25
bibliography entries, 49 citation groups/75 pairs, and 20 reference occurrences.
Seventeen independent field memberships were corrected in a new version to
restore combining marks and hyphens; all 75 field values and source roles stayed
unchanged. Original and corrected receipt chains remain separate. The original
issue scope excludes seven retained informal problem/procedure candidates from
formal statement, proof and algorithm inventories; no automatic guard changed.

A source-only parse after annotation freezing reproduces all 12 object
occurrences, all 25 entry identities and all 69 citation/reference source links.
Entry source boundaries differ only by retained trailing parser whitespace.
Sixteen references target objects: three equation references belong to O4 and
13 visual references remain separately typed. The parse took 0.075 seconds and
41,392 KiB RSS, with no alignment, detector reads or external calls. Automatic
source and rendering exclusions remain recorded. The independent supplement now
supplies all four equation body and number boxes; root review verified all eight
coordinate pairs, complete mathematical glyphs, source identity and retained
native losses. An explicit post-freeze construction-packet reconciliation and
actual validation remain required. The original
blind packet never contained the later parsed inventory; the adapter does not
claim that it did. Bibliography's independently read plain field roles also need
their own validated path rather than fabricated structured commands. This
checkpoint admits no new truth.

The bounded first-paper candidate construction took 0.688 seconds and 52,448 KiB
RSS. It reproduces the exact source-only inventory and keeps automatic acceptance
false. A real mixed-bundle attempt with byte-identical frozen annotations fails
at the legacy annotation-time `inputs` boundary; the traceback and diagnostic
input copies are retained. Those annotations cannot honestly claim they read a
candidate or parsed inventory before it existed. A proposed separate versioned
manual-tranche path would bind their actual blind inputs and the later exact
crosswalk, preserve the old validators and all automatic exclusions, and derive
only independently complete visual/math metric cohorts. Independent proposal
review approved that separate evidence format. A further post-freeze audit
rehashed 28 artifacts and independently reproduced all 12 object, 25 entry and
69 link correspondences, retaining every automatic exclusion and native loss.
That audit establishes the crosswalk, not bibliography field-role truth.

Source checkpoint `89aedba` implements the explicit `k1-manual-tranche-v1`
validator. Its first reusable codec supports complete direct figure/table/equation
inventories and separately reviewed formal negatives; positive formal or nested
objects fail until a suitable ownership codec exists. It validates the original
input and correction histories, complete source/native occurrence bijections,
mathematical and visual regions, informal role dispositions and all reference
roles. Bibliography and O8–O11 remain omitted. The actual manifest template binds
21 evidence documents and deliberately points to a pending, invalid final review;
it cannot admit a paper. All 230 offline tests pass in 0.536 seconds with 34,624 KiB
peak child RSS and zero external calls. Subsequent review corrections preserve
appended field-audit limitations and reviewed contained role passages, rehash
declared correction audit inputs, and pin the explicit manifest in release
evidence. All 233 current offline tests pass. The frozen templates remain unchanged;
their prose artifact count of 22 was a documentation error, not an omitted input.

Root source and construction review cleared `8c8fd16`, and a separate accepted
review/manifest binds the original 21-artifact closure. Actual full assembly
succeeded in 0.298 seconds with 74,816 KiB peak child RSS and no external calls.
The unpublished candidate contains eight complete visual objects, four equations
and 20 references (16 object references, including three O4 pairs), with reviewed
formal negative inventories. Automatic acceptance remains false and all source
diagnostics remain unchanged. Bibliography and O8–O11 are omitted. Current-source
full gates and separate final agent review remain pending; earlier full gates
apply only to `3154815`. No new truth release or detector measurement has run.

## Retained evidence

The machine-readable companion is
[`eval/evidence/k1-coverage-progress.json`](../../eval/evidence/k1-coverage-progress.json).
It binds the raw comparison, independent audits, source-role probes, capability
bounds and targeted pilots to their external paths and SHA-256 hashes. The
immutable limited release has a version-selected replay verifier pinned to its
original reviewed source bytes; changes to the current parser cannot silently
change historical truth or select another release implementation.
