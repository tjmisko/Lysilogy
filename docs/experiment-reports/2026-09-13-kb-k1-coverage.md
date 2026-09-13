# Reviewed manual K1 expansion — issues #105 and #97

`k1-limited-v2` publishes three independently reviewed papers: the two original
cohorts plus availability-selected `2210.11141`. It contains 23 annotated
figures/tables, five equations, four statements, one proof, two algorithms and
35 bibliography entries. The original v1 payloads, configuration and retained
verifier modules remain byte-identical. This bounded release addresses #105;
#97's approximately 500-paper target remains open and unmet. No automatic paper
is published, and no system acceptance is claimed.

## Fixed comparison population

Every full comparison uses the same frozen 1,000 eval inputs and the original
999 successful native indexes, retaining the original failed index record.
Issue #98's later recovery is separate from this comparison. Inputs SHA-256 is
`2d4504df1208d27dc56ddae5549d1a49465d5db8fff06765215d0351a63cf8e0`;
index-map SHA-256 is
`40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`.
All corpus bytes and large receipts remain external to the repository.

| Automatic source checkpoint | Parsed | Source/index failures | Raw accepted | Defensible automatic candidates | Wall seconds | Peak RSS KiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Historical `0910c35` | 626 | 374 | 0 | 0 | 1,017.604 | 512,176 |
| Reviewed baseline `9d3bd42` | 622 | 378 | 0 | 0 | 992.856 | 470,192 |
| Exploratory `4ada537` | 617 | 383 | 1 | 0 | 997.138 | 499,824 |
| Current `542da50` | 612 | 388 | 1 | 1, empty O5/O6 only | 1,032.039 | 453,392 |

The exploratory `4ada537` run used the reviewed source and frozen runner, with a 30-second
per-paper limit and 1.5 GiB memory cap. It made no network or model calls and
incurred no external cost. Both owner and independent root audits rehashed all
617 candidate payloads (119,831,173 bytes), validated their source inventories,
PDF/source hashes, PaperIds, index identities, strata and frozen order, and
reproduced the aggregate counts. Seven papers changed parse-success state from
the baseline. Unlaunched predecessor snapshots are retained separately.

That exploratory run's sole raw accepted candidate, `2007.05954`, was an invalid negative-cohort
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
diagnostics remain unchanged. Bibliography and O8–O11 are omitted.

After a normal docs-only main merge, all current-source gates passed at
`542da50`: formatting, strict all-target/all-feature Clippy, all Rust targets,
233 targeted Python tests, the affected objects/bibliography and scale checks,
G5, and an actual immutable-v1 replay. G5 ran 350 Rust, 374 Python and 85 Node
tests. The complete sequence took 15.402 seconds with 157,056 KiB peak child RSS;
all 160 retained source inputs stayed unchanged. Its 54 raw artifacts are copied
and SHA-verified in the external archive. The generated scorecard was the sole
dirty file during evaluation and was retained before restoration. Objects and
bibliography remained unavailable on this branch's empty collector inputs;
these checks do not remeasure the previously published limited truth. O30
remains zero violations across 10,000 fixture references. The existing v1 object
and bibliography payloads reproduced exactly. At that historical checkpoint,
separate review and the current full comparison still blocked publication; the
external configuration was deliberately incomplete. Those templates and gate
receipts remain immutable historical evidence.


Independent review of the new validator found page ownership and repeated-reference
membership gaps. Corrected source `1b41313` passes 235 offline tests and the
independent malformed-page, reused-occurrence and out-of-phrase probes. Actual
CLI assembly reproduces the same labels and exclusions in 0.310 seconds with
72,464 KiB peak child RSS. Exact canonical JSON comparison confirms that only the
producer implementation receipt changed.

The fresh frozen comparison at `542da50` completed all 1,000 original inputs using
the original 999-success index map: 612 parsed and 388 source/index failures,
1,032.039 seconds, 453,392 KiB peak RSS, no network/model calls or external cost.
Its one raw accepted paper, `2007.05954`, has only complete empty O5/O6 inventories;
all other metric cohorts are empty. Independent inspection of all 18 original
PDF pages and source/native evidence confirms only those formal negatives. It
supplies no automatic paper to the new release and no positive-kind automatic
admission. The raw report remains unchanged; the accepted negative disposition
is a separate review receipt.
The owner and root independently rehashed all 612 candidate/inventory pairs
(121,450,396 bytes), exact input order, mapped identities and reported counters.
The later manual-only corrections do not change the six-module automatic
execution closure; the report retains its actual `542da50` source identity.

Version-aware replay and collector dispatch at `8987d13` pass 238 truth and 35
collector tests. Actual v1 replay still reproduces both immutable payloads.
V2 now has its own pinned fourteen-module retained inventory and exact reviewed
manifest/configuration/output hashes. Both isolated version replays succeed. A review found that separate immediate
harness inputs would give O1/O2 duplicate owners; the correction keeps one active
explicitly selected cohort and freezes exact prior/new input and observation
bytes outside the active directory. Matching, geometry and score arithmetic
remain unchanged. Independent publication review verified the exact three-paper
configuration, both output previews, all retained modules and accepted source
correspondences. Actual immutable publication at `59742fd` matched both preview
hashes; old and new isolated replays reproduce both object/bibliography payloads.
V1 replay took 4.485 seconds; v2 took 4.683 seconds. Issue #105 owns this bounded
improvement. Issue #97 remains open; three papers do not meet its target.

## Production measurement on the frozen expanded cohort

| Cohort | Papers | Annotated regions | TP / FP / FN | O1 F1 | O2 median IoU |
| --- | ---: | ---: | --- | ---: | ---: |
| Previous v1 subset, reproduced within v2 | 2 | 15 | 15 / 0 / 0 | 1.000000 | 0.910779 |
| Expanded v2 | 3 | 23 | 23 / 1 / 0 | 0.978723 | 0.757008 |

Current production collection at `59742fd` took 13.006 seconds (13.080 seconds
including its wrapper), with 98,048 KiB peak child RSS and zero model/network
calls or external cost. All 23 independently annotated regions remain in O2,
including zeros; unknown regions are zero. Both old per-paper metric records
match the committed v1 observations exactly under canonical JSON. The new paper
contributes eight correct caption matches and one extra Figure 4 candidate from
a body paragraph. Six matched regions are null, Table 1's proposed body does not
overlap its independently reviewed body, and Figure 1 has IoU 0.757008.

Unadjusted `eval objects --check` correctly reports regression against the smaller
v1 cohort's ratchet values; the raw failed result is retained. The independently reviewed
explicit cohort-change justification binds the exact old/new comparison, keeps
the old records unchanged, and changes no scoring rule or objective target.
The justified objects check passes and records the new cohort baselines, retaining
the old values and exact evidence in the adjustment history. Both aggregate targets
remain met on this limited cohort (O1 ≥0.90, O2 ≥0.75);
the individual misses are tracked by [#106](https://github.com/tjmisko/Lysilogy/issues/106)
and the broader coverage gap remains open under #97. Bibliography
checks remain unavailable on this branch's collector input; v2 preserves the
original 35 independently reviewed entries and adds no new bibliography cohort.
Prior annotation/panel agent dollar costs are unavailable and remain unknown;
the measured runtime costs above apply only to the recorded offline executions.

The integrated gate attempt at `6eea188` ran all twelve commands successfully,
including G5's 350 Rust, 383 Python and 85 Node tests and both version replays.
Its wrapper nevertheless failed its final byte-idempotence assertion: the existing
JSON baseline read/write changed O1 and its recorded adjustment from
`0.9787234042553191` to the adjacent float `0.9787234042553192`. The actual measured
score remains the former. The failed receipt, all raw logs and exact before/after
bytes are retained. Two repeated own-CLI checks pass and leave the latter bytes
unchanged. This stable serialization is committed without changing a target,
measurement, justification or assertion; the same complete gate runner must pass
again before final clearance. The unchanged runner then passed at `4f3a289`
in 20.159 seconds with 150,784 KiB peak child RSS: all twelve commands, G5
(350 Rust / 383 Python / 85 Node), and both immutable replays pass. All 185
recorded source inputs stayed byte-identical throughout the run; the G5 result
records a clean worktree. The generated scorecard is unchanged at 1/5 hard gates
and 3/30 objectives met on the explicitly limited cohort. Both successful and
failed raw gate/result logs are SHA-verified in the external archive. Final PR
review remains separate from these executed checks.

## Retained evidence

The machine-readable companion is
[`eval/evidence/k1-coverage-progress.json`](../../eval/evidence/k1-coverage-progress.json).
It binds the raw comparison, independent audits, source-role probes, capability
bounds and targeted pilots to their external paths and SHA-256 hashes. The
immutable limited release has a version-selected replay verifier pinned to its
original reviewed source bytes; changes to the current parser cannot silently
change historical truth or select another release implementation.
