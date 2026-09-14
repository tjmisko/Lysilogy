# Independent LaTeX object truth

This evaluation-only code reads deposited arXiv sources in memory. It never extracts archive
members to disk, invokes a TeX engine, or evaluates shell escapes. Tests use synthetic source
strings and never publish them as K1.

`archive.py` bounds compressed/expanded bytes, member sizes/counts and text size; rejects links,
traversal, duplicate canonical paths, sparse and special members; and supports standalone gzip
LaTeX. `tex.py` preserves source positions while masking comments and inert definitions, expands
bounded literal includes, and records source membership and unsupported presentation commands.
A unique document root is the default. The builder accepts equivalent resolved closures when
every resource is established; first-page title substrings and filenames cannot break an ambiguity.

`parser.py` derives figure/table captions, numbered equations, custom theorem-like environments,
proof targets, algorithms, bibliography items, citation keys and object references from source
structure. Top-level align/gather/eqnarray rows remain distinct, suppressed rows are excluded, and
nested aligned/split environments stay inside their numbered equation. Unknown citation commands
or unknown object environments withhold exhaustive truth. Used macro aliases that transitively
contain structural/link commands, conditionals, and repeated definitions also withhold admission;
the reader does not execute their visibility or scope semantics. Fields come only from explicit
deposited bibliography-field markup that establishes printed field boundaries and role. Active
BibTeX databases supply comparison evidence, but BibTeX-only fields stay unknown: a word/year
appearing somewhere in an entry cannot establish a complete title or publication-year label.

`align.py` reads only actual PDF-index text and token geometry. It never reads production figure,
object, paragraph or citation predictions. Unique source-text matches retain half-open UTF-16
spans. Source contexts identify printed citation/reference occurrences; source keys supply their
destinations. Missing or ambiguous matches remain in per-paper overall completeness. An explicit
per-metric cohort requires every relevant source item to align at the declared quality threshold
(0.95); a missing member excludes the whole paper/kind, and never shrinks its denominator.
Bibliography entry and citation inventories must be exhaustive to enter O8–O10. See
[`eval/latex-contract.md`](../../../eval/latex-contract.md) for the cohort and omission contract.
Math matching preserves case and operators inside every object, bibliography entry and link
context. Unsupported math/script binding withholds that evidence. Multiple source objects cannot
certify the same PDF span.
Caption token bounds are explicitly not full figure-region labels; full-region and O11 panel
annotations require separate independent evidence.

The offline `k1_index` Rust example maps PDFs through the production identity registry into a
separate existing data root beneath `~/.cache/lysilogy/`. It verifies frozen PDF hashes before
and after local index extraction and keeps the canonical registry across reruns. Corpus PDFs
remain read-only under `~/Corpora/`; all full text and extraction caches remain external.

K1 publication, stratified build receipts, and three-agent panel judgments must establish actual
coverage before this issue is complete. An exploratory parse/alignment pilot is not final K1.

`python3 -B scripts/truth/latex/builder.py --pilot 25` freezes a category/year-stratified
exploratory input snapshot from verified eval PDF/source pairs, validates manifest/sidecar identity,
and retains candidates under `~/.cache/lysilogy/k1-builds/`. It builds the offline native helper,
uses Cargo's reported executable, and fingerprints source, executable, PDF, source archive, and
index bytes. It never changes corpus files. Main-file ties require a unique source title in the
actual PDF with independent complete-title evidence (not yet automated), or identical resolved
text, active bibliography, and graphics hashes. Uninterpreted local package/class semantics
withhold both root-equivalence claims and object inventory admission.
All regular archive members are hash-pinned in memory; binary resources are never decoded as TeX.
The full-build path currently refuses publication until the coverage and independent panel
contract is implemented and satisfied; exploratory reports explicitly retain that limitation.
Every aligned candidate retains the complete parsed object/entry/link inventory and its hash
alongside omissions, so numeric link indices and all metric denominators remain auditable.

A reviewed manual bundle can be assembled without model or network calls:

```sh
python3 -B scripts/truth/latex/manual.py \
  --candidate k1-2104.01511-candidate-d75ddba.json \
  --region-bundle k1-region-annotation/2104.01511v1 \
  --panel-bundle k1-panel-pilot/2104.01511v1-d75ddba
```

Those paths are relative to `~/.cache/lysilogy`. The tool uses the frozen full-eval input/map
receipts by default and writes an external hash-addressed candidate plus execution receipt.
It rehashes original artifacts and all reviewed page images, retains automatic exclusions,
and binds three distinct blinded panel identities. This is a reproducible exploratory manual
candidate, not a publication or a production O11 score. Prior agent judgment dollar costs
remain unknown when the harness does not expose them.

Unsupported numbering is retained explicitly: labeled suppressed equations remain candidates
with uncertain numbering, including standalone/starred environments. Numbering aliases withhold
inventory certification. Clipped link contexts and unsupported text rendering retain source
occurrences and member spans, while resource bounds still abort the paper. Mathematical
alphabets and script grouping require independently preserved semantics. Only the visually
identical micro-sign/Greek-mu encoding alias is folded in otherwise strict math alignment;
ordinary prose emphasis remains supported. Source payloads that are PDFs are reported as
unavailable TeX, rather than corrupt source archives.

For a separately reconciled complete equation/statement/proof/algorithm inventory:

```sh
python3 -B scripts/truth/latex/manual.py \
  --candidate k1-full-alignment-0910c35/papers/2503.05828.json \
  --object-bundle k1-manual-annotation/2503.05828
```

The object path uses exact direct membership plus separately owned footnote/child relationships.
It verifies both annotations and accepted reconciliation, source slices, reference roles, proof
attribution, actual serializer bytes, and all original/detail image paths. It preserves lossy
native math fidelity and does not claim semantic quote truth. It remains an external exploratory
candidate until final K1 publication passes its separate cohort requirements.

Add `--bibliography-bundle k1-manual-annotation/2503.05828` to the object command to
attach the separately reconciled bibliography. This validates the complete source bibitem and
citation inventory, exact printed memberships, explicit first-author/title/year source roles,
and both annotators' field anchors. Grouped citations retain printed order and source order
separately. Actual source/PDF/index, images, annotation serializer and review script hashes are
verified again. Only compact labels and provenance enter the overlay; full excerpts stay external.

`release.py --config <repository eval configuration>` reassembles all pinned manual bundles and
writes immutable `eval/truth/<k1-limited-version>/{objects,bibliography}.json`. A fixed configuration
must pin every consumed external document, both historical and current automatic build reports,
and the exact paper order. The limited release requires positive truth for every E1 kind and
complete metric cohorts, preserves supported negatives, and reports the unchanged 500-paper
coverage target with follow-up #97. The bibliography projection is accepted by the E1.2 collector
using its explicit `--k1` path. Release version, exact cohort size and selection bias must accompany
any detector measurement.

The explicit `k1-manual-visual-tranche-v1` format supports direct figures/tables and
separately reconciled formal negatives from the primary schema version 1 and
`lysilogy.blind-manual-inventory.v1` originals. Its manifest binds both inventories
and receipts, their real page-first histories, the native allowlist export, all
96-dpi original images, the original source export, and the root geometry/role
reconciliations. A later construction, source crosswalk and distinct reviewer
receipt bind those originals without changing their historical input ledgers.
Positive math/formal objects and overlapping source owners require another codec.

The visual crosswalk separately names each original `table_N_notes` ancillary group
and its `table_N_note_scope` reconciliation. The validator proves the full table
membership is the exact union of the independently recorded core, note text and
original superscript tokens. A marker can occur later in native reading order;
its exact native token and recorded note ownership remain required. Missing notes,
reused groups, changed source owners and cross-page membership fail validation.
The compact release retains separately owned attached notes and unknown external
locators. Visual/section/citation source occurrences are exhaustively crosswalked;
bibliography excerpts and field evidence remain external and O8–O11 are omitted
unless their separate existing codecs are supplied. Informal roles remain reviewable
and cannot silently disappear into a formal-negative cohort.

The planned `k1-limited-v3` selects a separate fixed 15-module implementation,
including this codec. Its pin remains unavailable until independent construction
and publication review. The v1 and v2 pins, fixed modules, payloads and configurations
remain unchanged; the collector defaults to v1 and requires explicit
`--truth-version k1-limited-v3` for the later cohort. Every version has a separate
observation path while one current figure/table metric input owns O1/O2; prior
input/observation pairs are preserved by the existing immutable history writer.

Historical automatic candidates retain their original bytes and confidence flags
as diagnostic history. The visual construction also records a fresh bounded parse
of the same deposited source and every changed top-level inventory field. Validation
recomputes that parse, checks exact source-occurrence correspondence, and uses its
current duplicate-label/raw-source guards for object and section destinations.
Each printed reference number is nonempty, belongs to its complete original phrase,
and cannot be reused by another source occurrence. Phrases may share context when
their independently recorded number spans remain distinct. Section destinations
must belong to the current parser's unique section heading; visual labels cannot
be reclassified as section labels. Current bibliography parsing differences stay
separate from the omitted bibliography metrics.

Producer independence is grounded separately from string distinctness. The visual
manifest retains the original assignment proposal, an audit of committed assignment
and freeze records, and a distinct reconciler's confirmation of those records against
both original inventories and receipts. Original role-only receipts remain unchanged.
The codec checks the actual retained history bytes and exact source excerpts, proposal
selection/prompt/output paths, complete confirmation hashes and final reviewer binding.
A proposed assignment or three different identity strings alone cannot enable an overlay.
