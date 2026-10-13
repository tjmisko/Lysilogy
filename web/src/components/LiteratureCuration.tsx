import { useState } from "react";

import { markAfterToggle, signalSummary } from "../lib/relevance";
import type { MarkView, ProminenceView, WorkMarkName } from "../types";

type LiteratureCurationProps = {
  mark: MarkView | null;
  prominence: ProminenceView | undefined;
  error: string | null;
  /** `null` clears the mark. The note is saved with the mark. */
  onSetMark: (mark: WorkMarkName | null, note: string | null) => void;
};

const MARK_BUTTONS: ReadonlyArray<{ mark: WorkMarkName; label: string; key: string }> = [
  { mark: "pinned", label: "Pin", key: "p" },
  { mark: "downplayed", label: "Downplay", key: "d" },
  { mark: "hidden", label: "Hide", key: "h" },
];

/** The "Relevance" section of the Work panel. Keys p/d/h are handled by the graph. */
export function LiteratureCuration({ mark, prominence, error, onSetMark }: LiteratureCurationProps) {
  const [noteDraft, setNoteDraft] = useState(mark?.note ?? "");
  const current = mark?.mark ?? null;
  const trimmedNote = noteDraft.trim();
  return (
    <section className="literature-relevance" aria-label="Relevance" data-mark={current ?? "none"}>
      <h4>Relevance</h4>
      {prominence !== undefined && (
        <p className="literature-signals" data-score={prominence.score.toFixed(2)}>
          Score {prominence.score.toFixed(2)} — {signalSummary(prominence)}
        </p>
      )}
      <div className="literature-mark-buttons" role="group" aria-label="Mark this work">
        {MARK_BUTTONS.map((button) => (
          <button
            key={button.mark}
            type="button"
            aria-pressed={current === button.mark}
            data-mark-button={button.mark}
            onClick={() => onSetMark(markAfterToggle(current, button.mark), trimmedNote === "" ? null : trimmedNote)}
          >
            {button.label} <kbd>{button.key}</kbd>
          </button>
        ))}
      </div>
      <input
        className="literature-mark-note"
        aria-label="Note for this mark"
        placeholder={current === null ? "Mark first, then add a note" : "Note (Enter saves)"}
        maxLength={2000}
        value={noteDraft}
        disabled={current === null}
        onChange={(event) => setNoteDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== "Enter" || event.nativeEvent.isComposing || current === null) return;
          event.preventDefault();
          onSetMark(current, trimmedNote === "" ? null : trimmedNote);
        }}
      />
      {error !== null && <p className="literature-hint literature-label-error" role="alert">{error}</p>}
    </section>
  );
}
