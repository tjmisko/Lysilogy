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
The published v3 visual tranche records each new single-page region as `{page, rect}`;
its earlier paper records retain region lists. Collector v6 interprets that exact singleton
shape only for explicit v3, applying the same page, coordinate and IoU checks as a one-item
list. It does not rewrite pinned truth. V1/v2 interpretation remains unchanged. Before
publishing a measurement, the count of O2 values must equal the selected frozen release's
`all_annotated_truth_objects` denominator; unsupported or missing geometry cannot silently
reduce a declared complete region cohort.

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
`figures:<version>:<native-etag>:graphics:<graphics-generation>`. Graphics version3 binds the
native ETag, exact PDF SHA256, canonical tool path/SHA256, page statuses, raw trace hashes,
accepted image placements and excluded-image counts. Its generation hashes the exact Rust JSON
serialization with the `generation` field empty; the collector retains those exact bytes and
checks their structural equality to the artifact. A separate cache key hashes JSON
`[3,native-etag,pdf-sha256,tool-sha256,mask-runtime]`, where the last item is
`[wrapper-path,wrapper-sha256,script-sha256]` or null. Existing native-only objects, changed source/tool bytes,
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

Mask support in graphics version3 is a separate bounded opacity observation. The original
XML remains the framing, group, clip and image-order authority. Only pages with excluded
image-mask operations invoke the fixed `graphics/masks.js` Device adapter. Its source hash,
canonical `prlimit` wrapper path/hash, and MuPDF hash enter the cache identity; source/tool/
wrapper hashes are rechecked after derivation. The optional Linux wrapper restricts the child
to768MiB address space,8 CPU seconds, no core file and16MiB regular-file output. Pipe output
is separately bounded, cancellation kills the child, and each mask command has a10s wall
limit shortened to the remaining30s paper budget. Missing wrapper/tool support remains explicit.
The existing `serde_json` dependency enables `float_roundtrip` (no new dependency) so
Device decimal coordinates decode exactly before strict native-f32 equality checks.
Its default binary64 parser shifted three valid page-3 values by one ULP in the first
experiment; the original failed support result is retained. This feature affects JSON
decoding globally, so full artifact/native-basis checks accompany the change. No epsilon,
coordinate narrowing or baseline target adjustment substitutes for exact equality.

No arbitrary PDF-supplied JavaScript executes. The trusted script arrives through seekable stdin from an unnamed, private regular file under
`~/.cache/lysilogy`. Existing `rustix` APIs open every cache ancestor without following symlinks;
`O_TMPFILE` creates no replaceable directory entry. The owner stays open through completion and
checks exact embedded bytes before/after. Unsupported file semantics leave masks unavailable.

The adapter decodes at most4million pixels per image and6million opacity samples per page,
including attached-mask verification, with512 ordered image/mask events and4096-pixel axis
limits. Rust binds every event ordinal, kind, exact finite f32 transform and dimensions to the
original XML sequence. New placements require exactly one active mask and identical image/
mask dimensions and transforms. The attached mask must have exactly the same complete decoded
samples and metadata. Both require no interpolation, color-key, decode mapping or orientation
ambiguity. The base image must independently decode to a pixmap with no alpha and a matching
1/3/4-component color space. Unmatched transparency cannot inherit mask-only evidence.

Every nonzero decoded opacity sample contributes its full pixel cell, including partial alpha.
The transformed tight envelope is a candidate rectangle; holes and disconnected interior are
not asserted painted. An empty mask admits no image. Any additional clip/crop, nested mask,
unsupported transform or missing pixel provenance keeps the image excluded. In particular,
intersecting the envelope of an irregular mask with a crop cannot prove that any pixels survive.
Native diagram/caption ownership remains governed by the existing detector.

Raw opacity receipts remain external under `~/.cache/lysilogy/object-graphics-masks/<sha>.json`;
only their hashes, evaluation status and admitted/empty counts enter graphics evidence. The
bridge retains exact bytes and the collector verifies runtime, path, hash, page, status and
count bindings. XML and opacity bytes share the64MiB paper allowance. Scoring, both immutable
truth cohorts and complete object denominators are unchanged. Independent synthetic opacity
fixtures are retained verbatim in `eval/fixtures/mask-support.json`; the test adapter produces
`mask-events.json` by running the exact trusted script with fake image APIs, preserving all
43 independently specified outcomes. Tests do not call MuPDF or any network/model provider.

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

Collector v6 accepts an explicit `--truth-version k1-limited-v1|k1-limited-v2|k1-limited-v3`;
v1 remains the default. The selected version fixes the configuration, retained
verifier manifest and module inventory, truth payload, build receipt, and output
paths. V2 writes `object-metrics-k1-limited-v2.json` observations. The harness
keeps one active `figure-table.json` input for the explicitly selected cohort.
Before replacement, the collector preserves the exact prior input and observation
bytes with a hash manifest under `eval/evidence/object-metrics-history/`; it also
freezes every new pair there. These histories sit outside the active input folder,
so overlapping releases cannot create duplicate O1/O2 owners.
All three versions use identical matching, arithmetic, geometry and native/detector
provenance checks. Coverage always identifies the selected immutable cohort;
results from the two releases must not be combined as disjoint samples.


## Contiguous table caption ownership (issue125, detector5)

A joined label/title is one exact native UTF-16 slice across two original paragraphs
separated only by at most four whitespace code units. Both paragraph pieces must be
contiguous and page-local. The new title path requires native provenance and valid
single-line glyph geometry on a complete native page; uppercase title text alone is
insufficient. Centered label/title placement and an observed grid with two numeric
columns sharing a row and preceding header/row evidence corroborate ownership.
Existing prose/caption/image boundaries and explicit heading/list/equation guards
constrain the grid. Unconfirmed extensions retain the prior label-only prediction if
one existed. Disjoint caption membership remains unavailable. The public anchor/body
member meanings, native index bytes, immutable truth, matching and IoU computation
are unchanged. The detector and collector's expected detector version advance together;
all four frozen cohorts must be measured and every caption/region delta retained.


## Vector candidate supplement (graphics5, detector6)

The optional vector path renders the complete original page at144 DPI with eight-bit
RGB antialiasing (`mutool draw -F pam -c rgb -r 144 -A 8`). It never seeds a body from
a path, clip, group or page bounding box. The existing strict image and mask parser
is unchanged. A separate trace mode permits finite `Normal` transparency groups,
including nonisolated/knockout groups, and requires actual curved paint, no images,
no unknown drawing state and only closed rectangular path clips; self-closing or unproved clips are unsupported. Original rendering
resolves paint order, opacity and compositing. It does not establish semantic ownership.

The first page command may be one self-closing `set_default_colorspaces` with exactly
`gray="DeviceGray"`, `rgb="DeviceRGB"`, `cmyk="DeviceCMYK"`, and `oi="None"`.
These identity defaults do not substitute a profile or output intent. Missing, extra,
duplicate or changed attributes, nonempty forms, repeated or late defaults, and defaults
inside drawing/group state withhold the renderer path. Graphics4 rejected even this
identity preamble in all23 measured traces (also present in all17 prior generated
fixtures), so its complete no-improvement run remains historical. Graphics5 invalidates
those unsupported cache records; candidate parameters and existing image behavior stay
unchanged. Acceptance of this preamble alone does not establish supported later commands
or a complete raster/component result.

All existing page image/mask work finishes before optional vectors use the remaining
30s/64MiB paper allowance. Each raster uses the hash-bound `prlimit` wrapper (768MiB
address space,5 CPU seconds, no core file,16MiB regular-file cap), at most5 wall seconds
shortened to the remaining paper time, and the shared bounded pipes/cancellation path.
There is no script or shell. Tool/PDF/wrapper hashes are rechecked after derivation.
The graphics5 cache key uses `[5,native-etag,pdf-sha256,tool-sha256,mask-runtime]`;
DPI, threshold, dimensions, component records, raster hashes and explicit statuses enter
the exact graphics generation. Prior generations cannot substitute for these bytes.

Only complete native pages without declared gaps can supply vector ownership. Every
non-whitespace native token needs finite, nonempty, page-contained geometry. At144
DPI, dimensions must exactly equal the rounded-up native page dimensions, with8192 pixels
per side and4,194,304 pixels total. Oversized pages do not select a lower DPI. At least
one RGB channel must differ from white by5 for a pixel to count. Eight-neighbor components
are formed over the full page before any exclusion;4097 components withhold the entire
new page result. Full pixel cells determine each component's integer bounds and count.
Raster scans and flood fill check the remaining paper deadline; no partial component
inventory is returned on failure. Near-white, very thin and subpixel omissions remain
limitations, as demonstrated by the fixed synthetic experiment.

Only compact components at most one median native font height in either extent are
considered. Whole components touching native glyph rectangles, recognized prose, caption
bounds or observed table grids are excluded. Complete groups use a1.5-font-height gap and
at least six components. The full group must cross no barrier and fit exactly one native
figure window. Windows use caption horizontal bounds, a5%-page top floor or preceding
barrier bottom plus4 points, and a35-point to70%-page height range. An overlapping preceding
table caption without an observed body withholds the new supplement. Combined groups and
the final union with the existing native body must also cross no known barrier. Image-backed
figures and table regions use their existing paths. A native glyph count above32768,
more than64 page captions or4096 barriers withholds this optional supplement. An explicit
upper bound of16,777,216 component exclusions, graph pairs and group/combined-envelope
checks applies across all eligible caption windows on the page. Repeated graphs cannot
multiply a per-invocation allowance. One graph has at most8,386,560 pairs. This count bound
applies to synchronous candidate work after the separately timed graphics stage.

These are candidate heuristics. Outlined prose absent from the native index can still look
like compact vector marks; its synthetic false candidate is retained. No semantic certainty,
truth eligibility or complete painted-interior claim follows from a successful raster.
The established source/native/annotation truth remains immutable, and every15/23/29/50
cohort outcome, zero, false positive and regression must be measured independently.

Exact full-page PAM bytes remain external at
`~/.cache/lysilogy/object-graphics-vectors/<sha256>.pam`. The source-backed bridge retains
them through the same safe immutable artifact channel as traces and masks. The collector
requires exact page/hash/path, fixed policy, native dimensions, PAM sample bytes, component
types/bounds/counts, status and aggregate resource accounting before scoring. The selected
Cargo producer owns component derivation; the collector does not rerun segmentation or
change matching, IoU, truth, denominators or targets. Raw raster failures are retained when
available, with no components admitted. Unknown trace, native, tool, resource and raster
states remain explicit; transient tool/time failures cannot be reused as successful caches.

## Inactive bounded per-paper transport

Issue #129 adds the mechanically checked, inactive v5 path described in
[k1-per-paper-contract.md](k1-per-paper-contract.md). It preserves v1–v4 replay,
truth bytes, metric definitions and targets. Activation and new truth admission
remain separate reviewed work; synthetic scale records are not K1 coverage.
