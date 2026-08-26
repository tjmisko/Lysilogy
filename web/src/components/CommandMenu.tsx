import { useEffect, useMemo, useRef, useState } from "react";

type CommandMenuProps = {
  onClose: () => void;
  onExecute: (command: string) => void;
};

const COMMANDS = [
  ["analyze", "Analyze with the selected reader"],
  ["analyze codex", "Analyze with Codex"],
  ["analyze claude", "Analyze with Claude"],
  ["analyze heuristic", "Run the offline structural pass"],
  ["queue", "Show live analysis jobs and feedback"],
  ["feedback", "Send feedback about the current atlas"],
  ["library", "Toggle the paper library"],
  ["switch", "Fuzzy-find another article"],
  ["abstract", "Open the paper orientation"],
  ["overview", "Open the argument overview"],
  ["atlas", "Open the argument overview"],
  ["glossary", "Open the technical glossary"],
  ["text", "Open the full paper text"],
  ["markdown", "Open reconstructed text"],
  ["pdf", "Open the source PDF"],
  ["spread", "Toggle the two-page PDF spread"],
  ["ink", "Toggle dark ink / original colours"],
  ["help", "Open the complete key map"],
] as const;

export function CommandMenu({ onClose, onExecute }: CommandMenuProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const matches = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (needle.length === 0) return [...COMMANDS];
    return COMMANDS.filter(([command, description]) =>
      `${command} ${description}`.toLocaleLowerCase().includes(needle),
    );
  }, [query]);
  const activeIndex = Math.min(active, Math.max(0, matches.length - 1));

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const execute = (): void => {
    const typed = query.trim();
    const exact = COMMANDS.find(([command]) => command === typed)?.[0];
    const value = exact ?? matches[activeIndex]?.[0] ?? typed;
    onExecute(value);
  };

  return (
    <div className="command-menu-backdrop" role="presentation" onMouseDown={onClose}>
      <section
        className="command-menu"
        role="dialog"
        aria-modal="true"
        aria-label="Command menu"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <label className="command-input">
          <span aria-hidden="true">:</span>
          <input
            ref={inputRef}
            value={query}
            aria-label="Command"
            autoComplete="off"
            spellCheck={false}
            placeholder="analyze"
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                onClose();
              } else if (event.key === "ArrowDown" || (event.ctrlKey && event.key === "n")) {
                event.preventDefault();
                setActive((index) => Math.min(Math.max(0, matches.length - 1), index + 1));
              } else if (event.key === "ArrowUp" || (event.ctrlKey && event.key === "p")) {
                event.preventDefault();
                setActive((index) => Math.max(0, index - 1));
              } else if (event.key === "Tab") {
                const match = matches[activeIndex];
                if (match !== undefined) {
                  event.preventDefault();
                  setQuery(match[0]);
                }
              } else if (event.key === "Enter") {
                event.preventDefault();
                execute();
              }
            }}
          />
          <kbd>esc</kbd>
        </label>
        <div className="command-results" role="listbox" aria-label="Available commands">
          {matches.map(([command, description], index) => (
            <button
              key={command}
              type="button"
              role="option"
              aria-selected={index === activeIndex}
              className={index === activeIndex ? "is-active" : ""}
              onMouseEnter={() => setActive(index)}
              onClick={() => onExecute(command)}
            >
              <strong>:{command}</strong>
              <span>{description}</span>
            </button>
          ))}
          {matches.length === 0 && <p>No matching command. Press Enter to try it.</p>}
        </div>
        <footer><kbd>↑ ↓</kbd> choose · <kbd>tab</kbd> complete · <kbd>↵</kbd> run</footer>
      </section>
    </div>
  );
}
