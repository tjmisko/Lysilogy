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
An evaluation-only Rust bridge calls production `load_cached` and the shared asynchronous
`objects::from_source` factory. It never scans a library, builds/rewrites a native index, writes
objects into the canonical data root, or invokes models/network calls. The current production
objects API uses the same source-backed factory via `load_or_build_from_source`; the explicit
`ObjectsArtifact::from_reading_index` factory remains a native-only fallback for callers without
source access. Neither factory reads legacy embedded `ReadingIndex.figures` as predictions.
The source reading-index endpoint retains those historical embedded records unchanged.

Native generation remains the original full index SHA256/ETag. Native-only detector generation
is SHA256 of UTF-8 `figures:<version>:<native-etag>`. The source-backed generation hashes UTF-8
`figures:<version>:<native-etag>:graphics:<graphics-generation>`. Graphics version2 binds the
native ETag, exact PDF SHA256, canonical tool path/SHA256, page statuses, raw trace hashes,
accepted image placements and excluded-image counts. Its generation hashes the exact Rust JSON
serialization with the `generation` field empty; the collector retains those exact bytes and
checks their structural equality to the artifact. A separate cache key hashes JSON
`[2,native-etag,pdf-sha256,tool-sha256]`. Existing native-only objects, changed source/tool bytes,
old detector versions and invalid graphics generations are rebuilt in the product cache.
Transient tool/resource failures remain explicit and are retried; unavailable tools retain
native-only geometry, with a new generation when the tool becomes available.

Optional graphics extraction uses `mutool draw -F trace -r 72 -N -L -m 134217728 -q -o -`
on caption-bearing pages only. It does not rasterize. A single worker permits at most64 page
traces,16MiB per page/64MiB per paper,5s per command/30s before starting another page; the
underlying command is killed on cancellation. Source/tool hashes are checked again afterward.
Unsupported or failed pages preserve explicit status plus the untouched native fallback.
The parser requires one requested page with finite, zero-origin dimensions matching its native
page within0.01 PDF point. It admits only opaque self-closing image operations with positive
intrinsic dimensions and finite axis-aligned/reflected/quarter-turn page-space unit-square
matrices wholly inside the page. Pixel dimensions never scale the placed rectangle. Graphics
version2 recognizes only full-page, isolated, non-knockout `Normal` groups with alpha1 and
`actualtext` metadata around text operations. Matrices remain in page space; group scopes must
balance their own clips without consuming an enclosing clip. A single explicitly closed
axis-aligned rectangle under `clip_path` supplies exact clipping bounds; nested rectangles
intersect an image's placed bounds until their matching `pop_clip`. Arbitrary paths, stroked
clips, text clips and image-mask clips still exclude affected images; soft masks, unsupported
groups/tiles or unknown drawing state withhold all page images. Text/glyph/path records provide
no inferred painted graphic bounds. The installed MuPDF Device reference describes the image
unit rectangle, clip stack and transparency-group semantics used by this bounded extension.
DTD declarations, malformed
framing, duplicate attributes, oversized tags/depth/inventories are rejected. Independently
specified synthetic traces cover these boundaries without consulting truth regions/predictions.

Image tiles seed the nearest compatible caption neighborhood; separately owned columns and
captions cannot be joined. Native labels extend a bounded connected neighborhood; table bands
stop before following image placements. These are detector heuristics, never truth matching.
Detector version3 additionally recognizes repeated numeric-column grids above captions and
requires multiple printed rows before a below-caption grid is admitted. A short same-column
unfinished reference sentence can reject a false caption without changing the separate
multi-line prose barrier used for diagram bounds. Original native schemas, both K1 versions,
caption matching and all-region denominators remain unchanged.
Partial raster inserts cannot replace a separately established diagram extent: at least three
complete caption-owned native labels must enclose the image seed, including whole labels above
and below it. Every label contributing to that native seed must contain at most twelve tokens,
no digits, and glyph heights below85% of the page's median body height. This conservative cue
uses the existing native connected bounds and padding; numeric grids, ordinary body text and
one-sided label evidence retain the image-only boundary. Unknown mask pixels remain unavailable.
The bridge retains raw traces immutably only under the dedicated external
`~/.cache/lysilogy/object-graphics-traces/<sha256>.xml` root, rejecting symlinked ancestors and
conflicting existing bytes. The collector checks every retained trace path/hash, source/tool
identity, native page inventory, finite placement geometry and evidence generation before
scoring. Raw traces/PDF text never enter the repo or native cache. Full response size remains
bounded at16MiB; per-paper compact native commitments preserve larger cohort capacity.

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

Collector v5 accepts an explicit `--truth-version k1-limited-v1|k1-limited-v2`;
v1 remains the default. The selected version fixes the configuration, retained
verifier manifest and module inventory, truth payload, build receipt, and output
paths. V2 writes `object-metrics-k1-limited-v2.json` observations. The harness
keeps one active `figure-table.json` input for the explicitly selected cohort.
Before replacement, the collector preserves the exact prior input and observation
bytes with a hash manifest under `eval/evidence/object-metrics-history/`; it also
freezes every new pair there. These histories sit outside the active input folder,
so overlapping releases cannot create duplicate O1/O2 owners.
Both versions use identical matching, arithmetic, geometry and native/detector
provenance checks. Coverage always identifies the selected immutable cohort;
results from the two releases must not be combined as disjoint samples.
