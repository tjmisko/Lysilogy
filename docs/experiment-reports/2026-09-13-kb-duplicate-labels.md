# Distinct source objects with ambiguous labels (E8.3, #107)

Duplicate labels previously aborted a complete source parse before any alignment.
The frozen `542da50` run contains 90 such failures across all seven categories.
The source change preserves each object occurrence and withholds ambiguous naming
relationships. It does not resolve labels from printed numbers or detector outputs.

Every object carries a deterministic commitment to its kind, environment, expanded
offsets, ordered source members and source-project identity. Unique existing IDs stay
unchanged; every member of a colliding ID group receives a source-occurrence ID.
The label ledger retains repeated commands on the same object and non-object claims
from section, item, subfigure/table and bibliography scopes. Reference and explicit
proof targets remain unresolved when any requested label is ambiguous. Unnamed proofs
retain the confirmed nearest-preceding source-statement rule. Both current manual
validators reject attempts to turn an ambiguous name into an object or section target.

The source-only correction also conservatively withholds explicitly defined known
source primitives, including a single redefinition of `label`; localizing duplicate
names cannot certify source execution that the parser does not understand. Existing
unknown-style, conditional, stored-argument, math fidelity, native-span uniqueness and
complete-kind guards remain. Duplicate bibliography keys are still a separate failure.
The current contract documents this distinction; retained v1/v2 source, payloads and
configuration remain unchanged.

## Frozen comparison

The selection is fixed before new parsing: first one prior duplicate-label failure
per category in corpus order, then all 90 prior duplicate failures and the 612 prior
successful parses. The remaining 298 historical failures stay in the original
1,000-paper population ledger with an explicit unexecuted disposition. They are not
current-source results. The input SHA is
`2d4504df1208d27dc56ddae5549d1a49465d5db8fff06765215d0351a63cf8e0`;
the original 999-success index-map SHA is
`40b0545ac4f34c4c1d389f3f7bbdb8b613a533cef6e566285b570ccd7ea0f785`.
The separately recovered thousandth native index is outside this comparison.

The offline runner retains source parsing before unchanged alignment, preserving a
parsed inventory even if alignment later fails. Each paper has the existing 30-second
source/alignment and 1.5-GiB address-space bounds. It verifies actual source/PDF/index
hashes and compiles exact retained module bytes directly; normal Python bytecode imports
were rejected during runner review. No deposited TeX is executed, no native index is
rebuilt, and no model or network calls are allowed. Raw eligibility is a diagnostic,
not published truth. The approximately 500-paper coverage target remains open in #97.

## Validation status

The clean own-worktree baseline at `3b44b1e` built the CLI offline and passed
`eval objects --root <worktree> --check` in 92.036 seconds, with 1,016,096 KiB peak
child RSS and $0 external calls. Its exact selected CLI and source/raw-log hashes are
retained under `~/.cache/lysilogy/duplicate-labels/before-3b44b1e/` (receipt SHA
`07d7bcc3480a4ceaf86e0f2449fcef16d2b2fb5c8e46da315872affa1b3c3de5`).

Independent source review cleared `f24f735` after 257 focused offline tests and seven
additional probe groups, including 20,000 independently generated interval-owner
comparisons. The receipt SHA is
`53a9c5d4c278560f629cdae9ca66ed944e4cc5f78706d7b2296fb17fdaddde2c`.
New tests cover repeated same-owner claims, secondary aliases, object/non-object
collisions, named and unnamed proofs, repeated includes, fallback-name collisions,
residual digest collisions, bounded evidence, unknown execution and manual laundering.

The first seven diagnostic papers all recovered source parsing and candidate
assembly: 485 objects, 352 bibliography entries, 486 citation commands and 461
reference commands remain inventoried. There are 394 label occurrences, including
42 ambiguous claims across 20 names. Twenty-eight source reference occurrences
remain explicitly ambiguous. All seven retain their source/alignment exclusions:
**zero raw accepted candidates and zero eligible metric cohorts**. These counts are
parsed source evidence, not verified truth objects. The unchanged reviewed run took
4.695 seconds including its wrapper (4.539 seconds for source/alignment), peaking at
102,656 KiB RSS, with $0 external calls and no native rebuild. Its receipt SHA is
`355ba395fc11cbb6801821d1234b1883028fdf60cce4bdf83253521e02332209`.

The historical `f24f735` comparison completed all 702 source parses and candidate
assemblies in 1,105.420 seconds including its wrapper, with 265,104 KiB peak RSS.
All 90 prior duplicate-label failures recovered. A mechanical comparison of the 612
previous successes found no lost or changed object occurrence, source link, bibliography
entry, or metric eligibility. It separately retains changed naming tables, proof targets
and conservative source-primitive exclusions. The report contains 37,779 parsed objects,
28,054 bibliography entries, 39,274 citation commands, 54,166 reference commands and
32,756 label claims; 629 claims across 233 literal names are ambiguous. These are source
counts, not independently admitted truth. The sole raw accepted candidate is the already
known `2007.05954` empty O5/O6 case; no positive metric cohort or new publication resulted.
The receipt SHA is `41edd48d26ace1720881af91904c49c7c58c64d7e5956ddd4ef169fac5e0e4bd`;
the mechanical comparison SHA is
`58db87f511bece0edcc82089c917607285daf253d0d242bb96519aaeb9e9ba36`.

That completed generation remains historical because a subsequent audit exposed an
inherited reference grammar bug: plain `ref`/`eqref`/`autoref`/`vref` arguments containing
commas had been split like lists. The complete original key is now retained; only supported
`cref`/`Cref` and citation commands split lists. Proof headings use the same parser. The
actual original-source audit found 27 affected reference occurrences and one proof heading
in ten of the 702 papers, all retained by exact source-member hash. Independent review
cleared correction `e63c5e0` and reproduced that audit (receipt SHA
`dcfcb51325a549d37d2242107eef7ace2cf4b098bba5f82ed4e9d672e2a0fe9b`).

A separate bounded correction at `54095cd` retains but does not resolve label spellings
containing unverified expandable tokens. Such a naming claim may alias any literal key,
so it withholds the paper's proven label destinations; dynamic lookups alone are local.
Both manual reference paths and named proof attribution enforce the same boundary.
Source occurrence identity, detection completeness, and unnamed nearest-statement proof
linkage remain separate. A read-only audit of all historical inventories found zero such
label or reference spellings; this is a synthetic correctness fix, not a coverage claim.
The old/new synthetic probe is retained with SHA
`6f4063b1537939394c7ec4e7b2c16cca28945bccdf3a572aa5c67f0cba119c2f`.

Review of `54095cd` exposed a distinct lexical issue: TeX's `^^` notation can encode
structural commands before tokenization, so naming-only uncertainty was insufficient.
The paired inert probe showed one lexically parsed equation versus two in the explicitly
written decoded fixture (review probe SHA
`1109170e9349cef83718bd616007eb111da065308a480ba0c77696caae275b43`).
Correction `05d3335` scans raw deposited members before masking, retains bounded marker
positions and member hashes, and withholds all automatic inventory certainty without
performing substitution. The current manual codecs reject this unsupported source form;
retained historical codecs remain unchanged. Comments, definitions and unselected members
cannot erase this guard. Parsed rows remain diagnostics when tokenization is unverified.

The corrected source has 276 passing offline Python tests and independent source
clearance at `05d3335` (receipt SHA
`3d2c3c767385876abdc78644a5866003f77c573269939c075e1828239197feae`).
Its new frozen launch is `e41103ace2e7e723ce08e01b5763c382363b9e759e9924d9d78aa21f71be15a3`.
Independent snapshot review verified all 14 Git modules, the source-only loader,
the original population/index bindings and the exact executable (receipt SHA
`f19f33b17255f2142af509c6502d112f5969ad19a0c2dd8d00182a9ae768f521`).

## Corrected result

The corrected first seven again retained all 485 objects and recovered all seven
parses, with zero eligible metrics. The run took 3.354 seconds and 105,120 KiB peak
RSS. Independent review verified the complete inventories, all 947 source links,
the 28 ambiguous references and the original class-file lexical markers (receipt
SHA `4dbc2f3a47c41bafb341b90527b20790389a424a598da54596cc6253d6f81ef2`).

The corrected 702-paper comparison completed in **870.998 seconds**, with
**315,696 KiB peak RSS**, no per-paper failures and $0 external cost. It recovered
all 90 previous duplicate-label failures and preserved all 612 previous parses.
It retained all 37,779 object occurrence identities, source memberships, link
occurrences and 28,054 bibliography entries from the preceding generation.
Twenty-seven reference targets in ten papers now preserve their literal comma keys;
the separate original-source audit also retains the affected proof heading.

| Result | Historical `f24f735` | Corrected `05d3335` |
| --- | ---: | ---: |
| Source parses / selected papers | 702 / 702 | 702 / 702 |
| Candidate assemblies / selected papers | 702 / 702 | 702 / 702 |
| Recovered prior duplicate-label failures | 90 / 90 | 90 / 90 |
| Raw accepted candidates | 1 | 0 |
| O5 / O6 eligible papers | 1 / 1 | 0 / 0 |
| All other eligible metric cohorts | 0 | 0 |
| Newly published truth papers | 0 | 0 |

The one eligibility loss is deliberate. `2007.05954` previously supplied only empty
formal-statement/proof inventories. Its deposited `svjour3.cls` contains five raw
`^^` markers; the current source contract cannot prove that token stream. It now
withholds O5/O6 along with the other metrics. The earlier independent empty-inventory
review remains historical evidence; it does not override the current guard. Across
the selected 702 papers, 88 contain 463 raw markers, including deposited class files.
This broader scan preserves lexical diagnostics but makes no assertion that each
marker actually alters a printed object.

The corrected receipt SHA is
`a77d28ff2c0a49181bbc31d12cbd34f4ce88d56637d1add6a93ffff58f0132f9`;
the report and complete ledger SHAs are
`cd8b553701b769ff1318d28f3d236c76d5385b425d912328835a00434b376970` and
`4dc2517e867846197d1a580f19701446bba48e0fc913bc65d2d5b145c6730a9e`.
The standalone old/new payload comparison took 13.309 seconds and 101,696 KiB RSS
(output SHA `951f05dd5163d9597b8a8f93fdadc2511ec58cf940cd531a2bbb08178a87fc0d`).
That script binds the historical ledger and current receipt/ledger/payloads; the
separate source, launch and independent result reviews verify actual module and
generation identity. Its diagnostic comparison is not an admission validator.
The complete independent result review rehashed both generations and all canonical
inputs, recomputed every source occurrence commitment and verified the historical
612-paper inventory preservation (receipt SHA
`68e0b5326c91ca11d6ad18b59f56f0a73c1074940e4260b36c9cdbb379fab7b3`).

An ordinary merge of main `fa841d4` produced `405c6a3` after the run ended. All 14
executed derivation modules remain byte-identical, and all 37 retained release files
are unchanged. The 79 production, Cargo and collector files equal main (closure
receipt SHA `045e80cd9e283ca7fb154ad711dc465cfbcac3c3481ff82ab36b900e7aab5d0b`).
Main's separately merged #106 measurement has O1 = 1.0 and O2 = 0.9076006899903768
on the unchanged three-paper, 23-region K1 v2 cohort. The clean #107 before check
had O1 = 0.9787234042553191 and O2 = 0.7570080448318404. That detector improvement
belongs to #106; this evaluation parser change introduces no detector behavior,
scoring change, baseline reset or new truth value. A later refresh of that same
collector, described below, binds the unchanged measurement to current source bytes.

## Final checks and retained measurement

At integrated `df1a11f`, formatting, strict all-target/all-feature Clippy, Rust
all-targets, 276 focused truth tests, 35 collector tests, and objects/bibliography/
scale checks passed. The initial G5 run failed only because the fresh worktree lacked
TypeScript for the API Node test. That failure remains intact (receipt SHA
`1e775eec335b950ebe68662df073df4f39162dad6e62f5979e2bbda893fff931`).
The exact locked TypeScript 5.9.3 archive was read from the local cache, verified
against its SHA-512 lock integrity and safely unpacked into 132 individually hashed
files; no install scripts or network calls ran. The affected G5 check and remaining
replays then passed: **362 Rust / 421 Python / 85 Node tests**, with both v1/v2
payload pairs reproduced exactly. The retry receipt SHA is
`b0012190ff821f5dcccc9b9c74bec2b0377a8c2710774fe43da53f53107d8885`.
The initial checks took 68.345 seconds / 1,247,648 KiB peak child RSS; the retry and
replays took 16.708 seconds / 151,344 KiB. All 207 recorded source/evidence input
bytes were unchanged during those checks. Both G5 runs' 30 raw command logs and
their receipts are archived outside the worktree, including the original failure.

The first objects check exited successfully but reported O1/O2 unavailable: the
inherited input correctly detected four stale current-parser fingerprints even
though v2 replay uses retained modules. Exit status alone did not establish metric
availability. After independent plan/wrapper review, the unchanged current
collector was rerun on the same three frozen papers. It reproduced all three paper
records, 23 object decisions, complete predictions, object commitments and both scores exactly, then
the own-worktree CLI verified available O1 = 1.0 and O2 = 0.9076006899903768.
The refresh took 13.637 seconds / 276,384 KiB peak RSS and $0 external cost (receipt
SHA `44027ba4162479e09c16c5771eb89f39957b4efb5fcf405aa49508aca023b1de`).
It rebuilt only the current measurement bridge offline; it did not rebuild a native
index, change truth/scoring/targets, or reset baselines. The old input and observation
remain immutable in collector history. Both selected executable files and their
Cargo receipts/logs are retained outside the worktree with exact hashes.

Final PR review remains pending. Exact run scripts,
source fingerprints, ledgers, receipts and retained review paths are indexed in
`eval/evidence/k1-duplicate-labels.json`. Every historical run and frozen truth
version remains unchanged. The 298 other failures were not rerun, and no result here
meets #97's approximately 500-paper defensible-coverage target. The next coverage
work must address independently demonstrated source/field fidelity or reviewed
manual strata; #109 separately tracks the existing BibTeX lexer defects.
