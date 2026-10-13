export type PassageSpan = { start: number; end: number };

/** Characters of the passage used to find it again when offsets have drifted. */
const PREFIX_LENGTH = 60;

function collapse(text: string): string {
  return text.replace(/\s+/gu, " ").trim();
}

/** The whitespace-collapsed text and, per collapsed code unit, its offset in `text`. */
function collapseWithOffsets(text: string): { collapsed: string; offsets: number[] } {
  const pieces: string[] = [];
  const offsets: number[] = [];
  let pendingSpace = -1;
  for (let at = 0; at < text.length; at += 1) {
    const unit = text[at] ?? "";
    if (/\s/u.test(unit)) {
      if (pieces.length > 0 && pendingSpace < 0) pendingSpace = at;
      continue;
    }
    if (pendingSpace >= 0) {
      pieces.push(" ");
      offsets.push(pendingSpace);
      pendingSpace = -1;
    }
    pieces.push(unit);
    offsets.push(at);
  }
  return { collapsed: pieces.join(""), offsets };
}

function nearestIndex(haystack: string, needle: string, hint: number): number {
  let best = -1;
  let from = 0;
  for (;;) {
    const found = haystack.indexOf(needle, from);
    if (found < 0) return best;
    if (best < 0 || Math.abs(found - hint) < Math.abs(best - hint)) best = found;
    if (found >= hint) return best;
    from = found + 1;
  }
}

/**
 * Find the span of `passage` in the reading-index `text`.
 *
 * `start` and `end` are the stored UTF-16 offsets and `passage` is the stored,
 * whitespace-collapsed text. When the offsets still select the passage they are
 * returned unchanged. Otherwise the passage, then its first 60 characters, is
 * searched for in the collapsed text, nearest to `start`. Returns null when the
 * passage is empty or cannot be found.
 */
export function locatePassage(text: string, start: number, end: number, passage: string): PassageSpan | null {
  const wanted = collapse(passage);
  if (wanted === "") return null;
  if (Number.isInteger(start) && Number.isInteger(end) && start >= 0 && end > start && end <= text.length
    && collapse(text.slice(start, end)) === wanted) {
    return { start, end };
  }
  const { collapsed, offsets } = collapseWithOffsets(text);
  const hint = offsets.findIndex((offset) => offset >= start);
  const nearest = hint < 0 ? collapsed.length : hint;
  let at = nearestIndex(collapsed, wanted, nearest);
  let length = wanted.length;
  if (at < 0) {
    at = nearestIndex(collapsed, wanted.slice(0, PREFIX_LENGTH), nearest);
    length = Math.min(wanted.length, collapsed.length - at);
  }
  if (at < 0) return null;
  const first = offsets[at];
  const last = offsets[at + length - 1];
  if (first === undefined || last === undefined) return null;
  return { start: first, end: last + 1 };
}
