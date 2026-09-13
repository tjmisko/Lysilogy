# Independent K1 alignment and metric cohorts

K1 comes from frozen arXiv eval PDF/source receipts. Source structure supplies identities,
memberships, labels and destinations; native PDF text supplies locations. Production object,
figure, paragraph and citation predictions never supply labels or cohort selection.

Each candidate retains its original category/year stratum, source/PDF/index hashes, actual
mapped PaperId, source membership, unsupported semantics, all expected/aligned counts, and every
exclusion reason. `alignment.overall_completeness` is the fraction of all independently parsed
objects, bibliography entries and link targets that aligned. It is retained even when low.

`eligible_kinds` and `metric_eligibility` are explicit exhaustive cohorts. A missing member never
shrinks a metric's denominator: the entire paper/kind is excluded from that metric. Source
semantics that could hide inventory, unknown object environments, or duplicate source claims on
one PDF span conservatively exclude every metric. Per-kind support may become more precise only
with an independently reviewed capability boundary.

| Metric | Complete inventory required from a paper |
| --- | --- |
| O1 | All figures and all tables, including independently supported empty inventories |
| O2 | Separate independent full-region annotation evidence; captions do not establish regions |
| O3 | Every numbered equation |
| O4 | Every source reference to an equation/statement and every target kind; unresolved target roles exclude the cohort |
| O5 | Every theorem-like statement |
| O6 | Every proof, its independently named or nearest-preceding source statement destination, and the statement inventory |
| O7 | Every algorithm |
| O8–O10 | Every bibliography entry and every source citation occurrence/target pair |
| O11 | Separate three-agent panel evidence over an independently frozen figure/table sample |

Exact, unique, fully rendered source/PDF matches have quality 1 under the declared alignment
criterion. This is a deterministic criterion, not a calibrated probability. The threshold remains
0.95; incomplete per-kind inventories are excluded even if their fraction exceeds that threshold.
An accepted paper contributes only to its explicitly eligible metrics. O2 and O11 remain false
until their separate evidence exists. Field labels remain unknown unless deposited field-role
markup independently establishes them; unknown labels and comparisons are counted explicitly.

Supported zero-object papers remain negative detection/precision cohorts, so their false positives
are counted. Empty source inventories require independent document-text alignment. An unrecognized
command or source semantic that could inject objects prevents an empty-inventory claim; an
unsupported bibliography printer is not an empty bibliography. At least one real positive example
of each required kind remains necessary for final K1 coverage; negative cohorts cannot satisfy it.

The bibliography projection preserves the schema-1 contract in `bibliography-contract.md` from
E1.2: only papers with `bibliography_eligible=true`, all independently identified entries, exact
half-open UTF-16 member spans, and every printed citation-target pair. Unknown title/first-author/
year values are omitted, never inferred by the detector being evaluated. Object collectors must
likewise select their declared metric cohort before reading detector outputs.

Publication freezes cohort paper IDs, eligibility, input/source hashes and label hashes before
any detector evaluation. The reproducible seeded order alternates categories, then years within
each category. Exploratory partial inputs remain external; they cannot be relabeled as final K1.
The complete build must retain the approximately 500-paper target, every E1 kind, documented
category/year coverage, omitted cohorts and reasons, and independent panel/region evidence.

O11 scoring is frozen before any detector ranking or panel vote inspection: average
`|model_top3 ∩ panelist_top3| / 3` across all three independent panelists and every frozen paper.
The denominator is nine opportunities per paper. Each panel vote has exactly three distinct valid
source IDs. E1.7 may produce three to five ordered choices; only its first three ranked positions
count. Duplicate/invalid IDs and missing positions receive no extra credit and are never backfilled
from rank four or five. A missing model result contributes zero with its full denominator.
Malformed or ambiguous rankings fail the collector. A consensus rank may be shown for display,
but it is not used to score O11. The target remains at least 0.70.

Manual figure/table evidence is a separate overlay over an immutable automatic candidate.
`manual.py` rehashes the frozen PDF/source/index and every original page render, then requires
an independent complete-region review and a separately reviewed source-to-visual association for
every source figure/table. The overlay never changes automatic confidence or exclusions. A
specific reviewed script-containing caption can receive manual evidence while its automatic
script-binding exclusion remains. Unknown source inventories cannot be repaired by caption boxes.
Three distinct blind panelist identities and exact vote/prompt/packet receipts are required;
identical rankings from independent agents are valid. The original panel receipts have two explicit
page-hash field spellings; the adapter accepts agreeing aliases and rejects conflicting duplicates.
An exact packet hash binds paper identity when the receipt omits its redundant arXiv ID.
Manual assembly remains exploratory until the final stratified publication requirements pass.

O1 uses one-to-one matching by kind and independent caption identity/membership. Region overlap
never chooses a match. Every unmatched prediction, including duplicates, is a false positive;
every unmatched truth object is a false negative. Matching must freeze deterministic ambiguity
and tie handling before prediction inspection. O2 is the median IoU over **every annotated truth
object**. A missed object, absent region or invalid predicted page/rectangle contributes zero.
The matching chosen for O1 is reused for O2, so a prediction cannot be selected for favorable
region overlap. Rectangles use one-based pages and PDF points from the unrotated CropBox's
upper-left corner; invalid coordinates never become repaired boxes. Multiple rectangles use
geometric unions per page before intersection/union area, avoiding overlapping-area double
counting. Matched-only IoU may be reported separately as a diagnostic, with its denominator.
A supported empty paper contributes O1 false-positive opportunities; it adds no annotated object
to O2, whose empty overall cohort is unavailable rather than a passing measurement. Targets
remain O1 F1 at least 0.90 and O2 median IoU at least 0.75.

Proof targets follow the confirmed E1.4 design: explicit named targets take precedence; an unnamed
proof links to the nearest preceding statement. Truth records whether the association comes from
an explicit source label, source structure, or separately reviewed source/PDF evidence. An
unresolved optional heading remains unlinked rather than being overridden by proximity. This
corrects the earlier overly narrow named-only truth contract; targets and denominators are unchanged.

The manual object overlay follows the same independent-review boundary for equations, statements,
proofs and algorithms. Both original annotations remain immutable. A separately accepted comparison
binds both hashes, complete source IDs, object-reference source occurrences, and any reviewed
non-object reference roles. Assembly rehashes source/PDF/index bytes, the exact annotation serializer,
and every original/detail image path. Direct UTF-16 members and their native-text hashes remain
separate from semantic transcription: lossy mathematical text is explicitly not quote truth.
Source-parent/child edges preserve floating algorithms; ancillary footnotes are separate members,
with no overlapping direct ownership. Full expected reference roles remain auditable even when
only equation/statement references contribute to O4. All proofs retain explicit or nearest-preceding
source association provenance. The overlay grants only its completely reviewed kinds, does not
change the automatic candidate, and cannot itself satisfy stratified K1 publication.

Manual bibliography evidence uses a separately accepted complete comparison. Every independently
parsed entry and citation command must survive, with one target pair per printed destination.
Both annotations and comparison agree on complete UTF-16 members, printed numbering, all known
field values and their source-role/native anchors. A field outside its entry or a changed citation
role/order cannot enter the cohort. This initial manual adapter requires all three requested fields
to be explicitly known; unsupported or unknown-field bundles are refused, not silently shortened.
The resulting schema-1 bibliography projection keeps all entries and mentions for O8–O10, while
provider identity and full mathematical transcription remain outside these labels.

Staged publication may create `k1-limited-v1` once every E1 kind has positive, independently
accepted truth, even while the original approximately 500-paper target remains unmet. Follow-up
[#97](https://github.com/tjmisko/Lysilogy/issues/97) owns coverage expansion and remeasurement.
The versioned release records historical and current full automatic coverage, exact metric
cohorts/denominators, all excluded candidate evidence hashes, strata and manual selection bias.
It makes no system-acceptance or broad-generalization claim. Complete reviewed absent kinds
remain negative examples. A release writer reassembles every original bundle and rehashes all
pinned evidence before publication; existing version bytes are immutable. Only derived object
memberships/regions, field labels, source positions, votes and hashes are committed. Full native
text, deposited source and review excerpts remain in the external cache.

Historical replay selects the verifier belonging to the release version. For
`k1-limited-v1`, twelve exact reviewed modules and a fixed hash manifest live in
`eval/implementations/k1-limited-v1/`. An isolated worker compiles only those
verified source bytes and reproduces both original payload hashes from their
original evidence. It ignores current parser modules and cached bytecode and
never rewrites the release. Collector receipts retain the selected manifest,
module and evidence hashes before and after measurement. Unknown versions fail
explicitly; a new version requires its own reviewed construction and selection.

Coverage expansion distinguishes known inventory effects from faithful rendering.
Finite standard Greek/math atoms, font sizes and line breaks cannot introduce
objects; an unsupported glyph, script, or font representation still withholds
the enclosing text from alignment. Arbitrary hooks, local programs and structural
redefinitions retain their exclusions. Structural tokens passed as custom-macro
arguments do not establish a live inventory: the macro may discard, duplicate, or
reorder them. The reader retains their original invocation/argument/object spans
as unsupported evidence. Nested custom argument forwarding, including tail calls
to standard stored/literal consumers that may consume additional caller tokens,
remains unverified rather than being
expanded. An explicit standard argument-role registry likewise withholds structural
tokens in URLs, keys, filenames, field roles and stored title/author/date content;
these slots cannot certify immediate visibility or multiplicity. Visible `href`
text and ordinary formatting arguments keep their separate rendering contract.
Unimplemented literal delimiter forms remain unsupported. Argument inspection has
cumulative byte/step limits and treats a star following a control word as a
separate TeX token. A direct equation boundary alias is supported
only when one global preamble `newcommand` or plain `def` has no arguments and its
entire body is one literal `begin` or `end` for a known equation environment.
Events retain the original invocation and definition positions; source text is
never rewritten. Scoped, repeated, indirect, parameterized and executable variants
remain unsupported, and equation row/numbering guards still apply.

Known mathematical array/alignment layout can preserve a source object inventory
without proving its rendered text. Array column declarations, row grouping and
matrix binding are not ordinary prose; objects, entries or contexts containing
these layouts retain an explicit fidelity exclusion until independently verified
geometry is available. This does not exclude another completely aligned kind.

An explicit References/Bibliography heading with content outside parsed entry
containers, or unmarked content inside a bibliography container, prevents a
negative bibliography/citation claim. A formal section includes all descendant
headings until the next sibling or ancestor; heading syntax itself is separate
from entry content, so a nested, completely parsed bibliography remains usable.
Likewise, a procedural heading or a
step-labeled list outside a parsed algorithm container prevents an algorithm
absence claim. These source-role exclusions retain original spans and affect the
relevant metric cohorts; a complete independent kind elsewhere remains usable.

The AMS `DeclareMathOperator` capability accepts a single top-level preamble
declaration following an explicit `amsmath` or `amsopn` package load. Its new
control word must not replace an existing primitive, environment entry/exit
command, or deposited definition. Names beginning with `end` are forbidden by
LaTeX's declaration contract. The
its complete body is bounded literal letters/digits/spaces and standard spacing
tokens. Braced and one-token names and the standard starred variant preserve
their original source spans. Scoped, conditional, repeated, deferred, dynamic,
and structurally active declarations remain unsupported. The resulting operator
is known only to have no object-inventory side effect; its mathematical font,
limits and text rendering remain unverified. This does not certify any local
package or style implementation.

This finite contract was checked against installed `amsopn.sty`, whose
`DeclareMathOperator`/`@declmathop` definitions create a new zero-argument
operator, with the star selecting limits behavior and a preamble-only guard.
The inspected file SHA-256 is
`c50239a5acc1037ff4df482878e5546be3cd9b8ac725151bc14748a103d800a8`.
The implementation reads no installed TeX files at runtime and executes no TeX.
