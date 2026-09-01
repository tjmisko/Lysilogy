import { useEffect, useMemo, useState } from "react";

import { api } from "../lib/api";
import type {
  AnalysisProvider,
  ExperimentCatalog,
  ExperimentJudgment,
  ExperimentView,
  LearningRamp,
} from "../types";

type ExperimentPanelProps = {
  paperId: string;
  provider: AnalysisProvider;
  onClose: () => void;
};

type Choice = "A" | "B" | "tie";

function ChoiceField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: Choice;
  onChange: (value: Choice) => void;
}) {
  return (
    <fieldset className="experiment-choice">
      <legend>{label}</legend>
      {(["A", "tie", "B"] as const).map((choice) => (
        <button
          key={choice}
          type="button"
          className={value === choice ? "is-active" : ""}
          onClick={() => onChange(choice)}
        >
          {choice === "tie" ? "Tie" : choice}
        </button>
      ))}
    </fieldset>
  );
}

function SourceLinks({ urls }: { urls: string[] }) {
  const publicUrls = urls.filter((url) => /^https?:\/\//iu.test(url));
  if (publicUrls.length === 0) return null;
  return (
    <span className="experiment-sources">
      {publicUrls.map((url, index) => (
        <a key={url} href={url} target="_blank" rel="noreferrer">
          source {index + 1}
        </a>
      ))}
    </span>
  );
}

function Ramp({ ramp }: { ramp: LearningRamp }) {
  return (
    <div className="learning-ramp">
      <section>
        <h4>Foothold</h4>
        <p><strong>Question.</strong> {ramp.foothold.question}</p>
        <p><strong>Answer.</strong> {ramp.foothold.answer}</p>
        <p><strong>Why care.</strong> {ramp.foothold.why_care}</p>
        <p><strong>Mechanism.</strong> {ramp.foothold.mechanism}</p>
      </section>

      <section>
        <h4>Concept ramp</h4>
        <ol>
          {ramp.concepts.map((concept) => (
            <li key={`${concept.term}-${concept.why_now}`}>
              <strong>{concept.term}</strong> — {concept.plain_language}
              <details>
                <summary>Technical definition and bridge</summary>
                <p>{concept.technical_definition}</p>
                <p><em>Why now:</em> {concept.why_now}</p>
                {concept.bridge !== null && <p><em>Bridge:</em> {concept.bridge}</p>}
                {concept.bridge_limit !== null && <p><em>Where it breaks:</em> {concept.bridge_limit}</p>}
              </details>
            </li>
          ))}
        </ol>
      </section>

      <section>
        <h4>Essential passages</h4>
        {ramp.essential_passages.map((passage, index) => (
          <article className="experiment-passage" key={`${passage.page}-${index}`}>
            <span>{passage.role} · p. {passage.page}</span>
            <blockquote>{passage.quote}</blockquote>
            {passage.context_before.length > 0 && (
              <p><em>Before reading:</em> {passage.context_before.join(" · ")}</p>
            )}
            <p><em>Read for:</em> {passage.read_for}</p>
          </article>
        ))}
      </section>

      {ramp.reception.length > 0 && (
        <section>
          <h4>Use and reception</h4>
          <ul>
            {ramp.reception.map((item, index) => (
              <li key={`${item.kind}-${index}`}>
                <strong>{item.kind}.</strong> {item.claim} <SourceLinks urls={item.source_urls} />
              </li>
            ))}
          </ul>
        </section>
      )}

      {ramp.counterarguments.length > 0 && (
        <section>
          <h4>Informed opposition</h4>
          {ramp.counterarguments.map((item, index) => (
            <article key={`${item.target}-${index}`}>
              <p><strong>{item.camp ?? "Paper-specific objection"} · {item.target}.</strong> {item.objection}</p>
              <p><em>Likely reply:</em> {item.likely_reply} <SourceLinks urls={item.source_urls} /></p>
            </article>
          ))}
        </section>
      )}

      {ramp.uncertainties.length > 0 && (
        <section>
          <h4>Uncertainties</h4>
          <ul>{ramp.uncertainties.map((item) => <li key={item}>{item}</li>)}</ul>
        </section>
      )}
    </div>
  );
}

export function ExperimentPanel({ paperId, provider, onClose }: ExperimentPanelProps) {
  const [catalog, setCatalog] = useState<ExperimentCatalog | null>(null);
  const [runs, setRuns] = useState<ExperimentView[]>([]);
  const [selectedExperiment, setSelectedExperiment] = useState("conceptual-bridge");
  const [selectedRun, setSelectedRun] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [judging, setJudging] = useState(false);
  const [overall, setOverall] = useState<Choice>("tie");
  const [traction, setTraction] = useState<Choice>("tie");
  const [rigor, setRigor] = useState<Choice>("tie");
  const [confidence, setConfidence] = useState(3);
  const [note, setNote] = useState("");

  const active = useMemo(
    () => runs.find((view) => view.run.id === selectedRun) ?? runs[0] ?? null,
    [runs, selectedRun],
  );

  useEffect(() => {
    let cancelled = false;
    void Promise.all([api.experimentCatalog(), api.experiments(paperId)])
      .then(([nextCatalog, nextRuns]) => {
        if (cancelled) return;
        setCatalog(nextCatalog);
        setRuns(nextRuns);
        setSelectedExperiment(nextCatalog.experiments[0]?.id ?? "");
        setSelectedRun(nextRuns[0]?.run.id ?? null);
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : "Could not load experiments");
      });
    return () => { cancelled = true; };
  }, [paperId]);

  useEffect(() => {
    if (active?.run.status !== "running") return;
    const timer = window.setInterval(() => {
      void api.experiment(paperId, active.run.id).then((next) => {
        setRuns((current) => [next, ...current.filter((item) => item.run.id !== next.run.id)]);
      }).catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "Could not refresh experiment");
      });
    }, 3000);
    return () => window.clearInterval(timer);
  }, [active?.run.id, active?.run.status, paperId]);

  const start = async (): Promise<void> => {
    if (selectedExperiment.length === 0 || provider === "heuristic") return;
    setStarting(true);
    setError(null);
    try {
      const next = await api.startExperiment(paperId, selectedExperiment, provider);
      setRuns((current) => [next, ...current]);
      setSelectedRun(next.run.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not start experiment");
    } finally {
      setStarting(false);
    }
  };

  const judge = async (): Promise<void> => {
    if (active === null) return;
    setJudging(true);
    setError(null);
    const judgment: ExperimentJudgment = {
      overall,
      early_traction: traction,
      rigor,
      confidence,
      note,
    };
    try {
      const next = await api.judgeExperiment(paperId, active.run.id, judgment);
      setRuns((current) => current.map((item) => item.run.id === next.run.id ? next : item));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not save judgment");
    } finally {
      setJudging(false);
    }
  };

  return (
    <div className="experiment-backdrop" role="presentation" onMouseDown={onClose}>
      <section
        className="experiment-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Learning-ramp experiments"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <span className="eyebrow">Prompt lab</span>
            <h2>Which ramp makes this paper click?</h2>
          </div>
          <button type="button" onClick={onClose} aria-label="Close experiments">×</button>
        </header>

        <div className="experiment-toolbar">
          <label>
            <span>Single dial</span>
            <select value={selectedExperiment} onChange={(event) => setSelectedExperiment(event.target.value)}>
              {catalog?.experiments.map((experiment) => (
                <option key={experiment.id} value={experiment.id}>{experiment.name}</option>
              ))}
            </select>
          </label>
          <button type="button" onClick={() => void start()} disabled={starting || provider === "heuristic"}>
            {starting ? "Starting…" : "Run blind A/B"}
          </button>
          <label>
            <span>Saved run</span>
            <select value={active?.run.id ?? ""} onChange={(event) => setSelectedRun(event.target.value)}>
              {runs.map((view) => (
                <option key={view.run.id} value={view.run.id}>
                  {view.run.experiment_name} · {view.run.status}
                </option>
              ))}
            </select>
          </label>
        </div>

        {provider === "heuristic" && <p className="experiment-notice">Choose Codex or Claude as the reader to run prompt experiments.</p>}
        {error !== null && <p className="experiment-error">{error}</p>}
        {active === null && <p className="experiment-empty">No runs yet. Choose one dial and run a blind comparison.</p>}

        {active !== null && (
          <>
            <div className="experiment-question">
              <strong>{active.run.experiment_name}</strong>
              <span>{active.run.question}</span>
              <small>{active.run.model} · {active.run.reasoning_effort} effort · identities {active.judged ? "revealed" : "blinded"}</small>
            </div>
            {active.run.status === "running" && <p className="experiment-running">Both arms are running independently. This view will refresh automatically.</p>}
            <div className="experiment-arms">
              {active.run.arms.map((arm) => (
                <article className="experiment-arm" key={arm.blind_label}>
                  <header>
                    <h3>Ramp {arm.blind_label}</h3>
                    {active.judged && <span>{arm.variant_label}</span>}
                  </header>
                  {active.judged && <p className="experiment-reveal">{arm.variant_instruction}</p>}
                  {arm.output !== null && <Ramp ramp={arm.output} />}
                  {arm.error !== null && <p className="experiment-error">{arm.error}</p>}
                  {arm.output === null && arm.error === null && <p>Generating…</p>}
                </article>
              ))}
            </div>

            {active.run.status === "completed" && !active.judged && (
              <form className="experiment-judgment" onSubmit={(event) => { event.preventDefault(); void judge(); }}>
                <ChoiceField label="Overall smoother ramp" value={overall} onChange={setOverall} />
                <ChoiceField label="Earlier traction" value={traction} onChange={setTraction} />
                <ChoiceField label="More rigorous" value={rigor} onChange={setRigor} />
                <label>
                  <span>Confidence · {confidence}/5</span>
                  <input type="range" min="1" max="5" value={confidence} onChange={(event) => setConfidence(Number(event.target.value))} />
                </label>
                <label className="experiment-note">
                  <span>What made the difference? (optional)</span>
                  <textarea value={note} maxLength={4000} onChange={(event) => setNote(event.target.value)} />
                </label>
                <button type="submit" disabled={judging}>{judging ? "Saving…" : "Save judgment & reveal prompts"}</button>
              </form>
            )}
          </>
        )}
      </section>
    </div>
  );
}
