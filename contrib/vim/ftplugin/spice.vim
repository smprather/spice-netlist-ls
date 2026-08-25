" ftplugin/spice.vim — fallback for vim8 (nvim uses after/ftplugin/spice.lua)
" Sets commentstring and optional format-on-save via LSP if available.

if exists("b:did_ftplugin")
  finish
endif
let b:did_ftplugin = 1

setlocal commentstring=*\ %s
setlocal iskeyword+=.,$,/
setlocal formatoptions-=t formatoptions+=croql

" If vim-lsp / vim-lsp-settings present, the server `spice-netlist-ls` will
" attach via lsp#register_server; manual fallback:
" let g:lsp_settings = { 'spice-netlist-ls': { 'cmd': ['spice-netlist-ls'], 'allowlist': ['spice'] } }
