import { useEffect } from "react";

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** Binds `h` / `l` (and the arrow keys) for a column view while the page's keys are live. */
export function useColumnKeys(enabled: boolean, move: (delta: -1 | 1) => string | null, select: (cursorId: string) => void): void {
  useEffect(() => {
    if (!enabled) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey || isEditable(event.target)) return;
      const delta = event.key === "h" || event.key === "ArrowLeft" ? -1 : event.key === "l" || event.key === "ArrowRight" ? 1 : null;
      if (delta === null) return;
      event.preventDefault();
      const target = move(delta);
      if (target !== null) select(target);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled, move, select]);
}
