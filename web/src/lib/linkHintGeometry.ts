import type { PaperLink } from "./paperLinks";
import { tokensInSpan, type ReadingIndex } from "./readingIndex.ts";
import { resolveSourceMarks, type SourceMark } from "./sourceGeometry.ts";
import type { SectionCrop } from "./sectionCrop";
import type { TextRect } from "../types";

export type HintBox = { left: number; top: number; right: number; bottom: number };
export type PositionedLink = { link: PaperLink; box: HintBox };
export const overlaps = (a: HintBox, b: HintBox) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

export function projectRect(root: HTMLElement, page: number, rect: TextRect): HintBox | null {
  const host = root.querySelector<HTMLElement>(`.pdf-text-layer-host[data-page="${page}"]`);
  const surface = host?.closest<HTMLElement>(".pdf-page-surface");
  if (host === null || surface == null) return null;
  const box = host.getBoundingClientRect();
  const width = Number(surface.dataset.pdfWidth), height = Number(surface.dataset.pdfHeight);
  if (!(width > 0 && height > 0 && box.width > 0 && box.height > 0)) return null;
  return { left: box.left + rect.x_min * box.width / width, right: box.left + rect.x_max * box.width / width,
    top: box.top + rect.y_min * box.height / height, bottom: box.top + rect.y_max * box.height / height };
}

/** Visibility follows both the viewport and the reader's exact section/visual crops. */
export function positionLinks(root: HTMLElement, links: PaperLink[], index: ReadingIndex | null, crops: Map<number, SectionCrop | null>): PositionedLink[] {
  const viewport = root.querySelector(".pdf-viewport")?.getBoundingClientRect();
  if (viewport === undefined) return [];
  const resolved = new Map<number, TextRect[]>();
  for (const page of new Set(links.map((link) => link.page))) {
    if (index === null) break;
    const host = root.querySelector<HTMLElement>(`.pdf-text-layer-host[data-page="${page}"]`);
    if (host === null) continue;
    const dimensions = index.pages.find((item) => item.number === page);
    if (dimensions === undefined) continue;
    const marks: SourceMark[] = links.flatMap((link, group) => {
      const span = link.span;
      return link.page !== page || span === undefined ? [] : tokensInSpan(index, span).filter((token) => token.page === page).flatMap((token) => token.rects.map((rect) => ({
        page, rect, token, group, kind: "match" as const, start: Math.max(token.start, span.start), end: Math.min(token.end, span.end),
      })));
    });
    for (const mark of resolveSourceMarks(marks, host, dimensions.width, dimensions.height, index.text)) resolved.set(mark.group, [...resolved.get(mark.group) ?? [], mark.rect]);
  }
  return links.flatMap((link, at) => {
    const window = root.querySelector(`[data-pdf-page="${link.page}"] .pdf-page-window`)?.getBoundingClientRect();
    if (window === undefined) return [];
    const clip = { left: Math.max(0, viewport.left, window.left), top: Math.max(0, viewport.top, window.top),
      right: Math.min(globalThis.innerWidth, viewport.right, window.right), bottom: Math.min(globalThis.innerHeight, viewport.bottom, window.bottom) };
    const crop = crops.get(link.page);
    for (const rect of resolved.get(at) ?? link.rects) {
      if (crop != null && !crop.regions.some((region) => rect.x_min >= region.x_min - 1 && rect.x_max <= region.x_max + 1 && rect.y_min >= region.y_min - 1 && rect.y_max <= region.y_max + 1)) continue;
      const box = projectRect(root, link.page, rect);
      if (box !== null && overlaps(box, clip)) return [{ link, box: { left: Math.max(box.left, clip.left), top: Math.max(box.top, clip.top), right: Math.min(box.right, clip.right), bottom: Math.min(box.bottom, clip.bottom) } }];
    }
    return [];
  });
}

export function placeHintBadges(items: PositionedLink[], width: number, viewport: HintBox): HintBox[] {
  const placed: HintBox[] = [];
  const positions = (anchor: number, low: number, high: number, step: number) => {
    const clamp = (value: number) => Math.max(low, Math.min(value, high));
    return [...new Set(Array.from({ length: 2 * Math.ceil(Math.max(0, high - low) / step) + 1 }, (_, at) =>
      clamp(anchor + (at % 2 === 1 ? Math.ceil(at / 2) : -at / 2) * step)))];
  };
  for (const item of items) {
    const left = Math.max(viewport.left, Math.min(item.box.left, viewport.right - width));
    const top = Math.max(viewport.top, Math.min(item.box.top - 15, viewport.bottom - 22));
    let selected: HintBox | undefined;
    // Grouped citations can share one anchor; their keys must remain separately readable.
    // Search both directions at page edges, keeping every badge inside the reader.
    for (const y of positions(top, viewport.top, viewport.bottom - 22, 24)) {
      for (const x of positions(left, viewport.left, viewport.right - width, width + 4)) {
        if (!placed.some((box) => overlaps({ left: x - 2, top: y - 2, right: x + width + 2, bottom: y + 22 }, box))) {
          selected = { left: x, top: y, right: x + width, bottom: y + 20 }; break;
        }
      }
      if (selected !== undefined) break;
    }
    placed.push(selected ?? { left, top, right: left + width, bottom: top + 20 });
  }
  return placed;
}
