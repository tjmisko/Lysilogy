import { EditorState, type Extension } from "@codemirror/state";
import { GutterMarker, gutter, EditorView } from "@codemirror/view";
import { indentUnit } from "@codemirror/language";

export type NotesVimOptions = {
  number: boolean; relativenumber: boolean; wrap: boolean; expandtab: boolean;
  tabstop: number; shiftwidth: number; textwidth: number; insertModeEscKeysTimeout: number; pcre: boolean;
};
export const defaultNotesVimOptions = (): NotesVimOptions => ({ number: false, relativenumber: false, wrap: true, expandtab: true, tabstop: 4, shiftwidth: 4, textwidth: 80, insertModeEscKeysTimeout: 200, pcre: true });
const aliases: Record<string, keyof NotesVimOptions> = { nu: "number", rnu: "relativenumber", et: "expandtab", ts: "tabstop", sw: "shiftwidth", tw: "textwidth" };

/** Parse only options whose behavior we implement, rather than accepting no-ops. */
export function setNotesVimOption(options: NotesVimOptions, token: string): void {
  const match = /^(\w+)(?:=(\d+)|([!&]))?$/u.exec(token);
  if (match === null || match[1] === undefined) throw new Error(`Unsupported option syntax: ${token}`);
  let name = match[1];
  let negate = false;
  let invert = match[3] === "!";
  if (name.startsWith("no")) { negate = true; name = name.slice(2); }
  else if (name.startsWith("inv")) { invert = true; name = name.slice(3); }
  const key = aliases[name] ?? name;
  if (!Object.hasOwn(options, key)) throw new Error(`Unsupported notes option: ${name}`);
  const typedKey = key as keyof NotesVimOptions;
  if (match[3] === "&") { Object.assign(options, { [key]: defaultNotesVimOptions()[typedKey] }); return; }
  const old = options[typedKey];
  if (typeof old === "boolean") {
    if (match[2] !== undefined) throw new Error(`${name} takes no numeric value`);
    Object.assign(options, { [key]: invert ? !old : !negate });
  } else {
    const value = Number(match[2]);
    const limit = key === "insertModeEscKeysTimeout" ? 5000 : key === "textwidth" ? 1000 : 32;
    if (negate || invert || match[2] === undefined || !Number.isInteger(value) || value < 1 || value > limit) throw new Error(`${name} requires a value between 1 and ${limit}`);
    Object.assign(options, { [key]: value });
  }
}

class LineLabel extends GutterMarker {
  constructor(readonly label: string) { super(); }
  eq(other: LineLabel): boolean { return this.label === other.label; }
  toDOM(): Node { return document.createTextNode(this.label); }
}

export function notesVimOptionExtensions(options: NotesVimOptions): Extension {
  return [
    EditorState.tabSize.of(options.tabstop), indentUnit.of(options.expandtab ? " ".repeat(options.shiftwidth) : "\t"),
    options.wrap ? EditorView.lineWrapping : [],
    options.number || options.relativenumber ? gutter({
      class: "cm-lineNumbers",
      lineMarker: (view, line) => {
        const number = view.state.doc.lineAt(line.from).number;
        const current = view.state.doc.lineAt(view.state.selection.main.head).number;
        return new LineLabel(String(options.relativenumber && (number !== current || !options.number) ? Math.abs(number - current) : number));
      },
      lineMarkerChange: (update) => update.selectionSet || update.docChanged,
      initialSpacer: (view) => new LineLabel(String(view.state.doc.lines)),
      updateSpacer: (_spacer, update) => new LineLabel(String(update.state.doc.lines)),
    }) : [],
    EditorView.theme({ ".cm-gutters": { backgroundColor: "transparent", color: "var(--ink-faint)", borderRight: "1px solid var(--line)" } }),
  ];
}
