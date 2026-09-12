import { Vim, getCM } from "@replit/codemirror-vim";
import { Compartment } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { request } from "./api";
import { compileVimrc, type VimrcDiagnostic, type VimrcOperation } from "./vimrc";
import { defaultNotesVimOptions, notesVimOptionExtensions, setNotesVimOption } from "./notesVimOptions";
import { installVimPrefixes, type NotesVimMapping } from "./notesVimPrefixes";

export type VimrcStatus = { path: string; exists: boolean; loading: boolean; diagnostics: VimrcDiagnostic[] };
type Document = { path: string; text: string; exists: boolean };
type ExParams = { argString?: string; line?: number; lineEnd?: number };
type VimBuffer = Parameters<typeof Vim.handleEx>[0];
const controllers = new WeakMap<object, NotesVimrc>();
const registeredAliases = new Set<string>();

function vimBuffer(view: EditorView): VimBuffer | null {
  const cm = getCM(view);
  return cm?.state.vim == null ? null : cm as VimBuffer;
}

function vimrcDocument(value: unknown): Document {
  if (typeof value !== "object" || value === null || !("path" in value) || typeof value.path !== "string"
    || !("text" in value) || typeof value.text !== "string" || !("exists" in value) || typeof value.exists !== "boolean") {
    throw new Error("The Vimrc response is incompatible. Restart the Lysilogy backend.");
  }
  return { path: value.path, text: value.text, exists: value.exists };
}

/** One notes buffer owns its options, aliases and asynchronous configuration load. */
export class NotesVimrc {
  private abort: AbortController | null = null;
  private alive = true;
  private options = defaultNotesVimOptions();
  private aliases = new Map<string, string>();
  private aliasDepth = 0;
  private prefixes: { destroy(): void } | null = null;
  private configurationText = "";
  private status: VimrcStatus = { path: ".vimrc", exists: false, loading: false, diagnostics: [] };

  constructor(private view: EditorView, private compartment: Compartment, private resetBindings: () => void,
    private onError: (message: string) => void, private onStatus?: (status: VimrcStatus) => void) {
    const cm = getCM(view);
    if (cm !== null) controllers.set(cm, this);
  }

  private publish(status: VimrcStatus): void { this.status = status; if (this.alive) this.onStatus?.(status); }

  async reload(argument = ""): Promise<void> {
    this.abort?.abort();
    const abort = new AbortController();
    this.abort = abort;
    const timer = setTimeout(() => abort.abort(new Error("Loading Vimrc timed out.")), 10000);
    this.publish({ ...this.status, loading: true });
    try {
      const document = vimrcDocument(await request<unknown>("/api/notes/vimrc", { signal: abort.signal, cache: "no-store" }));
      if (!this.alive || this.abort !== abort) return;
      const unquoted = argument.replace(/^(['"])(.*)\1$/u, "$2");
      const basename = document.path.split(/[\\/]/u).at(-1);
      if (unquoted !== "" && unquoted !== document.path && unquoted !== basename) throw new Error(`:source reloads the configured file (${document.path}). Set vim.vimrc in lysilogy.config.json to use another file.`);
      this.configurationText = document.text;
      const diagnostics = this.applyConfiguration();
      this.publish({ path: document.path, exists: document.exists, loading: false, diagnostics });
    } catch (reason) {
      if (!this.alive || this.abort !== abort) return;
      this.publish({ ...this.status, loading: false, diagnostics: [{ line: 0, message: reason instanceof Error ? reason.message : "Could not load Vimrc. Existing mappings remain active." }] });
    } finally { clearTimeout(timer); }
  }

  private applyConfiguration(): VimrcDiagnostic[] {
    const result = compileVimrc(this.configurationText);
    const diagnostics = [...result.diagnostics];
    this.prefixes?.destroy();
    this.prefixes = null;
    this.resetBindings();
    this.options = defaultNotesVimOptions();
    this.aliases.clear();
    const mapped = new Map<string, NotesVimMapping>();
    for (const operation of result.operations) {
      try { this.apply(operation, mapped, diagnostics); }
      catch (reason) { diagnostics.push({ line: operation.line, message: reason instanceof Error ? reason.message : String(reason) }); }
    }
    this.updateOptions();
    const cm = getCM(this.view);
    if (cm !== null) this.prefixes = installVimPrefixes(cm, [...mapped.values()]);
    return diagnostics;
  }

  private apply(operation: VimrcOperation, mapped: Map<string, NotesVimMapping>, diagnostics: VimrcDiagnostic[]): void {
    if (operation.type === "set") {
      for (const option of operation.options) {
        try { setNotesVimOption(this.options, option); }
        catch (reason) { diagnostics.push({ line: operation.line, message: reason instanceof Error ? reason.message : String(reason) }); }
      }
    } else if (operation.type === "command") {
      if (this.aliases.has(operation.name) && !operation.force) throw new Error(`Command ${operation.name} already exists; use command! to replace it.`);
      this.aliases.set(operation.name, operation.command);
      if (!registeredAliases.has(operation.name)) {
        Vim.defineEx(operation.name, operation.name, (cm, params: ExParams) => {
          controllers.get(cm)?.runAlias(operation.name, params.argString ?? "", params.line !== undefined || params.lineEnd !== undefined);
        });
        registeredAliases.add(operation.name);
      }
    } else if (operation.type === "mapclear") {
      for (const mode of operation.modes) {
        Vim.mapclear(mode);
        for (const key of mapped.keys()) if (key.startsWith(`${mode}:`)) mapped.delete(key);
      }
    } else {
      for (const mode of operation.modes) {
        const key = `${mode}:${operation.lhs}`;
        // Never remove engine defaults: its unmap API also splices built-ins,
        // corrupting the index used by nonrecursive mappings and mapclear.
        if (mapped.has(key)) { Vim.unmap(operation.lhs, mode); mapped.delete(key); }
        if (operation.type === "map") {
          if (operation.recursive) Vim.map(operation.lhs, operation.rhs, mode);
          else Vim.noremap(operation.lhs, operation.rhs, mode);
          mapped.set(key, { mode, lhs: operation.lhs, rhs: operation.rhs, recursive: operation.recursive });
        }
      }
    }
  }

  private runAlias(name: string, arguments_: string, hasRange: boolean): void {
    const command = this.aliases.get(name);
    if (command === undefined) { this.onError(`:${name} is not defined in the current Vimrc.`); return; }
    if (arguments_.trim() !== "" || hasRange) { this.onError(`:${name} does not accept arguments or a range.`); return; }
    if (this.aliasDepth >= 16) { this.onError("Vimrc command recursion limit reached."); return; }
    const cm = vimBuffer(this.view);
    if (cm === null) return;
    this.aliasDepth += 1;
    try { Vim.handleEx(cm, command); }
    catch (reason) { this.onError(reason instanceof Error ? reason.message : String(reason)); }
    finally { this.aliasDepth -= 1; }
  }

  private updateOptions(): void {
    const cm = vimBuffer(this.view);
    if (cm === null) return;
    // This adapter reads pcre and Insert escape timing globally. Only one notes
    // editor is mounted; reload/reset updates these without resetting registers.
    for (const name of ["textwidth", "pcre", "insertModeEscKeysTimeout"] as const) Vim.setOption(name, this.options[name], cm);
    this.view.dispatch({ effects: this.compartment.reconfigure(notesVimOptionExtensions(this.options)) });
  }

  setOptions(argument: string): void {
    if (argument === "") { this.onError("Use :set option or :set option=value. Supported: number, relativenumber, wrap, expandtab, tabstop, shiftwidth, textwidth, pcre, insertModeEscKeysTimeout."); return; }
    const failures: string[] = [];
    for (const token of argument.split(/\s+/u)) {
      try { setNotesVimOption(this.options, token); }
      catch (reason) { failures.push(reason instanceof Error ? reason.message : String(reason)); }
    }
    this.updateOptions();
    this.configurationText += `\nset ${argument}`;
    if (failures.length !== 0) this.onError(failures.join("; "));
  }

  configure(command: string): void {
    const next = this.configurationText + "\n" + command;
    if (new TextEncoder().encode(next).length > 64 * 1024) { this.onError("Vimrc session configuration exceeds 64 KiB. Reload with :source."); return; }
    this.configurationText = next;
    this.publish({ ...this.status, diagnostics: this.applyConfiguration() });
  }

  destroy(): void {
    this.alive = false;
    this.abort?.abort();
    this.prefixes?.destroy();
    const cm = getCM(this.view);
    if (cm !== null) controllers.delete(cm);
  }
}
