# Evaluation, corpus and evidence status

[Back to guide](README.md)

Snapshot: 2026-09-15, from main `4972d63f10efdc2a89d01901b1adc0624408e868` and the retained branch evidence identified below. This document consolidates existing results. No benchmark, model call, corpus download or application acceptance test was rerun for this handoff. Selected receipt hashes and published truth files were checked while writing it.

The evaluation infrastructure and corpus preparation work. The complete knowledge-base system has **not** passed acceptance: the committed scorecard has **1/5 hard gates passing and 3/30 objectives at target**. Most remaining values are unavailable because the required implementation, independent truth or collector is missing. Unavailable is neither a measured failure nor a pass.

See the [handoff index](README.md), [canonical scorecard](../../kb-scorecard.md), [execution plan and session log](../../knowledge-base-phases.md), and [evaluation interchange contract](../../../eval/README.md).

## What each stage proves

```text
Downloaded PDF + source archive       K0: input exists and its identity is verified
                |
Native PDF index                     text spans and page geometry are available
                |
Independent source/PDF annotation    K1: expected objects and exclusions are reviewed
                |
Current detector + metric collector  predicted objects are compared with that truth
                |
Evaluation harness                   score, baseline and regression checks
                |
Running application acceptance       complete user workflow and real 10k scale scenario
```

Each stage has a separate success condition. A downloaded paper is not annotated truth. A valid truth release does not establish detector accuracy. A passing fixture suite does not establish a working application workflow.

## Corpus: complete inputs and native indexes

| Population | PDFs | LaTeX source artifacts | Native indexes |
| --- | ---: | ---: | ---: |
| Frozen eval tier | 1,000 | 1,000 | 1,000 |
| Frozen scale tier | 10,000 | 49 overlapping eval papers | 10,000 |
| Unique union | **10,951** | **1,000** | **10,951** |

The tiers overlap by 49 papers. The 11,951 PDF/source artifacts occupy **46,892,682,976 bytes**, excluding metadata, indexes and receipts. Corpus content remains only under `/home/tjmisko/Corpora/arxiv/`; the separate mapping data root is `/home/tjmisko/.cache/lysilogy/arxiv-kb-data/`. No PDFs or deposited sources were committed or copied into the user's library.

The production download verification checked content hashes, byte lengths, versions, selection and availability evidence. The independent metadata audit checked all 11,951 sidecars. Downloads completed on September 13. Native preparation completed on September 14 after the final 201-paper batch; earlier failures and reviewed recoveries remain in the record. No download or native-index queue remained at the last implementation checkpoint.

Evidence:

- [Complete corpus report](../../experiment-reports/2026-09-13-kb-corpus-complete.md). Its final section describes the then-incomplete native preparation; the later audit below supersedes that status.
- Final native audit: `/home/tjmisko/.cache/lysilogy/scale-index-queue-v3/native-index-after-batch40-review.json`, SHA-256 `c043dca8a0c3b949d8bcdb38bd1260b55c589e4a4b0f48d5eee28a3107c462df`.
- Final native batch receipt: `/home/tjmisko/.cache/lysilogy/scale-index-batches/batch-0040/receipt.json`, SHA-256 `172bad0fa05210d5c9c4e07234208327cb55fa4d12c7ff26010a0b309d6795c6`.

**Still missing:** the production application scenario on the 10k tier, including measured rescan, Home/search, batch efficiency and the final report. Index preparation is not an O25/O27 benchmark or Playwright acceptance result. Do not restart completed downloads or invent a next native batch.

## Truth sets: what exists and what is missing

The intended populations are defined in the [design plan](../../knowledge-base-plan.md#ground-truth-sources).

| Set | Purpose | Current usable result | Missing work |
| --- | --- | --- | --- |
| K0 | Real arXiv inputs for evaluation and scale | Full eval and scale tiers, verified and indexed | Running-app scale validation |
| K1 | Expected paper objects, bibliography and mentions | Immutable limited V1–V5 releases on main; V5 has 11 papers with explicitly different per-metric eligibility | Approximately 500 defensible, stratified papers required by [#97](https://github.com/tjmisko/Lysilogy/issues/97); most owning detector metrics; V6 remains on #132 branch |
| K2 | Reference text → deposited DOI pairs | Builder and offline fixtures on [draft PR #93](https://github.com/tjmisko/Lysilogy/pull/93), issue #71 | Genuine Crossref deposited-reference build and measured consumers |
| K3 | Approximately 200 adjudicated entity pairs, including hard negatives | Planned under [#37](https://github.com/tjmisko/Lysilogy/issues/37) | Actual independently labeled gold set; blocked by bibliography work |
| K4 | ORCID-backed person clusters | Silver-label tooling on [draft PR #95](https://github.com/tjmisko/Lysilogy/pull/95), issue #72; stacked on PR #93 | Genuine OpenAlex/ORCID labels, conflict exclusions and measurements |
| K5 | 200 acquisition references with frozen open-access status | Builder and offline fixtures on draft PR #93 | Real K2-derived sample with recorded provider responses and field/age strata |
| K6 | Ten reading-list prompts, survey references and panel rubric | Planned under [#73](https://github.com/tjmisko/Lysilogy/issues/73) | Published truth/panel evidence and list evaluation |
| K7 | Citation-graph leave-one-out holdout for read-next | Builder and offline fixtures on draft PR #93 | Real mapped citation graph, bibliography dependencies and populated holdout |

Provider-backed truth is blocked by effective access to the already-approved registry/provider hosts, not by a confirmed missing credential. Tooling and fixture tests do not substitute for these real datasets. See the handoff's operating guide for the access state and resume sequence.

### Why automatic K1 generation has not produced usable coverage

The latest retained full run used source `6f49e5501abc50efa13aecbe7fec8b30176c1de3` and the same frozen 1,000-paper comparison population:

| Result | Value |
| --- | ---: |
| Parsed candidates | 724 |
| Explicit source/index failures | 276 |
| Automatic admissions | **0** |
| Automatic bibliography/O1–O11 eligible cohorts | **0** |
| Wall time | 940.162525 s |
| Peak RSS | 498,032 KiB |
| Network/model calls; external-call cost | 0 / 0; $0 |

Parsing recovers syntax and inventories. Admission additionally requires a complete expected inventory for the metric, defensible source semantics, and alignment to the original PDF. Unknown commands, local style behavior, control flow, manually formatted objects, ambiguous ownership and unsupported mathematical rendering prevent that stronger claim. A partial text match cannot establish that every relevant object was found.

This comparison intentionally retains the original 999 successful indexes and original failed index record. The later native recovery does not retrospectively change its denominator. Newer manual releases reuse this historical automatic evidence; they are not fresh full-corpus automatic reruns.

The run receipt is `/home/tjmisko/.cache/lysilogy/k1-full-alignment-6f49e55-visual-v3/receipt.json`, SHA-256 `3ec5b1a18091edc0b84357fb19ecb74d31ee50d5df49e7e183547ca7a1f89202`. Independent review is `/home/tjmisko/.cache/lysilogy/review-full-current-actual-6f49e55/review.json`, SHA-256 `843177d3615bb33efb9013629bfd881a50be8af0d680229787ac6407170aac0e`.

The September 14 structural diagnostic found that removing any single unknown-command guard would not clear a paper. Its ten-command counterfactual cleared only one of the 724 structural records before full semantic review. This was a diagnostic, not a parser change, truth admission or evidence that a short command allowlist solves coverage.

### Limited K1 publication history

Each release preserves prior reviewed records and their provenance. Eligibility belongs to a metric and paper, not to the release as a whole. The [truth contract](../../../eval/latex-contract.md) and [per-paper format contract](../../../eval/k1-per-paper-contract.md) specify these boundaries.

| Version | Location/status | Total papers | Added complete scope | Visual objects | Numbered equations |
| --- | --- | ---: | --- | ---: | ---: |
| V1 | Main; #70 | 2 | `2104.01511v1` visual/panel truth; `2503.05828` equation, formal, algorithm and bibliography truth | 15 | 1 |
| V2 | Main; #105 | 3 | `2210.11141`: eight visual objects and four equations | 23 | 5 |
| V3 | Main; #115 | 4 | `2409.03655`: six visual objects | 29 | 5 |
| V4 | Main; #117 | 9 | Five fixed visual-only papers, including a complete negative paper | 50 | 5 |
| V5 | **Newest main release**; #131 / PR #133 | 11 | `2404.17771v2`: seven equations; `1911.08525v2`: 48 equations | 50 | **60** |
| V6 | **Unmerged #132 branch** | 12 | `2310.01528v1`: seven statements, four proofs and three associated statement parts | 50 | 60 |

V4's five papers are `2310.04162`, `2001.05217`, `2002.03492`, `2207.03024` and `2303.07834`. The `2002.03492` visual-negative record remains in the population even though it adds no positive visual objects.

V5 retains four statements, one proof, two algorithms and 35 bibliography entries from earlier releases. V6 increases formal totals to **11 statements and five proofs**. Associated statement parts are retained separately from whole-statement counts. Numbered-equation membership does not certify reconstructed mathematical text for quotation.

The limited releases make selective detector work possible, but their small, manually selected populations are not representative coverage. Eleven admitted papers do not mean eleven papers complete for every metric. The approximately 500-paper requirement remains unchanged and unmet.

Key published reports:

- [V1 and source/annotation contract](../../experiment-reports/2026-09-13-kb-latex-truth.md).
- [V2, automatic coverage experiments and source-role corrections](../../experiment-reports/2026-09-13-kb-k1-coverage.md).
- [V3 visual tranche](../../experiment-reports/2026-09-13-kb-visual-tranche.md).
- [V4 five-paper pilot](../../experiment-reports/2026-09-14-kb-visual-only-pilot.md).
- [V5 numbered-equation release](../../experiment-reports/2026-09-14-kb-numbered-math-tranche.md).

### Release capacity and worker isolation

Issue #129 replaced the growing single-document layout with small manifests and complete per-paper children. The old V4 object document was 991,418 bytes, near the 1 MiB cap. The new format retains full paper records while publishing and replaying them in bounded serial workers. A failed final paper cannot silently produce a smaller successful release.

The real comparison preserved all 117 historical outcomes across 18 overlapping V1–V4 paper rows. A separate generated 500-record experiment demonstrated bounded publication/replay and rejection of a deliberately failing final record. **Those 500 generated records are not 500 real annotated papers, and this is not a production corpus benchmark.** See [the measured #129 report](../../experiment-reports/2026-09-14-kb-bounded-k1-replay.md).

Release dispatch still contains version/cohort-specific code. The architecture review recommends stable per-paper evidence and worker contracts so additions usually require data rather than more special routing. The bounded format is useful progress; it does not finish that simplification.

## Scorecard snapshot

### Hard gates

| Gate | Required result | Committed status |
| --- | --- | --- |
| G1 | Work and Person auto-merge precision ≥0.99, both components | Unavailable |
| G2 | Zero wrong-paper links after download | Unavailable |
| G3 | Zero published enrichment quote/source mismatches | Unavailable |
| G4 | Identical entities, IDs and aliases across two rebuilds | Unavailable |
| G5 | Real tests pass with network disabled and no model CLIs | **Pass** |

### Objectives

Values below reproduce the committed scorecard; `unavailable` is an explicit status, not an assumed zero.

| ID | Metric | Target | Current committed result |
| --- | --- | --- | --- |
| O1 | Figure/table detection F1 | ≥0.90 | **0.989899; at target** |
| O2 | Median figure/table region IoU | ≥0.75 | **0.891002; at target** |
| O3 | Numbered-equation detection F1 | ≥0.85 | Unavailable |
| O4 | Equation/statement mention link accuracy | ≥0.90 | Unavailable |
| O5 | Statement detection F1 | ≥0.85 | Unavailable |
| O6 | Proof → statement link accuracy | ≥0.85 | Unavailable |
| O7 | Algorithm detection F1 | ≥0.80 | Unavailable |
| O8 | Bibliography segmentation F1 | ≥0.95 | Unavailable |
| O9 | Title / first author / year accuracy | ≥0.90 / ≥0.90 / ≥0.95 | All unavailable |
| O10 | Citation marker → entry precision / recall | ≥0.97 / ≥0.85 | Both unavailable |
| O11 | Key-figure top-three panel agreement | ≥0.70 | Unavailable |
| O12 | Reference → identifier precision / recall | ≥0.98 / ≥0.80 | Both unavailable |
| O13 | Person-clustering B-cubed F1 | ≥0.90 | Unavailable |
| O14 | Auto-merge recall | ≥0.80 | Unavailable |
| O15 | Duplicate Works remaining | ≤0.02 | Unavailable |
| O16 | References reaching `identifier_found` | ≥0.85 | Unavailable |
| O17 | OA references reaching `downloaded` | ≥0.80 | Unavailable |
| O18 | Downloads mapped under `extract_heuristic` | ≥0.95 | Unavailable |
| O19 | Model calls per acquired reference | ≤0.30 | Unavailable |
| O20 | Normalized styled-citation match | ≥0.90 | Unavailable |
| O21 | BibTeX / RIS exports parsing without errors | Both 1.0 | Both unavailable |
| O22 | AI proposals resolved / fabricated | ≥0.90 / ≤0.05 | Both unavailable |
| O23 | AI-list relevance panel mean | ≥4/5 | Unavailable |
| O24 | Read-next recall@10 | ≥0.30 | Unavailable |
| O25 | No-change rescan at 10k | ≤2 s | Unavailable; historical baseline 15.503406 s |
| O26 | Home first render / search p95 at 10k | ≤500 / ≤150 ms | Both unavailable; historical baselines 1757.1 / 163.4 ms |
| O27 | Four-worker extraction efficiency | ≥0.70 | Unavailable |
| O28 | Two-hop query p95, 500k Works / 3M edges | ≤150 ms | Unavailable |
| O29 | Graph frame rate at 500 nodes | ≥30 fps | Unavailable |
| O30 | Provider-budget violations at 10k references | 0 | **0; at target** |

All R metrics remain unavailable in the committed scorecard: R1 agent-acquisition lift, R2 list/survey overlap, R3 inter-labeler agreement, and the six R4 cost/time components for enrichment, acquisition and list generation. Offline experiment receipts with zero external calls do not measure these product R4 workflows. Agent reasoning cost is unknown where not metered, not $0.

### Evidence limits on the three passing objectives

- **O1/O2:** the committed scorecard explicitly identifies **K1 V4**, 50 cases. Newer retained V5 measurements and #132's V6 collector preserve the exact values `0.98989898989899` / `0.8910021250829322`. That continuity is additional evidence; this handoff does not rewrite the committed scorecard as V5/V6. Current visual outcomes are 49 true positives, zero false positives, one false negative, two zero IoUs and no unknown truth regions.
- **O30:** a recorded 10,000-reference simulation measured zero violations of the configured budget algorithm across four providers, including cooldowns, deferrals and persistence/reload cases. It made no live provider calls. This is not evidence of current provider throughput or account credit. See the [provider-budget report](../../experiment-reports/2026-09-12-provider-budgets.md).
- **Bibliography branch only:** #25 / draft PR #94 retained limited K1 O8=1 over 35 entries and O10 precision/recall=1 over 50 markers. Title agreement was 30/35; first-author and year agreement 35/35. These are not merged global metrics. Genuine K2/full O9 remains missing, and [#103](https://github.com/tjmisko/Lysilogy/issues/103) tracks line-break/hyphen title handling.

The historical O25/O26 misses remain assigned to [#21](https://github.com/tjmisko/Lysilogy/issues/21) and [#22](https://github.com/tjmisko/Lysilogy/issues/22). The broad truth miss remains #97. Past detector misses prompted #106, #111, #121, #123 and #125; those fixes are merged. Passing aggregate O1/O2 does not erase the retained individual misses or limited coverage.

## #132: exact retained state and remaining validation

Branch `feat/e8.3-formal-tranche` is at `b4ff5b99ddbf29abec94947eb9cb54c3d21d32fb`; there is no PR yet. Its V6 release is not on main.

| Stage | Retained result | Limit |
| --- | --- | --- |
| Source/construction review | Corrections and independent/root reviews clear | Does not replace actual runtime validation |
| Actual attachment and publication | Succeeded; 18.074427 s publication; 13 workers including 11 direct V5 parent workers | Validates this 12-paper release, not 500-paper throughput |
| Publication review | Independent review clear; prior 11 complete records/provenance preserved | Version-specific release approval |
| Initial collector | Failed before measurement because the anonymous loader had no `sys.modules` entry | Failed result retained |
| Loader correction | Actual anonymous-loader regression and separate review clear | Final integrated gate rerun still needed |
| Corrected V1–V6 replays | All six succeeded; independent review clear | Replay is separate from the current collector |
| Corrected V6 collector | Exit 0; **86.598899 s**; 12 decisions, nine prediction papers, all 50 visual outcomes unchanged | **Independent collector audit still pending**; no O5/O6 detector measurement |
| Final completion | Not performed | Historical comparison, owning CLI after-check, latest-main integration, final gates/report/PR review and merge remain |

The full integrated gates recorded before the loader correction ran **428 Rust / 645 Python / 85 Node tests**, with formatting, strict Clippy and isolated G5. Those earlier gates are not relabeled as a complete run on corrected head `b4ff5b9`. Main's last merged #131 gate result was **428 Rust / 627 Python / 85 Node** at reviewed integration `ff33385`.

All paths in the next table are beneath `/home/tjmisko/.cache/lysilogy/`:

| Evidence | Relative local path | SHA-256 |
| --- | --- | --- |
| Root construction review | `review-formal-construction-132-root-v1/construction-review.json` | `c831240c8bb6a7955717f1c33d7ef0a4026bd7f79349a63958fbd62cf4e3f9f0` |
| Actual publication | `formal-tranche-132/publication-v1/receipt.json` | `d3667c657dd9a0a8e38201e16e0b37027eacd99f07d39564b5835c750ed5565c` |
| Independent publication review | `review-formal-runtime-132-v1/publication-review.json` | `7e371b58feeff03618377d01f1becb82cffa7098f6bd0e47071ca47fbf94107f` |
| Failed first collector | `formal-tranche-132/collector-v1/receipt.json` | `8abd59afeca4423c8898c0d2b9671432e821d11103e3e2d8e9549888af433ae0` |
| Loader-fix review | `review-formal-dispatch-b4ff5b9/review.json` | `b15a30e8be16a00c30155cbd6fc42e5e3e2583281c3c68c08544733700d8f2af` |
| Corrected replays | `formal-tranche-132/pinned-replays-v2/receipt.json` | `e247eb7102def857cd14d081216029b6d6c65c62e9ed635f72fde977e92511a4` |
| Independent replay review | `review-formal-runtime-132-v1/replay-v2-review.json` | `01daef223824b698cfdd61470bebb545bcea8f1b31b0a88a77333d108413e2fa` |
| Corrected collector, audit pending | `formal-tranche-132/collector-v2/receipt.json` | `3fd6b074743301261a34165653f0ffeb8bffc3254a88606f4831bee58c243599` |

The published V6 implementation manifest is `508bf5b2270ed497ab2cca901a3771f948e5da021d1076ab3f16cf652581e00b`. Preserve its 21 producer modules and 145 published/legacy files. The loader fix did not republish V6. #134 and #135 are reviewed proposals for later limited additions, not implemented truth or new scores.

## Failed experiments and engineering lessons

| Work | Observed result | Consequence |
| --- | --- | --- |
| Automatic source truth | Latest fixed 1,000-paper run: 724 parsed, zero admitted | Measure semantic capability and admitted-paper throughput; parser success alone is insufficient |
| Earlier apparent negative cohort | `2007.05954` contained manually formatted bibliography/procedure material omitted by parsed inventories | Retained as failed evidence; source-role guards prevent an incomplete inventory claiming an empty cohort |
| BibTeX syntax expansion, #109 | On the fixed 710-paper comparison, recovered five of eight failures but lost 18 earlier successes; net 13 fewer parsed papers | Preserve complete denominators and report regressions, not only recovered examples; [report](../../experiment-reports/2026-09-14-kb-bibtex-fields.md) |
| Recompile original papers, #110 | One of ten built; zero reproduced every original page at both 96 and 192 dpi | Installing missing packages alone would not prove reliable original-layout truth |
| V4 visual expansion | Initial O1 fell to 0.897959; later detector fixes restored 0.989899 | Cohort expansion revealed real errors; frozen outcomes and unchanged targets made the correction measurable |
| V5 collector | Initial allocation failure; identical request/binary subsequently passed at original 768 MiB limit | Cause remains unresolved; failed attempt and successful retry remain distinct evidence |
| V6 collector | Standalone replay passed, actual anonymous-loader entry point failed | Test the integrated production invocation early; the loader regression is now covered |
| Release growth | Near-1 MiB V4 document and repeated version-specific routing | Per-paper bounded storage is implemented; stable worker/evidence contracts need further simplification |

The [source-layout experiment](../../experiment-reports/2026-09-13-kb-source-layout-probe.md) admitted no truth. Nine builds lacked dependencies in the frozen runtime; the successful build had the original page count but none of the required complete-document pixel equivalence. No package installation or weakened similarity criterion was used to relabel that result.

## How to evaluate without overstating completion

Run evaluations from an implementation worktree, after its owning collector has regenerated source-bound observations. Evaluation writes results, baselines and the scorecard; it is not a read-only status command. Do not run it in the protected main working tree merely to refresh this report.

```sh
# In the owning worktree, with its built CLI and required cached dependencies:
lysilogy eval objects --check
lysilogy eval all --check

# Final system acceptance additionally requires complete coverage:
lysilogy eval all --check --require-complete
```

The first two commands permit unavailable suites during incremental development. The final command requires every hard gate and at least 24 of 30 objectives at target. Compound objectives count only when all their components meet target. Changed implementation, truth or observation hashes invalidate old measurements rather than silently reusing them.

G5 establishes a network namespace, verifies that network access is blocked and model CLIs are absent, then runs offline Rust, Python and frontend fixture suites. Browser smoke tests remain a separate obligation. The required running-app Playwright workflow, real 10k acceptance run, remaining gates and objectives, phase reports and final `docs/experiment-reports/<date>-kb-system.md` are still outstanding.

## Evidence locations and preservation

| Location | Contents | Handling |
| --- | --- | --- |
| [`eval/truth/`](../../../eval/truth/) | Derived labels, identities, cohort metadata and hashes | Immutable published versions; never replace with current parser output |
| [`eval/implementations/`](../../../eval/implementations/) | Exact retained application verifier modules | Preserve historical bytes; these are application code, not deposited paper sources |
| [`eval/evidence/`](../../../eval/evidence/) | Committed reports/manifests binding external execution evidence | Follow explicit receipt hashes and source heads |
| [`docs/experiment-reports/`](../../experiment-reports/) | Human-readable implementation and experiment reports | Historical reports describe their execution date; use newer checkpoints for current status |
| `/home/tjmisko/.cache/lysilogy/` | Raw attempts, source snapshots, binaries, annotations, renders, worker logs and independent reviews | Preserve both failures and successful retries; paths can outlive deleted worktrees |
| `/home/tjmisko/Corpora/arxiv/` | Original corpus PDFs, sources and receipts | Keep external; do not copy into Git, the user's vault or `.lysilogy` |

Portable evidence often retains an `original_path` into a removed worktree and a separate current `path` to archived bytes. A historical path's disappearance alone does not prove loss; use the retained mapping and hash. Conversely, a reported hash is not a substitute for missing bytes. Do not clean caches or immutable versions as part of a documentation update.

## Complete retained report index

The entries below are all experiment reports present at the snapshot. Earlier reader reports predate the KB buildout. Their inclusion records provenance; it does not imply those experiments were rerun or passed KB acceptance.

| Report | Scope |
| --- | --- |
| [2026-08-31-baseline.md](../../experiment-reports/2026-08-31-baseline.md) — Learning-ramp experiment baseline — 2026-08-31 | Pre-existing reader work |
| [2026-09-01-conceptual-bridge.md](../../experiment-reports/2026-09-01-conceptual-bridge.md) — Conceptual-bridge experiment — 2026-09-01 | Pre-existing reader work |
| [2026-09-11-reading-pipeline.md](../../experiment-reports/2026-09-11-reading-pipeline.md) — Reading pipeline implementation and validation — 2026-09-11 | Pre-existing reader work |
| [2026-09-11-sectioning.md](../../experiment-reports/2026-09-11-sectioning.md) — Coherent reading units — 2026-09-11 | Pre-existing reader work |
| [2026-09-12-corpus-proxy-transport.md](../../experiment-reports/2026-09-12-corpus-proxy-transport.md) — E8.2 corpus proxy transport follow-up | KB buildout / supporting experiment |
| [2026-09-12-e0.1-d2019ea2.md](../../experiment-reports/2026-09-12-e0.1-d2019ea2.md) — E0.1 measured synthetic 10k baseline | KB buildout / supporting experiment |
| [2026-09-12-e0.1-synthetic-benchmark.md](../../experiment-reports/2026-09-12-e0.1-synthetic-benchmark.md) — E0.1 synthetic benchmark tooling and initial fixture verification | KB buildout / supporting experiment |
| [2026-09-12-provider-budgets.md](../../experiment-reports/2026-09-12-provider-budgets.md) — Provider cache and budget measurement (E7.1) | KB buildout / supporting experiment |
| [2026-09-12-provider-lease-unlock.md](../../experiment-reports/2026-09-12-provider-lease-unlock.md) — Provider lock lifetime correction (#83) | KB buildout / supporting experiment |
| [2026-09-13-corpus-availability.md](../../experiment-reports/2026-09-13-corpus-availability.md) — Corpus availability and explicit recovery — 2026-09-13 | KB buildout / supporting experiment |
| [2026-09-13-corpus-source-endpoint.md](../../experiment-reports/2026-09-13-corpus-source-endpoint.md) — Canonical arXiv source endpoint — 2026-09-13 | KB buildout / supporting experiment |
| [2026-09-13-figure-detection.md](../../experiment-reports/2026-09-13-figure-detection.md) — Current figure/table detection and geometry (#101) | KB buildout / supporting experiment |
| [2026-09-13-kb-corpus-complete.md](../../experiment-reports/2026-09-13-kb-corpus-complete.md) — Complete K0 corpus verification — 2026-09-13 | KB buildout / supporting experiment |
| [2026-09-13-kb-duplicate-labels.md](../../experiment-reports/2026-09-13-kb-duplicate-labels.md) — Distinct source objects with ambiguous labels (E8.3, #107) | KB buildout / supporting experiment |
| [2026-09-13-kb-expanded-k1-regions.md](../../experiment-reports/2026-09-13-kb-expanded-k1-regions.md) — Expanded K1 figure and table regions | KB buildout / supporting experiment |
| [2026-09-13-kb-k1-coverage.md](../../experiment-reports/2026-09-13-kb-k1-coverage.md) — Reviewed manual K1 expansion — issues #105 and #97 | KB buildout / supporting experiment |
| [2026-09-13-kb-latex-truth.md](../../experiment-reports/2026-09-13-kb-latex-truth.md) — E8.3 independent LaTeX truth and limited K1 release | KB buildout / supporting experiment |
| [2026-09-13-kb-mask-regions.md](../../experiment-reports/2026-09-13-kb-mask-regions.md) — Masked figure regions from decoded opacity | KB buildout / supporting experiment |
| [2026-09-13-kb-native-diagnostics.md](../../experiment-reports/2026-09-13-kb-native-diagnostics.md) — Bounded native diagnostics recover the batch 28 failure | KB buildout / supporting experiment |
| [2026-09-13-kb-reading-page-failure.md](../../experiment-reports/2026-09-13-kb-reading-page-failure.md) — Reading-index page isolation (#98) | KB buildout / supporting experiment |
| [2026-09-13-kb-source-layout-probe.md](../../experiment-reports/2026-09-13-kb-source-layout-probe.md) — Confined source-layout reconstruction — E8.3 / #110 | KB buildout / supporting experiment |
| [2026-09-13-kb-visual-tranche.md](../../experiment-reports/2026-09-13-kb-visual-tranche.md) — Limited visual tranche — E8.3 / issue #115 | KB buildout / supporting experiment |
| [2026-09-13-object-metrics.md](../../experiment-reports/2026-09-13-object-metrics.md) — Independent figure/table metrics (E1.1 follow-up #96) | KB buildout / supporting experiment |
| [2026-09-13-person-name-parser.md](../../experiment-reports/2026-09-13-person-name-parser.md) — E2.3 person name parsing | KB buildout / supporting experiment |
| [2026-09-13-title-normalizer.md](../../experiment-reports/2026-09-13-title-normalizer.md) — E2.4 title keys and fuzzy scores | KB buildout / supporting experiment |
| [2026-09-14-kb-bibtex-fields.md](../../experiment-reports/2026-09-14-kb-bibtex-fields.md) — Complete BibTeX source fields (E8.3, #109) | KB buildout / supporting experiment |
| [2026-09-14-kb-bounded-k1-replay.md](../../experiment-reports/2026-09-14-kb-bounded-k1-replay.md) — Bounded K1 publication, replay and collection | KB buildout / supporting experiment |
| [2026-09-14-kb-numbered-math-tranche.md](../../experiment-reports/2026-09-14-kb-numbered-math-tranche.md) — Complete numbered-equation truth for two frozen papers | KB buildout / supporting experiment |
| [2026-09-14-kb-vector-regions.md](../../experiment-reports/2026-09-14-kb-vector-regions.md) — Bounded vector support for figure regions | KB buildout / supporting experiment |
| [2026-09-14-kb-visual-captions.md](../../experiment-reports/2026-09-14-kb-visual-captions.md) — Complete titles for split table captions | KB buildout / supporting experiment |
| [2026-09-14-kb-visual-only-pilot.md](../../experiment-reports/2026-09-14-kb-visual-only-pilot.md) — Fixed visual-only pilot — E8.3 / issue #117 | KB buildout / supporting experiment |
| [2026-09-14-kb-visual-regions.md](../../experiment-reports/2026-09-14-kb-visual-regions.md) — Native figure regions below tables | KB buildout / supporting experiment |
