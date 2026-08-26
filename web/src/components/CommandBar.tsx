type CommandBarProps = {
  view: "abstract" | "overview" | "glossary" | "text";
  textMode: "markdown" | "pdf";
  panelOpen: boolean;
};

export function CommandBar({ view, textMode, panelOpen }: CommandBarProps) {
  let commands: string[][];
  if (panelOpen) {
    commands = [
      ["j k", "move"],
      ["v", "visual"],
      ["c", "clarify"],
      ["y", "copy"],
      ["esc", "close"],
    ];
  } else if (view === "text" && textMode === "pdf") {
    commands = [
      ["C-u C-d / PgUp PgDn", "page"],
      ["2", "one / two pages"],
      ["I", "ink / colour"],
      ["+ −", "zoom"],
      ["m", "reconstructed"],
      ["p", "overview"],
      ["q", "queue"],
      [":", "commands"],
      ["?", "keys"],
    ];
  } else if (view === "text") {
    commands = [
      ["j k", "scroll"],
      ["g g / G", "top / end"],
      ["m", "overview"],
      ["p", "PDF"],
      ["F10", "switch article"],
      ["q", "queue"],
      [":", "commands"],
    ];
  } else if (view === "glossary") {
    commands = [
      ["j k", "move"],
      ["↵", "expand"],
      ["/", "find concept"],
      ["esc", "overview"],
      ["F10", "switch article"],
      ["q", "queue"],
      [":", "commands"],
    ];
  } else if (view === "abstract") {
    commands = [
      ["j k", "scroll"],
      ["g", "glossary"],
      ["m", "full text"],
      ["p", "PDF"],
      ["q", "queue"],
      [":", "commands"],
      ["?", "keys"],
    ];
  } else {
    commands = [
      ["h j k l / arrows", "move"],
      ["↵", "digest"],
      ["+ −", "page grid"],
      ["H", "AI evidence"],
      ["U", "my marks"],
      ["v", "mark source"],
      ["g", "glossary"],
      ["m", "full text"],
      ["p", "PDF"],
      ["q", "queue"],
      [":", "commands"],
      ["?", "keys"],
    ];
  }

  return (
    <nav className="command-bar" aria-label="Keyboard commands">
      {commands.map(([keys, action]) => (
        <span key={keys}>
          <kbd>{keys}</kbd> {action}
        </span>
      ))}
    </nav>
  );
}
