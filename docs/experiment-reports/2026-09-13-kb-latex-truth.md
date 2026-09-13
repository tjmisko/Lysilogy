# E8.3 independent LaTeX truth and limited K1 release

**`k1-limited-v1` is frozen**, with complete independently reviewed cohorts covering every E1
object kind. It contains two papers and preserves the original approximately **500-paper target
as unmet**, tracked by [follow-up #97](https://github.com/tjmisko/Lysilogy/issues/97). It establishes
no broad coverage or system acceptance. Both complete automatic runs accepted zero metric cohorts;
the limited release uses separately reviewed source/PDF annotations and retains those exclusions.

The committed [object truth](../../eval/truth/k1-limited-v1/objects.json) and
[bibliography projection](../../eval/truth/k1-limited-v1/bibliography.json) contain only derived
labels, membership/region coordinates, source positions, votes and hashes. PDFs, deposited source,
full reading-index text and review excerpts remain external. The fixed
[build configuration](../../eval/truth/k1-limited-v1-build.json) pins 33 external evidence documents;
module, candidate, image and artifact hashes are verified transitively as well.

## Implementation and source safety

The reader never extracts or executes TeX. It retains exact source members, full parsed object,
entry and link inventories, hashes, unsupported semantics and every omission. Unknown visibility,
local package programs, structural aliases, numbering ambiguity, mathematical scripts/alphabets
and ambiguous alignment withhold affected automatic cohorts. Clipped link contexts and unsupported
row rendering retain their source occurrences; resource bounds remain fatal. Known inventory
syntax does not imply faithful text rendering. See [the cohort contract](../../eval/latex-contract.md).

Manual validators preserve automatic results and require complete, independently accepted
reconciliation. They bind both annotations, actual source/PDF/index bytes, every used image,
serializers and review scripts. Source nesting, direct/ancillary membership, geometry, counts,
proof endpoints, field roles and reference roles must agree. Fields require exact balanced source
arguments and complete source/printed value agreement under explicit presentation rules; later
people cannot replace an unrecognized first author. Source math is never certified from flattened
native text alone. Caption token boxes never substitute for full figure regions.

The release writer reassembles every bundle, rejects symlinked output ancestors and refuses to
modify existing version bytes. It validates distinct historical/current source identities and
current automatic derivation equivalence, rehashes every retained candidate/inventory, and
recomputes all counts over the complete frozen population. Per-paper artifact hashes, mapped
PaperIds, indexes, arXiv versions and strata must match the original inputs.

## Corpus and native indexing

All 1,000 frozen eval PDF/source pairs were available. The offline native helper preserved the
existing dedicated `~/.cache/lysilogy/arxiv-kb-data` registry and mapped 1,000 unique PaperIds.
It produced **999 successful indexes** in **1,374.785 seconds**, with zero network/model calls.
Peak RSS was not instrumented. Root independently rehashed all 1,000 PDFs and 999 indexes:
6,171,809,461 bytes matched in 9.302 seconds.

The retained failure is `2308.05883v2`: three negative-width Poppler combining-circumflex rectangles
among 19,570 words. Independent visual/glyph inspection showed that swapping endpoints would
invent geometry. The finite/ordered coordinate gate stays unchanged. Follow-up #98 handles page
failure isolation separately; this frozen index run is not rewritten.

Native receipts: `~/.cache/lysilogy/k1-full-index-receipt.json`,
`k1-full-index-root-review.json`, and `layout-diagnostics/2308.05883v2/diagnostic.json`.
Helper executable SHA-256:
`9583e52411a0fc80466bfa4ad9f50d671d79ad8b218b9f4b5fccec506f7350c9`.
Helper source SHA-256:
`4cdce79da883df3fd656d5ba1f3dee3cbdc780532dab2f77587e20c664a4bf51`.
Full-index map SHA-256:
`40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`.

## Complete automatic coverage

Both runs used the same frozen 1,000 inputs and 999 valid indexes, one Python process, a 1.5 GiB
address-space cap and a 30-second per-paper diagnostic timeout. Every used PDF, source and index
was rehashed. Both exited 0 with zero network/model calls and $0 external-call cost.

| Run | Parsed | Accepted metric cohorts | Source/index failures | Runtime | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Historical `0910c35` | 626 | 0 | 374 | 1,017.604 s | 512,176 KiB |
| Current reviewed `9d3bd42` | 622 | 0 | 378 | 992.761 s | 470,192 KiB |

The current wrapper elapsed time was 992.856 seconds; the table uses the runner's measured time.
Current leading failures are duplicate object labels (88), unterminated arguments (39), legitimate
source payloads that are PDFs without TeX (39), crossing/unbalanced environments (22), no unique
deposited bibliography (19), and unsupported argument forms (19). Unsupported visibility and
rendering exclude all otherwise partially aligned candidates. The historical report also retained
pre-guard text-alignment diagnostics; these were never accepted cohorts and are not comparable
to the current report's positive eligible-kind counters.

All full candidates, summaries, copied modules, runner, launch and logs remain under
`~/.cache/lysilogy/k1-full-alignment-{0910c35,9d3bd42}/`. Root rehashed all 626 historical candidate
payloads (115,525,965 bytes) and all 622 current payloads (116,050,046 bytes), reproducing every
aggregate and source-inventory hash. Use the current `root-review-v2.json`: v1 had a mislabeled
historical diagnostic counter; its numerical values were unchanged. Report hashes:

- Historical: `8a21a70a94e132d6c42f96a487af45d5ad7deee3d4163675dac80ef3fcfb77e1`.
- Current: `24cf2c681e131575ed190ebaa9014c8ca055a7b00b0c044d4219cbe57140e1a8`.
- Current independent review: `314436ab2f0261d660ea5b2e860b900c796e5586c077bbe96cd84e5d4bb55708`.

## Limited manual cohorts

The release deliberately selects manageable complete papers in **cs.LG/2021** and **econ.TH/2025**.
This is substantial manual selection bias, not a representative stratified sample. Each metric
receives its whole independently reviewed paper/kind inventory; other cohorts remain excluded.
The release records excluded eval-paper counts for every metric and all frozen strata.

| Paper | Reviewed scope | Denominator |
| --- | --- | ---: |
| `2104.01511v1` | Figures / tables with full visual regions | 10 / 5 |
| `2104.01511v1` | Three independent top-three panel votes | 9 scoring opportunities |
| `2503.05828v1` | Equations / statements / proofs / algorithms | 1 / 4 / 1 / 2 |
| `2503.05828v1` | Equation/statement reference pairs (O4) | 10 |
| `2503.05828v1` | Complete bibliography entries | 35 |
| `2503.05828v1` | Known first-author / title / year labels | 35 / 35 / 35 |
| `2503.05828v1` | Citation groups / target pairs | 40 / 50 |
| `2503.05828v1` | Reviewed absent figures/tables | 1 negative paper |

Root and independent annotators inspected every original page and the deposited source inventory.
Original labels were frozen before detector predictions were used. The figure/table paper has a
separate accepted source-to-visual association review. A script-containing caption retains its
automatic exclusion while receiving manually reviewed identity and full-region evidence.

The math paper preserves eight objects, a separately owned footnote and nested floating algorithm
edges. Exact native memberships remain distinct from lossy math transcription: they are not
semantic quote truth. All 13 object references remain present; ten statement references enter O4,
three algorithm references stay outside it, and a reviewed section reference retains its source
and native provenance. The unnamed proof targets nearest-preceding Theorem 3.1, as required by the
confirmed E1.4 design. The bibliography preserves printed oddities, all 35 entries, 105 requested
field labels and footnote citation reordering. No provider Work/Person/identifier identity is inferred.

Three fresh blinded panel agents selected the same ordered top three source objects:
`object:tab:lateFusionResults`, `object:timeToEvent`, `object:shapPlot`. Aggregate agent wall time
was 313.431 seconds; the harness did not expose their dollar cost. No production enrichment or
ranking ran, so **O11 remains unmeasured**. Its denominator-nine policy was frozen before root
inspected any vote. Manual annotation time was not independently timed; hash/serialization times
are retained separately and are not annotation durations.

Original evidence is immutable under `~/.cache/lysilogy/k1-region-annotation/2104.01511v1/`,
`k1-panel-pilot/2104.01511v1-d75ddba/`, and `k1-manual-annotation/2503.05828/`.
At the final reviewed source, separate manual assemblies took 0.1234 and 0.1266 seconds.
The complete release validation/publication took **3.969 seconds**, with zero network/model calls
and $0 external-call cost. A second offline invocation reproduced both files byte-for-byte.

- Object release SHA-256: `0afccc35dc5eedb48b4df4e30ee33e06dac02a07f0b7cab6d6f5c53743202976`.
- Bibliography SHA-256: `a821d3070768175f2de7022928a6a19f6ef8cddce9edef726448447e24fc7c82`.

## Quality gates and scorecard delta

Final source/config checkpoint `c420782` passed formatting, strict Clippy with all targets/features,
all Rust targets, **144 offline LaTeX-tooling tests**, the production provider-budget simulation,
and objects/bibliography/scale/G5 eval checks. G5 independently executed **306 Rust / 254 Python /
85 Node tests**, with network isolation and model CLIs absent; its runner took 7.231 seconds.
Post-publication objects/bibliography/scale checks also passed. G5 remains passing and O30 remains
zero violations across 10,000 references. Detector objectives O1–O11 remain unmeasured by this
truth-builder issue; no fixture-derived objective value is introduced.

Baseline, intermediate and final receipts remain distinct. The original missing-TypeScript
baseline G5 failure was resolved by using the already installed local web dependencies, then
rerun successfully. Historical gates are retained in `eval/evidence/latex-truth-checkpoint.json`.
Final evidence is in `eval/evidence/latex-truth-release.json`, with command/log hashes and actual
counts. Eval dirty flags honestly include generated scorecard changes and subsequently published
truth files; implementation fingerprints and tested source identity are recorded separately.

The staged publication is a documented design deviation, not a reduced target. #97 owns expansion
toward approximately 500 independently supported papers and remeasurement on a new version.
Production O1/O2 measurement belongs to #96; bibliography field measurement also needs genuine
K2. Every detector score must retain this limited release identity and exact cohort size.
