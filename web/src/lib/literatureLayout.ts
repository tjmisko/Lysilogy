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

export const HOP_RADIUS = 96;
const HOP_SPREAD_RADIANS = Math.PI / 3;
const VIEWBOX_PADDING = 40;

export type WalkPlacement = RankedPlacement & {
  kind: "work";
  /** Hops from the center (2 for a child of a base node). */
  hop: number;
  parentId: string;
};

export type WalkLink = { from: string; to: string };

export type WalkBounds = { minX: number; minY: number; maxX: number; maxY: number };

export type WalkLayout = {
  placements: WalkPlacement[];
  walkEdges: WalkLink[];
  crossLinks: WalkLink[];
  /** Box around the center, the base placements, and the walk children. */
  bounds: WalkBounds;
};

type Point = { x: number; y: number };

function boundsOf(points: readonly Point[], center: Point): WalkBounds {
  const bounds = { minX: center.x, minY: center.y, maxX: center.x, maxY: center.y };
  for (const point of points) {
    bounds.minX = Math.min(bounds.minX, point.x);
    bounds.minY = Math.min(bounds.minY, point.y);
    bounds.maxX = Math.max(bounds.maxX, point.x);
    bounds.maxY = Math.max(bounds.maxY, point.y);
  }
  return bounds;
}

/**
 * Place each expansion's new children on an arc of `HOP_RADIUS` around their
 * parent, within sixty degrees either side of the direction pointing away from
 * the parent's own parent (the center for a base node). Children keep the order
 * given. A child that is already placed (base, earlier expansion, or the center)
 * gets a cross link instead of a second dot. An expansion whose parent is not
 * placed is ignored. The result depends only on the arguments.
 */
export function layoutWalk(
  base: readonly RankedPlacement[],
  expansions: readonly { parentId: string; childIds: readonly string[]; depth: number }[],
  centerPoint: Point,
): WalkLayout {
  const centerId = base.find((placement) => placement.kind === "center")?.id;
  const positions = new Map<string, Point & { walkParent: Point }>();
  for (const placement of base) {
    if (placement.kind === "cluster") continue;
    positions.set(placement.id, { x: placement.x, y: placement.y, walkParent: centerPoint });
  }
  const placements: WalkPlacement[] = [];
  const walkEdges: WalkLink[] = [];
  const crossLinks: WalkLink[] = [];

  for (const expansion of expansions) {
    const parent = positions.get(expansion.parentId);
    if (parent === undefined || expansion.parentId === centerId) continue;
    const away = { x: parent.x - parent.walkParent.x, y: parent.y - parent.walkParent.y };
    const outward = Math.hypot(away.x, away.y) < 1e-9 ? -Math.PI / 2 : Math.atan2(away.y, away.x);
    const fresh: string[] = [];
    for (const childId of expansion.childIds) {
      if (childId === expansion.parentId) continue;
      if (positions.has(childId)) {
        if (!crossLinks.some((link) => link.from === expansion.parentId && link.to === childId)) {
          crossLinks.push({ from: expansion.parentId, to: childId });
        }
        continue;
      }
      if (fresh.includes(childId)) continue;
      fresh.push(childId);
      positions.set(childId, { x: 0, y: 0, walkParent: parent });
    }
    fresh.forEach((childId, index) => {
      const offset = fresh.length === 1 ? 0 : -HOP_SPREAD_RADIANS + (2 * HOP_SPREAD_RADIANS * index) / (fresh.length - 1);
      const angle = outward + offset;
      const point = { x: round(parent.x + HOP_RADIUS * Math.cos(angle)), y: round(parent.y + HOP_RADIUS * Math.sin(angle)) };
      positions.set(childId, { ...point, walkParent: parent });
      placements.push({
        id: childId, ...point, ring: expansion.depth, kind: "work", hop: expansion.depth, parentId: expansion.parentId,
      });
      walkEdges.push({ from: expansion.parentId, to: childId });
    });
  }

  const baseWorks = base.filter((placement) => placement.kind !== "cluster");
  return { placements, walkEdges, crossLinks, bounds: boundsOf([...baseWorks, ...placements], centerPoint) };
}

/** The SVG viewBox: the canvas, grown to hold `bounds`, with padding where it grew. */
export function walkViewBox(canvas: FanoutSize, bounds: WalkBounds): string {
  const minX = bounds.minX < 0 ? bounds.minX - VIEWBOX_PADDING : 0;
  const minY = bounds.minY < 0 ? bounds.minY - VIEWBOX_PADDING : 0;
  const maxX = bounds.maxX > canvas.width ? bounds.maxX + VIEWBOX_PADDING : canvas.width;
  const maxY = bounds.maxY > canvas.height ? bounds.maxY + VIEWBOX_PADDING : canvas.height;
  return `${round(minX)} ${round(minY)} ${round(maxX - minX)} ${round(maxY - minY)}`;
}
