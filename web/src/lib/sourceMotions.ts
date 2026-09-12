import type { ReadingIndex, TextSpan } from "./readingIndex";

const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });
const segmentedSources = new Map<string, ReturnType<typeof graphemes.segment>>();

function segments(text: string): ReturnType<typeof graphemes.segment> {
  const cached = segmentedSources.get(text);
  if (cached !== undefined) return cached;
  // Segments.containing() locates a boundary directly; no document-sized array is
  // allocated on each keypress. Retain only the current/recent source strings.
  const result = graphemes.segment(text);
  if (segmentedSources.size >= 2) segmentedSources.delete(segmentedSources.keys().next().value ?? "");
  segmentedSources.set(text, result);
  return result;
}

/** The whole visible character containing a UTF-16 source offset. */
export function sourceGrapheme(text: string, offset: number): TextSpan {
  if (text.length === 0) return { start: 0, end: 0 };
  const at = Math.max(0, Math.min(text.length - 1, Number.isNaN(offset) ? 0 : Math.trunc(offset)));
  const character = segments(text).containing(at);
  if (character === undefined) return { start: at, end: at + 1 };
  return { start: character.index, end: character.index + character.segment.length };
}

/** Canonical cursor offset: the start of a complete grapheme, never half an emoji. */
export function sourceCursor(text: string, offset: number): number {
  return sourceGrapheme(text, offset).start;
}

/** Vim visual selections include both endpoint characters, in either direction. */
export function inclusiveSourceSpan(text: string, anchor: number, cursor: number): TextSpan {
  const first = sourceGrapheme(text, anchor);
  const last = sourceGrapheme(text, cursor);
  return { start: Math.min(first.start, last.start), end: Math.max(first.end, last.end) };
}

function nextObjectTarget(objects: TextSpan[], current: number, forward: boolean, target: (object: TextSpan) => number): number {
  let low = 0;
  let high = objects.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    const object = objects[middle];
    if (object === undefined) break;
    const position = target(object);
    if (forward ? position <= current : position < current) low = middle + 1;
    else high = middle;
  }
  const object = objects[forward ? low : low - 1];
  return object === undefined ? current : target(object);
}

/**
 * Source-coordinate Vim motions. Unrecognized keys return null so callers can
 * retain their other reader shortcuts. Document edges stop a recognized motion;
 * counts are capped at 999, and zero leaves the normalized cursor unchanged.
 *
 * e/E moves to this word's end from inside it, or the next end when already at
 * the end. ge/gE goes to the preceding end. Sentence motions use indexed starts:
 * ( first returns to this sentence's start; ) advances to the next sentence.
 */
export function moveSourceCursor(index: Pick<ReadingIndex, "text" | "objects">, cursor: number, key: string, count = 1): number | null {
  const characterMotion = ["h", "l", "ArrowLeft", "ArrowRight"].includes(key);
  const sentenceMotion = key === "(" || key === ")";
  const wordMotion = ["w", "W", "b", "B", "e", "E", "ge", "gE"].includes(key);
  if (!characterMotion && !sentenceMotion && !wordMotion) return null;
  const steps = Number.isFinite(count) ? Math.min(999, Math.max(0, Math.trunc(count))) : 1;
  let next = sourceCursor(index.text, cursor);
  const objects = sentenceMotion ? index.objects.sentence
    : ["W", "B", "E", "gE"].includes(key) ? index.objects.WORD : index.objects.word;
  const endMotion = ["e", "E", "ge", "gE"].includes(key);
  const forward = ["l", "ArrowRight", "w", "W", "e", "E", ")"].includes(key);
  const target = (object: TextSpan) => sourceCursor(index.text, endMotion ? object.end - 1 : object.start);
  for (let step = 0; step < steps; step++) {
    const before = next;
    if (characterMotion) {
      const character = sourceGrapheme(index.text, next);
      next = sourceCursor(index.text, forward ? character.end : character.start - 1);
    } else next = nextObjectTarget(objects, next, forward, target);
    if (next === before) break;
  }
  return next;
}

type LineWord = { token: ReadingIndex["tokens"][number]; rect: ReadingIndex["tokens"][number]["rects"][number] };
type SourceLine = { page: number; words: LineWord[] };
type SourceLines = { tokens: ReadingIndex["tokens"]; lines: SourceLine[]; byToken: Map<ReadingIndex["tokens"][number], number>; widths: Map<number, number> };
const lineIndices = new WeakMap<ReadingIndex, SourceLines>();
const tokenCharacters = new WeakMap<ReadingIndex["tokens"][number], number[]>();

function characters(token: ReadingIndex["tokens"][number]): number[] {
  const cached = tokenCharacters.get(token);
  if (cached !== undefined) return cached;
  const positions = Array.from(graphemes.segment(token.text), (character) => token.start + character.index);
  tokenCharacters.set(token, positions);
  return positions;
}

function linesFor(index: ReadingIndex): SourceLines {
  const cached = lineIndices.get(index);
  if (cached !== undefined) return cached;
  const result: SourceLines = { tokens: [], lines: [], byToken: new Map(), widths: new Map(index.pages.map((page) => [page.number, page.width])) };
  for (const token of index.tokens) {
    const rect = token.rects.find((item) => item.x_max > item.x_min && item.y_max > item.y_min);
    if (rect === undefined || token.end <= token.start) continue;
    result.tokens.push(token);
    const last = result.lines.at(-1);
    const previous = last?.words.at(-1);
    const previousRect = previous?.rect;
    const height = rect.y_max - rect.y_min;
    const sameLine = last?.page === token.page && previous !== undefined && previousRect !== undefined
      && Math.abs((rect.y_min + rect.y_max - previousRect.y_min - previousRect.y_max) / 2) <= .7 * Math.max(height, previousRect.y_max - previousRect.y_min)
      && rect.x_min >= previousRect.x_min - 2
      && rect.x_min - previousRect.x_max <= Math.max(height * 3, (result.widths.get(token.page) ?? 612) * .12)
      && !/[\r\n]/u.test(index.text.slice(previous.token.end, token.start));
    if (!sameLine) result.lines.push({ page: token.page, words: [] });
    result.lines.at(-1)?.words.push({ token, rect });
    result.byToken.set(token, result.lines.length - 1);
  }
  lineIndices.set(index, result);
  return result;
}

function wordX(word: LineWord, cursor: number): number {
  const starts = characters(word.token);
  let character = 0;
  while (character + 1 < starts.length && (starts[character + 1] ?? Infinity) <= cursor) character++;
  return word.rect.x_min + (word.rect.x_max - word.rect.x_min) * (character + .5) / Math.max(1, starts.length);
}

function lineCursor(index: ReadingIndex, line: SourceLine, x: number): number | null {
  let best: LineWord | null = null;
  let distance = Infinity;
  for (const word of line.words) {
    const next = Math.max(word.rect.x_min - x, x - word.rect.x_max, 0);
    if (next < distance) { best = word; distance = next; }
  }
  if (best === null) return null;
  // Where the target column falls in a real word space, retain that source
  // offset rather than forcing the cursor onto either neighboring word.
  for (let at = 1; at < line.words.length; at++) {
    const before = line.words[at - 1], after = line.words[at];
    if (before === undefined || after === undefined || x < before.rect.x_max || x > after.rect.x_min) continue;
    const gap = index.text.slice(before.token.end, after.token.start);
    if (/^[\t ]+$/u.test(gap) && after.rect.x_min - before.rect.x_max <= 2 * Math.min(before.rect.y_max - before.rect.y_min, after.rect.y_max - after.rect.y_min)) {
      const fraction = (x - before.rect.x_max) / Math.max(1, after.rect.x_min - before.rect.x_max);
      return sourceCursor(index.text, before.token.end + Math.min(gap.length - 1, Math.floor(fraction * gap.length)));
    }
  }
  const positions = characters(best.token);
  const fraction = (x - best.rect.x_min) / (best.rect.x_max - best.rect.x_min);
  const character = Math.max(0, Math.min(positions.length - 1, Math.floor(fraction * positions.length)));
  return sourceCursor(index.text, positions[character] ?? best.token.start);
}

/**
 * Move through visual source lines in the index's reading order, including
 * column/page transitions. Counted motions retain the original horizontal
 * position (scaled for a changed page width), even across short intervening
 * lines. Character positions inside word boxes are estimates; the overlay can
 * refine them using the PDF text layer's glyph advances.
 *
 * Inter-word spaces belong to their shared line. Wrapped/paragraph/page gaps
 * sit between the adjacent lines: j reaches the following line and k the
 * preceding line. Null means there is no usable geometry, not a document edge.
 * The line index is built once per immutable source index; keypresses use a
 * binary token lookup and scan only the destination line.
 */
export function moveSourceLine(index: ReadingIndex, cursor: number, direction: number, count = 1): number | null {
  const layout = linesFor(index);
  if (layout.lines.length === 0) return null;
  const at = sourceCursor(index.text, cursor);
  const steps = Number.isFinite(count) ? Math.min(999, Math.max(0, Math.trunc(count))) : 1;
  if (steps === 0 || direction === 0) return at;
  const down = direction > 0;
  let low = 0, high = layout.tokens.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if ((layout.tokens[middle]?.start ?? Infinity) <= at) low = middle + 1;
    else high = middle;
  }
  const before = layout.tokens[low - 1], after = layout.tokens[low];
  const beforeLine = before === undefined ? undefined : layout.byToken.get(before);
  const afterLine = after === undefined ? undefined : layout.byToken.get(after);
  const containing = before !== undefined && at >= before.start && at < before.end && beforeLine !== undefined;
  const inlineGap = beforeLine !== undefined && beforeLine === afterLine;
  const origin = containing || inlineGap ? beforeLine : down ? beforeLine ?? -1 : afterLine ?? layout.lines.length;
  if (down ? origin >= layout.lines.length - 1 : origin <= 0) return at;
  const target = layout.lines[Math.max(0, Math.min(layout.lines.length - 1, origin + (down ? steps : -steps)))];
  if (target === undefined) return at;
  const originToken = before ?? after;
  const originLine = originToken === undefined ? undefined : layout.lines[layout.byToken.get(originToken) ?? -1];
  const originWord = originLine?.words.find((word) => word.token === originToken);
  if (originWord === undefined) return null;
  let x = containing ? wordX(originWord, at) : originWord.rect.x_max;
  if (inlineGap && before !== undefined && after !== undefined) {
    const nextWord = originLine?.words.find((word) => word.token === after);
    if (nextWord !== undefined) {
      const fraction = (at - before.end + .5) / Math.max(1, after.start - before.end);
      x = originWord.rect.x_max + fraction * (nextWord.rect.x_min - originWord.rect.x_max);
    }
  } else if (before === undefined) x = originWord.rect.x_min;
  const sourceWidth = layout.widths.get(originWord.token.page) ?? 612;
  const targetWidth = layout.widths.get(target.page) ?? sourceWidth;
  return lineCursor(index, target, x * targetWidth / Math.max(1, sourceWidth));
}
