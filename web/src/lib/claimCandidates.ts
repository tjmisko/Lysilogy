import type { ClaimCandidate, WorkHit } from "../types";

/** A DOI in its comparable form: lowercase, without a resolver prefix. Null when empty. */
export function normalizeDoi(doi: string | null | undefined): string | null {
  if (doi === null || doi === undefined) return null;
  const bare = doi.trim().toLowerCase().replace(/^https?:\/\/(dx\.)?doi\.org\//u, "").replace(/^doi:\s*/u, "");
  return bare === "" ? null : bare;
}

/** A title in its comparable form: letters and digits only, case and accents folded. */
export function normalizeTitle(title: string): string {
  return title.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "");
}

/**
 * The library Work a provider candidate stands for, or null. Only hits that already are Works can
 * receive a claim; a matching DOI wins, else the same title (and the same year when both have one).
 */
export function matchCandidateWork(candidate: ClaimCandidate, hits: readonly WorkHit[]): string | null {
  const works = hits.filter((hit): hit is WorkHit & { work_id: string } => hit.work_id !== null);
  const doi = normalizeDoi(candidate.doi);
  if (doi !== null) {
    const byDoi = works.find((hit) => normalizeDoi(hit.doi) === doi);
    if (byDoi !== undefined) return byDoi.work_id;
  }
  const title = normalizeTitle(candidate.title);
  if (title === "") return null;
  const byTitle = works.find((hit) => normalizeTitle(hit.title) === title
    && (hit.year === null || candidate.year === null || hit.year === candidate.year));
  return byTitle?.work_id ?? null;
}

/** One line for a candidate: title, year and first author. */
export function candidateLabel(candidate: ClaimCandidate): string {
  const parts = [candidate.title];
  if (candidate.year !== null) parts.push(String(candidate.year));
  const first = candidate.authors[0];
  if (first !== undefined) parts.push(candidate.authors.length > 1 ? `${first} et al.` : first);
  return parts.join(" · ");
}

/** The next highlighted candidate when `c` is pressed again, wrapping around. */
export function nextCandidateIndex(current: number, count: number): number {
  if (count <= 0) return 0;
  return (current + 1) % count;
}
