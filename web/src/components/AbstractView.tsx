import type { PaperAnalysis } from "../types";

type AbstractViewProps = {
  analysis: PaperAnalysis;
  abstractPage: number | null;
  onOpenPage: (page: number) => void;
  onContinue: () => void;
};

function providerLabel(analysis: PaperAnalysis): string {
  if (analysis.provider === "heuristic") return "Offline digest";
  return `AI digest · ${analysis.provider === "codex" ? "Codex" : "Claude"}`;
}

function generationLabel(analysis: PaperAnalysis): string {
  return analysis.provider === "heuristic" ? "Offline-generated" : "AI-generated";
}

export function AbstractView({
  analysis,
  abstractPage,
  onOpenPage,
  onContinue,
}: AbstractViewProps) {
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
        <div>
          <span className="eyebrow">{providerLabel(analysis)} · supplement</span>
          <h2>What the abstract leaves out</h2>
          <p>{analysis.outsider_brief}</p>
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
