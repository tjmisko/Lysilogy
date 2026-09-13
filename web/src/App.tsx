import { flushSync } from "react-dom";
import { NotesPanel } from "./components/NotesPanel";
import { SectionFocus } from "./components/SectionFocus";
import { capturePages, animatePages, type PageSnapshot } from "./lib/pageTransition";
import { sectionPages } from "./lib/sectionScope";
import { handleNotesPaneKey } from "./lib/notesPaneKeys";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { AbstractView } from "./components/AbstractView";
import { PaperHeading } from "./components/PaperHeading";
import { CommandMenu } from "./components/CommandMenu";
import { PassageQuestion } from "./components/PassageQuestion";
import { DigestPanel } from "./components/DigestPanel";
import { ExperimentPanel } from "./components/ExperimentPanel";
import { GlossaryView } from "./components/GlossPanel";
import { HelpOverlay } from "./components/HelpOverlay";
import { HomePage } from "./components/HomePage";
import { LibraryRail } from "./components/LibraryRail";
import { MarkdownReader } from "./components/MarkdownReader";
import { PaperSwitcher } from "./components/PaperSwitcher";
import { PdfReader } from "./components/PdfReader";
import { QueuePanel } from "./components/QueuePanel";
import { ReaderToolsPanel } from "./components/ReaderToolsPanel";
import { SectionAtlas } from "./components/SectionAtlas";
import { useGlobalKeys } from "./hooks/useGlobalKeys";
import { useTabPhase } from "./hooks/useTabPhase";
import { api } from "./lib/api";
import type {
  AnalysisProvider,
  Clarification,
  Highlight,
  LayoutSentence,
  LibraryResponse,
  PaperMap,
  PaperSection,
  PaperView,
  ProcessingQueue,
} from "./types";

type Panel = "digest" | "help" | null;
type ViewMode = "abstract" | "overview" | "glossary" | "text";
type TextMode = "markdown" | "pdf";

const VIEW_ORDER: ViewMode[] = ["abstract", "overview", "glossary", "text"];
const VIEW_STORAGE_KEY = "lysilogy.reader-view";

const PROCESSING_STATES = new Set(["queued", "extracting", "analyzing"]);

function isEditableTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

function focusReaderPane(): void {
  const reader = document.querySelector<HTMLElement>(".section-focus .pdf-reader")
    ?? document.querySelector<HTMLElement>(".main-stage .pdf-reader")
    ?? document.querySelector<HTMLElement>(".section-boxes button.is-active")
    ?? document.querySelector<HTMLElement>(".main-stage");
  reader?.focus({ preventScroll: true });
}

function initialPaperId(): string | null {
  const params = new URLSearchParams(window.location.hash.replace(/^#/u, ""));
  return params.get("paper");
}

function initialView(): ViewMode {
  try {
    const stored = window.localStorage.getItem(VIEW_STORAGE_KEY);
    return VIEW_ORDER.includes(stored as ViewMode) ? stored as ViewMode : "abstract";
  } catch {
    return "abstract";
  }
}

export function App() {
  const [library, setLibrary] = useState<LibraryResponse | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(initialPaperId);
  const selectedIdRef = useRef(selectedId);
  const paperRequest = useRef(0);
  const appliedPaperRequest = useRef(0);
  useEffect(() => { selectedIdRef.current = selectedId; }, [selectedId]);
  const [paperView, setPaperView] = useState<PaperView | null>(null);
  const [pageSnapshots, setPageSnapshots] = useState<PageSnapshot[]>([]);
  const focusRestore = useRef<{ library: boolean; scroll: number } | null>(null);
  const [activeSection, setActiveSection] = useState(0);
  const [refreshingComponent, setRefreshingComponent] = useState<"abstract" | "context" | "structure" | null>(null);
  const [panel, setPanel] = useState<Panel>(null);
  const [preferredView, setView] = useState<ViewMode>(initialView);
  const view: ViewMode = paperView?.analysis == null ? "text" : preferredView;
  const [textMode, setTextMode] = useState<TextMode>("pdf");
  const [compactLayout, setCompactLayout] = useState(() => window.innerWidth < 1180);
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [queueOpen, setQueueOpen] = useState(false);
  const [experimentOpen, setExperimentOpen] = useState(false);
  const [toolsTab, setToolsTab] = useState<"supercut" | "references" | null>(null);
  const [referenceSeed, setReferenceSeed] = useState<{ text: string; page: number } | null>(null);
  const [focusQueueFeedback, setFocusQueueFeedback] = useState(false);
  const [queue, setQueue] = useState<ProcessingQueue>({ jobs: [] });
  const [libraryQuery, setLibraryQuery] = useState("");
  const [homeQuery, setHomeQuery] = useState("");
  const [homeActiveId, setHomeActiveId] = useState<string | null>(null);
  const [provider, setProvider] = useState<AnalysisProvider>("codex");
  const [pdfPage, setPdfPage] = useState(1);
  const [pdfPages, setPdfPages] = useState(1);
  const [pdfZoom, setPdfZoom] = useState(1);
  const [pdfSpread, setPdfSpread] = useState(false);
  const [darkInk, setDarkInk] = useState(true);
  const [paperMap, setPaperMap] = useState<PaperMap | null>(null);
  const [mapLoading, setMapLoading] = useState(false);
  const [showAiHighlights, setShowAiHighlights] = useState(true);
  const [showUserHighlights, setShowUserHighlights] = useState(true);
  const [markMode, setMarkMode] = useState(false);
  const [clarifySeed, setClarifySeed] = useState("");
  const [sourceQuestion, setSourceQuestion] = useState<{ text: string; page: number } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [sidebarPaperIds, setSidebarPaperIds] = useState<string[]>([]);
  const searchRef = useRef<HTMLInputElement>(null);
  const mainStageRef = useRef<HTMLElement>(null);
  const closeReaderTools = useCallback(() => setToolsTab(null), []);
  const home = selectedId === null;
  const [notesOpen, setNotesOpen] = useState(false);
  const notesRailRestore = useRef<boolean | null>(null);
  const openNotes = useCallback(() => {
    notesRailRestore.current = libraryOpen;
    setLibraryOpen(false);
    setNotesOpen(true);
  }, [libraryOpen]);
  const closeNotes = useCallback(() => {
    setNotesOpen(false);
    if (notesRailRestore.current !== null) setLibraryOpen(notesRailRestore.current);
    notesRailRestore.current = null;
    requestAnimationFrame(focusReaderPane);
  }, []);
  const [toolbarPinned, setToolbarPinned] = useState(false);
  const [toolbarPeek, setToolbarPeek] = useState(false);
  const [topbarHeight, setTopbarHeight] = useState(0);
  const topbarRef = useRef<HTMLElement>(null);
  const pendingSourceSearch = useRef(false);
  const requestNotesClose = useCallback((afterClose?: () => void): boolean => {
    const editor = document.querySelector(".notes-panel");
    if (editor === null) return false;
    return !editor.dispatchEvent(new CustomEvent("notes-close", { cancelable: true, detail: { afterClose } }));
  }, []);

  useEffect(() => {
    mainStageRef.current?.scrollTo({ top: 0 });
  }, [selectedId, textMode, view]);

  useEffect(() => {
    try {
      window.localStorage.setItem(VIEW_STORAGE_KEY, preferredView);
    } catch {
      // The reader remains usable when storage is unavailable.
    }
  }, [preferredView]);

  const refreshLibrary = useCallback(async (): Promise<LibraryResponse> => {
    const next = await api.library();
    setLibrary(next);
    return next;
  }, []);

  const refreshQueue = useCallback(async (): Promise<ProcessingQueue> => {
    const next = await api.queue();
    setQueue(next);
    return next;
  }, []);

  const loadPaper = useCallback(async (id: string): Promise<PaperView> => {
    const request = ++paperRequest.current;
    const next = await api.paper(id);
    // A slower request may finish after navigation to another paper or home.
    if (selectedIdRef.current === id && request > appliedPaperRequest.current) {
      appliedPaperRequest.current = request;
      setPaperView(next);
    }
    setLibrary((current) => current === null ? null : { ...current,
      papers: current.papers.map((paper) => paper.id === id ? next.paper : paper) });
    return next;
  }, []);

  useEffect(() => {
    let cancelled = false;
    void api.queue().then((nextQueue) => {
      if (!cancelled) setQueue(nextQueue);
    }).catch(() => {
      // The library remains usable if an older backend does not expose queue state.
    });
    void api
      .library()
      .then((nextLibrary) => {
        if (cancelled) return;
        setLibrary(nextLibrary);
        setLoading(false);
        const requested = selectedIdRef.current;
        if (requested === null) return;
        if (!nextLibrary.papers.some((paper) => paper.id === requested)) {
          selectedIdRef.current = null;
          setSelectedId(null);
          setLibraryOpen(false);
          window.history.replaceState(null, "", "#home");
          setError("This paper is no longer in the library.");
          return;
        }
        void loadPaper(requested).catch((reason: unknown) => {
          if (!cancelled && selectedIdRef.current === requested) setError(reason instanceof Error ? reason.message : "Could not load paper");
        });
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : "Could not load library");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // The initial URL selection is intentionally read only once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onFunctionKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      if (event.key.length === 1 && document.querySelector('.pdf-reader[data-link-hints="true"]') !== null) return;
      if (event.target instanceof Element && event.target.closest(".notes-panel") !== null) return;
      if (event.key === "F1") {
        event.preventDefault();
        // F1 explicitly changes the rail preference instead of restoring it on exit.
        focusRestore.current = null;
        notesRailRestore.current = null;
        setPanel(null);
        setSwitcherOpen(false);
        setCommandOpen(false);
        setQueueOpen(false);
        setLibraryOpen((open) => !open);
      } else if (event.key === "F10") {
        event.preventDefault();
        setPanel(null);
        setCommandOpen(false);
        setQueueOpen(false);
        if (compactLayout) setLibraryOpen(false);
        setSwitcherOpen((open) => !open);
      } else if (event.key === ":" && !isEditableTarget(event.target)) {
        event.preventDefault();
        setPanel(null);
        setSwitcherOpen(false);
        setQueueOpen(false);
        if (compactLayout) setLibraryOpen(false);
        setCommandOpen(true);
      } else if (event.key === "Q" && !isEditableTarget(event.target)) {
        event.preventDefault();
        setSwitcherOpen(false);
        setCommandOpen(false);
        if (compactLayout) setLibraryOpen(false);
        setFocusQueueFeedback(false);
        setQueueOpen((open) => !open);
        void refreshQueue();
      }
    };
    window.addEventListener("keydown", onFunctionKey, true);
    return () => window.removeEventListener("keydown", onFunctionKey, true);
  }, [compactLayout, refreshQueue]);

  useEffect(() => {
    if (!notesOpen) return;
    const onPaneKey = (event: KeyboardEvent): void => { handleNotesPaneKey(event); };
    window.addEventListener("keydown", onPaneKey, true);
    return () => window.removeEventListener("keydown", onPaneKey, true);
  }, [notesOpen]);

  useEffect(() => {
    const query = window.matchMedia("(max-width: 1179px)");
    const updateLayout = (event: MediaQueryListEvent | MediaQueryList): void => {
      setCompactLayout(event.matches);
      if (event.matches) setLibraryOpen(false);
    };
    query.addEventListener("change", updateLayout);
    return () => query.removeEventListener("change", updateLayout);
  }, []);

  const processing =
    (paperView !== null && PROCESSING_STATES.has(paperView.paper.status.state)) ||
    queue.jobs.some(
      (job) =>
        job.paper_id === selectedId &&
        (job.status.state === "queued" || job.status.state === "running"),
    );
  const analysisNeedsRefresh = paperView?.analysis != null && paperView.analysis.schema_version < 4;
  const queueHasActive = queue.jobs.some(
    (job) => job.status.state === "queued" || job.status.state === "running",
  );

  useEffect(() => {
    if (!queueOpen && !queueHasActive && !processing) return;
    const timer = window.setInterval(() => {
      const requests: Array<Promise<unknown>> = [refreshQueue()];
      if (queueHasActive) requests.push(refreshLibrary());
      if (processing && selectedId !== null) {
        requests.push(loadPaper(selectedId));
      }
      void Promise.all(requests).catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not refresh analysis status");
      });
    }, 850);
    return () => window.clearInterval(timer);
  }, [loadPaper, processing, queueHasActive, queueOpen, refreshLibrary, refreshQueue, selectedId]);

  const selectPaper = useCallback(
    function navigateToPaper(id: string, updateHistory = true): void {
      if (id !== selectedIdRef.current && requestNotesClose(() => navigateToPaper(id, updateHistory))) return;
      if (id === selectedIdRef.current) {
        setLibraryOpen(false);
        return;
      }
      if (updateHistory) window.history.pushState(null, "", `#paper=${encodeURIComponent(id)}`);
      selectedIdRef.current = id;
      appliedPaperRequest.current = ++paperRequest.current;
      focusRestore.current = null;
      setActiveSection(0);
      setPanel(null);
      setSwitcherOpen(false);
      setCommandOpen(false);
      setQueueOpen(false);
      setExperimentOpen(false);
      setToolsTab(null);
      setClarifySeed("");
      setSourceQuestion(null);
      setPaperMap(null);
      setMapLoading(false);
      setMarkMode(false);
      setPdfPage(1);
      setView("abstract");
      setTextMode("pdf");
      setSelectedId(id);
      setPaperView(null);
      setError(null);
      setLibraryOpen(false);
      void loadPaper(id).catch((reason: unknown) => {
        if (selectedIdRef.current === id) setError(reason instanceof Error ? reason.message : "Could not load paper");
      });
    },
    [loadPaper, requestNotesClose],
  );

  const openHome = useCallback(function navigateHome(updateHistory = true): void {
    if (requestNotesClose(() => navigateHome(updateHistory))) return;
    if (updateHistory && window.location.hash !== "#home") window.history.pushState(null, "", "#home");
    if (selectedIdRef.current !== null) setHomeActiveId(selectedIdRef.current);
    selectedIdRef.current = null;
    appliedPaperRequest.current = ++paperRequest.current;
    focusRestore.current = null;
    setSelectedId(null);
    setPaperView(null);
    setPaperMap(null);
    setMapLoading(false);
    setActiveSection(0);
    setPanel(null);
    setClarifySeed("");
    setSourceQuestion(null);
    setLibraryOpen(false);
    setSwitcherOpen(false);
    setCommandOpen(false);
    setQueueOpen(false);
    setExperimentOpen(false);
    setToolsTab(null);
    setError(null);
  }, [requestNotesClose]);

  useEffect(() => {
    const readRoute = (): void => {
      const id = initialPaperId();
      // Browsers emit both events for a single back/forward hash traversal.
      if (id === selectedIdRef.current) return;
      const applyRoute = () => {
        if (id === null) openHome(false);
        else if (library !== null && !library.papers.some((paper) => paper.id === id)) {
          openHome(false);
          window.history.replaceState(null, "", "#home");
          setError("This paper is no longer in the library.");
        } else selectPaper(id, false);
      };
      const requestedHash = window.location.hash;
      if (requestNotesClose(() => { window.history.replaceState(null, "", requestedHash); applyRoute(); })) {
        // Keep the visible paper and URL together while an unsaved note awaits a decision.
        window.history.replaceState(null, "", selectedIdRef.current === null ? "#home" : `#paper=${encodeURIComponent(selectedIdRef.current)}`);
        return;
      }
      applyRoute();
    };
    window.addEventListener("popstate", readRoute);
    window.addEventListener("hashchange", readRoute);
    return () => {
      window.removeEventListener("popstate", readRoute);
      window.removeEventListener("hashchange", readRoute);
    };
  }, [library, openHome, requestNotesClose, selectPaper]);

  const selectFromSwitcher = useCallback(
    (id: string): void => {
      setSwitcherOpen(false);
      selectPaper(id);
    },
    [selectPaper],
  );

  const analyze = useCallback((chosenProvider: AnalysisProvider = provider): void => {
    if (selectedId === null || processing) return;
    setError(null);
    setNotice(
      `${analysisNeedsRefresh ? "Refresh queued" : "Queued"} for ${chosenProvider}. Press Q to watch the live tasklist.`,
    );
    void api
      .analyze(
        selectedId,
        chosenProvider,
        paperView?.paper.status.state === "failed" || paperView?.paper.status.state === "ready",
      )
      .then(() => Promise.all([loadPaper(selectedId), refreshLibrary(), refreshQueue()]))
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not start analysis");
      });
  }, [
    analysisNeedsRefresh,
    loadPaper,
    paperView?.paper.status.state,
    processing,
    provider,
    refreshLibrary,
    refreshQueue,
    selectedId,
  ]);

  const sections = useMemo(() => paperView?.analysis?.sections ?? [], [paperView?.analysis]);
  const selectedSection = sections[activeSection] ?? null;

  const openSection = useCallback((section?: PaperSection, index?: number): void => {
    const target = section ?? sections[activeSection];
    if (target === undefined) return;
    if (view === "overview") {
      const alreadyFocused = panel === "digest";
      if (!alreadyFocused) focusRestore.current = { library: libraryOpen, scroll: mainStageRef.current?.scrollTop ?? 0 };
      const count = paperMap?.layout.pages.length ?? Math.max(1, target.pages.end);
      setPageSnapshots(capturePages(sectionPages(target, count), alreadyFocused));
      setLibraryOpen(false);
    }
    if (index !== undefined) setActiveSection(index);
    setMarkMode(false);
    setClarifySeed("");
    setPanel("digest");
  }, [activeSection, libraryOpen, panel, paperMap, sections, view]);

  const closeSection = useCallback((): void => {
    const target = sections[activeSection];
    const snapshots = target === undefined ? [] : capturePages(sectionPages(target, paperMap?.layout.pages.length ?? target.pages.end), true);
    setClarifySeed("");
    setPanel(null);
    const restore = focusRestore.current;
    focusRestore.current = null;
    if (restore !== null) setLibraryOpen(restore.library);
    window.requestAnimationFrame(() => {
      if (restore !== null && mainStageRef.current !== null) mainStageRef.current.scrollTop = restore.scroll;
      const button = Array.from(document.querySelectorAll<HTMLButtonElement>(".section-boxes button"))
        .find((node) => node.dataset.sectionId === target?.id);
      button?.focus({ preventScroll: true });
      animatePages(snapshots, false);
    });
  }, [activeSection, paperMap, sections]);

  const openPage = useCallback((page: number): void => {
    focusRestore.current = null;
    setLibraryOpen(false);
    setPdfPage(page);
    setPanel(null);
    setTextMode("pdf");
    setView("text");
  }, []);

  const openGlossary = useCallback((): void => {
    setPanel(null);
    setView("glossary");
  }, []);

  const cycleView = useCallback((delta: -1 | 1): void => {
    setPanel(null);
    const index = VIEW_ORDER.indexOf(view);
    const nextIndex = (index + delta + VIEW_ORDER.length) % VIEW_ORDER.length;
    const next = VIEW_ORDER[nextIndex] ?? view;
    if (next === "text") setTextMode("pdf");
    setView(next);
  }, [view]);

  const movePaper = useCallback(
    (delta: number): void => {
      if (library === null || selectedId === null) return;
      const paperIds = sidebarPaperIds.length > 0
        ? sidebarPaperIds
        : library.papers.map((paper) => paper.id);
      const index = paperIds.indexOf(selectedId);
      const nextId = paperIds[Math.max(0, Math.min(paperIds.length - 1, index + delta))];
      if (nextId !== undefined && index >= 0) selectPaper(nextId);
    },
    [library, selectPaper, selectedId, sidebarPaperIds],
  );

  const previous = useCallback((): void => {
    if (view === "text" && textMode === "pdf") {
      setPdfPage((page) => Math.max(1, page - (pdfSpread ? 2 : 1)));
    }
    else movePaper(-1);
  }, [movePaper, pdfSpread, textMode, view]);
  const next = useCallback((): void => {
    if (view === "text" && textMode === "pdf") {
      setPdfPage((page) => Math.min(pdfPages, page + (pdfSpread ? 2 : 1)));
    }
    else movePaper(1);
  }, [movePaper, pdfPages, pdfSpread, textMode, view]);

  const scrollMarkdown = useCallback((delta: number): void => {
    mainStageRef.current?.scrollBy({ top: delta, behavior: "smooth" });
  }, []);
  const scrollMarkdownTo = useCallback((edge: "start" | "end"): void => {
    const stage = mainStageRef.current;
    if (stage === null) return;
    stage.scrollTo({ top: edge === "start" ? 0 : stage.scrollHeight, behavior: "smooth" });
  }, []);

  const pageReader = useCallback((direction: -1 | 1, distance: "half" | "full"): void => {
    if (view === "text" && textMode === "pdf") {
      const step = pdfSpread ? 2 : 1;
      setPdfPage((page) => Math.max(1, Math.min(pdfPages, page + direction * step)));
      return;
    }
    const stage = mainStageRef.current;
    if (stage === null) return;
    const fraction = distance === "half" ? 0.5 : 0.9;
    stage.scrollBy({ top: direction * stage.clientHeight * fraction, behavior: "smooth" });
  }, [pdfPages, pdfSpread, textMode, view]);

  const sendFeedback = useCallback(async (feedback: string): Promise<void> => {
    if (selectedId === null) throw new Error("No paper is selected");
    setError(null);
    try {
      await api.feedback(selectedId, feedback, provider);
      setNotice("Feedback queued. The reader will retry from the saved paper state.");
      await Promise.all([loadPaper(selectedId), refreshLibrary(), refreshQueue()]);
    } catch (reason: unknown) {
      const failure = reason instanceof Error ? reason : new Error("Could not queue feedback");
      setError(failure.message);
      throw failure;
    }
  }, [loadPaper, provider, refreshLibrary, refreshQueue, selectedId]);

  const refreshComponent = useCallback((component: "abstract" | "context" | "structure"): void => {
    if (selectedId === null || refreshingComponent !== null) return;
    setRefreshingComponent(component);
    setNotice(component === "structure" ? "Grouping the paper into coherent reading sections…" : component === "abstract" ? "Checking the authored abstract…" : "Researching the paper’s history and influence…");
    void api.refreshComponent(selectedId, provider, component).then((next) => {
      setPaperView((current) => current?.paper.id === next.paper.id ? next : current);
      if (component === "structure" && selectedIdRef.current === next.paper.id) setActiveSection(0);
      setNotice(component === "structure" ? "Section map refreshed." : component === "abstract" ? "Abstract refreshed." : "Historical context refreshed.");
    }).catch((reason: unknown) => setError(reason instanceof Error ? reason.message : "Refresh failed"))
      .finally(() => { setRefreshingComponent(null); void refreshQueue(); });
    void refreshQueue();
  }, [provider, refreshQueue, refreshingComponent, selectedId]);

  const executeCommand = useCallback((rawCommand: string): void => {
    const [name = "", argument, ...extra] = rawCommand.trim().toLocaleLowerCase().split(/\s+/u);
    setCommandOpen(false);
    if (extra.length > 0) {
      setError(`Too many arguments for :${name}`);
      return;
    }
    if (selectedId === null && ["analyze", "feedback", "experiment", "supercut", "references", "refresh-abstract", "refresh-context", "refresh-structure", "abstract", "overview", "atlas", "glossary", "text", "pdf", "spread"].includes(name)) {
      setError("Open a paper before using this command.");
      return;
    }
    switch (name) {
      case "home":
        openHome();
        break;
      case "analyze": {
        const chosen = argument ?? provider;
        if (chosen !== "codex" && chosen !== "claude" && chosen !== "heuristic") {
          setError(`Unknown reader “${chosen}”. Use codex, claude, or heuristic.`);
          return;
        }
        setProvider(chosen);
        analyze(chosen);
        break;
      }
      case "queue":
        setFocusQueueFeedback(false);
        setQueueOpen(true);
        void refreshQueue();
        break;
      case "feedback":
        setFocusQueueFeedback(true);
        setQueueOpen(true);
        void refreshQueue();
        break;
      case "experiment":
        if (selectedId === null) {
          setError("Select a paper before opening the prompt lab.");
        } else {
          setExperimentOpen(true);
        }
        break;
      case "supercut":
      case "references":
        setReferenceSeed(null);
        setToolsTab(name);
        break;
      case "library":
        setLibraryOpen((open) => !open);
        break;
      case "switch":
        setSwitcherOpen(true);
        break;
      case "refresh-abstract":
        refreshComponent("abstract");
        break;
      case "refresh-context":
        refreshComponent("context");
        break;
      case "refresh-structure":
        refreshComponent("structure");
        break;
      case "abstract":
        setView("abstract");
        break;
      case "overview":
      case "atlas":
        setView("overview");
        break;
      case "glossary":
        openGlossary();
        break;
      case "text":
        setTextMode("pdf");
        setView("text");
        break;
      case "pdf":
        setTextMode("pdf");
        setView("text");
        break;
      case "spread":
        setTextMode("pdf");
        setView("text");
        setPdfSpread((spread) => !spread);
        break;
      case "ink":
        setDarkInk((value) => !value);
        break;
      case "help":
        setPanel("help");
        break;
      default:
        setError(`Unknown command :${name}`);
    }
  }, [analyze, openGlossary, openHome, provider, refreshComponent, refreshQueue, selectedId]);

  useTabPhase({
    nativeTab: home,
    // The switcher and the command menu read Tab themselves; everywhere else
    // Tab steps through the reading phases of the current paper.
    overlayHandlesTab: home || switcherOpen || commandOpen || queueOpen || sourceQuestion !== null || experimentOpen || toolsTab !== null || paperView?.analysis == null || (panel === "digest" && view === "overview"),
    onCycle: (delta) => {
      setQueueOpen(false);
      if (compactLayout) setLibraryOpen(false);
      cycleView(delta);
    },
  });

  useGlobalKeys({
    enabled:
      !home && panel === null && sourceQuestion === null && view !== "glossary" && !switcherOpen && !commandOpen && !queueOpen && !experimentOpen && toolsTab === null &&
      !(compactLayout && libraryOpen),
    activeIndex: activeSection,
    itemCount: sections.length,
    view,
    textMode,
    onMove: setActiveSection,
    onOpen: () => {
      if (selectedSection !== null) openSection(selectedSection, activeSection);
    },
    onDigest: () => selectedSection !== null && openSection(selectedSection, activeSection),
    onGloss: () => paperView?.analysis != null && openGlossary(),
    onHelp: () => setPanel("help"),
    onSearch: () => {
      pendingSourceSearch.current = true;
      setPanel(null);
      setTextMode("pdf");
      setView("text");
    },
    onToggleLibrary: () => setLibraryOpen((open) => !open),
    onToggleView: () => {
      if (view === "text" && textMode === "pdf") {
        setView("overview");
      } else {
        setTextMode("pdf");
        setView("text");
      }
    },
    onToggleMarkdown: () => {
      if (view === "text" && textMode === "markdown") {
        setTextMode("pdf");
      } else {
        setTextMode("markdown");
        setView("text");
      }
    },
    onToggleSpread: () => setPdfSpread((spread) => !spread),
    onEscape: () => {
      setLibraryOpen(false);
      if (view !== "overview") setView("overview");
    },
    onPrevious: previous,
    onNext: next,
    onPreviousPaper: () => movePaper(-1),
    onNextPaper: () => movePaper(1),
    onInvert: () => setDarkInk((value) => !value),
    onToggleAiHighlights: () => setShowAiHighlights((value) => !value),
    onToggleUserHighlights: () => setShowUserHighlights((value) => !value),
    onToggleMarkMode: () => setMarkMode((value) => !value),
    onZoom: (delta) => setPdfZoom((value) => Math.max(0.5, Math.min(2.5, value + delta))),
    onScroll: scrollMarkdown,
    onScrollTo: scrollMarkdownTo,
    onPage: pageReader,
  });

  useEffect(() => {
    if (notice === null) return;
    const timer = window.setTimeout(() => setNotice(null), 4_000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const relatedClaims = useMemo(() => paperView?.analysis?.claims ?? [], [paperView]);
  const hasAnalysis = paperView?.analysis != null;
  const analysisGeneratedAt = paperView?.analysis?.generated_at ?? null;

  useEffect(() => {
    if (selectedId === null || !hasAnalysis) return;
    const controller = new AbortController();
    window.queueMicrotask(() => {
      if (!controller.signal.aborted) setMapLoading(true);
    });
    void api.paperMap(selectedId, controller.signal)
      .then((next) => {
        setPaperMap(next);
      })
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) {
          setError(reason instanceof Error ? reason.message : "Could not align source pages");
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setMapLoading(false);
      });
    return () => controller.abort();
  }, [analysisGeneratedAt, hasAnalysis, selectedId]);

  const toggleHighlight = useCallback((start: LayoutSentence, end?: LayoutSentence): void => {
    if (selectedId === null || paperMap === null) return;
    const last = end ?? start;
    const startToken = Math.min(start.start_token, last.start_token);
    const endToken = Math.max(start.end_token, last.end_token);
    const existing = paperMap.highlights.find((highlight): highlight is Highlight =>
      highlight.origin.type === "user"
        && highlight.anchor.page === start.page
        && highlight.anchor.start_token === startToken
        && highlight.anchor.end_token === endToken,
    );
    if (existing !== undefined) {
      void api.deleteHighlight(selectedId, existing.id)
        .then(() => {
          setPaperMap((current) => current === null ? null : {
            ...current,
            highlights: current.highlights.filter((highlight) => highlight.id !== existing.id),
          });
          setNotice("Reader highlight removed from highlights.jsonl.");
        })
        .catch((reason: unknown) => {
          setError(reason instanceof Error ? reason.message : "Could not remove highlight");
        });
      return;
    }
    void api.createHighlight(selectedId, start.id, end?.id ?? null)
      .then((highlight) => {
        setPaperMap((current) => current === null ? null : {
          ...current,
          highlights: [...current.highlights, highlight],
        });
        setNotice("Reader highlight saved to highlights.jsonl.");
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not save highlight");
      });
  }, [paperMap, selectedId]);

  const clarifySentence = useCallback((text: string, page: number): void => {
    if (sections.length === 0) { setSourceQuestion({ text, page }); return; }
    const index = sections.findIndex(
      (section) => page >= section.pages.start && page <= section.pages.end,
    );
    if (index >= 0) setActiveSection(index);
    setClarifySeed(text);
    setMarkMode(false);
    setPanel("digest");
  }, [sections]);

  const clarify = useCallback(
    async (
      selection: string,
      question: string,
      clarificationProvider: AnalysisProvider,
    ): Promise<Clarification> => {
      if (selectedId === null || selectedSection === null) {
        throw new Error("No section is selected");
      }
      return api.clarify(
        selectedId,
        selectedSection.id,
        selection,
        question,
        clarificationProvider,
      );
    },
    [selectedId, selectedSection],
  );

  const openGlossSection = useCallback(
    (sectionId: string): void => {
      const index = sections.findIndex((section) => section.id === sectionId);
      if (index >= 0) {
        setActiveSection(index);
        setView("overview");
        setPanel("digest");
      }
    },
    [sections],
  );

  const scan = useCallback((): void => {
    setNotice("Scanning the local vault…");
    void api
      .scan()
      .then(setLibrary)
      .catch((reason: unknown) =>
        setError(reason instanceof Error ? reason.message : "Vault scan failed"),
      );
  }, []);

  const importRemotePdf = useCallback(async (url: string): Promise<void> => {
    setError(null);
    setNotice("Importing remote PDF into the vault…");
    try {
      const imported = await api.importPdf(url);
      setLibrary(imported.library);
      setLibraryQuery("");
      selectPaper(imported.paper.id);
      setTextMode("pdf");
      setView("text");
      setNotice(`Imported ${imported.paper.metadata.title}.`);
    } catch (reason: unknown) {
      const failure = reason instanceof Error ? reason : new Error("Could not import PDF");
      setError(failure.message);
      throw failure;
    }
  }, [selectPaper]);

  const markdownConverted = useCallback((): void => {
    if (selectedId === null) return;
    void Promise.all([loadPaper(selectedId), refreshLibrary()]).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : "Could not refresh converted paper");
    });
  }, [loadPaper, refreshLibrary, selectedId]);

  const currentPaper = paperView?.paper ??
    library?.papers.find((paper) => paper.id === selectedId) ??
    null;
  const analysis = paperView?.analysis ?? null;
  const abstractPage = analysis?.abstract_extraction?.start_page ?? analysis?.sections.find((section) => section.kind === "abstract")?.pages.start
    ?? null;
  const sectionFocused = panel === "digest" && view === "overview" && selectedSection !== null && analysis !== null && currentPaper !== null;
  const readingPdf = !home && paperView !== null && (sectionFocused || (view === "text" && textMode === "pdf"));
  useEffect(() => {
    if (sectionFocused || focusRestore.current === null) return;
    // Tabs, the article switcher, and other overlays can also leave the focused reader.
    const restore = focusRestore.current;
    focusRestore.current = null;
    window.queueMicrotask(() => setLibraryOpen(readingPdf ? false : restore.library));
  }, [readingPdf, sectionFocused]);

  useLayoutEffect(() => {
    if (!readingPdf) return;
    // Apply defaults on reader entry, while allowing manual sidebar/zoom changes inside it.
    setLibraryOpen(false);
    setPdfZoom(1);
  }, [readingPdf, sectionFocused, selectedId]);
  const toolbarVisible = !readingPdf || toolbarPinned || toolbarPeek;
  useLayoutEffect(() => {
    const header = topbarRef.current;
    if (header === null) return;
    const measure = () => setTopbarHeight(toolbarVisible ? header.getBoundingClientRect().height : 0);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(header);
    return () => observer.disconnect();
  }, [toolbarVisible, home]);
  useEffect(() => {
    if (!readingPdf || toolbarPinned) return;
    const reveal = (event: MouseEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      const overControls = target?.closest(".topbar, .pdf-toolbar, .reader-reveal") != null;
      if (event.clientY <= 8 || overControls) setToolbarPeek(true);
      else if (target?.closest(".notes-panel, .pdf-source-tools, .section-figure-links, .section-figure-view header") != null) return;
      else if (!(topbarRef.current?.contains(document.activeElement) && document.activeElement?.matches(":focus-visible"))) setToolbarPeek(false);
    };
    window.addEventListener("mousemove", reveal);
    return () => window.removeEventListener("mousemove", reveal);
  }, [readingPdf, toolbarPinned]);
  useEffect(() => {
    if (!pendingSourceSearch.current || view !== "text" || textMode !== "pdf") return;
    pendingSourceSearch.current = false;
    const frame = requestAnimationFrame(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "/", bubbles: true })));
    return () => cancelAnimationFrame(frame);
  }, [view, textMode, panel]);
  useEffect(() => {
    const onReaderKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey) return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest(".notes-panel") != null) return;
      if (document.querySelector('.pdf-reader[data-link-hints="true"]') !== null) return;
      if (event.key === "/" && !isEditableTarget(event.target) && !home && !readingPdf && panel === null && !commandOpen && !switcherOpen && !queueOpen && !experimentOpen && toolsTab === null && !(compactLayout && libraryOpen) && target?.closest(".library-rail") == null) {
        event.preventDefault(); event.stopImmediatePropagation(); pendingSourceSearch.current = true; setTextMode("pdf"); setView("text"); return;
      }
      if (!isEditableTarget(event.target) && event.key === "T" && readingPdf) {
        event.preventDefault(); event.stopImmediatePropagation();
        setToolbarPinned((value) => !value); setToolbarPeek(false); return;
      }
      if (!isEditableTarget(event.target) && event.key === "E" && !home) {
        // The PDF's WORD-end motion owns E in Visual/Cursor mode, regardless of the
        // registration order of the app and reader's capture listeners.
        if (document.querySelector('.pdf-reader[data-source-visual="true"], .pdf-reader[data-source-cursor="true"]') !== null) return;
        event.preventDefault(); event.stopImmediatePropagation();
        if (notesOpen) requestNotesClose(); else openNotes();
        return;
      }
      if (event.key !== "Escape" && event.key !== "q") return;
      if (event.key === "q" && isEditableTarget(event.target)) return;
      // Close one layer only. Local source/digest selection gets first refusal.
      let close: (() => void) | null = null;
      if (commandOpen) close = () => setCommandOpen(false);
      else if (switcherOpen) close = () => setSwitcherOpen(false);
      else if (queueOpen) close = () => setQueueOpen(false);
      else if (sourceQuestion !== null) close = () => setSourceQuestion(null);
      else if (experimentOpen) close = () => setExperimentOpen(false);
      else if (toolsTab !== null) close = () => setToolsTab(null);
      else if (panel === "help") close = () => setPanel(null);
      else {
        const quitLocal = (selector: string, name: string) => {
          const node = document.querySelector(selector);
          let handled = false;
          if (node !== null) flushSync(() => { handled = !node.dispatchEvent(new CustomEvent(name, { cancelable: true })); });
          return handled;
        };
        if (quitLocal(".section-figure-view", "figure-quit")
          || quitLocal('.pdf-reader[data-source-local-mode="true"], .pdf-reader[data-key-prefix="g"]', "source-quit")
          || quitLocal('.context-panel[data-local-selection="true"]', "digest-quit")) {
          event.preventDefault(); event.stopImmediatePropagation(); return;
        }
        if (markMode) close = () => setMarkMode(false);
        else if (notesOpen) close = () => { requestNotesClose(); };
        else if (sectionFocused) close = closeSection;
        else if (panel !== null) close = () => setPanel(null);
        else if (libraryOpen) close = () => setLibraryOpen(false);
        else if (!home && analysis !== null && view !== "overview") close = () => setView("overview");
        else if (!home) close = () => openHome();
      }
      if (close !== null) { event.preventDefault(); event.stopImmediatePropagation(); flushSync(close); }
    };
    window.addEventListener("keydown", onReaderKey, true);
    return () => window.removeEventListener("keydown", onReaderKey, true);
  }, [analysis, closeSection, commandOpen, compactLayout, experimentOpen, home, libraryOpen, markMode, notesOpen, openHome, openNotes, panel,
    queueOpen, readingPdf, requestNotesClose, sectionFocused, sourceQuestion, switcherOpen, toolsTab, view]);

  return (
    <div className={`app-shell ${home ? "is-home" : ""} ${notesOpen ? "has-notes" : ""} ${readingPdf ? "is-reading" : ""} ${sectionFocused ? "has-section-focus" : ""} ${libraryOpen ? "has-library" : ""}`} style={{ "--topbar-height": `${topbarHeight}px` } as CSSProperties}>
      {readingPdf && <button className="reader-reveal" type="button" aria-label="Show reader controls (T)" onFocus={() => setToolbarPeek(true)} onClick={() => { setToolbarPinned((value) => !value); setToolbarPeek(false); }} />}
      <LibraryRail
        open={libraryOpen}
        keyboardMode={(compactLayout || home) && libraryOpen && !switcherOpen && !commandOpen && !queueOpen && !experimentOpen && toolsTab === null && panel === null && sourceQuestion === null}
        name={library?.name ?? "Articles"}
        papers={library?.papers ?? []}
        selectedId={selectedId}
        query={libraryQuery}
        searchRef={searchRef}
        onQuery={setLibraryQuery}
        onSelect={selectPaper}
        onClose={() => setLibraryOpen(false)}
        onScan={scan}
        onHome={() => openHome()}
        onImport={importRemotePdf}
        onVisiblePapersChange={setSidebarPaperIds}
      />

      <div className={`workspace ${libraryOpen ? "rail-visible" : ""}`}>
        <header className="topbar" ref={topbarRef} hidden={!toolbarVisible}
          onFocusCapture={() => setToolbarPeek(true)} onBlurCapture={(event) => {
            const next = event.relatedTarget;
            if (!event.currentTarget.contains(next) && !(next instanceof Element && next.closest(".notes-panel, .pdf-source-tools, .section-figure-links, .section-figure-view") !== null)) setToolbarPeek(false);
          }}>
          <nav className="workspace-navigation" aria-label="Library navigation">
            <button type="button" aria-label="Lysilogy home" title="Home" onClick={() => openHome()}>⌂</button>
            <button type="button" aria-label="Toggle library (F1)" title="Library (F1)" onClick={() => setLibraryOpen((value) => !value)}>☰</button>
          </nav>
          {home ? <span className="home-topbar-label">Your reading library</span> : <div className="view-switch" role="group" aria-label="Reader view">
            {analysis === null ? <>
              <button type="button" className={textMode === "pdf" ? "is-active" : ""} onClick={() => setTextMode("pdf")}>PDF</button>
              <button type="button" className={textMode === "markdown" ? "is-active" : ""} onClick={() => setTextMode("markdown")}>Text</button>
            </> : <>
            <button
              type="button"
              className={view === "abstract" ? "is-active" : ""}
              aria-current={view === "abstract" ? "page" : undefined}
              onClick={() => {
                setPanel(null);
                setView("abstract");
              }}
            >
              Abstract
            </button>
            <button
              type="button"
              className={view === "overview" ? "is-active" : ""}
              aria-current={view === "overview" ? "page" : undefined}
              onClick={() => {
                setPanel(null);
                setView("overview");
              }}
            >
              Overview
            </button>
            <button
              type="button"
              className={view === "glossary" ? "is-active" : ""}
              aria-current={view === "glossary" ? "page" : undefined}
              onClick={openGlossary}
            >
              Glossary
            </button>
            <button
              type="button"
              className={view === "text" ? "is-active" : ""}
              aria-current={view === "text" ? "page" : undefined}
              onClick={() => {
                setPanel(null);
                setTextMode("pdf");
                setView("text");
              }}
            >
              Text
            </button>
            </>}
          </div>}
          <div className="topbar-actions">
            {!home && <button className="queue-button notes-toggle" type="button" aria-pressed={notesOpen} title="Markdown notes (E)" onClick={() => { if (notesOpen) requestNotesClose(); else openNotes(); }}>Notes <kbd>E</kbd></button>}
            {readingPdf && <button className="queue-button toolbar-pin" type="button" aria-pressed={toolbarPinned} title="Pin reader controls (T)" onClick={() => { setToolbarPinned((value) => !value); setToolbarPeek(false); }}>Pin <kbd>T</kbd></button>}

            {home ? <>
              <button className="queue-button" type="button" onClick={scan}>Rescan</button>
              <button className={`queue-button ${queueHasActive ? "has-work" : ""}`} type="button" onClick={() => { setQueueOpen(true); void refreshQueue(); }}>Queue <kbd>Q</kbd></button>
            </> : paperView === null ? <span className="current-paper-label">Opening paper…</span> : analysis !== null && currentPaper !== null ? <span className="current-paper-label" title={`${currentPaper.metadata.authors.join(", ")} — ${currentPaper.metadata.year ?? ""} — ${currentPaper.metadata.title}`}>
              {currentPaper.metadata.authors.length > 2 ? `${currentPaper.metadata.authors[0]} et al.` : currentPaper.metadata.authors.join(" & ")}
              {currentPaper.metadata.year == null ? "" : ` — ${currentPaper.metadata.year}`} — {currentPaper.metadata.title}
            </span> : <>
            <label className="provider-select">
              <span>Reader</span>
              <select value={provider} onChange={(event) => setProvider(event.target.value as AnalysisProvider)}>
                <option value="codex">Codex</option>
                <option value="claude">Claude</option>
                <option value="heuristic">Offline</option>
              </select>
            </label>
            <button
              className="analyze-button"
              type="button"
              onClick={() => analyze()}
              disabled={processing}
            >
              {processing ? (
                <><span className="loader" /> Reading</>
              ) : (
                <>{analysisNeedsRefresh ? "Refresh" : "Analyze"} </>
              )}
            </button>
            </>}
          </div>
        </header>

        <main className="main-stage" ref={mainStageRef} tabIndex={-1} inert={sectionFocused} aria-hidden={sectionFocused || undefined}>
          {loading && (
            <div className="center-state"><span className="loader large" /><p>Opening the vault…</p></div>
          )}
          {!loading && home && <HomePage papers={library?.papers ?? []} name={library?.name ?? "Library"}
            darkInk={darkInk} onToggleInk={() => setDarkInk((value) => !value)}
            query={homeQuery} onQuery={setHomeQuery} onSelect={selectPaper} activeId={homeActiveId} onActive={setHomeActiveId}
            keyboardEnabled={!libraryOpen && !switcherOpen && !commandOpen && !queueOpen && panel === null && !experimentOpen && toolsTab === null}
            onImport={() => setLibraryOpen(true)} />}
          {!loading && !home && paperView === null && <div className="center-state"><span className="loader large" /><p>Opening paper…</p></div>}
          {!home && currentPaper !== null && paperView !== null && (
            <>
              {view === "abstract" && <PaperHeading metadata={currentPaper.metadata} />}

              {view === "abstract" && analysis !== null ? (
                <AbstractView
                  analysis={analysis}
                  abstractPage={abstractPage}
                  onRefresh={refreshComponent}
                  refreshing={refreshingComponent}
                  onOpenPage={openPage}
                  onContinue={() => setView("overview")}
                />
              ) : view === "overview" && analysis !== null ? (
                <SectionAtlas
                  analysis={analysis}
                  activeIndex={activeSection}
                  onOpen={openSection}
                  sourceUrl={api.source(selectedId)}
                  paperTitle={currentPaper.metadata.title}
                  paperMap={paperMap}
                  mapLoading={mapLoading}
                  darkInk={darkInk}
                  showAi={showAiHighlights}
                  showUser={showUserHighlights}
                  markMode={markMode}
                  onShowAi={() => setShowAiHighlights((value) => !value)}
                  onShowUser={() => setShowUserHighlights((value) => !value)}
                  onMarkMode={() => setMarkMode((value) => !value)}
                  onToggleHighlight={toggleHighlight}
                  onClarifySentence={clarifySentence}
                />
              ) : view === "glossary" && analysis !== null ? (
                <GlossaryView
                  entries={analysis.glossary}
                  onBack={() => setView("overview")}
                  onSection={openGlossSection}
                />
              ) : (
                <section className="text-view" aria-label="Full paper text">
                  {textMode === "markdown" ? (
                    <MarkdownReader
                      key={selectedId}
                      paperId={selectedId}
                      title={currentPaper.metadata.title}
                      onConverted={markdownConverted}
                      onOpenPage={openPage}
                    />
                  ) : (
                    <PdfReader
                      key={selectedId}
                      toolbarVisible={toolbarVisible}
                      url={api.source(selectedId)}
                      title={currentPaper.metadata.title}
                      page={pdfPage}
                      zoom={pdfZoom}
                      darkInk={darkInk}
                      spread={pdfSpread}
                      keyboardEnabled={panel === null && sourceQuestion === null && !switcherOpen && !commandOpen && !queueOpen && !experimentOpen && toolsTab === null && !(compactLayout && libraryOpen)}
                      onZoom={(delta) => setPdfZoom((value) => Math.max(.5, Math.min(2.5, value + delta)))}
                      onPage={setPdfPage}
                      onPageCount={setPdfPages}
                      onGloss={() => paperView.analysis != null && openGlossary()}
                      onToggleInk={() => setDarkInk((value) => !value)}
                      onToggleSpread={() => setPdfSpread((spread) => !spread)}
                      onClarifySelection={clarifySentence}
                      onSaveReference={(text, page) => {
                        setReferenceSeed({ text, page });
                        setToolsTab("references");
                      }}
                    />
                  )}
                </section>
              )}
            </>
          )}
        </main>

      </div>

      {sectionFocused ? (
        <SectionFocus key={`${currentPaper.id}:${selectedSection.id}`} url={api.source(currentPaper.id)} title={currentPaper.metadata.title}
          toolbarVisible={toolbarVisible}
          analysis={analysis} section={selectedSection} index={activeSection} paperMap={paperMap} darkInk={darkInk}
          keyboardEnabled={!switcherOpen && !commandOpen && !queueOpen && !experimentOpen && toolsTab === null && sourceQuestion === null}
          snapshots={pageSnapshots} onToggleInk={() => setDarkInk((value) => !value)} onSection={openSection} onClose={closeSection}
          onFullPaper={openPage} onClarify={clarifySentence}
          onSaveReference={(text, page) => { setReferenceSeed({ text, page }); setToolsTab("references"); }}
          digest={(onPage, keyboardEnabled, navigation) => <DigestPanel key={`${selectedSection.id}:${clarifySeed}`} navigation={navigation}
            section={selectedSection} claims={relatedClaims} initialSelection={clarifySeed} keyboardEnabled={keyboardEnabled && !commandOpen && !queueOpen && !experimentOpen && toolsTab === null}
            onClose={closeSection} onGloss={openGlossary} onOpenPage={onPage} onClarify={clarify} />}
        />
      ) : panel === "digest" && selectedSection !== null ? (
        <DigestPanel key={`${selectedSection.id}:${clarifySeed}`} section={selectedSection} claims={relatedClaims}
          initialSelection={clarifySeed} onClose={() => { setClarifySeed(""); setPanel(null); }}
          onGloss={openGlossary} onOpenPage={openPage} onClarify={clarify} />
      ) : null}
      {notesOpen && selectedId !== null && <NotesPanel key={selectedId} paperId={selectedId} onClose={closeNotes} onFocusReader={focusReaderPane} />}
      {sourceQuestion !== null && selectedId !== null && <PassageQuestion key={`${selectedId}:${sourceQuestion.page}:${sourceQuestion.text}`}
        paperId={selectedId} text={sourceQuestion.text} page={sourceQuestion.page} provider={provider}
        onClose={() => { setSourceQuestion(null); mainStageRef.current?.querySelector<HTMLElement>(".pdf-reader")?.focus(); }} />}
      {panel === "help" && <HelpOverlay onClose={() => setPanel(null)} />}
      {switcherOpen && (
        <PaperSwitcher
          papers={library?.papers ?? []}
          selectedId={selectedId}
          onClose={() => setSwitcherOpen(false)}
          onSelect={selectFromSwitcher}
        />
      )}
      {commandOpen && (
        <CommandMenu onClose={() => setCommandOpen(false)} onExecute={executeCommand} />
      )}
      {queueOpen && (
        <QueuePanel
          queue={queue}
          selectedPaperId={selectedId}
          provider={provider}
          hasAnalysis={hasAnalysis}
          focusFeedback={focusQueueFeedback}
          onClose={() => {
            setFocusQueueFeedback(false);
            setQueueOpen(false);
          }}
          onSelectPaper={(id) => {
            setFocusQueueFeedback(false);
            setQueueOpen(false);
            selectPaper(id);
          }}
          onFeedback={sendFeedback}
        />
      )}
      {experimentOpen && selectedId !== null && (
        <ExperimentPanel
          paperId={selectedId}
          provider={provider}
          onClose={() => setExperimentOpen(false)}
        />
      )}

      {toolsTab !== null && selectedId !== null && (
        <ReaderToolsPanel
          key={selectedId}
          paperId={selectedId}
          title={currentPaper?.metadata.title ?? "Paper"}
          papers={library?.papers ?? []}
          provider={provider}
          initialTab={toolsTab}
          seed={referenceSeed}
          onClose={closeReaderTools}
          onLibraryChanged={refreshLibrary}
          onSource={(id, page) => {
            setToolsTab(null);
            selectPaper(id);
            setView("text");
            setTextMode("pdf");
            setPdfPage(page);
          }}
        />
      )}

      {error !== null && (
        <div className="toast error-toast" role="alert">
          <span>{error}</span>
          <button type="button" onClick={() => setError(null)}>×</button>
        </div>
      )}
      {notice !== null && <div className="toast" role="status">{notice}</div>}
    </div>
  );
}
