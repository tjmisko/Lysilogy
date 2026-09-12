import { Vim, getCM, vim } from "@replit/codemirror-vim";
import { Compartment, type Extension } from "@codemirror/state";
import { ViewPlugin } from "@codemirror/view";
import { NotesVimrc, type VimrcStatus } from "./notesVimrc";
import { defaultNotesVimOptions, notesVimOptionExtensions } from "./notesVimOptions";

export type NotesVimCommands = {
  onSave: (closeAfter?: boolean, force?: boolean) => void;
  onQuit: (discard?: boolean) => void;
  onFocusReader: () => void;
  onCommandError: (message: string) => void;
  onVimrcStatus?: (status: VimrcStatus) => void;
};

// Ex definitions belong to the engine, but file operations belong to one buffer.
// Never capture a paper's callbacks in a global Vim command definition.
const buffers = new WeakMap<object, NotesVimCommands>();
const configurations = new WeakMap<object, NotesVimrc>();
// The engine omits absent arguments/ranges at runtime, despite declaring some
// of them required in its upstream ExParams type.
type FileCommandParams = { commandName: string; argString?: string; line?: number; lineEnd?: number };

function fileCommand(name: string, prefix: string, action: (commands: NotesVimCommands, bang: boolean) => void): void {
  Vim.defineEx(name, prefix, (cm, params: FileCommandParams) => {
    const commands = buffers.get(cm);
    if (commands === undefined) return;
    const argument = (params.argString ?? "").trim();
    if (argument !== "" && argument !== "!") {
      commands.onCommandError(`:${params.commandName} operates on this paper's note. File arguments are not supported.`);
      return;
    }
    if (params.line !== undefined || params.lineEnd !== undefined) {
      commands.onCommandError(`:${params.commandName} operates on the whole note. A line range is not supported.`);
      return;
    }
    action(commands, argument === "!");
  });
}

fileCommand("write", "w", (commands, bang) => commands.onSave(false, bang));
fileCommand("quit", "q", (commands, bang) => commands.onQuit(bang));
fileCommand("wq", "wq", (commands, bang) => commands.onSave(true, bang));
fileCommand("xit", "x", (commands, bang) => commands.onSave(true, bang));
fileCommand("exit", "exi", (commands, bang) => commands.onSave(true, bang));
fileCommand("reader", "reader", (commands) => commands.onFocusReader());
Vim.defineEx("source", "so", (cm, params: FileCommandParams) => {
  if (params.line !== undefined || params.lineEnd !== undefined) { buffers.get(cm)?.onCommandError(":source does not accept a line range."); return; }
  void configurations.get(cm)?.reload((params.argString ?? "").trim());
});
for (const name of ["set", "setlocal"] as const) {
  Vim.defineEx(name, name === "set" ? "se" : "setl", (cm, params: FileCommandParams) => {
    configurations.get(cm)?.setOptions((params.argString ?? "").trim());
  });
}
// Route interactive map mutations through the same owner as sourced maps, so
// pending prefix timers never outlive :unmap / :mapclear or remove native keys.
for (const prefix of ["", "n", "i", "v", "x", "o"]) {
  for (const suffix of ["map", "noremap", "unmap", "mapclear"]) {
    const name = `${prefix}${suffix}`;
    const short = prefix !== "" && suffix === "map" ? `${prefix}m` : name;
    Vim.defineEx(name, short, (cm, params: FileCommandParams) => {
      configurations.get(cm)?.configure(`${name} ${params.argString ?? ""}`);
    });
  }
}

// Preserve the engine's default keymap length: removing its Ctrl-w no-op breaks
// noremap/mapclear's default/user boundary. An action keeps the prefix pending.
Vim.defineAction("notesWindowPrefix", (_cm, _args, state) => { state.inputState.keyBuffer = ["<C-w>"]; });
function defaultBindings(): void {
  Vim.mapclear();
  Vim.setOption("pcre", true);
  Vim.setOption("insertModeEscKeysTimeout", 200);
  Vim.noremap("U", "<C-r>", "normal");
  Vim.map("ZZ", ":x<CR>", "normal");
  Vim.map("ZQ", ":q!<CR>", "normal");
  Vim.mapCommand("<C-w>", "action", "notesWindowPrefix", {}, { context: "normal" });
  for (const key of ["<C-w>w", "<C-w>W", "<C-w><C-w>", "<C-w>h", "<C-w><Left>"]) Vim.map(key, ":reader<CR>", "normal");
}

export function notesVim(commands: NotesVimCommands): Extension {
  defaultBindings();
  const options = new Compartment();
  return [
    options.of(notesVimOptionExtensions(defaultNotesVimOptions())),
    vim({ status: true }),
    ViewPlugin.define((view) => {
      const cm = getCM(view);
      if (cm === null) return {};
      buffers.set(cm, commands);
      const configuration = new NotesVimrc(view, options, defaultBindings, commands.onCommandError, commands.onVimrcStatus);
      configurations.set(cm, configuration);
      void configuration.reload();
      return { destroy: () => { buffers.delete(cm); configurations.delete(cm); configuration.destroy(); } };
    }),
  ];
}
