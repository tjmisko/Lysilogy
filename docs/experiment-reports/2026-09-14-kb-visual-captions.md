# Complete titles for split table captions

Issue #125 recovers five table captions whose uppercase labels and titles occupy
separate native paragraphs. Detector 5 requires a contiguous title, compatible page
geometry and an observed table grid before extending caption ownership. The fixed
50-object cohort improves from 44 TP / 4 FP / 6 FN to 49 TP / 0 FP / 1 FN. Detection F1 rises from
0.8979591836734694 to 0.98989898989899, above the unchanged 0.90 target. Median region
IoU rises from0.8826424973982893 to 0.8863525877384802, above the unchanged 0.75 target.

The exact measured implementation is `999c96b1d19c07490c73ddcd9ada603449e70acc`.
All four frozen cohorts were collected again: 15/23/29/50 outcomes, representing
50 unique objects across nine papers. V1/V2/V3 outcomes are unchanged. Their O1
values remain 1; O2 remains 0.9107793204006069/0.9169720168893188/0.9169720168893188.
No region regresses. [Machine evidence](../../eval/evidence/visual-captions-125.json)
retains every comparison, original zero and receipt binding.

All six changed outcomes belong to [2310.04162v1](https://arxiv.org/abs/2310.04162v1):

| Object | Previous IoU | Current IoU |
|---|---:|---:|
| Table I, outcome comparison | 0 | 0.5393503782988236 |
| Table II, odometry test | 0 | 0.5352142521823823 |
| Table III, ablation | 0 | 0.30183001330558284 |
| Table IV, consuming time | 0 | 0.9422290460470415 |
| Table V, real-world ATE | 0 | 0.9011551352046338 |
| Figure 8, real-world result | 0 | 0.14077990837051904 |

The five recovered table captions each have caption Dice 1. Table I–IV previously
produced label-only false positives; Table V was absent. Figure 8 already had a correct
caption. Complete table-caption ownership changes its neighboring body evidence,
yielding a small native region; this is still weak coverage. Its page 8 graphics
evidence was already selected before this change. All nine complete graphics bases,
trace and mask inventories remain byte-identical. No new graphics support is claimed.

All 50 regions remain in the denominator. Zeros decrease from 8 to 2; unknown truth
regions remain 0. The historical `object:timeToEvent` region remains zero. Figure 3
in [2207.03024v1](https://arxiv.org/abs/2207.03024v1) remains the sole caption miss:
its native paragraph interleaves caption lines with unrelated prose. This change
keeps that case unresolved, as permitted by the issue, because its recovery requires
explicit disjoint caption spans and corresponding cache/API/evaluation bindings.
Body member anchors retain their documented purpose. Native reading indexes,
truth labels and metric matching are unchanged.

## Ownership boundaries and review findings

The extension requires a complete uppercase TABLE label and a single adjacent
uppercase title paragraph with exact native-token endpoints. Only whitespace may
separate the two spans. Page, font, line, center and gap checks precede table-grid
corroboration. Both original paragraphs become caption-owned and are excluded from
body candidates. The public schema is unchanged; the derived detector and collector
version advance to 5. Graphics version remains 3.

Grid corroboration uses retained paragraph members. Other captions, prose, headings,
lists and equations cannot supply table cells. Whole classified paragraphs crossing
the vertical corridor and the complete inferred grid width block the join. Bounded
cell counts and precomputed header candidates limit the new corroboration work.

Twenty new Rust regression functions cover the actual split forms and adversarial
page, column, glyph, endpoint, image, prose, heading, list, equation and ownership
boundaries. Positive fixtures include five Roman labels and body/float classifications.
Independent review found three structural gaps: a classified paragraph beginning
above the caption, a blocked outer table column, and raw tokens from excluded
paragraphs supplying grid evidence. Failing composed regressions were recorded
before repair. Retained-member filtering and complete paragraph/grid barriers resolve
all three. A retained paragraph gap is tested explicitly. Initial Clippy failures and
reviewer audit-preparation mistakes are preserved separately from successful evidence.

## Validation, cost and limits

The independent actual audit rehashed 588 artifacts (201,724,153 bytes), verified all
251 measurement files and recomputed 117 current and 117 prior cohort outcomes with
exact equality. It checked source snapshots against Git and all native/PDF/source
commitments. This is an overlapping fixed pilot, not 500 independently sampled papers;
the expanded coverage requirement in #97 remains open.

The before check built and ran the owning worktree CLI at e248ea2. The normal after
`objects --check` used the archived Cargo-selected CLI built from the measured source.
It passed and improved only O1/O2 baselines automatically. Existing adjustment history,
other metric baselines, objective targets and all hard gates remain unchanged. The
historical detector 4 cohort observations are retained runs, not new baseline executions.

Formatting, strict all-target/all-feature Clippy, all 406 Rust tests and the owning CLI
build passed at 999c96b. The remaining owning-CLI bibliography/scale checks and isolated
G5 passed at a99d21e, with the same 205 compiled source files. G5 ran 406 Rust / 581 Python /
85 Node tests in a separate network namespace with only loopback and model CLIs absent.
The combined remaining run took 25.118887312 seconds. O30 remains 0/10,000 in the offline
provider-budget fixture. The generated scorecard now shows 1/5 hard gates and 3/30
objectives at target (O1/O2/O30); unavailable suites remain unavailable. The worktree
used 132 byte-verified TypeScript 5.9.3 files from the existing local installation, with
no package download. No frontend source changed.

Actual four-cohort measurement took 156.770666305 seconds with 279328 KiB peak child RSS.
The own-CLI before build/check took 95.829719490 seconds, compiled gates 45.663558 seconds,
and normal after check 0.387215611 seconds. These wall times include build or replay
work where applicable; they are not controlled detector or 10k performance measurements.
External network/model calls and service cost are zero; agent reasoning cost is unknown.
No native re-extraction, full-model batch, corpus download or truth publication occurred.

Ordinary integration with current main follows the standing permission rule requiring
approval for rebases. Separate review and successful gates precede the merge commit.
The later vector-region issue #123 is coordinated to use detector 6 after integration.
Phase A/Wave A2, the production 10k scenario and final app/system acceptance remain open.
