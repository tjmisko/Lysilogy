/** A deliberately bounded Vimrc compiler. It produces configuration, never code to evaluate. */
export type VimrcMode = 'normal' | 'insert' | 'visual' | 'operatorPending';
export type VimrcOperation =
  | { type: 'map'; modes: VimrcMode[]; lhs: string; rhs: string; recursive: boolean; line: number }
  | { type: 'unmap'; modes: VimrcMode[]; lhs: string; line: number }
  | { type: 'mapclear'; modes: VimrcMode[]; line: number }
  | { type: 'set'; options: string[]; local: boolean; line: number }
  | { type: 'command'; name: string; command: string; force: boolean; line: number };
export type VimrcDiagnostic = { line: number; message: string };
export type CompiledVimrc = { operations: VimrcOperation[]; diagnostics: VimrcDiagnostic[] };

type Scalar = string | number | boolean;
type Token = { type: 'string' | 'number' | 'name' | 'operator'; value: string };
type Conditional = { parent: boolean; matched: boolean; active: boolean; hadElse: boolean; line: number };
const MAX_BYTES = 64 * 1024;
const MAX_OPERATIONS = 1024;
const MAX_DEPTH = 16;
const SPECIAL_KEYS: Record<string, string> = {
  esc: 'Esc', escape: 'Esc', cr: 'CR', enter: 'CR', return: 'CR',
  space: 'Space', tab: 'Tab', bs: 'BS', backspace: 'BS', del: 'Del', delete: 'Del',
  ins: 'Ins', insert: 'Ins', left: 'Left', right: 'Right', up: 'Up', down: 'Down',
  pageup: 'PageUp', pagedown: 'PageDown', home: 'Home', end: 'End', nop: 'Nop',
  lt: 'lt', bar: 'Bar', nul: 'Nul',
};

function canonicalKeys(keys: string, replacement = false): string {
  return keys.replace(/<([^<>]+)>| /g, (whole: string, notation: string | undefined) => {
    if (notation === undefined) return '<Space>';
    const parts = notation.split('-');
    const modifiers: string[] = [];
    while (parts.length > 1 && /^[csamd]$/i.test(parts[0] ?? '')) modifiers.push((parts.shift() ?? '').toUpperCase());
    const key = parts.join('-');
    const known = SPECIAL_KEYS[key.toLowerCase()] ?? (/^f\d{1,2}$/i.test(key) ? key.toUpperCase() : undefined);
    if (modifiers.length === 0) {
      if (known === 'Nop' && replacement) return '';
      if (known === 'Bar') return '|';
      if (known === 'lt') return '<';
      return known === undefined ? whole : `<${known}>`;
    }
    if (modifiers.length === 1 && modifiers[0] === 'S' && key.length === 1) return key.toUpperCase();
    const modifierOrder = ['C', 'A', 'M', 'D', 'S'];
    const prefix = modifierOrder.filter(modifier => modifiers.includes(modifier)).join('-');
    const letter = key.length === 1 ? (modifiers.includes('S') ? key.toUpperCase() : key.toLowerCase()) : key;
    return `<${prefix}-${known ?? letter}>`;
  });
}

function truthy(value: Scalar): boolean {
  // Vim converts strings to a number when it needs a Boolean.
  return typeof value === 'string' ? (Number.parseInt(value, 10) || 0) !== 0 : Boolean(value);
}

function tokenize(expression: string): Token[] {
  const tokens: Token[] = [];
  let position = 0;
  while (position < expression.length) {
    const char = expression[position];
    if (char !== undefined && /\s/.test(char)) { position++; continue; }
    if (char === '"' || char === "'") {
      const quote = char;
      position++;
      let value = '';
      let closed = false;
      while (position < expression.length) {
        const next = expression[position++];
        if (next === quote) {
          if (quote === "'" && expression[position] === "'") { value += "'"; position++; }
          else { closed = true; break; }
        } else if (next === '\\' && quote === '"') {
          const escaped = expression[position++];
          if (escaped === undefined) break;
          value += escaped === 'n' ? '\n' : escaped === 't' ? '\t' : escaped === 'r' ? '\r' : escaped;
        } else value += next ?? '';
      }
      if (!closed) throw new Error('Unterminated string.');
      tokens.push({ type: 'string', value });
      continue;
    }
    const rest = expression.slice(position);
    const number = /^-?\d+/.exec(rest)?.[0];
    if (number !== undefined) {
      tokens.push({ type: 'number', value: number }); position += number.length; continue;
    }
    const name = /^(?:(?:g|s|v):)?[A-Za-z_][A-Za-z0-9_]*/.exec(rest)?.[0];
    if (name !== undefined) {
      tokens.push({ type: 'name', value: name }); position += name.length; continue;
    }
    const operator = /^(?:==[?#]?|!=[?#]?|>=|<=|&&|\|\||[!><().,])/.exec(rest)?.[0];
    if (operator !== undefined) {
      tokens.push({ type: 'operator', value: operator }); position += operator.length; continue;
    }
    throw new Error(`Unsupported expression near “${rest.slice(0, 24)}”.`);
  }
  return tokens;
}

function evaluate(expression: string, variables: Map<string, Scalar>): Scalar {
  const tokens = tokenize(expression);
  let position = 0;
  let depth = 0;
  const at = (value: string) => tokens[position]?.value === value && tokens[position]?.type === 'operator';
  const consume = (value: string) => { if (!at(value)) throw new Error(`Expected “${value}”.`); position++; };
  function primary(enabled: boolean): Scalar {
    if (++depth > MAX_DEPTH) throw new Error(`Expression nesting exceeds ${MAX_DEPTH}.`);
    try {
      if (at('!')) { position++; return !truthy(primary(enabled)); }
      if (at('(')) { position++; const value = logicalOr(enabled); consume(')'); return value; }
      const token = tokens[position++];
      if (token === undefined) throw new Error('Expected a value.');
      if (token.type === 'number') return Number(token.value);
      if (token.type === 'string') return token.value;
      if (token.type !== 'name') throw new Error(`Unexpected “${token.value}”.`);
      if (at('(')) {
        if (token.value !== 'exists' && token.value !== 'has') throw new Error(`Unsupported function “${token.value}”.`);
        position++;
        const argument = logicalOr(enabled);
        consume(')');
        if (typeof argument !== 'string') throw new Error(`${token.value}() requires a string.`);
        if (token.value === 'has') return argument === 'lysilogy';
        return variables.has(argument);
      }
      if (token.value === 'v:true') return true;
      if (token.value === 'v:false') return false;
      const value = variables.get(token.value);
      if (value === undefined && enabled) throw new Error(`Undefined variable “${token.value}”.`);
      return value ?? false;
    } finally { depth--; }
  }
  function concat(enabled: boolean): Scalar {
    let value = primary(enabled);
    while (at('.')) {
      position++;
      const right = String(primary(enabled));
      const left = String(value);
      if (left.length + right.length > MAX_BYTES) throw new Error('Expanded Vimrc value exceeds 64 KiB.');
      value = left + right;
    }
    return value;
  }
  function compare(enabled: boolean): Scalar {
    let left = concat(enabled);
    while (tokens[position]?.type === 'operator' && /^(?:==[?#]?|!=[?#]?|>|<|>=|<=)$/.test(tokens[position]?.value ?? '')) {
      const operator = tokens[position++]?.value ?? '';
      const right = concat(enabled);
      if (operator.startsWith('==') || operator.startsWith('!=')) {
        const a = operator.endsWith('?') && typeof left === 'string' ? left.toLowerCase() : left;
        const b = operator.endsWith('?') && typeof right === 'string' ? right.toLowerCase() : right;
        const equal = typeof a === typeof b ? a === b : Number(a) === Number(b);
        left = operator.startsWith('!=') ? !equal : equal;
      } else {
        const a = Number(left); const b = Number(right);
        left = operator === '>' ? a > b : operator === '<' ? a < b : operator === '>=' ? a >= b : a <= b;
      }
    }
    return left;
  }
  function logicalAnd(enabled: boolean): Scalar {
    let value = compare(enabled);
    while (at('&&')) { position++; const right = compare(enabled && truthy(value)); value = truthy(value) && truthy(right); }
    return value;
  }
  function logicalOr(enabled: boolean): Scalar {
    let value = logicalAnd(enabled);
    while (at('||')) { position++; const right = logicalAnd(enabled && !truthy(value)); value = truthy(value) || truthy(right); }
    return value;
  }
  const value = logicalOr(true);
  if (position !== tokens.length) throw new Error('Unexpected text after the expression.');
  return value;
}

/** Double quotes start comments outside strings in expressions only after a complete operand. */
function withoutComment(text: string): string {
  let quote = '';
  let canComment = false;
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (quote !== '') {
      if (quote === '"' && char === '\\') { i++; continue; }
      if (char === quote) {
        if (quote === "'" && text[i + 1] === "'") i++;
        else { quote = ''; canComment = true; }
      }
    } else if (char === '"') {
      if (canComment && (i === 0 || /\s/.test(text[i - 1] ?? ''))) return text.slice(0, i).trimEnd();
      quote = '"';
    } else if (char === "'") quote = "'";
    else if (char !== undefined && !/\s/.test(char)) canComment = /[\w)]/.test(char);
  }
  return text.trimEnd();
}

function modesFor(prefix: string, bang: boolean): VimrcMode[] {
  if (prefix === 'n') return ['normal'];
  if (prefix === 'i') return ['insert'];
  if (prefix === 'v' || prefix === 'x') return ['visual'];
  if (prefix === 'o') return ['operatorPending'];
  return bang ? ['insert'] : ['normal', 'visual', 'operatorPending'];
}

function blockEnd(command: string): string | null {
  const keyword = /^(\w+)!?(?:\s|$)/.exec(command)?.[1] ?? '';
  if (keyword.length >= 2 && 'function'.startsWith(keyword)) return 'endfunction';
  if (/^def!?\b/.test(command)) return 'enddef';
  if (/^for\b/.test(command)) return 'endfor';
  if (keyword.length >= 2 && 'while'.startsWith(keyword)) return 'endwhile';
  if (/^try(?:\s|$)/.test(command)) return 'endtry';
  if (/^aug(?:r(?:o(?:u(?:p)?)?)?)?\s+(?!END(?:\s|$))/i.test(command)) return 'augroup END';
  const heredoc = /^(?:lua|python3?|py3)\s+<<\s*(?:trim\s+)?(\S+)/.exec(command);
  return heredoc?.[1] ?? null;
}

export function compileVimrc(text: string): CompiledVimrc {
  const operations: VimrcOperation[] = [];
  const diagnostics: VimrcDiagnostic[] = [];
  const diagnose = (line: number, message: string) => { diagnostics.push({ line, message }); };
  if (new TextEncoder().encode(text).byteLength > MAX_BYTES) {
    diagnose(1, 'Vimrc exceeds the 64 KiB limit.');
    return { operations, diagnostics };
  }
  const variables = new Map<string, Scalar>();
  const conditions: Conditional[] = [];
  const skipped: { end: string; line: number; opaque: boolean }[] = [];
  const lines: { text: string; line: number }[] = [];
  for (const [offset, physical] of text.replace(/^\uFEFF/, '').split(/\r?\n/).entries()) {
    const continuation = /^\s*\\(.*)$/.exec(physical);
    const previous = lines.at(-1);
    if (continuation !== null && previous !== undefined) previous.text += ` ${continuation[1] ?? ''}`;
    else lines.push({ text: physical, line: offset + 1 });
  }
  const active = () => conditions.at(-1)?.active ?? true;
  const expand = (keys: string) => {
    let length = keys.length;
    return keys.replace(/<(localleader|leader)>/gi, (whole: string, kind: string) => {
      const name = kind.toLowerCase() === 'localleader' ? 'maplocalleader' : 'mapleader';
      const value = String(variables.get(name) ?? '\\');
      length += value.length - whole.length;
      if (length > MAX_BYTES) throw new Error('Expanded mapping exceeds 64 KiB.');
      return value;
    });
  };
  const emit = (operation: VimrcOperation) => {
    if (operations.length >= MAX_OPERATIONS) throw new Error(`Vimrc exceeds the ${MAX_OPERATIONS} operation limit.`);
    operations.push(operation);
  };

  function compileCommand(raw: string, line: number, executionDepth = 0): void {
    if (executionDepth > MAX_DEPTH) throw new Error(`Execute nesting exceeds ${MAX_DEPTH}.`);
    const statement = raw.trim().replace(/^:\s*/, '');
    if (statement === '' || statement.startsWith('"')) return;
    const match = /^(\S+)(?:\s+(.*))?$/.exec(statement);
    const command = match?.[1] ?? '';
    let rest = match?.[2] ?? '';
    if (command === 'let') {
      const assignment = /^((?:(?:g|s):)?[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/.exec(rest);
      if (assignment === null) throw new Error('Only scalar let assignments are supported.');
      const name = assignment[1] ?? '';
      const value = evaluate(withoutComment(assignment[2] ?? ''), variables);
      variables.set(name, value);
      if (/^(?:g:)?map(?:local)?leader$/.test(name)) {
        variables.set(name.replace(/^g:/, ''), value);
        variables.set(`g:${name.replace(/^g:/, '')}`, value);
      }
      return;
    }
    if (command === 'execute' || command === 'exe') {
      const generated = evaluate(withoutComment(rest), variables);
      if (typeof generated !== 'string') throw new Error('Execute requires a string expression.');
      if (/[\r\n]/.test(generated)) throw new Error('Execute accepts one configuration command at a time.');
      compileCommand(generated, line, executionDepth + 1);
      return;
    }
    if (command === 'set' || command === 'setlocal' || command === 'setl') {
      rest = rest.replace(/\s+".*$/, '').trim();
      if (rest === '' || rest.includes('|')) throw new Error('Set requires option names or name=value assignments.');
      emit({ type: 'set', options: rest.split(/\s+/), local: command !== 'set', line });
      return;
    }
    if (command === 'command' || command === 'command!' || command === 'com' || command === 'com!') {
      const alias = /^([A-Z][A-Za-z0-9]*)\s+(.+)$/.exec(rest);
      if (alias === null) throw new Error('Command aliases require an uppercase name and an Ex command; command arguments and flags are unsupported.');
      emit({ type: 'command', name: alias[1] ?? '', command: alias[2] ?? '', force: command.endsWith('!'), line });
      return;
    }
    const mapping = /^([nivxo]?)(noremap|map|unmap|mapclear)(!)?$/.exec(command);
    if (mapping !== null) {
      const modes = modesFor(mapping[1] ?? '', mapping[3] === '!');
      if (mapping[3] === '!' && mapping[1] !== '') throw new Error('The ! form is supported only for map, noremap, unmap, and mapclear.');
      while (/^<(silent|buffer|expr|nowait|unique|script)>\s*/i.test(rest)) {
        const flag = /^<([^>]+)>\s*/.exec(rest);
        const value = flag?.[1]?.toLowerCase() ?? '';
        if (value !== 'silent' && value !== 'buffer') throw new Error(`Unsupported mapping flag <${value}>.`);
        rest = rest.slice(flag?.[0].length ?? 0);
      }
      const action = mapping[2];
      if (action === 'mapclear') {
        if (rest !== '' && !rest.startsWith('"')) throw new Error('Mapclear takes no arguments.');
        emit({ type: 'mapclear', modes, line }); return;
      }
      if (action === 'unmap') {
        const lhs = /^(\S+)(?:\s+".*)?$/.exec(rest)?.[1];
        if (lhs === undefined) throw new Error('Unmap requires exactly one key sequence.');
        emit({ type: 'unmap', modes, lhs: canonicalKeys(expand(lhs)), line }); return;
      }
      const keys = /^(\S+)\s+(.+)$/.exec(rest);
      if (keys === null) throw new Error('Map requires both a key sequence and a replacement.');
      emit({ type: 'map', modes, lhs: canonicalKeys(expand(keys[1] ?? '')), rhs: canonicalKeys(expand(keys[2] ?? ''), true), recursive: action === 'map', line });
      return;
    }
    throw new Error(`Unsupported Vimrc command “${command}”.`);
  }

  for (const entry of lines) {
    const statement = entry.text.trim().replace(/^:\s*/, '');
    const skip = skipped.at(-1);
    if (skip !== undefined) {
      const normalized = statement.replace(/\s+".*$/, '').trim();
      const abbreviatedEnd = !skip.opaque && /^end\w+$/.test(normalized) && normalized.length >= 4 && skip.end.startsWith(normalized);
      const groupEnd = skip.end === 'augroup END' && /^aug(?:r(?:o(?:u(?:p)?)?)?)?\s+END$/i.test(normalized);
      if (normalized === skip.end || abbreviatedEnd || groupEnd) skipped.pop();
      else if (!skip.opaque) {
        const nested = blockEnd(statement);
        if (nested !== null) skipped.push({ end: nested, line: entry.line, opaque: /^\w+\s+<</.test(statement) });
      }
      continue;
    }
    const end = blockEnd(statement);
    if (end !== null) {
      if (active()) diagnose(entry.line, `Unsupported block; skipped through “${end}”.`);
      skipped.push({ end, line: entry.line, opaque: /^\w+\s+<</.test(statement) });
      continue;
    }
    const conditional = /^(if|elseif|else|endif)(?:\s+(.*))?$/.exec(statement);
    if (conditional !== null) {
      const kind = conditional[1];
      const expression = withoutComment(conditional[2] ?? '');
      try {
        if (kind === 'if') {
          if (conditions.length >= MAX_DEPTH) {
            diagnose(entry.line, `Conditional nesting exceeds ${MAX_DEPTH}; remaining configuration was skipped.`);
            break;
          }
          const parent = active();
          const frame: Conditional = { parent, matched: false, active: false, hadElse: false, line: entry.line };
          conditions.push(frame);
          frame.active = parent && truthy(evaluate(expression, variables));
          frame.matched = frame.active;
        } else {
          const frame = conditions.at(-1);
          if (frame === undefined) throw new Error(`Unexpected ${kind ?? 'conditional'}.`);
          if (kind === 'endif') { conditions.pop(); continue; }
          frame.active = false;
          if (frame.hadElse) throw new Error('Elseif or else cannot follow else.');
          if (kind === 'else') {
            frame.hadElse = true;
            frame.active = frame.parent && !frame.matched;
          } else frame.active = frame.parent && !frame.matched && truthy(evaluate(expression, variables));
          frame.matched ||= frame.active;
        }
      } catch (error) { diagnose(entry.line, error instanceof Error ? error.message : 'Invalid conditional.'); }
      continue;
    }
    if (!active()) continue;
    try { compileCommand(statement, entry.line); }
    catch (error) {
      diagnose(entry.line, error instanceof Error ? error.message : 'Invalid Vimrc command.');
      if (operations.length >= MAX_OPERATIONS) break;
    }
  }
  for (const frame of conditions) diagnose(frame.line, 'If block is missing endif.');
  for (const frame of skipped) diagnose(frame.line, `Skipped block is missing “${frame.end}”.`);
  return { operations, diagnostics };
}
