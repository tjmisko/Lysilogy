import type { PaperMetadata } from "../types";

export function PaperHeading({ metadata }: { metadata: PaperMetadata }) {
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
  </section>;
}
