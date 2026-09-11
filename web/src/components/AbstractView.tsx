import type { ContextNote, ContextSource, PaperAnalysis } from "../types";

type AbstractViewProps = {
  analysis: PaperAnalysis;
  abstractPage: number | null;
  onOpenPage: (page: number) => void;
  onContinue: () => void;
  onRefresh?: (component: "abstract" | "context") => void;
  refreshing?: "abstract" | "context" | null;
};

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
  if (analysis.schema_version < 4 || (analysis.provider === "heuristic" && analysis.context_assessment == null)) {
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
  onRefresh,
  refreshing = null,
}: AbstractViewProps) {
  const context = validContext(analysis);
  const sourceNumbers = new Map(
    context.sources.map((source, index) => [source.id, index + 1] as const),
  );
  const sourceById = new Map(context.sources.map((source) => [source.id, source] as const));
  const hasSources = context.notes.length > 0 && context.sources.length > 0;
  const legacyNotes = context.notes.filter((note) => note.kind == null || note.kind === "legacy");
  const assessment = analysis.context_assessment;
  const renderNotes = (notes: ContextNote[]) => notes.map((note, index) => (
    <p className="context-note-text" key={`${note.text}-${index}`}>
      {note.text}{" "}
      <span className="context-note-citations" aria-label="Supporting sources">
        {note.source_ids.map((id) => {
          const source = sourceById.get(id);
          return source == null ? null : <a key={id} href={source.url} target="_blank" rel="noreferrer"
            aria-label={`Source ${sourceNumbers.get(id)}: ${source.title}`}>[{sourceNumbers.get(id)}]</a>;
        })}
      </span>
    </p>
  ));

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
          {onRefresh != null && <button type="button" disabled={refreshing !== null}
            onClick={() => onRefresh("abstract")}>{refreshing === "abstract" ? "Checking abstract…" : "Refresh abstract"}</button>}
          {abstractPage !== null && (
            <button type="button" onClick={() => onOpenPage(abstractPage)}>
              PDF page {abstractPage} ↗
            </button>
          )}
        </header>
        {analysis.author_abstract == null ? (
          <p className="missing-abstract">
            {analysis.abstract_extraction?.status === "needs_review"
              ? "The abstract’s boundaries or text could not be verified. Open the source to read it."
              : analysis.abstract_extraction?.status === "needs_ocr"
                ? "This paper needs text recognition before its abstract can be extracted."
                : "No authored abstract was identified in the extracted paper."}
          </p>
        ) : (
          <p>{analysis.author_abstract}</p>
        )}
      </article>

      <section className="reading-context" aria-label="Research history and subsequent influence">
        {(["before", "after"] as const).map((kind) => {
          const notes = context.notes.filter((note) => note.kind === kind);
          return <article className="reading-context-card" key={kind} data-context-kind={kind}>
            <span className="eyebrow">{kind === "before" ? "Before the paper" : "After the paper"}</span>
            <h2>{kind === "before" ? "The problem it entered" : "What followed"}</h2>
            {notes.length > 0 ? renderNotes(notes) : <p className="context-empty">
              {assessment == null ? "Research this paper’s history to add cited context."
                : kind === "before" ? "No account of prior research passed the evidence checks."
                  : "No subsequent influence was established by the inspected sources."}
            </p>}
          </article>;
        })}
      </section>
      {onRefresh != null && <button className="context-refresh" type="button" disabled={refreshing !== null}
        onClick={() => onRefresh("context")}>{refreshing === "context" ? "Researching and reviewing context…" : "Research before and after"}</button>}
      {legacyNotes.length > 0 && <article className="reading-context-card legacy-context">
        <span className="eyebrow">Earlier context</span>
        {renderNotes(legacyNotes)}
      </article>}
      {assessment != null && <details className="context-assessment">
        <summary>Evidence checks · {assessment.metrics.published_claims} claims shown</summary>
        <dl>
          <div><dt>Claims with citations</dt><dd>{assessment.metrics.cited_claims} / {assessment.metrics.proposed_claims || "—"}</dd></div>
          <div><dt>Citation links supported on review</dt><dd>{assessment.metrics.supported_links} / {assessment.metrics.proposed_links || "—"}</dd></div>
          <div><dt>Claims fully supported on review</dt><dd>{assessment.metrics.fully_supported_claims} / {assessment.metrics.proposed_claims || "—"}</dd></div>
          <div><dt>Claims without a complete review</dt><dd>{assessment.metrics.unassessed_claims}</dd></div>
        </dl>
        <p>A separate AI review checks cited passages, chronology, and usefulness. These counts describe its assessment; open the sources to inspect the evidence.</p>
        {assessment.evidence_gaps.length > 0 && <ul>{assessment.evidence_gaps.map((gap, index) => <li key={index}>{gap}</li>)}</ul>}
      </details>}

      {hasSources && (
        <section className="context-sources" aria-labelledby="context-sources-heading">
          <header>
            <div>
              <span className="eyebrow">Evidence trail</span>
              <h2 id="context-sources-heading">Sources</h2>
            </div>
            <span>{context.sources.length} checked</span>
          </header>
          <ol>
            {context.sources.map((source, index) => (
              <li className="context-source" key={source.id}>
                <span className="context-source-number">[{index + 1}]</span>
                <div>
                  <a className="context-source-title" href={source.url} rel="noreferrer" target="_blank">
                    {source.title} ↗
                  </a>
                  <p className="context-source-record">
                    {source.authors.length > 0 ? source.authors.join(", ") : "Author unknown"}
                    {source.year == null ? "" : ` · ${source.year}`} · {sourceHost(source.url)}
                  </p>
                  <p className="context-source-support">
                    <strong>Used for:</strong> {source.supports}
                  </p>
                  {source.excerpt != null && <blockquote className="context-source-excerpt">
                    “{source.excerpt}” {source.location != null && <cite>— {source.location}</cite>}
                  </blockquote>}
                  <span className="context-source-check">Link checked {checkedAt(source.verified_at)}</span>
                </div>
              </li>
            ))}
          </ol>
          <p className="context-verification-scope">
            Lysilogy resolved every redirect, rejected non-public destinations, and required an
            HTTP success response. That verifies link reachability at the recorded time—not that
            the source semantically proves the note. Open the exact record to inspect the evidence.
          </p>
        </section>
      )}

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
