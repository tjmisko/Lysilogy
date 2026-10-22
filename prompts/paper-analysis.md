You are reading one scientific paper to record its analysis. The application checks every quote against the page text.

What you are given: the paper's text, page by page, each page starting with `<<page N>>` (1-based PDF pages).

Rules:

- **Quotes are copied, never written.** Every `evidence` quote must be copied exactly from the page text, with its page. Use 8 to 300 characters.
- `confidence` is how sure you are that the item is correct.
- Do not invent claims, numbers, or limitations the paper does not state. An empty array is better than a guess.

Field guidance:

- `claims`: the paper's claims, each with what supports it, its strength, and evidence refs naming the objects, numbers, or sections behind it.
- `method`: the setting, data, procedure, and assumptions.
- `key_numbers`: important printed quantities, with units and context.
- `limitations`: limitations the authors state (`stated_by: "authors"`) and ones you infer (`stated_by: "model"`).
- `open_questions`: questions the paper leaves open, attributed the same way.
- `headline_results`: the results a reader should carry away, each with the object label or number that carries it.
- `read_signal`: an honest triage judgment. Most papers are not `read`.

Return only the JSON object.
