import { useEffect, useRef } from "react";

type HelpOverlayProps = { onClose: () => void };

const GROUPS = [
  {
    title: "Move",
    commands: [
      ["h j k l / arrows", "Move through tiles, lists, pages, and evidence sentences"],
      ["g g / G", "First / last tile"],
      ["[ / ]", "Previous / next paper or PDF page"],
      ["Ctrl-d / Ctrl-u", "Half-screen down / up; turn PDF page at the scrolling edge"],
      ["PageDown / PageUp", "Half-screen in PDF; full-screen in text views"],
      ["/", "Regex search in the paper; filter library or Glossary there"],
      ["b · j/k · ↵", "Open, move through, and choose from the mobile library"],
    ],
  },
  {
    title: "Read",
    commands: [
      ["↵ or o", "Open the selected paper or focused section"],
      ["Home / End", "First / last paper in the home grid"],
      ["d", "Open its contextual digest"],
      ["g", "Open the technical glossary (pause after one g)"],
      ["m", "Toggle PDF / requested Markdown reconstruction"],
      ["p", "Toggle overview / source PDF"],
      ["f · hint", "Follow a visible citation, figure, table, or PDF link"],
      ["Tab · Enter / Backspace / Esc", "Inspect a link hint · follow / edit hint / cancel"],
      ["Ctrl-o", "Return to the reading position before following a link"],
      ["2", "Toggle one-page / two-page PDF view"],
      ["W / H", "Fit PDF width / height (resets zoom)"],
      ["g W / g H", "Fit visible PDF content width / height with a small margin (resets zoom)"],
      ["P", "Toggle paged / continuous PDF reading"],
      ["R", "Rotate continuous scrolling: vertical / horizontal"],
      ["+ / −", "Show one fewer / one more page column in Overview"],
      ["I", "Toggle dark ink / true colour for every rendered PDF page"],
    ],
  },
  {
    title: "Select & ask",
    commands: [
      ["/ pattern ↵ · n / N", "Search source text and move through matches"],
      ["v · ap / ip", "Select around / inside a source paragraph"],
      ["aw / iw · aW / iW · as / is", "Word, whitespace-delimited WORD, or sentence objects"],
      ["h / l · w / b · e / E", "Select by character, word start, or word end"],
      ["( / )", "Extend to the previous / next sentence start"],
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
    title: "PDF · Cursor mode",
    commands: [
      ["C", "Toggle the read-only block cursor"],
      ["L", "Line numbers: relative (default), absolute, off"],
      ["j / k · ↑ / ↓", "Next / previous printed line; counts work (3j)"],
      ["h / l · ← / →", "Previous / next character"],
      ["0 / $ · g g / G", "Line start / end; document start / end"],
      ["PageDown / PageUp · Ctrl-d / Ctrl-u", "Move the cursor half a screen down / up"],
      ["v", "Select from the cursor; figures and tables are one outlined item"],
      ["y y", "Copy this line, or a figure/table caption with its source reference"],
      ["y + motion · y ap", "Copy through a movement, or copy the logical paragraph"],
      ["Esc / q", "Cancel the pending action or selection, then leave Cursor mode"],
    ],
  },
  {
    title: "Notes · Vim",
    commands: [
      ["i / a / o · Esc", "Enter Insert mode / return to Normal without closing notes"],
      ["v / V / Ctrl-v", "Visual / line / block selection"],
      ["/ · ? · n / N", "Search forward / backward in the note; repeat / reverse"],
      ["ciw · dap · yy · p", "Change a word, delete a paragraph, yank and put"],
      ["u / U or Ctrl-r · .", "Undo / redo; repeat the last change"],
      ["q{register} … q · @{register}", "Record and replay a macro"],
      [":w / Ctrl-s", "Save the Markdown note"],
      [":source", "Reload the configured .vimrc without losing edits or undo"],
      [":q · :wq / :x / ZZ · :q! / ZQ", "Quit · save and quit · discard and quit"],
      [":%s/old/new/g", "Substitute throughout this note"],
      ["Ctrl-w h / l / w · :reader", "Focus reader / notes / other pane; return to reader"],
    ],
  },
  {
    title: "Application",
    commands: [
      ["b", "Toggle the library"],
      ["F1", "Toggle the library outside the notes buffer"],
      ["F10", "Fuzzy-find and switch articles"],
      ["f in library", "Filter to mapped papers while the library has focus"],
      [":analyze", "Analyze with the selected provider"],
      [":supercut", "Create a source-verified cut with Lysilogos"],
      [":references", "Save citations, find papers, and check claims"],
      [":", "Open the command menu"],
      ["Q", "Toggle the live processing queue"],
      ["q / esc", "Leave selection, close a panel, return to map, then home"],
      ["T", "Show / pin reader controls (also reveal at the top edge)"],
      ["E", "Open / close Markdown notes"],
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
