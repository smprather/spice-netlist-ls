" syntax/spice.vim — regex fallback for SPICE netlists
" Precise net-name coloring comes from the LSP (semanticTokens) — this file
" is offline fallback: directives, params, numbers, strings, comments.
" With nvim+spice-netlist-ls running, nets (variable) and other tokens are
" colored by the server even if tree-sitter is not installed.
" Copy to ~/.vim/syntax/spice.vim or ~/.config/nvim/syntax/spice.vim.

if exists("b:current_syntax")
  finish
endif

" Case-insensitive like HSPICE/Spectre
syn case ignore

" Comments — full line: '*' or '$' or '//' (spectre) at first non-blank
syn match spiceComment "^\s*\*.*$" contains=spiceTodo
syn match spiceComment "^\s*\$.*$" contains=spiceTodo
syn match spiceComment "^\s*//.*$" contains=spiceTodo
" Inline: $ (HSPICE), ; (ngspice/ltspice), // (spectre) when preceded by space/=/,
" Keep permissive — highlight all three; false positives on $& refs are rare
syn match spiceComment "\s\zs\$.*$"
syn match spiceComment "\s\zs;.*$"
syn match spiceComment "\s\zs//.*$"
syn keyword spiceTodo TODO FIXME NOTE XXX contained

" Title line: first line not comment/directive before any other statement (vim can't track state, so just dim first line)
" Users who want title highlighted can enable via g:spice_highlight_title

" Directives: .<keyword> — keyword includes subckt/ends which get extra groups below
syn match spiceDirective "^\s*\.\s*\h\w*\>" contains=spiceKeyword
syn match spiceKeyword "^\s*\.\s*\zs\h\w*\>" contained

" Subckt definition — highlight def name as Type, ports as Identifier (nets via LSP will override)
" .subckt <name> <ports...> [params]
syn match spiceSubckt "^\s*\.subckt\>" nextgroup=spiceSubcktName skipwhite
syn match spiceSubcktName "\h\w*" contained nextgroup=spiceSubcktPort skipwhite
syn match spiceSubcktPort "\S\+" contained nextgroup=spiceSubcktPort,spiceParam,spiceComment skipwhite

" .ends [name]
syn match spiceEnds "^\s*\.ends\>\s*\h\w*"

" Simulator lang switch (scs per-section)
syn match spiceSimLang "^\s*simulator\>" nextgroup=spiceSimLangEq skipwhite
syn match spiceSimLangEq "lang" contained nextgroup=spiceOperator skipwhite
syn match spiceOperator "=" contained

" Instance / device lines: first token ^<letter><word> is element name (Function)
" Following tokens before '=' are nets / model (approximate — LSP gives precise)
syn match spiceInstance "^\s*\zs[A-Za-z]\w*" contains=spiceInstanceName
syn match spiceInstanceName "^[A-Za-z]\w*" contained

" Params: key=value  (and key = value with spaces)
" Highlight key as Identifier, = as Operator, value later as Number/String/Type
syn match spiceParam "\w\+\s*=" contains=spiceParamKey,spiceOperator
syn match spiceParamKey "\w\+" contained
syn match spiceOperator "=" contained

" Parens for spectre nodes: ( a b ) — highlight parens as Operator
syn match spiceOperator "[()]" 

" Numbers: spice values 1k, 1.2u, 10meg, 1e-9, {expr}, plus units
" Must come after param matches so units don't swallow
syn match spiceNumber "\<\d\+\(\.\d*\)\=\([eE][+-]\=\d\+\)\=\([a-zA-Z]*\)\?\>"
syn match spiceNumber "\<0\=\.\d\+\([eE][+-]\=\d\+\)\=\([a-zA-Z]*\)\?\>"
syn match spiceNumber "{\s*[^}]*\s*}"

" Strings: quoted include paths, param values '1.2*1p', "path"
syn region spiceString start=+"+ skip=+\\"+ end=+"+ contains=spiceNumber
syn region spiceString start=+'+ skip=+\\'+ end=+'+ contains=spiceNumber

" Continuation line: leading + (or + with spaces)
syn match spiceContinuation "^\s*+"
syn match spiceContinuation "^\s*+\s"

" Include / lib args already covered by String; add explicit for clarity
" (no extra group needed)

hi def link spiceComment Comment
hi def link spiceTodo Todo
hi def link spiceDirective Keyword
hi def link spiceKeyword Keyword
hi def link spiceSubckt Keyword
hi def link spiceSubcktName Type
hi def link spiceSubcktPort Identifier
hi def link spiceEnds Keyword
hi def link spiceSimLang Keyword
hi def link spiceSimLangEq Identifier
hi def link spiceInstance Function
hi def link spiceInstanceName Function
hi def link spiceParam Identifier
hi def link spiceParamKey Identifier
hi def link spiceOperator Operator
hi def link spiceNumber Number
hi def link spiceString String
hi def link spiceContinuation Special

let b:current_syntax = "spice"
