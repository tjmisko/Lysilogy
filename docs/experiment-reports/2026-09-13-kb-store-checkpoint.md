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

Nine added Rust scenarios cover contributor refresh/removal/split, conflicting versions/copies,
nonunique indexed candidates, stale title keys, same-length journal tampering, invalid Person
identifiers, coherent Person aggregation, interrupted projection upgrade and the arXiv Work/
WorkVersion identifier boundary. The last test covers modern and legacy arXiv forms, rejected
admission with unchanged journal, accepted version-specific metadata and explicit lookup scope. These tests are
written and formatted but **unrun**. Existing migration expectations now use the schema count.

`cargo fmt --all -- --check` and `git diff --check` pass. A direct Python sqlite3 audit executed
all three SQL migrations, checked foreign keys and verified that duplicate identifiers return
both entities through the covering lookup index. This validates SQL syntax/index shape only;
it does not substitute for running the Rust store or its quality gates.

The G4 adapter now consumes actual E8.4 K2 and a verified frozen manifest through a lazy,
evaluation-only loader. It reconstructs K2 and requires complete equality before passing original
Crossref messages, original fetch times and deposited case IDs into the production allocation and
admission scenario. It retains frozen source/manifest/loader fingerprints and revalidates after
measurement. The store itself does not depend on the unmerged truth builder. Missing E8.4 code
or genuine K2 leaves G4 unavailable. The previous provisional `K2.records` contract is retired.

The standalone KB Python suite passes eight existing tests and explicitly skips three new adapter
tests because E8.4 is not in this branch. Loading the reviewed E8.4 `reference_truth.py` read-only
from its separate worktree makes all seven rebuild-collector tests pass, including the three
adapter cases: original records with invalid/unlabeled references, rehashed label drift and receipt
time drift. These use invented tiny fixtures and cannot establish G4. Rust example execution,
Clippy and the store's Rust tests remain unrun. O28 still requires the full prescribed
500k-Work/3M-edge run once dependencies are available.
The main checkout's preview changes and the pre-existing worktree `.gitignore` addition were
left untouched. R4 for this offline work: zero provider/model calls, zero model cost.
