# Independent figure/table metrics (E1.1 follow-up #96)

The first actual production measurement on `k1-limited-v1` is **O1 = 0.7692307692** and
**O2 = 0.2448506858**. Both miss their unchanged targets (0.90 and 0.75). The collector,
truth, and detector remain separate: this issue measures existing predictions and makes no
production detector changes. [Follow-up #101](https://github.com/tjmisko/Lysilogy/issues/101)
retains the measured failures and next implementation ideas.

## Population and matching

The frozen release supplies two complete figure/table cohorts: `2104.01511v1` has ten figures
and five tables; `2503.05828v1` is an independently reviewed negative. Every annotated visual
body has independent full-region evidence. Zero truth regions are unknown. This is a small,
manually selected cs.LG/2021 + econ.TH/2025 sample, not representative corpus coverage.
The original 500-paper target remains unmet under #97.

The [contract](../../eval/object-metrics-contract.md) was committed and independently reviewed
before any production prediction was inspected. Matching requires kind, exact normalized printed
identity, page, and at least 50% overlap on **both** caption memberships in frozen UTF-16 source
coordinates. Roman labels retain their identity: I does not become Arabic 1. Region overlap never
selects a match. Ambiguous identities remain unresolved; multiple candidates for one truth have
one deterministic winner and all other predictions are false positives. Review corrected a
contradictory explicit anchor-page field before measurement.

O2 uses every independently annotated truth object, including zero for missed predictions and
absent/invalid prediction geometry. Multiple truth rectangles use geometric union, separated by
page. Matched-only IoU is retained solely as a diagnostic. Caption rectangles never create truth.

## Measured result

| Population | TP | FP | FN |
| --- | ---: | ---: | ---: |
| Figures, positive paper | 10 | 1 | 0 |
| Tables, positive paper | 0 | 0 | 5 |
| Figures/tables, negative paper | 0 | 0 | 0 |
| Total | 10 | 1 | 5 |

O1 is `2×10 / (2×10 + 1 + 5) = 0.7692307692`. All five Roman-numbered tables I–V are
absent from production predictions. The extra Figure 2 at page 3, UTF-16 `[11920,12407)`,
does not match the independently annotated caption; a separate Figure 2 prediction does match.
The detector follow-up will distinguish that prose reference from the actual caption.

O2's **15-value** median is **0.2448506858**. The five missed tables contribute zero. The
separate **10-match** median is **0.2906506211**, with figure IoUs approximately 0.110–0.555.
Independent rectangle arithmetic confirms that all ten matched predictions contain 100% of
the annotated truth body, but their areas are approximately 1.80–9.13 times too large. The
follow-up will remove excess surrounding coverage using page/image/vector/cell evidence,
retaining axes, legends and subpanels while excluding separately printed captions. No matching threshold,
truth label or metric target was adjusted after looking at these results.

## Production and provenance boundary

The read-only Rust bridge calls `source_index::load_cached` and
`objects::ObjectsArtifact::from_reading_index` on the exact frozen native indexes. This measures
actual cached production detections and the current object wrapper; it does not perform new
extraction. The current figure-detector source is checked against the source fingerprint that
created those caches. Changing that detector requires a separately verified current prediction
and anchor-generation relationship; silently reusing old detector caches cannot establish a
new measurement.

The collector reconstructs the immutable K1 release in memory from all pinned independent review
bundles and both full 1,000-paper automatic ledgers. Source/PDF/index hashes and canonical active
PaperIds are checked, including conflicts, original library root and before/after registry bytes.
No corpus, index, identity registry, objects artifact, or enrichment artifact was written.

Every measurement invokes Cargo, verifies its exact selected example source/target/executable,
and hashes source dependencies before and after. The native bridge also validates deserialized
PaperIds, regular paths and all symlink ancestors, caps index reads and cumulative response size,
and requires the exact native-cache ETag. Explicit source, index, truth, evidence, executable and
object-artifact receipts are retained. Raw prediction text remains external; committed observations
contain only derived identities, counts, anchor-match scores, IoUs and hashes.

The measured source is `855888608de5700be1c6b388342c86ef39460944`, after ordinary integration of
main's reviewed page-failure isolation. The existing K1 indexes were unchanged by that fix.

- K1 SHA-256: `0afccc35dc5eedb48b4df4e30ee33e06dac02a07f0b7cab6d6f5c53743202976`.
- Positive index SHA-256: `f40e9c89ec89842c9c6cc6e4bce4ffd97093fc8908f5a4edf1586d8ec801a21b`.
- Executable SHA-256: `19688638474493c75796f1f696949bfa39d2a6abf6fd0cf9f0d5c65f268005e2`.
- Raw prediction SHA-256: `d662cc9ef128ce01983b5bf33fd3b693db5a11f85d1dce0e8fd9fc1e42de4bb6`.

Measurement took **4.570 seconds**, including the warm Cargo verification and truth revalidation,
with **zero network calls, zero model calls and $0 external cost**. Peak RSS was not instrumented.
Raw predictions are at `~/.cache/lysilogy/object-metrics/<prediction-sha>/predictions.json`;
gate logs/build receipts are archived under `~/.cache/lysilogy/object-metrics/measurement-8558886/`.

## Validation and scorecard delta

All required source gates passed at the measured commit: formatting, strict Clippy across all
targets/features, all Rust targets, **24 offline collector tests and three bridge regressions**,
provider-budget simulation, scale checks and isolated G5. G5 executed **320 Rust / 278 Python /
85 Node tests**. No test registers fixture-derived objective values. Reviewer separately checked
200 exact integer-grid IoU/union examples and independently reconstructed the actual K1 release.

Objects checks before collector publication reported O1/O2 unavailable. The first local baseline
ran during offline tooling work on a dirty branch, and a second explicit pre-measurement check ran
at the reviewed source; their actual commit/dirty fields remain in the receipts. Neither is
misrepresented as an earlier clean implementation run. Post-measurement `eval objects --check`
and `eval scale --check` pass: the new O1/O2 baselines are recorded below target, G5 remains
passing, and O30 remains zero violations across 10,000 references. The branch scorecard has one
of five hard gates passing and one of 30 objectives at target; unavailable metrics are not passes.

[Exact evidence](../../eval/evidence/object-metrics.json) retains command/result/log hashes and
[compact observations](../../eval/evidence/object-metrics-observations.json) retain every match,
miss and IoU. #101 is the required measured detector follow-up. No score target, hard gate,
source truth, or final system acceptance requirement changed.
