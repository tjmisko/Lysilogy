import type { LayoutPage, PaperSection, TextRect } from "../types";
import type { ReadingIndex } from "./readingIndex";
import { sectionPageCrop } from "./sectionCrop.ts";

const contains = (outer: TextRect, inner: TextRect) => inner.x_min >= outer.x_min - 1 && inner.x_max <= outer.x_max + 1
  && inner.y_min >= outer.y_min - 1 && inner.y_max <= outer.y_max + 1;

/** Check the same clipped regions used by the source reader, including columns and footnotes. */
export function externalSectionFigures(index: ReadingIndex, section: PaperSection, pages: LayoutPage[]) {
  const crops = new Map(pages.map((page) => [page.number, sectionPageCrop(section, page, pages.length)]));
  const shown = (page: number, rects: TextRect[]) => rects.length > 0 && rects.every((rect) => crops.get(page)?.regions.some((region) => contains(region, rect)));
  return index.figures.filter((figure) => {
    const citedHere = figure.references.some((reference) => shown(reference.page, reference.rects));
    if (!citedHere) return false;
    const caption = index.tokens.filter((token) => token.start < figure.end && token.end > figure.start).flatMap((token) => token.rects);
    return !shown(figure.page, caption) || figure.rect === null || !shown(figure.page, [figure.rect]);
  });
}
