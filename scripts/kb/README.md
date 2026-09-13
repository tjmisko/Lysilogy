# Knowledge-base storage and measurement

`lysilogy --data <data-root> kb rebuild` opens/migrates `<data-root>/kb/kb.sqlite` and replays
canonical allocation/admission/decision records plus reading-list mirrors. A fresh root creates
an empty migrated projection. The command does not initialize the PDF library, discover files,
load application configuration, call providers/models, or read notes. It never changes the
canonical `paper-identities.json` registry. Rebuild updates projection rows in one transaction
inside the existing WAL database; it does not rename or delete a database in use.

`KbStore::allocate_work` and `allocate_person` persist one random ID per stable origin before
returning it. `admit` verifies a relative source file's SHA-256 and raw payload, retains its exact
revision, and records an observation binding. Adapters own semantic acceptance of extracted or
provider metadata. Cache discovery alone does not create entities. `record_decision` stores
already-authorized typed merge/split/distinct decisions; it is not an automatic entity matcher.

Canonical paths:

- `kb/decisions.jsonl`: versioned allocation, admission-reference and Decision envelope records;
  sequential IDs and a hash chain detect edits, missing records and incomplete tails.
- `kb/admitted/<sha256>.json`: the exact accepted per-paper/provider observation revision and
  typed projection. Mutable source files and expiring provider caches cannot erase it.
- `kb/lists/<id>.json`: canonical list JSON, mirrored without rewriting its content.
- `paper-identities.json`: the independent existing PDF identity registry, left intact.

The store rejects malformed canonical data instead of truncating or replacing it. Keep an
incomplete journal or modified retained artifact for diagnosis; repair canonical state deliberately.

The production neighborhood query uses a consistent SQLite snapshot across root resolution,
the two-hop traversal and induced edge lookup. The outgoing primary key and incoming citation
index cover both directions. Public results include explicit truncation flags. Trigram searches
accept literal phrases of at least three characters and at most 4,096 bytes; their result limit
must be 1–1,000. Titles recompute the shared normalized key for both population and queries; names use display names.

Current metadata aggregates all canonically bound observation revisions in stable observation-ID
order. Refreshes replace one contribution; merge/split rebuild the affected aggregates. Complete
competing assertions and their provenance are available through `KbStore::assertions`. The scalar
precedence is a storage default pending resolver policy. Identifier lookups return nonunique
canonical candidates with explicit truncation and never decide identity merges. Work candidate
lookup indexes explicit Work identifiers only, excluding WorkVersion-only identifiers. Work-level
arXiv IDs must be versionless; version-specific IDs remain on WorkVersion. Lookup never strips a
version or invents a missing Work identifier.

## O28 graph benchmark

```sh
python3 scripts/kb/graph-bench.py
python3 scripts/eval/provider-budgets.py
cargo run --offline -- eval scale --check
```

The release example creates a fresh run under `~/.cache/lysilogy/kb-graph-bench/`: exactly 500,000
Works and 3,000,000 distinct nonself directed edges, including concentrated incoming hubs. It
runs 200 complete production two-hop queries (40 hub, 160 uniform) and independently checks
node/edge membership by adjacency BFS outside timing. Timings include SQL traversal, sorting,
materialization and conversion into the public result types. They exclude graph setup and truth
checking. No warm-up observations are removed. The collector requires exact inventory, FTS5,
the prescribed query mix, correct results and no truncation before publishing O28.

A small implementation smoke can run the example with `--works 1000 --queries 200 --data <fresh>`.
It does not publish scale metrics. Ordinary unit tests use small fixtures and never run the full
benchmark. All build/data directories remain on disk, outside `/tmp` and the user library.

## G4 rebuild collector

```sh
python3 scripts/kb/rebuild-eval.py --truth eval/truth/reference-resolution.json \
  --frozen-root ~/.cache/lysilogy/reference-truth \
  --manifest ~/.cache/lysilogy/reference-truth/crossref-manifest.json
cargo run --offline -- eval resolution --check
```

Until the actual K2 truth file exists, the collector reports unavailable and removes only its
own stale generated input. A synthetic unit fixture never establishes G4. The evaluation-only adapter lazily loads E8.4's
`scripts/truth/reference_truth.py`; until that reviewed loader is present, it reports unavailable.
The production store has no dependency on the truth builder.

Supply actual K2 (`schema_version: 1`, `truth_set: K2`, `sources`, `works`, `cases`, `coverage`,
`built_at`, content-derived `version`) and its frozen Crossref manifest. The loader verifies exact
receipt/body bytes and original timestamps. The adapter reconstructs K2 from its retained work
origins and missing requested DOI identities, then requires full equality, including source/case
identity and content version. Extra or refreshed Crossref snapshots fail validation.

Only after this check does an isolated `verified-input.json` under the external derived cache
carry original Crossref messages, fetch times and K2's deposited DOI case IDs into the production
allocate/admit scenario. Invalid or absent reference DOI fields remain in the original message;
they produce no citation edge. No input field is reconstructed from a predicted entity. The Rust
example accepts this explicitly tagged derived input, not the retired provisional `K2.records`
shape. It creates one deliberate duplicate exact-DOI source to exercise canonical alias replay.
The collector verifies frozen/K2 bytes again afterward and retains loader/source/manifest hashes.

It compares complete logical snapshots before rebuild, after transactional rebuild, and after
deleting only its evaluator-owned SQLite file and rebuilding from the same canonical records.

The check includes entities, stable IDs, observations, versions, local copies if any, authorships,
citations and their distinct evidence records, aliases, decisions and list mirrors. K2 setup does
not fabricate local copies or PDFs. Generated observations and independently owned collector
inputs remain in ignored `eval/inputs/`; a final report retains measured values and provenance.
