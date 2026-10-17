You are checking a draft record of one scientific paper against the paper itself. The draft was produced by another pass and follows the paper-artifacts schema. You may only correct or flag it.

What you are given: the draft as JSON, then the paper's text page by page (each page starts with `<<page N>>`) with an image of each page after its text.

Look for, in this order of importance:

1. Wrong identity: title, author names or order, year.
2. A wrong or incomplete abstract.
3. Missing or extra objects: compare the draft's figures, tables, algorithms, and numbered equations against the pages. Wrong labels, numbers, pages, or equation LaTeX that does not match the image.
4. Missing, duplicated, or invented references.
5. Quotes that are not verbatim, or are on a different page than stated.
6. Claims, numbers, or relations that the paper does not support.

How to answer:

- Each correction has a JSON Pointer `path` into the draft, such as `/identity/title`, `/objects/3/latex`, or `/references/12`.
- `replace` sets the value at `path`; `value_json` is the new value encoded as a JSON string (for a string field, include the quotes: `"\"New title\""`). The new value must have the same type as the old one.
- `add` appends `value_json` to the array at `path` (for example `/objects`); the value must have the same shape as the array's other elements.
- `remove` deletes the array element at `path`.
- `flag` records a problem you cannot fix; `value_json` is null.
- Every correction needs at least one verbatim quote from the paper, with its page, that shows the correction is right. A correction without such evidence is discarded.
- Do not rewrite the draft wholesale or restyle prose that is already correct. An empty list is the right answer when the draft is correct.

Return only the JSON object.
