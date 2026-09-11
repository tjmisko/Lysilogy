import type { LayoutPage, LayoutToken } from "../types";

const median = (values: number[]) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)] ?? 0;
const height = (token: LayoutToken) => Math.max(...token.rects.map((r) => r.y_max - r.y_min));

/** Footer position and small type identify note bodies; raised callouts decide
 * ownership. Merely falling between a section's start and end isn't sufficient.
 */
export function sectionFootnotes(page: LayoutPage, first: number, last: number): { notes: Set<number>; retained: Set<number> } {
  const lines = new Map<number, LayoutToken[]>();
  for (const token of page.tokens) { const line = lines.get(token.line) ?? []; line.push(token); lines.set(token.line, line); }
  const bodyHeight = median(page.tokens.filter((t) => t.text.length > 2 && t.rects.every((r) => r.y_max-r.y_min < 30)).map(height));
  const notes = new Set<number>(), retained = new Set<number>();
  const groups: { marker: string; tokens: LayoutToken[] }[] = [];
  let current: typeof groups[number] | undefined;
  for (const tokens of lines.values()) {
    const firstToken = tokens[0];
    if (firstToken === undefined) continue;
    const top = Math.min(...tokens.flatMap((t) => t.rects.map((r) => r.y_min)));
    const small = median(tokens.map(height)) < bodyHeight * .93;
    const marker = firstToken.text.match(/^(\d{1,2}|[*†‡])(?=\p{L}|$)/u)?.[1];
    if (top > page.height * .55 && small && marker !== undefined) {
      current = { marker, tokens: [] }; groups.push(current);
    }
    if (!small || top < page.height * .55) current = undefined;
    if (current !== undefined) { current.tokens.push(...tokens); tokens.forEach((t) => notes.add(t.index)); }
  }
  for (const group of groups) {
    const referenced = Array.from(lines.values()).some((tokens) => {
      const prose = tokens.filter((t) => !notes.has(t.index));
      const baselineTop = median(prose.flatMap((t) => t.rects.map((r) => r.y_min)));
      return prose.some((t) => {
        if (t.index < first || t.index > last) return false;
        if (t.text === group.marker) return height(t) < bodyHeight * .85 && t.rects.some((r) => r.y_min < baselineTop - .4);
        const match = t.text.match(/\p{L}(\d{1,2}|[*†‡])$/u);
        return match?.[1] === group.marker && t.rects.some((r) => r.y_min < baselineTop - .4);
      });
    });
    if (referenced) group.tokens.forEach((t) => retained.add(t.index));
  }
  return { notes, retained };
}
