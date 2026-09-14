# Bounded K1 per-paper transport, format 1

Issue #129 adds a release and measurement mechanism. `k1-limited-v5` remains
unpublished: its fixed manifest pin is `None`. It adds no annotation, admission,
detector change, or completed-paper claim. All 73 v1–v4 implementation, config,
and truth files remain immutable and use their existing replay path.

The new object root is a canonical JSON manifest (`schema_version: 2`,
`layout: k1-per-paper-v1`). Each ordered descriptor commits the exact paper ID,
arXiv ID/version, ordinal, metric eligibility, kind counts, child path, byte
length and SHA-256. Children live at `papers/<16-hex-paper-id>.json`. The
bibliography root selects descriptors eligible for both O8 and O10 from the same
complete children; it does not reinterpret bibliography fields.

Each child contains the existing complete manual projection. Unscored inventory,
unknowns, exclusions, source memberships, attached content and provenance survive.
A wholly ineligible selected candidate remains an ordered child with all flags
false and no independent alignment claim. Its full candidate is retained in
`unscored_inventory.retained_candidate`. Transport never promotes an eligibility
flag. Later annotation codecs may use the same projection seam; this format adds
no annotation codec.

O1–O11 cohorts and denominators are recomputed from each metric's own flags.
An O1-only record contributes no O2 values; an O3-ineligible equation contributes
no O3 denominator. Reviewed empty visual papers remain in the O1 cohort and can
expose false positives. `retained_papers` includes wholly ineligible rows;
`release_papers` and the unchanged 500-paper target exclude them. Global
`retained_object_counts` counts projected objects/entries across rows separately
from eligible positives. Opaque candidate inventories beneath ineligible rows
are not relabeled as independently counted objects.

## Fixed limits

| Boundary | Limit |
| --- | --- |
| Manifest, config, history result, compact receipt | 1 MiB each |
| Truth, prediction, decision child; harness input | 8 MiB each |
| Selected papers | 1,000 unique original eval identities |
| Truth / prediction / decision totals | 1 GiB each, independently counted |
| JSON nesting / structural token budget | 64 / 200,000 |
| Global O2 values, including zeros | 100,000 |
| Worker request / pipe scratch / stderr prefix | 8 KiB / 8 KiB / 256 KiB |
| Replay worker | 768 MiB address space; 60 CPU seconds; 90 wall seconds |
| Replay coordinator | 768 MiB address space; 90 × (papers + 2) wall seconds |
| Serial bridge call | One paper; 40 wall seconds; 8 MiB stdout |
| Collector coordinator | 768 MiB address space; 220 × papers + 540 wall seconds |
| New immutable file write | 20 GiB free plus the next file's bytes |

Coordinator and child may coexist: limits are per process, not a measured total
RSS claim. Allocator overhead, peak RSS, CPU and elapsed time require separate
resource measurement. These finite rejection limits are not 500-paper timing
promises. Original evidence retains its 32 MiB bound; corpus hashes stream in
1 MiB chunks with the existing 512 MiB PDF/source bound. Harness evidence hashing
uses an 8 KiB scratch buffer without changing hash semantics.

JSON byte/depth/token limits, duplicate keys and nonfinite numbers are checked
before use. Encoding is incremental and bounded. Subprocess transport drains
both pipes concurrently, rejects at limit + 1, checks deadlines around spawn,
wait and cleanup, and kills the owned process group even after leader exit.
Failure retains requests, bounded output prefixes, status and receipts. Complete
per-worker receipts remain external; compact commitments cross the caller's
response boundary. Full paper/native graphs do not accumulate across workers.

## Publication and replay

The explicit producer is `python3 -I -B scripts/truth/latex/versioned.py --publish
--config <reviewed-repository-eval-config>`. Only fixed application paths are
source-loaded. Isolated workers compile verified bytes directly: stale `.pyc`,
search paths, preloaded namespaces and arbitrary module paths cannot replace the
selected implementation. Future replay also requires the fixed v5 pin and its
exact retained 19-module bundle.

One history worker checks both automatic runs and original input order/index map
once per replay. Each following worker assembles one paper through the existing
manual validators and checks its evidence before/after. Complete original
evidence is checked again at the final boundary. Source, config, child and root
bytes are bound throughout. Renaming a historical receipt cannot establish a
fresh derivation.

Publication stages children serially in a distinct sibling directory and appends
a durable external ledger. Roots are formed only after every selected row is
present. Final verification recomputes roots and rejects missing, extra,
duplicated, reordered, redirected or changed children. Only then is the directory
renamed to its final release path. Failure retains staging and a failed ledger;
there is no subset publication or old-row fallback. A retry may reuse equal
staged bytes but reassembles and reverifies every row. Construction leaves the
activation pin disabled.

## Collection and evidence closure

The new collector calls the existing Cargo-selected bridge for one paper at a
time and retains each full response externally. Every manifest row gets a full
decision child, including formal-only and all-false rows. Existing native,
graphics, object-generation and executable checks remain. O1 micro F1 uses its
own complete cohort; O2 takes the median of all its declared values, including
misses as zero. No per-paper median, batch average, geometry fallback or outcome
selection is introduced. Targets .90/.75 remain unchanged.

One compact active input owns O1/O2. Each metric lists every committed truth and
decision child as ordinary hashed evidence, plus the observation. Altering a child
invalidates harness evidence; a manifest hash alone is insufficient. The compact
observation commits an external full-prediction manifest. Each measured decision
binds raw prediction, canonical inputs, object/graphics hashes and process receipt.
Before publication the collector replays truth again and rehashes children,
predictions, inputs, source and executable. Late failure retains raw generation
and ledger while preserving the active measurement. Existing immutable history
preserves old input/observation bytes and one active owner.

Synthetic 500-paper fixtures establish transport/arithmetic only. Real validation
must separately preserve all ordered papers and 15/23/29/50 outcomes of the
unchanged v1–v4 cohorts. Neither experiment establishes 500 defensible K1 papers;
issue #97 stays open.
