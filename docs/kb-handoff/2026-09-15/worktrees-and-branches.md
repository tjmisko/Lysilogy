# Worktrees, branches, and unfinished changes

[Back to guide](README.md)

Inventory captured **2026-09-16T04:34:30.832246+00:00** (September 15 in America/Los_Angeles), before the handoff documentation commit. Implementation baseline: `4972d63f10efdc2a89d01901b1adc0624408e868`; cached `origin/main` matched. This is a dated inventory, not a claim that refs never change.

## Worktrees

Git registered **11 worktrees**, including the temporary handoff worktree and one missing-directory entry. Five are unfinished KB implementation worktrees. Ahead/behind means commits unique to the branch/main respectively, calculated using `git rev-list --left-right --count main...HEAD`; these are history counts, not measures of work remaining.

| Branch | Absolute directory | HEAD at snapshot | Ahead / behind main | State |
| --- | --- | --- | --- | --- |
| `main` | `/home/tjmisko/Projects/Lysilogy` | `4972d63f10efdc2a89d01901b1adc0624408e868` | 0 / 0 | Dirty; see below. Shared checkout; ten protected PDF-preview changes. Documentation commits only. |
| `docs/kb-handoff-20260915` | `/home/tjmisko/Projects/Lysilogy/.worktrees/docs/kb-handoff-20260915` | `4972d63f10efdc2a89d01901b1adc0624408e868` | 0 / 0 | Clean. Temporary documentation branch for this handoff. Snapshot predates its commit/PR; remove only this worktree after delivery. |
| `feat/e1.2-bibliography` | `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e1.2-bibliography` | `201c53c7a7041f7740b68e7495e4b93358f8639b` | 29 / 242 | Clean. [#25](https://github.com/tjmisko/Lysilogy/issues/25); draft [#94](https://github.com/tjmisko/Lysilogy/pull/94). Limited K1 measurements; missing genuine K2/full O9. |
| `feat/e2.1-kb-store` | `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e2.1-kb-store` | `905c3410cf22cb74c26aa652b2bb544cdea07c1a` | 9 / 335 | Dirty; see below. [#33](https://github.com/tjmisko/Lysilogy/issues/33); no PR. Implementation written; Rust build/gates still unrun; pending registry access. |
| `feat/e8.3-formal-tranche` | `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e8.3-formal-tranche` | `b4ff5b99ddbf29abec94947eb9cb54c3d21d32fb` | 6 / 3 | Dirty; see below. [#132](https://github.com/tjmisko/Lysilogy/issues/132); no PR, no upstream. V6 producer/replays and collector retry succeeded; independent collector audit and final gates remain. |
| `feat/e8.4-reference-truth` | `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e8.4-reference-truth` | `d5e12aa748b7c19a833989799c565c3309d01d65` | 12 / 353 | Clean. [#71](https://github.com/tjmisko/Lysilogy/issues/71); draft [#93](https://github.com/tjmisko/Lysilogy/pull/93). Builders tested offline; genuine provider snapshots and final truth absent. |
| `feat/e8.5-person-labels` | `/home/tjmisko/Projects/Lysilogy/.worktrees/feat/e8.5-person-labels` | `c867ec694d3c5c87eecb262012b2d3677a601037` | 20 / 337 | Clean. [#72](https://github.com/tjmisko/Lysilogy/issues/72); draft [#95](https://github.com/tjmisko/Lysilogy/pull/95), based on reference-truth branch. Genuine person labels absent. |
| `feat/paper-link-hints` | `/tmp/lysilogy-link-hints` | `b750307900457634b3f3a55d449eb01a5cb63ff3` | 0 / 469 | Clean. Pre-existing reader work; [#10](https://github.com/tjmisko/Lysilogy/pull/10) merged. Historical checkout, not an active KB implementation. |
| `fix/pdf-dark-rendering` | `/tmp/lysilogy-pdf-dark-rendering` | `35d8d2ca60d6cfec017e5672aa879a58e5a5bf74` | 0 / 474 | Clean. Pre-existing reader work, already contained in main. Retained checkout; do not delete as KB cleanup. |
| `feat/lysilogos-supercut-references` | `/tmp/lysilogy-reader-tools.CDW5tk` | `d92091affc1f276439f85f82ff35d12648f95dd4` | Not queried | Missing directory. Pre-existing reader work; [#7](https://github.com/tjmisko/Lysilogy/pull/7) merged. Directory missing; Git marks entry prunable. Left untouched. |
| `feat/pdf-visual-fit` | `/tmp/lysilogy-visual-fit` | `441475898e5db3f5d9b15d101795d6080910826d` | 0 / 477 | Clean. Pre-existing reader work, already contained in main. Retained checkout; do not delete as KB cleanup. |

## Dirty worktrees: preserve these files

Paths are relative to the worktree named in each heading. Nothing in these lists was reverted, staged or cleaned by the handoff. “Untracked directory” can contain many evidence files; it is not disposable just because Git does not track it.

### `main`

```text
 M .gitignore
 M web/package.json
 M web/src/App.tsx
 M web/src/components/HomePage.tsx
 M web/src/components/PaperPreview.tsx
 M web/src/lib/pdfPreview.ts
?? web/scripts/pdf-preview-cache.test.mjs
?? web/scripts/pdf-preview-smoke.mjs
?? web/src/lib/pdfPreviewCache.ts
?? web/src/lib/pdfPreviewStorage.ts
```

### `feat/e2.1-kb-store`

```text
 M .gitignore
```

### `feat/e8.3-formal-tranche`

```text
 M eval/inputs/objects/figure-table.json
?? eval/evidence/object-metrics-history/ac0417e46b5bd362387377589159fb379dffedfc3882cdfe48d012f7d3004d9e/
?? eval/evidence/object-metrics-layout/5b394151e9f278d0-1789401958885792637/
```

The ten main-checkout preview files match the earlier protected SHA-256 snapshot exactly. That snapshot is `~/.cache/lysilogy/kb-preview-before.json`; the handoff inventory records each comparison. The SQLite worktree also retains important ignored notes at `target/kb-resume-notes.md`. Its old unfinished patch is historical only: later committed corrections supersede it. Do not apply that patch.

The formal branch’s changed collector input and two untracked evidence directories are the successful collector-v2 output. Its source HEAD does not include those files. Preserve them with their receipts before preparing the final PR. See [resume order](operations-and-resume.md#resume-the-current-branches).

## Open PRs

| PR | Head branch | Base | Draft | Blocker |
| --- | --- | --- | --- | --- |
| [#93](https://github.com/tjmisko/Lysilogy/pull/93) | `feat/e8.4-reference-truth` | `main` | Yes | Real K2/K5 provider evidence; K7 also needs bibliography/scale graph. |
| [#94](https://github.com/tjmisko/Lysilogy/pull/94) | `feat/e1.2-bibliography` | `main` | Yes | Real K2 and complete O9 measurement; five known title misses. |
| [#95](https://github.com/tjmisko/Lysilogy/pull/95) | `feat/e8.5-person-labels` | `feat/e8.4-reference-truth` | Yes | Real K2 inputs and deposited-ORCID labels; keep the stack on #93. |

Neither #33 nor #132 has a PR. #134 and #135 are issue proposals with reviewed preparation, **no worktree and no code branch** yet; their planned names are `feat/e8.3-numbered-2301` and `feat/e8.3-formal-2011`. Their existence in the phase plan must not be mistaken for existing refs.

## All local branches

The inventory includes inactive branches without worktrees. Empty upstream means no configured upstream; it does not establish whether another remote ref exists. No fetch/prune was performed for this report. GitHub PR state was queried separately.

| Branch | HEAD | Upstream | Tracking difference |
| --- | --- | --- | --- |
| `codex/topbar-reading-views` | `4f15d4dd5774def9710d10dad30f7e3e7e9f5e3f` | `origin/codex/topbar-reading-views` | None recorded |
| `docs/kb-handoff-20260915` | `4972d63f10efdc2a89d01901b1adc0624408e868` | None | None recorded |
| `docs/knowledge-base-plan` | `f43bacbffda79b32a44a01883f857600b68bb202` | `origin/docs/knowledge-base-plan` | None recorded |
| `feat/e1.2-bibliography` | `201c53c7a7041f7740b68e7495e4b93358f8639b` | `origin/feat/e1.2-bibliography` | None recorded |
| `feat/e2.1-kb-store` | `905c3410cf22cb74c26aa652b2bb544cdea07c1a` | `origin/feat/e2.1-kb-store` | None recorded |
| `feat/e8.3-formal-tranche` | `b4ff5b99ddbf29abec94947eb9cb54c3d21d32fb` | None | None recorded |
| `feat/e8.4-reference-truth` | `d5e12aa748b7c19a833989799c565c3309d01d65` | `origin/feat/e8.4-reference-truth` | None recorded |
| `feat/e8.5-person-labels` | `c867ec694d3c5c87eecb262012b2d3677a601037` | `origin/feat/e8.5-person-labels` | None recorded |
| `feat/lysilogos-supercut-references` | `d92091affc1f276439f85f82ff35d12648f95dd4` | `origin/feat/lysilogos-supercut-references` | None recorded |
| `feat/paper-link-hints` | `b750307900457634b3f3a55d449eb01a5cb63ff3` | `origin/feat/paper-link-hints` | None recorded |
| `feat/pdf-visual-fit` | `441475898e5db3f5d9b15d101795d6080910826d` | None | None recorded |
| `fix/pdf-dark-rendering` | `35d8d2ca60d6cfec017e5672aa879a58e5a5bf74` | None | None recorded |
| `main` | `4972d63f10efdc2a89d01901b1adc0624408e868` | `origin/main` | None recorded |
| `refactor/rename-lysilogy` | `149aae2cf16b8458847bcfb703c7a979185e935c` | `origin/refactor/rename-lysilogy` | None recorded |

## Cached remote refs

`origin` below is the display name for the symbolic `origin/HEAD` alias, not an additional branch. These refs may include merged branches retained on the remote. Deletion was not part of this task.

| Ref | HEAD |
| --- | --- |
| `origin` | `4972d63f10efdc2a89d01901b1adc0624408e868` |
| `origin/codex/atlas-page-grid` | `83e74af4cd04b80b57748a6601728bd376926c9c` |
| `origin/codex/topbar-reading-views` | `4f15d4dd5774def9710d10dad30f7e3e7e9f5e3f` |
| `origin/docs/knowledge-base-plan` | `f43bacbffda79b32a44a01883f857600b68bb202` |
| `origin/feat/e1.2-bibliography` | `201c53c7a7041f7740b68e7495e4b93358f8639b` |
| `origin/feat/e2.1-kb-store` | `905c3410cf22cb74c26aa652b2bb544cdea07c1a` |
| `origin/feat/e8.4-reference-truth` | `d5e12aa748b7c19a833989799c565c3309d01d65` |
| `origin/feat/e8.5-person-labels` | `c867ec694d3c5c87eecb262012b2d3677a601037` |
| `origin/feat/lysilogos-supercut-references` | `d92091affc1f276439f85f82ff35d12648f95dd4` |
| `origin/feat/paper-link-hints` | `b750307900457634b3f3a55d449eb01a5cb63ff3` |
| `origin/main` | `4972d63f10efdc2a89d01901b1adc0624408e868` |
| `origin/pr/2` | `ffabf4cf940def18dad192e1b9cb9e394565d253` |
| `origin/refactor/rename-lysilogy` | `149aae2cf16b8458847bcfb703c7a979185e935c` |
| `origin/review/paper-link-hints-base` | `571c3eda7850a2691324767dc5eca15d35c2931d` |
| `origin/review/reader-tools-baseline` | `42a6f13fa2e77d7c335f5de50ccf57590b3e25f4` |

## Storage and running work

- Free bytes on `/home` at capture: **12,875,173,888** (11.99 GiB). This is below the project’s **20 GiB free-space floor**. No large job was started for this handoff.
- Approximate allocated build targets: bibliography 882 MiB; SQLite 535 MiB; formal 653 MiB; reference truth 629 MiB; person labels 629 MiB. The whole `~/.cache/lysilogy` directory is approximately 39 GiB and includes retained evidence/data, not merely expendable build files.
- `/tmp` is a 7.7 GiB RAM filesystem, with approximately 2.9 GiB used at capture. The unrelated reader worktrees already there were left untouched.
- At the last implementation checkpoint (2026-09-14 16:17 UTC), no corpus download, native-index queue, or heavy job remained. The corrected #132 collector had terminated successfully. No new implementation jobs were started during this handoff.
- The current tool process list is sandbox-scoped and cannot certify all host processes. The old agent names in environment history are not evidence of running downloads. Team inspection showed only the root agent before the documentation tasks were started.
- Before freeing a completed implementation target, preserve ignored resume notes and portable receipts. Do not delete corpus inputs, the corpus data root, unfinished worktrees or evidence directories to recover space. Space elsewhere on the host was not inspected.

## Snapshot provenance

- `~/.cache/lysilogy/handoff-20260915/workspace.json` — SHA-256 `0fd426f05efb922dec4788fee943cb7a6482bd78080c88d2781d61ff63063962`.
- `~/.cache/lysilogy/handoff-20260915/issues.json` — SHA-256 `8bd3fa3ff56c586bb586bf55546d6ecc2f8c6ae1247aad18c508b9153290dfac`.
- `~/.cache/lysilogy/handoff-20260915/prs.json` — SHA-256 `2d7f673670992aac860137a4f7b66edbec2eb9a6ae51cde2e896033190055149`.

These local JSON files hold the exact inventory behind the tables. The Markdown report is committed; cache files are local handoff evidence. After merging this documentation, main and the temporary docs branch will naturally differ from this pre-documentation snapshot.
