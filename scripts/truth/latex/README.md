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

The reviewed `k1-limited-v3` selects a separate fixed 15-module implementation,
including this codec. It retains the three complete v2 paper records and adds
2409.03655v1: six visual objects with separately reviewed formal negatives, for
29 visual objects across four papers. New O8–O11 eligibility remains omitted.
This limited release does not satisfy the 500-paper coverage target. The v1 and
v2 pins, fixed modules, payloads and configurations remain unchanged; the collector
defaults to v1 and requires explicit `--truth-version k1-limited-v3` for the later
cohort. Every version has a separate
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

The separate `k1-manual-visual-only-tranche-v1` codec supports an O1/O2 projection
when the same paper also contains unscored formal mathematics, procedures or
unresolved references. It cannot emit the formal-negative overlay or combine
with bibliography/panel overlays. Both original printed visual inventories and
every current syntactic visual source occurrence must be accounted for. A source
figure environment around a printed equation, or a reviewed nonrendered literal
empty-macro payload, stays explicit in the source-role ledger. It is never silently
dropped to manufacture a visual negative.

The crosswalk checks per-occurrence kinds/labels, complete original source caption
arguments, exact UTF-16 caption memberships, page ownership and independently
reconciled full-body geometry. The supported included-source case has one literal
input/include and three exact expanded source-map pieces, with both original
dependency records. Separately recorded table notes retain their source owner
and native spans through the visual-only overlay. They do not become statements
or independent metric objects.

An excluded printed nonvisual occurrence must bind the same source span, kind
and printed label in both originals. Its remaining float context admits only
the finite text/math/spacing commands listed in the codec, bare rule framing,
and unique earlier local zero-argument aliases built from that same grammar.
Additional environments, graphics, unknown commands, parameterized aliases and
redefinitions reject. An empty-wrapper exclusion requires one literal definition;
other definitions, dynamic assignments, conditional or grouped source scope are
unsupported. Deferred assignment primitives inside source macros also reject,
and the empty consumer cannot be used as an unexplained control-sequence argument.
The supported invocation must immediately follow only whitespace/comments after
the exact closing source span of a preceding independently paired visual. An
adjacent declaration, arbitrary prose, or unreviewed syntactic closing command
does not establish this boundary; unresolved preceding argument consumption
rejects. This deliberate restriction does not claim general TeX token ownership.
Rule checks run over the complete bounded alias composition, preserving adjacent
token effects. These checks constrain the recorded source role; they do not execute
TeX, prove arbitrary package semantics or certify mathematical rendering. The
distinct original-page/construction review and retained source limitations remain
required. Every attached note must have complete native token rectangles inside
its reviewed full body and membership disjoint from every caption. Missing native
geometry stays an explicit rejection, without inferred rectangles.

Every original top-level value and collection member is retained by its canonical
JSON pointer/hash, original role/ownership labels and scored/unscored disposition;
the unchanged original artifact supplies all nested source/native/fidelity fields.
Every current parsed object, reference/citation occurrence and bibliography entry
also has an explicit ordinal/identity/hash/source-role record. The original raw
inventories, additive corrections and later current-parser construction remain
separate. This retention does not adjudicate omitted O3–O11 labels.

The planned `k1-limited-v4` has its own fixed 16-module set and remains disabled
until separately reviewed construction and immutable publication establish its
manifest pin. It preserves the complete v1/v2/v3 bundles and prior four compact
paper rows. Explicit collector selection preserves both supported historical
region shapes and still requires every frozen O2 value, including zeros. Matching
rules and IoU arithmetic remain unchanged; the 500-paper target remains open.

`tranche_numbered_math.py` adds the separate
`k1-manual-numbered-math-tranche-v1` format for complete O3-only construction.
The fixed proposal contains all seven numbered equations in 2404.17771v2 and all
48 in 1911.08525v2. Exact original/source/native identity, finite source-scope
review, full body/number geometry, complete denominators, original producer
history and a distinct final construction review are required. Whitespace-only
body support is rejected. Native corruption remains explicit nonquote evidence.

Every other original/source role, unknown field, ownership endpoint and all
fourteen pending/source-failure dispositions remain unscored. The whole archive,
original encoding and complete source metadata are retained; a source/printed
identity review does not waive automatic source guards. No TeX executes. This
format cannot mix with another manual overlay, and it does not change the legacy
O3–O7 contract. See the [numbered-only contract](../../../eval/latex-contract.md#complete-numbered-only-construction).

V5 publishes the explicitly reviewed twenty-module inventory through the existing
per-paper producer, after separate source/module, concrete construction and actual
release clearance. Its eleven children retain the nine prior paper records exactly
and add these two complete O3 inventories. V1–V4 bundles remain immutable. The
release enumerates current source provenance while preserving prior paper
semantics, metric flags and outcomes. The 500-paper target remains open.
