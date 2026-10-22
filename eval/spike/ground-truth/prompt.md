# Ground-truth prompt

You are building a reference record for one academic paper. Read the attached
PDF and return a single JSON object matching `oracle.schema.json`. Return JSON
only, with no prose before or after it.

## Rules

- **Title.** Copy the title exactly as printed on the first page, including
  subtitle and capitalization. Do not normalize or correct it.
- **Authors.** Copy the authors in printed order, given name and family name
  ("David H. Autor", not "Autor, D. H."). Drop affiliations, email addresses,
  footnote symbols and equal-contribution marks. For long lists include every
  author the paper prints.
- **Abstract.** Copy the abstract verbatim, paragraphs separated by a blank
  line. Drop the printed "Abstract" heading and any keywords. If the paper
  prints no abstract, use `null`.
- **Sections.** Build a reading map, not a table of contents. Follow the policy
  below. Use 1-based PDF page numbers. `start_text` and `end_text` are short
  verbatim phrases from where the unit begins and ends.

## Section policy

Build a reading map of coherent, digestible topics, not a table of contents.

- Aim for 1–5 pages of actual content per unit, usually 1–2 pages. Prefer a
  small number of useful reading chunks. Two short fragments on opposite sides
  of a page break are not two pages of reading.
- Choose topic boundaries first. Keep a parent topic's introduction,
  mechanisms, variants, examples, and qualifications together when they fit
  within this range. A printed subheading is evidence of internal structure,
  not a demand for another map region.
- Keep a short methods setup with its method, an experiment with its ablations,
  and an argument with its examples. Merge only adjacent material that answers
  the same clear question. Do not combine unrelated major topics just to hit a
  page target.
- Split a topic approaching five pages at a meaningful internal turn. Rare
  shorter units are appropriate for a genuinely independent central idea or a
  short conclusion.
- Use a concise topic title rather than a breadcrumb of parent and subsection
  names. The summary synthesizes the whole unit in one or two sentences.
- Cover the substantive argument in order, without gaps or duplicate
  parent/child regions. Keep references as at most one optional navigation
  unit; do not divide a bibliography into chunks. Group appendices by useful
  topic.
- Before returning JSON, review the complete map and fold short dependent units
  into a parent or neighbor.

## Output

```json
{
  "title": "...",
  "authors": ["..."],
  "abstract": "...",
  "sections": [
    {
      "title": "...",
      "pages": {"start": 1, "end": 2},
      "source_span": {
        "start_text": "...",
        "start_page": 1,
        "end_text": "...",
        "end_page": 2
      },
      "summary": "..."
    }
  ]
}
```
