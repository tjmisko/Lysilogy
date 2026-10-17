# Product roadmap and goals

Status: 2026-10-16. This document records the product direction settled on
2026-10-16, including the corrections that reset the earlier extraction plan. It
supersedes the framing in [tasks/README.md](tasks/README.md) and
[tasks/HANDOFF.md](tasks/HANDOFF.md) where they disagree.

## What this is

Lysilogy is a keyboard-first atlas and contextual reader for scientific papers.
It is a browsing and navigation surface: a reader moves between the literature,
a paper, and a passage, and stays oriented at every level.

## Who it is for

An insider or an outsider to a field who wants to move between levels quickly,
engage deeply with the good papers, and understand big results as connected to
papers. The reader does not want summaries to replace the good papers. For the
good papers, the original text is the point.

## The core model

- **The unit is the paper.** The literature is the topic of interest, but it can
  only be understood by studying the papers and their interrelations.
- **Two speeds, both first-class.**
  - *Glance*: a sub-minute judgment. Is this worth reading? What is it? Where
    does it sit? What is the one result? Is the summary enough?
  - *Deep*: the argument, the objects, the method, the best original passages.
- **Reading is replaceable for the forgettable and not for the good.** The tool
  makes it possible to *smell* which is which, or see it at a glance.
- **The afternoon scenario is the design target.** In an afternoon a reader
  might look into 100 papers, 90 at a glance in under a minute each; spend three
  hours reading one in full and 30 minutes each on three others; and finish
  knowing what they crossed through.

## Goals

1. **Triage at a glance.** A paper must be skimmable in seconds: identity, type,
   thesis, position, headline result, and an honest read/skim/glance judgment.
2. **Deep engagement with the good papers.** The argument and the objects are
   augmented, not replaced. The reader can always reach the original text.
3. **Position early.** Situating a paper in the literature belongs at the front
   of the scanning surface, not buried. The current "abstract page" position
   card is the prototype and is janky.
4. **Model the literature through relations.** Who builds on, contrasts with,
   critiques, replicates, and uses whom, with a plain-language why and a
   verbatim quote. Lineages over time and debate clusters organize the map.
5. **Time and people.** Not only what is live now, but what was live, when, and
   who made it live.
6. **A session the reader can review.** What was seen, what it was judged to be,
   what was crossed through.
7. **Resonance, sparingly.** Point the reader at details, items, or concepts
   that will surprise or delight. This is a final pass, gated on a fuzzy working
   model of what the reader wants, and 90% of papers will have nothing.

## What the reader gets from one paper

Two layers per paper, plus the cross-paper and user layers.

### Glance layer

- identity: title, subtitle, authors in order, year, venue, identifiers,
  published/preprint state
- type: survey, method, empirical, theory, dataset, position, replication,
  resource
- thesis: one plain sentence
- position: predecessors, targets (contrasts/critiques), replications, each with
  a one-line why and a citation
- headline result: the main finding and the object or number that carries it
- read/skim/glance judgment with a reason, labelled as model judgment
- triage signals from outside the paper (citations, venue, contestedness) come
  from the graph or external sources, not the paper

### Deep layer

- **argument / supercut**: short, worth reading, simple. Ordered segments of the
  paper's own sentences threaded with model connectors. Length and quote density
  adapt to the importance of the paper: good ideas and good prose get more
  original text and more length; a forgettable paper can be one paragraph.
  There is a fuzzy floor of model text for papers that say it badly.
- research question or hypothesis
- claims with an evidence map: statement, strength, and the object, number, or
  section that supports it
- method card: setting, data or sample, procedure, assumptions
- key numbers: the quantitative results with their context
- limitations and open questions
- objects: figures, tables, equations, each with label, page, caption, plain
  language meaning, why it matters, and where it is referenced
- notation and definitions, plus a glossary of terms
- references and in-text citations

### Cross-paper layer

- edges: relation, plain-language why, verbatim quote, page, using the existing
  vocabulary (`background`, `uses_method`, `uses_data`, `extends`, `supports`,
  `contrasts`, `critiques`, `compares`, `other`)
- lineages and threads over time
- debate clusters, which organize how the map is drawn
- a per-paper position card
- temporal view: what was live, when, who

### User layer

Separated from the paper model and used sparingly.

- a fuzzy, not-too-constraining working model of what the reader wants and why
  they are reading. This is not always transparent or stable, so it is never
  treated as ground truth.
- a session log: seen, judgment, crossed through
- the resonance pass, gated on the working model

## Corrections that reset the plan (2026-10-16)

These corrections supersede earlier framing in `tasks/`.

- **PDF only.** The product never has LaTeX. The rendered page is the evidence.
- **The LaTeX harness is a benchmark only.** It measures text-layer fidelity and
  supplies an inventory oracle (how many equations, figures, tables, references
  exist, and their labels/numbers). It is not a runtime path and is not how we
  reconstruct equations.
- **Equations are read from the image.** Find the displayed equations in the
  PDF, read them off the page image, reconstruct LaTeX (or close), give each a
  plain-language meaning and its role, and organize them. Deterministic text
  extraction cannot do this; the benchmark confirmed it.
- **The deterministic-first extraction plan is on hold.** A large deterministic
  pipeline was built for abstract, title, bibliography, floats, and boundaries.
  It is not proving out. The new hypothesis is that one agent, after basic
  resource extraction, can fill the whole paper data model more accurately and
  more cheaply. We benchmark before committing further.
- **Abstract is a model pass.** Nearly every paper has an abstract. Pass the
  first few pages (text and, when needed, images) to a cheap model with a light
  pre-detection of the word "Abstract". Near-100% is expected.
- **The model proposes, the application admits where it can.** For text, a value
  is admitted only when it is verbatim in the source. For equations and images,
  the crop is the evidence and admission is fidelity to the image.
- **Model judgments are labelled.** Read/skim calls, resonance, and inferences
  are model output, never presented as paper fact.

## Architecture hypothesis under test

Basic resource extraction is the only deterministic stage:

1. PDF to page-marked text.
2. PDF to page images.

Everything else is produced by a model pass that fills the paper data model and
checks its own work. The application owns admission, provenance, and the
spend ledger. This hypothesis is tested by the spike in
[tasks/HANDOFF-extraction-spike.md](tasks/HANDOFF-extraction-spike.md) before
any further deterministic or agent infrastructure is built.

## What is kept

- the reading index and basic resource extraction
- the spend ledger and per-call funding/cost recording
- the LaTeX truth harness, as a benchmark and inventory oracle
- the citation-graph edge vocabulary and claim provenance
- ramps and the reading-index concepts

## Non-goals and cautions

- Do not replace reading for the good papers. The original text stays reachable.
- Do not overwhelm. Better to miss an interesting detail than to flood the
  reader. Resonance is rare and opt-in.
- Do not rebuild deterministic proofs for things a model can copy.
- Not a long-form literature-review generator; the reader does the reading.

## Open questions

1. How much extra model work lineages, debate clusters, and position cards
   need, and whether they run per paper or over the graph.
2. How the user working model is stored and scoped (session, workspace, goal),
   and how the resonance pass is gated on it.
3. What the triage signals are and where they come from (citations, venue,
   contestedness).
4. Which paper-level fields the agent can be trusted with versus those that need
   the graph.
