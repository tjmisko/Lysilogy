import { useEffect, useRef } from "react";

/** True when the event comes from a text field, where letters are typing and not commands. */
export function isTypingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
}

/** Which way a movement key goes: ArrowUp/Down and Ctrl-p/n always; k/j and h/l only outside a text field. */
export function dialogDirection(event: KeyboardEvent): -1 | 1 | null {
  const typing = isTypingTarget(event.target);
  if (event.key === "ArrowUp" || (event.ctrlKey && (event.key === "p" || event.key === "k"))) return -1;
  if (event.key === "ArrowDown" || (event.ctrlKey && (event.key === "n" || event.key === "j"))) return 1;
  if (!typing && !event.ctrlKey && !event.metaKey && !event.altKey) {
    if (event.key === "k") return -1;
    if (event.key === "j") return 1;
  }
  return null;
}

/**
 * Own the keyboard while a dialog is open. `handler` returns true for keys it used; those never
 * reach the page underneath. Registered on the capture phase of window so no page handler wins.
 */
export function useDialogKeys(handler: (event: KeyboardEvent) => boolean): void {
  const latest = useRef(handler);
  useEffect(() => { latest.current = handler; });
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.isComposing) return;
      if (latest.current(event)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        return;
      }
      // Anything else must not leak into the page below, except typing in the dialog's own field.
      if (!isTypingTarget(event.target)) event.stopImmediatePropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
}
