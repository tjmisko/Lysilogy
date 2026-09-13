# E2.4 title keys and fuzzy scores

`kb::titles::title_key` now normalizes common title presentation variants, and
`title_similarity` supplies a deterministic trigram candidate score for later resolution.
`$\alpha$-Divergence` matches `α-divergence`, subtitle colons match spaced hyphens, and
`Attention Is All You Need` remains distinct from `Attention Is Not All You Need`.
The SQLite FTS5 table and population belong to #33, which will consume this shared key API
when the branches integrate. This change introduces no identity merges.

## Decisions and review corrections

The key folds compatibility Unicode, letter accents, case and ordinary punctuation while
preserving word boundaries, repeated words, negation, mathematical operators and script binding.
Known TeX presentation commands are decoded without executing TeX. Unknown commands retain their
names, case and argument boundaries. The parser is iterative, including deeply nested input.

Review found that dropping all braces could conflate fraction arguments and exponent grouping;
retained arguments could also lose a minus sign, Unicode script binding, or an escaped delimiter.
Those cases now have distinguishing regressions. Unicode superscript/subscript digits and letters
normalize to explicit script groups, including inside unknown arguments. Mathematical negation
combining overlays remain significant, preserving `≠` versus `=` and `∉` versus `∈`.

The script classes use 308 single-character compatibility super/sub records from Unicode 16 plus
Unicode 17's [U+A7F1 modifier capital S](https://unicode.org/charts/nameslist/n_A720.html#A7F1).
An exhaustive comparison of all 1,112,064 Unicode scalar values against the actual
`unicode-normalization` 0.1.25 crate found only that added NFKD mapping. The evidence retains the
audit program, versions and difference. A test requires an explicit table review when the crate's
Unicode version changes.

Similarity follows the design's character-trigram approach: multiset Sørensen–Dice with two
boundary sentinels at each end. Repeated grams count; empty keys score zero. Different keys can
share the same gram multiset and score one, so exact equality uses the key itself. A score is
neither a match probability nor an automatic merge threshold. Unequal keys over 1,024 Unicode
characters receive zero fuzzy similarity; normalization and exact comparison still process the
full input in linear space. This bounds fuzzy comparisons without truncating exact keys.

## Validation and metrics

Implementation `7c20a7066b3d3206a0c954c624f89fd237303dbe` includes current main (`a144434`) and passes formatting,
strict all-target/all-feature Clippy, and all 305 Rust tests. Ten new title tests include the broad
fixture tables, exact trigram arithmetic, empty/oversized inputs, deep nesting and Unicode-version
checks. G5 passed 305 Rust, 80 Python and 85 Node tests in an isolated network namespace with no
model CLIs. G5 wall time: 6.743 seconds; model calls: 0; cost: $0.

Resolution, persons, scale and tests evaluations pass `--check`. Their original JSON results,
source hashes, G5 receipt and log summaries/hashes are retained in
[title-normalizer.json](../../eval/evidence/title-normalizer.json). Before/after results retain
their actual commit and dirty flags; any dirty state during these checks reflects generated
scorecard output. No source changed during the checks.

The resolution suite passes `--check` both before and after, with K3 and the owning collectors
still unavailable. These 52 equivalent pairs and 35 distinguishing pairs are regression fixtures;
they do not establish G1, G4, O12 or O14. The shared FTS5 population and real resolution metrics
remain dependent on #33 and the later truth/resolution issues. No target or baseline was relaxed.
G5 remains passing and O30 remains zero violations per 10,000 simulated admissions.

The earlier O25/O26 synthetic measurements remain historical baselines with existing follow-ups
#21/#22. No full-vault timing was repeated for this pure parser change. Final system acceptance
and the live 10k arXiv scenario remain outstanding.
