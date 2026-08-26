import type { ContextNote, ContextSource, PaperAnalysis } from "../types";

type AbstractViewProps = {
  analysis: PaperAnalysis;
  abstractPage: number | null;
  onOpenPage: (page: number) => void;
  onContinue: () => void;
};

function providerLabel(analysis: PaperAnalysis, hasSources: boolean): string {
  if (analysis.provider === "heuristic") return "Offline digest · source paper";
  const provider = analysis.provider === "codex" ? "Codex" : "Claude";
  return `AI digest · ${provider}${hasSources ? " · cited" : ""}`;
}

function generationLabel(analysis: PaperAnalysis): string {
  return analysis.provider === "heuristic" ? "Offline-generated" : "AI-generated";
}

function checkedAt(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "during analysis";
  return `${date.toISOString().replace(/\.\d{3}Z$/u, "Z")} UTC`;
}

function sourceHost(value: string): string {
  try {
    return new URL(value).hostname.replace(/^www\./u, "");
  } catch {
    return "source link";
  }
}

function validContext(analysis: PaperAnalysis): {
  notes: ContextNote[];
  sources: ContextSource[];
} {
  if (analysis.schema_version < 4 || analysis.provider === "heuristic") {
    return { notes: [], sources: [] };
  }
  const candidates = analysis.context_sources ?? [];
  const sourceIds = new Set(candidates.map((source) => source.id));
  const notes = (analysis.context_notes ?? []).filter(
    (note) =>
      note.source_ids.length > 0 && note.source_ids.every((sourceId) => sourceIds.has(sourceId)),
  );
  const citedIds = new Set(notes.flatMap((note) => note.source_ids));
  return {
    notes,
    sources: candidates.filter((source) => citedIds.has(source.id)),
  };
}

export function AbstractView({
  analysis,
  abstractPage,
  onOpenPage,
  onContinue,
}: AbstractViewProps) {
  const context = validContext(analysis);
  const sourceNumbers = new Map(
    context.sources.map((source, index) => [source.id, index + 1] as const),
  );
  const sourceById = new Map(context.sources.map((source) => [source.id, source] as const));
  const hasSources = context.notes.length > 0 && context.sources.length > 0;
  const legacyModelContext = analysis.provider !== "heuristic" && analysis.schema_version < 4;

  return (
    <section className="abstract-view" aria-label="Paper abstract and orientation">
      <header className="view-introduction">
        <div>
          <span className="view-number">01</span>
          <span className="eyebrow">Orient in under a minute</span>
        </div>
        <p>
          Start with the shortest useful reading, then separate the authors&apos; own account from
          generated context.
        </p>
      </header>

      <article className="abstract-tldr">
        <span className="eyebrow">{generationLabel(analysis)} · one-sentence TL;DR</span>
        <p>{analysis.thesis}</p>
      </article>

      <article className="authored-abstract">
        <header>
          <div>
            <span className="eyebrow">Authors&apos; words</span>
            <h2>Abstract</h2>
          </div>
          {abstractPage !== null && (
            <button type="button" onClick={() => onOpenPage(abstractPage)}>
              PDF page {abstractPage} ↗
            </button>
          )}
        </header>
        {analysis.author_abstract == null ? (
          <p className="missing-abstract">
            No authored abstract was identified in the extracted paper. Lysilogos leaves this
            space empty rather than substituting generated prose.
          </p>
        ) : (
          <p>{analysis.author_abstract}</p>
        )}
      </article>

      <article className="abstract-supplement">
        <div className="supplement-mark" aria-hidden="true">+</div>
        <div className="supplement-content">
          <span className="eyebrow">{providerLabel(analysis, hasSources)} · supplement</span>
          <h2>What the abstract leaves out</h2>
          {analysis.provider === "heuristic" ? (
            <p className="context-note-text">{analysis.outsider_brief}</p>
          ) : hasSources ? (
            <div className="context-notes">
              {context.notes.map((note, noteIndex) => (
                <p className="context-note-text" key={`${note.text}-${noteIndex}`}>
                  {note.text}{" "}
                  <span className="context-note-citations" aria-label="Supporting sources">
                    {note.source_ids.map((sourceId) => {
                      const source = sourceById.get(sourceId);
                      const number = sourceNumbers.get(sourceId);
                      return source == null || number == null ? null : (
                        <a
                          aria-label={`Source ${number}: ${source.title}`}
                          href={source.url}
                          key={sourceId}
                          rel="noreferrer"
                          target="_blank"
                        >
                          [{number}]
                        </a>
                      );
                    })}
                  </span>
                </p>
              ))}
            </div>
          ) : (
            <p className="context-empty">
              {legacyModelContext
                ? "This map predates cited context. Refresh it to research exact sources and check their links before showing field history or reception."
                : "No field-history, reception, or later-interpretation note is shown because no complete citation set passed independent link checks."}
            </p>
          )}

          {hasSources && (
            <section className="context-sources" aria-labelledby="context-sources-heading">
              <header>
                <div>
                  <span className="eyebrow">Evidence trail</span>
                  <h3 id="context-sources-heading">Exact sources</h3>
                </div>
                <span>{context.sources.length} checked</span>
              </header>
              <ol>
                {context.sources.map((source, index) => (
                  <li className="context-source" key={source.id}>
                    <span className="context-source-number">[{index + 1}]</span>
                    <div>
                      <a
                        className="context-source-title"
                        href={source.url}
                        rel="noreferrer"
                        target="_blank"
                      >
                        {source.title} ↗
                      </a>
                      <p className="context-source-record">
                        {source.authors.length > 0 ? source.authors.join(", ") : "Author unknown"}
                        {source.year == null ? "" : ` · ${source.year}`} · {sourceHost(source.url)}
                      </p>
                      <p className="context-source-support">
                        <strong>Used for:</strong> {source.supports}
                      </p>
                      <span className="context-source-check">
                        Link checked {checkedAt(source.verified_at)}
                      </span>
                    </div>
                  </li>
                ))}
              </ol>
              <p className="context-verification-scope">
                Lysilogos resolved every redirect, rejected non-public destinations, and required
                an HTTP success response. That verifies link reachability at the recorded time—not
                that the source semantically proves the note. Open the exact record to inspect the
                evidence.
              </p>
            </section>
          )}
        </div>
      </article>

      <footer className="view-continuation">
        <div>
          <span className="eyebrow">Next level</span>
          <strong>See how the argument is built.</strong>
        </div>
        <button type="button" onClick={onContinue}>
          Continue to overview <span>02 →</span>
        </button>
      </footer>
    </section>
  );
}
