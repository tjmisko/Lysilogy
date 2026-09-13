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
actual production predictions in the exact frozen native indexes, wrapped by the current objects
implementation. The current figure-detector source must equal the source used for those caches;
a detector change requires separately reviewed new-generation measurement, not stale cache reuse.

Every measurement invokes Cargo and verifies its selected compiler artifact and current source
hashes; editing a saved build receipt cannot substitute an executable. The original registry,
PDF/source/index, executable, source and truth bytes are hashed before and
after the run. Canonical registry records must be active, unambiguous, belong to the declared
corpus root and agree with each source hash and path. Changed truth, source, index generations,
registry or evidence fail before publication. External raw artifacts stay in the dedicated cache;
only compact observation identities/counts/IoUs/hashes and collector inputs enter the repo.
Tests use synthetic arithmetic/provenance fixtures only; they never register objective values.
