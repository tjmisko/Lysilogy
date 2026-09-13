# Bibliography collector input contract (version 1)

`scripts/eval/bibliography.py` runs the production Rust object builder over existing external
reading-index caches. It accepts `--k1 eval/truth/k1-bibliography.json`, optional
`--k2 eval/truth/k2-references.json`, and an explicit `--index-root` under the corpus/cache storage.
PDFs, LaTeX sources, full reading-index text, and raw object artifacts stay outside the repository.
Only independent derived labels, matching decisions, normalized field outcomes, and hashes are
retained. No network or model call occurs in the collector.

K1 must be built independently from LaTeX bibliography/citation structure and PDF alignment;
production bibliography segmentation or parsed metadata cannot supply its labels. The builder
records alignment method/quality and excludes papers below its declared quality threshold.
This collector consumes a bibliography projection of K1 with the following shape (a schema
illustration, not truth or a measurement):

```json
{
  "schema_version": 1,
  "truth_set": "K1",
  "version": "k1-bibliography-v1",
  "origin": "arxiv-latex",
  "alignment_threshold": 0.95,
  "papers": [{
    "arxiv_id": "<actual arXiv ID>",
    "paper_id": "<actual 16-character local paper ID>",
    "pdf_sha256": "<actual PDF hash>",
    "source_sha256": "<actual LaTeX archive hash>",
    "index": {"path": "<path relative to --index-root>", "sha256": "<actual cache hash>"},
    "alignment": {"quality": 1.0, "method": "<independent alignment method>"},
    "entries": [{
      "id": "<independent bibitem/truth ID>",
      "spans": [{"start": 123, "end": 456}],
      "field_labels": {"title": "<known title>", "first_author": null, "year": "2020"}
    }],
    "mentions": [{"start": 42, "end": 45, "target": "<independent bibitem/truth ID>"}]
  }]
}
```

All spans are half-open UTF-16 offsets in that exact index. They are never Rust byte offsets or
layout token indices. Entry members may be disjoint. Entry IDs are unique per paper and are not
production `bib-*` IDs. Each mention labels one exact printed occurrence and one target; grouped
citations have one pair per destination. Repeated occurrences in one sentence remain separate.
An exhaustive entry/mention inventory is required for each included paper so false positives can
be counted. Unknown field labels are null or omitted; an empty string is invalid, not a known
field. Known `year` labels contain a four-digit year with an optional printed suffix.

K2 uses the E8.4 reference truth schema: `schema_version`, `truth_set: "K2"`, `version`, `sources`,
`works`, `cases`, and `coverage`. Only cases with `eligible_bibliography: true` and
`input.kind: "deposited_unstructured"` enter the bibliography field population. Their input is
the actually deposited `input.text`; labels come independently from the deposited structured
`field_labels.{title,first_author,year}`. Structured references rendered into an input string are
excluded because that would leak the answers into the test. Case IDs, snapshot IDs, and JSON
pointers are retained with each result. Known and excluded field counts are reported by truth set.

Segmentation O8 matches entries one-to-one using identical source membership after ignoring only
whitespace. Every unmatched predicted entry is a false positive; every unmatched truth entry is a
false negative. Matching decisions and normalized span signatures are retained for inspection.
O9 evaluates every independently known field, including fields on unmatched truth entries:
missing/wrong predictions count wrong. Unknown labels are excluded with explicit counts. Title
comparison uses the shared Rust `title_key`; author comparison folds presentation case, Unicode
compatibility, whitespace and punctuation, without fuzzy identity or initials expansion. Years
compare four-digit publication years, preserving suffixes in the underlying parsed evidence.
O9 requires known labels from both genuine K1 and K2 populations for each component; missing truth
or zero known labels in either population keeps that component
unavailable. No synthetic fixture can establish these objectives.

O10 compares exact occurrence/target pairs after the segmentation mapping. A wrong destination
contributes a false positive and a missed true pair; unresolved predictions contribute only the
missed true pair. Duplicate predicted pairs count as extra false positives. All pairs from the
included papers enter the denominator; missing segmentation never silently drops a citation.
Precision with no predicted pairs is unavailable; recall with no truth pairs is unavailable.
Geometry fragments for the same source occurrence on different pages count as one pair;
duplicate predictions on the same page remain extra false positives.

The collector verifies the external index SHA-256 before and after invoking the builder and
records the opaque generation derived from those persisted bytes. Relative paths cannot escape
the supplied storage root or resolve through protected `.env`/`.secrets` names. Its eval input
fingerprints the production parser, object/type plumbing, shared normalization helpers, bridge,
Cargo manifest/lock, collector, truth labels and derived observations. Source-PDF/archive hashes
come from K1's independently verified provenance. The report retains all matching disagreements,
known/excluded field counts, model calls (zero), cost (zero), and measured wall time.
