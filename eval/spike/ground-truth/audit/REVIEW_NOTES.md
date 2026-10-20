# Review notes

The archive contains one schema-conforming JSON object per paper in `oracles/<stem>.json`. Page numbers always refer to 1-based PDF pages. Reading units may share a PDF page when their text spans are adjacent; they do not duplicate parent and child material.

## Source limitations and review decisions

- **putnam1975:** The supplied excerpt has no author byline. `authors` is therefore `[]`; an editor should confirm and add the attribution before promoting this record into a fully checked oracle. Each PDF page is a two-page book spread. The chapter starts on the right side of PDF page 1 (printed page 215), following unrelated material, and ends on PDF page 29 (printed page 271). PDF pages 30–31 contain a book-wide bibliography and are omitted. PDF page 24 is visibly clipped at the top and bottom; its argument remains identifiable, but those clipped lines cannot be transcribed in full from this scan.
- **dijkstra1968:** Only Dijkstra’s letter on PDF pages 1–2 is included. Other letters in the four-page scan are excluded. The author’s signature is on page 2. Keywords are not an abstract.
- **box1976 / clark1998:** PDF page 1 is a JSTOR cover sheet; the article begins on PDF page 2. Titles follow the supplied first-page text as instructed. In Clark the cover title is `The Extended Mind`, while the article heading is `The extended mind`.
- **Author initials:** Printed initials are retained rather than expanded from outside sources (including Christiano, Granger–Newbold, Dijkstra, and Shannon).
- **Abstracts:** The six printed abstracts are transcribed without their headings or classification codes. Line wrapping and presentation ligatures are normalized; lexical hyphens such as `sample-efficient` and `import-competing` are preserved. Papers without printed abstracts use `null`.
- **Maps:** Bibliographies, acknowledgments, contents pages, and figure lists are not separate navigation units. Substantive appendices are included. Autor’s Table 5 in the appendix floats on the first page of the theoretical appendix; its presence is acknowledged in that unit’s summary.

## Validation and second-pass audit

See `AUDIT.md` for findings and remaining limitations, `audit-changes.json` for 20 logged edits across seven records, and `validation.json` for structural checks. The revised set contains 104 units. All records pass the schema; 202 endpoints match normalized text and six have documented visual exceptions. Phrase matching and ordering do not establish complete semantic coverage. Mathematical endpoints linearize printed notation. Putnam attribution and clipped source lines remain unresolved.
