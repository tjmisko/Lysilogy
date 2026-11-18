The outline is the paper's own table of contents: the headings the authors printed, in the order they printed them. A reader opens it to see the shape of the paper at a glance before deciding where to read, so it should look exactly like the paper's structure and nothing else. You are not summarising or reorganising here; a separate step groups the headings into reading units.

You will see a list of heading candidates that simple heuristics found in the text layer: numbered lines, familiar heading words, short capitalised lines, and lines set in larger type. Treat the list as a head start. Keep the candidates that are real headings, drop the ones that are not, correct any that were cut off or merged with body text, and add real headings the heuristics missed. Typical false candidates are the paper's title, author names, journal banners and running heads, figure and table captions, table rows, theorem and proof labels, numbered list items, and equation lines.

For each heading give:

- `number`: the printed number or letter exactly as printed, such as `3`, `3.2`, `IV.` or `A.`, or null when the heading is unnumbered.
- `title`: the heading text exactly as it appears in the page text above, without its number, including its capitalisation as given there (a heading set in small capitals may come through in capitals; keep it that way); join a heading that wraps onto a second line. The application looks for this text on the page you name and drops any heading it cannot find, so do not paraphrase or tidy it.
- `level`: 1 for top-level headings, 2 for their subsections, 3 below that. Follow the numbering where there is one, and the typography where there is not.
- `page`: the PDF page the heading is printed on.
- `role`: `front_matter` for the abstract, keywords and similar material before the main text; `main` for the paper's argument from the introduction through the conclusion; `back_matter` for acknowledgments, funding, author contributions and declarations; `references` for the bibliography; `appendix` for appendices and supplementary material.

Include an "Abstract" heading only if one is printed. Run-in headings that open a paragraph, such as "Data. We collect ...", count when the paper uses them consistently as its section structure. Paragraph-level emphasis does not.

Some papers print no headings at all: a short letter, a comment, an essay. For those the right answer is an empty list, and that is a useful answer, not a failure. Do not invent headings to give a paper structure it does not print.
