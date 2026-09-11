type PageCrop = { x: number; y: number; width: number; height: number };
export type PageSnapshot = { page: number; canvas: HTMLCanvasElement; rect: DOMRect; filter: string; crop?: PageCrop };
const animations = new Set<Animation>();

function pageCrop(element: HTMLElement): PageCrop | undefined {
  const value = element.closest<HTMLElement>("[data-page-crop]")?.dataset.pageCrop;
  return value === undefined ? undefined : JSON.parse(value) as PageCrop;
}

function croppedRect(rect: DOMRect, crop: PageCrop): DOMRect {
  return new DOMRect(rect.left + rect.width * crop.x, rect.top + rect.height * crop.y,
    rect.width * crop.width, rect.height * crop.height);
}

export function capturePages(pages: number[], fromReader = false): PageSnapshot[] {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return [];
  return pages.slice(0, 6).flatMap((page) => {
    const selector = fromReader ? `.section-focus [data-pdf-page="${page}"] canvas` : `.source-page[data-page="${page}"] canvas`;
    const original = document.querySelector<HTMLCanvasElement>(selector);
    if (original === null || original.dataset.rendered !== "true" || original.width === 0 || original.height === 0) return [];
    const rect = original.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0 || rect.bottom < 0 || rect.top > window.innerHeight) return [];
    const canvas = document.createElement("canvas");
    canvas.width = original.width; canvas.height = original.height;
    canvas.getContext("2d")?.drawImage(original, 0, 0);
    return [{ page, canvas, rect, filter: getComputedStyle(original).filter, crop: fromReader ? pageCrop(original) : undefined }];
  });
}

export function animatePages(snapshots: PageSnapshot[], intoReader: boolean): () => void {
  for (const animation of animations) animation.cancel();
  animations.clear();
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return () => {};
  const cleanup: Array<() => void> = [];
  snapshots.forEach(({ page, canvas, rect, filter, crop }, index) => {
    const destination = document.querySelector<HTMLElement>(intoReader
      ? `.section-focus [data-pdf-page="${page}"] .pdf-page-surface`
      : `.source-page[data-page="${page}"] canvas`);
    if (destination === null) return;
    let target = destination.getBoundingClientRect();
    if (target.width === 0 || target.height === 0) return;
    const enteringCrop = intoReader ? pageCrop(destination) : undefined;
    if (enteringCrop !== undefined) {
      const source = canvas;
      canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(source.width * enteringCrop.width));
      canvas.height = Math.max(1, Math.round(source.height * enteringCrop.height));
      canvas.getContext("2d")?.drawImage(source, source.width * enteringCrop.x, source.height * enteringCrop.y,
        source.width * enteringCrop.width, source.height * enteringCrop.height, 0, 0, canvas.width, canvas.height);
      rect = croppedRect(rect, enteringCrop);
    } else if (!intoReader && crop !== undefined) target = croppedRect(target, crop);
    Object.assign(canvas.style, { position: "fixed", zIndex: "75", pointerEvents: "none", left: `${target.left}px`, top: `${target.top}px`,
      width: `${target.width}px`, height: `${target.height}px`, transformOrigin: "top left", filter, boxShadow: "0 12px 40px #0008" });
    canvas.className = "page-transition-snapshot";
    canvas.setAttribute("aria-hidden", "true");
    document.body.append(canvas);
    const animation = canvas.animate([
      { transform: `translate(${rect.left - target.left}px, ${rect.top - target.top}px) scale(${rect.width / target.width}, ${rect.height / target.height})`, opacity: 1 },
      { transform: "none", opacity: 1, offset: .88 },
      { transform: "none", opacity: 0 },
    ], { duration: 420, delay: index * 30, easing: "cubic-bezier(.2,.75,.2,1)", fill: "both" });
    animations.add(animation);
    const remove = () => { animation.cancel(); animations.delete(animation); canvas.remove(); };
    void animation.finished.then(remove, remove);
    cleanup.push(remove);
  });
  return () => cleanup.forEach((remove) => remove());
}
