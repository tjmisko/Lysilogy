# Confined source-layout reconstruction — E8.3 / #110

The frozen ten-paper experiment is a **no-go** with the installed runtime:
**1/10 papers built and 0/10 reproduced every original page**. The fixed criteria
were at least eight builds and five complete matching documents, with every page
matching at both 96 dpi and 192 dpi. No K1 labels were published. Issue #97's
approximately 500-paper coverage requirement remains open.

Nine papers stopped at an unavailable class or package in the frozen runtime.
The remaining paper, 1911.08525v2, rebuilt to eight pages, matching the original
page count, but all 16 page/resolution comparisons had different decoded pixels.
This result does not establish why each pixel differs or whether a source-layout
approach could work with another reviewed runtime.

| Selected paper | Original pages | Build result | First unavailable dependency |
| --- | ---: | --- | --- |
| [2210.11141v1](https://arxiv.org/abs/2210.11141v1) |5|Failed|silence.sty|
| [2310.04162v1](https://arxiv.org/abs/2310.04162v1) |8|Failed|IEEEtran.cls|
| [2404.17771v2](https://arxiv.org/abs/2404.17771v2) |6|Failed|tablefootnote.sty|
| [2011.00685v3](https://arxiv.org/abs/2011.00685v3) |8|Failed|algorithm2e.sty|
| [2001.05217v1](https://arxiv.org/abs/2001.05217v1) |7|Failed|bbm.sty|
| [2409.03655v1](https://arxiv.org/abs/2409.03655v1) |6|Failed|wrapfig.sty|
| [2002.03492v1](https://arxiv.org/abs/2002.03492v1) |5|Failed|revtex4-1.cls|
| [1911.08525v2](https://arxiv.org/abs/1911.08525v2) |8|Built; no matching whole document|—|
| [2207.03024v1](https://arxiv.org/abs/2207.03024v1) |5|Failed|subfigure.sty|
| [2303.07834v2](https://arxiv.org/abs/2303.07834v2) |7|Failed|bbm.sty|

## Selection and execution

Selection preceded deposited source execution and used the earlier availability
ordering, with one first-ranked paper in each of seven strata and the next three
eligible second-ranked papers. All had at most eight original pages. Detector
results, parse success and rebuilt layout did not select the papers. Exact source,
PDF and native-index identities were verified before and after; all ten remained
in the denominator. No corpus artifacts were copied into the repository, vault
or application data directory.

The executed source was 8c153b342f47588f5f5c4aeaf156b6d3ce9da333. The external
snapshot copied committed bootstrap/module bytes and frozen selection/runtime
inputs. Isolated Python compiled verified source bytes directly; adjacent bytecode
could not replace them. The launch bound Python, bwrap and unshare and required
the exact independent review. Each sandbox invocation rechecked the tool pins.

PDFLaTeX ran inside fresh user/network/PID/IPC/UTS/cgroup and mount namespaces,
with no host home, project or corpus root exposed. Only copied bounded source
inputs, exact trusted runtime mounts and fixed output inodes were available.
Shell escape and first-line format selection were disabled. A native aarch64
positive syscall filter denied network/process creation and limit changes.
Inputs/runtime/root/output directories were read-only; individual pre-created
output files were writable. Each process had 768 MiB address space, bounded CPU,
per-file output limits and a 20 GiB free-space floor. The finite inode count bounded
aggregate output without relying on polling. Builds allowed two passes within
90 seconds; the absolute 15-minute batch deadline also covered sandbox setup.

The full invocation took 15.785762 seconds (batch 15.766313 seconds), with 45 confined
processes: 36 passed and nine failed. All recorded output-size/inode invariants
passed. The maximum recorded controller-lifetime child RSS high-water mark was
59,840 KiB; this is not a per-invocation peak measurement. Network/model calls and
external experiment cost were zero/$0. Agent-development cost is not included.

## Validation and review findings

Before final integration, the isolated G5 gate passed 362 Rust, 440 Python and 85 Node tests. The 57 new layout
tests cover bounded archives and paths, aliases, missing capabilities, syscall
controls, memory/CPU/output limits, interruption cleanup, stale bytecode, frozen
tool identities, full-denominator failures and absolute deadlines. Generated TeX
tests verify fixed engine/format behavior, disabled shell escape, hidden host
sentinels, refused input writes and terminated infinite expansion. Fresh builds
reproduce identical decoded pixels, while changed superscripts, glyphs and
fractions fail exact comparison at both resolutions. No test makes live provider
or model calls or executes a deposited source.

Two production findings were corrected before any deposited execution: writable
source/runtime aliases, and a deadline that initially excluded controller setup.
Independent reproductions now refuse those cases. A generated test initially
expected the literal format name LaTeX; installed LaTeX correctly identifies
itself as LaTeX2e. That test expectation was corrected, with the failed generation
retained. Earlier capability snapshots remain historical evidence; they are not
relabeled as execution of newer source bytes.

The final exact-source synthetic refresh passed seven cases in 8.952154 seconds,
with 453 retained artifacts. Independent launch review verified those artifacts,
all 45 synthetic confinement receipts, all 16 synthetic rasters, the frozen ten
inputs and 242 gate-source fingerprints before authorizing the actual experiment.

The independent actual-result audit verified 739 file references, all 234 archive-to-materialized source members, all 30 original artifact identities, 45 process receipts and 32 rasters. It confirms the no-go outcome without a result blocker. Four overwritten first-pass states remain explicitly historical hashes.

Final integration `21fd6445e0af408564973ed7acb0f8fe882f4331` includes reviewed #107 and #111. Formatting,
strict all-target/all-feature Clippy, the own CLI build, isolated G5, a fresh
current-source v2 collector and `eval objects --check` all pass. G5 counts are
**368 Rust / 481 Python / 85 Node**, including the 57 layout tests.
The initial integrated attempt took **69.746979 seconds**. Its
formatting, Clippy, build and G5 commands passed, but its availability assertion
correctly failed: the wrapper invoked `--build`, which returns without measuring.
The corrected `--executable` invocation and affected objects check took
**28.400502 seconds**. Unchanged earlier quality checks remain bound;
the failed wrapper receipt is preserved. Independent wrapper review initially
missed this dispatch error; the result guard prevented an unavailable metric from
being reported as success.

The before-change objects check explicitly measured O1 = 0.9787234042553191 and
O2 = 0.7570080448318404. Incoming #106 and #111 account for the improvement to
**O1 = 1 and O2 = 0.9169720168893188**. Issue #110 contributes zero detector,
scoring or truth changes. The final refresh preserves all three complete paper
records, all 23 decisions, coverage, matched-only median, predictions and
object/truth/external-input hashes exactly from merged #111. Its newly generated
objects result has both metrics explicitly available. No hard gate or target changed.

The earlier source-only gate's objects command exited successfully with unavailable
metrics after the G5 tooling fingerprint changed. That historical result remains
retained and is not presented as a measured pass. The final refresh resolves its
availability gap without modifying truth or outcomes.

## Limits and next action

This experiment installs no packages, changes no source, substitutes no engine,
relaxes no target and admits no truth from approximate page similarity. The
source-date epoch remains the preselected 0; date fidelity is explicitly unknown.
The current installed TeX release need not reproduce the original toolchain.
Missing dependencies describe the frozen admitted runtime, without asserting
their absence from every possible installation.

Fixed output inodes deliberately disallow arbitrary auxiliary files and rename.
SyncTeX can leave a bounded busy file, and no reviewed query decoder or layout
truth codec exists. First-pass output hashes describe their state at that pass;
the second pass intentionally reuses those inodes. Separate pass logs/receipts
and final output bytes remain, but intermediate mutable outputs were not copied
into a separate archive. This limits retrospective inspection of that first pass.

The baseline CLI executable was hashed at execution, but its bytes were not
archived before later builds replaced it. Its committed source, raw Cargo
selection and command logs remain; they do not substitute for the old executable
bytes. The later exact-source gate CLI and final integrated CLI/bridge are
archived separately.

The embedded `source.deposited_source_executed: false` value is historical materialization-stage metadata: that step copied bytes without executing them. It does not describe the subsequent build or complete experiment, which executed eleven PDFLaTeX invocations across the ten selected papers.

The next source-layout experiment would first require a reviewed, complete,
content-addressed TeX runtime and a measured reproduction check on the unchanged
selection. That is follow-up work toward #97, not automatic repair of this failed
experiment. Any later truth admission still needs a separate reviewed codec and
complete original-PDF/source evidence. The current phase/system acceptance is
unchanged and incomplete.

## Durable evidence

The [machine evidence](../../eval/evidence/source-layout-probe.json) binds the source, selection, runtime, launch,
independent reviews, actual outcomes, gate logs and metric refresh. Artifacts live
under `~/.cache/lysilogy/source-layout-probe/` and separate review directories;
the paper corpus remains under `~/Corpora/arxiv/`. PDFs and deposited sources are
not committed.

- Selection 9b3542e0366b13a3fb66cfafd92dc21a5ef341b435a924502ce66e3b82e6df0d.
- Launch 0b02e7bfc0faa9f4fdff157d8345c0c086c7f03c2b5c95ca9a14deb30429f725.
- Independent launch review 0d967578086257e9fece3172aa59a730da931788d4f8de72a81e15e9a86752f3.
- Execution receipt fc4f6de170b1b87d630312ad7b903f7e9fd545e6e77ad04dce6b46b8aec1bea7.
- Complete experiment 613e314ff3eaffa07f4beeb16591a82f2e6cf4bb772db79f73bf06ef69b48d60.
- Exact-source synthetic 2a4cb03532d1d069cae457ef2cefacb24b7ad7a91a40ede56d0076695e1a99cf.
- Integrated gates aca2073e9eb9dcf25915666b08c0cb0fedde76120a51b144e1ec01fa358c0c28.

- Independent actual review a6baf0118b50cbc9b4003d29f5dc6c6b842ea689995db7d65e08a72e0d703d48.

- Final integrated checks 2e0f3510e06f36125b936c7c8853d4ded01b16f5e318887200bb40cf7780116b.
- Portable inventory 18476b4dd2b9202410f6c095210c775a8cf4f4349999c3a2f8a3a0c71018c457: 2576 files / 843285720 bytes.
