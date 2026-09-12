import { Vim, getCM, vim } from "@replit/codemirror-vim";
import type { Extension } from "@codemirror/state";
import { ViewPlugin } from "@codemirror/view";

export type NotesVimCommands = {
  onSave: (closeAfter?: boolean, force?: boolean) => void;
  onQuit: (discard?: boolean) => void;
  onFocusReader: () => void;
  onCommandError: (message: string) => void;
};

// Ex definitions belong to the engine, but file operations belong to one buffer.
// Never capture a paper's callbacks in a global Vim command definition.
const buffers = new WeakMap<object, NotesVimCommands>();
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
Vim.map("ZZ", ":x<CR>", "normal");
Vim.map("ZQ", ":q!<CR>", "normal");
// The engine's single-key Normal Ctrl-w no-op otherwise wins over every
// multi-key window mapping. Keep its separate Insert-mode word deletion.
Vim.unmap("<C-w>", "normal");
for (const key of ["<C-w>w", "<C-w>W", "<C-w><C-w>", "<C-w>h", "<C-w><Left>"]) {
  Vim.map(key, ":reader<CR>", "normal");
}

export function notesVim(commands: NotesVimCommands): Extension {
  return [
    vim({ status: true }),
    ViewPlugin.define((view) => {
      const cm = getCM(view);
      if (cm !== null) buffers.set(cm, commands);
      return { destroy: () => { if (cm !== null) buffers.delete(cm); } };
    }),
  ];
}
