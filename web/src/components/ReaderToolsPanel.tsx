import { useCallback, useEffect, useRef, useState } from "react";
import "../reader-tools.css";
import type { AnalysisProvider, PaperOverview } from "../types";
import { cutMarkdown, downloadText, readerToolsApi } from "../lib/readerTools";
import type { CutFormat, ReaderTools, SavedReference, Supercut, ToolAction } from "../lib/readerTools";

type Props = {
  paperId: string; title: string; papers: PaperOverview[]; provider: AnalysisProvider;
  initialTab: "supercut" | "references"; seed: { text: string; page: number } | null;
  onClose: () => void; onSource: (id: string, page: number) => void; onLibraryChanged: () => Promise<unknown>;
};

function CutView({ cut, title, onSource }: { cut: Supercut; title: string; onSource: (page: number) => void }) {
  const printRef = useRef<HTMLDivElement>(null);
  const [printError, setPrintError] = useState<string | null>(null);
  const pages = [...new Set(cut.paragraphs.map((paragraph) => paragraph.cut_page))];
  const print = (): void => {
    const sheets = printRef.current?.querySelectorAll<HTMLElement>(".cut-sheet") ?? [];
    if (cut.format === "six_pages" && [...sheets].some((sheet) => sheet.scrollHeight > sheet.clientHeight + 1)) {
      setPrintError("A page exceeds the print area. Download Markdown or generate another cut.");
      return;
    }
    setPrintError(null);
    window.print();
  };
  return <section className="supercut-result">
    <div className="reader-tool-actions">
      <span>{Math.floor(100 * cut.source_words / cut.total_words)}% exact source · {cut.total_words} words · {cut.agent}</span>
      <button type="button" onClick={() => downloadText("supercut.md", cutMarkdown(cut, title))}>Download Markdown</button>
      <button type="button" onClick={print}>Print / save PDF</button>
    </div>
    {printError !== null && <p role="alert">{printError}</p>}
    <div ref={printRef} className={cut.format === "six_pages" ? "cut-document six-page-cut" : "cut-document"}>
      {pages.map((page) => <article className="cut-sheet" key={page}>
        <header><span>{title}</span><span>{cut.format === "six_pages" ? `Supercut · ${page}/6` : "Ten-paragraph Supercut"}</span></header>
        {cut.paragraphs.filter((paragraph) => paragraph.cut_page === page).map((paragraph, index) =>
          <p key={index}>{paragraph.segments.map((segment, segmentIndex) => <span key={segmentIndex}>
            {segmentIndex > 0 && " "}
            {segment.kind === "connector"
              ? <span className="cut-connector"><span className="connector-label">Lysilogos connector: </span>{segment.text}</span>
              : <>{segment.text}<button className="cut-source" type="button" onClick={() => onSource(segment.source_page ?? 1)}>p. {segment.source_page}</button></>}
          </span>)}</p>)}
      </article>)}
    </div>
  </section>;
}

function ReferenceCard({ reference, papers, disabled, modelDisabled, onFind, onConnect, onLink, onRemove, onSource }: {
  reference: SavedReference; papers: PaperOverview[]; disabled: boolean; modelDisabled: boolean;
  onFind: () => void; onConnect: (question: string) => void; onLink: (id: string) => void;
  onRemove: () => void; onSource: (id: string, page: number) => void;
}) {
  const [question, setQuestion] = useState(reference.question ?? "");
  return <article className="saved-reference">
    <header><h3>{reference.citation}</h3><button type="button" disabled={disabled} onClick={onRemove} aria-label={`Remove reference: ${reference.citation}`}>Remove</button></header>
    {reference.source_page !== null && <p>Saved from PDF page {reference.source_page}</p>}
    {reference.note !== "" && <p>{reference.note}</p>}
    <div className="reader-tool-actions">
      <button type="button" disabled={modelDisabled} onClick={onFind}>Find &amp; fetch paper</button>
      <label>Link a paper in your library
        <select aria-label="Linked paper" disabled={disabled} value={reference.linked_paper_id ?? ""} onChange={(event) => onLink(event.target.value)}>
          <option value="" disabled>Choose a paper…</option>
          {papers.map((paper) => <option key={paper.id} value={paper.id}>{paper.metadata.title}</option>)}
        </select>
      </label>
      {reference.linked_paper_id !== null && <button type="button" onClick={() => {
        if (reference.linked_paper_id !== null) onSource(reference.linked_paper_id, 1);
      }}>Open linked paper</button>}
    </div>
    {reference.candidate !== null && <div className="reference-candidate">
      <strong>Search candidate: {reference.candidate.title}</strong>
      <p>{reference.candidate.explanation}</p>
      {reference.candidate.landing_url !== null && <a href={reference.candidate.landing_url} target="_blank" rel="noreferrer">Source page</a>}
      {reference.candidate.pdf_url === null && <p>No public PDF was found. You can import a copy and link it above.</p>}
    </div>}
    {reference.linked_paper_id !== null && <form onSubmit={(event) => {
      event.preventDefault();
      if (question.trim() !== "") onConnect(question.trim());
    }}>
      <label>Claim or question
        <textarea aria-label="Claim or question" value={question} maxLength={4000} onChange={(event) => setQuestion(event.target.value)}
          placeholder="Does this paper support the claim that…?" />
      </label>
      <div className="reader-tool-actions">
        <button type="submit" disabled={modelDisabled || question.trim() === ""}>Check claim</button>
        <button type="button" disabled={modelDisabled} onClick={() => {
          const contextQuestion = "Explain how the cited paper connects to the current paper, and which ideas, evidence, or assumptions it contributes.";
          setQuestion(contextQuestion);
          onConnect(contextQuestion);
        }}>Explain connection</button>
      </div>
    </form>}
    {reference.connection !== null && <section className="reference-connection">
      <h4>Lysilogos · {reference.connection.verdict}</h4>
      <p>{reference.connection.connector}</p>
      <p className="reference-limitation">{reference.connection.limitation}</p>
      {reference.connection.evidence.map((evidence, index) => <blockquote key={index}>
        <p>{evidence.quote}</p>
        <button type="button" onClick={() => onSource(evidence.paper_id, evidence.source_page)}>
          {papers.find((paper) => paper.id === evidence.paper_id)?.metadata.title ?? "Current paper"} · PDF p. {evidence.source_page}
        </button>
      </blockquote>)}
      <small>Quotations are checked against both papers. The relationship is Lysilogos’s interpretation.</small>
    </section>}
  </article>;
}

export function ReaderToolsPanel({ paperId, title, papers, provider, initialTab, seed, onClose, onSource, onLibraryChanged }: Props) {
  const [tab, setTab] = useState(initialTab);
  const [tools, setTools] = useState<ReaderTools>({ supercuts: [], references: [], jobs: [] });
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [format, setFormat] = useState<CutFormat>("ten_paragraphs");
  const [cutId, setCutId] = useState("");
  const [citation, setCitation] = useState(seed?.text ?? "");
  const [sourcePage, setSourcePage] = useState(seed?.page.toString() ?? "");
  const [note, setNote] = useState("");
  const closeRef = useRef<HTMLButtonElement>(null);
  const running = tools.jobs.some((job) => job.status === "running");
  const latestJob = tools.jobs.at(-1);
  const disabled = loading || busy || running;
  const modelDisabled = disabled || provider === "heuristic";
  const cut = tools.supercuts.find((entry) => entry.id === cutId) ?? tools.supercuts.at(-1);

  const refresh = useCallback(async (): Promise<void> => {
    setTools(await readerToolsApi.load(paperId));
  }, [paperId]);

  useEffect(() => {
    const previousFocus = document.activeElement;
    const dismiss = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", dismiss, true);
    closeRef.current?.focus();
    return () => {
      window.removeEventListener("keydown", dismiss, true);
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, [onClose]);

  useEffect(() => {
    const controller = new AbortController();
    void readerToolsApi.load(paperId, controller.signal).then((next) => {
      setTools(next);
      setLoading(false);
    }).catch((reason: unknown) => {
      if (!controller.signal.aborted) {
        setError(reason instanceof Error ? reason.message : "Could not load reader tools");
        setLoading(false);
      }
    });
    return () => controller.abort();
  }, [paperId]);

  useEffect(() => {
    if (!running) return;
    let cancelled = false;
    let timer: number;
    const poll = async (): Promise<void> => {
      try {
        const next = await readerToolsApi.load(paperId);
        if (cancelled) return;
        setTools(next);
        if (!next.jobs.some((job) => job.status === "running")) await onLibraryChanged();
      } catch (reason) {
        if (!cancelled) setError(reason instanceof Error ? reason.message : "Could not refresh task");
      }
      if (!cancelled) timer = window.setTimeout(() => { void poll(); }, 1500);
    };
    timer = window.setTimeout(() => { void poll(); }, 1500);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [onLibraryChanged, paperId, running]);

  const act = async (action: () => Promise<unknown>): Promise<void> => {
    setBusy(true);
    setError(null);
    try { await action(); await refresh(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Reader tool failed"); }
    finally { setBusy(false); }
  };
  const start = (action: ToolAction): void => { void act(() => readerToolsApi.start(paperId, action, provider)); };

  return <div className="reader-tools-backdrop" onMouseDown={onClose}>
    <section className="reader-tools-panel" role="dialog" aria-modal="true" aria-label="Lysilogos reader tools"
      onMouseDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Escape") { event.preventDefault(); onClose(); }
        if (event.key === "Tab") {
          event.preventDefault();
          const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), input, textarea, select:not(:disabled), a[href]")]
            .filter((element) => element.getClientRects().length > 0);
          const index = focusable.findIndex((element) => element === document.activeElement);
          const delta = event.shiftKey ? -1 : 1;
          focusable[(index + delta + focusable.length) % focusable.length]?.focus();
        }
      }}>
      <header className="reader-tools-heading">
        <div><span className="eyebrow">Lysilogy</span><h2>Lysilogos</h2><p>Cut a paper. Follow a reference. Connect the evidence.</p></div>
        <button ref={closeRef} type="button" onClick={onClose} aria-label="Close reader tools">Close</button>
      </header>
      <nav className="reader-tools-tabs" aria-label="Reader tools">
        <button type="button" aria-pressed={tab === "supercut"} onClick={() => setTab("supercut")}>Supercut</button>
        <button type="button" aria-pressed={tab === "references"} onClick={() => setTab("references")}>References ({tools.references.length})</button>
      </nav>
      <p className="reader-tools-paper">{title}</p>
      {error !== null && <p className="reader-tool-error" role="alert">{error}</p>}
      {loading && <p role="status">Loading saved work…</p>}
      {running && <p role="status">Lysilogos is working with {tools.jobs.find((job) => job.status === "running")?.provider}. You can close this panel and return later.</p>}
      {provider === "heuristic" && <p>Select Codex or Claude in the reader toolbar to generate cuts, find papers, or compare claims. Saving and linking references works offline.</p>}
      {(latestJob?.status === "failed" ? [latestJob] : []).map((job) =>
        <div className="reader-tool-error" key={job.id}><p>{job.error}</p>
          <button type="button" disabled={modelDisabled}
            onClick={() => start(job.action)}>Retry task</button>
        </div>)}
      {tab === "supercut" ? <>
        <form className="supercut-controls" onSubmit={(event) => { event.preventDefault(); start({ kind: "supercut", format }); }}>
          <label>Cut length<select value={format} onChange={(event) => setFormat(event.target.value as CutFormat)}>
            <option value="ten_paragraphs">Ten paragraphs</option><option value="six_pages">Six printable pages</option>
          </select></label>
          <button type="submit" disabled={modelDisabled}>Create Supercut</button>
        </form>
        <p>At least 80% exact source text. Lysilogos’s connector sentences are labeled. Six-page cuts use fixed Letter pages with up to 420 words per page.</p>
        {tools.supercuts.length > 0 && <label className="saved-cut-picker">Saved cuts<select value={cut?.id ?? ""} onChange={(event) => setCutId(event.target.value)}>
          {[...tools.supercuts].reverse().map((entry) => <option key={entry.id} value={entry.id}>
            {entry.format === "six_pages" ? "Six pages" : "Ten paragraphs"} · {new Date(entry.created_at).toLocaleString()}
          </option>)}
        </select></label>}
        {cut !== undefined && <CutView key={cut.id} cut={cut} title={title} onSource={(page) => onSource(paperId, page)} />}
      </> : <>
        <form className="save-reference-form" onSubmit={(event) => {
          event.preventDefault();
          void act(async () => {
            await readerToolsApi.save(paperId, citation.trim(), sourcePage === "" ? null : Number(sourcePage), note.trim());
            setCitation(""); setSourcePage(""); setNote("");
          });
        }}>
          <label>Citation<textarea aria-label="Citation" required value={citation} maxLength={4000} onChange={(event) => setCitation(event.target.value)} placeholder="Paste a bibliography entry, title, DOI, or citation to follow later." /></label>
          <div className="reader-tool-actions">
            <label>PDF page (optional)<input type="number" min="1" step="1" value={sourcePage} onChange={(event) => setSourcePage(event.target.value)} /></label>
            <label>Why save it?<input value={note} maxLength={4000} onChange={(event) => setNote(event.target.value)} /></label>
            <button type="submit" disabled={loading || busy || citation.trim() === ""}>Save reference</button>
          </div>
        </form>
        {!loading && tools.references.length === 0 && <p>No saved references yet. Select a citation in the PDF and choose “Save citation”, or paste one above.</p>}
        {[...tools.references].reverse().map((reference) => <ReferenceCard key={reference.id}
          reference={reference} papers={papers.filter((paper) => paper.id !== paperId)} disabled={disabled} modelDisabled={modelDisabled}
          onFind={() => start({ kind: "find_reference", reference_id: reference.id })}
          onConnect={(question) => start({ kind: "connect_reference", reference_id: reference.id, question })}
          onLink={(id) => { void act(() => readerToolsApi.link(paperId, reference.id, id)); }}
          onRemove={() => { void act(() => readerToolsApi.remove(paperId, reference.id)); }}
          onSource={onSource} />)}
      </>}
    </section>
  </div>;
}
