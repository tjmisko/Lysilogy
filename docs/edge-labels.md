# Edge labels: why a paper cites each work

Status: 2026-11-18. Built and tested against stub model answers only; the
database-backed tests skip without `LYSILOGY_TEST_DATABASE_URL` and have
not been run against the review fixes below. No live model has labelled an
edge with the version 3 prompt yet.

Every citation edge in the literature graph (paper A cites work B) can carry
one current label saying why A cites B: a relation, a reason, a verbatim
quote, and how much A does with B. Read across a paper's references, the
labels show which works it stands on, which it tests or argues with, and
which it only mentions; read across the papers that cite it, they show what
happened after. The Abstract page's "why it matters" reads the same labels,
so the literature positioning and the graph agree.

## The contract

- **One current label per edge**, stored as a `graph.relationship_claim`
  row with a null `superseded_by`. Nothing is deleted: a replacement points
  the old row at the new one.
- **Every label is grounded in the citing text of its own edge.** A
  labeler answer is kept only when its quote (at least three words) is an
  exact substring of a passage the model was shown, and, in a passage
  widened by a neighbour sentence, when it overlaps the citing sentence or
  sits in a neighbour that carries no other work's citation marker
  (`quote_outside_citing_sentence` otherwise). A core-reading relation is
  written only when its quote (whitespace and case folded) is in one of the
  edge's stored mention passages, or, for an edge without stored mentions,
  in a core citing sentence for it (`not_in_citing_passage` otherwise);
  quote admission alone only checked it against the page. A person's new
  quote is checked against the stored passage; a quote kept unchanged in an
  edit is not re-checked.
- **Every label records its path** (`label_path`, migration 0011):
  `core_relation` (the core reading's admitted relations), `edge_labeler`
  (the dedicated per-edge labeler) or `user`. Rows from before the migration
  are classified where they can be, and an unclassified AI row reads as
  `edge_labeler`. Labeler rows also record `engagement` and
  `prompt_version`.
- **Models propose, the application admits, and nothing is dropped
  silently.** Every core relation and every edge ends in one counted
  outcome; reference keys that do not resolve are counted by reason and
  listed by name in the run record.

### Vocabulary

Relations (unchanged since version 1, so stored labels, the prominence
counts and the core reading's schema keep working): `background`,
`uses_method`, `uses_data`, `extends`, `supports`, `contrasts`, `critiques`,
`compares`, `other`.

Version 2 adds `engagement` instead of new relations, because the finer
distinction the owner asked for is about weight, not kind:

| engagement | meaning |
| --- | --- |
| `substantive` | the paper does something with this work: takes a method, data or result, tests, extends or disputes it, or measures against it |
| `perfunctory` | mentioned in passing, often one of several works in one bracket |
| `not_shown` | the passages do not show why the work is cited |

`background` and `other` are the honest relations for perfunctory and
unclear citations, and the application enforces it for `not_shown`: an
answer pairing `not_shown` with any other relation is rejected as
`inconsistent_engagement`. Labels from the core reading have no engagement
(unstated). A label "names a role" when its engagement is `substantive`,
or no engagement is stated and its relation is neither `background` nor
`other`; the run counts these (`labelled_with_role`) and the context
dossier ranks only these as leaned on.

Version 3 asks first for `cites_this_work`: `false` means no passage refers
to the cited work (a citation-matching error). Such an answer is counted as
`misattributed` and no label is stored.

## The merge rule

`lysilogy_ingest::labels::merge_decision` is the only place two paths meet:

1. A person's label, or any label a person accepted or rejected, is never
   replaced by automation.
2. The core reading's relation is the cheap first label. A newer core
   reading replaces an older unreviewed core label.
3. The edge labeler read the edge's own passages with a brief written for
   this question, so its unreviewed label outranks a core label: a core
   relation never replaces it.
4. By default the labeler fills only edges without a label. With
   `relabel_unreviewed` it may replace any unreviewed AI label; with
   `relabel_older_prompt` (the `relabel_older_prompt` flag of the run
   route and `--relabel-older-prompt` of `lysilogy labels`) only unreviewed
   edge-labeler labels from an older prompt version. Replacing a core label
   that named another relation is counted as `disagreements_with_core`.

A core relation that meets a label it does not replace, or replaces one
with another relation, is listed in the run record as a disagreement with
both relations and the existing path.

## When the step runs

- After analysis: the initial analysis job has a `labels` task after
  `persist` (`AppState::label_paper_edges`, `src/edge_labels.rs`), then a
  `context-labels` task that rewrites the context node when the run wrote
  or replaced a label, so the first analysis of a linked paper already
  carries its own labels in "why this paper matters" (one more context
  call). The `labels` task is completed, a warning for a partial run, or
  failed; neither task fails the analysis.
- After the literature flow links a paper and refreshes its local evidence
  (unless a CLI sidecar labeler was asked for), so linking produces labels
  and a fresh `edge-labels.json`.
- On demand: `POST /api/papers/{id}/edge-labels/run` (optional body
  `{"provider": "openrouter", "relabel_older_prompt": false}`) and
  `lysilogy labels <paper>`. Both resume a partial run and rewrite the
  context when labels changed.

A run is `partial` when edges were left by failed calls, the budget or the
call limit, and `failed` when every labeler call failed; `detail` gives the
counts and the first error.

The steps:

1. **Skip, saying why**, when there is no graph (`LYSILOGY_DATABASE_URL`),
   the provider is the offline reader, or the paper is not linked to a Work
   with a recorded document version (run the literature flow first).
2. **Refresh local evidence** (bibliography entries and in-text mentions;
   local work, no network). A failure is recorded and existing mentions are
   used.
3. **Seed from the core reading** (`labels::seed`). The last run's
   `paper-artifacts.json` gives `relations`, `citations` and `references`.
   Each relation's `reference_key` is resolved to an edge through the
   paper's stored bibliography: the printed key, then a numeric key, then
   the first author's surname and year (a letter suffix must agree), then
   the core reading's own bibliography entry by DOI and then by title. One
   match resolves; several are `ambiguous_entry`; none is
   `no_matching_entry`; an entry with no accepted Work or no edge is
   `entry_not_in_graph`; an empty key is `empty_key`. A key that packs
   several works (`6, 7, 10`, `[3-5]`, `A, 2004; B, 2009`) is split, on `;`
   always and on `,` for numeric keys with ranges expanded, and each part
   resolved and grounded on its own (`multi_key`, `key_parts`). Edges are
   keyed by bibliography entry, so a preprint and its journal version that
   resolve to one Work stay two edges. A title match needs three shared
   words, 80% of the longer title, and the year to agree when both have
   one. Relations marked low confidence are left for the labeler. The core
   reading's citing sentences become passages for edges that have no
   stored mention. The stored model is the one `paper-artifacts.json`
   names, not the current configuration.
4. **Run the labeler** chosen by the provider setting
   (`labeling::labeler_for`): `OpenRouterLabeler` for OpenRouter, the CLI
   sidecars for Claude and Codex. It labels edges with citing passages and
   no label, most-mentioned first, up to `analysis.label_limit` calls.
5. **Record the run** in `papers/<id>/edge-labels.json`: status, labeler,
   prompt version, the analysis pipeline version, what seeding and
   labelling did (every count), and every outgoing edge with its current
   label.

### Budget and resumption

`OpenRouterLabeler` uses the analysis settings: `analysis.label_model`
(default: the judgment model, then the core model), the provider order and
the shared spend ledger, plus its own per-paper cap
`analysis.label_budget_usd` (default 0.10). A call refused by either budget
counts as `over_budget`, and a call past `label_limit` as `over_limit`;
both make the run partial, and running the step again resumes. Every raw
answer, including one that fails grounding, is cached in
`source.source_record` under a key that hashes the prompt version, the full
prompt text (brief and request as formatted), the schema and the request,
so an edit to the brief is never served a stale answer even without a
version bump, and a rerun pays only for edges never answered under the
current prompt. The run records `brief_digest`, a digest of the brief and
schema, so grades can tell prompt edits apart within one version. Bumping `LABEL_PROMPT_VERSION`
makes every edge without a label eligible again; edges that already have a
label keep it unless a relabel is asked for.

The `lit` job's `labels` option and the `lysilogy lit --labels` command
still label on demand through a CLI sidecar, with the same brief but
without core seeding.

## The prompt

`prompts/edge-label.md` (embedded by the ingest crate as `LABEL_BRIEF`) is
a brief to the model: who reads the labels (someone scanning a literature
from above), the question (why does this paper cite that work, and what
role does it play in its argument), what good and weak reasons look like
(illustrative examples about made-up reef-ecology papers, kept out of the
evaluation set by the prompt lint; the honest declines are described in
prose rather than given as sentences to copy), the honest uses of
`background`, `other`, `perfunctory` and `not_shown`, where a borrowed
definition or framework goes, and the order to decide in. The schema asks
for `cites_this_work`, `passage_index`, `quote`, `engagement`, `relation`,
`reason` in that order.

After the brief the model sees:

- the citing paper's title and its thesis from the analysis;
- the cited work's title, year, authors, bibliography entry, and its
  abstract when the library holds and has analysed it;
- up to six passages: each stored mention widened by one sentence on each
  side within its paragraph (from the reading index), with the citing
  sentence on its own, the citation marker as printed and how many works
  that marker cites at once, and the page; then the core reading's citing
  sentences when the edge has no stored mention.

## Where the labels are read

- **Literature graph** (`LiteratureGraph.tsx`): labelled edges are drawn
  heavier; hovering an edge shows its reason; selecting a work shows its
  reason under the canvas; the center paper's panel lists every labelled
  reference grouped by relation; the work panel's label cards accept,
  reject and edit as before and name the path and engagement.
- **Context node**: the neighbour dossier carries each neighbour's current
  label (rejected ones left out), both directions, and works with a
  substantive label rank first. `prompts/context.md` tells the model it may
  rely on a label as graph-record evidence (`dossier` basis), and that a
  perfunctory one only shows the work was cited. The stored dossier keeps
  the label, so a reviewer sees what the node read. The context is
  rewritten after any labels run that wrote a label, so the node reads the
  paper's own labels.
- **Abstract page**: `GET /api/papers/{id}/edge-labels` (outgoing labels,
  labelled incoming edges, the last run). Dossier claims and read-alongside
  works show the edge's "why" (the live label first, the stored copy
  otherwise); "How later papers cite it" lists incoming labels; the
  provenance line gives label coverage.
- **Review**: rubric component `edge_labels` (see `eval/review/README.md`).

## Tests

Stub answers, with a local Postgres for the database tests
(`LYSILOGY_TEST_DATABASE_URL`; they skip without it):

- grounding failures and the new engagement check (`labels.rs`);
- unresolvable reference keys, low confidence, duplicates, invalid
  relations and idempotent seeding (`should_seed_core_relations_and_report_unresolvable_keys`);
- a core relation and a labeler label disagreeing, both ways
  (`should_record_disagreement_between_core_relation_and_labeler`);
- cache reuse: answers cached under the previous prompt version are not
  reused, a run that stopped at its limit resumes from the cache;
- core citing sentences for an edge with no mention;
- the whole step over a linked paper (`api::labels`), the run record and
  task line, the review pin and summary, the dossier rendering and ranking,
  and the web helpers (`npm run test:reader-output`, `npm run test:review`).

## Not yet known

- Whether a real model follows the brief: specific reasons, honest
  perfunctory labels, `not_shown` when it should, quotes that show the role.
- How often core reference keys resolve on real bibliographies, and how
  often the core reading and the labeler disagree.
- The cost per paper of up to forty label calls on the judgment model.
- The OpenRouter labeler has never made a live request. Before relying on
  this, run `cargo test --workspace` with a Postgres in
  `LYSILOGY_TEST_DATABASE_URL`, and record one live label call on an e13
  paper to compare with the e13 relations.
- How often a cheap model answers `cites_this_work: false`, and whether the
  citing-sentence rule rejects many good quotes from neighbour sentences.
