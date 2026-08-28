import { useEffect, useRef } from "react";

type TabPhaseOptions = {
  /**
   * True while an overlay defines its own Tab behaviour (the paper switcher
   * moves its selection, the command menu completes). Tab is still kept away
   * from the browser; it just is not read as a phase change.
   */
  overlayHandlesTab: boolean;
  onCycle: (delta: -1 | 1) => void;
};

/**
 * Tab selects the next reading phase and never moves browser focus.
 *
 * The listener sits on `document` in the capture phase so it sees the key
 * before any component handler and regardless of what currently holds focus —
 * an input, a roving-tabindex list item, a button, or the body. Cancelling the
 * default action here cancels it for the whole dispatch, so focus traversal
 * cannot happen even in app states where the other key handlers stand down.
 */
export function useTabPhase(options: TabPhaseOptions): void {
  const optionsRef = useRef(options);

  useEffect(() => {
    optionsRef.current = options;
  }, [options]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== "Tab") return;
      // Ctrl/Cmd/Alt+Tab belong to the browser and the window manager.
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      event.preventDefault();
      const current = optionsRef.current;
      if (current.overlayHandlesTab) return;
      current.onCycle(event.shiftKey ? -1 : 1);
    };

    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, []);
}
