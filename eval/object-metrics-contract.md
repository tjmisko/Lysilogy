# Figure/table measurement contract (O1/O2, v1)

This contract is frozen before looking at detector predictions. Only the independently reviewed,
complete K1 paper/kind cohorts are evaluated. The initial release is `k1-limited-v1`: two papers,
including one complete figure/table negative, and 15 independently annotated visual bodies.
It is a manually selected, limited cohort; the original 500-paper target remains unmet under #97.

O1 is micro F1 across figures and tables. A prediction may match a truth object only when its
kind and printed number agree, its page agrees, and its caption anchor overlaps at least half
of both its own non-whitespace UTF-16 membership and the independent truth caption membership.
Printed labels normalize only Figure/Fig./Table prefixes, ASCII case and surrounding punctuation;
Roman labels retain their exact Roman identity (I is distinct from Arabic 1).
No fuzzy caption, geometric or region-overlap matching is permitted. Caption membership comes
from exact spans in the frozen index, not detector captions or body-region boxes. All spans must
land on valid UTF-16 boundaries within the declared native page. If a prediction qualifies for
multiple truth objects, it is ambiguous and remains unmatched. For multiple predictions for one
truth, choose greatest caption-membership Dice score, then original prediction position for a
stable tie. Remaining duplicates are false positives; unmatched truth is false negative. Region
quality never selects the winning prediction. Prediction IDs need not be unique to be penalized.

O2 is the median IoU over **all independently region-annotated truth objects** in eligible O2
cohorts. Each missed detection contributes zero. A matched prediction with absent, nonfinite,
reversed, zero-area or out-of-page geometry contributes zero; it still retains its O1 detection
result. Independently annotated full visual regions are required; caption geometry never supplies
truth. Multiple truth rectangles use exact planar union/intersection area, separated by page.
Matched-only IoU is a separate diagnostic and cannot replace the objective denominator. Unknown
truth regions and excluded O2 cohorts are reported separately and never guessed. The O1 target
is 0.90; the O2 target is 0.75. Empty populations cannot create a passing metric.

The collector revalidates the immutable K1 configuration, original automatic reports and all
review bundles, reproducing the published projections in memory. It checks full per-kind
inventories/negatives, independent evidence hashes, source/PDF/index identity and mapped PaperIds.
An evaluation-only Rust bridge calls the production `load_cached` and
`ObjectsArtifact::from_reading_index` functions. It never scans a library, builds an index,
writes objects into the canonical data root, or invokes an extractor/model/network. These are
actual current production predictions derived from the exact frozen native indexes. Detector
version 2 recomputes captions and regions in `ObjectsArtifact::from_reading_index`; it never reads
legacy embedded `ReadingIndex.figures` as its predictions. The independent native generation
remains the SHA256/ETag of the original unchanged index. `figure_detector_generation` is SHA256
of UTF-8 `figures:<version>:<native-etag>`, and object cache reuse requires both that fingerprint
and the current detector version. Existing objects without these fields are rebuilt atomically.
The source reading-index endpoint retains its historical embedded figure records; current KB/API
objects use the refreshed factory. No canonical native cache is rewritten for measurement.

The bridge retains a compact `native-json-f32-v1` commitment to every typed native field,
excluding only the unused embedded `figures`. The full original index SHA256 and native schema6
remain mandatory. The independently implemented protocol begins with UTF-8
`lysilogy-native-basis-v1` plus NUL. Tags are `n` (null), `f`/`t` (booleans), `i` (integer),
`r` (f32), `s` (string), `a` (array), `o` (object). Integers use canonical signed decimal ASCII;
integers and exact UTF-8 strings carry an unsigned 64-bit big-endian byte length. Arrays and
objects carry an unsigned 64-bit big-endian element/member count. Object keys sort by UTF-8
bytes and are encoded as strings before their values. Floats use their four big-endian IEEE754
bytes, retaining signed zero; nonfinite or inexact non-f32 typed values fail. Nesting is bounded
at64; integers must fit signed64 or unsigned64. Python accepts the original schema6 serializer's
shortest round-tripping f32 decimal, but rejects extra unsupported f64 precision/overflow.
Shared independently specified vectors cover framing, Unicode, bool/int separation and float
boundaries. All native field mutations are checked; original index/PDF/source hashes still bind
exact stored bytes. The commitment avoids copying megabytes of native text into every response.

The collector checks the derived detector version/generation and computes the same commitment
from the frozen index. This replaces the v1 collector's historical-detector source-equality guard
with explicit current production derivation evidence; scoring and denominators remain unchanged.

Every measurement invokes Cargo and verifies its selected compiler artifact and current source
hashes; editing a saved build receipt cannot substitute an executable. The original registry,
PDF/source/index, executable, source and truth bytes are hashed before and
after the run. Canonical registry records must be active, unambiguous, belong to the declared
corpus root and agree with each source hash and path. Changed truth, source, index generations,
registry or evidence fail before publication. External raw artifacts stay in the dedicated cache;
only compact observation identities/counts/IoUs/hashes and collector inputs enter the repo.
Tests use synthetic arithmetic/provenance fixtures only; they never register objective values.
