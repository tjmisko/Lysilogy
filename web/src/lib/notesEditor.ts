import { EditorState, type Extension } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, drawSelection, highlightActiveLine, keymap, placeholder } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { notesVim, type NotesVimCommands } from "./notesVim";

type EditorOptions = NotesVimCommands & { text: string; parent: HTMLElement; onChange: (text: string) => void };

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

export function createNotesEditor({ text, parent, onChange, ...commands }: EditorOptions): EditorView {
  const extensions: Extension[] = [
    // Vim must precede ordinary keymaps, which then provide Insert-mode editing.
    notesVim(commands), history(),
    drawSelection(), highlightActiveLine(), markdown,
    placeholder("Write notes in Markdown…"),
    EditorView.contentAttributes.of({ "aria-label": "Paper notes", spellcheck: "true" }),
    EditorView.theme({ "&": { height: "100%" }, ".cm-scroller": { overflow: "auto" }, ".cm-content": { padding: "18px 0", minHeight: "100%" }, ".cm-line": { padding: "0 20px" } }, { dark: true }),
    keymap.of([
      { key: "Mod-s", run: () => { commands.onSave(); return true; } },
      indentWithTab,
      ...defaultKeymap, ...historyKeymap,
    ]),
    EditorView.updateListener.of((update) => {
      if (!update.docChanged) return;
      onChange(update.state.doc.toString());
    }),
  ];
  return new EditorView({ state: EditorState.create({ doc: text, extensions }), parent });
}
