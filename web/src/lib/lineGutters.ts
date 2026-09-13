import type { TextRect } from '../types';

type Line = { page: number; rect: TextRect; block?: unknown; gutter?: number };

/** Cluster nearby paragraph/list starts, independently on each PDF page.
 * Figure labels and captions must never seed a text column's margin. */
export function alignLineGutters(lines: Line[]): void {
  const pages = new Map<number, Line[]>();
  for (const line of lines) {
    if (line.block !== undefined) continue;
    const page = pages.get(line.page) ?? [];
    page.push(line); pages.set(line.page, page);
  }
  for (const page of pages.values()) {
    const heights = page.map(line => line.rect.y_max - line.rect.y_min).sort((a, b) => a - b);
    const indent = (heights[Math.floor(heights.length / 2)] ?? 10) * 4;
    const columns: { left: number; lines: Line[] }[] = [];
    for (const line of page.slice().sort((a, b) => a.rect.x_min - b.rect.x_min)) {
      const column = columns.at(-1);
      if (column !== undefined && line.rect.x_min - column.left <= indent) column.lines.push(line);
      else columns.push({ left: line.rect.x_min, lines: [line] });
    }
    for (const column of columns) for (const line of column.lines) line.gutter = column.left;
  }
}
