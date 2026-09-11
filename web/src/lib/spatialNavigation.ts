export type Direction = "left" | "right" | "up" | "down";
export type SpatialBox = { section: number; left: number; right: number; top: number; bottom: number };

/** Navigate rendered regions, including fragments of a section on different rows. */
export function spatialNeighbor(boxes: SpatialBox[], origin: number, direction: Direction): number | null {
  const from = boxes[origin];
  if (from === undefined) return null;
  const vertical = direction === "up" || direction === "down";
  const sign = direction === "up" || direction === "left" ? -1 : 1;
  const axes = (box: SpatialBox) => vertical
    ? { near: box.top, far: box.bottom, low: box.left, high: box.right }
    : { near: box.left, far: box.right, low: box.top, high: box.bottom };
  const source = axes(from), cross = (source.low + source.high) / 2;
  const primary = (source.near + source.far) / 2;
  let best: number | null = null, bestScore = Infinity;
  boxes.forEach((box, index) => {
    if (box.section === from.section) return;
    const target = axes(box), center = (target.near + target.far) / 2;
    if (sign * (center - primary) <= 1) return;
    const forwardGap = Math.max(0, sign > 0 ? target.near - source.far : source.near - target.far);
    const crossGap = Math.max(0, target.low - cross, cross - target.high);
    const score = forwardGap + crossGap * 3 + Math.abs((target.low + target.high) / 2 - cross) * .01;
    if (score < bestScore) { best = index; bestScore = score; }
  });
  return best;
}

export function moveInSectionMap(activeSection: number, direction: Direction): number | null {
  const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>(
    ".section-boxes:not(.is-disabled) button[data-section-index]",
  )).filter((button) => button.getBoundingClientRect().width > 0);
  const boxes = buttons.map((button) => ({ section: Number(button.dataset.sectionIndex), ...rect(button) }));
  let origin = buttons.findIndex((button) => button === document.activeElement && Number(button.dataset.sectionIndex) === activeSection);
  if (origin < 0) origin = boxes.findIndex((box) => box.section === activeSection);
  const next = spatialNeighbor(boxes, origin, direction);
  if (next === null) return null;
  buttons[next]?.focus({ preventScroll: true });
  buttons[next]?.scrollIntoView({ block: "nearest", inline: "nearest" });
  return boxes[next]?.section ?? null;
}

function rect(button: HTMLElement) {
  const { left, right, top, bottom } = button.getBoundingClientRect();
  return { left, right, top, bottom };
}
