# Operations and resume guide

[Back to guide](README.md)

This is a continuation guide for the unfinished KB build, not a claim that the finished KB application is available. Commands below were checked against source/documentation; they were not all executed during the handoff. Read each operation's scope before using it.

## Storage map

| Location | Contents / rule |
| --- | --- |
| `/home/tjmisko/Projects/Lysilogy` | Main checkout. Ten unrelated PDF-preview files are dirty; protect them. No implementation edits here. |
| `.worktrees/feat/…` under the repository | Five unfinished implementation checkouts. Exact paths and dirty files are in the [worktree inventory](worktrees-and-branches.md). |
| `~/Corpora/arxiv/` | Frozen corpus selection, metadata, inventory, PDF/source artifacts, manifest and verification sidecars. Never copy papers into the repository or user vault. |
| `~/Corpora/arxiv/pdf/` | Deduplicated union: 10,951 PDFs. This directory is not exactly the 10,000-paper scale selection. |
| `~/Corpora/arxiv/source/` | 1,000 eval-tier source artifacts. Deposited TeX is untrusted; do not execute it outside the explicitly reviewed experiment confinement. |
| `~/.cache/lysilogy/arxiv-kb-data/` | Existing separate corpus data root and native indexes. Preserve identities and generated evidence; do not reset or remap it casually. |
| `~/.cache/lysilogy/` | Benchmarks, truth construction, review receipts, portable snapshots and retained command logs. Much of this is evidence, not a disposable cache. |
| `eval/` in the repository | Compact truth labels, evidence bindings, collector inputs, results and baseline history; no downloaded PDFs/LaTeX. |
| `docs/experiment-reports/` | Versioned experiment/measurement reports. See the [evidence index](evaluation-and-corpus.md). |
| `~/.cache/lysilogy/artifacts/` | Standalone dark HTML explanation, its generator/checks and browser failure log. |
| `/tmp` | RAM filesystem. No new corpus, benchmark or build storage. Existing unrelated reader worktrees are outside KB cleanup scope. |

The root README predates some KB changes: its blanket statement that IDs are path-derived is stale after #20. The corpus README also illustrates a different independent data-root location. For this existing corpus, keep using the recorded `~/.cache/lysilogy/arxiv-kb-data` root; do not create a second competing identity registry from a copied example.

## First checks in a resumed session

These commands inspect repository state without compiling, downloading or touching the user vault:

```sh
cd /home/tjmisko/Projects/Lysilogy
git status --short
git worktree list --porcelain
git branch -avv
gh pr list --repo tjmisko/Lysilogy --state open
df -h /home/tjmisko /tmp
python3 scripts/corpus/corpus.py --root "$HOME/Corpora/arxiv" status
```

Read the final [session-log entry](../../knowledge-base-phases.md#session-log), the owning issue and its design section before changing anything. Compare all dirty files with this handoff. A prior “clean” row is not permission to overwrite changes made since the snapshot. Check current agents and actual retained job receipts; do not assume old agent names or PID files describe a live process.

**Free space was 12,875,173,888 bytes (11.99 GiB), below the 20 GiB floor.** Do not start another large build, corpus operation or scale benchmark until the floor is restored. The five active target directories total only about 3.3 GiB; deleting them all would not by itself restore the floor and would lose useful state/rebuild caches. Inventory completed scratch outputs, retain required receipts and notes, and reclaim only verified disposable artifacts. Do not delete corpus, vault, notes or a data root.

No background downloader or native-index queue remained at the last implementation checkpoint. The current tool process namespace does not prove host-wide absence of jobs. This documentation session started none.

## Permissions and external access

| Hosts / capability | Approval and last known state | Work affected |
| --- | --- | --- |
| `oaipmh.arxiv.org`, `export.arxiv.org`, `storage.googleapis.com`; writable corpus/cache roots | Previously approved and used to finish corpus preparation. Existing artifacts are retained. | No further corpus download is currently required. |
| `index.crates.io`, `static.crates.io` | Approved by the user. Last effective check, September 14, was denied; no saved grant was found then. | Missing Rust dependencies on #33 prevent compilation and real gates. |
| `api.crossref.org`, `api.openalex.org` | Approved by the user. Last effective check, September 14, was denied; real HTTP/provider outcomes were not obtained. | #71/#72 genuine reference/acquisition/person truth. |
| `registry.npmjs.org`, `api.semanticscholar.org`, `api.opencitations.net` | Proposed additional grants only; not approved or applied. | Possible future dependency/provider work. |

The already-prepared four-host script is:

```sh
python3 ~/.config/lysilogy/apply-codex-kb-network-permissions.py
```

The prior session could not persist the change because the mounted configuration was read-only, even under approved escalation. That was **not an automatic approval-review rejection**. The recorded unblock is to run the script in a normal terminal, restart Codex and verify effective host access from the resumed tool environment. The four hosts do not need approval again. This handoff does not claim the script has since run and does not apply the separate seven-host proposal script.

No missing credential is confirmed. Optional provider keys/contact configuration must remain outside committed evidence; never read environment or secrets files to discover values. A domain allowlist denial is not a provider-authentication failure. Historical Git pushes also printed a credential-storage-lock warning while exiting successfully; use exit status and the remote result before declaring a push blocked.

## Resume the current branches

Implementation was paused for strategic discussion. The sequence below preserves the existing plan; it does not adopt the proposed cross-phase scheduling change.

### 1. Finish #132's already-produced evidence

Worktree: `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e8.3-formal-tranche`.
Branch: `feat/e8.3-formal-tranche`, source HEAD `b4ff5b99ddbf29abec94947eb9cb54c3d21d32fb`.
There is no PR and no configured upstream. Preserve the changed collector input and untracked evidence directories listed in the inventory.

The corrected collector succeeded in **86.598899 seconds**, with 12 release decisions, nine visual prediction papers and all 50 visual outcomes unchanged. New formal truth adds no O5/O6 detector score. Its receipt is `~/.cache/lysilogy/formal-tranche-132/collector-v2/receipt.json`, SHA-256 `3fd6b074743301261a34165653f0ffeb8bffc3254a88606f4831bee58c243599`.

Remaining steps:

1. Run a separate audit of that actual collector retry, not just its command or source code.
2. Compare retained #117 historical outcomes and complete inherited records. Keep collector-v1's failure and collector-v2's success as distinct evidence.
3. Run the owning CLI's objects after-check and record exact inputs/scorecard delta.
4. Preserve any dirty evidence before integrating current main by an ordinary merge. The standing instruction requires asking before rebase; prior sessions recorded ordinary merges as the authorized deviation.
5. Run final integrated quality gates once disk capacity permits. Older full-suite receipts preceded the loader correction and do not establish that exact final head passed every gate.
6. Write the report, open a draft PR with `Closes #132`, obtain a separate final diff review, fix findings, then merge with a merge commit.
7. Preserve portable source/evidence/receipts; remove only the completed worktree/branch. Update phase notes and session log.

The [evidence report](evaluation-and-corpus.md) lists the prior publication, replay and review receipts. The concrete anonymous-loader regression is already fixed on this branch; do not repeat the original diagnosis as unfinished implementation work.

### 2. Unblock #33 and genuine provider truth

| Issue / branch | Next concrete work | What cannot be claimed yet |
| --- | --- | --- |
| #33 `feat/e2.1-kb-store` | Preserve dirty `.gitignore` and `target/kb-resume-notes.md`; obtain approved registry access; build committed Rust source; address startup error propagation; run migration/rebuild tests and prescribed scale benchmark. | Python SQL checks and formatting do not establish compiled Rust correctness, G4 or O28. |
| #71 `feat/e8.4-reference-truth`, draft #93 | Integrate current main carefully; build genuine frozen Crossref/OpenAlex K2/K5 evidence under provider budgets, then complete K7 when bibliography and scale graph exist. | Offline fixture builders are not production truth sets. |
| #72 `feat/e8.5-person-labels`, draft #95 | Preserve the stack on #71; use real upstream inputs and deposited ORCID evidence for K4. | Synthetic fixtures are not person silver labels; absent ORCID is not negative identity evidence. |
| #25 `feat/e1.2-bibliography`, draft #94 | Preserve limited K1 results; obtain K2 and complete O9 measurement, integrate current main, repeat affected gates and independent review. | Limited K1 segmentation/link results do not certify broad bibliography accuracy. Five known title misses remain in #103. |

These branches have diverged substantially from main; consult the commit counts and actual diffs, rather than merging all of them indiscriminately. Shared `Cargo.toml`, `src/api.rs`, `src/main.rs`, `src/lib.rs`, domain types and object adapters are conflict areas. Do not flatten #95's PR base before #93 is integrated.

### 3. Continue truth coverage and the remaining waves

#134 (`feat/e8.3-numbered-2301`) is queued after #132, with no worktree yet. It proposes four complete numbered equations from 2301.05184v1. #135 (`feat/e8.3-formal-2011`) follows #132 and #134, proposing three Problems and one Theorem from 2011.00685v3 for O5 only. Their preparation reviews are not runtime or publication approval. Use the exact issue bodies and parent contracts; never rewrite retained truth releases.

#97 remains the approximately 500-paper stratified coverage task. A one-paper semantic capability pilot, stable per-paper evidence format and earlier product integration were recommended in the strategy review. No new scheduling policy or lower coverage target was adopted. A3 still needs #37, and B–D product work remains planned. The [issue register](issue-register.md) lists every wave and dependency.

## Build and validation workflow

Use an existing implementation worktree for its issue, or create a new one with:

```sh
gh worktree create --branch <branch-from-the-phase-plan>
```

Run at most three implementations concurrently, with **one heavy build/replay/benchmark at a time** on the 16 GB aarch64 machine. Existing Cargo settings for bounded offline gates are:

```sh
export CARGO_NET_OFFLINE=true
export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Keep each worktree's target on disk in that worktree. When dependencies are missing, an offline failure is a dependency blocker, not a source-test failure; retrieve only approved dependencies in a separate setup step before running offline tests. Do not remove isolation or introduce live provider/model calls into tests.

For frontend changes, in that worktree's `web/` directory:

```sh
npm run typecheck
npm run lint
npm run build
```

Run the relevant fixture-backed `test:*` and `smoke:*` scripts listed in its own `web/package.json`. Main's uncommitted preview package scripts are user work and are not part of a clean implementation checkout. Generic `npm run smoke` expects a locally absent corpus fixture; use the documented fixture smokes or an explicit scratch Playwright harness. A static check is not a browser run.

Python truth tests use custom `should_…_when_…` discovery in their direct script entry points. Invoke the documented scripts, not an unexamined default discovery command that could report zero tests. Preserve source revision, commands, counts and exit statuses in the PR/report. Tests belong beside code and must verify behavior or real integration boundaries.

### Evaluation writes results

From the owning worktree, after building its binary:

```sh
./target/debug/lysilogy eval objects --check --root .
```

Substitute the suite owned by the issue. This writes result/scorecard/baseline artifacts under the selected root; it is not read-only. Run before and after a change and keep provenance intact. Do not evaluate in the dirty main checkout just to refresh a status report. An available suite pass does not establish system completeness.

The final completeness command is:

```sh
./target/debug/lysilogy eval all --check --require-complete --root .
```

It must require all gates and at least 24/30 objectives. G5 executes isolated tests, so the `all` command can be a heavy operation. Real truth generation, production collector measurements and the Playwright scenario remain separate prerequisites; an eval invocation does not invent missing datasets.

## Inspect the current reader safely

The existing Rust/Axum + React/PDF.js reader predates most KB work. It can be built from a clean worktree using the repository's [README](../../../README.md), with Rust 1.88+, Node 22+, Poppler and built web assets. To inspect an already-provisioned corpus data root without selecting the user's vault by default, explicitly pass all paths:

```sh
./target/debug/lysilogy \
  --library "$HOME/Corpora/arxiv/pdf" \
  --data "$HOME/.cache/lysilogy/arxiv-kb-data" \
  --notes "$HOME/.cache/lysilogy/kb-inspection-notes" \
  serve --bind 127.0.0.1:7319 --web web/dist
```

This starts a server and may update generated cache/job state; it is not a read-only inventory command and was not run for this handoff. Preserve the existing data-root identity registry. Do not run whole-corpus model ingestion. For an explicit bounded offline analysis, select one paper and `--provider heuristic`; the CLI's default provider is model-backed. Default paths otherwise point to `local-articles`, `.lysilogy` and `Notes`, which are inappropriate for an accidental corpus experiment.

There is no `corpus` subcommand on main: use `python3 scripts/corpus/corpus.py`. Likewise, `kb rebuild` exists only on the unfinished SQLite branch, not the delivered main CLI. Published equation/formal truth does not make equation/proof navigation appear in the current UI.

## Merge, cleanup and continuity

Each implementation requires a draft PR, `Closes #n`, tests, scorecard delta and deviations; a distinct reviewer; current-main integration; and passing final gates before a merge commit. Use ordinary main merges under the recorded no-rebase deviation. Commit messages end with:

```text
Co-Authored-By: Codex <noreply@openai.com>
```

After merging, preserve any local-only evidence and ignored notes before deleting that issue's branch/worktree. Do not use blanket cleanup across `.worktrees`, `/tmp` or the cache. Update the phase checkbox/notes and append a session entry with the last merged issue, exact unfinished branches/PRs, background jobs, next action and unresolved blockers. Commit documentation updates without staging the user's preview changes.

After the last required issue of each phase, perform its exit checks and write `docs/experiment-reports/<date>-kb-phase-<X>.md`. After all phases, drive the actual built app with Playwright and write the final KB system report with screenshots and scorecard evidence. Live verification is limited to three enriched papers, one AI reading list and no `full_model` batch; record external cost and elapsed time separately from unmetered agent reasoning.

No phase or system completion is asserted by this handoff. The immediate known problems are insufficient free space, unverified approved-host access, insufficient truth coverage and unfinished integration—not a confirmed need for paid credentials, more human labels or package installation by sudo.
