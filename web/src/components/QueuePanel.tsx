import { useEffect, useMemo, useRef, useState } from "react";

import type { AnalysisJob, AnalysisProvider, ProcessingQueue } from "../types";

type QueuePanelProps = {
  queue: ProcessingQueue;
  selectedPaperId: string | null;
  provider: AnalysisProvider;
  hasAnalysis: boolean;
  focusFeedback: boolean;
  onClose: () => void;
  onSelectPaper: (id: string) => void;
  onFeedback: (feedback: string) => Promise<void>;
};

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
}

function statusLabel(job: AnalysisJob): string {
  switch (job.status.state) {
    case "queued":
      return "Queued";
    case "running":
      return job.kind === "revision" ? "Revising" : `Running · ${job.status.stage}`;
    case "completed":
      return "Complete";
    case "failed":
      return `Failed · ${job.status.stage}`;
  }
}

export function QueuePanel({
  queue,
  selectedPaperId,
  provider,
  hasAnalysis,
  focusFeedback,
  onClose,
  onSelectPaper,
  onFeedback,
}: QueuePanelProps) {
  const jobs = useMemo(() => queue.jobs.slice(0, 16), [queue.jobs]);
  const [active, setActive] = useState(0);
  const [feedback, setFeedback] = useState("");
  const [sending, setSending] = useState(false);
  const feedbackRef = useRef<HTMLTextAreaElement>(null);
  const activeIndex = Math.min(active, Math.max(0, jobs.length - 1));
  const activeCount = queue.jobs.filter((job) =>
    job.status.state === "queued" || job.status.state === "running",
  ).length;
  const currentJob = useMemo(
    () => queue.jobs.find((job) => job.paper_id === selectedPaperId) ?? null,
    [queue.jobs, selectedPaperId],
  );

  useEffect(() => {
    if (focusFeedback) feedbackRef.current?.focus();
  }, [focusFeedback]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape" || (event.key === "q" && !isEditable(event.target))) {
        event.preventDefault();
        onClose();
        return;
      }
      if (isEditable(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "j" || event.key === "ArrowDown") {
        event.preventDefault();
        setActive((index) => Math.min(Math.max(0, jobs.length - 1), index + 1));
      } else if (event.key === "k" || event.key === "ArrowUp") {
        event.preventDefault();
        setActive((index) => Math.max(0, index - 1));
      } else if (event.key === "Enter" || event.key === "o") {
        const job = jobs[activeIndex];
        if (job !== undefined) {
          event.preventDefault();
          onSelectPaper(job.paper_id);
        }
      } else if (event.key === "f") {
        event.preventDefault();
        feedbackRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeIndex, jobs, onClose, onSelectPaper]);

  const submitFeedback = async (): Promise<void> => {
    if (feedback.trim().length === 0 || sending) return;
    setSending(true);
    try {
      await onFeedback(feedback.trim());
      setFeedback("");
    } catch {
      // The application-level toast carries the backend error; retain the draft.
    } finally {
      setSending(false);
    }
  };

  const feedbackDisabled =
    selectedPaperId === null || !hasAnalysis || provider === "heuristic" ||
    currentJob?.status.state === "queued" || currentJob?.status.state === "running";

  return (
    <div className="queue-backdrop" role="presentation" onMouseDown={onClose}>
      <aside
        className="queue-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="queue-heading"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <span className="eyebrow">Live tasklists</span>
            <h2 id="queue-heading">Processing queue</h2>
            <p>{activeCount === 0 ? "No active readers" : `${activeCount} active reader${activeCount === 1 ? "" : "s"}`}</p>
          </div>
          <button className="icon-button" type="button" onClick={onClose} aria-label="Close queue">×</button>
        </header>

        <div className="queue-list" role="listbox" aria-label="Analysis jobs">
          {jobs.map((job, index) => (
            <article
              key={`${job.paper_id}:${job.created_at}`}
              role="option"
              tabIndex={index === activeIndex ? 0 : -1}
              aria-selected={index === activeIndex}
              className={`queue-job ${index === activeIndex ? "is-active" : ""}`}
              onFocus={() => setActive(index)}
              onClick={() => onSelectPaper(job.paper_id)}
            >
              <div className="queue-job-heading">
                <strong>{job.paper_title}</strong>
                <span>{job.progress}%</span>
              </div>
              <div className="queue-progress" aria-label={`${job.progress}% complete`}>
                <i style={{ width: `${job.progress}%` }} />
              </div>
              <div className="queue-job-meta">
                <span>{statusLabel(job)}</span>
                <span>{job.provider}{job.resumable ? " · resumable" : ""}</span>
              </div>
              {job.feedback !== null && (
                <p className="queue-request">“{job.feedback}”</p>
              )}
              <ol className="queue-tasks">
                {job.tasks.map((task) => (
                  <li key={task.id} className={`task-${task.status}`}>
                    <i aria-hidden="true" />
                    <span>{task.label}{task.detail === null ? "" : ` — ${task.detail}`}</span>
                  </li>
                ))}
              </ol>
              {job.status.state === "failed" && <p className="queue-error">{job.status.message}</p>}
            </article>
          ))}
          {jobs.length === 0 && (
            <div className="queue-empty">
              <span>✓</span>
              <strong>The queue is clear.</strong>
              <p>Run <kbd>:analyze</kbd> to map the current paper.</p>
            </div>
          )}
        </div>

        <form
          className="feedback-form"
          onSubmit={(event) => {
            event.preventDefault();
            void submitFeedback();
          }}
        >
          <div>
            <span className="eyebrow">Retry with context</span>
            <strong>Feedback for the current atlas</strong>
          </div>
          <textarea
            ref={feedbackRef}
            value={feedback}
            rows={4}
            maxLength={8_000}
            disabled={feedbackDisabled}
            placeholder={
              provider === "heuristic"
                ? "Choose Codex or Claude to revise from feedback."
                : hasAnalysis
                  ? "What should the reader reconsider, explain, or correct?"
                  : "Analyze this paper before requesting a revision."
            }
            onChange={(event) => setFeedback(event.target.value)}
            onKeyDown={(event) => {
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                event.preventDefault();
                void submitFeedback();
              }
            }}
          />
          <footer>
            <span>
              {currentJob?.resumable === true && currentJob.provider === provider
                ? "Will resume the saved agent session."
                : "Will use the complete saved source and atlas state."}
            </span>
            <button type="submit" disabled={feedbackDisabled || sending || feedback.trim().length === 0}>
              {sending ? "Queuing…" : "Send & retry"}
            </button>
          </footer>
        </form>
        <div className="queue-keyline"><kbd>j k / ↑ ↓</kbd> move · <kbd>f</kbd> feedback · <kbd>q</kbd> close</div>
      </aside>
    </div>
  );
}
