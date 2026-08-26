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
          ["[ ]", "page"],
          ["I", "ink / colour"],
          ["+ −", "zoom"],
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
