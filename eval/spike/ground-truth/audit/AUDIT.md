# Second-pass oracle audit

The maps have broad topic coverage, but the first pass was too confident about their reliability. Working backward from the records to the supplied papers found substantive overclaims, ambiguous scope, and incomplete endpoints. Seven of eleven records were revised, with 20 logged edits. There are now 104 reading units, down from 105 after one merge.

## Material findings

| Record | Defect in the previous oracle | Correction and source evidence |
|---|---|---|
| Christiano 2017, units 3–4 | Reward-access wording blurred preference-trained agents with the true-reward baseline. “Online updates prevent exploitation” claimed a general safety result. | Distinguish reward access, identify single-run human-feedback results, and describe offline exploitation as empirical motivation for interleaved feedback rather than a guarantee. |
| Autor 2013, unit 7 | The summary concealed the difference between the raw and supply-discounted employment-loss calculations. | Include both 44% and 21% estimates (PDF pp. 19–20). |
| Autor 2013, units 10–11 | Transfer categories appeared equally established, and “findings survive” overstated robustness across outcomes. | Identify the insignificant medical-benefit estimate (Table 8, PDF p. 29). Manufacturing-employment effects persist, but intermediate-input adjustment makes nonmanufacturing-wage and household-wage-income estimates lose conventional significance (Table 10, PDF p. 32). |
| Hubinger 2019, unit 9 | “Removing pressure” overstated the argument and flattened a hypothesis into a result. | Preserve the hypothesized status and qualifications concerning random drift and complexity penalties (PDF p. 28). |
| Amodei 2016, unit 4 | Proposed oversight methods sounded established; the span omitted a substantive final footnote. | Use proposal language, include the subagent-alignment qualification, and extend the endpoint through the footnote (PDF p. 13). |
| Kaplan 2020, former units 8–9 | The supplemental unit claimed a floating figure that occurs before its start anchor. | Merge the adjacent limitations and efficiency material into one physical span, PDF pp. 22–24. |

## Boundary defects

- Hubinger’s glossary ended at “objective.”, which occurs twice on PDF p. 37. The endpoint now uniquely identifies the final entry rather than allowing the last entries to disappear.
- Dijkstra’s endpoint stopped mid-sentence; it now includes the printed conclusion on PDF p. 2.
- Autor’s unit 2 stopped before the terminal employment-share ratio on PDF p. 8.
- Seven Shannon endpoints stopped before final words or mathematical expressions (PDF pp. 10, 28, 35, 38, 45, 47, 51). They now include the relevant conclusions. The opening drop-cap word was also restored.

These were principally boundary and fidelity defects. This pass did not establish a newly missing whole main section. It also does not establish that every sentence, footnote, or graphical element has been exhaustively audited.

## Coverage assessment by record

| Paper | Units now | Second-pass disposition |
|---|---:|---|
| Amodei | 7 | Corrected proposal language and substantive-footnote boundary. |
| Autor | 15 | Corrected aggregate estimates, uncertainty, robustness scope, and formula endpoint. |
| Box | 5 | No material mismatch found in the backward checks; unchanged. |
| Christiano | 6 | Corrected reward-access ambiguity and safety overclaim. |
| Clark | 5 | No material mismatch found in the backward checks; unchanged. |
| Dijkstra | 1 | Completed the final sentence; unrelated letters remain excluded. |
| Granger–Newbold | 4 | No material mismatch found in the backward checks; unchanged. |
| Hubinger | 12 | Qualified the hypothesis and repaired glossary coverage. |
| Kaplan | 9 | Merged units to resolve figure ownership. |
| Putnam | 19 | No new material mismatch found; source and attribution limitations remain. |
| Shannon | 21 | Repaired incomplete endpoints and opening anchor. |

## What validation establishes—and does not

All eleven records pass the supplied schema. All 208 endpoints were checked: 202 match uniquely after text normalization, while six require explicit visual exceptions for extraction errors, line wrapping, or mathematical typography. Order checks pass using text positions and documented locators for those exceptions. Mathematical anchors linearize printed notation; they are not byte-identical PDF text. The full exceptions appear in `validation.json`.

Phrase existence and ordering cannot certify summary truth or complete coverage. No numerical coverage percentage is justified without defining whether the denominator includes references, footnotes, floats, and scan losses. The initial validation statement implied more than these checks established; it has been replaced.

Putnam remains unsuitable as fully verified ground truth: the provided excerpt has no author byline, so `authors` remains empty, and PDF p. 24 contains visibly clipped lines. Its chapter occupies the right half of PDF p. 1 through p. 29; the book-wide bibliography on pp. 30–31 is excluded. Autor’s floating appendix Table 5 remains acknowledged in the theoretical-appendix unit; this is a topic-purity limitation of the single-interval schema. Clark’s cover title and article-heading capitalization differ; the record follows the prompt’s first-page rule.

The corrected records are stronger reference drafts, with these limitations explicitly retained. `audit-changes.json` contains exact before/after text for review.
