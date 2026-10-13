import type { AcquisitionJobState } from "../types";

/** The part of an acquisition record the status line needs. */
export type AcquisitionLike = {
  state: string;
  unavailable_reason?: string | null;
  checked_at?: string | null;
};

const REASON_TEXT: Readonly<Record<string, string>> = {
  no_identifier: "the work has no DOI, arXiv ID, or OpenAlex ID to look up",
  no_oa_location: "no open-access copy was found",
  all_links_failed: "open-access links were found but none could be downloaded",
};

const GENERIC_REASON = "no open-access copy could be fetched";

/** The reason an acquisition is unavailable, in words. Unknown reasons get a generic text. */
export function unavailableReasonText(reason: string | null | undefined): string {
  if (reason === null || reason === undefined) return GENERIC_REASON;
  return Object.hasOwn(REASON_TEXT, reason) ? (REASON_TEXT[reason] ?? GENERIC_REASON) : GENERIC_REASON;
}

function checkedDate(checkedAt: string | null | undefined): string | null {
  if (typeof checkedAt !== "string") return null;
  const match = /^\d{4}-\d{2}-\d{2}/u.exec(checkedAt);
  return match === null ? null : match[0];
}

function runningText(state: string | undefined): string {
  switch (state) {
    case "identifier_found": return "Looking for an open-access copy…";
    case "link_found": return "Downloading…";
    case "downloaded": return "Saving to the library…";
    case "mapped": return "Fetched.";
    default: return "Resolving identifiers…";
  }
}

/** One line describing where a Work's acquisition stands. */
export function acquisitionStatusText(
  view: AcquisitionLike | null | undefined,
  jobState: AcquisitionJobState | "none" = "none",
): string {
  if (jobState === "running") return runningText(view?.state);
  if (view === null || view === undefined) {
    return jobState === "failed" ? "The fetch failed." : "Not fetched yet.";
  }
  switch (view.state) {
    case "unresolved": return jobState === "failed" ? "The fetch failed." : "Not fetched yet.";
    case "identifier_found":
    case "link_found":
    case "downloaded": return "The fetch did not finish. Try again.";
    case "mapped": return "Fetched.";
    case "unavailable": {
      const date = checkedDate(view.checked_at);
      const reason = unavailableReasonText(view.unavailable_reason);
      return `Unavailable: ${reason}${date === null ? "" : ` (checked ${date})`}.`;
    }
    default: return "The fetch status is unknown.";
  }
}

/**
 * Whether a fetch can start for the Work: it has no local paper and no fetch is
 * running. Unavailable Works can be fetched again as a retry.
 */
export function canFetch(
  workId: string,
  localPapers: Readonly<Record<string, readonly unknown[] | undefined>>,
  acquisitions: Readonly<Record<string, AcquisitionLike | undefined>>,
  jobState: AcquisitionJobState | "none" = "none",
): boolean {
  if (jobState === "running") return false;
  if ((localPapers[workId]?.length ?? 0) > 0) return false;
  return acquisitions[workId]?.state !== "running";
}

/** Whether a fetch of the Work would be a retry of an unavailable verdict. */
export function isRetry(view: AcquisitionLike | null | undefined): boolean {
  return view?.state === "unavailable";
}
