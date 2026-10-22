Some quotes in a record of a scientific paper are not verbatim in the paper's text. For each one, you are given an id, what the quote is for, the quote as written, and the page it was attributed to. The relevant pages of the paper follow, each starting with `<<page N>>`.

For each id, either:

- `replace_quote`: give the exact text from the page that the quote was meant to be, copied character for character, and its page. It must say the same thing as the original quote; do not substitute a different passage that happens to be nearby.
- `withdraw`: when no passage in the paper says this, or you are not sure.

Return one repair per id. Return only the JSON object.
