import { selectionSpans, tokensInSpan, type ReadingIndex, type TextSpan } from "./readingIndex.ts";
import type { TextRect } from "../types";

export type PaperDestination = { page: number; rect: TextRect; label: string };
export type PaperLink = {
  id: string;
  kind: "reference" | "figure" | "table" | "link";
  label: string;
  page: number;
  rects: TextRect[];
  span?: TextSpan;
  destination: PaperDestination | { url: string };
};

type Target = TextSpan & { kind: PaperLink["kind"]; key: string; label: string; destination: PaperDestination };
const identifier = String.raw`(?:[A-Z]\.?\d+(?:\.\d+)*(?:[a-z]|\([a-z]\))?|\d+(?:\.\d+)*(?:[a-z]|\([a-z]\))?|[IVXLCDM]+)`;
const captionPattern = String.raw`\b((?:Supplementary\s+)?(?:Figures?|Figs?\.?|Tables?|Tabs?\.?))\s*(${identifier})(?![\p{L}\d])`;
const normalize = (text: string) => text.normalize("NFKC").toLocaleLowerCase().replace(/[\s()]/gu, "").replace(/^([a-z])\.(?=\d)/u, "$1");
const displayKey = (prefix: string, label: string) => normalize(`${/supplementary/iu.test(prefix) && !/^s/iu.test(label) ? "S" : ""}${label}`);
const years = (text: string) => Array.from(text.matchAll(/\b(?:18|19|20)\d{2}[a-z]?\b/gu), (match) => match[0]);

function destination(index: ReadingIndex, span: TextSpan, label: string): PaperDestination | null {
  const tokens = tokensInSpan(index, span);
  const first = tokens[0];
  if (first === undefined || first.rects.length === 0) return null;
  const rects = tokens.filter((token) => token.page === first.page).flatMap((token) => token.rects);
  return { page: first.page, label, rect: {
    x_min: Math.min(...rects.map((rect) => rect.x_min)), y_min: Math.min(...rects.map((rect) => rect.y_min)),
    x_max: Math.max(...rects.map((rect) => rect.x_max)), y_max: Math.max(...rects.map((rect) => rect.y_max)),
  } };
}

/** Only resolve identifiers to destinations actually present in this paper. */
export function discoverPaperLinks(index: ReadingIndex): PaperLink[] {
  const blocks = index.objects.paragraph.flatMap((paragraph) => selectionSpans(paragraph).map((span) => ({ ...span, kind: paragraph.kind })))
    .sort((a, b) => a.start - b.start);
  const targets: Target[] = [];
  const addTarget = (span: TextSpan, kind: Target["kind"], key: string, label: string) => {
    if (targets.some((target) => target.start === span.start && target.key === key && target.kind === kind)) return;
    const dest = destination(index, span, label);
    if (dest !== null) targets.push({ ...span, kind, key, label, destination: dest });
  };
  for (const block of blocks) {
    const text = index.text.slice(block.start, block.end);
    const match = new RegExp(`^\\s*${captionPattern}`, "iu").exec(text);
    // Caption classification prevents prose such as "Figure 2 shows…" becoming a destination.
    if (match !== null && (block.kind === "caption" || /^\s*[:.](?:\s|$)/u.test(text.slice(match[0].length))
      || text.trim() === match[0].trim())) {
      const kind = /tab/iu.test(match[1] ?? "") ? "table" : "figure";
      const key = displayKey(match[1] ?? "", match[2] ?? "");
      addTarget(block, kind, key, text.trim());
    }
  }
  for (const figure of index.figures) {
    const match = new RegExp(captionPattern, "iu").exec(figure.label);
    if (match !== null) addTarget(figure, /tab/iu.test(match[1] ?? "") ? "table" : "figure", displayKey(match[1] ?? "", match[2] ?? ""), figure.caption);
  }

  // Bibliography boundaries and printed entry labels establish the citation convention.
  const bibliography = blocks.find((block) => /^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:references(?: and notes| cited)?|bibliography|literature cited|works cited)\s*[:.]?$/iu.test(index.text.slice(block.start, block.end).trim()));
  const bibliographyStart = bibliography?.end ?? Infinity;
  const bibliographyEnd = blocks.find((block) => block.start > bibliographyStart && block.kind === "heading"
    && /^(?:appendix|supplement|acknowledg)/iu.test(index.text.slice(block.start, block.end).trim()))?.start ?? index.text.length;
  const entries = blocks.filter((block) => block.start >= bibliographyStart && block.start < bibliographyEnd && block.kind !== "heading");
  const entryPattern = /(?:^|\n)\s*(\[([\p{L}\d][\p{L}\d+,:.\-–]{0,30})\]|\((\d{1,4})\)|(\d{1,4})[.)])\s+/gu;
  for (const block of entries) {
    const text = index.text.slice(block.start, block.end);
    const labels = Array.from(text.matchAll(entryPattern));
    if (labels.length === 0) {
      addTarget(block, "reference", "", text.trim());
    } else for (let at = 0; at < labels.length; at++) {
      const match = labels[at];
      if (match === undefined) continue;
      const span = { start: block.start + match.index + match[0].length - match[0].trimStart().length,
        end: block.start + (labels[at + 1]?.index ?? text.length) };
      addTarget(span, "reference", normalize(match[2] ?? match[3] ?? match[4] ?? ""), index.text.slice(span.start, span.end).trim());
    }
  }
  const links: PaperLink[] = [];
  const add = (span: TextSpan, target: Target) => {
    if (span.start >= target.start && span.start < target.end || span.start >= bibliographyStart && span.start < bibliographyEnd) return;
    const label = index.text.slice(span.start, span.end).trim();
    for (const page of new Set(tokensInSpan(index, span).map((token) => token.page))) {
      const id = `${span.start}:${span.end}:${target.start}:${page}`;
      if (links.some((link) => link.id === id)) continue;
      const rects = tokensInSpan(index, span).filter((token) => token.page === page).flatMap((token) => token.rects);
      if (rects.length) links.push({ id, span, kind: target.kind, label, page, rects, destination: target.destination });
    }
  };
  const resolve = (kind: Target["kind"], key: string, span: TextSpan) => {
    let matches = targets.filter((target) => target.kind === kind && target.key === normalize(key));
    if (!matches.length && kind !== "reference" && /\d[a-z]$/u.test(normalize(key))) matches = targets.filter((target) => target.kind === kind && target.key === normalize(key).replace(/[a-z]$/u, ""));
    // Duplicate labels are ambiguous; an embedded PDF link can still resolve them.
    if (matches.length === 1 && matches[0] !== undefined) add(span, matches[0]);
  };
  // Appendices can follow the bibliography; their citations still belong to the paper.
  const body = index.text;
  for (const match of body.matchAll(new RegExp(captionPattern, "giu"))) {
    const kind = /tab/iu.test(match[1] ?? "") ? "table" : "figure";
    const supplementary = /supplementary/iu.test(match[1] ?? "");
    const key = displayKey(match[1] ?? "", match[2] ?? "");
    resolve(kind, key, { start: match.index, end: match.index + match[0].length });
    const tail = body.slice(match.index + match[0].length);
    const list = new RegExp(`^\\s*(?:[,;&]|and|[-–−])\\s*(${identifier})(?![\\p{L}\\d])`, "iu");
    let consumed = 0, previous = key;
    for (let count = 0; count < 30; count++) {
      const next = list.exec(tail.slice(consumed));
      if (next === null) break;
      const nextKey = `${supplementary && !/^S/iu.test(next[1] ?? "") ? "S" : ""}${next[1] ?? ""}`;
      const span = { start: match.index, end: match.index + match[0].length + consumed + next[0].length };
      const range = /^\s*[-–−]/u.test(next[0]);
      for (const item of range ? expandRange(previous, nextKey) : [nextKey]) resolve(kind, item, span);
      previous = nextKey; consumed += next[0].length;
    }
  }
  for (const match of body.matchAll(/\[([^\]\n]{1,120})\]|\((\d{1,4}(?:\s*[,;–−-]\s*\d{1,4})*)\)/gu)) {
    if (/(?:\b(?:eq(?:uation)?s?|sec(?:tion)?s?|fig(?:ure)?s?|tables?|theorems?|lemmas?|propositions?)\.?)\s*$/iu.test(body.slice(Math.max(0, match.index - 30), match.index))) continue;
    const span = { start: match.index, end: match.index + match[0].length };
    for (const key of (match[1] ?? match[2] ?? "").split(/\s*[,;]\s*/u)) {
      const range = /^(\d+)\s*[-–−]\s*(\d+)$/u.exec(key.trim());
      for (const item of range === null ? [key] : [range[1] ?? "", ...expandRange(range[1] ?? "", range[2] ?? "")]) resolve("reference", item, span);
    }
  }
  // Superscript citations require raised geometry; plain prose numbers are never citations.
  for (let at = 1; at < index.tokens.length; at++) {
    const token = index.tokens[at], before = index.tokens[at - 1];
    if (token === undefined || before === undefined) continue;
    if (token.start >= bibliographyStart && token.start < bibliographyEnd) continue;
    if (!/^\d{1,4}(?:[,–-]\d{1,4})*$/u.test(token.text) || token.page !== before.page) continue;
    const a = before.rects.at(-1), b = token.rects[0];
    if (a === undefined || b === undefined || b.y_max >= a.y_max - (a.y_max - a.y_min) * .2
      || b.y_min > a.y_min || b.y_max < a.y_min || b.x_min < a.x_max - 2 || b.x_min - a.x_max > 6) continue;
    for (const part of token.text.split(",")) {
      const range = /^(\d+)[–-](\d+)$/u.exec(part);
      for (const key of range === null ? [part] : [range[1] ?? "", ...expandRange(range[1] ?? "", range[2] ?? "")]) resolve("reference", key, token);
    }
  }
  // Author–year variants are learned from bibliography surnames and actual years.
  const authorTargets = new Map<string, Target[]>();
  for (const target of targets.filter((item) => item.kind === "reference")) {
    const entry = target.label.replace(/^(?:\[[^\]]+\]|\(\d+\)|\d+[.)])\s*/u, "");
    const author = /^(?:[A-Z]\.\s*)*([\p{L}][\p{L}\p{M}'’−-]+)/u.exec(entry)?.[1];
    if (author === undefined) continue;
    // Publication year is near the author block; later years may be in the title or URL.
    const year = years(entry.slice(0, 250))[0];
    if (year === undefined) continue;
    const key = `${author.toLocaleLowerCase()}:${year}`;
    authorTargets.set(key, [...authorTargets.get(key) ?? [], target]);
  }
  for (const match of body.matchAll(/([\p{L}][\p{L}\p{M}'’−-]+)(?:\s+(?:et\s+al\.?|(?:and|&)\s+[\p{L}][\p{L}\p{M}'’−-]+))?\s*[,([]?\s*((?:18|19|20)\d{2}[a-z]?(?:\s*[,;]\s*(?:(?:18|19|20)\d{2})?[a-z]?)*)\b/gu)) {
    const citedYears = years(match[2] ?? "");
    // APA's "2020a,b" inherits the year for the second suffix.
    const firstYear = citedYears[0]?.slice(0, 4);
    if (firstYear !== undefined) for (const suffix of (match[2] ?? "").matchAll(/[,;]\s*([a-z])\b/gu)) citedYears.push(firstYear + (suffix[1] ?? ""));
    for (const year of citedYears) {
      const matches = authorTargets.get(`${(match[1] ?? "").toLocaleLowerCase()}:${year}`) ?? [];
      const end = match.index + match[0].length;
      if (matches.length === 1 && matches[0] !== undefined) add({ start: match.index, end: end + (/[[(]/u.test(match[0]) && /[\])]/u.test(body[end] ?? "") ? 1 : 0) }, matches[0]);
    }
  }
  return links.sort((a, b) => a.page - b.page || (a.span?.start ?? 0) - (b.span?.start ?? 0));
}

function expandRange(from: string, to: string): string[] {
  const a = /^([a-z]*)(\d+)$/iu.exec(from), b = /^([a-z]*)(\d+)$/iu.exec(to);
  if (a === null || b === null || b[1] && normalize(a[1] ?? "") !== normalize(b[1])) return [to];
  const start = Number(a[2]), end = Number(b[2]);
  if (end <= start || end - start > 30) return [to];
  return Array.from({ length: end - start }, (_, at) => `${a[1]}${start + at + 1}`);
}

/** Equal-length home-row codes are prefix-free, including on dense reference pages. */
export function hintCodes(count: number): string[] {
  const alphabet = "asdfghjkl";
  const width = Math.max(1, Math.ceil(Math.log(Math.max(1, count)) / Math.log(alphabet.length)));
  return Array.from({ length: count }, (_, at) => {
    let code = "";
    for (let digit = 0; digit < width; digit++) { code = (alphabet[at % alphabet.length] ?? "a") + code; at = Math.floor(at / alphabet.length); }
    return code;
  });
}
