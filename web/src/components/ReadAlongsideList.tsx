import { useCallback, useEffect, useState } from "react";

import { acquisitionStatusText } from "../lib/acquisition";
import { api } from "../lib/api";
import { basisLabel, relationLabel } from "../lib/readerOutput";
import type { AcquisitionStatus, ReadAlongside } from "../types";

const POLL_MILLISECONDS = 2000;

type Props = {
  items: ReadAlongside[];
  onOpenPaper: (paperId: string) => void;
};

/**
 * Works worth reading next to the paper. A work the library holds opens in the
 * reader; a known graph work can be fetched through the usual acquisition
 * flow and opens once it lands; anything else is shown as a citation only.
 */
export function ReadAlongsideList({ items, onOpenPaper }: Props) {
  if (items.length === 0) return null;
  return (
    <section className="read-alongside" aria-labelledby="read-alongside-heading">
      <header>
        <span className="eyebrow">Surrounding papers</span>
        <h2 id="read-alongside-heading">Read alongside</h2>
      </header>
      <ol>
        {items.map((item, index) => (
          <ReadAlongsideItem key={`${item.work_id ?? item.title}-${index}`} item={item} onOpenPaper={onOpenPaper} />
        ))}
      </ol>
    </section>
  );
}

function ReadAlongsideItem({ item, onOpenPaper }: { item: ReadAlongside; onOpenPaper: (paperId: string) => void }) {
  const [status, setStatus] = useState<AcquisitionStatus | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const running = status?.job.state === "running";
  const fetchedPaper = item.local_paper_id ?? status?.paper_id ?? null;
  const basis = basisLabel(item.basis);

  useEffect(() => {
    if (!running || item.work_id == null) return;
    const workId = item.work_id;
    const controller = new AbortController();
    const timer = window.setInterval(() => {
      api.workAcquisition(workId, controller.signal).then(setStatus).catch((reason: unknown) => {
        if (!controller.signal.aborted) setFailure(reason instanceof Error ? reason.message : "The fetch status could not be read.");
      });
    }, POLL_MILLISECONDS);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [item.work_id, running]);

  const startFetch = useCallback(() => {
    if (item.work_id == null) return;
    setFailure(null);
    const retry = status?.acquisition?.state === "unavailable";
    setStatus((current) => ({
      work_id: item.work_id ?? "",
      job: { state: "running" },
      acquisition: current?.acquisition ?? null,
      paper_id: current?.paper_id ?? null,
    }));
    api.fetchWork(item.work_id, retry ? { retry_unavailable: true } : undefined).catch((reason: unknown) => {
      setStatus(null);
      setFailure(reason instanceof Error ? reason.message : "The fetch could not start.");
    });
  }, [item.work_id, status]);

  const byline = [item.authors, item.year == null ? null : String(item.year)].filter((part) => part != null && part !== "").join(" · ");
  return (
    <li className="read-alongside-item" data-basis={item.basis} data-relation={item.relation}>
      <div className="read-alongside-head">
        <span className="read-alongside-relation">{relationLabel(item.relation)}</span>
        <strong>{item.title}</strong>
        {byline !== "" && <span className="read-alongside-byline">{byline}</span>}
      </div>
      <p>{item.why}</p>
      <div className="read-alongside-actions">
        <span className={`context-basis basis-${item.basis}`} title={basis.long}>{basis.short}</span>
        {fetchedPaper !== null ? (
          <button type="button" onClick={() => onOpenPaper(fetchedPaper)}>Open in reader</button>
        ) : item.work_id != null ? (
          <>
            <button type="button" disabled={running} onClick={startFetch}>
              {running ? "Fetching…" : status?.acquisition?.state === "unavailable" ? "Try fetching again" : "Fetch PDF"}
            </button>
            {status !== null && (
              <span className="read-alongside-status" role="status">
                {acquisitionStatusText(status.acquisition, status.job.state)}
              </span>
            )}
          </>
        ) : (
          <span className="read-alongside-status">Not in the citation graph</span>
        )}
        {failure !== null && <span className="read-alongside-status is-error" role="alert">{failure}</span>}
      </div>
    </li>
  );
}
