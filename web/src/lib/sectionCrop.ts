import type { LayoutPage, PaperSection, TextRect } from "../types";
import { sectionSourceSpan } from "./sectionScope.ts";

export type SectionCrop = { bounds: TextRect; regions: TextRect[] };

function bounds(rects: TextRect[]): TextRect {
  return {
    x_min: Math.min(...rects.map((r) => r.x_min)), y_min: Math.min(...rects.map((r) => r.y_min)),
    x_max: Math.max(...rects.map((r) => r.x_max)), y_max: Math.max(...rects.map((r) => r.y_max)),
  };
}

function subtract(rect: TextRect, cut: TextRect): TextRect[] {
  const left = Math.max(rect.x_min, cut.x_min), right = Math.min(rect.x_max, cut.x_max);
  const top = Math.max(rect.y_min, cut.y_min), bottom = Math.min(rect.y_max, cut.y_max);
  if (left >= right || top >= bottom) return [rect];
  return [
    { ...rect, y_max: top }, { ...rect, y_min: bottom },
    { x_min: rect.x_min, x_max: left, y_min: top, y_max: bottom },
    { x_min: right, x_max: rect.x_max, y_min: top, y_max: bottom },
  ].filter((r) => r.x_max > r.x_min && r.y_max > r.y_min);
}

/** Keep the original page geometry, including figures between selected lines.
 * Separate reading columns remain separate regions; an outer bounding box alone
 * would expose the preceding column at the top of a boundary page.
 */
export function sectionPageCrop(section: PaperSection, page: LayoutPage, pageCount: number): SectionCrop | null {
  const span = sectionSourceSpan(section, pageCount);
  if (span === null || page.number < span.start.page || page.number > span.end.page
    || !Number.isFinite(page.width) || !Number.isFinite(page.height) || page.width <= 0 || page.height <= 0) return null;
  const first = page.number === span.start.page ? span.start.start_token : 0;
  const last = page.number === span.end.page ? span.end.end_token : Infinity;
  const valid = (r: TextRect) => Object.values(r).every(Number.isFinite) && r.x_min >= 0 && r.y_min >= 0
    && r.x_max <= page.width && r.y_max <= page.height && r.x_min < r.x_max && r.y_min < r.y_max;
  if (!page.tokens.some((t) => t.index === first) || (last !== Infinity && !page.tokens.some((t) => t.index === last))) return null;
  if (page.tokens.some((t) => t.rects.length === 0 || !t.rects.every(valid))) return null;
  const lines = new Map<number, typeof page.tokens>();
  for (const token of page.tokens) {
    const line = lines.get(token.line) ?? [];
    line.push(token); lines.set(token.line, line);
  }
  const selected: TextRect[] = [], excluded: TextRect[] = [];
  for (const tokens of lines.values()) {
    const rects = tokens.flatMap((t) => t.rects).filter(valid);
    if (rects.length === 0) continue;
    const box = bounds(rects);
    const folio = /^\d+$/u.test(tokens.map((t) => t.text).join("")) && box.y_min > page.height * .85
      && Math.abs((box.x_min + box.x_max) / 2 - page.width / 2) < page.width * .1;
    const kept = tokens.filter((t) => !folio && t.index >= first && t.index <= last);
    const keptRects = kept.flatMap((t) => t.rects);
    if (keptRects.length > 0) selected.push(bounds(keptRects));
    excluded.push(...tokens.filter((t) => !kept.includes(t)).flatMap((t) => t.rects).filter(valid));
  }
  if (selected.length === 0) return null;
  // Bridge successive lines within a column, preserving diagrams and equations
  // in the space between them. Never bridge a return to the top of a new column.
  const outer = bounds(selected);
  const interrupted = excluded.some((r) => r.x_min < outer.x_max && r.x_max > outer.x_min && r.y_min < outer.y_max && r.y_max > outer.y_min);
  // An uninterrupted section keeps its whole content rectangle, so a large
  // figure flanked by short lines isn't narrowed to the width of those lines.
  const regions = interrupted ? selected.map((r) => ({ ...r })) : [outer];
  if (interrupted) {
    for (let i = 1; i < selected.length; i++) {
      const previous = selected[i - 1], next = selected[i];
      if (previous === undefined || next === undefined) continue;
      if (next.y_min >= previous.y_min && Math.min(previous.x_max, next.x_max) > Math.max(previous.x_min, next.x_min)) {
        regions.push(bounds([previous, next]));
      }
    }
  }
  const padded = regions.map((r) => ({ x_min: Math.max(0, r.x_min - 6), x_max: Math.min(page.width, r.x_max + 6),
    y_min: Math.max(0, r.y_min - 3), y_max: Math.min(page.height, r.y_max + 3) }));
  // Guard mixed lines and unusual PDF reading order as well as ordinary columns.
  const clipped = excluded.reduce((remaining, r) => remaining.flatMap((region) => subtract(region,
    { x_min: r.x_min - .5, x_max: r.x_max + .5, y_min: r.y_min - .5, y_max: r.y_max + .5 })), padded);
  return clipped.length === 0 ? null : { bounds: bounds(clipped), regions: clipped };
}
