/** A short final crop cannot always align to the top of the scroll pane.
 * Choose the page occupying the most reading space, ignoring the sticky toolbar.
 */
export function visiblePdfPage(host: HTMLElement): number | null {
  const viewport = host.getBoundingClientRect();
  const top = viewport.top + (host.querySelector(".pdf-toolbar")?.clientHeight ?? 0);
  let page: number | null = null, largest = 0;
  for (const frame of host.querySelectorAll<HTMLElement>("[data-pdf-page]")) {
    const rect = frame.getBoundingClientRect();
    const visible = Math.max(0, Math.min(rect.bottom, viewport.bottom) - Math.max(rect.top, top))
      * Math.max(0, Math.min(rect.right, viewport.right) - Math.max(rect.left, viewport.left));
    if (visible > largest) { largest = visible; page = Number(frame.dataset.pdfPage); }
  }
  return page;
}
