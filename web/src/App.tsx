import { SectionFocus } from "./components/SectionFocus";
import { capturePages, animatePages, type PageSnapshot } from "./lib/pageTransition";
import { sectionPages } from "./lib/sectionScope";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { AbstractView } from "./components/AbstractView";
import { PaperHeading } from "./components/PaperHeading";
import { CommandMenu } from "./components/CommandMenu";
import { PassageQuestion } from "./components/PassageQuestion";
import { DigestPanel } from "./components/DigestPanel";
import { ExperimentPanel } from "./components/ExperimentPanel";
import { GlossaryView } from "./components/GlossPanel";
import { HelpOverlay } from "./components/HelpOverlay";
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
  PaperOverview,
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

function paperByPreference(library: LibraryResponse, requested: string | null): PaperOverview | null {
  if (requested !== null) {
    const match = library.papers.find((paper) => paper.id === requested);
    if (match !== undefined) return match;
  }
  return (
    library.papers.find(
      (paper) =>
        paper.status.state === "ready" && paper.metadata.title.toLowerCase().includes("go to"),
    ) ??
    library.papers.find((paper) => paper.status.state === "ready") ??
    library.papers.find((paper) => paper.metadata.title.toLowerCase().includes("go to")) ??
    library.papers[0] ??
    null
  );
}

export function App() {
  const [library, setLibrary] = useState<LibraryResponse | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(initialPaperId);
  const selectedIdRef = useRef(selectedId);
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
  const [libraryOpen, setLibraryOpen] = useState(() => window.innerWidth >= 1180);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [queueOpen, setQueueOpen] = useState(false);
  const [experimentOpen, setExperimentOpen] = useState(false);
  const [toolsTab, setToolsTab] = useState<"supercut" | "references" | null>(null);
  const [referenceSeed, setReferenceSeed] = useState<{ text: string; page: number } | null>(null);
  const [focusQueueFeedback, setFocusQueueFeedback] = useState(false);
  const [queue, setQueue] = useState<ProcessingQueue>({ jobs: [] });
  const [libraryQuery, setLibraryQuery] = useState("");
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
    const next = await api.paper(id);
    setPaperView(next);
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
      .then(async (nextLibrary) => {
        if (cancelled) return;
        setLibrary(nextLibrary);
        const preferred = paperByPreference(nextLibrary, selectedId);
        if (preferred === null) return;
        setSelectedId(preferred.id);
        await loadPaper(preferred.id);
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
      if (event.key === "F1") {
        event.preventDefault();
        // F1 explicitly changes the rail preference instead of restoring it on exit.
        focusRestore.current = null;
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
      } else if (event.key === "q" && !isEditableTarget(event.target)) {
        event.preventDefault();
        setSwitcherOpen(false);
        setCommandOpen(false);
        if (compactLayout) setLibraryOpen(false);
        setFocusQueueFeedback(false);
        setQueueOpen((open) => !open);
        void refreshQueue();
      }
    };
    window.addEventListener("keydown", onFunctionKey);
    return () => window.removeEventListener("keydown", onFunctionKey);
  }, [compactLayout, refreshQueue]);

  useEffect(() => {
    const query = window.matchMedia("(max-width: 1179px)");
    const updateLayout = (event: MediaQueryListEvent | MediaQueryList): void => {
      setCompactLayout(event.matches);
      setLibraryOpen(!event.matches);
    };
    query.addEventListener("change", updateLayout);
    return () => query.removeEventListener("change", updateLayout);
  }, []);

  useEffect(() => {
    if (selectedId === null) return;
    window.history.replaceState(null, "", `#paper=${selectedId}`);
  }, [selectedId]);

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
    (id: string): void => {
      if (id === selectedId) {
        setLibraryOpen(false);
        return;
      }
      setActiveSection(0);
      setPanel(null);
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
      setLibraryOpen(window.innerWidth >= 1180);
      void loadPaper(id).catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not load paper");
      });
    },
    [loadPaper, selectedId],
  );

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
      `${analysisNeedsRefresh ? "Refresh queued" : "Queued"} for ${chosenProvider}. Press q to watch the live tasklist.`,
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
    if (focusRestore.current !== null) {
      setLibraryOpen(focusRestore.current.library);
      focusRestore.current = null;
    }
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
    switch (name) {
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
  }, [analyze, openGlossary, provider, refreshComponent, refreshQueue, selectedId]);

  useTabPhase({
    // The switcher and the command menu read Tab themselves; everywhere else
    // Tab steps through the reading phases of the current paper.
    overlayHandlesTab: switcherOpen || commandOpen || queueOpen || sourceQuestion !== null || experimentOpen || toolsTab !== null || paperView?.analysis == null || (panel === "digest" && view === "overview"),
    onCycle: (delta) => {
      setQueueOpen(false);
      if (compactLayout) setLibraryOpen(false);
      cycleView(delta);
    },
  });

  useGlobalKeys({
    enabled:
      panel === null && sourceQuestion === null && view !== "glossary" && !switcherOpen && !commandOpen && !queueOpen && !experimentOpen && toolsTab === null &&
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
      setLibraryOpen(true);
      window.setTimeout(() => searchRef.current?.focus(), 0);
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
  useEffect(() => {
    if (sectionFocused || focusRestore.current === null) return;
    // Tabs, the article switcher, and other overlays can also leave the focused reader.
    const restore = focusRestore.current;
    focusRestore.current = null;
    window.queueMicrotask(() => setLibraryOpen(restore.library));
  }, [sectionFocused]);


  return (
    <div className={`app-shell ${sectionFocused ? "has-section-focus" : ""} ${libraryOpen ? "has-library" : ""}`}>
      <LibraryRail
        open={libraryOpen}
        keyboardMode={compactLayout && libraryOpen}
        name={library?.name ?? "Articles"}
        papers={library?.papers ?? []}
        selectedId={selectedId}
        query={libraryQuery}
        searchRef={searchRef}
        onQuery={setLibraryQuery}
        onSelect={selectPaper}
        onClose={() => setLibraryOpen(false)}
        onScan={scan}
        onImport={importRemotePdf}
        onVisiblePapersChange={setSidebarPaperIds}
      />

      <div className={`workspace ${libraryOpen ? "rail-visible" : ""}`}>
        <header className="topbar">
          <button
            className="brand"
            type="button"
            onClick={() => setLibraryOpen((open) => !open)}
            aria-label="Toggle library"
          >
            <img className="brand-mark" src="/lambda-mark.svg" alt="" />
            <strong>LYSILOGY</strong>
          </button>
          <div className="view-switch" role="group" aria-label="Reader view">
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
          </div>
          <div className="topbar-actions">
            {analysis !== null && currentPaper !== null ? <span className="current-paper-label" title={`${currentPaper.metadata.authors.join(", ")} — ${currentPaper.metadata.year ?? ""} — ${currentPaper.metadata.title}`}>
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
              disabled={selectedId === null || processing}
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

        <main className="main-stage" ref={mainStageRef} inert={sectionFocused} aria-hidden={sectionFocused || undefined}>
          {loading && (
            <div className="center-state"><span className="loader large" /><p>Opening the vault…</p></div>
          )}
          {!loading && (library?.papers.length ?? 0) === 0 && (
            <div className="center-state">
              <span className="empty-glyph">∅</span>
              <h1>No PDFs discovered</h1>
              <p>Point <code>--library</code> at a directory containing papers, then rescan.</p>
            </div>
          )}
          {currentPaper !== null && (
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
                  sourceUrl={api.source(selectedId ?? currentPaper.id)}
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
              ) : selectedId === null ? null : (
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
