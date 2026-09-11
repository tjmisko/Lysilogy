import type { PaperSection } from "../types";

export function sectionSourceSpan(section: PaperSection, pageCount: number) {
  const span = section.source_span;
  const valid = span != null && Number.isInteger(span.start.page) && Number.isInteger(span.end.page)
    && span.start.page >= 1 && span.end.page <= pageCount
    && Number.isInteger(span.start.start_token) && span.start.start_token >= 0
    && Number.isInteger(span.end.end_token) && span.end.end_token >= 0
    && (span.start.page < span.end.page || (span.start.page === span.end.page && span.start.start_token <= span.end.end_token));
  return valid ? span : null;
}

export function sectionPages(section: PaperSection, pageCount: number): number[] {
  if (!Number.isInteger(pageCount) || pageCount < 1) return [];
  const span = sectionSourceSpan(section, pageCount);
  const { start, end } = span !== null ? { start: span.start.page, end: span.end.page } : section.pages;
  if (!Number.isFinite(start) || !Number.isFinite(end) || start > end || end < 1 || start > pageCount) return [];
  const first = Math.max(1, Math.ceil(start));
  const last = Math.min(pageCount, Math.floor(end));
  return Array.from({ length: Math.max(0, last - first + 1) }, (_, index) => first + index);
}
