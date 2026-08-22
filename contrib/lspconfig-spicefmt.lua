-- Draft entry for nvim-lspconfig: `lsp/spicefmt.lua` upstream.
-- Users with lspconfig do `require('lspconfig').spicefmt.setup()`;
-- native-0.11 users can copy after/ftplugin/spice.lua instead (preferred —
-- one less plugin dependency, works offline with just a git clone).

---@type vim.lsp.Config
return {
  cmd = { "spice-netlist-ls" },
  filetypes = { "spice", "cir", "scs", "subckt" },
  root_markers = { ".git", "spicefmt.toml" },
  docs = {
    description = [[
https://github.com/smprather/spice-netlist-ls

Formatter + linter + LSP for SPICE netlists (HSPICE golden, dialects for
ngspice/LTspice/spectre). Ships as a single statically-linked binary —
`cargo install spice-netlist-ls`, or grab the tarball from Releases for an
offline install.
]],
  },
}