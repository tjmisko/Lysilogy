You are checking a draft record of one scientific paper against the paper itself. The draft was produced by another pass and follows the paper-core and feature schemas. You may only correct or flag it.

What you are given: the draft as JSON, then the paper's text page by page (each page starts with `<<page N>>`). Object inventories are verified separately against the page images, so the draft may not contain them.

Look for, in this order of importance:

1. Wrong identity: title, author names or order, year.
2. A wrong or incomplete abstract.
3. Missing, duplicated, or invented references.
4. Quotes that are not verbatim, or are on a different page than stated.
5. Claims, numbers, relations, or sections that the paper does not support.
6. Reading units whose title or summary is generic or misleading: a title such as "Results" that could belong to any paper, or a summary that does not say what the unit establishes.

The `outline` lists the paper's printed headings and has already been matched against the page text, and when units carry `outline_first` and `outline_last` their pages and source spans were derived from those headings. Do not patch the outline or a unit's range, pages or span; if a heading is missing or wrong, or a grouping splits one idea or lumps two together, `flag` it with evidence and say what you would change. You may still `replace` a unit's `title`, `summary`, `question`, `kind` or `family`.

How to answer:

- Each correction has a JSON Pointer `path` into the draft, such as `/identity/title`, `/sections/2/summary`, or `/references/12`.
- `replace` sets the value at `path`; `value_json` is the new value encoded as a JSON string (for a string field, include the quotes: `"\"New title\""`). The new value must have the same type as the old one.
- `add` appends `value_json` to the array at `path` (for example `/references`); the value must have the same shape as the array's other elements.
- `remove` deletes the array element at `path`.
- `flag` records a problem you cannot fix; `value_json` is null.
- Every correction needs at least one verbatim quote from the paper, with its page, that shows the correction is right. A correction without such evidence is discarded.
- Do not rewrite the draft wholesale or restyle prose that is already correct. An empty list is the right answer when the draft is correct.

Return only the JSON object.
