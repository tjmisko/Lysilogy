import "./ListsPage.css";

export type HistoryPageProps = {
  /** Open a paper from the history (at its saved position, once capture lands). */
  onOpenPaper: (paperId: string) => void;
  /** Open the list picker for a paper or Work (`Y`). */
  onAddToList: (target: { paperId?: string; workId?: string }) => void;
  keyboardEnabled: boolean;
};

/** A placeholder for `#history`; the history page task replaces this file. */
export function HistoryPage({ keyboardEnabled }: HistoryPageProps) {
  return <section className="lists-page" aria-label="Reading history" data-keyboard={keyboardEnabled}>
    <header className="lists-head"><h1>Reading history</h1></header>
    <div className="lists-empty">
      <p>The reading history page is not in this build yet.</p>
    </div>
  </section>;
}
