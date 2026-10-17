# 02 · Bibliography integrity

Status: proposed, 2026-10-16. Priority P0.

## Problem

The local bibliography is the source of every "why is this cited" label, every
local citation edge, and every locally created Work. `bib-v1` is demonstrably
fragile on real papers, and its failures are silent: a mis-split entry becomes a
stub Work with a wrong or missing title, a missed reference list becomes "no
bibliography entries", and a wrongly bounded list can swallow the paper body.

The user's requirement: **correctly extract all the titles, subtitles, years and
listed authors if they are listed in the paper**, and double-check citations with
a very cheap model before trusting them.

## Evidence from the dev database

Six papers currently have local evidence; several show failures.

| Finding | Example |
| --- | --- |
| Entry split mid-title | "Riad Akrour, Marc Schoenauer, and Michele Sebag. Preference-based policy learning. Machine" and "learning and knowledge discovery in databases, pages 12–27, 2011." became two entries |
| Reference list not found, body indexed | One `local_bibliography` record has 53 entries, 2 longer than 1,500 chars; the body's method/experiment prose is in the "bibliography" |
| Whole paper as bibliography | One record has 2 entries, both enormous |
| Venue parsed as authors, no title | Untitled stubs with `author` claims like `["learning", "knowledge discovery in databases", "pages 12–27"]`, `["Conference on Learning Representations"]`, `["Austin"]`, `["Information Processing Systems", "pages 4565–4573"]` |
| Known parser weakness | `sentence_boundary` skips the `.` in "et al." instead of treating it as the author-list boundary ([`fields.rs`](../../crates/lysilogy-ingest/src/bibliography/fields.rs)) |

## Goal

The bibliography is found, split and parsed correctly on normal and hostile
layouts; a cheap model verifies each entry against its printed text; every drop
or ambiguity is an explicit, counted state.

## Scope

1. **Region detection.**
   - Recognize `References`, `Bibliography`, `Works Cited`, `Literature`,
     non-English equivalents and unlabeled reference lists by layout (last pages,
     hanging indents, year density) rather than a single heading string.
   - Never fall back to "the rest of the document". If no region is found, record
     `bibliography_not_found` and route to the extraction repair path
     ([01](01-text-extraction-fidelity.md)).
   - Cut the region at the true start and end; do not include appendices or
     acknowledgements.

2. **Entry splitting.**
   - Join continuation lines correctly: indentation runs, column boundaries,
     wrapped titles, page breaks, hyphenation.
   - Do not split on a line break inside a sentence; use author-year or numbered
     markers as the primary signal.
   - Handle styles: numbered `[1]`, `1.`, author-year, footnote-style, and
     mixed.

3. **Field parsing.**
   - Fix the "et al." boundary; handle initials and multi-word venues.
   - Title vs venue: use position, italics/bold evidence where available, and
     known venue patterns; never accept a pure venue as a title.
   - Keep the raw entry text as the immutable evidence; parsed fields stay
     fallible interpretations.

4. **Cheap model verification pass.**
   - For every entry, a cheap model (Haiku-class or `deepseek-v4.1-flash`) sees
     the printed entry and answers whether the parsed title, authors, year and
     venue are correct, with a verbatim quote when it disagrees.
   - Grounding rule stays the same as the field pass: a corrected value is only
     kept when it is verbatim in the entry; otherwise it is flagged, not used.
   - Cache per entry with a prompt version; reruns cost nothing.
   - Cheap pass only: this is verification, not research. A failure to verify is
     an explicit `unverified` state, never a silent discard.

5. **Citation cross-check.**
   - Every in-text mention's printed key must map to exactly one entry; report
     `ambiguous`, `orphan_entry` (never cited) and `unresolved_mention` counts.
   - Every resolved Work should point back to the entry that named it; an
     entry with no Work, or a Work created from a degraded entry, is flagged.

6. **No silent drops.**
   - `no_bibliography_entries`, `entry_not_stored`, `entry_fields_unreadable`,
     `index_text_changed` and similar warnings must reach the user-visible
     summary, not just logs.

## Non-goals

- Perfect resolution of every entry to a Work; that is identity resolution
  ([03](03-work-identity-and-hydration.md)).
- Parsing non-bibliographic reference formats such as legal citations.

## Acceptance

- On the audit corpus, every entry has a title or an explicit reason it does not,
  and no entry contains body text beyond a bounded length.
- The verification pass resolves the known mis-parses; an `unverified`/`dropped`
  reason is recorded for the rest.
- A paper with no bibliography is reported as such; it is not silently indexed
  as if it had none.
- Reruns are idempotent; a changed index text invalidates the extraction.
- The dev database is backfilled and the 8 bibliography-derived untitled Works
  are either titled or explicitly parked.

## Open questions

1. Which cheap model, and does the verification pass run by default with local
   evidence (as the field pass does) or only on demand?
2. When the model and `bib-v1` disagree, do we insert a new `bib-model-v2` claim
   at a higher confidence inside the citing-bibliography tier, or only flag?
3. Do we store the model's corrected entry as a separate claim on the Work or
   repair the entry row in place?
