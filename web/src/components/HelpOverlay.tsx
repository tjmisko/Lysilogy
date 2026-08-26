import { useEffect, useRef } from "react";

type HelpOverlayProps = { onClose: () => void };

const GROUPS = [
  {
    title: "Move",
    commands: [
      ["h j k l / arrows", "Move through tiles, lists, pages, and evidence sentences"],
      ["g g / G", "First / last tile"],
      ["[ / ]", "Previous / next paper or PDF page"],
      ["/", "Search the library or Gloss"],
      ["b · j/k · ↵", "Open, move through, and choose from the mobile library"],
    ],
  },
  {
    title: "Read",
    commands: [
      ["↵ or o", "Open the focused section"],
      ["d", "Open its contextual digest"],
      ["g", "Open the technical glossary (pause after one g)"],
      ["m", "Toggle overview / reconstructed full text"],
      ["p", "Toggle overview / source PDF"],
      ["I", "Toggle dark ink / true colour for every rendered PDF page"],
    ],
  },
  {
    title: "Select & ask",
    commands: [
      ["v", "Start visual paragraph selection"],
      ["j / k", "Extend the selection"],
      ["o", "Swap the active end"],
      ["c", "Clarify the selection in context"],
      ["y", "Copy the selection"],
      ["H", "Toggle AI-cited prehighlights in the source map"],
      ["U", "Toggle reader-created highlights in the source map"],
      ["v · j/k · space", "Select and save exact PDF sentence ranges"],
    ],
  },
  {
    title: "Application",
    commands: [
      ["b", "Toggle the library"],
      ["F1", "Toggle the library from anywhere"],
      ["F10", "Fuzzy-find and switch articles"],
      ["f", "Filter to mapped papers while the library is open"],
      ["a", "Analyze with the selected provider"],
      ["esc", "Leave a mode or close a panel"],
      ["?", "Show this reference"],
    ],
  },
] as const;

export function HelpOverlay({ onClose }: HelpOverlayProps) {
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    closeRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <section
        className="help-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="keymap-heading"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <span className="eyebrow">No mouse required</span>
            <h2 id="keymap-heading">Key map</h2>
          </div>
          <button ref={closeRef} className="icon-button" type="button" onClick={onClose} aria-label="Close key map">
            ×
          </button>
        </header>
        <div className="key-groups">
          {GROUPS.map((group) => (
            <section key={group.title}>
              <h3>{group.title}</h3>
              {group.commands.map(([keys, action]) => (
                <div key={keys}>
                  <kbd>{keys}</kbd>
                  <span>{action}</span>
                </div>
              ))}
            </section>
          ))}
        </div>
        <p>
          The top bar moves from Abstract to Overview to Glossary to Text, increasing detail at
          each step. In a digest, visual mode selects whole semantic fragments—digest paragraphs,
          quotes, and explanations—so keyboard selection remains stable across responsive layouts.
        </p>
      </section>
    </div>
  );
}
