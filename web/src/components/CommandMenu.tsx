import { useEffect, useMemo, useRef, useState } from "react";

import { ARGUMENT_COMMANDS } from "../lib/listRoute";

type CommandMenuProps = {
  onClose: () => void;
  onExecute: (command: string) => void;
};

const COMMANDS = [
  ["analyze", "Analyze with the selected reader"],
  ["analyze codex", "Analyze with Codex"],
  ["analyze claude", "Analyze with Claude"],
  ["queue", "Show live analysis jobs and feedback"],
  ["feedback", "Send feedback about the current atlas"],
  ["experiment", "Compare two learning-ramp prompts blind"],
  ["supercut", "Ask Lysilogos for a ten-paragraph or six-page cut"],
  ["references", "Save citations, find papers, and connect evidence"],
  ["home", "Open the paper library home"],
  ["library", "Toggle the paper library"],
  ["switch", "Fuzzy-find another article"],
  ["refresh-abstract", "Re-extract and verify only the authored abstract"],
  ["refresh-structure", "Regroup the paper into coherent reading sections"],
  ["abstract", "Open the paper orientation"],
  ["overview", "Open the argument overview"],
  ["atlas", "Open the argument overview"],
  ["glossary", "Open the technical glossary"],
  ["text", "Open the full paper text"],
  ["pdf", "Open the source PDF"],
  ["spread", "Toggle the two-page PDF spread"],
  ["ink", "Toggle dark ink / original colours"],
  ["help", "Open the complete key map"],
  ["lists", "Open the reading lists"],
  ["list", "Open a reading list by title: :list <title>"],
  ["list-new", "Create a reading list: :list-new <title>"],
  ["generate", "Draft a list for a learning goal: :generate <goal>"],
  ["history", "Open the reading history"],
  ["read", "Mark the current paper read"],
  ["reading", "Mark the current paper reading"],
  ["unread", "Mark the current paper unread"],
  ["order", "Write the list order: :order topological | year | foundational-first"],
  ["verify", "Retry the list's pending claims"],
  ["delete-list", "Delete a list: :delete-list <its exact title>"],
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

  // A command that needs an argument waits for it instead of running bare.
  const choose = (value: string): void => {
    if (ARGUMENT_COMMANDS.includes(value)) setQuery(`${value} `);
    else onExecute(value);
  };

  const execute = (): void => {
    const typed = query.trim();
    const first = typed.split(/\s+/u)[0] ?? "";
    if (ARGUMENT_COMMANDS.includes(first.toLowerCase()) && typed.length > first.length) {
      onExecute(typed);
      return;
    }
    const exact = COMMANDS.find(([command]) => command === typed)?.[0];
    choose(exact ?? matches[activeIndex]?.[0] ?? typed);
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
              onClick={() => choose(command)}
            >
              <strong>:{command}</strong>
              <span>{description}</span>
            </button>
          ))}
          {matches.length === 0 && <p>No matching command. Press Enter to try it.</p>}
        </div>
      </section>
    </div>
  );
}
