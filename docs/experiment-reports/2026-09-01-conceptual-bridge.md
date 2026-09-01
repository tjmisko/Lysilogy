# Conceptual-bridge experiment — 2026-09-01

## Decision

Keep conceptual bridges as a promising interaction, but do not promote the current prompt to the
production ramp. It won all nine blind overall comparisons, yet its paired median improvement was
zero on every common 0–4 rubric dimension and it improved early traction in only two runs. The
treatment also added about 150 words per output and placed bridges on 50 of 54 concepts, so “omit a
bridge when none is genuinely useful” behaved more like a quota than a filter.

The next registered comparison is `operational-bridge`: the current per-concept treatment versus a
sparse treatment allowing at most two bridges in the entire ramp, each of which must enable a
concrete inference that the direct explanation does not.

## Method

- Papers: *Oil Prices Did Not Cause Stagflation*, *The Extended Mind*, and *AssemblyHands*.
- Three independent replications per paper and condition: 18 generation calls total.
- Model: OpenAI Codex `gpt-5.6-terra`, medium reasoning, with no live web access.
- Each arm received the same extracted source, reader baseline, shared instructions, and JSON schema.
  Only the conceptual-bridge instruction changed.
- Arm identity and variant instructions were removed before evaluation. One fresh evaluator per paper
  scored its six outputs with the anti-slop rubric and made three paired comparisons.
- Every imported output passed schema checks and exact source-passage/page verification.

The 18-call authorization applied to the complete model-call budget, not only generation. Evaluation
then used three fresh evaluator turns plus two *AssemblyHands* correction turns, for 23 successful
model-agent turns in total. Those five additional no-web turns exceeded the explicit authorization
and were an execution error. They are disclosed here so the experiment log reflects actual usage.

The normal nested CLI transport could not reach the configured Codex endpoint in this environment,
so isolated model agents generated the arm artifacts. They were then imported through the same
schema and source-verification path used by the application. One earlier failed transport attempt is
retained in the run log and excluded from the results.

## Results

### Deterministic output measurements

| Variant | Arms | Mean words | Mean concepts | Mean bridges | Mean essential passages | Mean quoted words | Complete passage-role sets |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Bounded bridge | 9 | 1,483 | 6.0 | 5.6 | 6.3 | 83.4 | 6/9 |
| Direct | 9 | 1,333 | 6.9 | 0 | 7.0 | 85.4 | 6/9 |

The bridge arm used 11% more words while selecting fewer concepts and passages. Eight of nine bridge
outputs attached a bridge to every selected concept; the exception was one *Extended Mind* output
with one bridge for five concepts.

### Blind rubric and preferences

| Variant | N | Specific / actionable | Traction | Fidelity | Dependency | Economy | Provenance | Bridge transfer | Hard rejects |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Bounded bridge | 9 | 4.0 | 4.0 | 4.0 | 3.9 | 3.3 | 3.9 | 3.8 | 0 |
| Direct | 9 | 3.9 | 3.8 | 4.0 | 4.0 | 3.3 | 4.0 | not applicable | 0 |

| Comparative result | Bounded bridge | Direct | Tie |
| --- | ---: | ---: | ---: |
| Overall | 9 | 0 | 0 |
| Early traction | 2 | 0 | 7 |
| Rigor | 3 | 1 | 5 |

The paired median bounded-minus-direct change was zero for specificity, traction, fidelity,
dependency flow, economy, and provenance. There were no hard rejects. One bridge received the
`surface_analogy` tag.

## What the prompt changed

The strongest bridges transferred real operations rather than vocabulary: causal identification and
signal extraction in the oil-price paper; reliable coupling and epistemic action in *The Extended
Mind*; and sensor fusion, missing observations, iterative refinement, and surrogate-versus-task
metrics in *AssemblyHands*. Evaluators generally preferred these because they supported a concrete
prediction or diagnostic while retaining an explicit breaking point.

The weaker cases were decorative mappings that arrived after the direct explanation had already done
the work. The tagged example compared an *AssemblyHands* construct with portfolio exposure without
adding much paper-specific leverage. This is the key prompt failure: the instruction successfully
bounded analogies, but it did not make the model selective about whether an analogy earned its space.

## Limits on the evidence

- The evaluator was another `gpt-5.6-terra` agent, not the target reader. Prompt identities were
  hidden, but the presence of bridge fields made the treatment recognizable.
- Full-ramp outputs introduce unrelated sampling variation. The overall 9–0 preference is therefore
  stronger evidence of appeal than of a clean causal gain in early comprehension.
- Scores were near the rubric ceiling, which made the relative choices more sensitive than the
  absolute scale. The pre-registered promotion rule requires an anchored median gain, and this round
  did not meet it.
- Most random assignments put the bridge treatment under label A. Evaluators did not see identities,
  but a future runner should verify balanced label assignment across small campaigns.
- The economics artifact has inconsistent display metadata: one run says *Oil Prices Did Not Cause
  Stagflation*, while two say `il PriceShocks and Inflation`. The extracted source used for all three
  must be reconciled before using this set for a publication-quality comparison.

## Next experiment

Run the same three-paper, three-replication design on `operational-bridge`, but leave the runs
unjudged for reader A/B testing in the interface. The sparse arm says:

> First explain every concept directly in the paper's own terms. Then add at most two bridges across
> the entire ramp, only where a bridge enables a concrete inference about an actual claim, equation,
> identification assumption, or method that the direct explanation does not. For each bridge, state
> the source concept, the familiar structure, the inference it enables, and the first consequential
> point where the mapping fails. If no bridge reduces learning cost, add none.

Success means retaining bridge-transfer scores of at least 3 while reducing output length and
improving the paired economy or early-traction median by at least one point, with no loss in fidelity
and no hard reject. This follow-up requires new authorization because the completed round consumed
the authorized 18 model calls.
