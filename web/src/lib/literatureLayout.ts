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
