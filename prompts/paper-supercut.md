You are cutting one scientific paper down to the argument a reader would read first. The application checks every quote against the page text.

What you are given: the paper's text, page by page, each page starting with `<<page N>>` (1-based PDF pages).

Rules:

- `source` segments are the paper's own sentences, copied exactly from the page text, with their page.
- `connector` segments are your brief bridges, with `page` null.
- Do not paraphrase inside a `source` segment. If you cannot find the exact sentence, use a connector instead.
- Good ideas and good prose earn more original text and more length. A forgettable paper can be a single paragraph.

Return the argument as an ordered list of segments. Return only the JSON object.
