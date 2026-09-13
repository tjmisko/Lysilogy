import { selectionSpans, tokensInSpan, type ReadingIndex, type TextSpan } from "./readingIndex.ts";
import type { TextRect } from "../types";
import { objectsMatchGeneration, type ObjectsArtifact } from "./objects.ts";

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
export function discoverPaperLinks(index: ReadingIndex, artifact: ObjectsArtifact | null = null, generation: string | null = null): PaperLink[] {
  const entries = artifact !== null && objectsMatchGeneration(artifact, generation) ? artifact.objects.filter((object) => object.kind === "bib_entry") : [];
  const blocks = index.objects.paragraph.flatMap((paragraph) => selectionSpans(paragraph).map((span) => ({ ...span, kind: paragraph.kind })))
    .sort((a, b) => a.start - b.start);
  // Keep the existing figure/table exclusion usable when objects fail or are
  // stale. This masks one section; entry splitting and citation resolution are
  // exclusively backend operations.
  const heading = blocks.find((block) => /^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:references(?: and notes| cited)?|bibliography|literature cited|works cited)\s*[:.]?$/iu.test(index.text.slice(block.start, block.end).trim()));
  const bibliographyStart = heading?.end ?? Infinity;
  const bibliographyEnd = blocks.find((block) => block.start > bibliographyStart && block.kind === "heading"
    && /^(?:(?:\d+|[IVXLCDM]+)[.\s]+)?(?:appendix|appendices|supplement|acknowledg)/iu.test(index.text.slice(block.start, block.end).trim()))?.start ?? index.text.length;
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
  // Bibliography detection and marker resolution are backend source facts.
  // The frontend only projects verified occurrence geometry and destinations.
  for (const entry of entries) {
    const dest = destination(index, entry.member_anchors[0] ?? entry.anchor, entry.label);
    if (dest === null) continue;
    for (const mention of entry.mentions) {
      const span = mention.anchor;
      if (!Number.isInteger(span.start) || !Number.isInteger(span.end) || span.start < 0 || span.end <= span.start || span.end > index.text.length || mention.rects.length === 0) continue;
      links.push({ id: `reference:${entry.id}:${span.start}:${span.end}:${span.page}`,
        kind: "reference", label: index.text.slice(span.start, span.end), page: span.page,
        rects: mention.rects, span, destination: dest });
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
