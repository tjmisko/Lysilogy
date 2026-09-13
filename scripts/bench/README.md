# Synthetic library benchmarks

From the repository root, with Poppler, cached Cargo/npm dependencies, and Playwright Chromium installed:

```sh
python3 scripts/bench/run.py
cargo run --offline -- eval scale --check
cargo run --offline -- eval tests --check
```

The runner generates exactly 10,000 deterministic small PDFs beneath
`~/.cache/lysilogy/bench-vault/papers/`, verifies every byte against seed 19, builds the real Rust
binary/example in release mode and the production frontend, and measures the running application.
The generator varies title, author, year, one-to-three-page length, filename, and directory depth.
`--seed`, `--count`, and `--root` select a different owned configuration; the root must be the
designated directory or a child of it. Existing different files/configurations and symlink paths
are rejected. Generation resumes exact files. No PDFs are copied to the repository or `/tmp`.
At least 5 GiB free is required before the run; extraction artifacts can be much larger than PDFs.

Every run creates a fresh `runs/<random-id>/` beneath that vault. Its catalog artifacts, browser
temporary profile, notes root, config, server log, raw observations, and screenshot are isolated.
Previous run directories are retained. The benchmark never opens the real library, notes, default
data root, or a model. The browser permits only GET/HEAD requests to its own loopback origin;
it rejects external requests and all mutations. Source thumbnails retain normal app behavior.
The server always terminates after the browser measurement, including on failure.

## Measured phases

1. Initial `LibraryCatalog::scan` with fresh application data. The OS page cache is already warm
   from generation/verification; this is not an OS cold-cache claim.
2. Three discovered-only no-change scans through the production scan/replace methods.
3. Three serial and three four-worker trials using the real `PdfExtractor` (Poppler text,
   layout, and metadata). Trial order alternates 1/4 then 4/1; output hashes must match across
   every trial. Throughput includes task dispatch, extraction, and output hashing; it excludes
   persistence. `--extraction-limit` may bound this exploratory capacity sample.
4. A separate serial setup pass extracts and persists **every** generated paper through the
   real `ArtifactStore`, even if capacity trials used a smaller sample. Its wall time is reported
   separately. The first populated scan and three subsequent populated no-change scans are
   timed independently; every paper must have a valid extraction artifact.
5. Playwright opens the real app with that populated data root in three fresh browser contexts,
   at 1280×800 and locale en-US. First render runs from navigation start until the populated
   home has the verified total count and a visible card, followed by two animation frames
   (a paint opportunity). This includes assets/API/React work; it does not await thumbnails.
6. Twenty deterministic title, author, and year queries per context. Each starts from the full
   populated home. Browser capture listeners timestamp input dispatch; completion requires the
   independently computed result count and first sorted paper, followed by a paint opportunity.
   The nearest-rank p95 includes filtering, removing old cards, and rendering new results.
   Playwright command transport time is excluded.

`result.json` retains machine/CPU information, release profile, source commit/dirty state,
source hashes, manifest hash, zero model cost, total wall time (including setup/builds), and
per-phase summaries. Raw backend/browser observations remain next to it. Full-vault generation
and integrity checks run before and after measurement; changed source hashes prevent publication.

## Evaluation publication

Only verified 10,000-paper runs with at least three rescans, three navigations, and twenty searches
publish `eval/inputs/scale/synthetic-vault.json`, plus derived manifest/observations under
`eval/inputs/evidence/synthetic-vault/`. Publication replaces only this collector's files atomically.
The provider budget collector's O30 observations remain untouched.

- **O25:** median populated no-change scan, seconds, 10,000 actual persisted extraction artifacts.
- **O26.render:** median home first render, milliseconds, at 10,000 papers.
- **O26.search:** nearest-rank p95 of the measured searches, milliseconds, at 10,000 papers.
- **O27:** unavailable. `serial_seconds / four_worker_seconds / 4` is reported as experimental
  extractor capacity only. The application's production `ingest` remains serial; E0.5 must
  measure its own shipping worker path before claiming this objective. There is no theoretical
  0.25 baseline and no production scheduler change in this issue.

Smaller `--count` runs retain local reports but never publish scale metrics. Missing storage or
any phase failure leaves the run incomplete; it cannot establish a baseline. On a source change,
run the collector again before `eval scale --check`; old observations do not receive fresh hashes.
Retained synthetic measurements are useful baselines for E0.3–E0.5, not a substitute for the later
10k arXiv acceptance scenario.

## Bounded verification

```sh
python3 scripts/bench/test_vault.py
python3 scripts/bench/test_run.py
cargo test --offline --example scale_bench
node --test web/scripts/scale-bench.test.mjs
python3 scripts/bench/smoke.py
```

The first four are offline unit/fixture tests, included by G5. The last command explicitly drives
a live loopback app with exactly eight tiny generated PDFs and real extraction artifacts in a
temporary test directory; it verifies three navigations and sixty searches, then removes that
directory. This bounded fixture exception never creates a 10k vault or publishes metrics.
Its derived JSON and screenshot remain under `target/bench-fixture-*`; timing values use a debug
backend and establish functional coverage only. Browser smoke is separate from the no-network
unit-test suite.
