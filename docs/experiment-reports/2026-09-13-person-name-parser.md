# E2.3 person name parsing

`kb::names` now parses one printed author name into explicit, unranked alternatives and derives
coarse family-name/first-initial candidate keys. It preserves the exact raw input. This is a pure
parser foundation: it does not create Persons, merge identities, or claim resolver precision.

## Decisions

Commas delimit family-first forms. In unmarked full names, both orders and compound-name
boundaries survive. Dotted initials constrain possible splits; bare one-letter tokens also retain literal family
interpretations (`O Kye` has both orders). Compact capital groups retain
both word and initial interpretations, including mixed interpretations of multiple groups.
Hyphenated given names preserve their structure while exposing each component initial.
Unmarked Roman suffixes retain the possible initial/name interpretation as well. No surname
list, ethnicity, or language guess chooses a preferred alternative.

A particle is a possible component, never an exclusion rule: Le and Van can be complete family
names, and Al can be a given name. A multiword particle prefix also retains the literal compound
family interpretation. Single undivided names have no inferred given initial and emit no
wildcard key. Unsupported syntax and explicit limits (1024 bytes, 16 words, 256 alternatives)
return an unresolved reason; ambiguity overflow never returns a truncated candidate set.

Display components use NFC and normalize whitespace and typographic apostrophes/hyphens. The
raw string remains byte-for-byte unchanged. Candidate keys separately fold compatibility forms,
case, accents, punctuation, and common ligatures; umlaut ae/oe/ue alternatives make Müller and
Mueller overlap. Keys retain their own family/initial pairing per interpretation. Shared coarse
keys do not prove identity: tests explicitly preserve distinct John/Jane and Jr./Sr. evidence.

## Review corrections and validation

Root review identified that treating particle words as exclusions loses real names such as
`Le, Xuan`, `Van, Alice`, and `Al Smith`. Those interpretations are now retained and covered by a
dedicated regression table. Independent review also reproduced uppercase letters appearing after
compatibility decomposition, making styled names' keys differ from ordinary spellings. Keys now
normalize compatibility forms before umlaut construction and fold case after decomposition, with
regressions for mathematical bold/double-struck and fullwidth family and given names.

Final implementation `464baf9cafcb272b12f56f6dea6319f47ebd5550` passes formatting, strict all-target/all-feature Clippy,
and all 295 Rust tests (16 new name-parser tests with broad fixture tables). G5 passed
295 Rust, 80 Python, and 85 Node tests in an isolated network namespace with no model CLIs.
Persons, resolution, scale, and tests evaluations all pass `--check`. Before/after suite results,
implementation hashes, exact G5 evidence, and log hashes are retained in
[person-name-parser.json](../../eval/evidence/person-name-parser.json). Evaluation dirty flags
include the generated scorecard and this report; the committed implementation is identified
separately. Model calls: 0; cost: $0. G5 wall time: 7.495 seconds.

## Metrics and limits

The persons and resolution suites were run before and after this change with `--check`.
K3/K4 truth and resolution collectors remain unavailable; parser fixtures do not establish G1,
G4, O13, or O14. No hard gate, target, or ratchet baseline was relaxed. G5 and the provider-budget
O30 simulation remain the available checks; O30 stays at zero violations per 10,000 admissions.

The earlier synthetic O25/O26 measurements remain historical baselines with open follow-ups
#21/#22. They were not copied into a fresh collector or reattested after source changes; this
pure parser change does not justify rerunning the expensive full-vault timings or claiming a
scale improvement. Live corpus and final system acceptance remain outstanding.
