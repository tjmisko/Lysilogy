import { acquisitionStatusText, canFetch, isRetry } from "../lib/acquisition";
import { statusGlyph, statusOf } from "../lib/listViews";
import type { AcquisitionJobState, ListDetail, ListNodeView } from "../types";
import { WorkProvenance } from "./WorkProvenance";

/** The side panel `↵` opens on a Work that is not held (or on a claim): provenance, state, fetch. */
export function ListWorkPanel(props: {
  detail: ListDetail;
  node: ListNodeView;
  workTitles: Readonly<Record<string, string>>;
  fetchJob: AcquisitionJobState | "none";
  fetchMessage: string | null;
  onFetch: () => void;
  onOpenPaper: (paperId: string) => void;
  onClose: () => void;
}) {
  const { detail, node, workTitles, fetchJob, fetchMessage, onFetch, onOpenPaper, onClose } = props;
  const work = node.work;
  const workId = node.work_id;
  const papers = workId === null ? [] : (detail.local_papers[workId] ?? []);
  const view = workId === null ? null : (detail.acquisitions[workId] ?? null);
  const fetchable = workId !== null && papers.length === 0 && canFetch(workId, detail.local_papers, detail.acquisitions, fetchJob);
  const title = work?.display_title ?? node.claim?.title ?? node.label ?? "Untitled";
  const authors = work?.display_authors ?? node.claim?.authors ?? [];
  const year = work?.display_year ?? node.claim?.year ?? null;
  return (
    <aside className="list-panel" aria-label="Paper details" data-node-id={node.id}>
      <header>
        <h2>{title}</h2>
        <button type="button" aria-label="Close details" onClick={onClose}>×</button>
      </header>
      <p className="list-panel-line">
        {authors.length > 0 ? authors.slice(0, 4).join(", ") + (authors.length > 4 ? " et al." : "") : "Unknown author"}
        {year !== null && ` · ${year}`}
      </p>
      {workId !== null && <p className="list-panel-line" data-status={statusOf(detail, workId)}>{statusGlyph(statusOf(detail, workId))} {statusOf(detail, workId)}</p>}
      {node.kind === "claim" && <div className="list-panel-block">
        <p className="list-panel-line">Claim · {node.claim_state ?? "unverified"}</p>
        {node.claim?.raw != null && <p className="list-panel-line">As entered: {node.claim.raw}</p>}
        {node.candidates.length > 0 && <>
          <p className="list-panel-label">Possible matches</p>
          <ul>{node.candidates.map((candidate) => <li key={candidate.provider_id}>{candidate.title}{candidate.year === null ? "" : ` (${candidate.year})`}</li>)}</ul>
        </>}
      </div>}
      {workId !== null && <div className="list-panel-block">
        {papers.length > 0
          ? <button type="button" className="list-panel-action" onClick={() => { const first = papers[0]; if (first !== undefined) onOpenPaper(first.paper_id); }}>Open in the reader</button>
          : <>
            <p className="list-panel-line" data-fetch={fetchJob}>
              {acquisitionStatusText(view, fetchJob)}{fetchMessage === null ? "" : ` ${fetchMessage}`}
            </p>
            {fetchable && <button type="button" className="list-panel-action" onClick={onFetch}>{isRetry(view) ? "Retry fetch" : "Fetch"} <kbd>f</kbd></button>}
          </>}
      </div>}
      {node.path_steps.length > 0 && <div className="list-panel-block">
        <p className="list-panel-label">Reached through</p>
        <p className="list-panel-line">{node.path_steps.map((step) => step.title ?? "Untitled").join(" › ")}</p>
      </div>}
      <div className="list-panel-block">
        <p className="list-panel-label">Added</p>
        <p className="list-panel-line">{node.source}{node.review === "proposed" ? " · proposed" : ""}{node.added_at.slice(0, 10) === "" ? "" : ` · ${node.added_at.slice(0, 10)}`}</p>
        {node.rationale !== null && <p className="list-panel-line">{node.rationale}</p>}
        {node.note !== null && <p className="list-panel-line">Note: {node.note}</p>}
      </div>
      {node.context_links.length > 0 && <div className="list-panel-block">
        <p className="list-panel-label">Connects to papers you read</p>
        <ul>{node.context_links.map((link) => <li key={`${link.work_id}:${link.relation}`}>
          {workTitles[link.work_id] ?? "A paper you read"} · {link.relation.replace("_", " ")}{link.note === "" ? "" : ` — ${link.note}`}
        </li>)}</ul>
      </div>}
      {work !== null && <div className="list-panel-block"><WorkProvenance work={work} /></div>}
    </aside>
  );
}
