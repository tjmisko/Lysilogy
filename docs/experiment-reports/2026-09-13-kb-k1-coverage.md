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

Current lightweight validation passes 198 offline tests with Python warnings
treated as errors. Tests cover macro argument visibility/multiplicity, literal
and stored roles, original include spans, math fidelity, section hierarchy,
declaration scope/redefinitions, reserved environment names and valid controls.
Independent review has cleared the role and math-layout guards and the
conservative AMS imported-namespace correction at `8a8f58c`. Its 198 pinned
tests and 11 independent probes pass, including both original namespace/load
findings. Stored-load and imported argument-consumer regressions are included.

The clean before checks for the affected objects and bibliography suites passed
using an explicitly rooted, source-equivalent borrowed CLI, with all 72
Rust/Cargo/config inputs compared against `fd024fc`. The producer source and
binary hash are retained, along with the limitation that no standalone Cargo
JSON CLI build receipt existed. Final validation must use this worktree's own
build and all required gates; it has not yet run. No final scorecard delta,
expanded truth release or completion of issue #97 is claimed.

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

Prepared blind packets bind all source members and actual PDF/index hashes.
Their native export contains only text, pages and word rectangles; embedded
historical detector figures/objects and automatic outcomes are omitted. Primary
and independent annotators have separate output directories and first enumerate
the complete paper without a parser inventory. Existing reconciled annotation
contracts remain the publication boundary. This pilot will measure annotation
throughput, complete per-kind coverage and demonstrated tooling gaps before
committing to the remaining 14 papers. Selection and packet preparation alone
admit no truth, and the page cap introduces an explicit short-paper bias.

## Retained evidence

The machine-readable companion is
[`eval/evidence/k1-coverage-progress.json`](../../eval/evidence/k1-coverage-progress.json).
It binds the raw comparison, independent audits, source-role probes, capability
bounds and targeted pilots to their external paths and SHA-256 hashes. The
immutable limited release has a version-selected replay verifier pinned to its
original reviewed source bytes; changes to the current parser cannot silently
change historical truth or select another release implementation.
