import assert from 'node:assert/strict';
import test from 'node:test';
import { compileVimrc } from '../src/lib/vimrc.ts';

function clean(text) {
  const result = compileVimrc(text);
  assert.deepEqual(result.diagnostics, []);
  return result.operations;
}

test('normal redo and separate mode remaps compile without executing their right-hand sides', () => {
  const operations = clean('nnoremap U <C-r>\ninoremap <C-s> <Esc>:w<CR>\nvnoremap J <Nop>\nxmap K k\nonoremap p ip');
  assert.deepEqual(operations[0], { type:'map', modes:['normal'], lhs:'U', rhs:'<C-r>', recursive:false, line:1 });
  assert.deepEqual(operations.map(op => op.modes), [['normal'],['insert'],['visual'],['visual'],['operatorPending']]);
  assert.equal(operations[1].rhs, '<Esc>:w<CR>');
  assert.equal(operations[3].recursive, true);
});

test('map, noremap, unmap, and mapclear retain ordering and per-mode semantics', () => {
  const operations = clean('map a b\nnoremap b c\nnunmap a\nxmapclear\nmap! j k\nunmap! j\nmapclear!');
  assert.deepEqual(operations.map(op => op.type), ['map','map','unmap','mapclear','map','unmap','mapclear']);
  assert.deepEqual(operations[0].modes, ['normal','visual','operatorPending']);
  assert.deepEqual(operations[2].modes, ['normal']);
  assert.deepEqual(operations[3].modes, ['visual']);
  assert.deepEqual(operations[4].modes, ['insert']);
  assert.equal(operations[1].recursive, false);
});

test('leaders expand at definition time, with aliases and independent local leaders', () => {
  const operations = clean(String.raw`nnoremap <leader>x x
let g:mapleader = " "
let maplocalleader = ','
nnoremap <silent> <buffer> <Leader>y <localleader>p
let mapleader = ';'
nnoremap <leader>z <leader>y
let g:mapleader = ':'
nnoremap <leader>q q`);
  assert.equal(operations[0].lhs, '\\x');
  assert.equal(operations[1].lhs, '<Space>y');
  assert.equal(operations[1].rhs, ',p');
  assert.equal(operations[2].lhs, ';z');
  assert.equal(operations[2].rhs, ';y');
  assert.equal(operations[3].lhs, ':q');
});

test('scalar variables, safe expressions, and execute support computed mappings', () => {
  const operations = clean(String.raw`let g:redo = 'U'
let s:enabled = v:true
if has('lysilogy') && exists('g:redo') && s:enabled
  execute 'nnoremap ' . g:redo . " \<C-r>"
elseif has('nvim')
  lua evil()
else
  nnoremap U u
endif
let s:name = 'it''s'
if s:name ==# "it's" && 3 >= 2 && !(0 || v:false)
  nnoremap Z z
endif`);
  assert.deepEqual(operations.map(op => [op.lhs,op.rhs]), [['U','<C-r>'],['Z','z']]);
});

test('conditions short circuit variable reads and skip inactive unsupported commands', () => {
  const operations = clean(String.raw`if exists('g:missing') && g:missing
  call system('never')
elseif !exists('g:missing') || g:missing
  nnoremap U <C-r>
else
  let = broken
endif
if 0
  if missing
    source private.vim
  endif
endif`);
  assert.equal(operations.length, 1);
  assert.equal(operations[0].lhs, 'U');
});

test('literal mapping register quotes survive while expression and full-line comments are ignored', () => {
  const operations = clean(String.raw`" a comment
let g:redo = "U" " inline comment
if g:redo ==# 'U' " another comment
  nnoremap <leader>y "+y
  nnoremap y "ay
endif
nnoremap q q " this is a literal mapping suffix, as in Vim`);
  assert.equal(operations[0].rhs, '"+y');
  assert.equal(operations[1].rhs, '"ay');
  assert.equal(operations[2].rhs, 'q<Space>"<Space>this<Space>is<Space>a<Space>literal<Space>mapping<Space>suffix,<Space>as<Space>in<Space>Vim');
});

test('continuations retain the originating line and UTF-8 key sequences', () => {
  const operations = clean(String.raw`let g:key = 'λ'
execute 'nnoremap ' . g:key
  \ . ' <C-r>'
:nnoremap é e`);
  assert.equal(operations[0].line, 2);
  assert.equal(operations[0].lhs, 'λ');
  assert.equal(operations[0].rhs, '<C-r>');
  assert.equal(operations[1].line, 4);
});

test('set operations and command aliases compile as inert configuration for the adapter', () => {
  const operations = clean('set number tabstop=4 expandtab " comment\nsetlocal nowrap\ncommand! Save w\ncommand Exit wq');
  assert.deepEqual(operations[0], {type:'set',options:['number','tabstop=4','expandtab'],local:false,line:1});
  assert.deepEqual(operations[1], {type:'set',options:['nowrap'],local:true,line:2});
  assert.deepEqual(operations[2], {type:'command',name:'Save',command:'w',force:true,line:3});
  assert.deepEqual(operations[3], {type:'command',name:'Exit',command:'wq',force:false,line:4});
});

test('unsupported flags and executable commands produce diagnostics without leaking mappings', () => {
  const result = compileVimrc('nnoremap <expr> j evil()\nnmap <nowait> k k\nnmap <unique> a a\nsource ~/.vimrc\n!echo evil\nlua do_evil()\ncommand -nargs=* Bad call evil()\nnoremap U <C-r>');
  assert.equal(result.diagnostics.length, 7);
  assert.deepEqual(result.operations.map(op => op.lhs), ['U']);
  assert.match(result.diagnostics[0].message, /expr/);
});

test('unsupported blocks including inactive nested blocks never apply enclosed maps', () => {
  const result = compileVimrc(String.raw`function! Configure()
  if 1
    nnoremap X x
  endif
  for key in ['x']
    nnoremap A a
  endfor
endfunction
lua << EOF
nnoremap B b
EOF
augroup Local
nnoremap C c
augroup END
if 0
  function! Inactive()
    endif
    nnoremap D d
  endfunction
endif
nnoremap U <C-r>`);
  assert.equal(result.diagnostics.length, 3);
  assert.deepEqual(result.operations.map(op => op.lhs), ['U']);
});

test('unsupported expressions fail closed and later independent mappings still compile', () => {
  const result = compileVimrc("let g:x = system('echo nope')\nif g:x\nnnoremap X x\nendif\nexecute 'nnoremap U <C-r>'\nexecute \"nnoremap A a\\nnnoremap B b\"");
  assert.equal(result.diagnostics.length, 3);
  assert.deepEqual(result.operations.map(op => op.lhs), ['U']);
});

test('bounded input, operation counts, expression depth, and conditional depth cannot run away', () => {
  assert.match(compileVimrc('é'.repeat(32769)).diagnostics[0].message, /64 KiB/);
  const repeated = compileVimrc('nnoremap U <C-r>\n'.repeat(1030));
  assert.equal(repeated.operations.length, 1024);
  assert.match(repeated.diagnostics[0].message, /operation limit/);
  const expression = compileVimrc(`let g:x = ${'('.repeat(20)}1${')'.repeat(20)}`);
  assert.match(expression.diagnostics[0].message, /nesting/);
  const nested = compileVimrc('if 1\n'.repeat(20) + 'nnoremap U <C-r>\n' + 'endif\n'.repeat(20));
  assert.equal(nested.operations.length, 0);
  assert.match(nested.diagnostics[0].message, /nesting/);
});

test('missing block terminators and misplaced else are diagnosed with original line numbers', () => {
  const result = compileVimrc('else\nif 1\nnnoremap U <C-r>\nfunction! Incomplete()\nnnoremap X x');
  assert.deepEqual(result.diagnostics.map(item => item.line), [1,4,2,4]);
  assert.deepEqual(result.operations.map(op => op.lhs), ['U']);
});

test('special keys are canonicalized for the case-sensitive engine and Nop disables its mapping', () => {
  const operations = clean('nnoremap U <C-R>\ninoremap jj <ESC>\nnmap <space>x <pagedown><HOME><s-tab><cr>\nnoremap <c-r> <nop>\niunmap <esc>');
  assert.equal(operations[0].rhs, '<C-r>');
  assert.equal(operations[1].rhs, '<Esc>');
  assert.equal(operations[2].lhs, '<Space>x');
  assert.equal(operations[2].rhs, '<PageDown><Home><S-Tab><CR>');
  assert.equal(operations[3].lhs, '<C-r>');
  assert.equal(operations[3].rhs, '');
  assert.equal(operations[4].lhs, '<Esc>');
});

test('computed string expansion is bounded even when input is small', () => {
  const doubling = compileVimrc("let g:x = 'ab'\n" + 'let g:x = g:x . g:x\n'.repeat(30));
  assert.match(doubling.diagnostics[0].message, /Expanded Vimrc value/);
  const leader = compileVimrc(`let mapleader = '${'x'.repeat(10000)}'\nnmap ${'<leader>'.repeat(7)} y`);
  assert.match(leader.diagnostics[0].message, /Expanded mapping/);
  assert.equal(leader.operations.length, 0);
});

test('heredoc content is opaque even when it resembles unsupported Vim block commands', () => {
  const result = compileVimrc('lua << EOF\nfunction fake()\nnnoremap A a\nEOF\nnnoremap U <C-r>');
  assert.equal(result.diagnostics.length, 1);
  assert.deepEqual(result.operations.map(op => op.lhs), ['U']);
});

test('common abbreviated unsupported block commands are skipped, too', () => {
  const result = compileVimrc('fun! Configure()\nnnoremap A a\nendf\nwh 1\nnnoremap B b\nendw\naug Local\nnnoremap C c\naug END\nnnoremap U <C-r>');
  assert.equal(result.diagnostics.length, 3);
  assert.deepEqual(result.operations.map(op => op.lhs), ['U']);
});
