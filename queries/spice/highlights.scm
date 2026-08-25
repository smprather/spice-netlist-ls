; queries/spice/highlights.scm — tree-sitter highlighting for SPICE
; ------------------------------------------------------------
; Two modes:
; 1) LSP semanticTokens (primary): spice-netlist-ls colors nets precisely
;    per element arity and dialect. No tree-sitter install required — nvim +
;    helix get full highlighting via the server.
; 2) Offline tree-sitter fallback: if you install a `tree-sitter-spice`
;    parser (e.g. https://github.com/tree-sitter/tree-sitter-spice) this
;    file provides the highlight queries. Net accuracy then depends on that
;    grammar — keep it in sync with src/parser.rs element_node_count.
;
; Install for nvim-treesitter:
;   mkdir -p ~/.config/nvim/queries/spice
;   cp queries/spice/highlights.scm ~/.config/nvim/queries/spice/highlights.scm
; Helix: copy to runtime/queries/spice/highlights.scm
; See contrib/helix/languages.toml for language config.

; Comments: * $ ; // and inline variants
(comment) @comment
(line_comment) @comment

; Directives and subckt keywords
(directive_name) @keyword
((directive) @keyword
  (#match? @keyword "^\.(subckt|ends|model|param|include|inc|lib|probe|measure|meas|alter|protect|control|endc|step|backanno|temp|options|global|save|plot|print|ac|dc|tran|op)$"))
(subckt_keyword) @keyword  ; .subckt / .ends
(simulator_lang) @keyword

; Subckt definition name → Type, ports → variable (nets)
(subckt_definition name: (identifier) @type)
(subckt_definition port: (identifier) @variable)
(subckt_definition param_key: (identifier) @property)

; Instance name → function, nodes → variable
(instance name: (identifier) @function)
(instance node: (identifier) @variable)
(instance port: (identifier) @variable)

; Model / value
(model_name) @type
(value) @number
(number) @number
(string) @string

; Params
(param_key) @property
(param_value) @number
(param_value) @string
(param_value) @type

; Operators
["=" "(" ")"] @operator
(continuation) @operator

; Inline strings (include paths, expressions {…})
(quoted_string) @string
(braced_expression) @string

; ------------------------------------------------------------------
; Fallback for grammars that expose flat tokens (no instance/node split)
; These generic captures ensure something is colored even if the grammar
; is shallow — the LSP will still provide precise net tokens on top.
; ------------------------------------------------------------------
(identifier) @variable  ; fallback
(directive) @keyword
