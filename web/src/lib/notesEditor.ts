import { EditorState, type Extension } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, drawSelection, highlightActiveLine, keymap, placeholder } from "@codemirror/view";

type Snapshot = { text: string; anchor: number; head: number };
type EditorOptions = { text: string; parent: HTMLElement; onChange: (text: string) => void; onSave: () => void };

/** Lightweight Markdown decorations for the notes buffer; the document stays plain Markdown. */
function markdownDecorations(view: EditorView) {
  const decorations: Array<{ from: number; to: number; className: string }> = [];
  let fenced = false;
  for (let number = 1; number <= view.state.doc.lines; number += 1) {
    const line = view.state.doc.line(number);
    if (/^\s*(```|~~~)/u.test(line.text)) {
      fenced = !fenced;
      decorations.push({ from: line.from, to: line.to, className: "notes-md-code" });
      continue;
    }
    if (fenced) { decorations.push({ from: line.from, to: line.to, className: "notes-md-code" }); continue; }
    if (/^#{1,6}\s/u.test(line.text)) decorations.push({ from: line.from, to: line.to, className: "notes-md-heading" });
    if (/^\s*>/u.test(line.text)) decorations.push({ from: line.from, to: line.to, className: "notes-md-quote" });
    const patterns: Array<[RegExp, string]> = [
      [/`[^`\n]+`/gu, "notes-md-code"],
      [/\*\*[^*\n]+\*\*|__[^_\n]+__/gu, "notes-md-strong"],
      [/(?<!\*)\*[^*\n]+\*(?!\*)|(?<!_)_[^_\n]+_(?!_)/gu, "notes-md-emphasis"],
      [/!?\[[^\]\n]+\]\([^\n)]*\)/gu, "notes-md-link"],
      [/^\s*(?:[-+*]|\d+\.)\s(?:\[[ xX]\]\s)?/gu, "notes-md-marker"],
    ];
    for (const [pattern, className] of patterns) {
      for (const match of line.text.matchAll(pattern)) decorations.push({ from: line.from + match.index, to: line.from + match.index + match[0].length, className });
    }
  }
  return Decoration.set(decorations.filter((mark) => mark.to > mark.from).map((mark) => Decoration.mark({ class: mark.className }).range(mark.from, mark.to)), true);
}

const markdown = ViewPlugin.fromClass(class {
  decorations;
  constructor(view: EditorView) { this.decorations = markdownDecorations(view); }
  update(update: { docChanged: boolean; view: EditorView }) { if (update.docChanged) this.decorations = markdownDecorations(update.view); }
}, { decorations: (plugin) => plugin.decorations });

export function createNotesEditor({ text, parent, onChange, onSave }: EditorOptions): EditorView {
  const undo: Snapshot[] = [], redo: Snapshot[] = [];
  let restoring = false, lastInput = 0;
  const snapshot = (state: EditorState): Snapshot => ({ text: state.doc.toString(), anchor: state.selection.main.anchor, head: state.selection.main.head });
  const restore = (view: EditorView, source: Snapshot[], destination: Snapshot[]): boolean => {
    const previous = source.pop();
    if (previous === undefined) return true;
    destination.push(snapshot(view.state));
    restoring = true;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: previous.text }, selection: { anchor: previous.anchor, head: previous.head }, scrollIntoView: true });
    restoring = false;
    lastInput = 0;
    return true;
  };
  const extensions: Extension[] = [
    EditorView.lineWrapping, drawSelection(), highlightActiveLine(), markdown,
    placeholder("Write notes in Markdown…"),
    EditorView.contentAttributes.of({ "aria-label": "Paper notes", spellcheck: "true" }),
    EditorView.theme({ "&": { height: "100%" }, ".cm-scroller": { overflow: "auto" }, ".cm-content": { padding: "18px 0", minHeight: "100%" }, ".cm-line": { padding: "0 20px" } }, { dark: true }),
    keymap.of([
      { key: "Mod-s", run: () => { lastInput = 0; onSave(); return true; } },
      { key: "Mod-z", run: (view) => restore(view, undo, redo), shift: (view) => restore(view, redo, undo) },
      { key: "Mod-y", run: (view) => restore(view, redo, undo) },
    ]),
    EditorView.domEventHandlers({ blur: () => { lastInput = 0; }, beforeinput: (event, view) => {
      if (event.inputType === "historyUndo" || event.inputType === "historyRedo") {
        event.preventDefault();
        return event.inputType === "historyUndo" ? restore(view, undo, redo) : restore(view, redo, undo);
      }
      return false;
    } }),
    EditorView.updateListener.of((update) => {
      if (!update.docChanged) return;
      if (!restoring) {
        const now = Date.now();
        const typing = update.transactions.some((transaction) => transaction.isUserEvent("input.type"));
        if (!typing || now - lastInput > 750 || undo.length === 0) undo.push(snapshot(update.startState));
        // Bound history even when a reader pastes a very large document.
        while (undo.length > 50 || (undo.length > 1 && undo.reduce((bytes, item) => bytes + item.text.length, 0) > 8 * 1024 * 1024)) undo.shift();
        redo.length = 0;
        lastInput = typing ? now : 0;
      }
      onChange(update.state.doc.toString());
    }),
  ];
  return new EditorView({ state: EditorState.create({ doc: text, extensions }), parent });
}
