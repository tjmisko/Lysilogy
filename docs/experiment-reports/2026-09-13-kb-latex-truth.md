# E8.3 independent LaTeX truth: implementation and coverage checkpoint

K1 is **not yet published**. This checkpoint implements bounded offline source parsing,
independent text alignment, explicit exhaustive metric cohorts, and separately reviewed manual
region/panel evidence. The complete 1,000-paper exploratory run found **zero publishable automatic
cohorts**. The approximately 500-paper target remains unmet; no detector objective is reported as
passing from these diagnostics.

The source reader never extracts or executes TeX. It retains exact source members, all parsed
objects/entries/links, a canonical inventory hash, unsupported semantics and every omitted member.
Unknown source visibility, local package programs, structural aliases, numbering ambiguity,
mathematical scripts/alphabets and ambiguous alignment remain explicit. Clipped link contexts and
unsupported row rendering no longer erase unrelated inventories. Resource bounds remain fatal.
Standard inventory capability is separate from rendering capability; no arbitrary command is
silenced to improve coverage. See [the cohort contract](../../eval/latex-contract.md).

## Actual corpus and native indexing

The frozen eval tier contains all 1,000 PDF/source pairs across the planned strata. The offline
native helper used the existing dedicated `~/.cache/lysilogy/arxiv-kb-data` registry and mapped
1,000 unique PaperIds. It produced **999 successful indexes** in **1,374.785 seconds**, with zero
network/model calls. Peak RSS was not instrumented. Source archives and PDFs remain external.

The retained failure is `2308.05883v2`: three negative-width Poppler combining-circumflex rectangles
among 19,570 words. Independent visual/glyph inspection showed that swapping endpoints would
invent geometry. The finite/ordered coordinate gate remains unchanged. Root independently
rehashed all 1,000 PDFs and 999 successful indexes: 6,171,809,461 bytes matched in 9.302 seconds.

Native receipts: `~/.cache/lysilogy/k1-full-index-receipt.json`,
`k1-full-index-root-review.json`, and
`layout-diagnostics/2308.05883v2/diagnostic.json`. The helper executable SHA-256 is
`9583e52411a0fc80466bfa4ad9f50d671d79ad8b218b9f4b5fccec506f7350c9`;
its source SHA-256 is `4cdce79da883df3fd656d5ba1f3dee3cbdc780532dab2f77587e20c664a4bf51`.
The full-index map SHA-256 is
`40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`.

## Full exploratory alignment

The immutable run at historical parser checkpoint `0910c35` rehashed PDF/source/index bytes and
processed all 1,000 frozen inputs with one Python process, a 1.5 GiB address-space cap and a
30-second per-paper diagnostic timeout. It completed with exit 0 in **1,017.604 seconds**, peak
RSS **512,176 KiB**, with zero network/model calls and $0 external-call cost.

| Result | Papers |
| --- | ---: |
| Parsed source inventory | 626 |
| Accepted automatic metric cohort | 0 |
| Complete figure text alignment before source-semantic guards | 75 |
| Complete table text alignment before source-semantic guards | 126 |
| Complete algorithm text alignment before source-semantic guards | 42 |
| Complete statement text alignment before source-semantic guards | 1 |
| Complete equation or proof text alignment before source-semantic guards | 0 |

The last five rows are diagnostics only. Source-semantic guards exclude these papers; they are
not accepted truth. Major whole-paper exclusions were duplicate object labels (84), unterminated
arguments (39), legitimate deposited PDF payloads without TeX (39), and crossing/unbalanced
source environments (22). Two per-paper timeouts remain exclusions. Every parsed candidate had
at least one unknown inventory command; common examples included standard relation/accents and
layout dimensions, alongside actual unsupported control flow, definitions and local styles.

Retained files are under `~/.cache/lysilogy/k1-full-alignment-0910c35/`: all full candidates,
`papers.jsonl`, `report.json`, exact copied parser modules, runner, launch and raw log. The
separate `receipt.json` hashes each receipt. Report SHA-256:
`8a21a70a94e132d6c42f96a487af45d5ad7deee3d4163675dac80ef3fcfb77e1`.

Later independently reviewed corrections preserve standalone suppressed equations, transitive
numbering aliases, semantic math alphabets and the exact mu glyph encoding alias. Literal TeX
penalty integers are now consumed as layout parameters. Those corrections are **absent from the
historical run**; its results remain immutable. Four separately retained current-source probes
at `b363638` remain excluded and do not replace any frozen annotation packet.

## Independent manual evidence

Before inspecting detector predictions, root and a separate reviewer visually verified all eight
pages of `2104.01511v1`, all ten figures and five tables, full visual-body regions, and their exact
source-caption associations. One caption with scripted feature names retains its automatic
script-binding exclusion and receives separate manually reviewed source/visual evidence.

Three fresh blinded panel agents independently read all pages and returned the same ordered top
three source objects: `object:tab:lateFusionResults`, `object:timeToEvent`, `object:shapPlot`.
Their aggregate wall time was 313.431 seconds; the harness did not expose their dollar cost.
No production enrichment/ranking was run, so **O11 remains unmeasured**. The fixed nine-opportunity
per-paper scoring policy was recorded before root inspected any vote.

The committed manual CLI at `864c96c` rehashed the actual PDF, source, index and all page renders,
validated the exact independent region/association/vote receipts, and assembled an external
candidate in 0.116 seconds. Candidate SHA-256:
`f6989274e1b96d46642bcc97ddc7afe7f9e1da78d6d646e973ca32eb0bbc6b13`.
It is retained under `~/.cache/lysilogy/k1-manual-candidates/<sha256>/`, with an assembly receipt.
It preserves the full automatic candidate and its exclusions. This is one exploratory paper;
manual-validator software review and final stratified publication remain separate acceptance
steps. Root is additionally constructing a bounded complete math/statement/proof/algorithm
annotation for `2503.05828`; that evidence is not incorporated or claimed here.

## Validation and remaining work

The latest source checkpoint has **102 passing offline Python tests**. Independent reviewers
cleared the core through `4151464`; the subsequent finite inventory capability and manual-evidence
integration are under separate review. A historical full quality checkpoint at `60c0c35` passed
formatting, strict Clippy, all Rust targets, objects/bibliography eval checks and G5, with actual
G5 counts **306 Rust / 166 Python / 85 Node**. Baseline results and that exact checkpoint's logs
remain under `target/k1-before/` and `target/k1-checkpoint-60c0c35/`. Final integrated gates must
still run after the remaining corrections; historical gates are not claimed for newer source.

K1's approximately 500-paper target and every-kind acceptance remain open. Next work is to finish
independent manual-validator review, establish complete source/PDF evidence for unsupported math
kinds, improve bounded supported source capabilities, and remeasure without shortening any
inventory. Production O1/O2 measurement belongs to follow-up #96; bibliography measurements also
wait for actual accepted K1/K2. No fixture-derived metric or exploratory subset substitutes for
final truth publication.
