# Provider lock lifetime correction (#83)

Provider request ownership now ends when its Rust guard drops. The guard explicitly unlocks
the file before releasing the process-local mutex, including failures while reading or parsing
saved budget state. A duplicated descriptor can no longer prolong an already-finished request's
connection lock. Request spacing, window quotas, persisted cooldowns and admission rules are
unchanged, and the original restart test still requires at least 599 seconds of a 600-second
cooldown.

The original intermittent E0.1 test failure did not record its returned value. A separate,
deterministic reproduction confirmed the defect: after ending the request while a duplicate
descriptor remained open, the unchanged implementation returned a one-second connection-lock
deferral instead of the saved cooldown. The new regression was run before the fix and failed
with that exact result. This establishes the defect without claiming certainty about the
original intermittent run. Committed tests use `File::try_clone`, without unsafe fork code,
network calls or model calls.

Two added regression tests verify the saved cooldown remains observable while a duplicate
outlives the lease, and verify lock release after both non-file state and malformed JSON errors.
All eight budget tests pass, including concurrent quota exhaustion, active connection exclusion
and the existing restart cooldown check. The production guard owns the lock before state I/O
begins and remains inside the HTTP request lease until response handling finishes.

Before and after `eval scale --check` measured **O30 = 0 violations / 10,000 references**.
Both traces exercised 216 deferrals, 72 cooldowns and 32 serialization/reload cycles, with
2,500 admissions per provider. `eval tests --check` passed G5 before and after: 275 then 277 Rust
tests, plus 50 Python and 84 Node tests in each isolated run. Formatting and strict clippy
passed. The fixed source was measured at clean commit
`76a91c279274b6679d4bddf76c18fd8407a6c339`.

The final collector took 0.463 seconds and G5 took 6.015 seconds. The initial collector's
95.071 seconds included the new worktree's cold compilation; it is not a comparable throughput
baseline. All model cost was $0. The scorecard remains at 1/5 gates passing and 1/30 objectives
at target; other metrics are unavailable. No target or baseline was lowered.

Full before/after collector fingerprints, isolation evidence, counts and the failing regression
output are retained in `eval/evidence/provider-lease-unlock.json`. Reproduce with
`python3 scripts/eval/provider-budgets.py`, `cargo run -- eval scale --check` and
`cargo run -- eval tests --check`. All fixtures used isolated temporary roots; no user library,
notes or provider storage was accessed.
