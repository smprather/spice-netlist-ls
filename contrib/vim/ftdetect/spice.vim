" ftdetect/spice.vim — filetype detection for SPICE netlists
" Drop-in: copy to ~/.vim/ftdetect/spice.vim or ~/.config/nvim/ftdetect/spice.vim
" Already covered if you use after/ftplugin/spice.lua (nvim ≥0.11 sets filetype via LSP filetypes).
" This is for vim8 / offline users who want pure regex highlighting without LSP.

augroup spice_detect
  autocmd!
  autocmd BufNewFile,BufRead *.sp,*.cir,*.ckt,*.net,*.spice set filetype=spice
  autocmd BufNewFile,BufRead *.scs,*.subckt,*.sub set filetype=spice
  " Spectre netlists often use .scs but may be detected as superset;
  " the same syntax file handles both — dialect differences (.param // etc) are highlighted permissively.
augroup END
