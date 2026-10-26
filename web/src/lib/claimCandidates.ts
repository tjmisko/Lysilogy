import type { ClaimCandidate } from "../types";

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
