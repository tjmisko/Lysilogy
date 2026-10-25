import { statusGlyph, type ItemRow } from "../../lib/listViews";
import type { ListViewProps } from "./props";

/** A plain row for the views that do not draw their own yet: glyph, title, year and a detail. */
export function StubRow({ row, props, detail }: { row: ItemRow; props: ListViewProps; detail?: string }) {
  const selected = props.selectedId === row.id;
  return (
    <li
      className={`list-row ${selected ? "is-selected" : ""}`}
      data-node-id={row.id}
      data-kind={row.kind}
      data-status={row.status}
      onClick={() => props.onSelect(row.id)}
      onDoubleClick={() => props.onOpen(row.id)}
    >
      <span className="list-row-ordinal">{row.ordinal}</span>
      <span className="list-row-status" aria-hidden="true">{row.kind === "claim" ? "?" : statusGlyph(row.status)}</span>
      <span className="list-row-main">
        <span className="list-row-line">
          <span className="list-row-title">{row.title}</span>
          {row.year !== null && <span className="list-row-year">{row.year}</span>}
          {detail !== undefined && <span className="list-row-held">{detail}</span>}
        </span>
      </span>
    </li>
  );
}
