import type { TextSpan } from "./readingIndex";

export type RegexResult = { matches: TextSpan[]; truncated: boolean; error: string | null };

// Runs only in a disposable worker in the reader. A parent deadline also covers
// catastrophic backtracking inside a single exec() call.
export function findRegexMatches(text: string, pattern: string, limit = 10000): RegexResult {
  try {
    const expression = new RegExp(pattern, /[A-Z]/u.test(pattern) ? "gu" : "giu");
    const matches: TextSpan[] = [];
    let match: RegExpExecArray | null;
    while ((match = expression.exec(text)) !== null) {
      if (match[0].length !== 0) matches.push({ start: match.index, end: match.index + match[0].length });
      else expression.lastIndex += (text.codePointAt(expression.lastIndex) ?? 0) > 0xffff ? 2 : 1;
      if (matches.length >= limit) return { matches, truncated: true, error: null };
    }
    return { matches, truncated: false, error: null };
  } catch (reason) {
    return { matches: [], truncated: false, error: reason instanceof Error ? reason.message : "Invalid regular expression" };
  }
}
