" Lysilogy notes. Portable mappings from linux-config's goose/remap.lua.
" Reload this file with :source (or :source .vimrc) without closing the note.
let mapleader = " "
let maplocalleader = " "

nnoremap U <C-r>
nnoremap <C-d> <C-d>zz
nnoremap <C-u> <C-u>zz
nnoremap <PageDown> <C-d>zz
nnoremap <PageUp> <C-u>zz
inoremap <PageDown> <C-o><C-d><C-o>zz
inoremap <PageUp> <C-o><C-u><C-o>zz
nnoremap n nzz
nnoremap N Nzz
nnoremap * *zz
nnoremap ; ,
nnoremap , ;
nnoremap <Home> _
xnoremap <leader>p "_dP

set number relativenumber
set tabstop=4 shiftwidth=4 expandtab
set nowrap

" Basic scripting and aliases can share one file with other Vim environments.
if has('lysilogy')
  command! W write
  command! Wq wq
endif

" Neovim plugins, Lua functions, filesystem/window commands, and arbitrary
" Vimscript functions are not executed by this CodeMirror notes buffer.
