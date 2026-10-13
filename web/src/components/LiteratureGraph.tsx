import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { acquisitionStatusText, canFetch, isRetry, type AcquisitionLike } from "../lib/acquisition";
import { ApiError, api } from "../lib/api";
import { LITERATURE_QUIT_EVENT } from "../lib/literatureEvents";
import {
  layoutRankedFanout, layoutWalk, planVisibleLabels, walkViewBox,
  type LabelRect, type LabelRequest, type RankedPlacement, type WalkPlacement,
} from "../lib/literatureLayout";
import {
  DEFAULT_TOP_N, clusterAriaLabel, findTagByName, markAfterToggle, nextTopN, nodeRadius, normalizeTagName, toggleInSet,
} from "../lib/relevance";
import type {
  AcquisitionJobState, AcquisitionStatus, AcquisitionView, EdgeLabel, EdgeLabelEdit, ExpansionView, GraphEdge,
  GraphNeighbors, LocalPaper, MarkView, NeighborCluster, ProminenceView, TagView, Work, WorkCitations, WorkMarkName,
} from "../types";
import {
  EXPANSION_TOP_N, MAX_UI_DEPTH, MAX_UI_EXPANSIONS, addExpansion, canExpand, collapseExpansion, fanoutKnown,
  type WalkExpansion,
} from "../lib/walk";
import { LiteratureCuration, TAG_INPUT_CLASS } from "./LiteratureCuration";
import { LiteratureLegend, TAG_KEY_LIMIT } from "./LiteratureLegend";
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

/** A node being expanded: waiting on the server's expand job, or fetching its children. */
type ExpandingEntry = { phase: "job" | "fetch"; parentDepth: number };

type PlacedNode = RankedPlacement | WalkPlacement;

function hopOf(placement: PlacedNode): number {
  if ("hop" in placement) return placement.hop;
  return placement.kind === "center" ? 0 : 1;
}

const RELATIONS = [
  "background", "uses_method", "uses_data", "extends", "supports", "contrasts", "critiques", "compares", "other",
] as const;

const MARK_KEYS: Readonly<Record<string, WorkMarkName>> = { p: "pinned", d: "downplayed", h: "hidden" };

const CANVAS = { width: 1000, height: 700 } as const;
const POLL_MILLISECONDS = 1000;
const MAX_NODE_TICKS = 3;
const FALLBACK_TAG_COLOR = "#888888";
const LABEL_LIMIT = 30;
const FULL_LABEL_LIMIT = 90;

function clusterPillWidth(cluster: NeighborCluster): number {
  return Math.max(56, `${cluster.label} · ${cluster.count}`.length * 6.2 + 18);
}

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

/** Walk children label away from their parent; base nodes label away from the canvas middle. */
function labelOnRight(placement: PlacedNode, positions: ReadonlyMap<string, PlacedNode>): boolean {
  if (!("parentId" in placement)) return placement.x >= CANVAS.width / 2;
  const parent = positions.get(placement.parentId);
  return placement.x >= (parent?.x ?? CANVAS.width / 2);
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [centerWork, setCenterWork] = useState<Work | null>(null);
  const [topN, setTopN] = useState(DEFAULT_TOP_N);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [tagFilter, setTagFilter] = useState<ReadonlySet<string>>(() => new Set());
  const [tags, setTags] = useState<TagView[]>([]);
  const [tagsToken, setTagsToken] = useState(0);
  const [tagError, setTagError] = useState<string | null>(null);
  const [neighborsError, setNeighborsError] = useState<string | null>(null);
  const [markOverrides, setMarkOverrides] = useState<ReadonlyMap<string, MarkView | null>>(() => new Map());
  const [markError, setMarkError] = useState<string | null>(null);
  const pendingMarks = useRef(0);
  const clusterLabels = useRef(new Map<string, string>());
  const [workPanelOpen, setWorkPanelOpen] = useState(false);
  const [citations, setCitations] = useState<CitationsState>({ kind: "loading" });
  const [patches, setPatches] = useState<ReadonlyMap<string, EdgeLabel>>(() => new Map());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [labelError, setLabelError] = useState<string | null>(null);
  const [fetches, setFetches] = useState<ReadonlyMap<string, FetchEntry>>(() => new Map());
  const [neighborsToken, setNeighborsToken] = useState(0);
  const [citationsToken, setCitationsToken] = useState(0);
  const [walk, setWalk] = useState<readonly WalkExpansion[]>([]);
  const [walkData, setWalkData] = useState<ReadonlyMap<string, GraphNeighbors>>(() => new Map());
  const [expanding, setExpanding] = useState<ReadonlyMap<string, ExpandingEntry>>(() => new Map());
  const [walkNotice, setWalkNotice] = useState<string | null>(null);
  const [viewOverrides, setViewOverrides] = useState<ReadonlyMap<string, ExpansionView>>(() => new Map());
  const expandingRef = useRef<ReadonlyMap<string, ExpandingEntry>>(new Map());
  const walkAbort = useRef<AbortController | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // Everything in flight for the walk dies with the overlay.
  useEffect(() => {
    const controller = new AbortController();
    walkAbort.current = controller;
    return () => controller.abort();
  }, []);

  // Load the paper's Work and its references. Aborted on unmount or reload.
  useEffect(() => {
    const controller = new AbortController();
    void (async () => {
      try {
        const link = await api.paperWork(paperId, controller.signal);
        if (controller.signal.aborted) return;
        setCenterWork(link.work);
        if (link.work === null) setPhase({ kind: "nowork" });
      } catch (reason: unknown) {
        if (controller.signal.aborted) return;
        setPhase(isUnavailable(reason)
          ? { kind: "unavailable" }
          : { kind: "error", message: failureMessage(reason) });
      }
    })();
    return () => controller.abort();
  }, [paperId, reloadToken]);

  // Load the tag list once per overlay and again after every tag mutation.
  useEffect(() => {
    const controller = new AbortController();
    api.listTags(controller.signal)
      .then((loaded) => setTags(loaded.tags))
      .catch((reason: unknown) => {
        if (!controller.signal.aborted && !isUnavailable(reason)) setTagError(failureMessage(reason));
      });
    return () => controller.abort();
  }, [tagsToken]);

  // Load the ranked neighbors. Any change to top-N, expansions, or filters refetches;
  // the previous request is aborted and the graph stays on screen meanwhile.
  useEffect(() => {
    if (centerWork === null) return;
    const controller = new AbortController();
    api.graphNeighbors(centerWork.id, "references", { limit: topN, expand: expanded, tags: tagFilter }, controller.signal)
      .then((neighbors) => {
        for (const cluster of neighbors.clusters ?? []) clusterLabels.current.set(cluster.key, cluster.label);
        setNeighborsError(null);
        if (pendingMarks.current === 0) setMarkOverrides(new Map());
        setPhase((current) => current.kind === "graph" && current.work.id === centerWork.id
          ? { ...current, neighbors }
          : { kind: "graph", work: centerWork, neighbors });
      })
      .catch((reason: unknown) => {
        if (controller.signal.aborted) return;
        setNeighborsError(failureMessage(reason));
        setPhase((current) => {
          if (current.kind === "graph") return current;
          return isUnavailable(reason) ? { kind: "unavailable" } : { kind: "error", message: failureMessage(reason) };
        });
      });
    return () => controller.abort();
  }, [centerWork, expanded, neighborsToken, tagFilter, topN]);

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
            setSelectedId(null);
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
  const clusters = useMemo<NeighborCluster[]>(() => graph?.neighbors.clusters ?? [], [graph]);
  const clustersByKey = useMemo(() => new Map(clusters.map((cluster) => [cluster.key, cluster])), [clusters]);
  const placements = useMemo<RankedPlacement[]>(() => {
    if (graph === null) return [];
    return layoutRankedFanout(
      graph.work.id,
      graph.neighbors.nodes.map((node) => node.id),
      clusters.map((cluster) => cluster.key),
      CANVAS,
    );
  }, [clusters, graph]);
  const centerPoint = useMemo(() => ({ x: CANVAS.width / 2, y: CANVAS.height / 2 }), []);
  const walkLayout = useMemo(() => layoutWalk(placements, walk, centerPoint), [centerPoint, placements, walk]);
  const allPlacements = useMemo<PlacedNode[]>(() => [...placements, ...walkLayout.placements], [placements, walkLayout]);
  const positions = useMemo(() => new Map(allPlacements.map((placement) => [placement.id, placement])), [allPlacements]);
  const viewBox = useMemo(() => walkViewBox(CANVAS, walkLayout.bounds), [walkLayout]);
  // Child payloads join the base graph's lookups; the base graph wins on a clash.
  const merged = useMemo(() => {
    const payloads = [...walkData.values(), ...(graph === null ? [] : [graph.neighbors])];
    const works = new Map<string, Work>();
    const localPapers: GraphNeighbors["local_papers"] = {};
    const acquisitions: GraphNeighbors["acquisitions"] = {};
    const prominence: Record<string, ProminenceView> = {};
    const marks: Record<string, MarkView> = {};
    const workTags: Record<string, string[]> = {};
    const expansions: Record<string, ExpansionView> = {};
    for (const [workId, view] of viewOverrides) expansions[workId] = view;
    for (const payload of payloads) {
      for (const node of payload.nodes) works.set(node.id, node);
      Object.assign(localPapers, payload.local_papers);
      Object.assign(acquisitions, payload.acquisitions);
      Object.assign(prominence, payload.prominence);
      Object.assign(marks, payload.marks);
      Object.assign(workTags, payload.work_tags);
      Object.assign(expansions, payload.expansions);
    }
    if (graph !== null) works.set(graph.work.id, graph.work);
    return { works, localPapers, acquisitions, prominence, marks, workTags, expansions };
  }, [graph, viewOverrides, walkData]);
  const worksById = merged.works;
  const edgeByCited = useMemo(() => {
    const map = new Map<string, GraphEdge>();
    if (graph === null) return map;
    for (const edge of graph.neighbors.edges) {
      if (edge.citing === graph.work.id) map.set(edge.cited, edge);
    }
    return map;
  }, [graph]);
  const selectedIndex = selectedId === null ? 0 : allPlacements.findIndex((placement) => placement.id === selectedId);
  const activeIndex = Math.max(0, selectedIndex);
  const activePlacement = allPlacements[activeIndex];
  const activeWork = activePlacement?.kind === "cluster" || activePlacement === undefined
    ? undefined : worksById.get(activePlacement.id);
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

  const markFor = useCallback((workId: string): MarkView | null => {
    const override = markOverrides.get(workId);
    if (override !== undefined) return override;
    return merged.marks[workId] ?? null;
  }, [merged, markOverrides]);
  const applyMark = useCallback((workId: string, next: WorkMarkName | null, note: string | null): void => {
    setMarkError(null);
    setMarkOverrides((current) => new Map(current).set(workId, next === null ? null : { mark: next, note }));
    pendingMarks.current += 1;
    const call = next === null ? api.clearWorkMark(workId) : api.setWorkMark(workId, next, note);
    call.then(() => {
      pendingMarks.current -= 1;
      setNeighborsToken((token) => token + 1);
    }).catch((reason: unknown) => {
      pendingMarks.current -= 1;
      setMarkOverrides((current) => {
        const reverted = new Map(current);
        reverted.delete(workId);
        return reverted;
      });
      setMarkError(failureMessage(reason));
    });
  }, []);

  const tagsById = useMemo(() => new Map(tags.map((tag) => [tag.id, tag])), [tags]);
  const afterTagMutation = useCallback((): void => {
    setTagsToken((token) => token + 1);
    setNeighborsToken((token) => token + 1);
  }, []);
  const assignTagByName = useCallback((workId: string, name: string, color: string): void => {
    setTagError(null);
    const fail = (reason: unknown): void => setTagError(failureMessage(reason));
    const assign = (tag: TagView): Promise<void> => api.assignTag(workId, tag.id);
    const known = findTagByName(tags, name);
    if (known !== undefined) {
      assign(known).then(afterTagMutation).catch(fail);
      return;
    }
    api.createTag(normalizeTagName(name), color)
      .catch(async (reason: unknown) => {
        // Another session may have created the same name; reuse it instead of failing.
        if (!(reason instanceof ApiError) || reason.status !== 409) throw reason;
        const refreshed = (await api.listTags()).tags;
        const match = findTagByName(refreshed, name);
        if (match === undefined) throw reason;
        return match;
      })
      .then(assign)
      .then(afterTagMutation)
      .catch((reason: unknown) => {
        fail(reason);
        setTagsToken((token) => token + 1);
      });
  }, [afterTagMutation, tags]);
  const unassignTag = useCallback((workId: string, tag: TagView): void => {
    setTagError(null);
    api.unassignTag(workId, tag.id).then(afterTagMutation).catch((reason: unknown) => setTagError(failureMessage(reason)));
  }, [afterTagMutation]);
  const toggleTagFilter = useCallback((tagId: string): void => {
    setTagFilter((current) => toggleInSet(current, tagId));
  }, []);
  const clearTagFilter = useCallback((): void => setTagFilter(new Set()), []);

  // Draw only labels that do not collide. The center and active Work always keep theirs; pinned Works choose first.
  const visibleLabels = useMemo(() => {
    const requests: LabelRequest[] = [];
    const pinnedRequests: LabelRequest[] = [];
    const deepRequests: LabelRequest[] = [];
    const reserved: LabelRect[] = [];
    allPlacements.forEach((placement, index) => {
      if (placement.kind === "cluster") {
        const cluster = clustersByKey.get(placement.clusterKey ?? "");
        if (cluster === undefined) return;
        const width = clusterPillWidth(cluster);
        reserved.push({ left: placement.x - width / 2, right: placement.x + width / 2, top: placement.y - 12, bottom: placement.y + 12 });
        return;
      }
      const work = worksById.get(placement.id);
      if (work === undefined) return;
      const isCenter = placement.kind === "center";
      const isActive = index === activeIndex;
      const pinned = markFor(work.id)?.mark === "pinned";
      const deep = hopOf(placement) >= 2;
      (pinned ? pinnedRequests : deep ? deepRequests : requests).push({
        id: placement.id,
        x: placement.x,
        y: placement.y,
        length: workLabel(work, isActive || isCenter ? FULL_LABEL_LIMIT : LABEL_LIMIT).length,
        anchor: isCenter ? "middle" : labelOnRight(placement, positions) ? "start" : "end",
        fontSize: isActive || isCenter ? 12 : 10,
        offsetY: isCenter ? 24 : 3,
        forced: isCenter || isActive,
      });
    });
    // Pinned Works claim label space first, then hop one in rank order, then deeper hops.
    return planVisibleLabels([...pinnedRequests, ...requests, ...deepRequests], reserved);
  }, [activeIndex, allPlacements, clustersByKey, markFor, positions, worksById]);

  const briefs = merged.acquisitions;
  const activeView: AcquisitionLike | null = activeFetch?.view ?? (activeWork === undefined ? null : briefs[activeWork.id] ?? null);
  const allLocalPapers = useMemo(
    () => ({ ...merged.localPapers, ...(citations.kind === "ready" ? citations.citations.local_papers : {}) }),
    [citations, merged],
  );
  const fetchable = activeWork !== undefined && citations.kind === "ready" && canFetch(
    activeWork.id,
    allLocalPapers,
    briefs,
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

  const expandCluster = useCallback((cluster: NeighborCluster): void => {
    setExpanded((current) => new Set(current).add(cluster.key));
    setSelectedId(cluster.work_ids[0] ?? null);
  }, []);
  const toggleExpansion = useCallback((key: string): void => {
    setExpanded((current) => toggleInSet(current, key));
  }, []);

  // Expansion in place. A node with known fanout is fetched at once; otherwise the
  // server expands it first and we poll its job.
  const finishExpanding = useCallback((workId: string): void => {
    setExpanding((current) => {
      const next = new Map(current);
      next.delete(workId);
      return next;
    });
  }, []);
  const failExpanding = useCallback((workId: string, message: string): void => {
    finishExpanding(workId);
    setWalkNotice(message);
  }, [finishExpanding]);
  const loadChildren = useCallback((workId: string, parentDepth: number): void => {
    const signal = walkAbort.current?.signal;
    setExpanding((current) => new Map(current).set(workId, { phase: "fetch", parentDepth }));
    api.graphNeighbors(workId, "references", { limit: EXPANSION_TOP_N }, signal)
      .then((neighbors) => {
        if (signal?.aborted) return;
        const view = neighbors.expansions?.[workId];
        if (view !== undefined) setViewOverrides((current) => new Map(current).set(workId, view));
        const childIds = neighbors.nodes.map((node) => node.id).filter((id) => id !== workId);
        if (childIds.length === 0) {
          const reason = view?.state === "unavailable" ? `References are unavailable${view.unavailable_reason === null ? "" : ` (${relationName(view.unavailable_reason)})`}.` : "No references are known for this work.";
          failExpanding(workId, reason);
          return;
        }
        setWalkData((current) => new Map(current).set(workId, neighbors));
        setWalk((current) => addExpansion(current, workId, childIds, parentDepth));
        finishExpanding(workId);
      })
      .catch((reason: unknown) => {
        if (!signal?.aborted) failExpanding(workId, failureMessage(reason));
      });
  }, [failExpanding, finishExpanding]);

  const toggleWalk = useCallback((workId: string): void => {
    if (graph === null) return;
    setWalkNotice(null);
    if (workId === graph.work.id) {
      setWalkNotice("These are already the references of this paper.");
      return;
    }
    const placement = positions.get(workId);
    if (placement === undefined || placement.kind === "cluster") return;
    if (expandingRef.current.has(workId)) return;
    const verdict = canExpand(walk, workId, (id) => {
      const known = positions.get(id);
      return known === undefined ? 1 : hopOf(known);
    });
    if (verdict === "too_deep") {
      setWalkNotice(`Expansion stops ${MAX_UI_DEPTH} hops from the center.`);
      return;
    }
    if (verdict === "too_many") {
      setWalkNotice(`At most ${MAX_UI_EXPANSIONS} works can be expanded at once. Collapse one first.`);
      return;
    }
    if (walk.some((expansion) => expansion.parentId === workId)) {
      const remaining = collapseExpansion(walk, workId);
      const kept = new Set(remaining.map((expansion) => expansion.parentId));
      setWalk(remaining);
      setWalkData((current) => new Map([...current].filter(([parentId]) => kept.has(parentId))));
      return;
    }
    const parentDepth = hopOf(placement);
    if (fanoutKnown(merged.expansions[workId], merged.prominence[workId])) {
      loadChildren(workId, parentDepth);
      return;
    }
    setExpanding((current) => new Map(current).set(workId, { phase: "job", parentDepth }));
    api.expandWork(workId)
      .then((job) => {
        if (walkAbort.current?.signal.aborted) return;
        if (job.state === "succeeded") loadChildren(workId, parentDepth);
        else if (job.state === "failed") failExpanding(workId, job.message ?? "Expanding this work failed.");
        else if (job.state === "idle") failExpanding(workId, "The expansion did not start. Try again.");
      })
      .catch((reason: unknown) => {
        if (!walkAbort.current?.signal.aborted) failExpanding(workId, failureMessage(reason));
      });
  }, [failExpanding, graph, loadChildren, merged, positions, walk]);

  expandingRef.current = expanding;
  const jobKey = [...expanding].filter(([, entry]) => entry.phase === "job").map(([workId]) => workId).sort().join(",");
  // Poll expansion jobs once a second. The interval dies with the overlay.
  useEffect(() => {
    if (jobKey === "") return;
    const controller = new AbortController();
    const timer = window.setInterval(() => {
      for (const workId of jobKey.split(",")) {
        void (async () => {
          try {
            const status = await api.expansionStatus(workId, controller.signal);
            const entry = expandingRef.current.get(workId);
            if (controller.signal.aborted || entry?.phase !== "job" || status.job.state === "running") return;
            if (status.job.state === "failed") failExpanding(workId, status.job.message ?? "Expanding this work failed.");
            else if (status.job.state === "idle") failExpanding(workId, "The expansion is no longer running. Try again.");
            else loadChildren(workId, entry.parentDepth);
          } catch (reason: unknown) {
            if (!controller.signal.aborted) failExpanding(workId, failureMessage(reason));
          }
        })();
      }
    }, POLL_MILLISECONDS);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [failExpanding, jobKey, loadChildren]);

  // Keep expansions whose parent is still on the canvas; drop the rest with their subtrees.
  useEffect(() => {
    if (graph === null) return;
    const orphans = walk.filter((expansion) => !positions.has(expansion.parentId));
    if (orphans.length === 0) return;
    const remaining = orphans.reduce((list, orphan) => collapseExpansion(list, orphan.parentId), walk);
    const kept = new Set(remaining.map((expansion) => expansion.parentId));
    setWalk(remaining);
    setWalkData((current) => new Map([...current].filter(([parentId]) => kept.has(parentId))));
  }, [graph, positions, walk]);

  // A notice belongs to the selection it was raised on.
  useEffect(() => setWalkNotice(null), [selectedId]);

  // Enter, click and Space: open a Work's panel, or expand a cluster.
  const activateIndex = useCallback((index: number): void => {
    const placement = allPlacements[index];
    if (placement === undefined) return;
    if (placement.kind === "cluster") {
      const cluster = clustersByKey.get(placement.clusterKey ?? "");
      if (cluster !== undefined) expandCluster(cluster);
      return;
    }
    setEditingId(null);
    setLabelError(null);
    setMarkError(null);
    setTagError(null);
    setSelectedId(placement.id);
    setCitations({ kind: "loading" });
    setWorkPanelOpen(true);
  }, [allPlacements, clustersByKey, expandCluster]);

  // Esc and q reach the App close chain first. It asks us to close the Work panel
  // before it closes the overlay, and cancels the event when we did.
  useEffect(() => {
    const root = rootRef.current;
    if (root === null) return;
    const onQuit = (event: Event): void => {
      if (!workPanelOpen) return;
      event.preventDefault();
      // One layer per press: a focused tag input, the label editor, then the panel.
      const focused = document.activeElement;
      if (focused instanceof HTMLInputElement && focused.classList.contains(TAG_INPUT_CLASS)) focused.blur();
      else if (editingId !== null) setEditingId(null);
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
      if (event.key === "w") {
        if (editingId !== null || activeWork === undefined) return;
        event.preventDefault();
        toggleWalk(activeWork.id);
        return;
      }
      if (workPanelOpen) {
        if (editingId !== null) return;
        const requestedMark = MARK_KEYS[event.key];
        if (requestedMark !== undefined) {
          if (activeWork === undefined || activeWork.id === centerId) return;
          event.preventDefault();
          const current = markFor(activeWork.id);
          applyMark(activeWork.id, markAfterToggle(current?.mark ?? null, requestedMark), current?.note ?? null);
          return;
        }
        if (event.key === "t") {
          if (activeWork === undefined || activeWork.id === centerId) return;
          const input = rootRef.current?.querySelector<HTMLInputElement>(`.${TAG_INPUT_CLASS}`);
          if (input === null || input === undefined) return;
          event.preventDefault();
          input.focus();
          return;
        }
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
      if (event.key === "0") {
        event.preventDefault();
        setTagFilter(new Set());
        return;
      }
      if (event.key >= "1" && event.key <= "9") {
        const tag = tags[Number(event.key) - 1];
        if (Number(event.key) > TAG_KEY_LIMIT || tag === undefined) return;
        event.preventDefault();
        toggleTagFilter(tag.id);
        return;
      }
      if (event.key === "j" || event.key === "ArrowDown" || event.key === "l" || event.key === "ArrowRight") {
        event.preventDefault();
        setSelectedId(allPlacements[(activeIndex + 1) % allPlacements.length]?.id ?? null);
      } else if (event.key === "k" || event.key === "ArrowUp" || event.key === "h" || event.key === "ArrowLeft") {
        event.preventDefault();
        setSelectedId(allPlacements[(activeIndex + allPlacements.length - 1) % allPlacements.length]?.id ?? null);
      } else if (event.key === "Enter" || event.key === "o") {
        event.preventDefault();
        activateIndex(activeIndex);
      } else if (event.key === "c") {
        event.preventDefault();
        setExpanded(new Set());
      } else if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        setTopN((current) => nextTopN(current, 1));
      } else if (event.key === "-") {
        event.preventDefault();
        setTopN((current) => nextTopN(current, -1));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [actionableLabel, activateIndex, activeIndex, activeWork, applyMark, centerId, editingId, fetchable, markFor, phase.kind, allPlacements, reviewLabel, runLiterature, showInPaper, startFetch, tags, toggleTagFilter, toggleWalk, workPanelOpen]);

  // A Work that left the canvas (hidden, downplayed, filtered) takes its panel with it.
  const panelWorkGone = workPanelOpen && graph !== null && selectedId !== null
    && !allPlacements.some((placement) => placement.id === selectedId);
  useEffect(() => {
    if (panelWorkGone) setWorkPanelOpen(false);
  }, [panelWorkGone]);

  const localIds = useMemo(() => new Set(
    Object.entries(merged.localPapers)
      .filter(([, papers]) => papers.length > 0)
      .map(([workId]) => workId),
  ), [merged]);

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
                <svg viewBox={viewBox} role="list" preserveAspectRatio="xMidYMid meet">
                  {placements.slice(1).map((placement) => {
                    const center = placements[0];
                    if (center === undefined || placement.kind !== "work") return null;
                    const edge = edgeByCited.get(placement.id);
                    const labeled = edge?.labels.some((label) => resolveLabel(label, patches).review_state !== "rejected") ?? false;
                    const localOnly = edge !== undefined && edge.sources.includes("local") && !edge.sources.includes("provider");
                    const kind = labeled ? "is-labeled" : localOnly ? "is-local-only" : "";
                    return <line key={`edge-${placement.id}`} className={`literature-edge ${kind}`} data-edge-kind={kind === "" ? "provider" : kind.slice(3)} x1={center.x} y1={center.y} x2={placement.x} y2={placement.y} />;
                  })}
                  {walkLayout.walkEdges.map((edge) => {
                    const from = positions.get(edge.from);
                    const to = positions.get(edge.to);
                    if (from === undefined || to === undefined) return null;
                    return <line key={`walk-${edge.from}-${edge.to}`} className="literature-walk-edge" data-walk-edge={`${edge.from}>${edge.to}`} x1={from.x} y1={from.y} x2={to.x} y2={to.y} />;
                  })}
                  {walkLayout.crossLinks.map((link) => {
                    const from = positions.get(link.from);
                    const to = positions.get(link.to);
                    if (from === undefined || to === undefined) return null;
                    return <line key={`cross-${link.from}-${link.to}`} className="literature-cross-link" data-cross-link={`${link.from}>${link.to}`} x1={from.x} y1={from.y} x2={to.x} y2={to.y} />;
                  })}
                  {allPlacements.map((placement, index) => {
                    const isActive = index === activeIndex;
                    const rightSide = labelOnRight(placement, positions);
                    if (placement.kind === "cluster") {
                      const cluster = clustersByKey.get(placement.clusterKey ?? "");
                      if (cluster === undefined) return null;
                      const text = `${cluster.label} · ${cluster.count}`;
                      const width = clusterPillWidth(cluster);
                      return (
                        <g
                          key={placement.id}
                          role="listitem"
                          tabIndex={0}
                          className={`literature-node literature-cluster ${isActive ? "is-active" : ""}`}
                          data-cluster-key={cluster.key}
                          data-cluster-kind={cluster.kind}
                          data-cluster-count={cluster.count}
                          aria-label={clusterAriaLabel(cluster)}
                          transform={`translate(${placement.x} ${placement.y})`}
                          onClick={() => expandCluster(cluster)}
                          onFocus={() => setSelectedId(placement.id)}
                          onKeyDown={(event) => {
                            if (event.key !== " ") return;
                            event.preventDefault();
                            expandCluster(cluster);
                          }}
                        >
                          <title>{cluster.sample_titles.join("\n")}</title>
                          <rect className="literature-cluster-pill" x={-width / 2} y={-10} width={width} height={20} rx={10} />
                          <text className="literature-cluster-label" textAnchor="middle" y={3.5}>{text}</text>
                        </g>
                      );
                    }
                    const work = worksById.get(placement.id);
                    if (work === undefined) return null;
                    const isCenter = placement.kind === "center";
                    const isLocal = localIds.has(work.id);
                    const labelX = isCenter ? 0 : rightSide ? 11 : -11;
                    const labelY = isCenter ? 24 : 3;
                    const anchor = isCenter ? "middle" : rightSide ? "start" : "end";
                    const nodeFetch = fetches.get(work.id);
                    const acquisition = nodeFetch?.job === "running"
                      ? "running"
                      : nodeFetch?.view?.state ?? briefs[work.id]?.state ?? "none";
                    const prominence = merged.prominence[work.id];
                    const mark = isCenter ? null : markFor(work.id);
                    const tagIds = merged.workTags[work.id] ?? [];
                    const isExpanded = walk.some((expansion) => expansion.parentId === work.id);
                    const isRunning = expanding.has(work.id);
                    const radius = isCenter ? 8 : prominence !== undefined ? nodeRadius(prominence.score) : isLocal ? 5.5 : 4;
                    return (
                      <g
                        key={placement.id}
                        role="listitem"
                        tabIndex={0}
                        className={`literature-node ${isActive ? "is-active" : ""} ${isCenter ? "is-center" : ""} ${isLocal ? "is-local" : ""}`}
                        data-node-id={work.id}
                        data-local={isLocal ? "true" : "false"}
                        data-acquisition={acquisition}
                        data-mark={mark?.mark ?? "none"}
                        data-hop={hopOf(placement)}
                        data-expanded={isExpanded ? "true" : undefined}
                        data-expansion={isRunning ? "running" : undefined}
                        data-score={prominence === undefined ? undefined : prominence.score.toFixed(2)}
                        data-tags={tagIds.join(",")}
                        aria-label={`${workLabel(work, FULL_LABEL_LIMIT)}${isLocal ? ", held locally" : ""}${mark === null ? "" : `, ${mark.mark}`}`}
                        transform={`translate(${placement.x} ${placement.y})`}
                        onClick={() => activateIndex(index)}
                        onDoubleClick={() => toggleWalk(placement.id)}
                        onFocus={() => setSelectedId(placement.id)}
                        onKeyDown={(event) => {
                          if (event.key !== " ") return;
                          event.preventDefault();
                          activateIndex(index);
                        }}
                      >
                        <circle className="literature-halo" r={isActive ? 15 : 12} />
                        {mark?.mark === "pinned" && <circle className="literature-pin-ring" r={radius + 3} />}
                        <circle className="literature-dot" r={radius} />
                        {!isCenter && tagIds.length > 0 && (
                          <g className="literature-ticks" data-tick-count={tagIds.length}>
                            {tagIds.slice(0, MAX_NODE_TICKS).map((tagId, tickIndex, shown) => (
                              <circle
                                key={tagId}
                                className="literature-tick"
                                data-tag-id={tagId}
                                r={2.6}
                                cx={(tickIndex - (shown.length - 1) / 2) * 6.5}
                                cy={-(radius + 5)}
                                fill={tagsById.get(tagId)?.color ?? FALLBACK_TAG_COLOR}
                              />
                            ))}
                            {tagIds.length > MAX_NODE_TICKS && (
                              <text className="literature-tick-more" x={(MAX_NODE_TICKS / 2) * 6.5 + 1} y={-(radius + 2.5)}>+{tagIds.length - MAX_NODE_TICKS}</text>
                            )}
                          </g>
                        )}
                        {visibleLabels.has(placement.id)
                          ? (
                            <text className="literature-label" x={labelX} y={labelY} textAnchor={anchor}>
                              {workLabel(work, isActive || isCenter ? FULL_LABEL_LIMIT : LABEL_LIMIT)}
                            </text>
                          )
                          : <title>{workLabel(work, FULL_LABEL_LIMIT)}</title>}
                      </g>
                    );
                  })}
                </svg>
                {walkNotice !== null && <p className="literature-walk-notice" role="status" data-walk-notice="true">{walkNotice}</p>}
                <LiteratureLegend
                  graph={graph.neighbors}
                  placementCount={placements.length - 1}
                  expanded={expanded}
                  clusterLabels={clusterLabels.current}
                  onToggleExpansion={toggleExpansion}
                  tags={tags}
                  tagFilter={tagFilter}
                  onToggleTag={toggleTagFilter}
                  onClearTagFilter={clearTagFilter}
                  error={neighborsError}
                />
              </section>

              {workPanelOpen && activeWork !== undefined && (
                <WorkPanel
                  work={activeWork}
                  isCenter={activeWork.id === graph.work.id}
                  mark={markFor(activeWork.id)}
                  prominence={merged.prominence[activeWork.id]}
                  markError={markError}
                  onSetMark={(next, note) => applyMark(activeWork.id, next, note)}
                  tags={tags}
                  assignedTagIds={merged.workTags[activeWork.id] ?? []}
                  expansionView={merged.expansions[activeWork.id]}
                  isExpanded={walk.some((expansion) => expansion.parentId === activeWork.id)}
                  isExpanding={expanding.has(activeWork.id)}
                  onToggleExpansion={() => toggleWalk(activeWork.id)}
                  tagError={tagError}
                  onAssignTag={(name, color) => assignTagByName(activeWork.id, name, color)}
                  onUnassignTag={(tag) => unassignTag(activeWork.id, tag)}
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
  isCenter: boolean;
  mark: MarkView | null;
  prominence: ProminenceView | undefined;
  markError: string | null;
  onSetMark: (mark: WorkMarkName | null, note: string | null) => void;
  tags: readonly TagView[];
  assignedTagIds: readonly string[];
  tagError: string | null;
  onAssignTag: (name: string, color: string) => void;
  onUnassignTag: (tag: TagView) => void;
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
  expansionView: ExpansionView | undefined;
  isExpanded: boolean;
  isExpanding: boolean;
  onToggleExpansion: () => void;
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
      {!props.isCenter && (
        <LiteratureCuration
          key={work.id}
          mark={props.mark}
          prominence={props.prominence}
          error={props.markError}
          onSetMark={props.onSetMark}
          tags={props.tags}
          assignedTagIds={props.assignedTagIds}
          tagError={props.tagError}
          onAssignTag={props.onAssignTag}
          onUnassignTag={props.onUnassignTag}
        />
      )}
      {!props.isCenter && (
        <section className="literature-expansion" aria-label="References">
          <button type="button" className="literature-action" disabled={props.isExpanding} onClick={props.onToggleExpansion}>
            {props.isExpanding ? "Expanding…" : props.isExpanded ? "Collapse references" : "Expand references"} <kbd>w</kbd>
          </button>
          {props.expansionView?.state === "unavailable" && (
            <p className="literature-hint" role="status">
              References are unavailable{props.expansionView.unavailable_reason === null ? "" : `: ${relationName(props.expansionView.unavailable_reason)}`}.
            </p>
          )}
        </section>
      )}
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
