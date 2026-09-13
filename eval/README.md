# Knowledge-base evaluation

Run from a checkout without initializing the library, vault, notes, or data root:

```sh
cargo run -- eval objects --check
cargo run -- eval tests --check
cargo run -- eval all --check --require-complete
```

`eval --check` defaults to `all`. Available suites: `objects`, `bibliography`, `resolution`,
`persons`, `acquisition`, `citations`, `lists`, `read-next`, `scale`, `tests`, and `all`.
G5 executes real isolated tests in `tests` and `all`; the other suites never invoke models or
network. The Git repository defaults to the current directory (`--root` overrides it).

Dirty state conservatively includes all nonignored untracked files and tracked changes, including
generated scorecard/baseline updates. Each run records timestamp, commit and dirty state, per-component values, case counts, evidence
hashes, truth versions, known cost, wall time, previous baselines, and failures in
`eval/results/<suite>/<timestamp>-<commit>.json`. The scorecard combines the most recent result
for each component, ordered by timestamp then suite. A newer unavailable result replaces an
older measured result. The scorecard has no generation timestamp, so identical inputs produce
identical Markdown. Result and input caches are ignored by Git; derived labels, baselines, and
reports may be committed. No PDF or LaTeX source belongs here.

During buildout, unavailable metrics do not fail `--check`; they also do not pass a gate or
count at target. Final acceptance uses `--require-complete` in addition to `all --check` and
requires every gate and at least 24 of 30 objectives. Missed-objective follow-up issues and the
live end-to-end scenario are recorded in the system report, outside this numerical check.

## Collector contract

E8.1 supplies the harness, metric definitions, aggregators, and G5 runner. Each subsequent
feature/truth issue adds its evaluator to produce `eval/inputs/<suite>/<collector-id>.json`.
The original `eval/inputs/<suite>.json` format remains supported alongside these files. A collector must
run the actual implementation over local truth, record the observations, and fingerprint all
implementation dependencies that affect the measurement. Refresh these observations **before**
running `eval` after a change; evaluation deliberately refuses stale hashes. The evidence contract
makes results auditable but cannot prove the correctness of a collector or its labels; reviewers
must inspect that computation in the owning issue.

Collectors sharing a suite own disjoint metric IDs. For example, catalog benchmarks may write
`eval/inputs/scale/catalog.json` for O25–O29 while provider simulation writes
`eval/inputs/scale/provider-budgets.json` for O30. Each file retains its own implementation,
truth, observation evidence, cost, and wall time. The harness never unions or re-fingerprints
those dependencies. A stale collector makes only its metrics unavailable. Duplicate metric
ownership fails, including duplicates between the legacy file and named collectors and claims
from stale inputs; migrate an old owner explicitly before publishing the same metric elsewhere.

Discovery reads only direct `<collector-id>.json` children, in deterministic order. IDs use
1–64 lowercase letters, digits, dots, underscores, or hyphens and start with a letter or digit.
A suite permits at most 32 input files including its legacy file, with an 8 MiB limit per regular
JSON file. Put raw traces elsewhere (for example `eval/inputs/evidence/`), not beside collector
inputs. Each collector should atomically replace only its own input file.

Result `costs_usd` and `wall_seconds` maps retain each named collector under
`<suite>/<collector-id>`; legacy files retain their original `<suite>` key. `collectors` records
the input path, exact input SHA-256, the payload's `collector` label/version, and owned metric IDs.
The payload `collector` field may describe an evaluator version independently of the stable
filename. Old result records without the added `collectors` field remain readable.

The JSON schema corresponds to public Rust types in `src/eval/measurement.rs`. The following is
a schema example, **not a measured result** (replace hashes with actual SHA-256 values):

```json
{
  "schema_version": 1,
  "suite": "objects",
  "collector": "objects-evaluator-v1",
  "implementation": [
    {"path": "src/source_index/figures.rs", "version": "v1", "sha256": "<actual hash>"}
  ],
  "truth_sets": {
    "K1": {"path": "eval/truth/objects.json", "version": "k1-v1", "sha256": "<actual hash>"}
  },
  "metrics": {
    "O1": {
      "sample": {"method": "f1", "true_positive": 90, "false_positive": 10, "false_negative": 10},
      "cases": 100,
      "evidence": [
        {"path": "eval/inputs/object-observations.json", "version": "v1", "sha256": "<actual hash>"}
      ]
    }
  },
  "cost_usd": 0,
  "wall_seconds": 1.2
}
```

Evidence files must be safe repository-relative files; escaping paths and `.env`/`.secrets`
paths are rejected. External corpus files remain outside the repo; reference a committed derived
manifest or truth labels here. `version` cannot be blank. Every observation requires nonempty
implementation, truth, evidence, and a positive evaluated case count; an empty test population
cannot establish a zero-error gate. Missing/changed files make the metric unavailable. Unknown
metric IDs, wrong suites, unsupported schemas, negative costs, and out-of-domain values fail.

Supported samples:

- `value`: a finite `value` computed by the collector (e.g. B-cubed F1, counts, or efficiency).
- `ratio`: `numerator` / positive `denominator`; an empty denominator is unavailable.
- `f1`: `2 * true_positive / (2 * true_positive + false_positive + false_negative)`.
- `mean`, `median`, `p95`: a nonempty `values` array; median averages the two central values and
  p95 uses nearest rank. Nonfinite samples are unavailable.

Fractions use 0–1, milliseconds and seconds retain the scorecard's units, costs are USD, panel
ratings use 1–5. Unknown model cost is `null`, never a fabricated zero. `wall_seconds` is the
collector's measured runtime; R4 additionally records per-paper/reference/list cost and time.
Compound IDs are explicit: `G1.works`, `G1.persons`; `O9.title`, `O9.first_author`, `O9.year`;
`O10.precision`, `O10.recall`; `O12.precision`, `O12.recall`; `O21.bibtex`, `O21.ris`;
`O22.resolved`, `O22.fabricated`; `O26.render`, `O26.search`; and the six `R4.*` rows. See the
scorecard for the full registry. Both G1 components must pass for G1 to pass; every component of
an objective must meet its target for that objective to count. Run both `resolution` and
`acquisition` for identifier resolution changes affecting O12. O30 belongs to `scale`.

## Ratchets and explicit resets

The first successful measurement establishes its baseline. Every subsequent successful run
ratchets upward for quality and downward for latency, counts, and calls/reference. Objectives
permit 0.01 absolute fraction regression and 10% relative latency regression; other units have
zero tolerance. A permitted decline never lowers the baseline, preventing cumulative drift.
Hard gates use exact thresholds with no tolerance and cannot be waived. R metrics have no target
or ratchet. An entire run with a failed gate or unjustified regression leaves baselines intact.

An objective may be explicitly reset using `--justification eval/justifications/<name>.json`:

```json
{
  "O1": {
    "reason": "Explain the changed measurement and why the new baseline is justified.",
    "evidence": {
      "path": "docs/experiment-reports/<date>-object-audit.md",
      "sha256": "<actual hash>",
      "version": "audit-v1"
    }
  }
}
```

The reason and hashed report are retained in `eval/baselines.json` adjustment history, along
with previous and new values. Every first-parent Git commit touching the baseline file is
audited, including deletions: committing a weaker number cannot bypass the ratchet. Each
weakening must append a new adjustment matching the previous value, and its report must already
exist with the recorded hash at that commit. Later report edits neither invalidate a justified
historical change nor repair a previously unjustified one. Existing adjustment history cannot be
removed or modified. Commit an improved baseline before resetting from that improved value;
several directly chained justified resets may share a commit. Shallow repositories must fetch
their history before evaluation can verify this invariant. Target changes require evidence in the design plan and a code
review of `registry.rs`; justification only changes the objective baseline.

## G5 isolation

G5 requires Linux `unshare --user --map-root-user --net`, Python 3, the installed Rust toolchain,
Node/npm, and the project's cached dependencies. It does not install anything. The runner checks
installed compiler tools including `mold`/`ld.mold`; a regression links a fresh executable using
the production restricted PATH, selecting mold when installed. The runner also checks
that the network namespace changed and only loopback exists, probes a documentation-only
address, and constructs an allowlisted PATH with no `codex`, `claude`, `gemini`, `aider`, or
`ollama`. It executes Rust tests with Cargo offline, all `test*.py` unittest suites beneath
nonhidden `scripts/` directories (collecting both `test*` and `should_*` methods and rejecting
empty suites), and all
`test`/`test:*` frontend package scripts (rechecking PATH after npm adds its executable paths),
plus frontend `web/scripts/*.test.mjs` files not named or statically imported by those commands, retaining separate logs plus `evidence.json` beneath `target/eval-g5/<run>/`. It fails
closed if isolation or a command fails; no missing-tool condition produces a pass.

Browser smoke scripts are separate phase/system verification because some require absent corpus
fixtures or a running app. G5 intentionally covers the unit test suites, whose contract forbids
network and model calls. A nested `eval all` during G5 is unavailable to prevent recursive runs.
