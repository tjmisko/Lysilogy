# E2.1 store: offline review checkpoint

Issue #33 remains incomplete. Registry access to index.crates.io and static.crates.io is denied;
no dependency download, new host probe, Rust build, Clippy, Rust tests, G5, G4 or O28 ran during
this correction checkpoint. No gate or objective is newly established. The existing baseline
and unfinished historical patch remain under the worktree target directory.

The corrections address all five retained storage review findings:

- Aggregate all current canonically bound observations after admission, merge and split. Stable
  observation-ID ordering selects default scalar values; complete competing entity assertions,
  admitted revision hashes and raw observations remain queryable. Parsed Person components stay
  together. Identifiers, versions and local copies are unions; refresh removes only its own
  contribution. Contradictory same-key copies or versions fail before journal append.
- Validate canonical Person identifiers and WorkVersion identifiers before projection. ORCID
  checksum validation does not establish registry assignment. Source strings remain retained.
- Recompute title keys through the shared E2.4 function for FTS population and queries.
- Add a nonunique covering identifier index and bounded candidate APIs; no identity merge is
  inferred from a shared identifier.
- Detect changed journal prefixes before incremental writes and validate cached hashes before
  clearing for rebuild. Schema 3 has a completion marker so a failed projection upgrade retries.

Eight added Rust scenarios cover contributor refresh/removal/split, conflicting versions/copies,
nonunique indexed candidates, stale title keys, same-length journal tampering, invalid Person
identifiers, coherent Person aggregation and interrupted projection upgrade. These tests are
written and formatted but **unrun**. Existing migration expectations now use the schema count.

`cargo fmt --all -- --check` and `git diff --check` pass. A direct Python sqlite3 audit executed
all three SQL migrations, checked foreign keys and verified that duplicate identifiers return
both entities through the covering lookup index. This validates SQL syntax/index shape only;
it does not substitute for running the Rust store or its quality gates.

The G4 adapter's provisional raw-record input must be replaced with actual E8.4 K2 plus its frozen
Crossref provenance before measurement. No synthetic PDF, reference record or fixture establishes
G4. O28 still requires the full prescribed 500k-Work/3M-edge run once dependencies are available.
The main checkout's preview changes and the pre-existing worktree `.gitignore` addition were
left untouched. R4 for this offline work: zero provider/model calls, zero model cost.
