/** Fixed-size pages over the filtered library so a 10k vault never renders at once. */
export const HOME_PAGE_SIZE = 250;

export type PageWindow = { page: number; pageCount: number; start: number; end: number };

/** Clamp `page` into range and describe the half-open slice it covers. */
export function pageWindow(total: number, page: number, size = HOME_PAGE_SIZE): PageWindow {
  const pageCount = Math.max(1, Math.ceil(Math.max(0, total) / Math.max(1, size)));
  const clamped = Math.min(Math.max(0, Math.trunc(page)), pageCount - 1);
  const start = Math.min(clamped * size, Math.max(0, total));
  return { page: clamped, pageCount, start, end: Math.min(start + size, Math.max(0, total)) };
}

/** The page that shows position `index`, or null when the item is not in the list. */
export function pageOfIndex(index: number, size = HOME_PAGE_SIZE): number | null {
  return Number.isInteger(index) && index >= 0 ? Math.floor(index / Math.max(1, size)) : null;
}

/** Human summary such as "1–250 of 10,951" without a locale surprise in tests. */
export function pageSummary(window: PageWindow, total: number): string {
  if (total === 0) return "0 papers";
  return `${window.start + 1}–${window.end} of ${total}`;
}
