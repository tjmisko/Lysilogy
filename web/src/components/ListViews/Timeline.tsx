import { useCallback, useMemo } from "react";

import { timelineGrid, timelineGridColumns, timelineYearMove } from "../../lib/listViewNav";
import { statusGlyph, timeline, type ItemRow } from "../../lib/listViews";
import { useColumnKeys } from "./ColumnKeys";
import type { ListViewProps } from "./props";
import "./ListColumns.css";

/** View 3: works on a year axis, oldest first; the selected work's derived citations are marked. */
export function Timeline(props: ListViewProps) {
  const { detail, selectedId, onSelect, keyboardEnabled } = props;
  const model = useMemo(() => timeline(detail), [detail]);
  const grid = useMemo(() => timelineGrid(model.buckets, model.undated), [model]);
  const columns = useMemo(() => timelineGridColumns(grid), [grid]);
  const rowById = useMemo(() => new Map([...model.buckets.flatMap((bucket) => bucket.rows), ...model.undated].map((row) => [row.id, row])), [model]);
  const move = useCallback((delta: -1 | 1) => timelineYearMove(columns, selectedId, delta), [columns, selectedId]);
  useColumnKeys(keyboardEnabled, move, onSelect);

  const titles = useMemo(() => new Map([...model.buckets.flatMap((bucket) => bucket.rows), ...model.undated].map((row) => [row.id, row.title])), [model]);
  const relation = useMemo(() => {
    const cites = new Map<string, string[]>();
    const citedBy = new Map<string, string[]>();
    for (const link of model.links) {
      cites.set(link.from, [...(cites.get(link.from) ?? []), link.to]);
      citedBy.set(link.to, [...(citedBy.get(link.to) ?? []), link.from]);
    }
    return { cites, citedBy };
  }, [model.links]);

  if (model.buckets.length === 0 && model.undated.length === 0) {
    return <div className="list-empty"><p>Nothing to place on a timeline yet.</p></div>;
  }

  const linked = new Set<string>(selectedId === null ? [] : [...(relation.cites.get(selectedId) ?? []), ...(relation.citedBy.get(selectedId) ?? [])]);
  const card = (row: ItemRow) => {
    const cites = relation.cites.get(row.id) ?? [];
    const citedBy = relation.citedBy.get(row.id) ?? [];
    const names = (ids: string[]): string => ids.map((id) => titles.get(id) ?? id).join("; ");
    return <li
      key={row.id}
      className={`list-col-work ${selectedId === row.id ? "is-selected" : ""} ${linked.has(row.id) ? "is-linked" : ""}`}
      data-node-id={row.id}
      data-kind={row.kind}
      data-status={row.status}
      onClick={() => onSelect(row.id)}
      onDoubleClick={() => props.onOpen(row.id)}
    >
      <span className="list-col-glyph" aria-hidden="true">{row.kind === "claim" ? "?" : statusGlyph(row.status)}</span>
      <span className="list-col-title">{row.title}</span>
      {cites.length > 0 && <span className="list-col-cite" title={`Cites ${names(cites)}`}>cites {cites.length}</span>}
      {citedBy.length > 0 && <span className="list-col-cite" title={`Cited by ${names(citedBy)}`}>cited by {citedBy.length}</span>}
    </li>;
  };

  const column = (index: number) => {
    const col = grid.columns[index];
    if (col === undefined) return null;
    const label = col.year === null ? "Undated" : String(col.year);
    return <section key={col.key} className="list-col" data-year={col.year ?? "undated"}>
      <h3 className="list-col-head">{label}</h3>
      <ol>{col.entries.map((entry) => rowById.get(entry.id)).filter((row): row is ItemRow => row !== undefined).map(card)}</ol>
    </section>;
  };
  // The grid puts the unknown column first; the walk order (and h/l) keeps undated rows last.
  const unknownIndex = grid.columns.findIndex((col) => col.year === null);
  return <div className="list-columns list-timeline" aria-label="Timeline">
    {grid.slots.map((slot) => {
      if (slot.kind === "empty_year") return <div key={`empty:${slot.year}`} className="list-col-empty" title={String(slot.year)} aria-hidden="true" />;
      if (slot.kind === "gap") {
        const range = `${slot.fromYear}\u2013${slot.toYear}`;
        return <div key={`gap:${slot.fromYear}`} className="list-col-gapslot" title={`${slot.toYear - slot.fromYear + 1} years without a paper`}>{range}</div>;
      }
      return slot.columnIndex === unknownIndex ? null : column(slot.columnIndex);
    })}
    {unknownIndex >= 0 && column(unknownIndex)}
  </div>;
}
