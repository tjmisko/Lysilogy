# Independent LaTeX object truth

This evaluation-only code reads deposited arXiv sources in memory. It never extracts archive
members to disk, invokes a TeX engine, or evaluates shell escapes. Tests use synthetic source
strings and never publish them as K1.

`archive.py` bounds compressed/expanded bytes, member sizes/counts and text size; rejects links,
traversal, duplicate canonical paths, sparse and special members; and supports standalone gzip
LaTeX. `tex.py` preserves source positions while masking comments and inert definitions, expands
bounded literal includes, and records source membership and unsupported presentation commands.
A unique document root is the default. An explicit root needs independent PDF/title or equivalent
resolved-closure evidence from the builder; filenames alone never break an ambiguity.

`parser.py` derives figure/table captions, numbered equations, custom theorem-like environments,
proof targets, algorithms, bibliography items, citation keys and object references from source
structure. Top-level align/gather/eqnarray rows remain distinct, suppressed rows are excluded, and
nested aligned/split environments stay inside their numbered equation. Unknown citation commands
or unknown object environments withhold exhaustive truth. Used macro aliases that transitively
contain structural/link commands, conditionals, and repeated definitions also withhold admission;
the reader does not execute their visibility or scope semantics. Fields come only from explicit
active BibTeX databases or bibliography-field markup. BibTeX values must agree with the deposited
rendered entry; stale, unrelated, and unverified fields remain unknown with conflict evidence.

`align.py` reads only actual PDF-index text and token geometry. It never reads production figure,
object, paragraph or citation predictions. Unique source-text matches retain half-open UTF-16
spans. Source contexts identify printed citation/reference occurrences; source keys supply their
destinations. Missing or ambiguous matches count against per-paper alignment quality (default
threshold 0.95). Bibliography entry and citation inventories must be exhaustive to enter O8–O10.
Equation matching preserves case, operators, scripts and grouping; unsupported rendered math
withholds that equation. Multiple source objects cannot certify the same PDF span.
Caption token bounds are explicitly not full figure-region labels; full-region and O11 panel
annotations require separate independent evidence.

The offline `k1_index` Rust example maps PDFs through the production identity registry into a
separate existing data root beneath `~/.cache/lysilogy/`. It verifies frozen PDF hashes before
and after local index extraction and keeps the canonical registry across reruns. Corpus PDFs
remain read-only under `~/Corpora/`; all full text and extraction caches remain external.

K1 publication, stratified build receipts, and three-agent panel judgments must establish actual
coverage before this issue is complete. An exploratory parse/alignment pilot is not final K1.
