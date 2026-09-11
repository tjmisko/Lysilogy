import type { TextRect } from "../types";

/** CSS/canvas clipping doesn't remove text from native copy or accessibility.
 * Prune the PDF.js text layer too, preserving original item and character offsets.
 */
export function cropTextLayer(layer: HTMLElement, regions: TextRect[], scaleX: number, scaleY: number): void {
  const origin = layer.getBoundingClientRect();
  const contains = (rect: DOMRect) => regions.some((r) => rect.left >= origin.left + r.x_min * scaleX - .5
    && rect.right <= origin.left + r.x_max * scaleX + .5
    && (rect.top + rect.bottom) / 2 >= origin.top + r.y_min * scaleY
    && (rect.top + rect.bottom) / 2 <= origin.top + r.y_max * scaleY);
  for (const span of layer.querySelectorAll<HTMLElement>("span[data-text-index]")) {
    if (span.firstChild?.nodeType !== Node.TEXT_NODE || contains(span.getBoundingClientRect())) continue;
    const node = span.firstChild;
    const text = node.textContent ?? "";
    const range = document.createRange();
    const pieces: { start: number; end: number; rect: DOMRect }[] = [];
    for (let offset = 0; offset < text.length;) {
      const end = offset + ((text.codePointAt(offset) ?? 0) > 0xffff ? 2 : 1);
      range.setStart(node, offset); range.setEnd(node, end);
      const rect = range.getBoundingClientRect();
      if (contains(rect)) {
        const previous = pieces.at(-1);
        if (previous?.end === offset) previous.end = end;
        else pieces.push({ start: offset, end, rect });
      }
      offset = end;
    }
    range.setStart(node, 0);
    range.setEnd(node, Math.min(text.length, (text.codePointAt(0) ?? 0) > 0xffff ? 2 : 1));
    const original = range.getBoundingClientRect();
    const replacements = pieces.map(({ start, end, rect }) => {
      const piece = span.cloneNode(false) as HTMLElement;
      piece.textContent = text.slice(start, end);
      piece.dataset.textOffset = String(start);
      piece.style.left = `calc(${span.style.left || "0px"} + ${rect.left - original.left}px)`;
      piece.style.top = `calc(${span.style.top || "0px"} + ${rect.top - original.top}px)`;
      return piece;
    });
    span.replaceWith(...replacements);
  }
}
