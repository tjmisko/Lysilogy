type CommandBarProps = {
  view: "atlas" | "pdf";
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
          ["i", "ink / colour"],
          ["+ −", "zoom"],
          ["p", "atlas"],
          ["?", "keys"],
        ]
      : [
          ["h j k l", "move"],
          ["↵", "digest"],
          ["g", "gloss"],
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
