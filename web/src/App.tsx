import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { AbstractView } from "./components/AbstractView";
import { CommandBar } from "./components/CommandBar";
import { CommandMenu } from "./components/CommandMenu";
import { DigestPanel } from "./components/DigestPanel";
import { GlossaryView } from "./components/GlossPanel";
import { HelpOverlay } from "./components/HelpOverlay";
import { LibraryRail } from "./components/LibraryRail";
import { MarkdownReader } from "./components/MarkdownReader";
import { PaperSwitcher } from "./components/PaperSwitcher";
import { PdfReader } from "./components/PdfReader";
import { QueuePanel } from "./components/QueuePanel";
import { SectionAtlas } from "./components/SectionAtlas";
import { useGlobalKeys } from "./hooks/useGlobalKeys";
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

function statusLabel(paper: PaperOverview | null): string {
  if (paper === null) return "No paper";
  switch (paper.status.state) {
    case "discovered":
      return "Not analyzed";
    case "extracted":
      return "Text ready";
    case "queued":
      return `Queued · ${paper.status.provider}`;
    case "extracting":
      return "Extracting text";
    case "analyzing":
      return `Reading · ${paper.status.provider}`;
    case "ready":
      return "Analysis ready";
    case "failed":
      return `Failed · ${paper.status.stage}`;
  }
}

export function App() {
  const [library, setLibrary] = useState<LibraryResponse | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(initialPaperId);
  const [paperView, setPaperView] = useState<PaperView | null>(null);
  const [activeSection, setActiveSection] = useState(0);
  const [panel, setPanel] = useState<Panel>(null);
  const [view, setView] = useState<ViewMode>("abstract");
  const [textMode, setTextMode] = useState<TextMode>("markdown");
  const [compactLayout, setCompactLayout] = useState(() => window.innerWidth < 1180);
  const [libraryOpen, setLibraryOpen] = useState(() => window.innerWidth >= 1180);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [queueOpen, setQueueOpen] = useState(false);
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
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const mainStageRef = useRef<HTMLElement>(null);

  useEffect(() => {
    mainStageRef.current?.scrollTo({ top: 0 });
  }, [selectedId, textMode, view]);

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
        const nextPaper = await api.paper(preferred.id);
        setPaperView(nextPaper);
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
        setPanel(null);
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
      setPaperMap(null);
      setMapLoading(false);
      setMarkMode(false);
      setPdfPage(1);
      setView("abstract");
      setTextMode("markdown");
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

  const openSection = useCallback((_section?: PaperSection, index?: number): void => {
    if (index !== undefined) setActiveSection(index);
    setClarifySeed("");
    setPanel("digest");
  }, []);

  const openPage = useCallback((page: number): void => {
    setPdfPage(page);
    setPanel(null);
    setTextMode("pdf");
    setView("text");
  }, []);

  const openGlossary = useCallback((): void => {
    setPanel(null);
    setView("glossary");
  }, []);

  const movePaper = useCallback(
    (delta: number): void => {
      if (library === null || selectedId === null) return;
      const index = library.papers.findIndex((paper) => paper.id === selectedId);
      const next = library.papers[Math.max(0, Math.min(library.papers.length - 1, index + delta))];
      if (next !== undefined) selectPaper(next.id);
    },
    [library, selectPaper, selectedId],
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
      case "library":
        setLibraryOpen((open) => !open);
        break;
      case "switch":
        setSwitcherOpen(true);
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
        setView("text");
        break;
      case "markdown":
        setTextMode("markdown");
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
  }, [analyze, openGlossary, provider, refreshQueue]);

  useGlobalKeys({
    enabled:
      panel === null && view !== "glossary" && !switcherOpen && !commandOpen && !queueOpen &&
      !(compactLayout && libraryOpen),
    activeIndex: activeSection,
    itemCount: sections.length,
    view,
    textMode,
    onMove: setActiveSection,
    onOpen: () => {
      if (selectedSection !== null) openSection(selectedSection, activeSection);
    },
    onDigest: () => selectedSection !== null && setPanel("digest"),
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
        setView("overview");
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
  const abstractPage = analysis?.sections.find((section) => section.kind === "abstract")?.pages.start
    ?? null;
  const activeJobCount = queue.jobs.filter(
    (job) => job.status.state === "queued" || job.status.state === "running",
  ).length;

  return (
    <div className="app-shell">
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
      />

      <div className={`workspace ${libraryOpen ? "rail-visible" : ""}`}>
        <header className="topbar">
          <button
            className="brand"
            type="button"
            onClick={() => setLibraryOpen((open) => !open)}
            aria-label="Toggle library"
          >
            <span>Λ</span>
            <strong>LYSILOGY</strong>
          </button>
          <div className="view-switch" role="group" aria-label="Reader view">
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
                setView("text");
              }}
            >
              Text
            </button>
          </div>
          <div className="topbar-actions">
            <button
              className={`queue-button ${activeJobCount > 0 ? "has-work" : ""}`}
              type="button"
              onClick={() => {
                setFocusQueueFeedback(false);
                setQueueOpen(true);
                void refreshQueue();
              }}
            >
              Queue{activeJobCount > 0 ? ` ${activeJobCount}` : ""}
            </button>
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
          </div>
        </header>

        <main className="main-stage" ref={mainStageRef}>
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
              <section className="paper-heading">
                <div className="paper-kicker">
                  <span className={`status-pip status-${currentPaper.status.state}`} />
                  {statusLabel(currentPaper)}
                  {paperView?.analysis !== null && paperView?.analysis !== undefined && (
                    <> · {paperView.analysis.provider}</>
                  )}
                </div>
                <h1>{currentPaper.metadata.title}</h1>
                <div className="paper-byline">
                  <span>{currentPaper.metadata.authors.join(", ") || "Unknown author"}</span>
                  {currentPaper.metadata.year !== null && <span>{currentPaper.metadata.year}</span>}
                  {currentPaper.metadata.page_count !== null && (
                    <span>{currentPaper.metadata.page_count} pages</span>
                  )}
                </div>
              </section>

              {view !== "text" && analysis === null ? (
                <section className="unanalyzed-state">
                  <div className="unmapped-grid" aria-hidden="true">
                    {Array.from({ length: 15 }, (_, index) => <i key={index} />)}
                  </div>
                  <div>
                    <span className="eyebrow">Unmapped paper</span>
                    <h2>Build the path from abstract to source.</h2>
                    <p>
                      Text is extracted locally. {provider === "heuristic" ? "The offline structural pass" : `${provider}, with web research and write access only to its live tasklist`} builds the orientation, overview, technical glossary, quotes, and context.
                    </p>
                    <button type="button" onClick={() => analyze()} disabled={processing}>
                      {processing ? "Reading the paper…" : `Analyze with ${provider}`}
                    </button>
                    {currentPaper.status.state === "failed" && (
                      <p className="inline-error">{currentPaper.status.message}</p>
                    )}
                  </div>
                </section>
              ) : view === "abstract" && analysis !== null ? (
                <AbstractView
                  analysis={analysis}
                  abstractPage={abstractPage}
                  onOpenPage={openPage}
                  onContinue={() => setView("overview")}
                />
              ) : view === "overview" && analysis !== null ? (
                <SectionAtlas
                  analysis={analysis}
                  activeIndex={activeSection}
                  onActiveIndex={setActiveSection}
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
                  onOpenPage={openPage}
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
                  <header className="text-view-header">
                    <div>
                      <span className="view-number">04</span>
                      <span className="eyebrow">Read the paper</span>
                    </div>
                    <div className="mini-switch" role="group" aria-label="Text format">
                      <button
                        type="button"
                        className={textMode === "markdown" ? "is-active" : ""}
                        onClick={() => setTextMode("markdown")}
                      >
                        Reconstructed
                      </button>
                      <button
                        type="button"
                        className={textMode === "pdf" ? "is-active" : ""}
                        onClick={() => setTextMode("pdf")}
                      >
                        PDF
                      </button>
                    </div>
                  </header>
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
                      onPage={setPdfPage}
                      onPageCount={setPdfPages}
                      onToggleInk={() => setDarkInk((value) => !value)}
                      onToggleSpread={() => setPdfSpread((spread) => !spread)}
                    />
                  )}
                </section>
              )}
            </>
          )}
        </main>

        <CommandBar view={view} textMode={textMode} panelOpen={panel === "digest"} />
      </div>

      {panel === "digest" && selectedSection !== null && (
        <DigestPanel
          key={`${selectedSection.id}:${clarifySeed}`}
          section={selectedSection}
          claims={relatedClaims}
          initialSelection={clarifySeed}
          onClose={() => {
            setClarifySeed("");
            setPanel(null);
          }}
          onGloss={openGlossary}
          onOpenPage={openPage}
          onClarify={clarify}
        />
      )}
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
