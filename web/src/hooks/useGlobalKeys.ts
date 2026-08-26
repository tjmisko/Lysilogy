import { useEffect, useRef } from "react";

type GlobalKeyOptions = {
  enabled: boolean;
  activeIndex: number;
  itemCount: number;
  view: "abstract" | "overview" | "glossary" | "text";
  textMode: "markdown" | "pdf";
  onMove: (index: number) => void;
  onOpen: () => void;
  onDigest: () => void;
  onGloss: () => void;
  onHelp: () => void;
  onSearch: () => void;
  onToggleLibrary: () => void;
  onToggleView: () => void;
  onToggleMarkdown: () => void;
  onToggleSpread: () => void;
  onEscape: () => void;
  onPrevious: () => void;
  onNext: () => void;
  onInvert: () => void;
  onToggleAiHighlights: () => void;
  onToggleUserHighlights: () => void;
  onToggleMarkMode: () => void;
  onZoom: (delta: number) => void;
  onScroll: (delta: number) => void;
  onScrollTo: (edge: "start" | "end") => void;
  onPage: (direction: -1 | 1, distance: "half" | "full") => void;
};

function isEditable(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

function columnCount(): number {
  if (window.innerWidth >= 1500) return 5;
  if (window.innerWidth >= 1100) return 4;
  if (window.innerWidth >= 720) return 3;
  return 1;
}

export function useGlobalKeys(options: GlobalKeyOptions): void {
  const optionsRef = useRef(options);
  const pendingG = useRef<number | null>(null);

  useEffect(() => {
    optionsRef.current = options;
  }, [options]);

  useEffect(() => {
    const clearPendingG = (): void => {
      if (pendingG.current !== null) {
        window.clearTimeout(pendingG.current);
        pendingG.current = null;
      }
    };

    const onKeyDown = (event: KeyboardEvent): void => {
      const current = optionsRef.current;
      if (!current.enabled) return;
      if (event.key === "Escape") {
        clearPendingG();
        current.onEscape();
        return;
      }
      if (isEditable(event.target) || event.metaKey || event.altKey) return;
      if (event.ctrlKey) {
        if (event.key === "d" || event.key === "u") {
          event.preventDefault();
          current.onPage(event.key === "d" ? 1 : -1, "half");
        }
        return;
      }

      const columns = columnCount();
      const clamp = (index: number): number =>
        Math.max(0, Math.min(current.itemCount - 1, index));
      switch (event.key) {
        case "h":
        case "ArrowLeft":
          if (current.view === "overview") {
            event.preventDefault();
            current.onMove(clamp(current.activeIndex - 1));
          } else if (current.view === "text" && current.textMode === "pdf") {
            event.preventDefault();
            current.onPrevious();
          }
          break;
        case "l":
        case "ArrowRight":
          if (current.view === "overview") {
            event.preventDefault();
            current.onMove(clamp(current.activeIndex + 1));
          } else if (current.view === "text" && current.textMode === "pdf") {
            event.preventDefault();
            current.onNext();
          }
          break;
        case "j":
        case "ArrowDown":
          event.preventDefault();
          if (current.view === "text" || current.view === "abstract") current.onScroll(120);
          else if (current.view === "overview") current.onMove(clamp(current.activeIndex + columns));
          break;
        case "k":
        case "ArrowUp":
          event.preventDefault();
          if (current.view === "text" || current.view === "abstract") current.onScroll(-120);
          else if (current.view === "overview") current.onMove(clamp(current.activeIndex - columns));
          break;
        case "G":
          event.preventDefault();
          clearPendingG();
          if (
            current.view === "abstract"
            || (current.view === "text" && current.textMode === "markdown")
          ) current.onScrollTo("end");
          else if (current.view === "overview") current.onMove(Math.max(0, current.itemCount - 1));
          break;
        case "g":
          event.preventDefault();
          if (pendingG.current !== null) {
            clearPendingG();
            if (
              current.view === "abstract"
              || (current.view === "text" && current.textMode === "markdown")
            ) current.onScrollTo("start");
            else if (current.view === "overview") current.onMove(0);
          } else {
            pendingG.current = window.setTimeout(() => {
              pendingG.current = null;
              optionsRef.current.onGloss();
            }, 420);
          }
          break;
        case "Enter":
        case "o":
          if (current.view === "overview") {
            event.preventDefault();
            current.onOpen();
          }
          break;
        case "d":
          if (current.view === "overview") {
            event.preventDefault();
            current.onDigest();
          }
          break;
        case "?":
          event.preventDefault();
          current.onHelp();
          break;
        case "/":
          event.preventDefault();
          current.onSearch();
          break;
        case "b":
          event.preventDefault();
          current.onToggleLibrary();
          break;
        case "p":
          event.preventDefault();
          current.onToggleView();
          break;
        case "m":
          event.preventDefault();
          current.onToggleMarkdown();
          break;
        case "2":
          if (current.view === "text" && current.textMode === "pdf") {
            event.preventDefault();
            current.onToggleSpread();
          }
          break;
        case "[":
          event.preventDefault();
          current.onPrevious();
          break;
        case "]":
          event.preventDefault();
          current.onNext();
          break;
        case "I":
          event.preventDefault();
          current.onInvert();
          break;
        case "H":
          if (current.view === "overview") {
            event.preventDefault();
            current.onToggleAiHighlights();
          }
          break;
        case "U":
          if (current.view === "overview") {
            event.preventDefault();
            current.onToggleUserHighlights();
          }
          break;
        case "v":
          if (current.view === "overview") {
            event.preventDefault();
            current.onToggleMarkMode();
          }
          break;
        case "=":
        case "+":
          if (current.view === "text" && current.textMode === "pdf") {
            event.preventDefault();
            current.onZoom(0.1);
          }
          break;
        case "-":
          if (current.view === "text" && current.textMode === "pdf") {
            event.preventDefault();
            current.onZoom(-0.1);
          }
          break;
        case "PageDown":
          event.preventDefault();
          current.onPage(1, "full");
          break;
        case "PageUp":
          event.preventDefault();
          current.onPage(-1, "full");
          break;
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      clearPendingG();
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);
}
