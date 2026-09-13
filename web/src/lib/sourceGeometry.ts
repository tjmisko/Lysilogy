import type { ReadingToken, TextSpan } from "./readingIndex";
import type { TextRect } from "../types";

export type SourceMark = TextSpan & {
  page: number;
  rect: TextRect;
  kind: "match" | "current" | "visual" | "cursor" | "block-cursor" | "block-visual" | "line-number";
  label?: string;
  active?: boolean;
  gutterX?: number;
  token: ReadingToken;
  geometry?: "native" | "estimated";
  group: number;
};
type Position = { node: Text; offset: number };
type Run = { text: string; starts: Position[]; ends: Position[]; bounds: DOMRect };
const ligatures: Record<string, string | undefined> = { "ﬀ": "ff", "ﬁ": "fi", "ﬂ": "fl", "ﬃ": "ffi", "ﬄ": "ffl", "ﬅ": "st", "ﬆ": "st" };

function addText(run: Run, node: Text): void {
  for (let at = 0; at < node.length;) {
    const point = node.data.codePointAt(at) ?? 0;
    const raw = String.fromCodePoint(point);
    const text = ligatures[raw] ?? (/^[\u00ad\u200b\0]$/u.test(raw) ? "" : raw);
    for (let unit = 0; unit < text.length; unit++) {
      run.text += text.charAt(unit);
      run.starts.push({ node, offset: at });
      run.ends.push({ node, offset: at + raw.length });
    }
    at += raw.length;
  }
}

function nativeRuns(host: HTMLElement): Run[] {
  const runs: Run[] = [];
  for (const span of host.querySelectorAll<HTMLElement>("span[data-text-index]")) {
    if (span.firstChild?.nodeType !== Node.TEXT_NODE || span.textContent.length === 0) continue;
    const bounds = span.getBoundingClientRect();
    if (bounds.width === 0 || bounds.height === 0) continue;
    const previous = runs.at(-1);
    // Bold/italic runs can split a word. Join only physically adjacent runs on
    // the same baseline, never a separate column or a subsequent line.
    if (previous !== undefined && Math.abs(previous.bounds.top - bounds.top) < Math.min(bounds.height, previous.bounds.height) * .3
      && bounds.left >= previous.bounds.left && Math.abs(bounds.left - previous.bounds.right) < bounds.height * .25) {
      addText(previous, span.firstChild as Text);
      previous.bounds = new DOMRect(previous.bounds.left, Math.min(previous.bounds.top, bounds.top), bounds.right - previous.bounds.left,
        Math.max(previous.bounds.bottom, bounds.bottom) - Math.min(previous.bounds.top, bounds.top));
    } else {
      const run: Run = { text: "", starts: [], ends: [], bounds };
      addText(run, span.firstChild as Text);
      runs.push(run);
    }
  }
  return runs;
}

function estimatedRect(mark: SourceMark): TextRect {
  const { token, rect } = mark;
  const length = Math.max(1, token.end - token.start);
  const from = Math.max(0, mark.start - token.start) / length;
  const to = Math.min(length, mark.end - token.start) / length;
  return { ...rect, x_min: rect.x_min + (rect.x_max - rect.x_min) * from, x_max: rect.x_min + (rect.x_max - rect.x_min) * to };
}

/** Use PDF.js's actual glyph advances for native text; OCR has word boxes only. */
export function resolveSourceMarks(marks: SourceMark[], host: HTMLElement | null, width: number, height: number, text: string): SourceMark[] {
  const origin = host?.getBoundingClientRect();
  const usable = host !== null && origin !== undefined && origin.width > 0 && origin.height > 0;
  const runs = usable ? nativeRuns(host) : [];
  const scaleX = usable ? origin.width / width : 1;
  const scaleY = usable ? origin.height / height : 1;
  const range = host?.ownerDocument.createRange();
  const matches = new Map<ReadingToken, { run: Run; at: number } | null>();
  const rectInPdf = (rect: DOMRect): TextRect => ({
    x_min: (rect.left - (origin?.left ?? 0)) / scaleX, x_max: (rect.right - (origin?.left ?? 0)) / scaleX,
    y_min: (rect.top - (origin?.top ?? 0)) / scaleY, y_max: (rect.bottom - (origin?.top ?? 0)) / scaleY,
  });
  const resolved = marks.flatMap((mark): SourceMark[] => {
    if (mark.kind.startsWith("block-") || mark.kind === "line-number") return [mark];
    const { token } = mark;
    if (!matches.has(token)) {
      let best: { run: Run; at: number } | null = null;
      let distance = Infinity;
      if (token.provenance === "native" && range !== undefined && token.text.length > 0) {
        for (const run of runs) {
          const runBox = rectInPdf(run.bounds);
          const center = (mark.rect.y_min + mark.rect.y_max) / 2;
          if (center < runBox.y_min - 4 || center > runBox.y_max + 4) continue;
          for (let at = run.text.indexOf(token.text); at >= 0; at = run.text.indexOf(token.text, at + 1)) {
            const from = run.starts[at], to = run.ends[at + token.text.length - 1];
            if (from === undefined || to === undefined) continue;
            range.setStart(from.node, from.offset); range.setEnd(to.node, to.offset);
            const box = rectInPdf(range.getBoundingClientRect());
            // Text can differ between Poppler and PDF.js. Never substitute a
            // matching word from another position/column when alignment fails.
            const tolerance = Math.max(mark.rect.y_max - mark.rect.y_min, (mark.rect.x_max - mark.rect.x_min) * .25);
            if (box.x_max < mark.rect.x_min - tolerance || box.x_min > mark.rect.x_max + tolerance
              || Math.abs(box.x_min - mark.rect.x_min) > tolerance * 1.5) continue;
            const score = Math.abs(box.x_min - mark.rect.x_min) + Math.abs(box.x_max - mark.rect.x_max)
              + 4 * Math.abs((box.y_min + box.y_max - mark.rect.y_min - mark.rect.y_max) / 2);
            if (score < distance) { distance = score; best = { run, at }; }
          }
        }
      }
      matches.set(token, best);
    }
    const match = matches.get(token);
    if (match != null && range !== undefined) {
      const from = match.run.starts[match.at + mark.start - token.start];
      const to = match.run.ends[match.at + mark.end - token.start - 1];
      if (from !== undefined && to !== undefined) {
        range.setStart(from.node, from.offset); range.setEnd(to.node, to.offset);
        return Array.from(range.getClientRects()).filter((rect) => rect.width > 0 && rect.height > 0)
          .map((rect) => ({ ...mark, rect: rectInPdf(rect), geometry: "native" }));
      }
    }
    return [{ ...mark, rect: estimatedRect(mark), geometry: "estimated" }];
  });
  return mergeSourceLines(resolved, text);
}

/** Fill ordinary word spaces, keeping paragraph/column and search-group gaps. */
export function mergeSourceLines(marks: SourceMark[], text: string): SourceMark[] {
  const result: SourceMark[] = [];
  for (const mark of marks) {
    const previous = result.at(-1);
    const a = previous?.rect, b = mark.rect;
    if (previous !== undefined && a !== undefined && ["match", "current", "visual"].includes(mark.kind)
      && previous.kind === mark.kind && previous.page === mark.page && previous.group === mark.group
      && mark.start >= previous.end && /^[\t ]*$/u.test(text.slice(previous.end, mark.start))
      && Math.min(a.y_max, b.y_max) - Math.max(a.y_min, b.y_min) >= .65 * Math.min(a.y_max - a.y_min, b.y_max - b.y_min)
      && b.x_min >= a.x_min && b.x_min - a.x_max < Math.max(a.y_max - a.y_min, b.y_max - b.y_min) * 3) {
      previous.rect = { x_min: Math.min(a.x_min, b.x_min), y_min: Math.min(a.y_min, b.y_min), x_max: Math.max(a.x_max, b.x_max), y_max: Math.max(a.y_max, b.y_max) };
      previous.end = mark.end;
      if (mark.geometry !== "native") previous.geometry = "estimated";
    } else result.push({ ...mark });
  }
  return result;
}
