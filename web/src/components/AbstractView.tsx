import {
  CONTEXT_PARTS,
  basisCounts,
  basisLabel,
  downgradeText,
  paperContext,
  pipelineLabel,
  readerWarnings,
} from "../lib/readerOutput";
import type { ContextClaim, ContextNote, ContextSource, PaperAnalysis, PaperContext } from "../types";
import { ReadAlongsideList } from "./ReadAlongsideList";

type AbstractViewProps = {
  analysis: PaperAnalysis;
  abstractPage: number | null;
  onOpenPage: (page: number) => void;
  onOpenPaper: (paperId: string) => void;
  onContinue: () => void;
  onRefresh?: (component: "abstract" | "context") => void;
  refreshing?: "abstract" | "context" | "structure" | null;
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

function ClaimList({ claims, context, onOpenPage, onOpenPaper }: {
  claims: ContextClaim[];
  context: PaperContext;
  onOpenPage: (page: number) => void;
  onOpenPaper: (paperId: string) => void;
}) {
  const works = new Map(context.read_alongside.filter((item) => item.work_id != null)
    .map((item) => [item.work_id as string, item] as const));
  return <ul className="context-claim-list">{claims.map((claim, index) => {
    const label = basisLabel(claim.basis);
    const downgrade = downgradeText(claim);
    const cited = claim.basis === "dossier" ? claim.work_ids.map((id) => works.get(id)).filter((item) => item !== undefined) : [];
    return <li className="context-claim" data-basis={claim.basis} key={`${claim.text}-${index}`}>
      <span className={`context-basis basis-${claim.basis}`} title={label.long}>{label.short}</span>
      <p>{claim.text}</p>
      {claim.basis === "paper" && claim.evidence.map((quote, quoteIndex) => (
        <blockquote className="context-claim-quote" key={quoteIndex}>
          “{quote.quote}”{" "}
          <button type="button" className="context-claim-page" onClick={() => onOpenPage(quote.page)}
            aria-label={`Open PDF page ${quote.page}`}>p. {quote.page} ↗</button>
        </blockquote>
      ))}
      {cited.length > 0 && <p className="context-claim-works">Graph record:{" "}
        {cited.map((work, workIndex) => <span key={work.work_id}>
          {workIndex > 0 && "; "}
          {work.local_paper_id != null
            ? <button type="button" className="context-claim-work" onClick={() => onOpenPaper(work.local_paper_id as string)}>{work.title}</button>
            : <cite>{work.title}</cite>}
          {work.year != null && ` (${work.year})`}
        </span>)}
      </p>}
      {downgrade !== null && <p className="context-claim-downgrade">Shown as background: {downgrade}</p>}
    </li>;
  })}</ul>;
}

function PipelineChecks({ analysis }: { analysis: PaperAnalysis }) {
  const warnings = readerWarnings(analysis);
  const pipeline = pipelineLabel(analysis.pipeline_version);
  const f1 = analysis.abstract_agreement?.token_f1;
  if (warnings.length === 0 && pipeline === null && f1 == null) return null;
  return <details className="pipeline-checks">
    <summary>
      {warnings.length === 0 ? "Pipeline checks passed" : `${warnings.length} pipeline ${warnings.length === 1 ? "check" : "checks"} flagged`}
      {pipeline !== null && <span className="pipeline-version" title={analysis.pipeline_version ?? undefined}> · {pipeline}</span>}
    </summary>
    {warnings.length > 0 && <ul>{warnings.map((warning, index) => (
      <li key={index} data-warning-kind={warning.kind}><code>{warning.field}</code> {warning.message}</li>
    ))}</ul>}
    {f1 != null && <p>The model’s abstract and the deterministic extraction agree at token F1 {f1.toFixed(2)}.</p>}
  </details>;
}

function ContextNodeView({ context, onOpenPage, onOpenPaper }: {
  context: PaperContext;
  onOpenPage: (page: number) => void;
  onOpenPaper: (paperId: string) => void;
}) {
  const counts = basisCounts(context);
  return <>
    <section className="reading-context is-node" aria-label="Research history and subsequent influence">
      {CONTEXT_PARTS.map(({ part, eyebrow, heading }) => {
        const claims = context[part];
        return <article className="reading-context-card" key={part} data-context-kind={part}>
          <span className="eyebrow">{eyebrow}</span>
          <h2>{heading}</h2>
          {claims.length > 0
            ? <ClaimList claims={claims} context={context} onOpenPage={onOpenPage} onOpenPaper={onOpenPaper} />
            : <p className="context-empty">Nothing was written for this part.</p>}
        </article>;
      })}
    </section>
    {context.reception_note != null && context.reception_note.trim() !== "" && (
      <p className="context-reception-note"><strong>Not known:</strong> {context.reception_note}</p>
    )}
    <p className="context-provenance">
      {counts.paper} from the paper · {counts.dossier} from the citation graph · {counts.background} unverified background
    </p>
    <ReadAlongsideList items={context.read_alongside} onOpenPaper={onOpenPaper} />
  </>;
}

export function AbstractView({
  analysis,
  abstractPage,
  onOpenPage,
  onOpenPaper,
  onContinue,
  onRefresh,
  refreshing = null,
}: AbstractViewProps) {
  const node = paperContext(analysis);
  const context = validContext(analysis);
  const sourceNumbers = new Map(
    context.sources.map((source, index) => [source.id, index + 1] as const),
  );
  const sourceById = new Map(context.sources.map((source) => [source.id, source] as const));
  const hasSources = context.notes.length > 0 && context.sources.length > 0;
  const legacyNotes = context.notes.filter((note) => note.kind == null || note.kind === "legacy");
  const assessment = analysis.context_assessment;
  const renderNotes = (notes: ContextNote[]) => <ul className="context-note-list">{notes.map((note, index) => (
    <li className="context-note-text" key={`${note.text}-${index}`}>
      {note.text}{" "}
      <span className="context-note-citations" aria-label="Supporting sources">
        {note.source_ids.map((id) => {
          const source = sourceById.get(id);
          return source == null ? null : <a key={id} href={source.url} target="_blank" rel="noreferrer"
            aria-label={`Source ${sourceNumbers.get(id)}: ${source.title}`}>[{sourceNumbers.get(id)}]</a>;
        })}
      </span>
    </li>
  ))}</ul>;

  return (
    <section className="abstract-view" aria-label="Paper abstract and orientation">
      <article className="abstract-tldr">
        <span className="eyebrow">{generationLabel(analysis)} · one-sentence TL;DR</span>
        <p>{analysis.thesis}</p>
      </article>

      {node !== null && node.lead.length > 0 && (
        <article className="why-it-matters" aria-labelledby="why-it-matters-heading">
          <span className="eyebrow">Surrounding context</span>
          <h2 id="why-it-matters-heading">Why it matters</h2>
          <ClaimList claims={node.lead} context={node} onOpenPage={onOpenPage} onOpenPaper={onOpenPaper} />
        </article>
      )}

      <article className="authored-abstract">
        <header>
          <div>
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

      {node !== null ? <ContextNodeView context={node} onOpenPage={onOpenPage} onOpenPaper={onOpenPaper} />
        : <section className="reading-context" aria-label="Research history and subsequent influence">
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
      </section>}
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

      <PipelineChecks analysis={analysis} />

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
