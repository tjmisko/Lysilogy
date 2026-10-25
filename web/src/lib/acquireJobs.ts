export type AcquireJob = { state: "running" | "failed"; message: string | null };

export type PollStep = {
  /** True when the server job has ended and the poll loop for this Work stops. */
  finished: boolean;
  /** The job to show next: still running, failed with the server message, or cleared (null). */
  job: AcquireJob | null;
};

/**
 * One poll result for a Work whose fetch was already accepted. Running keeps polling; failed shows
 * the message; anything else (idle, succeeded) clears the job so the reloaded detail speaks.
 */
export function jobAfterPoll(state: string, message: string | null | undefined): PollStep {
  if (state === "running") return { finished: false, job: { state: "running", message: null } };
  if (state === "failed") return { finished: true, job: { state: "failed", message: message ?? null } };
  return { finished: true, job: null };
}
