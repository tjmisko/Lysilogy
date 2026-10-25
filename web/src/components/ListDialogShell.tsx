import type { ReactNode } from "react";

import "./ListPage.css";

/** The F10 switcher's frame, reused by every list dialog so they look alike. */
export function DialogShell(props: {
  eyebrow: string;
  title: string;
  hint?: string;
  onClose: () => void;
  children: ReactNode;
  footer: ReactNode;
}) {
  const { eyebrow, title, hint, onClose, children, footer } = props;
  return (
    <div className="switcher-backdrop" role="presentation" data-list-overlay="true" onMouseDown={onClose}>
      <section
        className="paper-switcher list-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <span className="eyebrow">{eyebrow}</span>
            <h2>{title}</h2>
          </div>
          {hint !== undefined && <kbd>{hint}</kbd>}
        </header>
        <div className="list-dialog-body">{children}</div>
        <footer>{footer}</footer>
      </section>
    </div>
  );
}
