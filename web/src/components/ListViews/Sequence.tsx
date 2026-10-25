import { useMemo, type CSSProperties } from "react";

import { nextReadingStatus } from "../../lib/listKeys";
import { groupCursorId, sequenceModel } from "../../lib/listViewModel";
import { countsLabel, statusCounts, statusGlyph, type ItemRow, type TopicGroup } from "../../lib/listViews";
import type { AcquisitionState } from "../../types";
import { InlineInput } from "../InlineInput";
import type { ListViewProps } from "./props";

const FETCH_LABEL: Readonly<Record<AcquisitionState, string>> = {
  unresolved: "not held",
  identifier_found: "id found",
  link_found: "link found",
  downloaded: "saving",
  mapped: "fetched",
  unavailable: "unavailable",
};

function heldLabel(row: ItemRow): string | null {
  if (row.kind === "claim") return null;
  if (row.held) return "held";
  return row.fetch === null ? "not held" : FETCH_LABEL[row.fetch];
}

function contextLine(row: ItemRow, workTitles: Readonly<Record<string, string>>): string | null {
  const first = row.contextLinks[0];
  if (first === undefined) return null;
  const title = workTitles[first.work_id] ?? "a paper you read";
  const more = row.contextLinks.length > 1 ? ` +${row.contextLinks.length - 1}` : "";
  return `↔ ${title}${more}`;
}

function Row({ row, props }: { row: ItemRow; props: ListViewProps }) {
  const { selectedId, viewState, workTitles, tagColors, editingNoteId } = props;
  const selected = row.id === selectedId;
  const context = contextLine(row, workTitles);
  const held = heldLabel(row);
  const glyph = row.kind === "claim" ? "?" : statusGlyph(row.status);
  return (
    <li
      className={`list-row ${selected ? "is-selected" : ""} ${viewState.cutId === row.id ? "is-cut" : ""} ${viewState.edgeFrom === row.id ? "is-edge-from" : ""}`}
      data-node-id={row.id}
      data-kind={row.kind}
      data-status={row.status}
      data-claim-state={row.claimState ?? undefined}
      onClick={() => props.onSelect(row.id)}
      onDoubleClick={() => props.onOpen(row.id)}
    >
      <span className="list-row-ordinal">{row.ordinal}</span>
      <button
        type="button"
        className="list-row-status"
        tabIndex={-1}
        disabled={row.kind === "claim"}
        aria-label={row.kind === "claim" ? "Claim" : `${row.status}: cycle status`}
        title={row.kind === "claim" ? "Resolve the claim to give it a reading status" : `${row.status} (r)`}
        onClick={(event) => {
          event.stopPropagation();
          props.onSelect(row.id);
          if (row.workId !== null) void props.actions.setStatus(row.workId, nextReadingStatus(row.status));
        }}
      >{glyph}</button>
      <span className="list-row-main">
        <span className="list-row-line">
          <span className="list-row-title">{row.title}</span>
          {row.year !== null && <span className="list-row-year">{row.year}</span>}
          {row.kind === "claim" && <span className="list-row-claim" data-claim-state={row.claimState ?? undefined}>claim · {row.claimState ?? "unverified"}</span>}
          {row.node.review === "proposed" && <span className="list-row-proposed">proposed</span>}
          {row.topics.map((label) => {
            const color = tagColors[label.toLowerCase()];
            return <span key={label} className="list-row-topic" style={color === undefined ? undefined : ({ "--topic-color": color } as CSSProperties)}>{label}</span>;
          })}
          {held !== null && <span className="list-row-held" data-held={row.held ? "true" : "false"}>{held}</span>}
        </span>
        {editingNoteId === row.id
          ? (
            <InlineInput
              label="Note"
              className="list-row-input"
              initial={row.note ?? ""}
              placeholder="Why is this on the list?"
              onSave={(value) => props.onSaveNote(row.id, value)}
              onCancel={props.onCancelNote}
            />
          )
          : row.note !== null && <span className="list-row-note">{row.note}</span>}
        {context !== null && <span className="list-row-context">{context}</span>}
      </span>
    </li>
  );
}

function GroupHead({ group, props }: { group: TopicGroup; props: ListViewProps }) {
  const cursor = groupCursorId(group.key);
  const color = group.topic?.label == null ? undefined : props.tagColors[group.topic.label.toLowerCase()];
  return (
    <li
      className={`list-group-head ${props.selectedId === cursor ? "is-selected" : ""}`}
      data-group-key={group.key}
      data-folded={group.folded ? "true" : "false"}
      style={color === undefined ? undefined : ({ "--topic-color": color } as CSSProperties)}
      onClick={() => props.onSelect(cursor)}
      onDoubleClick={() => props.onOpen(cursor)}
    >
      <span aria-hidden="true">{group.folded ? "▸" : "▾"}</span>
      <span className="list-group-label">{group.label}</span>
      <span className="list-group-counts">{group.rows.length} · {countsLabel(statusCounts(group.rows.filter((row) => row.kind === "work")))}</span>
    </li>
  );
}

/** View 1: the flat list in order, or grouped by topic (`zg`). */
export function Sequence(props: ListViewProps) {
  const { detail, viewState } = props;
  const model = useMemo(() => sequenceModel(detail, viewState), [detail, viewState]);
  if (model.rows.length === 0 && (model.groups === null || model.groups.length === 0)) {
    const filtered = viewState.filter.trim() !== "";
    return <div className="list-empty" data-empty={filtered ? "filtered" : "list"}>
      {filtered
        ? <p>Nothing matches “{viewState.filter.trim()}”. Press <kbd>/</kbd> to change the filter.</p>
        : <p>This list is empty. Press <kbd>a</kbd> to add a paper by name, or <kbd>Y</kbd> on any paper to add it here.</p>}
    </div>;
  }
  return <ol className="list-sequence" aria-label="Reading sequence">
    {model.groups === null
      ? model.rows.map((row) => <Row key={row.id} row={row} props={props} />)
      : model.groups.map((group) => <GroupSection key={group.key} group={group} props={props} />)}
  </ol>;
}

function GroupSection({ group, props }: { group: TopicGroup; props: ListViewProps }) {
  return <>
    <GroupHead group={group} props={props} />
    {group.visible.map((row) => <Row key={row.id} row={row} props={props} />)}
  </>;
}
