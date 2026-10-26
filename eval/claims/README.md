# Held-out claim verifier fixtures

Two sets for scoring the claim verifier (`decide_claim` and `verify_claim`, frozen at HEAD
`f28c030` for this evaluation) on cases the development fixtures in `tests/fixtures/claims/`
never used. Both are built by `examples/build_claim_fixtures.rs --set holdout` and load with
`tests/claims_support` (`load_fixtures`), the same as the base fixtures: `cases.json` is
`{real, fake, near_miss}` and `responses.json` is the recorded provider responses, so
replaying them touches no network.

| Path | What |
| --- | --- |
| `e1/cases.planned.json` | The whole E1 plan (350 cases), frozen, plus `order` (recording order) and `meta`. |
| `e1/cases.json`, `e1/responses.json` | The E1 cases whose recording is complete, and the responses that replay them. **Partial: see "E1 status".** |
| `e3/spec.json` | The hand-written E3 cases (input to the builder). |
| `e3/cases.planned.json` | The E3 spec with ground truth resolved and checked against OpenAlex. |
| `e3/cases.json`, `e3/responses.json` | The recorded E3 set. Complete (70 of 70). |

Each `cases.json` carries a top-level `status` (`recorded`, `planned`, `complete`) and extra
per-case fields that `ClaimCase` ignores. Do not score E1 as a whole until `status.complete`.

## E1: held-out replay of real citations

Sampled from the dev vault (read only), not from the base fixtures' bibliography entries:

1. Every `(cited Work, citing paper)` pair from `graph.citation_assertion` (status asserted
   or accepted) where the citing Work is linked to a `reader.document`, and the cited Work
   has an active OpenAlex ID and a title. Each cited Work is assigned to one random citing
   paper (seed `0x5EED_E1CA`, a fresh seed; the base set uses `0x00C1_A1D5`).
2. Each citing paper keeps at most 8 of its Works (`--max-per-paper`); papers are dealt
   round-robin, so any prefix is spread across papers. The plan as built draws from 49 citing
   papers with at most 5 each (the vault cannot supply more per paper at 200 Works).
3. Each candidate is fetched from OpenAlex (free singleton lookups, cached in
   `~/.cache/lysilogy-scratch/claims-holdout-openalex-works.json`). The claim is built from
   the OpenAlex record, not from the vault's copy.
4. Excluded: every OpenAlex ID (40), DOI (13) and title key (48) that appears in
   `tests/fixtures/claims/cases.json` (real, fake and near-miss); IDs OpenAlex has merged into
   another Work; records with fewer than 3 title words or 15 title characters, no author, no
   year, or `<` (markup) in the title; and repeated titles. The checks are in
   `holdout_eligible`. The base set shares no Work, DOI or title with E1.

Cases (350 planned):

- **Real, 150.** Planner-style claim: title, first at most 3 author surnames, year, venue.
  45 (30%) also carry the Work's DOI (kind `doi`, never an arXiv DOI); the other 105 are kind
  `title`. Ground truth `truth` = the Work's OpenAlex ID. `source_paper` = the citing Work.
- **Fake, 150.** Perturbations of the real cases, the six base kinds, 25 each:
  `fabricated_doi` (real title, made-up `10.1145/` DOI), `real_doi_wrong_title` (a DOI-case
  Work's DOI and authors with another Work's title and year), `authors_swapped`,
  `recombined_title` (title words taken from three real titles), `year_off` (year moved by 4),
  `invented` (an invented title and two invented authors; titles checked against the
  exclusion list). `perturbation_of` names the case a fake was built from. `truth` is null.
- **Near-miss, 50.** Real Works not used in the real set: 16 `typo` (two adjacent letters
  swapped in the longest word), 16 `dropped_subtitle` (title cut at its colon, head of at
  least 5 words), 18 `abbreviated_authors` (full names reduced to initials, `J. T. Ash`).
  They should still verify to `truth`.

Recording order puts the cases that cost no title search first (DOI cases), then each real
`title` case with the three fakes that reuse its search (`fabricated_doi`,
`real_doi_wrong_title`, `authors_swapped`), then the remaining kinds round-robin, so a partial
recording is still a balanced sample of real, near-miss and fake cases.

### E1 status

**Partial: 53 of 350 recorded** (45 DOI cases, 2 title cases, 6 fakes). OpenAlex's free tier
allows 1000 credits a day and charges 10 for each title search (singleton lookups and
Crossref are free); E3 used the day's allowance. The 297 unrecorded cases need roughly 330
more title searches (halves of titles are searched again when a title finds nothing, which
the invented and recombined fakes and the typo cases always do), about three more days of
quota. Everything recorded is kept in
`~/.cache/lysilogy-scratch/claims-holdout-e1-responses.json`, so a rerun replays it for free
and continues where it stopped:

```
LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test \
  cargo run --example build_claim_fixtures -- --set holdout --part e1 --rerecord --max-searches 95
```

The plan is read from `e1/cases.planned.json`, so the run needs no dev database and cannot
drift with it (`--replan` rebuilds it). The run records one case at a time, drops a case whose
requests were refused, replays what it recorded in a fresh scratch database, and writes
`cases.json` and `responses.json` only if the replay misses nothing. `--rerecord` is needed
because the files exist; it replaces them with the longer recording. Deleting the cache file
makes the next run fetch everything again.

## E3: hard cases

70 hand-specified cases in `e3/spec.json` (57 accept, 13 must-not-accept). Each accept case
names an OpenAlex Work as `truth_id` with the title it should have (`truth_title`). The
builder fetches that Work (free lookup) and fails unless its title matches `truth_title`
(similarity at least 0.9); if the claim carries a DOI it must resolve to the truth or an
alternate. I looked up every truth ID on OpenAlex (title, authors, year) before writing it
down. Accept cases go in `real` with `truth` set, must-not-accept cases in `fake` with
`truth` null, so the base report code runs unchanged. Extra fields on every case:

- `expected`: `accept` (resolve to `truth` or any `alt_truth`) or `must_not_accept` (the
  outcome must not be a Work, whatever it resolves to).
- `alt_truth`: other OpenAlex records of the same paper that count as correct. Score a
  resolution to `truth` or any `alt_truth` as correct; `claims_support::Report` counts only
  `truth`. Five of these were added after the first recording, when the verifier resolved to a
  second record of the same paper (checked by title, authors and year): preprint-03, book-01,
  etal-01, old-00, old-05, each noted in the case's `note`. No other ground truth was changed
  after seeing a result.
- `truth_title`, `truth_year`: what OpenAlex holds. `note`: why the case is hard.

Kinds (count): `same_title` (8 accept; generic titles such as "Deep learning", "Attention",
"Dropout", each tied to a different paper by authors and year), `same_title_wrong_paper` (2) and
`same_title_wrong_year` (1) must-not-accept, `preprint_vs_published` (9; year or title
changed, one with an arXiv ID that OpenAlex resolves to a mis-titled record, one old-style
arXiv ID), `book` (5), `chapter` (4), `thesis` (3), `initials_authors` (3), `et_al_authors`
(2), `non_ascii` (6; diacritics in author names and titles, and the same with them dropped),
`math_unicode_title` (3; a Greek letter, a superscript, LaTeX `$N$`), `wrong_venue` (3),
`pre_1970` (7, back to 1865), `retracted` (4; OpenAlex prefixes some titles with
`RETRACTED:`; a retracted paper is still a real paper and is accepted to its Work), and
`invented_plausible` (10 must-not-accept: invented titles in a real author's field and style,
with that author's surname).

OpenAlex data quality is part of what E3 measures: dates off by years, duplicate records,
a preprint DOI resolving to an unrelated paper, empty author lists. Those are real, and the
ground truth is the Work a person would mean, not whatever OpenAlex returns for the DOI.

Baseline of the frozen verifier on E3, informational only (the decision rules were not
changed): 36 of 57 accept cases resolve to `truth`; 5 more resolve to a listed `alt_truth`;
16 do not resolve to the paper (7 unverified, 4 ambiguous, 3 contradicted, and the 2 that
resolve to a wrong Work: a different `Deep learning` record with no authors, and the Nature
paper for the Goodfellow textbook); none of the 13 must-not-accept cases resolves to a Work
(0 fakes verified or matched).

## Rebuilding

```
scripts/build-claim-fixtures   # the base set, unchanged
cargo run --example build_claim_fixtures -- --set holdout --part e3|e1|all [--rerecord]
    [--max-searches N] [--seed N] [--real N] [--fake N] [--near N] [--max-per-paper N]
    [--doi-percent N] [--exclude cases.json] [--spec spec.json] [--replan] [--out DIR]
```

`--out` defaults to `eval/claims` for `--set holdout` and `--max-searches` to 90. Fixtures are
never overwritten without `--rerecord`. Environment: `LYSILOGY_DATABASE_URL` (dev database,
read only, used only to build the E1 plan), `LYSILOGY_TEST_DATABASE_URL` (a server to create a
scratch database on), `LYSILOGY_CONTACT_EMAIL` and `LYSILOGY_OPENALEX_API_KEY` if set. With
an API key the daily budget is larger and E1 finishes in one run.
