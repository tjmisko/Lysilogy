# Feature request: compact, wrapping reader navigation with branding in the library sidebar

## Problem

The paper citation currently collides with the Abstract / Overview / Glossary / Text controls.
Long titles and author lists, an open library, narrow windows, and browser zoom make the overlap
worse. The app name takes space needed by reading controls. The PDF view also spends substantial
height on app navigation and source controls.

## Requested behavior

- Move the Lysilogy mark/name into the library sidebar, immediately above the Vault label.
  It remains a keyboard-accessible link to the paper-grid home page.
- Keep all four analyzed-paper phases available and legible: Abstract, Overview, Glossary, Text.
  Unanalyzed papers retain only PDF / Text and their single Analyze action.
- Give phase controls a stable layout priority. Put author/year/title metadata in the remaining
  space, wrapping onto another line when necessary instead of overlapping or pushing phases away.
- Use natural, bounded wrapping for long titles, long author lists, small screens, and increased
  text size. Do not rely on a fixed-width citation or duplicate paper headings.
- Preserve a way to reach home and the sidebar when the sidebar is closed, without reinstating
  a large branding block in the reading bar.
- In PDF/section reading, navigation and source controls can hide by default and reveal from the
  top edge or an explicit keyboard toggle. Hidden controls must not capture focus or leave a blank
  strip. Revealed controls must remain usable while hovered/focused.

## Acceptance criteria

1. At 390, 768, 1280, and 2048 CSS pixels, long citations never overlap phase buttons or actions.
2. The four analyzed-paper controls keep their order and remain keyboard reachable; PDF/Text
   remains the unanalyzed-paper navigation.
3. Opening/closing the sidebar and switching phases does not lose the paper or reset its reader.
4. Branding appears above Vault in the sidebar and opens home by pointer or keyboard.
5. A hidden toolbar has no layout footprint; top-edge reveal and the documented toggle both work.
6. Section crops, digest height, PDF fit width/height, and notes layout use the actual available
   viewport after toolbar changes. Open dialogs retain their own keyboard/focus behavior.

## Validation

Browser checks with a very long title, 24 authors, mapped/unmapped papers, a visible/hidden sidebar,
mobile widths, keyboard-only operation, and live viewport resizing. Include a regression assertion
that phase controls and the citation have non-overlapping rectangles.
