import type { ListMembership, PaperMetadata } from "../types";
import "./ListPage.css";

const CHIP_LIMIT = 3;

type PaperHeadingProps = {
  metadata: PaperMetadata;
  /** The lists the paper is on; the first three show as chips. */
  memberships?: ListMembership[];
  onOpenList?: (listId: string) => void;
  /** Opens the full membership popover (`M`). */
  onMembership?: () => void;
};

function chipLabel(membership: ListMembership): string {
  const topic = membership.topics[0];
  return topic === undefined ? membership.title : `${membership.title} · ${topic}`;
}

export function PaperHeading({ metadata, memberships = [], onOpenList, onMembership }: PaperHeadingProps) {
  const { title, authors, year } = metadata;
  const longList = authors.length > 6 || authors.join(", ").length > 150;
  return <section className="paper-heading" aria-label="Paper title and authors">
    <h1>{title}</h1>
    <div className="paper-byline">
      {longList ? <details className="paper-authors">
        <summary>{authors.slice(0, 2).join(", ")} <span className="author-count">and {authors.length - 2} more</span></summary>
        <ol aria-label="Full author list">{authors.map((author, i) => <li key={`${i}:${author}`}>{author}</li>)}</ol>
      </details> : <span>{authors.join(", ") || "Unknown author"}</span>}
      {year !== null && <span className="paper-year">{year}</span>}
    </div>
    {memberships.length > 0 && <div className="paper-lists" aria-label="Reading lists">
      {memberships.slice(0, CHIP_LIMIT).map((membership) => <button
        key={membership.node_id}
        type="button"
        className="paper-list-chip"
        title={`Open “${membership.title}”`}
        onClick={() => onOpenList?.(membership.list_id)}
      >{chipLabel(membership)}</button>)}
      {memberships.length > CHIP_LIMIT && <span className="paper-lists-more">+{memberships.length - CHIP_LIMIT}</span>}
      <button type="button" className="paper-list-chip" title="Lists this paper is on" onClick={() => onMembership?.()}>lists <kbd>M</kbd></button>
    </div>}
  </section>;
}
