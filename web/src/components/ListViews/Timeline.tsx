import { useCallback, useMemo } from "react";

import { EMPTY_RUN_MAX } from "../../lib/histogramLayout";
import { timelineColumns, timelineYearMove } from "../../lib/listViewNav";
import { statusGlyph, timeline, type ItemRow } from "../../lib/listViews";
import { useColumnKeys } from "./ColumnKeys";
import type { ListViewProps } from "./props";
import "./ListColumns.css";

/** Quiet years between two columns, as a gap note; short runs are not worth a note, as in the histogram. */
function gapLabel(years: number): string | null {
  return years > EMPTY_RUN_MAX ? `${years} years without a paper` : null;
}

/** View 3: works on a year axis, oldest first; the selected work's derived citations are marked. */
export function Timeline(props: ListViewProps) {
  const { detail, selectedId, onSelect, keyboardEnabled } = props;
  const model = useMemo(() => timeline(detail), [detail]);
  const columns = useMemo(() => timelineColumns(model.buckets, model.undated), [model]);
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

  return <div className="list-columns list-timeline" aria-label="Timeline">
    {model.buckets.map((bucket, index) => {
      const previous = model.buckets[index - 1];
      const gap = previous === undefined ? null : gapLabel(bucket.year - previous.year - 1);
      return <section key={bucket.year} className="list-col" data-year={bucket.year}>
        {gap !== null && <p className="list-col-gap">{gap}</p>}
        <h3 className="list-col-head">{bucket.year}</h3>
        <ol>{bucket.rows.map(card)}</ol>
      </section>;
    })}
    {model.undated.length > 0 && <section className="list-col" data-year="undated">
      <h3 className="list-col-head">Undated</h3>
      <ol>{model.undated.map(card)}</ol>
    </section>}
  </div>;
}
