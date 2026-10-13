import { useId, useState } from "react";

import { findTagByName, markAfterToggle, nextTagColor, normalizeTagColor, signalSummary } from "../lib/relevance";
import type { MarkView, ProminenceView, TagView, WorkMarkName } from "../types";

type LiteratureCurationProps = {
  mark: MarkView | null;
  prominence: ProminenceView | undefined;
  error: string | null;
  /** `null` clears the mark. The note is saved with the mark. */
  onSetMark: (mark: WorkMarkName | null, note: string | null) => void;
  /** Every tag in the workspace. */
  tags: readonly TagView[];
  /** Tag IDs assigned to this Work. */
  assignedTagIds: readonly string[];
  tagError: string | null;
  /** Assign an existing tag, or create one with `color` first, by name. */
  onAssignTag: (name: string, color: string) => void;
  onUnassignTag: (tag: TagView) => void;
};

const MARK_BUTTONS: ReadonlyArray<{ mark: WorkMarkName; label: string; key: string }> = [
  { mark: "pinned", label: "Pin", key: "p" },
  { mark: "downplayed", label: "Downplay", key: "d" },
  { mark: "hidden", label: "Hide", key: "h" },
];

/** CSS class the graph uses to focus the tag input on `t`. */
export const TAG_INPUT_CLASS = "literature-tag-input";

/** The "Relevance" and "Tags" sections of the Work panel. Keys p/d/h/t are handled by the graph. */
export function LiteratureCuration({
  mark, prominence, error, onSetMark, tags, assignedTagIds, tagError, onAssignTag, onUnassignTag,
}: LiteratureCurationProps) {
  const [noteDraft, setNoteDraft] = useState(mark?.note ?? "");
  const [tagDraft, setTagDraft] = useState("");
  const [colorDraft, setColorDraft] = useState<string | null>(null);
  const listId = useId();
  const current = mark?.mark ?? null;
  const trimmedNote = noteDraft.trim();
  const assigned = new Set(assignedTagIds);
  const assignedTags = tags.filter((tag) => assigned.has(tag.id));
  const suggestions = tags.filter((tag) => !assigned.has(tag.id));
  const existing = findTagByName(tags, tagDraft);
  const color = normalizeTagColor(colorDraft ?? "") ?? nextTagColor(tags);
  const submitTag = (): void => {
    if (tagDraft.trim() === "") return;
    onAssignTag(tagDraft, existing === undefined ? color : existing.color);
    setTagDraft("");
    setColorDraft(null);
  };
  return (
    <>
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
      <section className="literature-tags" aria-label="Tags">
        <h4>Tags</h4>
        {assignedTags.length > 0 && (
          <ul className="literature-tag-list">
            {assignedTags.map((tag) => (
              <li key={tag.id} className="literature-tag-assigned" data-tag-id={tag.id}>
                <i className="literature-swatch" style={{ background: tag.color }} />
                {tag.name}
                <button type="button" aria-label={`Remove tag ${tag.name}`} onClick={() => onUnassignTag(tag)}>×</button>
              </li>
            ))}
          </ul>
        )}
        <div className="literature-tag-form">
          <input
            className={TAG_INPUT_CLASS}
            aria-label="Add a tag"
            placeholder="Add a tag (t) — Enter assigns or creates"
            list={listId}
            maxLength={96}
            value={tagDraft}
            onChange={(event) => setTagDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
              event.preventDefault();
              submitTag();
            }}
          />
          <datalist id={listId}>
            {suggestions.map((tag) => <option key={tag.id} value={tag.name} />)}
          </datalist>
          <input
            type="color"
            className="literature-tag-color"
            aria-label="Color for a new tag"
            title={existing === undefined ? "Color for a new tag" : "This tag already exists; its color is kept"}
            value={existing === undefined ? color : existing.color}
            disabled={existing !== undefined}
            onChange={(event) => setColorDraft(event.target.value)}
          />
        </div>
        {tagError !== null && <p className="literature-hint literature-label-error" role="alert">{tagError}</p>}
      </section>
    </>
  );
}
