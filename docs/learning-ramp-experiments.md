# Learning-ramp experiments

## Product intent

Lysilogy should make papers at the edge of, or just beyond, the reader's current understanding
meaningfully more legible. It is not primarily a summarizer. Its job is to construct a smooth ramp
from what the reader already knows to the paper's actual claims, vocabulary, evidence, and place in
the field without sanding away important technical distinctions.

The initial reader baseline is:

- strong mathematical literacy;
- economics and finance;
- some physics;
- AI and machine learning;
- intelligent-outsider knowledge, rather than assumed specialist knowledge, in the paper's field.

This baseline is an experimental input, not a permanent universal persona. Later work should make
it editable and should learn more specific, demonstrated knowledge without treating a broad field
label as proof that the reader knows every concept in it.

## The desired experience

A successful reading ramp gives the reader traction early and adds detail monotonically. Each step
should make the next step cheaper to understand. The reader should be able to stop at any level with
a coherent, honest mental model, while continuing reveals the paper's real terminology and
qualification rather than merely repeating a longer summary.

The core strategies to test are:

1. **Bridges from known fields.** Use analogies to math, economics, physics, finance, or AI when the
   mapping is structurally useful. State where each analogy breaks so familiarity does not become a
   false equivalence.
2. **Dependency-aware glossing.** Introduce load-bearing terms in an order that minimizes forward
   references. Compare this with just-in-time glosses in source reading order.
3. **The essential 10%.** Identify the smallest set of exact passages that carries the paper's
   question, mechanism, evidence, and qualification. Include the minimum context and prerequisite
   concepts required to understand those passages; do not merely select quotable sentences.
4. **Use and reception.** Explain what later work cites or quotes from the paper, what the result is
   used for, and recurrent ways it is extended, simplified, or abused. Keep paper-internal claims
   visibly separate from externally researched reception.
5. **Informed opposition.** Steelman counterarguments available inside the field. When the paper
   belongs to a recognizable camp, identify competing camps, their strongest objections, the
   assumptions in dispute, and the response the authors' camp would likely make.

## What the experiments should optimize

“Best” means a ramp that performs well on several dimensions, not the answer that sounds most
impressive:

- **early traction:** how quickly the reader acquires a usable mental model;
- **dependency flow:** whether explanations arrive before they are needed;
- **transfer:** whether bridges to known fields genuinely reduce learning cost;
- **fidelity:** whether simplification preserves the paper's mechanism, evidence, and caveats;
- **reading leverage:** whether the suggested 10% is worth reading and comes with enough context;
- **field calibration:** whether reception and counterarguments sound like the field rather than a
  generic debate template;
- **uncertainty hygiene:** whether source claims, external interpretation, analogy, and inference are
  distinguishable;
- **economy:** useful understanding gained per minute and per token.

## Experimental method

Prompt experiments should normally change one named dimension at a time. The paper, model,
reasoning effort, output schema, and shared instructions remain fixed within a run. Both variants
are generated independently and presented under blind labels `A` and `B`; prompt identities are
revealed only after judgment. Runs and judgments are plain JSON/JSONL artifacts beside the paper.

The first experiment catalog isolates five dials:

| Dial | Variant A | Variant B | Main question |
| --- | --- | --- | --- |
| conceptual bridge | no analogy requirement | mapped analogy with a break point | Does prior-domain transfer improve traction without distortion? |
| glossary order | paper encounter order | prerequisite/dependency order | Which order makes later explanations easiest to parse? |
| essential passages | importance-only selection | comprehension-budget selection | Does budgeting context improve the value of the chosen 10%? |
| reception | influence summary | concrete use / misuse taxonomy | Which makes the paper's actual afterlife clearer? |
| opposition | paper-local caveats | camp-aware steelmanned opposition | Which produces informed skepticism instead of generic limitations? |

The interface's initial human eval asks for an overall preference, early-traction preference, rigor
preference, confidence, and an optional note. This comparative vote is useful for blind inspection,
but it cannot distinguish “both good” from “both bad.” Before interpreting a win, score each arm
independently with the factuality gate and anchored dimensions in
[`experiments/rubric.json`](../experiments/rubric.json). Record both-fail, both-good, tie,
not-applicable, and confidence separately. Aggregates should segment results by paper distance and
field: a prompt that works for a nearby economics paper may fail on a distant biology paper.

The experiment name and research question are evaluator metadata. They must not appear in either
generation prompt because treatment-oriented wording can teach the control arm what the treatment
is supposed to do. The catalog reader baseline is passed into generation, but its broad field names
are only possible bridge domains—not proof that the reader knows every concept in those fields.

Exploratory tests should compare the affected component over at least three generations per
condition and paper. Full-ramp generations are a confirmatory test because unrelated stochastic
changes in other sections can otherwise dominate preference. Reception and field-opposition tests
also require a frozen evidence dossier shared by both arms; independent searches change both the
prompt and the evidence corpus.

## Guardrails

- Never invent an analogy merely to satisfy the format. “No useful bridge” is a valid result.
- Analogies must include the important mismatch or failure mode.
- Essential passages must be exact source text with page numbers and must cover at least one
  qualification or limitation when the paper contains one.
- Externally researched use, reception, camps, and counterarguments require explicit sources.
- A citation or prominent later use is evidence of influence, not evidence that the paper is right.
- Counterarguments must target the paper's assumptions, methods, inference, or scope; avoid generic
  “more data is needed” filler.
- The experiment lane must not overwrite the canonical `analysis.json` or reader highlights.

## Decision rule

Do not promote a prompt variant from one pleasing example. Use multiple papers spanning “near,”
“edge,” and “beyond edge” difficulty, inspect failures, and prefer variants that improve the target
dimension by at least one anchored point without a hard reject or a material loss in rigor,
specificity, or economy. Use at least three generations per condition and paper and report paired
median score changes plus failure rates, not only wins. Promote stable winners into the production
prompts one dial at a time, then rerun the representative set.

The dated reports in [`docs/experiment-reports/`](experiment-reports/) are the experiment log. A
report must distinguish canonical-analysis audits, failed runs, exploratory component tests, and
completed confirmatory A/B runs so that absence of evidence is not accidentally reported as a
prompt result.
