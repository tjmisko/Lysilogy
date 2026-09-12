import type { TextRect } from "../types";

export type TextSpan = { start: number; end: number };
export type ReadingToken = TextSpan & { text: string; page: number; rects: TextRect[]; provenance: "native" | "ocr" };
export type ReadingPage = TextSpan & { number: number; width: number; height: number; provenance: "native" | "ocr" | "unavailable"; confidence: number | null };
export type ReadingIndex = {
  schema_version: number;
  text: string;
  pages: ReadingPage[];
  tokens: ReadingToken[];
  objects: Record<"word" | "WORD" | "sentence", TextSpan[]> & { paragraph: (TextSpan & { kind: string })[] };
  figures: { id: string; label: string; page: number; caption: string; start: number; end: number; rect: TextRect | null; confidence: string; references: (TextSpan & { page: number; rects: TextRect[] })[] }[];
  gaps: { page: number; reason: string }[];
};

export function readingIndexUrl(sourceUrl: string): string {
  return sourceUrl.replace(/\/source(?:\?.*)?$/u, "/reading-index");
}

export function objectAt(index: ReadingIndex, cursor: number, key: string, around: boolean): TextSpan | null {
  const kinds: Record<string, keyof ReadingIndex["objects"] | undefined> = { w: "word", W: "WORD", s: "sentence", p: "paragraph" };
  const kind = kinds[key];
  if (kind === undefined) return null;
  const spans = index.objects[kind];
  const found = spans.find((span) => span.start <= cursor && span.end > cursor) ?? spans.find((span) => span.start > cursor) ?? spans.at(-1);
  if (found === undefined) return null;
  let { start, end } = found;
  if (around) {
    const boundary = kind === "word" || kind === "WORD" ? /[\t ]/u : /\s/u;
    while (end < index.text.length && boundary.test(index.text[end] ?? "")) end++;
    if (end === found.end) while (start > 0 && boundary.test(index.text[start - 1] ?? "")) start--;
  }
  return { start, end };
}

export function tokensInSpan(index: ReadingIndex, span: TextSpan): ReadingToken[] {
  let low = 0;
  let high = index.tokens.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if ((index.tokens[middle]?.end ?? 0) <= span.start) low = middle + 1;
    else high = middle;
  }
  const result: ReadingToken[] = [];
  for (let at = low; at < index.tokens.length; at++) {
    const token = index.tokens[at];
    if (token === undefined || token.start >= span.end) break;
    result.push(token);
  }
  return result;
}

export function nearestToken(index: ReadingIndex, page: number, x: number, y: number): ReadingToken | null {
  let best: ReadingToken | null = null;
  let distance = Infinity;
  for (const token of index.tokens) {
    if (token.page !== page) continue;
    for (const rect of token.rects) {
      const dx = Math.max(rect.x_min - x, x - rect.x_max, 0);
      const dy = Math.max(rect.y_min - y, y - rect.y_max, 0);
      const next = dx * dx + dy * dy * 4;
      if (next < distance) { best = token; distance = next; }
    }
  }
  return best;
}
