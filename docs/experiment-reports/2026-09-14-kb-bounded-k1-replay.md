# Bounded K1 publication, replay and collection

Issue #129 adds a bounded per-paper release format and serial replay/collector path.
The existing V4 `objects.json` is 991,418 bytes, close to the legacy 1 MiB document
limit. In the measured scratch layout the same complete nine records occupy
973,901 child bytes behind a 27,745-byte object manifest; the largest child is
282,395 bytes. This removes the requirement to load a complete release as one
document without discarding fields, source memberships, unknowns or excluded rows.

This is a mechanism change. All 73 V1–V4 implementation/configuration/truth files
remain exact, the future V5 manifest pin remains disabled, and no new real paper
is admitted. The approximately 500-paper requirement in #97, Phase A/Wave A2 and
overall application acceptance remain open.

## Format and failure boundaries

The [format contract](../../eval/k1-per-paper-contract.md) defines canonical small
roots and complete ordered paper children. Each descriptor binds identity, ordinal,
eligibility, counts, path, bytes and hash. Metric denominators use each metric's own
cohort. O1-only, O2-only, formal-only and wholly ineligible records cannot contribute
to another metric merely because they contain raw objects. Complete reviewed
negative papers remain scored; all-false records remain present with their full
unscored candidate. Global retained object counts stay separate from scored counts.

The history worker verifies original automatic evidence once, then each isolated
worker assembles one paper. Publication forms roots only after every selected
identity succeeds, verifies the complete staged release, and finally renames it.
Failure retains staging, requests, bounded pipe output, process status and ledgers.
There is no shorter-subset publication or fallback to a previously successful row.
The serial collector retains a decision for every row and runs the existing bridge
for each paper in the O1/O2 union. One active metric input binds every truth and
decision child; compact observations also commit full external predictions and
process evidence. The harness streams evidence hashes without changing digest bytes.

Finite limits include 1 MiB roots, 8 MiB children/responses/inputs, 1,000 selected
identities, 1 GiB independent truth/prediction/decision totals, JSON depth 64 and
200,000 structural tokens, and 100,000 O2 values including zeros. Replay workers
have 768 MiB address space, 60 CPU seconds and 90 wall seconds. Coordinators and
serial bridge calls have separate finite limits. Writes preserve 20 GiB free plus
the next file's bytes. Pipes drain concurrently with exact limit-plus-one rejection;
timeouts, cancellation and terminal leaders clean up the owned process group.
These are rejection boundaries, not promises about a 500-paper production workload.

The projection seam remains available to later annotation codecs. This change
does not implement or relax any annotation codec, source parser, detector, native
index, matching rule, region geometry or eligibility policy.

## Complete real-cohort comparison

The measured source is `47ef6f9aaf54cb7520e3db2fbb8d086d599740e3`. Fresh original
V1–V4 collectors first verify their frozen releases. Separate scratch releases then
exercise the new publication, Coordinator, process transport, replay and serial
collector against all original complete records. Across overlapping releases this
is 18 complete paper rows and 117 outcomes, representing nine unique papers and
50 unique objects. Ordered negative-only papers and all raw prediction fields remain.

| Cohort | Papers | Outcomes | O1 before and after | O2 before and after | Zero IoUs |
|---|---:|---:|---:|---:|---:|
| V1 | 2 | 15 | 1 | 0.9107793204006069 | 1 |
| V2 | 3 | 23 | 1 | 0.9169720168893188 | 1 |
| V3 | 4 | 29 | 1 | 0.9176618036504631 | 1 |
| V4 | 9 | 50 | 0.98989898989899 | 0.8910021250829322 | 2 |

Every complete observation row, prediction and all 117 outcomes compare exactly;
there are no regressions or unknown truth regions. V4 remains 49 TP / 0 FP / 1 FN.
Independent review recomputed caption matching and region arithmetic and verified
all 44 replay workers, 18 bridge calls and metric-child evidence. The active input
remains the actual legacy V4 measurement. Baseline bytes, adjustment histories and
the .90/.75 targets do not change.

The scratch producer is an explicit, fixed measurement adapter: only the expected
versioned worker invocation is replaced. Its history action freshly runs the
original verifier; its paper action returns the exact original complete record.
This measures new transport/publication/replay and real serial collection. It does
not claim that the current annotation worker freshly assembled legacy annotations,
nor that adapter runtime predicts the cost of a future annotation codec. Current
manual-worker behavior is tested separately through source-bound offline fixtures.

The combined real experiment took 337.663 seconds. Independently measured parent
CPU was 31.489 seconds and waited-child CPU 318.180 seconds; kernel peak RSS values
were 108,896 KiB for the parent and 187,552 KiB for children. These high-water values
are not added and do not establish simultaneous whole-process-tree memory use.

## Generated 500-record resource and late-failure test

The fixed synthetic fixture contains 500 inert records with 128 KiB unknown payloads,
interleaved metric cohorts, 84 negative records and 83 wholly ineligible records.
Its 66,039,373 complete child bytes sit behind a 250,913-byte object manifest.
Independent review reconstructed all 11 ordered metric cohorts and denominators,
including O1-only/O2-only cases and unscored raw inventories. These are generated
records and fixture flags, not 500 corpus papers or truth admissions.

Successful publication plus full serial replay took 240.528 seconds and completed
1,002 workers. The separate late-failure phase took 153.095 seconds: the history
worker and 499 paper workers succeeded, then the exact final ordinal returned exit
7. All 499 staged children and the failed final request/output/receipt remain, but
no final release root exists. The last row is a negative example, so complete
identity checks are necessary even when positive denominators alone look complete.

The combined synthetic experiment took 393.982 seconds, including coordination
overhead. Parent CPU was 311.618 seconds and waited-child CPU 75.163 seconds. The
outer kernel parent/child RSS peaks were both 39,632 KiB; they are separate
high-water measurements, not an aggregate. Phase parent/child peaks were
33,056/31,520 KiB for completion and 33,568/33,568 KiB for failure. This experiment
measures serial publication and replay only: it is not a 500-paper native,
collector, annotation-codec or end-to-end corpus benchmark.

## Preserved failures and validation

Independent source review found and corrected three boundaries: SIGTERM bypassed
worker cleanup; transport retention omitted the free-space check; and nested bridge
JSON could exceed the new depth/token limits. Regression controls replay the exact
failures, including interruption with a custom prior signal handler and rejection
before metric publication. Source copies, initial findings and corrected reviews
remain retained.

Measurement preparation separately corrected aggregate accounting for external
worker/output roots, reserved final failure metadata after an exhausted budget,
and preserved executable mode on the archived helper. The first actual real launch
failed before any cohort: the build-stage 1.5 GiB virtual-address limit prevented
the installed linker from mapping its inputs. A retained exact Cargo diagnostic
reproduced the failure; the same command completed under a finite 4 GiB build-stage
limit. Scratch coordination returns to 1.5 GiB, and production per-paper limits
remain 768 MiB. The failed launch, diagnostics, raw compiler output and selected
executable are preserved. A later synthetic launch explicitly binds only the 16
expected generated metric/history files instead of requiring the premeasurement
clean tree; unknown source or output changes still reject.

Formatting, strict all-target/all-feature Clippy and all-target Rust pass. Isolated
G5 records 428 Rust / 612 Python / 85 Node tests; provider-budget, bibliography and
scale checks pass. After ordinary main integration, the Rust quality gates run
again on `a08cca9`; Python/Node and measured source continuity are exact. Owning-CLI
checks before and after produce fresh available 50-case O1/O2 values with unchanged
baseline bytes. No expensive real or synthetic experiment is relabeled as a fresh
post-integration run.

Portable evidence retains full child outputs, failed generations, compiler and G5
logs, source inventories/copies, selected CLI/helper executables and generated
provider-budget inputs/observations. The existing TypeScript 5.9.3 package supplies
132 byte-verified retained files without a download. The external file manifest
also covers worker/prediction directories outside the experiment root. Original
corpus artifacts remain at their canonical paths with their reviewed hash bindings.
[Machine evidence](../../eval/evidence/bounded-k1-replay-129.json) identifies exact
receipts and reviews. Provider/model-service calls and billed external cost are
zero; agent reasoning cost is unknown. No deposited source is executed, no native
index is rebuilt, and no new truth release is activated.
