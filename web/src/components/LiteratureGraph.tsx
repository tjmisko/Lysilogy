import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { acquisitionStatusText, canFetch, isRetry, type AcquisitionLike } from "../lib/acquisition";
import { ApiError, api } from "../lib/api";
import { LITERATURE_QUIT_EVENT } from "../lib/literatureEvents";
import { layoutFanout, type FanoutPlacement } from "../lib/literatureLayout";
import type {
  AcquisitionJobState, AcquisitionStatus, AcquisitionView, EdgeLabel, EdgeLabelEdit, GraphEdge, GraphNeighbors,
  LocalPaper, Work, WorkCitations,
} from "../types";
import "./LiteratureGraph.css";

type LiteratureGraphProps = {
  paperId: string;
  paperTitle: string;
  onClose: () => void;
  /** Close the overlay and open a local paper. */
  onOpenPaper: (paperId: string) => void;
  /** Close the overlay and show a stored citing sentence in its paper. */
  onOpenPassage: (target: PassageTarget) => void;
};

export type PassageTarget = { paperId: string; page: number; start: number; end: number; passage: string };

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

/** What the panel knows about one Work's fetch in this session. */
type FetchEntry = {
  job: AcquisitionJobState;
  view: AcquisitionView | null;
  paperId: string | null;
  message: string | null;
  /** True once this session watched the fetch finish with a paper in the library. */
  fresh: boolean;
};

const RELATIONS = [
  "background", "uses_method", "uses_data", "extends", "supports", "contrasts", "critiques", "compares", "other",
] as const;

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

function relationName(relation: string): string {
  return relation.replace(/_/gu, " ");
}

/** The label with any in-session review or edit applied. Follows an edit chain. */
function resolveLabel(label: EdgeLabel, patches: ReadonlyMap<string, EdgeLabel>): EdgeLabel {
  let current = label;
  for (let step = 0; step < 16; step += 1) {
    const next = patches.get(current.id);
    if (next === undefined) return current;
    if (next.id === current.id) return next;
    current = next;
  }
  return current;
}

function labelCaption(label: EdgeLabel): string {
  if (label.origin === "user") return "Edited";
  const state = label.review_state === "accepted" ? "Accepted" : "Unreviewed";
  return label.model === null ? state : `${state} · ${label.model}`;
}

function canShowInPaper(label: EdgeLabel): boolean {
  const mention = label.mention;
  return mention !== null && mention.paper_id !== null && mention.passage !== null
    && mention.passage_start !== null && mention.passage_end !== null;
}

function isUnavailable(reason: unknown): boolean {
  return reason instanceof ApiError && reason.status === 503;
}

export function LiteratureGraph({ paperId, paperTitle, onClose, onOpenPaper, onOpenPassage }: LiteratureGraphProps) {
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const [reloadToken, setReloadToken] = useState(0);
  const [selected, setSelected] = useState(0);
  const [workPanelOpen, setWorkPanelOpen] = useState(false);
  const [citations, setCitations] = useState<CitationsState>({ kind: "loading" });
  const [patches, setPatches] = useState<ReadonlyMap<string, EdgeLabel>>(() => new Map());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [labelError, setLabelError] = useState<string | null>(null);
  const [fetches, setFetches] = useState<ReadonlyMap<string, FetchEntry>>(() => new Map());
  const [neighborsToken, setNeighborsToken] = useState(0);
  const [citationsToken, setCitationsToken] = useState(0);
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
        const neighbors = await api.graphNeighbors(link.work.id, "references", {}, controller.signal);
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
            setPatches(new Map());
            setEditingId(null);
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
  const edgeByCited = useMemo(() => {
    const map = new Map<string, GraphEdge>();
    if (graph === null) return map;
    for (const edge of graph.neighbors.edges) {
      if (edge.citing === graph.work.id) map.set(edge.cited, edge);
    }
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
  }, [activeWorkId, citationsToken]);

  const centerId = graph?.work.id;
  const activeFetch = activeWorkId === undefined ? undefined : fetches.get(activeWorkId);

  // Reload the neighbors after a fetch lands, so the node gets its local dot.
  useEffect(() => {
    if (neighborsToken === 0 || centerId === undefined) return;
    const controller = new AbortController();
    api.graphNeighbors(centerId, "references", {}, controller.signal)
      .then((neighbors) => setPhase((current) => current.kind === "graph" ? { ...current, neighbors } : current))
      .catch(() => undefined);
    return () => controller.abort();
  }, [centerId, neighborsToken]);

  const recordFetch = useCallback((workId: string, status: AcquisitionStatus, fresh: boolean): void => {
    setFetches((current) => {
      const previous = current.get(workId);
      // A status read before the server registered our job must not cancel it.
      if (previous?.job === "running" && status.job.state === "idle") return current;
      return new Map(current).set(workId, {
        job: status.job.state,
        view: status.acquisition,
        paperId: status.paper_id,
        message: status.job.message ?? null,
        fresh: fresh || (previous?.fresh ?? false),
      });
    });
  }, []);

  // Read the open Work's fetch status once, so a job started earlier shows up.
  useEffect(() => {
    if (activeWorkId === undefined) return;
    const controller = new AbortController();
    api.workAcquisition(activeWorkId, controller.signal)
      .then((status) => recordFetch(activeWorkId, status, false))
      .catch(() => undefined);
    return () => controller.abort();
  }, [activeWorkId, recordFetch]);

  // Poll the fetch once a second while it runs. Closing the panel stops it.
  const pollingWorkId = activeFetch?.job === "running" ? activeWorkId : undefined;
  useEffect(() => {
    if (pollingWorkId === undefined) return;
    const controller = new AbortController();
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const status = await api.workAcquisition(pollingWorkId, controller.signal);
          if (controller.signal.aborted) return;
          if (status.job.state === "running") {
            recordFetch(pollingWorkId, status, false);
            return;
          }
          window.clearInterval(timer);
          const landed = status.acquisition?.state === "mapped" && status.paper_id !== null;
          recordFetch(pollingWorkId, status, landed);
          if (!landed) return;
          setNeighborsToken((token) => token + 1);
          setCitationsToken((token) => token + 1);
        } catch (reason: unknown) {
          if (controller.signal.aborted) return;
          window.clearInterval(timer);
          setFetches((current) => new Map(current).set(pollingWorkId, {
            job: "failed", view: current.get(pollingWorkId)?.view ?? null, paperId: null,
            message: failureMessage(reason), fresh: false,
          }));
        }
      })();
    }, POLL_MILLISECONDS);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [pollingWorkId, recordFetch]);

  const briefs = graph?.neighbors.acquisitions;
  const activeView: AcquisitionLike | null = activeFetch?.view ?? (activeWork === undefined ? null : briefs?.[activeWork.id] ?? null);
  const allLocalPapers = useMemo(
    () => ({ ...graph?.neighbors.local_papers, ...(citations.kind === "ready" ? citations.citations.local_papers : {}) }),
    [citations, graph],
  );
  const fetchable = activeWork !== undefined && citations.kind === "ready" && canFetch(
    activeWork.id,
    allLocalPapers,
    briefs ?? {},
    activeFetch?.job ?? "none",
  );
  const startFetch = useCallback((): void => {
    if (activeWork === undefined) return;
    const workId = activeWork.id;
    const retry = isRetry(activeView);
    setFetches((current) => new Map(current).set(workId, {
      job: "running", view: current.get(workId)?.view ?? null, paperId: null, message: null, fresh: false,
    }));
    api.fetchWork(workId, retry ? { retry_unavailable: true } : undefined).catch((reason: unknown) => {
      setFetches((current) => new Map(current).set(workId, {
        job: "failed", view: current.get(workId)?.view ?? null, paperId: null,
        message: failureMessage(reason), fresh: false,
      }));
    });
  }, [activeView, activeWork]);

  // Edges between the center and the open Work, with in-session reviews applied.
  const panelEdges = useMemo(() => {
    if (graph === null || activeWork === undefined || activeWork.id === graph.work.id) return [];
    const applied = (edge: GraphEdge | undefined, heading: string): PanelEdge[] => edge === undefined ? [] : [{
      heading,
      labels: edge.labels.map((label) => resolveLabel(label, patches)),
      mentionCount: edge.mention_count,
    }];
    const incoming = citationEdges(citations).find((edge) => edge.citing === activeWork.id && edge.cited === graph.work.id);
    return [
      ...applied(edgeByCited.get(activeWork.id), "Why it is cited"),
      ...applied(incoming, "Why it cites this"),
    ];
  }, [activeWork, citations, edgeByCited, graph, patches]);
  const actionableLabel = useMemo(
    () => panelEdges.flatMap((edge) => edge.labels).find((label) => label.review_state !== "rejected"),
    [panelEdges],
  );

  const recordLabel = useCallback((previous: EdgeLabel, next: EdgeLabel): void => {
    setPatches((current) => new Map(current).set(previous.id, next));
  }, []);
  const reviewLabel = useCallback((label: EdgeLabel, decision: "accept" | "reject"): void => {
    setLabelError(null);
    const call = decision === "accept" ? api.acceptLabel : api.rejectLabel;
    call(label.id).then((next) => recordLabel(label, next)).catch((reason: unknown) => setLabelError(failureMessage(reason)));
  }, [recordLabel]);
  const saveLabelEdit = useCallback((label: EdgeLabel, edit: EdgeLabelEdit): void => {
    setLabelError(null);
    api.editLabel(label.id, edit)
      .then((next) => { recordLabel(label, next); setEditingId(null); })
      .catch((reason: unknown) => setLabelError(failureMessage(reason)));
  }, [recordLabel]);
  const showInPaper = useCallback((label: EdgeLabel): void => {
    const mention = label.mention;
    if (mention === null || mention.paper_id === null || mention.passage === null
      || mention.passage_start === null || mention.passage_end === null) return;
    onOpenPassage({
      paperId: mention.paper_id,
      page: mention.page ?? 1,
      start: mention.passage_start,
      end: mention.passage_end,
      passage: mention.passage,
    });
  }, [onOpenPassage]);

  const openWorkPanel = useCallback((index: number): void => {
    setEditingId(null);
    setLabelError(null);
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
      // One layer per press: the label editor first, then the panel.
      if (editingId !== null) setEditingId(null);
      else setWorkPanelOpen(false);
    };
    root.addEventListener(LITERATURE_QUIT_EVENT, onQuit);
    return () => root.removeEventListener(LITERATURE_QUIT_EVENT, onQuit);
  }, [editingId, workPanelOpen]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || isEditable(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "r" && (phase.kind === "nowork" || phase.kind === "error")) {
        event.preventDefault();
        runLiterature();
        return;
      }
      if (phase.kind !== "graph") return;
      if (workPanelOpen) {
        if (editingId !== null) return;
        if (event.key === "f") {
          if (fetchable) { event.preventDefault(); startFetch(); }
          return;
        }
        if (actionableLabel === undefined) return;
        if (event.key === "a") { event.preventDefault(); reviewLabel(actionableLabel, "accept"); }
        else if (event.key === "x") { event.preventDefault(); reviewLabel(actionableLabel, "reject"); }
        else if (event.key === "e") { event.preventDefault(); setLabelError(null); setEditingId(actionableLabel.id); }
        else if (event.key === "g" && canShowInPaper(actionableLabel)) { event.preventDefault(); showInPaper(actionableLabel); }
        return;
      }
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
  }, [actionableLabel, activeIndex, editingId, fetchable, openWorkPanel, phase.kind, placements.length, reviewLabel, runLiterature, showInPaper, startFetch, workPanelOpen]);

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
                    const edge = edgeByCited.get(placement.id);
                    const labeled = edge?.labels.some((label) => resolveLabel(label, patches).review_state !== "rejected") ?? false;
                    const localOnly = edge !== undefined && edge.sources.includes("local") && !edge.sources.includes("provider");
                    const kind = labeled ? "is-labeled" : localOnly ? "is-local-only" : "";
                    return <line key={`edge-${placement.id}`} className={`literature-edge ${kind}`} data-edge-kind={kind === "" ? "provider" : kind.slice(3)} x1={center.x} y1={center.y} x2={placement.x} y2={placement.y} />;
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
                    const nodeFetch = fetches.get(work.id);
                    const acquisition = nodeFetch?.job === "running"
                      ? "running"
                      : nodeFetch?.view?.state ?? briefs?.[work.id]?.state ?? "none";
                    return (
                      <g
                        key={placement.id}
                        role="listitem"
                        tabIndex={0}
                        className={`literature-node ${isActive ? "is-active" : ""} ${isCenter ? "is-center" : ""} ${isLocal ? "is-local" : ""}`}
                        data-node-id={work.id}
                        data-local={isLocal ? "true" : "false"}
                        data-acquisition={acquisition}
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
                  <span><i className="literature-line is-labeled" /> reason known</span>
                  <span><i className="literature-line is-local-only" /> found in this paper only</span>
                  <span>{placements.length - 1} cited</span>
                  <span><kbd>j</kbd>/<kbd>k</kbd> move · <kbd>↵</kbd> open · <kbd>Esc</kbd> close</span>
                </p>
              </section>

              {workPanelOpen && activeWork !== undefined && (
                <WorkPanel
                  work={activeWork}
                  state={citations}
                  edges={panelEdges}
                  editingId={editingId}
                  labelError={labelError}
                  onReview={reviewLabel}
                  onStartEdit={(label) => { setLabelError(null); setEditingId(label.id); }}
                  onCancelEdit={() => setEditingId(null)}
                  onSaveEdit={saveLabelEdit}
                  onShow={showInPaper}
                  onOpenPaper={onOpenPaper}
                  fetchEntry={activeFetch}
                  fetchView={activeView}
                  canFetch={fetchable}
                  onFetch={startFetch}
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

type PanelEdge = { heading: string; labels: EdgeLabel[]; mentionCount: number };

function citationEdges(state: CitationsState): GraphEdge[] {
  return state.kind === "ready" ? state.citations.edges : [];
}

type WorkPanelProps = {
  work: Work;
  state: CitationsState;
  edges: PanelEdge[];
  editingId: string | null;
  labelError: string | null;
  onReview: (label: EdgeLabel, decision: "accept" | "reject") => void;
  onStartEdit: (label: EdgeLabel) => void;
  onCancelEdit: () => void;
  onSaveEdit: (label: EdgeLabel, edit: EdgeLabelEdit) => void;
  onShow: (label: EdgeLabel) => void;
  onOpenPaper: (paperId: string) => void;
  fetchEntry: FetchEntry | undefined;
  fetchView: AcquisitionLike | null;
  canFetch: boolean;
  onFetch: () => void;
  onClose: () => void;
};

function LabelEditor({ label, onSave, onCancel }: { label: EdgeLabel; onSave: (edit: EdgeLabelEdit) => void; onCancel: () => void }) {
  const [reason, setReason] = useState(label.reason);
  const [quote, setQuote] = useState(label.quote ?? "");
  const [relation, setRelation] = useState(label.relation);
  const submit = (): void => {
    const trimmedQuote = quote.trim();
    onSave({
      reason: reason.trim(),
      ...(relation === label.relation ? {} : { relation }),
      ...(trimmedQuote === "" || trimmedQuote === (label.quote ?? "") ? {} : { quote: trimmedQuote }),
    });
  };
  const onEnter = (event: React.KeyboardEvent): void => {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    submit();
  };
  return (
    <form className="literature-editor" onSubmit={(event) => { event.preventDefault(); submit(); }}>
      <label>
        Reason
        <textarea aria-label="Reason" autoFocus rows={3} value={reason} onChange={(event) => setReason(event.target.value)} onKeyDown={onEnter} />
      </label>
      <label>
        Quote (copied exactly from the citing sentence)
        <input aria-label="Quote" value={quote} onChange={(event) => setQuote(event.target.value)} onKeyDown={onEnter} />
      </label>
      <label>
        Relation
        <select aria-label="Relation" value={relation} onChange={(event) => setRelation(event.target.value)}>
          {RELATIONS.map((name) => <option key={name} value={name}>{relationName(name)}</option>)}
        </select>
      </label>
      <p className="literature-hint"><kbd>↵</kbd> save · <kbd>Shift</kbd>+<kbd>↵</kbd> new line · <kbd>Esc</kbd> cancel</p>
      <div className="literature-label-actions">
        <button type="submit">Save</button>
        <button type="button" onClick={onCancel}>Cancel</button>
      </div>
    </form>
  );
}

function LabelCard({ label, props, rejected }: { label: EdgeLabel; props: WorkPanelProps; rejected: boolean }) {
  const editing = props.editingId === label.id;
  const showable = canShowInPaper(label);
  return (
    <article className={`literature-label-card ${rejected ? "is-rejected" : ""}`} data-label-id={label.id} data-review-state={label.review_state}>
      <p className="literature-label-line">
        <span className="literature-relation">{relationName(label.relation)}</span>
        <span className="literature-reason">{label.reason}</span>
      </p>
      {label.quote !== null && label.quote !== "" && <blockquote>{label.quote}</blockquote>}
      <p className="literature-caption">{labelCaption(label)}{label.mention?.page != null && ` · p. ${label.mention.page}`}</p>
      {editing
        ? <LabelEditor label={label} onSave={(edit) => props.onSaveEdit(label, edit)} onCancel={props.onCancelEdit} />
        : (
          <div className="literature-label-actions">
            <button type="button" disabled={!showable} title={showable ? undefined : "The citing paper is not in the library"} onClick={() => props.onShow(label)}>Show in paper <kbd>g</kbd></button>
            {!rejected && <button type="button" onClick={() => props.onReview(label, "accept")} disabled={label.review_state === "accepted"}>Accept <kbd>a</kbd></button>}
            {!rejected && <button type="button" onClick={() => props.onReview(label, "reject")}>Reject <kbd>x</kbd></button>}
            {rejected && <button type="button" onClick={() => props.onReview(label, "accept")}>Accept</button>}
            {!rejected && <button type="button" onClick={() => props.onStartEdit(label)}>Edit <kbd>e</kbd></button>}
          </div>
        )}
    </article>
  );
}

function EdgeReasons({ edge, props }: { edge: PanelEdge; props: WorkPanelProps }) {
  const active = edge.labels.filter((label) => label.review_state !== "rejected");
  const rejected = edge.labels.filter((label) => label.review_state === "rejected");
  return (
    <section className="literature-reasons" aria-label={edge.heading}>
      <h4>{edge.heading}</h4>
      {active.length === 0 && rejected.length === 0 && (
        <p className="literature-hint">
          {edge.mentionCount > 0 ? `Mentioned ${edge.mentionCount} time${edge.mentionCount === 1 ? "" : "s"}; no reason has been written yet.` : "No reason has been written yet."}
        </p>
      )}
      {active.map((label) => <LabelCard key={label.id} label={label} props={props} rejected={false} />)}
      {rejected.length > 0 && (
        <details className="literature-rejected">
          <summary>Rejected ({rejected.length})</summary>
          {rejected.map((label) => <LabelCard key={label.id} label={label} props={props} rejected />)}
        </details>
      )}
    </section>
  );
}

function FetchControl({ props }: { props: WorkPanelProps }) {
  const { fetchEntry, fetchView, canFetch: allowed, onFetch } = props;
  const job = fetchEntry?.job ?? "none";
  const retry = isRetry(fetchView);
  return (
    <>
      <p className="literature-hint literature-fetch-status" role="status" data-fetch-job={job}>
        {job !== "running" && "No copy of this work is in the library. "}{acquisitionStatusText(fetchView, job)}
        {job === "failed" && fetchEntry?.message != null && ` ${fetchEntry.message}`}
      </p>
      <button type="button" className="literature-action" disabled={!allowed} onClick={onFetch}>
        {retry ? "Retry" : "Fetch open-access copy"} <kbd>f</kbd>
      </button>
    </>
  );
}

function WorkPanel(props: WorkPanelProps) {
  const { work, state, edges, labelError, onOpenPaper, onClose, fetchEntry } = props;
  const freshPaperId = fetchEntry?.fresh === true ? fetchEntry.paperId : null;
  const own = (state.kind === "ready" ? state.citations.local_papers[work.id] ?? [] : [])
    .filter((paper) => paper.paper_id !== freshPaperId);
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
      {edges.map((edge) => <EdgeReasons key={edge.heading} edge={edge} props={props} />)}
      {labelError !== null && <p className="literature-hint literature-label-error" role="alert">{labelError}</p>}
      {state.kind === "loading" && <p className="literature-hint" role="status">Loading citations…</p>}
      {state.kind === "error" && <p className="literature-hint" role="alert">{state.message}</p>}
      {state.kind === "ready" && (
        <>
          <h4>Local copy</h4>
          {freshPaperId !== null && (
            <>
              <p className="literature-hint literature-fetch-status" role="status">Fetched.</p>
              <button type="button" className="literature-action" autoFocus onClick={() => onOpenPaper(freshPaperId)}>Open in reader</button>
            </>
          )}
          {own.length > 0 && <PaperButtons papers={own} onOpenPaper={onOpenPaper} />}
          {own.length === 0 && freshPaperId === null && <FetchControl props={props} />}
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
