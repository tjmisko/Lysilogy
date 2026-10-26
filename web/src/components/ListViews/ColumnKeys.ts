import { useEffect, useRef } from "react";

import { followsPrefix } from "../../lib/listViewNav";

/** Keys that open a page key sequence (zo, dd, gl ...) and how long the page waits for the next key. */
const SEQUENCE_PREFIXES: ReadonlySet<string> = new Set(["z", "d", "g"]);
const SEQUENCE_WINDOW_MS = 800;

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** Binds `h` / `l` (and the arrow keys) for a column view while the page's keys are live. */
export function useColumnKeys(enabled: boolean, move: (delta: -1 | 1) => string | null, select: (cursorId: string) => void): void {
  const prefixAt = useRef<number | null>(null);
  useEffect(() => {
    if (!enabled) return;
    const onKey = (event: KeyboardEvent): void => {
      // Track the page's pending prefix from the keys themselves, so handler order does not matter.
      const now = performance.now();
      const afterPrefix = followsPrefix(prefixAt.current, now, SEQUENCE_WINDOW_MS);
      prefixAt.current = SEQUENCE_PREFIXES.has(event.key) && !afterPrefix ? now : null;
      if (afterPrefix) return;
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
