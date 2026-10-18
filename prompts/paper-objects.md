You are listing the objects printed on a few pages of a scientific paper: figures, tables, algorithms, and numbered displayed equations. Other pages are handled separately, so list only what is printed on the pages you are given.

What you are given: the text of each page, starting with a marker like `<<page 7>>` (1-based PDF page), followed by an image of that page. The images are the evidence for equations, figures and tables.

Rules:

- **Figures, tables, algorithms:** list one whose caption is printed on these pages. A figure with panels (a), (b), (c) under one caption is one figure. Give the printed label (`Figure 3`, `Table IV`, `Algorithm 1`), the number alone (`3`, `IV`, `1`), and the caption copied verbatim from the page text.
- **Equations:** list every displayed equation whose number is printed on these pages, such as `(12)`, `(A.3)` or `(2.7)`. Read its LaTeX off the page image. One object per printed number: when several lines carry several numbers, list each number separately; never write a range like `24-28`. Skip unnumbered displays. Equations have no caption: leave `caption` null.
- **Not objects:** a mention in the running text ("as shown in Figure 3", "by (12)") is not the object. Only the printed caption or the equation number itself counts.
- `page` is the page where the caption or the equation number is printed.
- For every object give a short plain-language meaning, why it matters in the paper, and the pages you can see referring to it (leave the list empty if none are in view).
- An empty list is the right answer when these pages print no objects.

Return only the JSON object.
