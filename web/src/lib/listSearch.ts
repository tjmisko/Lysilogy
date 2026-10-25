// Fuzzy list search for the picker, and turning search hits or typed text into node adds.

import type { ClaimInput, ListNodeAdd, WorkHit } from "../types";

/** PaperSwitcher's scorer: higher is better, null is no match. */
export function fuzzyScore(query: string, candidate: string): number | null {
  const needle = query.toLocaleLowerCase().replaceAll(/\s+/gu, "");
  const haystack = candidate.toLocaleLowerCase();
  if (needle.length === 0) return 0;
  let score = 0;
  let cursor = -1;
  for (const character of needle) {
    const next = haystack.indexOf(character, cursor + 1);
    if (next < 0) return null;
    const boundary = next === 0 || /[\s\-—/]/u.test(haystack[next - 1] ?? "");
    score += boundary ? 8 : 2;
    score += next === cursor + 1 ? 5 : Math.max(-8, cursor - next + 1);
    cursor = next;
  }
  return score - haystack.length / 200;
}

/** Items whose title matches, best first. A blank query keeps the given order. */
export function rankByTitle<T extends { title: string }>(items: readonly T[], query: string): T[] {
  if (query.trim() === "") return [...items];
  const scored = items.flatMap((item, index) => {
    const score = fuzzyScore(query, item.title);
    return score === null ? [] : [{ item, score, index }];
  });
  return scored.sort((left, right) => right.score - left.score || left.index - right.index).map((entry) => entry.item);
}

const DOI = /^(?:https?:\/\/(?:dx\.)?doi\.org\/|doi:\s*)?(10\.\d{4,9}\/\S+)$/iu;
const ARXIV = /^(?:arxiv:\s*|https?:\/\/arxiv\.org\/abs\/)?(\d{4}\.\d{4,5})(?:v\d+)?$/iu;
const TRAILING_YEAR = /^(.*\S)\s*\((\d{4})\)$/su;
const TITLE_LIMIT = 500;

const clip = (text: string): string => Array.from(text).slice(0, TITLE_LIMIT).join("");

/**
 * A claim for text typed into the add dialog; the server verifies it before admitting
 * anything. A bare DOI or arXiv id becomes an identifier claim, `Title (2017)` carries its year.
 * Blank text gives null.
 */
export function claimFromText(text: string): ListNodeAdd | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const doi = DOI.exec(trimmed)?.[1];
  if (doi !== undefined) return { claim: { title: trimmed, doi, raw: trimmed } };
  const arxiv = ARXIV.exec(trimmed)?.[1];
  if (arxiv !== undefined) return { claim: { title: trimmed, arxiv, raw: trimmed } };
  const dated = TRAILING_YEAR.exec(trimmed);
  const claim: ClaimInput = dated === null
    ? { title: clip(trimmed), raw: trimmed }
    : { title: clip(dated[1] ?? trimmed), year: Number(dated[2]), raw: trimmed };
  return { claim };
}

/**
 * What adding a search hit sends: a Work already in the graph by id, a held paper by its
 * paper id, and a provider hit as a claim (the server verifies it against the provider).
 */
export function hitToAdd(hit: WorkHit): ListNodeAdd {
  if (hit.work_id !== null) return { work_id: hit.work_id };
  if (hit.paper_id !== null) return { paper_id: hit.paper_id };
  const claim: ClaimInput = {
    title: hit.title,
    authors: hit.authors,
    year: hit.year,
    doi: hit.doi,
  };
  return { claim };
}

/** Distinct hits: the same Work found by library and graph search shows once. */
export function dedupeHits(hits: readonly WorkHit[]): WorkHit[] {
  const seen = new Set<string>();
  const kept: WorkHit[] = [];
  for (const hit of hits) {
    const key = hit.work_id ?? hit.paper_id ?? hit.openalex_id ?? hit.doi ?? `${hit.title.toLowerCase()}\u0000${hit.year ?? ""}`;
    if (seen.has(key)) continue;
    seen.add(key);
    kept.push(hit);
  }
  return kept;
}
