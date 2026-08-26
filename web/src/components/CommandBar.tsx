type CommandBarProps = {
  view: "atlas" | "markdown" | "pdf";
  panelOpen: boolean;
};

export function CommandBar({ view, panelOpen }: CommandBarProps) {
  const commands = panelOpen
    ? [
        ["j k", "move"],
        ["v", "visual"],
        ["c", "clarify"],
        ["y", "copy"],
        ["esc", "close"],
      ]
    : view === "pdf"
      ? [
          ["C-u C-d / PgUp PgDn", "page"],
          ["2", "one / two pages"],
          ["I", "ink / colour"],
          ["+ −", "zoom"],
          ["q", "queue"],
          [":", "commands"],
          ["p", "atlas"],
          ["?", "keys"],
        ]
      : view === "markdown"
        ? [
            ["j k", "scroll"],
            ["g g / G", "top / end"],
            ["m", "atlas"],
            ["p", "PDF"],
            ["F10", "switch article"],
            ["q", "queue"],
            [":", "commands"],
          ]
      : [
          ["h j k l / arrows", "move"],
          ["↵", "digest"],
          ["H", "AI evidence"],
          ["U", "my marks"],
          ["v", "mark source"],
          ["g", "gloss"],
          ["m", "Markdown"],
          ["p", "PDF"],
          ["q", "queue"],
          [":", "commands"],
          ["?", "keys"],
        ];

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
