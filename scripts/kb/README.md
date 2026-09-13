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
must be 1–1,000. Titles use the stored normalized key, and names use display names.

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
python3 scripts/kb/rebuild-eval.py --truth eval/truth/reference-resolution.json
cargo run --offline -- eval resolution --check
```

Until the actual K2 truth file exists, the collector reports unavailable and removes only its
own stale generated input. A synthetic unit fixture never establishes G4. The initial adapter
expects the following shape from E8.4's real deposited-reference build (coordinate its schema
when K2 lands):

```json
{
  "schema_version": 1,
  "truth_id": "K2",
  "version": "the-real-truth-build-version",
  "records": [
    {"retrieved_at": "2026-09-13T00:00:00Z", "crossref": {"DOI": "<real DOI>", "reference": []}}
  ]
}
```

This example illustrates the shape only. At least one genuine deposited DOI reference pair is
required. `crossref` is the original cached Crossref message object, with its title, authors and
references; the explicit truth build retains provenance. The collector sets up an isolated root
under `~/.cache/lysilogy/kb-rebuild-eval/`, admits those real records via production APIs, and
creates a deliberate duplicate of one exact DOI source to exercise canonical alias replay.
It compares complete logical snapshots before rebuild, after transactional rebuild, and after
deleting only its evaluator-owned SQLite file and rebuilding from the same canonical records.

The check includes entities, stable IDs, observations, versions, local copies if any, authorships,
citations and their distinct evidence records, aliases, decisions and list mirrors. K2 setup does
not fabricate local copies or PDFs. Generated observations and independently owned collector
inputs remain in ignored `eval/inputs/`; a final report retains measured values and provenance.
