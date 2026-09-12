const pending = new WeakMap<Document, number>();

/** Called before PDF shortcuts as well as the app's own shortcuts. */
export function handleNotesPaneKey(event: KeyboardEvent): boolean {
  if (event.defaultPrevented || event.isComposing) return false;
  const target = event.target instanceof Element ? event.target : null;
  const doc = target?.ownerDocument ?? document;
  const notes = doc.querySelector(".notes-panel");
  // Vim handles its own Ctrl-w sequences, including word deletion in Insert.
  if (notes === null || target?.closest(".notes-panel, input, textarea, select, [contenteditable=true]") != null) {
    pending.delete(doc);
    return false;
  }
  const prefixAt = pending.get(doc);
  pending.delete(doc);
  if (prefixAt !== undefined && Date.now() - prefixAt < 1500) {
    if (!event.metaKey && !event.altKey && ["w", "W", "l", "ArrowRight"].includes(event.key)) {
      notes.dispatchEvent(new Event("notes-focus"));
    } else if (!["h", "ArrowLeft", "Escape"].includes(event.key)) return false;
  } else if (event.ctrlKey && !event.metaKey && !event.altKey && event.key === "w") {
    pending.set(doc, Date.now());
  } else return false;
  event.preventDefault();
  event.stopImmediatePropagation();
  return true;
}
