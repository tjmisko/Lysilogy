# E8.2 corpus proxy transport follow-up

Issue #85 fixes the corpus client's inability to use an explicitly approved proxy in a managed
environment without direct DNS access. The first live direct attempt failed all six attempts
before committing metadata: 93.08 seconds and 30,096 KiB peak RSS. Storage had already been
repaired. A subsequent proxy probe independently encountered the tool's host permission boundary.

The client now accepts `--proxy-env HTTPS_PROXY` or `--proxy-env https_proxy`, reads only that
process variable, and uses its HTTP CONNECT proxy for the existing HTTPS destinations. Direct
transport remains the default. Missing or invalid configuration fails explicitly. Python's
standard transport does not provide TLS to an HTTPS-scheme proxy, so such URLs are rejected;
certificate and hostname verification of the origin remain enabled. The design and operations
documentation record this explicit transport choice and its separate permission boundary.

An independent review found the unsupported TLS-proxy scheme and exception-context leaks during
response cleanup or interruption. Both were corrected before final validation. The redaction
boundary covers connection setup, reads, cleanup and durable cooldown persistence, while
cancellation still propagates. HTTP error responses close even when persistence is interrupted.
Host restrictions, redirect refusal, the shared three-second rate lock, retry budgets, durable
Retry-After deadlines, bounded responses and the disk floor retain their existing behavior.

Validation at implementation head `a92c9944baf3e213035784b221a031274fe75d64` passed:

- `cargo fmt --all -- --check`.
- `cargo clippy --offline --all-targets --all-features -- -D warnings`.
- `cargo test --offline --all-targets`: 277 Rust tests.
- `python3 scripts/eval/provider-budgets.py` and `cargo run --offline -- eval scale --check`:
  O30 remains zero violations across 10,000 admissions.
- `cargo run --offline -- eval tests --check`: G5 passes 277 Rust, 61 Python and 84 Node tests
  in 6.05 seconds. The Python total contains 52 corpus tests and 9 eval-tooling tests.

The 11 new corpus tests cover explicit variable selection, ignored ambient proxies, malformed
configuration, unsupported TLS-proxy schemes, destination restrictions, request and response
credential redaction, cross-client pacing and cooldown persistence, interruption, and CLI
dispatch. All tests run without network or model calls. No frontend source changed. Cargo used
one job, debug information disabled and incremental compilation disabled. Heavy gates waited
until the separate 10k benchmark's timed phases had completed. The earlier Clippy process was
interrupted by the daemon restart and its partial log was not counted as a pass.

The retained historical pre-change baseline has G5 passing and O30 = 0/10k, with 277 Rust,
50 Python and 84 Node tests; the issue's original corpus-only run passed 41 tests. An immediate
pre-change eval run was not retained, so this comparison is explicitly historical rather than
a claimed fresh before/after experiment. The fresh scale run records a clean source; G5
conservatively records dirty state after scale generated its scorecard. Neither source file
changed, and their hashes are retained with the full eval results and isolated-test receipt in
[`eval/evidence/corpus-proxy-transport.json`](../../eval/evidence/corpus-proxy-transport.json).
No hard gate, objective target or baseline was lowered. This worktree has one passing gate
(G5) and one objective at target (O30); corpus-dependent metrics remain unavailable.

After the user restarted the daemon, the root agent received a valid 1,890-byte OAI-PMH
Identify response through the explicit proxy and launched the resumable build from this source.
At 2026-09-13 06:32:45 UTC, the root agent observed 53,300 committed metadata records and an
incomplete `cs:cs:CV` harvest, then confirmed that the process remained live. The frozen selection
did not yet exist and no PDFs or sources had been downloaded. These bounded observations are
retained in the evidence receipt without proxy configuration or credentials.
The live operation is separate from offline validation and remains owned by the root agent.
This is transport evidence, not complete K0 acceptance: the metadata harvest, frozen selection,
both downloaded tiers and full verification still need their own final results. Selection's
peak RSS remains unmeasured. This change and its validation made no model calls and cost $0
in model usage.
