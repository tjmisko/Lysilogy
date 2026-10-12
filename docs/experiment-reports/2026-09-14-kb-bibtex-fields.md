# Complete BibTeX source fields (E8.3, #109)

The evaluation reader now preserves complete BibTeX fields in the original decoded
source. Literal percent text no longer disappears through TeX comment masking,
and brace-protected internal quotes no longer truncate quoted values. The finite
scanner retains every selected field/value span and source hash, checks its full
tail, and preserves unknown macros, concatenations and all duplicate occurrences.
It also supports reference-confirmed apostrophe/parentheses keys and underscore/@
metadata prefixes. It never executes deposited TeX or BibTeX.

Database fields remain supplementary evidence. They cannot establish absent
printed BBL roles, override those roles, or turn source recovery into accepted
truth. All raw `^^`, ambiguous-label, source/math/native guards and retained v1/v2
implementations and truth remain unchanged. No detector, scorer or target changed.

## Frozen comparison

Selection uses the same original 1,000-paper ledger and original 999-success index
map. The executed cohort contains exactly the eight original quoted-field failures
and all 702 reviewed #107 source successes (612 historical plus 90 recovered there),
in frozen corpus order. The other 290 rows remain historical and unexecuted; this
is **not a current full-1,000 run**. Selection does not depend on new outcomes.

| Generation | Executed papers | Source inventories / candidates | Time | Peak child RSS |
|---|---:|---:|---:|---:|
| `acf77a9`, first eight | 8 | 3 / 3 | 5.438177 s | 169,984 KiB |
| `142b835`, first eight | 8 | 5 / 5 | 7.339952 s | 171,152 KiB |
| `142b835`, full comparison | 710 | 683 / 683 | 848.687413 s | 335,680 KiB |
| `6a9d65a`, first eight | 8 | 5 / 5 | 7.346050 s | 170,608 KiB |
| `6a9d65a`, final comparison | 710 | **689 / 689** | **859.394191 s** | **332,656 KiB** |

The final generation recovers five of the original eight failures and preserves
684 of the prior 702 successes (598/612 historical and 86/90 recovered by #107). It retains 21 failures: 18 previous successes are
now explicitly withheld, and three original failures remain. The net source count
is 13 below the previous parser across this fixed 710-paper population; this is
not an unqualified source-coverage gain. Every generation has zero raw accepted
candidates and zero eligible O1–O11 metrics. No automatic truth was published.

All 683 inventory/candidate pairs shared with the preceding `142b835` generation
are byte-identical. The narrow final correction recovers six additional papers
without losing another prior parse; the remaining 21 error strings are unchanged.
Against #107, every surviving complete object/reference inventory and printed BBL
entry identity is preserved. The only entry differences are supplementary
`field_conflicts` metadata: 13,380 rows across 303 papers. Printed field labels and
all other entry fields are unchanged.

Independent review rehashed 4,549 artifacts and checked 206,806 original field
spans for the final generation. It separately bound both source/module generations,
all ledger outcomes, the complete surviving inventories and the six new recoveries.
All original inputs, intermediate failures, unlaunched preparations and review
corrections remain retained. The experiments made no network/model calls and incurred zero external service cost;
agent reasoning cost is not measured.

## Remaining source exclusions

These are the first encountered failures in each complete parse attempt, not an
exhaustive catalogue of all defects in those databases. No failed tail is skipped
or repaired. Generated-only reference probes establish the tested lexical behavior;
ordinary unit tests remain inert and require no installed BibTeX program.

| Boundary | Prior-success paper IDs | Disposition |
|---|---|---|
| In-record percent token | 2109.04966, 2401.14013, 2201.05950 | Malformed field syntax in the retained reference tests; percent outside a value is not silently stripped as a TeX comment. |
| Missing comma | 2002.03113, 2212.08979, 2208.11323 | Malformed field delimiter; no punctuation repair. |
| Bare multiword value | 2103.02733 | Malformed unquoted/unjoined value; no inferred quoting or concatenation. |
| Missing equals sign | 2010.02153 | Malformed field assignment. |
| Multiple commands on one physical line | 2201.01806, 2112.14697, 2003.13370, 2205.09070, 2301.11955, 2311.18071 | Deliberately unsupported outer line behavior; no silent skipping of later commands. |
| Empty key | 2409.11682, 2107.09150, 2203.01177 | The reference accepts this syntax, but empty database keys remain unsupported. No synthetic key or BBL association is invented. |
| Record/atom cap | 2401.10937 | Existing cumulative 20,000-step resource bound. This is not a malformed-source claim or a reason to raise the cap for one paper. |

The three original failures that remain are 2010.02241 (empty field value),
2104.06866 (same-line commands) and 2005.05130 (spaced `ARTICLE NUMBER` field name).
The retained classification binds every ID to its original archive/member hashes,
exact source offsets and reference evidence. Approximately 500 defensible stratified
papers remain the open coverage target in #97; these lexical results do not meet it.

## Validation and attribution

Final integrated source `19f549e` passes formatting, strict all-target/all-feature
Clippy, all-target Rust tests, the Python suites and offline G5: **368 Rust,
505 Python and 85 Node tests**. Both immutable truth versions replay successfully.
The twelve-command gate run took98.014107seconds with1,268,704KiB peak child RSS.
Its first objects result correctly remained unavailable because the parser
fingerprint was stale; an exit0 alone was not treated as metric availability.

The reviewed actual `--executable` collector refresh then took28.447967seconds
with277,792KiB peak child RSS. The newly available own-CLI result measures
O1=1 and O2=0.9169720168893188, with all three complete paper records,23 outcomes,
full prediction bytes, truth, coverage and external-input hashes exactly matching
incoming main. One zero-IoU region remains in the full denominator. Independent
actual review found no unresolved findings. The raw Cargo/G5 logs, final CLI,
bridge and all392 gated source files are retained outside the worktree.

Initial wrapper-review findings and the stopped preparation caused by generated
G5 bytecode remain recorded; the corrected wrapper rejects stale result selection
and build-only dispatch. No production source changed after the measured gates.
Ordinary main merges replaced rebase under the standing permission restriction.
The before-change executable is retained with its original receipt hash.
Both final executables are also archived.


The clean before-change own-CLI objects check took 94.620132 seconds including
build and measured O1 = 1 and O2 = 0.9076006899903768 on the unchanged three-paper,
23-object v2 truth. Incoming #111 accounts for the later O2 = 0.9169720168893188.
Issue #109 has no detector/scoring/truth contribution to that change. The final
refresh preserved all incoming paper records, predictions, counts, geometry,
truth and external-input hashes exactly and produced a newly available objects
result. Raw source-parse loss is separate from accepted K1 object performance.

Evidence is retained under `~/.cache/lysilogy/bibtex-fields/`; committed
`eval/evidence/bibtex-fields.json` binds exact source, inputs, scripts, receipts,
raw test/build logs, executable archives and review records. Historical generations
and immutable truth remain independent of the disposable worktree.
