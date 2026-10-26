# Claim verifier evaluation, October 2026

Status: 2026-10-26. Verifier: `decide_claim` / `verify_claim` frozen at `0913e7f`. The only
change since then is the `Ablation` switch, and its default reproduces the shipped rules (the
equality test passes). No rule behaviour was changed for this evaluation.

Sources: `eval/claims/results.json` (all numbers below unless noted), `eval/claims/README.md`
(fixture construction), and `eval/claims/e2/` (goals, claims, labels, spot check). Method:
an offline replay of recorded OpenAlex/Crossref responses, with an empty graph and a cold
response cache before every claim, so the local-match shortcut never fires and calls per
claim are cold-cache counts. Intervals are 95% Wilson. "Upper" is the rule-of-three 95% upper
bound on a rate observed as zero. I re-ran the default arm with a scratch copy of the harness
to get per-claim rows for this report, and its aggregate counts match `results.json`.

## Summary

- **No pass-bar line can be evaluated yet.** E2 has labels but no verifier verdicts, because
  the OpenAlex free daily budget ran out before its responses were recorded. E1 is 53 of
  350 cases. No pass is claimed.
- **No false accepts anywhere in the default configuration**: 0/12 on E3 (upper 0.25) and
  0/6 on partial E1 (upper 0.50). These bounds are too wide to support a safety claim.
- **E3 (hard cases, complete): real recall 38/56 = 67.9% (54.8 to 78.6), and 2 wrong-Work
  resolutions.** Both wrong Works are generic-title cases ("Deep learning"). Five of the 38
  correct answers count only because an `alt_truth` was added after the first recording.
  Without those, recall is 33/56 (58.9%).
- **E2 (live model claims, labels only):** neither model produced a paper that does not
  exist (0/272). DeepSeek gave flawed details on 12/120 agreed claims (10.0%, 5.8 to 16.7),
  Fable on 2/150 (1.3%, 0.4 to 4.7). Because no claim is labelled `does_not_exist`, the
  E2 false-accept bar is vacuous on this sample.
- **The rules that carry the most weight on E3 are duplicate collapsing (7 correct answers)
  and the split-title retry (4).** The surname check is the only rule that stops the E1
  `authors_swapped` fakes. Removing it accepts 2 of 2. The identifier year and author check
  is **untested, not shown to be noise**: the two claims it would change could not be
  replayed in the ablated arm.
- **2 claims need your spot check**: `eval/claims/e2/spot_check.json`.

## Headline

| Set | Status | Scored | False accepts | Wrong Work | Real recall | Ambiguous | Pending | Calls / claim | Title searches / claim |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| E1 | **partial**, 53/350, informational only | 53 | 0/6 (upper 0.50) | 0/47 (upper 0.064) | 46/47, 97.9% (88.9 to 99.6) | 2/53 (0/47 real) | 0/53 | 2.00 | 0.17 |
| E2 | **blocked**: labelled, not verified | 0 of 272 | not evaluable (0 `does_not_exist` labels) | n/a | n/a | n/a | n/a | n/a | n/a |
| E3 | complete, final | 68 of 70 | 0/12 (upper 0.25) | 2/56, 3.6% (1.0 to 12.1) | 38/56, 67.9% (54.8 to 78.6) | 5/68 (5/56 real) | 0/68 | 2.03 | 1.28 |

E3 excludes 2 of its 70 claims as unreplayable: `e3-initials-02` (Adam, "D. P. Kingma") and
`e3-nonascii-05` (3D U-Net, diacritics dropped). The recording was made with an accumulating
graph, and the local-match shortcut answered these two, so the empty-graph replay asks for
title searches that were never recorded. The empty-graph score (38/56) is lower than the
README baseline (41/58), and that is expected.

E1 is not a balanced sample. 45 of its 47 real cases carry a DOI, so the 97.9% mostly
measures DOI resolution. It has 2 title cases, 6 fakes and no near-miss cases. The one real
miss is a fixture defect, not a verifier defect: in `e1-real-doi-006` OpenAlex holds the
authors as single letters (`"S", "A", "G"`), so no surname can match. Its outcome is
`contradicted`.

Nondeterminism: two identical default runs gave identical per-claim outcomes on E1 and E3.
The `e3-venue-01` coin flip the README reports did not recur once the graph was reset per
claim. That claim is now consistently `ambiguous`.

### Pass bar

| Requirement | Status | Observed so far |
| --- | --- | --- |
| 0 false accepts on E2 | not evaluable | E2 not scored. When scored, the bar is vacuous: 0 claims labelled `does_not_exist` |
| 0 false accepts on E1 | not evaluable | 0/6 (partial) |
| E2 real recall >= 85% | not evaluable | E2 not scored |
| E1 real recall >= 90% | not evaluable | 46/47 (partial, DOI-heavy) |

To finish the evaluation: record E2 (`cargo run --example e2_verify -- --dir eval/claims/e2`)
and the rest of E1 (the README command, `--part e1 --max-searches 95`, about three more days
of the free budget, or one run with `LYSILOGY_OPENALEX_API_KEY`). Then rerun
`LYSILOGY_TEST_DATABASE_URL=postgresql://$USER@127.0.0.1:5433/lysilogy_test cargo run --example claims_holdout_eval`.
The E2 attempt so far made 12 live requests (1 title search) before the budget ran out. Its
11 responses are kept in `~/.cache/lysilogy-scratch/e2-responses.json`.

## False accepts and wrong-Work cases

**False accepts (default rules): none** in E1 (0/6) or E3 (0/12). Under ablation there are 2,
both on E1 with the surname check removed:

- `e1-fake-authors-swapped-000`: "Open hearts build lives: Positive emotions, induced
  through loving-kindness meditation, build consequential personal resources." with author
  `Gorn`, 2008. Default: `unverified`. Ablated: verified to W2125750403 (the real
  Fredrickson et al. paper).
- `e1-fake-authors-swapped-001`: "The Moral Machine experiment" with author `Clark`, 2018.
  Default: `unverified`. Ablated: verified to W2896252141 (Awad et al.).

**Wrong Work (default rules): 2, both on E3.**

- `e3-same-title-01`: "Deep learning", Salakhutdinov, 2014, KDD (a tutorial abstract;
  truth W2911666613). Verified to **W4244888682**, "DEEP LEARNING" (2014, CRC chapter,
  `10.1201/b17103-3`), which has **no authors** in OpenAlex. A record with no authors passes
  the surname check (`authors_agree` returns true when either side is empty). The truth was
  not in any recorded search result.
- `e3-same-title-03`: "Deep Learning", Goodfellow, Bengio, Courville, 2016, MIT Press (the
  textbook; truth W2557283755). Verified to **W2919115771**, the 2015 Nature review by
  LeCun, Bengio and Hinton. Duplicate collapsing (`same_paper`) treats the two as one
  paper: the title keys are equal, the years are within 1, and they share the surname
  Bengio. It then keeps the more-referenced record. Removing duplicate collapsing, the year
  window or the surname check each turns this into a non-accept.

No ablation introduced a new wrong Work. Each ablation's wrong-Work delta is 0 or -1 or -2.

## E2: DeepSeek vs Fable

Both models answered the reading-list planner's real plan-pass prompt for the same 8
vault-derived goals (size 15, level `survey`, empty profile and graph context). Two
independent labellers (A and B) labelled every claim. Agreed labels are ground truth, and
disagreements go to the spot check.

| | DeepSeek (`deepseek-v4.1-flash`) | Fable (`claude-fable-5-1`) |
| --- | --- | --- |
| Claims | 121 | 151 |
| Agreed labels | 120 | 150 |
| `exists` | 108 | 148 |
| `exists_wrong_details` | 12, 10.0% (5.8 to 16.7) | 2, 1.3% (0.4 to 4.7) |
| `does_not_exist` | 0 (upper 0.025) | 0 (upper 0.020) |
| Spot check | 1 | 1 |
| With a DOI or arXiv ID | 95/121 | 146/151 |
| Wrong DOI (agreed) | 2 of 20 DOIs | 1 of 57 DOIs |
| Wrong arXiv ID (agreed) | 3 of 76 IDs (+1 disputed) | 0 of 90 IDs (+1 disputed) |

Neither model invented a paper. Every error is a real paper with some details wrong. The
breakdown by kind (agreed `exists_wrong_details`):

DeepSeek (12):

- **Invented or garbled co-authors on a real paper (6).** `ds-01-01` "Deep reinforcement
  learning from human preferences" names "Milad Baroni" (real: Miljan Martic). `ds-01-02`
  and `ds-02-03` are the same Leike et al. paper with "Milad Martic", and the second copy
  also has "Vladislav Tretyak". In `ds-01-14` (AI Alignment survey) "Hailong Yang" replaces
  Hantao Lou. In `ds-02-01` (Christiano et al. 2018) "Eric Neyman" replaces Dario Amodei.
  In `ds-08-15` (Quiet-STaR) three co-authors are invented. In each case the first author
  is right.
- **Conflations (2).** `ds-02-14` puts the title of Phuong et al. 2024 ("Evaluating frontier
  models for dangerous capabilities") on the author list and year of Shevlane et al. 2023,
  with an unrelated arXiv ID (2303.05656, an EHR diffusion paper). `ds-07-05` puts the title
  of Ellerman, Marcantonini and Zaklan (REEP 2016) on Laing, Sato, Grubb and Comberti, with
  a DOI that resolves to an unrelated paper.
- **Wrong identifier on a real paper (2).** `ds-04-14` DexMV cites arXiv 2203.09754 (a
  physics paper; real 2108.05877). `ds-04-10` uses the dataset nickname "EgoHOS" as the
  title and cites an unrelated arXiv ID.
- **Wrong DOI (1).** `ds-05-06` (Böhringer and Jochem 2007) gives a DOI that resolves to
  "Managing without growth".
- **Wrong venue (1).** `ds-05-07` (Stanton, HDI history) names a GDAE working paper. It is a
  PERI working paper.

Fable (2):

- `fa-07-11` (Haberl et al. 2020, ERL): last author "Dominik Creutzig" (real: Felix). The
  surname is right.
- `fa-07-15` (Parry and Small 2005, AER): the DOI does not resolve (404 at Crossref and
  OpenAlex).

DeepSeek's characteristic error is a fabricated co-author or identifier attached to a real
paper, and twice a full conflation of two papers. Fable's two errors are single-field slips.
Fable writes much longer author lists (mean 10.3 authors per claim vs 5.2), which gives the
surname check more to match.

**Verifier verdicts on E2: none yet.** The following is what the frozen rules imply. These
are predictions, not results:

- **11 agreed-`exists` claims cite arXiv papers whose OpenAlex arXiv-DOI record carries a
  corrupted title**, per both labellers: `ds-01-06`, `ds-01-10`, `ds-08-07`, `ds-08-13`,
  `fa-01-04`, `fa-01-05`, `fa-01-14`, `fa-02-09`, `fa-03-04`, `fa-08-08`, `fa-08-14`
  (InstructGPT, Constitutional AI, chain-of-thought, PPO, grokking progress measures, and
  others). This is the E3 `e3-preprint-01` (BERT) pattern. The identifier's title disagrees,
  the identifier is dropped, and a title match after a dropped identifier never
  auto-verifies (`decide_claim` returns `contradicted` with candidates). That alone could
  cost up to 11/256 = 4.3 points of E2 recall.
- The wrong-arXiv and wrong-DOI claims should come out `contradicted` with the real paper as
  a candidate, not accepted.
- Wrong co-authors should not block acceptance, because one matching surname is enough.

Label-quality caveats for scoring E2. OpenAlex was rate-limited during labelling:

- 4 agreed claims have no OpenAlex ID at all (`ds-07-15`, `fa-05-01`, `fa-08-16`,
  `fa-08-17`). An acceptance of any of them will be unadjudicated.
- For 32 agreed DeepSeek claims (`ds-02-09`, `ds-02-14`, `ds-03-01` to `ds-03-16`, and
  `ds-04-01` to `ds-04-14`) only labeller B gave an ID.
- 2 agreed claims carry two different IDs, and either counts as correct. `ds-05-02` has the
  2009 Stiglitz-Sen-Fitoussi report and its 2010 book edition. `ds-06-01` has Bound and
  Johnson AER 1992 and the 1989 NBER working paper.

## E3: failures by kind

| Kind | Accept cases | Correct | Failures |
| --- | --- | --- | --- |
| same_title | 9 | 2 | 1 ambiguous, 4 unverified, 2 wrong Work |
| preprint_vs_published | 9 | 6 | 2 contradicted, 1 unverified |
| pre_1970 | 7 | 7 | |
| non_ascii | 5 (+1 excluded) | 5 | |
| book | 5 | 2 | 1 ambiguous, 1 contradicted, 1 unverified |
| chapter | 4 | 3 | 1 unverified |
| retracted | 4 | 2 | 2 ambiguous |
| thesis | 3 | 3 | |
| math_unicode_title | 3 | 3 | |
| wrong_venue | 3 | 2 | 1 ambiguous |
| initials_authors | 2 (+1 excluded) | 2 | |
| et_al_authors | 2 | 1 | 1 unverified |
| **must-not-accept** (invented_plausible 10, same_title_wrong_paper 1, same_title_wrong_year 1) | 12 | 12 not accepted (all `unverified`) | |

The 18 accept-case failures, by mechanism:

**Authorless OpenAlex records pass the surname check (4 claims).**

- `e3-same-title-00` ("Deep learning", LeCun/Bengio/Hinton, 2015, Nature) and `e3-venue-01`
  (the same claim with venue Science) are both `ambiguous` between the truth W2919115771 and
  the authorless "DEEP LEARNING" W4244888682 (2014, within the ±1 year window).
- `e3-same-title-01` is the wrong Work described above.
- `e3-book-04` (Bishop, Pattern Recognition and Machine Learning, 2006) is `ambiguous` among
  5 records, including both truth records. Three of the five have no authors (one of them
  the alt truth W4212863985).

**Duplicate collapsing merges different works with a generic title (1).** `e3-same-title-03`,
wrong Work, described above.

**Generic title, truth not in the top 10 search results (6).** These are all `unverified`,
and the truth record appears in no recorded response:

- `e3-same-title-02` "Deep Learning" (Akleman 2020)
- `e3-same-title-05` "Attention" (Luck and Vecera 2002)
- `e3-same-title-06` "Attention" (Moray 2017)
- `e3-chapter-03` "Attention" (Fuster 2005)
- `e3-same-title-09` "Dropout" (Srivastava et al. 2014, cited by the head of its title)
- `e3-book-03` "The Elements of Statistical Learning: Data Mining, Inference, and
  Prediction". OpenAlex holds it without the subtitle, and the search for the full title did
  not return it.

**A dropped identifier blocks a correct title match (2).**

- `e3-preprint-01` (BERT, arXiv 1810.04805) is `contradicted`. OpenAlex resolves the arXiv
  DOI to a mis-titled record. The title search then finds the truth, which is listed as the
  only candidate, but a title match after a dropped identifier is never verified.
- `e3-preprint-08` (Maldacena, hep-th/9711200, 1997) is `contradicted`. The resolved record
  is dated 1999, two years off. The candidate is the alt truth W4249956767 (1998).

**The identifier's year is off by 2 (1).** `e3-book-02` (Nielsen and Chuang, 2010, DOI of
the 10th anniversary edition) is `contradicted`. The DOI resolves to the 2012 record. The
candidates are two other editions (W32507011 from 2011, W3103020408 from 2010), not the
truth.

**Provider year far off (1).** `e3-preprint-05` ("Attention Is All You Need", 2017) is
`unverified`. OpenAlex dates the record 2025, and the search's year filter (2016 to 2018)
excludes it.

**Retraction records (2).**

- `e3-retracted-00` (Wakefield 1998) is `ambiguous` between the truth "RETRACTED: ..." and
  a second record titled "... (Retracted article. See vol 375, pg 445, 2010)". The title
  keys differ, so they are not collapsed.
- `e3-retracted-02` (Obokata 2014) is `ambiguous` between the truth and the "Retraction
  Note: ..." notice W2047629610, which has the same authors.

**"et al." in a single author string (1).** `e3-etal-00` ("K. He et al.") is `unverified`.
No usable surname is extracted. With the surname check ablated it verifies correctly.

Recoverability: all 5 `ambiguous` failures, and 2 of the 3 `contradicted` ones
(`e3-preprint-01`, `e3-preprint-08`), list the truth or an alt truth among their
candidates. Resolving in place can fix them with one keystroke. Counting those, 45/56 accept
cases reach the right paper.

## E4: ablations

Each arm switches off one rule. Deltas are against the default, computed only on claims
replayable in both arms. Claims whose ablated run needs an unrecorded request are excluded,
and the counts are in `results.json`.

E3:

| Ablation (rule switched off) | Compared | Correct | Ambiguous | Wrong Work | False accepts | Title searches | Provider calls | Claims changed |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `duplicate_collapsing` | 68 | **-7** | +8 | -1 | 0 | 0 | -8 | 8 |
| `split_title_retry` | 68 | **-4** | 0 | 0 | 0 | -27 | -31 | 4 |
| `year_window` (exact year) | 67 | **-3** (5 lost, 2 gained) | -2 | -1 | 0 | 0 | -4 | 8 |
| `surname_check` | 63 | -1 (2 lost, 1 gained) | +4 | -2 | 0 | 0 | -3 | 5 |
| `identifier_checks` | 66 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

E1 (partial; 2 title cases, so the title-path rules are barely exercised):

| Ablation | Compared | Change |
| --- | --- | --- |
| `surname_check` | 52 | **+2 false accepts** (both `authors_swapped`, listed above), +2 calls |
| `duplicate_collapsing`, `split_title_retry`, `year_window` | 53 | none |
| `identifier_checks` | 52 | none |

The rules that carry weight:

- **Duplicate collapsing.** Without it, 7 correct answers become `ambiguous` between two
  OpenAlex records of the same paper: `e3-preprint-02`, `e3-chapter-00`, `e3-nonascii-00`,
  `e3-nonascii-01`, `e3-venue-00`, `e3-old-00` and `e3-retracted-01`. This is the largest
  single effect, but the same rule produces the `e3-same-title-03` wrong Work.
- **Split-title retry.** It rescues `e3-nonascii-02`, `e3-nonascii-03`, `e3-math-00` and
  `e3-old-06`. It costs 27 extra title searches on E3. Only 7 of those are spent on the 4
  rescued claims, and 20 go to the 10 invented fakes, which find nothing anyway. A title
  search costs 10 OpenAlex credits.
- **Year window ±1.** Requiring an exact year loses `e3-same-title-04`, `e3-preprint-00`,
  `e3-preprint-02`, `e3-preprint-06` and `e3-old-01`. It gains `e3-same-title-00` and
  `e3-venue-01`, which only fail because of the authorless 2014 record. The window carries
  weight, and its downside comes entirely from the authorless-record problem.
- **Surname check.** It is the only rule that rejects the `authors_swapped` fakes (2/2 on
  E1). On E3, ablating it costs `e3-book-01` and `e3-old-04` (both become `ambiguous`), gains
  `e3-etal-00`, and turns both wrong Works into `ambiguous`. On E3 it is roughly neutral.
  On fakes it is essential.
- **Identifier year and author checks.** No observed effect, but this is **untested rather
  than noise**. The two E3 claims the check decides, `e3-preprint-08` and `e3-book-02`, both
  needed unrecorded responses in the ablated arm and were excluded. E1 has no DOI fakes
  whose year or authors alone disagree.

On the tuning fixtures (`tests/fixtures/claims/`), `surname_check` and `year_window` change
verdicts. No ablation looks like pure tuning noise. That said, E3 deltas of 1 to 4 claims out
of 56 are single-digit counts with identifiable mechanisms, not stable rate estimates.

## Live generation findings (E2 collection)

**DeepSeek** (`deepseek/deepseek-v4.1-flash` via OpenRouter, served by Morph): 8 calls, one per
goal, sent through the planner's own prompt, client and a scratch spend ledger.

- **OpenRouter spend: $0.0460 total** ($0.0044 to $0.0075 per goal).
- All 8 answers parsed with the planner's `parse_draft`, with `finish_reason: stop`.
- Prompt about 1,520 tokens. Completions 5,425 to 9,376 tokens, of which 71% is reasoning
  (40,323 of 56,685).
- The largest completion is 78% of the 12,000-token cap the collector set. Larger `size`
  values risk truncation.
- Latency 30 to 58 s per goal.

**Fable** (`claude-fable-5-1` via the Claude Code CLI on the plan, `eval/claims/e2/fable-run.sh`):
8 calls, one per goal, using `--json-schema` structured output.

- All 8 returned a structured answer in 2 turns.
- The planner's schema had to have its `$schema` key removed. `schema.fable.json` is
  otherwise identical.
- The CLI reports a nominal **$4.92** ($0.45 to $0.76 per goal). This is billed to the plan,
  not cash.
- Each call carried about 20.3k input tokens, mostly the CLI's own context. The planner
  prompt is about 1.5k.
- Output 6.4k to 12.4k tokens. Latency 75 to 157 s per goal.

**Prompt adherence:**

- Both models were asked for about 15 papers. DeepSeek gave 15 or 16 per goal (121 total).
  Fable gave 16 to 22 (151, mean 18.9).
- DeepSeek used "Surname, Given" author form in 15 claims and left `venue` null in 34.
  Fable did neither.
- The prompt says to give a DOI or arXiv ID "only when you are sure". DeepSeek still gave 5
  wrong identifiers out of 96 given (2 DOIs, 3 arXiv), and Fable 1 out of 147.
- DeepSeek repeated a paper across two goals (`ds-01-02`, `ds-02-03`) with different wrong
  co-authors. `extract.py` drops only exact duplicates, and found none.

## Recommended rule changes

**All of these are untested proposals.** Several were found by reading E3 failures, so E3
cannot validate them. Each must be checked on a fresh set (the rest of E1, E2, and a new
hard-case set written without looking at these results). For each one, report the change in
false accepts first.

1. **Do not let an authorless record satisfy the surname check when the claim names
   authors.** One option: drop authorless plausible records whenever another plausible record
   matches a surname. Expected on E3: `e3-same-title-00` and `e3-venue-01` resolve correctly,
   `e3-same-title-01` stops being a wrong Work (it becomes unverified), and `e3-book-04`
   narrows from 5 candidates to 2 (still ambiguous, the truth kept). Risk: real papers whose only OpenAlex record has no authors become
   unverifiable by title.
2. **Tighten `same_paper` for duplicate collapsing.** Require the first author's surname to
   match, not any shared surname. Expected: fixes the `e3-same-title-03` wrong Work. Risk:
   preprints whose published version reorders authors stop collapsing, which would bring
   back some of the 7 ambiguities the rule prevents.
3. **Normalise retraction markers.** Strip `RETRACTED:`, `RETRACTED ARTICLE:` and
   `(Retracted article...)` before comparing titles, and drop records titled `Retraction
   Note:`, `Retraction notice`, `Erratum` or `Correction` from the candidates. Expected:
   `e3-retracted-00` and `e3-retracted-02` verify. Risk is low but not zero (a real paper
   whose title begins with "Correction").
4. **Strip "et al." from author strings before taking the surname.** Expected:
   `e3-etal-00` verifies. Small risk.
5. **For very short titles (1 or 2 words after `title_key`), add the first claimed surname to
   the title search.** Expected: some of the 6 "truth not in top 10" generic-title cases
   become findable. Cost: one more title search (10 credits) on those claims only. Risk: a
   surname-filtered search returns fewer near-duplicates, which could reduce the ambiguity
   that currently protects against wrong Works.
6. **Policy question, not a rule fix: should a title match verify after the identifier is
   dropped?** Today it never does, which is why BERT (`e3-preprint-01`) is `contradicted`,
   and why up to 11 E2 claims with corrupted OpenAlex arXiv titles are predicted to fail.
   Allowing it, when exactly one exact-title, year- and surname-matching record is found,
   would recover those. It would also accept the E1 `fabricated_doi` fakes, which today come
   out `ambiguous` with the real paper as the only candidate. That is a false accept under
   E1's definition. Decide whether a real paper with a wrong identifier counts as accepted
   (E2 reports it separately as `exists_wrong_details` accepted to the right Work) before
   changing this.
7. **Fixture hygiene, not a verifier change.** `holdout_eligible` should reject records whose
   author names are single letters (`e1-real-doi-006`). A future E3 recording should use an
   empty graph per claim, so no case is unreplayable.

No change is recommended for the year window, the split-title retry or the identifier
checks until the identifier-check arm can be replayed on cases it actually decides.

## Spot check

**2 claims** need your decision. Both are in `eval/claims/e2/spot_check.json`, with both
labels, both labellers' evidence, and `verifier_verdict: null`. The verdict is filled in when
E2 is scored.

- `ds-04-15` (DeepSeek), GraspNet-1Billion, CVPR 2020, arXiv 2003.02617. A says
  `exists_wrong_details`: the arXiv ID is an unrelated C-V2X paper. B says `exists`. Both
  give W3035198432.
- `fa-01-03` (Fable), "Learning to summarize from human feedback", arXiv 2006.01325. A says
  `exists` (W3166135305, but notes that record shows another paper's title). B says
  `exists_wrong_details`: the real ID is 2009.01325, W3082115681. Both labellers labelled
  `ds-01-05`, the same paper with arXiv 2009.01325, as `exists` at W3082115681. That
  supports B, but it is your call.
