import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { CommandBar } from "./components/CommandBar";
import { DigestPanel } from "./components/DigestPanel";
import { GlossPanel } from "./components/GlossPanel";
import { HelpOverlay } from "./components/HelpOverlay";
import { LibraryRail } from "./components/LibraryRail";
import { MarkdownReader } from "./components/MarkdownReader";
import { PaperSwitcher } from "./components/PaperSwitcher";
import { PdfReader } from "./components/PdfReader";
import { SectionAtlas } from "./components/SectionAtlas";
import { useGlobalKeys } from "./hooks/useGlobalKeys";
import { api } from "./lib/api";
import type {
  AnalysisProvider,
  Clarification,
  LibraryResponse,
  PaperOverview,
  PaperSection,
  PaperView,
} from "./types";

type Panel = "digest" | "gloss" | "help" | null;
type ViewMode = "atlas" | "markdown" | "pdf";

const PROCESSING_STATES = new Set(["queued", "extracting", "analyzing"]);

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
      return "Atlas ready";
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
  const [view, setView] = useState<ViewMode>("atlas");
  const [compactLayout, setCompactLayout] = useState(() => window.innerWidth < 1180);
  const [libraryOpen, setLibraryOpen] = useState(() => window.innerWidth >= 1180);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [libraryQuery, setLibraryQuery] = useState("");
  const [provider, setProvider] = useState<AnalysisProvider>("codex");
  const [pdfPage, setPdfPage] = useState(1);
  const [pdfPages, setPdfPages] = useState(1);
  const [pdfZoom, setPdfZoom] = useState(1);
  const [darkInk, setDarkInk] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const mainStageRef = useRef<HTMLElement>(null);

  const refreshLibrary = useCallback(async (): Promise<LibraryResponse> => {
    const next = await api.library();
    setLibrary(next);
    return next;
  }, []);

  const loadPaper = useCallback(async (id: string): Promise<PaperView> => {
    const next = await api.paper(id);
    setPaperView(next);
    return next;
  }, []);

  useEffect(() => {
    let cancelled = false;
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
        setLibraryOpen((open) => !open);
      } else if (event.key === "F10") {
        event.preventDefault();
        setPanel(null);
        if (compactLayout) setLibraryOpen(false);
        setSwitcherOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onFunctionKey);
    return () => window.removeEventListener("keydown", onFunctionKey);
  }, [compactLayout]);

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
    paperView !== null && PROCESSING_STATES.has(paperView.paper.status.state);

  useEffect(() => {
    if (!processing || selectedId === null) return;
    const timer = window.setInterval(() => {
      void Promise.all([loadPaper(selectedId), refreshLibrary()]).catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not refresh analysis status");
      });
    }, 1_500);
    return () => window.clearInterval(timer);
  }, [loadPaper, processing, refreshLibrary, selectedId]);

  const selectPaper = useCallback(
    (id: string): void => {
      if (id === selectedId) {
        setLibraryOpen(false);
        return;
      }
      setActiveSection(0);
      setPanel(null);
      setPdfPage(1);
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

  const analyze = useCallback((): void => {
    if (selectedId === null || processing) return;
    setError(null);
    setNotice(`Queued for ${provider}. You can keep browsing while it reads.`);
    void api
      .analyze(selectedId, provider, paperView?.paper.status.state === "failed")
      .then(() => Promise.all([loadPaper(selectedId), refreshLibrary()]))
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not start analysis");
      });
  }, [loadPaper, paperView?.paper.status.state, processing, provider, refreshLibrary, selectedId]);

  const sections = useMemo(() => paperView?.analysis?.sections ?? [], [paperView?.analysis]);
  const selectedSection = sections[activeSection] ?? null;

  const openSection = useCallback((_section?: PaperSection, index?: number): void => {
    if (index !== undefined) setActiveSection(index);
    setPanel("digest");
  }, []);

  const openPage = useCallback((page: number): void => {
    setPdfPage(page);
    setPanel(null);
    setView("pdf");
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
    if (view === "pdf") setPdfPage((page) => Math.max(1, page - 1));
    else movePaper(-1);
  }, [movePaper, view]);
  const next = useCallback((): void => {
    if (view === "pdf") setPdfPage((page) => Math.min(pdfPages, page + 1));
    else movePaper(1);
  }, [movePaper, pdfPages, view]);

  const scrollMarkdown = useCallback((delta: number): void => {
    mainStageRef.current?.scrollBy({ top: delta, behavior: "smooth" });
  }, []);
  const scrollMarkdownTo = useCallback((edge: "start" | "end"): void => {
    const stage = mainStageRef.current;
    if (stage === null) return;
    stage.scrollTo({ top: edge === "start" ? 0 : stage.scrollHeight, behavior: "smooth" });
  }, []);

  useGlobalKeys({
    enabled: panel === null && !switcherOpen && !(compactLayout && libraryOpen),
    activeIndex: activeSection,
    itemCount: sections.length,
    view,
    onMove: setActiveSection,
    onOpen: () => {
      if (selectedSection !== null) openSection(selectedSection, activeSection);
    },
    onDigest: () => selectedSection !== null && setPanel("digest"),
    onGloss: () => paperView?.analysis != null && setPanel("gloss"),
    onHelp: () => setPanel("help"),
    onSearch: () => {
      setLibraryOpen(true);
      window.setTimeout(() => searchRef.current?.focus(), 0);
    },
    onToggleLibrary: () => setLibraryOpen((open) => !open),
    onToggleView: () => setView((current) => (current === "atlas" ? "pdf" : "atlas")),
    onToggleMarkdown: () =>
      setView((current) => (current === "markdown" ? "atlas" : "markdown")),
    onAnalyze: analyze,
    onEscape: () => {
      setLibraryOpen(false);
      if (view !== "atlas") setView("atlas");
    },
    onPrevious: previous,
    onNext: next,
    onInvert: () => setDarkInk((value) => !value),
    onZoom: (delta) => setPdfZoom((value) => Math.max(0.5, Math.min(2.5, value + delta))),
    onScroll: scrollMarkdown,
    onScrollTo: scrollMarkdownTo,
  });

  useEffect(() => {
    if (notice === null) return;
    const timer = window.setTimeout(() => setNotice(null), 4_000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const relatedClaims = useMemo(() => paperView?.analysis?.claims ?? [], [paperView]);

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
            <strong>LYSILOGOS</strong>
            <small>paper atlas</small>
          </button>
          <div className="view-switch" role="group" aria-label="Reader view">
            <button
              type="button"
              className={view === "atlas" ? "is-active" : ""}
              onClick={() => setView("atlas")}
            >
              Atlas
            </button>
            <button
              type="button"
              className={view === "markdown" ? "is-active" : ""}
              onClick={() => setView("markdown")}
            >
              Markdown <kbd>m</kbd>
            </button>
            <button
              type="button"
              className={view === "pdf" ? "is-active" : ""}
              onClick={() => setView("pdf")}
            >
              PDF <kbd>p</kbd>
            </button>
          </div>
          <div className="topbar-actions">
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
              onClick={analyze}
              disabled={selectedId === null || processing}
            >
              {processing ? <><span className="loader" /> Reading</> : <>Analyze <kbd>a</kbd></>}
            </button>
            <button className="icon-button" type="button" onClick={() => setPanel("help")} aria-label="Show key map">
              ?
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

              {view === "atlas" ? (
                paperView?.analysis === null || paperView?.analysis === undefined ? (
                  <section className="unanalyzed-state">
                    <div className="unmapped-grid" aria-hidden="true">
                      {Array.from({ length: 15 }, (_, index) => <i key={index} />)}
                    </div>
                    <div>
                      <span className="eyebrow">Unmapped paper</span>
                      <h2>Turn this PDF into a reading atlas.</h2>
                      <p>
                        Text is extracted locally. {provider === "heuristic" ? "The offline structural pass" : `${provider} in read-only plan mode`} builds the sections, quotes, context, and Gloss.
                      </p>
                      <button type="button" onClick={analyze} disabled={processing}>
                        {processing ? "Reading the paper…" : `Analyze with ${provider}`} <kbd>a</kbd>
                      </button>
                      {currentPaper.status.state === "failed" && (
                        <p className="inline-error">{currentPaper.status.message}</p>
                      )}
                    </div>
                  </section>
                ) : (
                  <SectionAtlas
                    analysis={paperView.analysis}
                    activeIndex={activeSection}
                    onActiveIndex={setActiveSection}
                    onOpen={openSection}
                  />
                )
              ) : selectedId === null ? null : view === "markdown" ? (
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
                  onPage={setPdfPage}
                  onPageCount={setPdfPages}
                  onToggleInk={() => setDarkInk((value) => !value)}
                />
              )}
            </>
          )}
        </main>

        <CommandBar view={view} panelOpen={panel === "digest" || panel === "gloss"} />
      </div>

      {panel === "digest" && selectedSection !== null && (
        <DigestPanel
          key={selectedSection.id}
          section={selectedSection}
          claims={relatedClaims}
          onClose={() => setPanel(null)}
          onGloss={() => setPanel("gloss")}
          onOpenPage={openPage}
          onClarify={clarify}
        />
      )}
      {panel === "gloss" && paperView?.analysis !== null && paperView?.analysis !== undefined && (
        <GlossPanel
          entries={paperView.analysis.glossary}
          onClose={() => setPanel(null)}
          onSection={openGlossSection}
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
