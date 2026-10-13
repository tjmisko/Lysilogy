export type FanoutNode = {
  id: string;
  title?: string | null;
  year?: number | null;
};

export type FanoutSize = { width: number; height: number };

export type FanoutPlacement = {
  id: string;
  x: number;
  y: number;
  /** 0 for the center, then 1, 2, ... outward. */
  ring: number;
};

export const MAX_NODES_PER_RING = 36;
const EDGE_MARGIN = 56;
const INNER_RADIUS_FRACTION = 0.3;

function round(value: number): number {
  return Math.round(value * 100) / 100;
}

function compareNodes(left: FanoutNode, right: FanoutNode): number {
  const leftYear = left.year ?? Number.POSITIVE_INFINITY;
  const rightYear = right.year ?? Number.POSITIVE_INFINITY;
  if (leftYear !== rightYear) return leftYear < rightYear ? -1 : 1;
  const leftTitle = (left.title ?? "").trim().toLowerCase();
  const rightTitle = (right.title ?? "").trim().toLowerCase();
  // Untitled nodes sort after titled ones within the same year.
  if ((leftTitle === "") !== (rightTitle === "")) return leftTitle === "" ? 1 : -1;
  if (leftTitle !== rightTitle) return leftTitle < rightTitle ? -1 : 1;
  if (left.id === right.id) return 0;
  return left.id < right.id ? -1 : 1;
}

/**
 * Place the center at the middle and the other nodes on concentric rings.
 * Order is year, then title, so the layout does not depend on input order.
 * Duplicate ids collapse and the center id never appears as a neighbor.
 */
export function layoutFanout(
  centerId: string,
  nodes: readonly FanoutNode[],
  size: FanoutSize,
): FanoutPlacement[] {
  const centerX = size.width / 2;
  const centerY = size.height / 2;
  const placements: FanoutPlacement[] = [
    { id: centerId, x: round(centerX), y: round(centerY), ring: 0 },
  ];

  const seen = new Set<string>([centerId]);
  const ordered: FanoutNode[] = [];
  for (const node of [...nodes].sort(compareNodes)) {
    if (seen.has(node.id)) continue;
    seen.add(node.id);
    ordered.push(node);
  }
  if (ordered.length === 0) return placements;

  const ringCount = Math.ceil(ordered.length / MAX_NODES_PER_RING);
  const outerRadius = Math.max(1, Math.min(size.width, size.height) / 2 - EDGE_MARGIN);
  const innerRadius = outerRadius * INNER_RADIUS_FRACTION;
  const radiusStep = (outerRadius - innerRadius) / ringCount;

  for (let ring = 1; ring <= ringCount; ring += 1) {
    const members = ordered.slice((ring - 1) * MAX_NODES_PER_RING, ring * MAX_NODES_PER_RING);
    const radius = innerRadius + radiusStep * ring;
    const angleStep = (2 * Math.PI) / members.length;
    // Stagger alternate rings by half a step so labels do not stack radially.
    const startAngle = -Math.PI / 2 + (ring % 2 === 0 ? angleStep / 2 : 0);
    members.forEach((node, position) => {
      const angle = startAngle + angleStep * position;
      placements.push({
        id: node.id,
        x: round(centerX + radius * Math.cos(angle)),
        y: round(centerY + radius * Math.sin(angle)),
        ring,
      });
    });
  }
  return placements;
}

export type RankedPlacement = FanoutPlacement & {
  kind: "center" | "work" | "cluster";
  /** Set on cluster placements; their `id` is `cluster:<key>`. */
  clusterKey?: string;
};

export const CLUSTER_ID_PREFIX = "cluster:";

type RankedSlot = { id: string; kind: "work" | "cluster"; clusterKey?: string };

/**
 * Place the center, then the server-ranked Works in the order given (no sorting),
 * then the cluster nodes, on concentric rings of at most `MAX_NODES_PER_RING`.
 * Clusters take the slots after the last Work and open a new ring when the
 * outermost one is full. Duplicate ids and the center id are ignored.
 */
export function layoutRankedFanout(
  centerId: string,
  rankedIds: readonly string[],
  clusterKeys: readonly string[],
  size: FanoutSize,
): RankedPlacement[] {
  const centerX = size.width / 2;
  const centerY = size.height / 2;
  const placements: RankedPlacement[] = [
    { id: centerId, x: round(centerX), y: round(centerY), ring: 0, kind: "center" },
  ];

  const seen = new Set<string>([centerId]);
  const slots: RankedSlot[] = [];
  for (const id of rankedIds) {
    if (seen.has(id)) continue;
    seen.add(id);
    slots.push({ id, kind: "work" });
  }
  const seenClusters = new Set<string>();
  for (const key of clusterKeys) {
    if (seenClusters.has(key)) continue;
    seenClusters.add(key);
    slots.push({ id: `${CLUSTER_ID_PREFIX}${key}`, kind: "cluster", clusterKey: key });
  }
  if (slots.length === 0) return placements;

  const ringCount = Math.ceil(slots.length / MAX_NODES_PER_RING);
  const outerRadius = Math.max(1, Math.min(size.width, size.height) / 2 - EDGE_MARGIN);
  const innerRadius = outerRadius * INNER_RADIUS_FRACTION;
  const radiusStep = (outerRadius - innerRadius) / ringCount;

  for (let ring = 1; ring <= ringCount; ring += 1) {
    const members = slots.slice((ring - 1) * MAX_NODES_PER_RING, ring * MAX_NODES_PER_RING);
    const radius = innerRadius + radiusStep * ring;
    const angleStep = (2 * Math.PI) / members.length;
    const startAngle = -Math.PI / 2 + (ring % 2 === 0 ? angleStep / 2 : 0);
    members.forEach((slot, position) => {
      const angle = startAngle + angleStep * position;
      placements.push({
        id: slot.id,
        x: round(centerX + radius * Math.cos(angle)),
        y: round(centerY + radius * Math.sin(angle)),
        ring,
        kind: slot.kind,
        ...(slot.clusterKey === undefined ? {} : { clusterKey: slot.clusterKey }),
      });
    });
  }
  return placements;
}

export type LabelRequest = {
  id: string;
  x: number;
  y: number;
  /** Characters in the rendered label. */
  length: number;
  /** Which way the text runs from the dot, as the SVG `text-anchor`. */
  anchor: "start" | "end" | "middle";
  /** Font size in SVG units. */
  fontSize: number;
  /** Vertical offset of the text from the dot. */
  offsetY: number;
  /** Always shown (center, active, pinned); never hidden by a collision. */
  forced: boolean;
};

export type LabelRect = { left: number; top: number; right: number; bottom: number };

const LABEL_CHAR_WIDTH = 0.58;
const LABEL_DOT_GAP = 11;
const LABEL_PADDING = 2;

/** Estimated box of a label, slightly generous so real text stays inside it. */
export function labelRect(request: LabelRequest): LabelRect {
  const width = request.length * request.fontSize * LABEL_CHAR_WIDTH + 2 * LABEL_PADDING;
  const height = request.fontSize * 1.3 + 2 * LABEL_PADDING;
  const centerY = request.y + request.offsetY - request.fontSize * 0.3;
  const left = request.anchor === "start"
    ? request.x + LABEL_DOT_GAP
    : request.anchor === "end" ? request.x - LABEL_DOT_GAP - width : request.x - width / 2;
  return { left, top: centerY - height / 2, right: left + width, bottom: centerY + height / 2 };
}

function overlaps(left: LabelRect, right: LabelRect): boolean {
  return left.left < right.right && right.left < left.right && left.top < right.bottom && right.top < left.bottom;
}

/**
 * Decide which labels to draw so that none overlap. Forced labels and the
 * reserved rectangles (cluster pills) go first; the rest are taken in the
 * order given (rank order) and skipped when they would collide with one
 * already kept. A skipped Work keeps its dot, aria label, and tooltip.
 */
export function planVisibleLabels(
  requests: readonly LabelRequest[],
  reserved: readonly LabelRect[],
): Set<string> {
  const visible = new Set<string>();
  const taken: LabelRect[] = [...reserved];
  for (const request of requests) {
    if (!request.forced) continue;
    visible.add(request.id);
    taken.push(labelRect(request));
  }
  for (const request of requests) {
    if (request.forced) continue;
    const rect = labelRect(request);
    if (taken.some((other) => overlaps(rect, other))) continue;
    visible.add(request.id);
    taken.push(rect);
  }
  return visible;
}
