import { CodeMirror, Vim } from '@replit/codemirror-vim';
import type { VimrcMode } from './vimrc';

export type NotesVimMapping = { mode: VimrcMode; lhs: string; rhs: string; recursive: boolean };
type PrefixMode = 'normal' | 'visual';
type VimState = NonNullable<CodeMirror['state']['vim']>;
type PrefixArgs = { prefix: string; mode: PrefixMode; repeat?: number; repeatIsExplicit?: boolean; registerName?: string | null };
type Pending = { prefix: string; mode: PrefixMode; state: VimState['inputState'] };
const controllers = new WeakMap<object, PrefixController>();
const ACTION = 'lysilogyMappingPrefix';
const TIMEOUT_MS = 1000;
let nextBinding = 0;

function keys(sequence: string): string[] { return sequence.match(/<[^>]+>|./gu) ?? []; }
function identity(mode: string, lhs: string): string { return `${mode}:${lhs}`; }

Vim.defineAction(ACTION, (cm, args, state) => {
  controllers.get(cm)?.begin(args as PrefixArgs, state);
});

// Macro playback and recursive mappings invoke handleKey without a DOM event.
// Keep their prefix fallback identical to physical typing, while leaving the
// core in charge of recording, literal arguments, selection, and actual edits.
const coreHandleKey = Vim.handleKey.bind(Vim);
Vim.handleKey = (cm, key, origin) => {
  if (!key.startsWith('<LysilogyPrefix')) controllers.get(cm)?.beforeKey(key);
  return coreHandleKey(cm, key, origin);
};

/**
 * The core prefers a native full key (Space/j) to a configured partial match.
 * Reserve proper prefixes as user actions without changing its default-keymap
 * boundary. Keeping these in the core also makes recorded lhs keys replayable.
 */
class PrefixController {
  private prefixes: { mode: PrefixMode; prefix: string }[] = [];
  private prefixKeys = new Set<string>();
  private mappings = new Map<string, NotesVimMapping>();
  private pending: Pending | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private alive = true;

  constructor(private cm: CodeMirror, mappings: NotesVimMapping[]) {
    for (const mapping of mappings) {
      if (mapping.mode !== 'normal' && mapping.mode !== 'visual') continue;
      this.mappings.set(identity(mapping.mode, mapping.lhs), mapping);
      const tokens = keys(mapping.lhs);
      if (tokens.length > 128) throw new Error('Vim mappings support at most 128 keys per sequence.');
      for (let length = 1; length < tokens.length; length++) {
        const prefix = tokens.slice(0, length).join('');
        // Counts and Escape are grammar, rather than ordinary command prefixes.
        if (/^\d+$/.test(prefix) || tokens[0] === '<Esc>' || tokens[0] === '<C-[>' || tokens[0] === '<C-c>') continue;
        const key = identity(mapping.mode, prefix);
        if (this.prefixKeys.has(key)) continue;
        if (this.prefixes.length >= 2048) throw new Error('Vim configuration exceeds 2048 distinct mapping prefixes.');
        this.prefixKeys.add(key);
        this.prefixes.push({ mode: mapping.mode, prefix });
      }
    }
    controllers.set(cm, this);
    this.installBindings();
    cm.cm6.dom.addEventListener('keydown', this.keydown, true);
    cm.cm6.dom.addEventListener('pointerdown', this.cancel, true);
    cm.cm6.dom.addEventListener('focusout', this.focusout, true);
  }

  private installBindings(): void {
    if (!this.alive) return;
    for (const { mode, prefix } of this.prefixes) {
      Vim.mapCommand(prefix, 'action', ACTION, { prefix, mode }, { context: mode });
    }
  }

  private removeBindings(): void {
    // Each is our most recently installed user mapping, never an engine default.
    for (const { mode, prefix } of this.prefixes) Vim.unmap(prefix, mode);
  }

  begin(args: PrefixArgs, state: VimState): void {
    this.clearTimer();
    const count = args.repeatIsExplicit ? String(args.repeat ?? 1) : '';
    // Actions reset inputState. Restore the count and selected register so that
    // the completed command behaves like the original multi-key mapping.
    state.inputState.keyBuffer = count === '' ? [args.prefix] : [count, args.prefix];
    if (args.registerName !== undefined && args.registerName !== null) state.inputState.registerName = args.registerName;
    this.pending = { prefix: args.prefix, mode: args.mode, state: state.inputState };
    this.timer = setTimeout(() => { this.timer = null; this.flush(); }, TIMEOUT_MS);
  }

  private clearTimer(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private current(): Pending | null {
    const pending = this.pending;
    const state = this.cm.state.vim;
    if (pending === null || state == null || state.insertMode || state.inputState !== pending.state
      || (state.visualMode ? 'visual' : 'normal') !== pending.mode
      || state.inputState.keyBuffer.at(-1) !== pending.prefix) return null;
    return pending;
  }

  private cancel = (): void => {
    const pending = this.current();
    this.clearTimer();
    this.pending = null;
    if (pending !== null) {
      pending.state.keyBuffer = [];
      pending.state.prefixRepeat = [];
      pending.state.motionRepeat = [];
    }
  };

  private focusout = (event: FocusEvent): void => {
    if (!(event.relatedTarget instanceof Node) || !this.cm.cm6.dom.contains(event.relatedTarget)) this.cancel();
  };

  private keydown = (event: KeyboardEvent): void => {
    const pending = this.current();
    if (pending === null || event.isComposing) return;
    const key = Vim.vimKeyFromEvent(event, this.cm.state.vim ?? undefined);
    if (key === undefined) return;
    if (!(event.target instanceof Node) || !this.cm.cm6.contentDOM.contains(event.target)) { this.cancel(); return; }
    this.beforeKey(key);
  };

  beforeKey(key: string): void {
    const pending = this.current();
    if (pending === null) return;
    if (key === '<Esc>' || key === '<C-[>' || key === '<C-c>') { this.cancel(); return; }
    const candidate = identity(pending.mode, pending.prefix + key);
    if (this.prefixKeys.has(candidate) || this.mappings.has(candidate)) {
      // Let the core consume this real key, including macro recording. A longer
      // prefix action re-arms the timer; a complete mapping finishes normally.
      this.clearTimer();
      this.pending = null;
    } else this.flush();
  }

  private dispatch(mapping: NotesVimMapping): void {
    const binding = `<LysilogyPrefix${++nextBinding}>`;
    if (mapping.recursive) Vim.map(binding, mapping.rhs, mapping.mode);
    else Vim.noremap(binding, mapping.rhs, mapping.mode);
    try {
      // Actual lhs keystrokes were already recorded by the core. This private
      // dispatch must never enter a macro register.
      Vim.handleKey(this.cm, binding, 'mapping');
    } finally { Vim.unmap(binding, mapping.mode); }
  }

  private flush(): void {
    const pending = this.current();
    this.clearTimer();
    this.pending = null;
    if (pending === null || !this.alive) return;
    const count = /^\d*/.exec(pending.state.keyBuffer.join(''))?.[0] ?? '';
    pending.state.keyBuffer = count === '' ? [] : [count];
    const exact = this.mappings.get(identity(pending.mode, pending.prefix));
    if (exact !== undefined) {
      this.dispatch(exact);
      return;
    }
    // Temporarily remove our own overrides so Space alone still moves right,
    // j alone still moves down, and a native partial (e.g. f) can await its key.
    this.removeBindings();
    try { this.dispatch({ mode: pending.mode, lhs: pending.prefix, rhs: pending.prefix, recursive: true }); }
    finally { this.installBindings(); }
  }

  destroy(): void {
    if (!this.alive) return;
    this.cancel();
    this.alive = false;
    this.removeBindings();
    controllers.delete(this.cm);
    this.cm.cm6.dom.removeEventListener('keydown', this.keydown, true);
    this.cm.cm6.dom.removeEventListener('pointerdown', this.cancel, true);
    this.cm.cm6.dom.removeEventListener('focusout', this.focusout, true);
  }
}

export function installVimPrefixes(cm: CodeMirror, finalMappings: NotesVimMapping[]): { destroy(): void } {
  controllers.get(cm)?.destroy();
  return new PrefixController(cm, finalMappings);
}
