import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { ApiError, api } from "../lib/api";
import { LITERATURE_QUIT_EVENT } from "../lib/literatureEvents";
import { layoutFanout, type FanoutPlacement } from "../lib/literatureLayout";
import type { GraphNeighbors, LocalPaper, Work, WorkCitations } from "../types";
import "./LiteratureGraph.css";

type LiteratureGraphProps = {
  paperId: string;
  paperTitle: string;
  onClose: () => void;
  /** Close the overlay and open a local paper. */
  onOpenPaper: (paperId: string) => void;
};

type Phase =
  | { kind: "loading" }
  | { kind: "unavailable" }
  | { kind: "error"; message: string }
  | { kind: "nowork" }
  | { kind: "running" }
  | { kind: "graph"; work: Work; neighbors: GraphNeighbors };

type CitationsState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; citations: WorkCitations };

const CANVAS = { width: 1000, height: 700 } as const;
const POLL_MILLISECONDS = 1000;
const LABEL_LIMIT = 30;
const FULL_LABEL_LIMIT = 90;

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement;
}

function truncate(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit - 1).trimEnd()}…`;
}

function workTitle(work: Work): string {
  const title = work.display_title?.trim() ?? "";
  return title === "" ? `Untitled (${work.id.slice(0, 8)}…)` : title;
}

function workLabel(work: Work, limit: number): string {
  const title = truncate(workTitle(work), limit);
  return work.display_year === null ? title : `${title} · ${work.display_year}`;
}

function failureMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "The literature request failed.";
}

function isUnavailable(reason: unknown): boolean {
  return reason instanceof ApiError && reason.status === 503;
}

export function LiteratureGraph({ paperId, paperTitle, onClose, onOpenPaper }: LiteratureGraphProps) {
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const [reloadToken, setReloadToken] = useState(0);
  const [selected, setSelected] = useState(0);
  const [workPanelOpen, setWorkPanelOpen] = useState(false);
  const [citations, setCitations] = useState<CitationsState>({ kind: "loading" });
  const rootRef = useRef<HTMLDivElement>(null);

  // Load the paper's Work and its references. Aborted on unmount or reload.
  useEffect(() => {
    const controller = new AbortController();
    void (async () => {
      try {
        const link = await api.paperWork(paperId, controller.signal);
        if (link.work === null) {
          setPhase({ kind: "nowork" });
          return;
        }
        const neighbors = await api.graphNeighbors(link.work.id, "references", controller.signal);
        setPhase({ kind: "graph", work: link.work, neighbors });
      } catch (reason: unknown) {
        if (controller.signal.aborted) return;
        setPhase(isUnavailable(reason)
          ? { kind: "unavailable" }
          : { kind: "error", message: failureMessage(reason) });
      }
    })();
    return () => controller.abort();
  }, [paperId, reloadToken]);

  // Poll the lit job once a second while it runs. The interval dies with the overlay.
  const running = phase.kind === "running";
  useEffect(() => {
    if (!running) return;
    const controller = new AbortController();
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const job = await api.literatureJob(paperId, controller.signal);
          if (controller.signal.aborted || job.state === "running") return;
          window.clearInterval(timer);
          if (job.state === "failed") {
            setPhase({ kind: "error", message: job.message ?? "The literature job failed." });
          } else if (job.state === "idle") {
            setPhase({ kind: "error", message: "The literature job is no longer running. Try again." });
          } else if (job.summary?.status.kind === "unmatched") {
            setPhase({ kind: "error", message: `No matching Work found: ${job.summary.status.reason}` });
          } else {
            setSelected(0);
            setPhase({ kind: "loading" });
            setReloadToken((token) => token + 1);
          }
        } catch (reason: unknown) {
          if (controller.signal.aborted) return;
          window.clearInterval(timer);
          setPhase({ kind: "error", message: failureMessage(reason) });
        }
      })();
    }, POLL_MILLISECONDS);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [paperId, running]);

  const runLiterature = useCallback((): void => {
    setPhase({ kind: "running" });
    api.startLiterature(paperId).catch((reason: unknown) => {
      setPhase(isUnavailable(reason)
        ? { kind: "unavailable" }
        : { kind: "error", message: failureMessage(reason) });
    });
  }, [paperId]);

  const graph = phase.kind === "graph" ? phase : null;
  const placements = useMemo<FanoutPlacement[]>(() => {
    if (graph === null) return [];
    return layoutFanout(
      graph.work.id,
      graph.neighbors.nodes.map((node) => ({ id: node.id, title: node.display_title, year: node.display_year })),
      CANVAS,
    );
  }, [graph]);
  const worksById = useMemo(() => {
    const map = new Map<string, Work>();
    if (graph === null) return map;
    map.set(graph.work.id, graph.work);
    for (const node of graph.neighbors.nodes) map.set(node.id, node);
    return map;
  }, [graph]);
  const activeIndex = Math.min(selected, Math.max(0, placements.length - 1));
  const activePlacement = placements[activeIndex];
  const activeWork = activePlacement === undefined ? undefined : worksById.get(activePlacement.id);
  const activeWorkId = workPanelOpen ? activeWork?.id : undefined;

  // Fetch the selected Work's citation neighborhood while its panel is open.
  useEffect(() => {
    if (activeWorkId === undefined) return;
    const controller = new AbortController();
    api.workCitations(activeWorkId, controller.signal)
      .then((loaded) => setCitations({ kind: "ready", citations: loaded }))
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) setCitations({ kind: "error", message: failureMessage(reason) });
      });
    return () => controller.abort();
  }, [activeWorkId]);

  const openWorkPanel = useCallback((index: number): void => {
    setSelected(index);
    setCitations({ kind: "loading" });
    setWorkPanelOpen(true);
  }, []);

  // Esc and q reach the App close chain first. It asks us to close the Work panel
  // before it closes the overlay, and cancels the event when we did.
  useEffect(() => {
    const root = rootRef.current;
    if (root === null) return;
    const onQuit = (event: Event): void => {
      if (!workPanelOpen) return;
      event.preventDefault();
      setWorkPanelOpen(false);
    };
    root.addEventListener(LITERATURE_QUIT_EVENT, onQuit);
    return () => root.removeEventListener(LITERATURE_QUIT_EVENT, onQuit);
  }, [workPanelOpen]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || isEditable(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "r" && (phase.kind === "nowork" || phase.kind === "error")) {
        event.preventDefault();
        runLiterature();
        return;
      }
      if (phase.kind !== "graph" || workPanelOpen) return;
      if (event.key === "j" || event.key === "ArrowDown" || event.key === "l" || event.key === "ArrowRight") {
        event.preventDefault();
        setSelected((index) => (Math.min(index, placements.length - 1) + 1) % placements.length);
      } else if (event.key === "k" || event.key === "ArrowUp" || event.key === "h" || event.key === "ArrowLeft") {
        event.preventDefault();
        setSelected((index) => (Math.min(index, placements.length - 1) + placements.length - 1) % placements.length);
      } else if (event.key === "Enter" || event.key === "o") {
        event.preventDefault();
        openWorkPanel(activeIndex);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeIndex, openWorkPanel, phase.kind, placements.length, runLiterature, workPanelOpen]);

  const localIds = useMemo(() => {
    if (graph === null) return new Set<string>();
    return new Set(
      Object.entries(graph.neighbors.local_papers)
        .filter(([, papers]) => papers.length > 0)
        .map(([workId]) => workId),
    );
  }, [graph]);

  return (
    <div className="literature-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div
        ref={rootRef}
        className="literature-graph"
        role="dialog"
        aria-label="Literature graph"
        data-work-open={workPanelOpen ? "true" : "false"}
        data-phase={phase.kind}
      >
        <header>
          <div>
            <span className="literature-kicker">Literature · what this paper cites</span>
            <h2>{truncate(paperTitle, 110)}</h2>
          </div>
          <button type="button" aria-label="Close literature graph (Esc)" onClick={onClose}>×</button>
        </header>

        <div className="literature-body">
          {phase.kind === "loading" && <p className="literature-status" role="status">Loading the citation graph…</p>}
          {phase.kind === "unavailable" && (
            <div className="literature-status" role="alert">
              <p>The literature graph is not available.</p>
              <p className="literature-hint">Start the backend with <code>LYSILOGY_DATABASE_URL</code> set to a Postgres database.</p>
            </div>
          )}
          {phase.kind === "error" && (
            <div className="literature-status" role="alert">
              <p>{phase.message}</p>
              <button type="button" className="literature-action" onClick={runLiterature}>Run lit again <kbd>r</kbd></button>
            </div>
          )}
          {phase.kind === "nowork" && (
            <div className="literature-status">
              <p>This paper is not linked to a Work yet.</p>
              <p className="literature-hint">Running lit finds its record, links it, and loads the works it cites.</p>
              <button type="button" className="literature-action" onClick={runLiterature}>Run lit <kbd>r</kbd></button>
            </div>
          )}
          {phase.kind === "running" && <p className="literature-status" role="status">Linking this paper and loading its references…</p>}

          {graph !== null && (
            <>
              <section className="literature-canvas" aria-label="Works cited by this paper">
                <svg viewBox={`0 0 ${CANVAS.width} ${CANVAS.height}`} role="list" preserveAspectRatio="xMidYMid meet">
                  {placements.slice(1).map((placement) => {
                    const center = placements[0];
                    if (center === undefined) return null;
                    return <line key={`edge-${placement.id}`} className="literature-edge" x1={center.x} y1={center.y} x2={placement.x} y2={placement.y} />;
                  })}
                  {placements.map((placement, index) => {
                    const work = worksById.get(placement.id);
                    if (work === undefined) return null;
                    const isCenter = placement.ring === 0;
                    const isActive = index === activeIndex;
                    const isLocal = localIds.has(work.id);
                    const rightSide = placement.x >= CANVAS.width / 2;
                    const labelX = isCenter ? 0 : rightSide ? 11 : -11;
                    const labelY = isCenter ? 24 : 3;
                    const anchor = isCenter ? "middle" : rightSide ? "start" : "end";
                    return (
                      <g
                        key={placement.id}
                        role="listitem"
                        tabIndex={0}
                        className={`literature-node ${isActive ? "is-active" : ""} ${isCenter ? "is-center" : ""} ${isLocal ? "is-local" : ""}`}
                        data-node-id={work.id}
                        data-local={isLocal ? "true" : "false"}
                        aria-label={`${workLabel(work, FULL_LABEL_LIMIT)}${isLocal ? ", held locally" : ""}`}
                        transform={`translate(${placement.x} ${placement.y})`}
                        onClick={() => openWorkPanel(index)}
                        onFocus={() => setSelected(index)}
                        onKeyDown={(event) => {
                          if (event.key !== " ") return;
                          event.preventDefault();
                          openWorkPanel(index);
                        }}
                      >
                        <circle className="literature-halo" r={isActive ? 15 : 12} />
                        <circle className="literature-dot" r={isCenter ? 8 : isLocal ? 5.5 : 4} />
                        <text className="literature-label" x={labelX} y={labelY} textAnchor={anchor}>
                          {workLabel(work, isActive || isCenter ? FULL_LABEL_LIMIT : LABEL_LIMIT)}
                        </text>
                      </g>
                    );
                  })}
                </svg>
                <p className="literature-legend">
                  <span><i className="literature-key is-local" /> held locally</span>
                  <span><i className="literature-key" /> cited work</span>
                  <span>{placements.length - 1} cited</span>
                  <span><kbd>j</kbd>/<kbd>k</kbd> move · <kbd>↵</kbd> open · <kbd>Esc</kbd> close</span>
                </p>
              </section>

              {workPanelOpen && activeWork !== undefined && (
                <WorkPanel
                  work={activeWork}
                  state={citations}
                  onOpenPaper={onOpenPaper}
                  onClose={() => setWorkPanelOpen(false)}
                />
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function PaperButtons({ papers, onOpenPaper }: { papers: LocalPaper[]; onOpenPaper: (paperId: string) => void }) {
  return (
    <ul className="literature-papers">
      {papers.map((paper) => (
        <li key={paper.paper_id}>
          <button type="button" onClick={() => onOpenPaper(paper.paper_id)}>{paper.title}</button>
        </li>
      ))}
    </ul>
  );
}

function WorkPanel({
  work,
  state,
  onOpenPaper,
  onClose,
}: {
  work: Work;
  state: CitationsState;
  onOpenPaper: (paperId: string) => void;
  onClose: () => void;
}) {
  const own = state.kind === "ready" ? state.citations.local_papers[work.id] ?? [] : [];
  const citingLocal = state.kind === "ready"
    ? state.citations.incoming
      .map((incoming) => ({ work: incoming, papers: state.citations.local_papers[incoming.id] ?? [] }))
      .filter((entry) => entry.papers.length > 0)
    : [];
  return (
    <aside className="literature-work" aria-label="Work">
      <header>
        <h3>{workTitle(work)}</h3>
        <button type="button" aria-label="Close work panel (Esc)" onClick={onClose}>×</button>
      </header>
      <p className="literature-work-meta">
        {work.display_year ?? "year unknown"} · {work.kind}
        {state.kind === "ready" && ` · cites ${state.citations.outgoing.length} · cited by ${state.citations.incoming.length} known`}
      </p>
      {state.kind === "loading" && <p className="literature-hint" role="status">Loading citations…</p>}
      {state.kind === "error" && <p className="literature-hint" role="alert">{state.message}</p>}
      {state.kind === "ready" && (
        <>
          <h4>Local copy</h4>
          {own.length === 0
            ? <p className="literature-hint">No copy of this work is in the library.</p>
            : <PaperButtons papers={own} onOpenPaper={onOpenPaper} />}
          <h4>Cited by local papers</h4>
          {citingLocal.length === 0
            ? <p className="literature-hint">No library paper is known to cite this work.</p>
            : (
              <ul className="literature-papers">
                {citingLocal.flatMap((entry) => entry.papers).map((paper, index) => (
                  <li key={`${paper.paper_id}-${index}`}>
                    <button type="button" onClick={() => onOpenPaper(paper.paper_id)}>{paper.title}</button>
                  </li>
                ))}
              </ul>
            )}
        </>
      )}
    </aside>
  );
}
