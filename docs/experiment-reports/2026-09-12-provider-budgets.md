# Provider cache and budget measurement (E7.1)

The configured provider budget admitted all 10,000 simulated references with **0 violations**
(O30 target: 0). Each of OpenAlex, Semantic Scholar, OpenCitations and Crossref handled 2,500
references. The trace includes 216 deferred admissions, 72 injected server cooldowns and 32
serialization/reload cycles. No network or model was used; model cost was $0.

The recorded fixture is `eval/truth/provider-budget-batch.json`. It configures 1,100 ms minimum
spacing, 50 requests per 60,000 ms window, a 37-second cooldown every 137 admissions per provider,
and a state reload every 311 admissions. `examples/provider_budget_eval.rs` runs the same
`BudgetState::reserve` and `BudgetState::cooldown` methods used by production HTTP leases.
`ProviderBudgets::acquire` persists admissions before network dispatch and holds the provider's
process/file locks through response consumption.

`scripts/eval/provider-budgets.py` independently checks observed grants for spacing, window
quota, active server cooldowns, duplicate admission and complete reference coverage. It also
checks that all configured providers, cooldowns and restart cases were exercised. Its offline
unit tests deliberately violate spacing, window and cooldown constraints and confirm that the
auditor detects them. The simulation validates the configured admission algorithm; separate
transport/storage tests verify its integration, concurrent exhaustion and restart persistence.
It does not claim a live provider throughput or account-credit measurement.

The final measured collector wall time was 0.448 seconds with cached Cargo artifacts. The trace
SHA-256 was `652cce8669163944e95582aa51a81cb3781ed1388fe75a36842202230707d030`; the
fixture SHA-256 was `3272d90d6aa27b2c8081ef5ff6e7c823d840bc645bf7c6a69859e83d8f6b5cd9`.

The generated trace and evidence hashes are retained in
`eval/inputs/provider-budget-observations.json` and `eval/inputs/scale/provider-budgets.json`.
The collector fingerprints its implementation, the Rust example, the checked-in fixture,
Cargo manifest/lock and the exact trace. Reproduce with:

```sh
python3 scripts/eval/provider-budgets.py
cargo run -- eval scale --check
```

Before this implementation, O30 was unavailable: E8.1 had not yet merged and there was no batch
collector. The first measured value is 0, reaching the unchanged objective. No follow-up objective
miss is needed. The collector's measured wall time is stored in its generated input; it includes
the offline Cargo invocation and audit, rather than an invented simulated-time cost.

After integrating E8.1 with a preserving-history merge, Rust formatting and strict clippy
passed; 243 Rust unit and 12 integration tests passed, including 16 new cache/budget/transport
tests. `eval scale --check` measured O30 = 0 and ratcheted its first baseline to zero. A fresh
`eval tests --check` passed G5 in a new network namespace with no model CLIs: 255 Rust, 49
Python (including four independent budget audit tests), and 84 Node tests. G5 took 6.181 seconds.
The generated scorecard records 1/5 gates passing and 1/30 objectives at target; other metrics
remain unavailable. Existing frontend dependencies were accessed through a worktree symlink.

Retained evidence is `eval/evidence/provider-budgets-initial.json` (the full collector input,
observed counts and hashes) and `eval/evidence/g5-provider-budgets.json` (isolation and command
results). Raw generated traces and command logs remain ignored and reproducible. The collector
atomically replaces only its own trace and input, allowing disjoint scale collectors to coexist.

Default shared storage is `~/.cache/lysilogy/providers/`, independent of library/data roots. A
service may pass an explicit shared root and per-provider policies; isolated tests use temporary
fixture roots. No user library or data root was changed during this implementation. Runtime
provider calls remain subject to the environment's network/storage permissions; this report
contains no live provider success claim.
