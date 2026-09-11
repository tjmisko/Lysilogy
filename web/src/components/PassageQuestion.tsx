import { useEffect, useRef, useState } from "react";
import type { AnalysisProvider, Clarification } from "../types";
import { api } from "../lib/api";

type Props = { paperId: string; text: string; page: number; provider: AnalysisProvider; onClose: () => void };

export function PassageQuestion({ paperId, text, page, provider: initialProvider, onClose }: Props) {
  const [question, setQuestion] = useState("");
  const [provider, setProvider] = useState(initialProvider);
  const [answer, setAnswer] = useState<Clarification | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { input.current?.focus(); }, []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return <div className="passage-question-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="passage-question" role="dialog" aria-modal="true" aria-labelledby="passage-question-heading"
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Escape") { event.preventDefault(); onClose(); }
        if (event.key === "Tab") {
          const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), textarea, select'));
          const first = controls[0], last = controls.at(-1);
          if (event.shiftKey && event.target === first) { event.preventDefault(); last?.focus(); }
          if (!event.shiftKey && event.target === last) { event.preventDefault(); first?.focus(); }
        }
      }}>
      <header><div><span className="eyebrow">Source · page {page}</span><h2 id="passage-question-heading">Ask about this passage</h2></div>
        <button type="button" className="icon-button" aria-label="Close passage question" onClick={onClose}>×</button></header>
      <blockquote>{text}</blockquote>
      <form onSubmit={(event) => {
        event.preventDefault(); if (busy) return;
        setBusy(true); setError(null);
        void api.clarify(paperId, null, text, question, provider).then(setAnswer)
          .catch((reason: unknown) => setError(reason instanceof Error ? reason.message : "Could not explain this passage"))
          .finally(() => setBusy(false));
      }}>
        <label>What would you like to understand?<textarea ref={input} value={question} onChange={(event) => setQuestion(event.target.value)} rows={3} placeholder="Explain this passage, or ask a specific question." /></label>
        <div className="clarify-controls"><select aria-label="Clarification provider" value={provider} onChange={(event) => setProvider(event.target.value as AnalysisProvider)}>
          <option value="codex">Codex</option><option value="claude">Claude</option><option value="heuristic">Offline</option>
        </select><button type="submit" disabled={busy}>{busy ? "Reading source…" : "Ask"}</button></div>
      </form>
      {error !== null && <p className="inline-error" role="alert">{error}</p>}
      {answer !== null && <article className="passage-answer" aria-live="polite"><p>{answer.answer}</p>{answer.limitation && <small>{answer.limitation}</small>}</article>}
    </section>
  </div>;
}
