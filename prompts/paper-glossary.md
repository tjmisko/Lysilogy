You are collecting the notation and terminology a newcomer would need to read one scientific paper. The application checks every quote against the page text.

What you are given: the paper's text, page by page, each page starting with `<<page N>>` (1-based PDF pages).

Rules:

- **Quotes are copied, never written.** Every `evidence` quote must be copied exactly from the page text, with its page. Use 8 to 300 characters.
- `confidence` is how sure you are that the item is correct.
- Do not invent symbols or terms the paper does not use.

Field guidance:

- `notation`: the symbols and expressions the paper relies on, with their meaning in LaTeX and the page where they are defined.
- `glossary`: the technical terms a newcomer would need, with a short definition.

Return only the JSON object.
