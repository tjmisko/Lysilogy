import type { ClaimView, FieldProvenance, ProvenanceColumnKey } from "../types";

/** Source columns in trust order, highest first. */
export const PROVENANCE_COLUMN_ORDER: readonly ProvenanceColumnKey[] = [
  "user",
  "publisher",
  "paper",
  "openalex",
  "library_metadata",
  "citing_bibliography",
];

export type CellMark = "selected" | "agrees" | "disagrees" | "excluded";

/** One distinct value in a source cell, with every claim that printed it. */
export type CellValue = {
  display: string;
  mark: CellMark;
  count: number;
  claims: ClaimView[];
};

const MARK_ORDER: readonly CellMark[] = ["selected", "agrees", "disagrees", "excluded"];
const UNREADABLE_DISPLAY = "(unreadable)";

function groupMark(claims: ClaimView[]): CellMark {
  if (claims.some((claim) => claim.agreement === "selected")) return "selected";
  if (claims.every((claim) => claim.agreement === "excluded")) return "excluded";
  if (claims.some((claim) => claim.agreement === "agrees")) return "agrees";
  return "disagrees";
}

/**
 * The values one source column holds for a field, grouped by printed value.
 * Retracted overrides are history and never shown.
 */
export function cellValues(field: FieldProvenance, column: ProvenanceColumnKey): CellValue[] {
  const groups = new Map<string, ClaimView[]>();
  for (const claim of field.claims) {
    if (claim.origin !== column) continue;
    if (claim.origin === "user" && claim.retracted_at !== null) continue;
    const display = claim.display ?? UNREADABLE_DISPLAY;
    const group = groups.get(display);
    if (group === undefined) groups.set(display, [claim]);
    else group.push(claim);
  }
  const values: CellValue[] = [];
  for (const [display, claims] of groups) {
    values.push({ display, mark: groupMark(claims), count: claims.length, claims });
  }
  return values.sort(
    (left, right) =>
      MARK_ORDER.indexOf(left.mark) - MARK_ORDER.indexOf(right.mark) || right.count - left.count,
  );
}

/** The curator's live override for a field, if any. */
export function activeOverride(field: FieldProvenance): ClaimView | null {
  return (
    field.claims.find(
      (claim) => claim.origin === "user" && claim.retracted_at === null && claim.agreement !== "excluded",
    ) ?? null
  );
}

export type OverrideParse = { ok: true; value: string | number | string[] } | { ok: false; error: string };

const MAX_AUTHORS = 100;
const MAX_TEXT = 500;
const DOI_PREFIX = /^(?:https?:\/\/(?:dx\.)?doi\.org\/|doi:\s*)/i;

/** Turn what the curator typed into the JSON value the override endpoint expects. */
export function parseOverrideInput(field: FieldProvenance["field"], text: string): OverrideParse {
  const trimmed = text.trim();
  if (field === "authors") {
    const authors = text.split(/[\n;]/).map((author) => author.trim()).filter((author) => author !== "");
    if (authors.length === 0) return { ok: false, error: "Enter at least one author." };
    if (authors.length > MAX_AUTHORS) return { ok: false, error: `At most ${MAX_AUTHORS} authors.` };
    return { ok: true, value: authors };
  }
  if (field === "year") {
    if (!/^\d+$/.test(trimmed)) return { ok: false, error: "Year must be a whole number." };
    const year = Number(trimmed);
    if (year < 1400 || year > 2200) return { ok: false, error: "Year must be between 1400 and 2200." };
    return { ok: true, value: year };
  }
  if (field === "doi") {
    const doi = trimmed.replace(DOI_PREFIX, "").trim();
    if (!doi.startsWith("10.")) return { ok: false, error: "A DOI starts with 10." };
    return { ok: true, value: doi };
  }
  if (trimmed === "") return { ok: false, error: `Enter a ${field}.` };
  if (trimmed.length > MAX_TEXT) return { ok: false, error: `At most ${MAX_TEXT} characters.` };
  return { ok: true, value: trimmed };
}

/** "A, B, C +4": the first `limit` authors then how many were left out. */
export function formatAuthors(authors: readonly string[], limit = 6): string {
  if (authors.length <= limit) return authors.join(", ");
  return `${authors.slice(0, limit).join(", ")} +${authors.length - limit}`;
}

const FIELD_LABELS: Record<FieldProvenance["field"], string> = {
  title: "Title",
  year: "Year",
  authors: "Authors",
  venue: "Venue",
  doi: "DOI",
};

export function fieldLabel(field: FieldProvenance["field"]): string {
  return FIELD_LABELS[field];
}

/** The text an override editor starts with. */
export function overrideDraft(field: FieldProvenance["field"], display: string | null): string {
  if (display === null) return "";
  return field === "authors" ? display.split("; ").join("\n") : display;
}
