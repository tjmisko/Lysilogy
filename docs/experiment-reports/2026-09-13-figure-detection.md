# Current figure/table detection and geometry (#101)

The current production objects path reaches **O1=1.0** and **O2=0.9107793204** on the
unchanged `k1-limited-v1` cohort. Both exceed their original 0.90/0.75 targets. All ten
figures and five Roman-numbered tables match; the independently reviewed negative paper
still produces no figure/table objects. The previous false Figure 2 prose caption is gone.

This is two manually selected papers (cs.LG/2021 positive, econ.TH/2025 negative), not
representative 500-paper validation. [#97](https://github.com/tjmisko/Lysilogy/issues/97)
retains that coverage target. No truth, caption matching threshold, region denominator,
metric target, canonical PDF/source/index, or identity registry changed.

## Measurements

| Checkpoint | TP/FP/FN | O1 F1 | O2 median, all 15 regions | Disposition |
| --- | --- | ---: | ---: | --- |
| Original production, clean 9dc3f27 baseline | 10/1/5 | 0.7692307692 | 0.2448506858 | Both objectives missed |
| Native-label iteration 6180f0a | 15/0/0 | 1.0 | 0.0 | Exploratory; existing wide-diagram fixture failed |
| Reviewed source-backed iteration 0722e9a | 15/0/0 | 1.0 | 0.9107793204 | Both objectives at target |

The original matched figure boxes contained all annotated body pixels but were 1.80–9.13
times too large. Tightening to observed native labels/cells exposed a separate limitation:
eight raster plots had no native labels at all. That intermediate regression and failed
`cursor_objects` log remain retained. The final iteration adds independently bound PDF
image placements and fixes wide labels, distant numeric columns, and neighboring-float
ownership. The original cursor fixture now passes without weakened assertions.

Every truth object remains in O2's 15-value denominator, including zero for absent/invalid
prediction geometry. There are zero unknown truth regions. One-to-one matching still uses
kind, printed label, page and independent non-whitespace UTF-16 caption membership; region
overlap never chooses a match. The matched-only diagnostic now equals the all-truth median
because all 15 identities match. The Rust eval result differs from the collector's full
precision O2 by one final binary rounding unit; both round to 0.9107793204 and are retained.

Residual per-object geometry remains explicit: Figure 9 has no region (IoU 0), Figures 6/8
have IoUs 0.5542/0.5256, and Figure 10 is 0.7456. All five tables exceed 0.915. Four positive-paper
pages have complete supported traces; two are explicitly unsupported and use native fallback.
Next work should diagnose those unsupported trace forms and separate nearby native-only
float membership, then verify improvements on the expanded independent cohort. Passing a
median does not mean every region is correct.

## Production and provenance

The API and evaluation bridge share asynchronous `objects::from_source` derivation. The
source-aware product cache binds detector version 2, native ETag, exact PDF SHA256, graphics
version 1, and tool path/SHA256. Old native-only objects are rebuilt; unchanged cached native
text, tokens, geometry, OCR/gap provenance and UTF-16 anchors stay untouched. The legacy
reading-index endpoint still exposes its historical embedded `figures`; the explicit
native-only factory remains available to callers without source access.

Optional `mutool` traces provide page-space image placements without rasterization. The
parser accepts bounded opaque axis-aligned/reflected/quarter-turn images on the exact native
page; it excludes active clips and withholds masks/groups/unknown state. Missing or failed
tools retain explicit status and native fallback. Native/caption ownership prevents bitmap
tables and adjacent figures from sharing a body. Resources are bounded to one worker,
64 page traces, 16 MiB/page, 64 MiB/paper and 5 s/command with a 30 s paper scheduling limit.

The measurement retains six immutable traces totaling 2,498,853 bytes outside the repository.
Raw trace hashes, PDF/tool/native identities and exact graphics generation bytes are checked
before scoring. A separately reviewed typed native commitment covers every native field
except unused historical figures, preserving exact original index SHA256/schema 6 while
avoiding a multi-megabyte native-text response per paper. The protocol has independent
cross-language vectors and sampled real-index verification. The collector invokes Cargo,
verifies its selected compiler artifact, and rehashes all source/canonical inputs afterward.

Final source: `0722e9a816a1be1a583f2484c792a62c59fb5abb`.
Final main integration and gates: `281785b4a2a8fe5b83005c8f311581e86b5924bd`.
All 79 measured implementation fingerprints are identical across those checkpoints.
K1 SHA256: `0afccc35dc5eedb48b4df4e30ee33e06dac02a07f0b7cab6d6f5c53743202976`.
Prediction SHA256: `26c249cd3f551af0923c78aef734978408fbcaa17ce260507e46447ef8776775`.
Executable SHA256: `f4e49c2b6822d621e6f626a00673ab23ecc292e69b3e6609aca1184181cb6e11`.

Actual graphics measurement: 8.1600 seconds, 277,840 KiB maximum RSS, **$0**, zero network/model
calls. This is local detector cost, not model-enrichment R4 evidence. The earlier native-only
exploration took 6.1976 seconds; the clean baseline's cold bridge build took 94.3848 seconds and
its collector 5.0696 seconds. These timing scopes differ and do not establish a latency score.

## Verification and retained evidence

All required integrated gates pass: formatting, strict all-target/all-feature Clippy, all
Rust targets, 28 collector tests, provider-budget simulation, `eval objects --check`,
`eval scale --check`, and isolated G5. G5 ran **350 Rust, 282 Python and 85 Node tests**, with only
loopback in its separate network namespace, failed external-network probe, and model CLIs
absent. O30 remains 0 violations/10,000 simulated references. Web source was unchanged.

Added 30 Rust and 4 Python regressions cover literal Roman captions, prose continuation, disjoint
members/page ownership, connected wide labels, remote numeric columns, image/table isolation,
cache generations, malformed trace framing/state/dimensions, resource bounds, typed native
commitments, and external graphics receipts. Twenty independently authored trace cases/variants
are included unchanged. Separate source and compiled reviews cleared the final implementation; root independently
recomputed every actual caption membership and IoU;
PR/evidence review is recorded separately after this report is committed.

The generated local scorecard now shows G5 passing and O1/O2/O30 at target: **1/5 hard gates,
3/30 objectives**. Other unavailable gates/objectives remain unavailable; this is not Phase A
or system acceptance. Automatic baselines improve to 1.0/0.9107793204 without any target reset.

[Machine-readable evidence](../../eval/evidence/figure-detection.json) retains exact before,
exploratory and final receipts, all gate/G5 logs, source hashes, build/executable, review
receipts and immutable external paths. [Exact observations](../../eval/evidence/figure-detection-observations.json)
retain every caption match and IoU. Generated eval results mark the tree dirty because their
tracked scorecard/baselines changed; the separately retained implementation hashes all match
the committed tested source. No PDFs, LaTeX sources, raw native text or trace bodies are committed.
