-- after/ftplugin/spice.lua — drop-in for Neovim ≥0.11 without any plugin manager.
-- Copy or symlink this file into `~/.config/nvim/after/ftplugin/spice.lua`,
-- or `rsync -a after/ftplugin/ ~/.config/nvim/after/ftplugin/` from the repo.

-- Register the language server. `cmd` assumes `spice-netlist-ls` is on $PATH;
-- air-gapped users who unpacked the tarball elsewhere can override it below.
vim.lsp.config["spicefmt"] = {
  cmd = { vim.env.SPICEFMT_LS_CMD or "spice-netlist-ls" },
  filetypes = { "spice", "cir", "scs", "subckt" },
  root_markers = { ".git", "spicefmt.toml" },
}
-- Enable only in this buffer; `enable()` would attach on every filetype match
-- in other windows too, which is exactly what ftplugin is for.
vim.lsp.enable("spicefmt")

-- Format-on-save through the LSP (`textDocument/formatting`). Remove if the
-- user prefers to trigger manually.
local group = vim.api.nvim_create_augroup("SpicefmtFormatOnSave", { clear = false })
vim.api.nvim_clear_autocmds({ group = group, buffer = 0 })
vim.api.nvim_create_autocmd("BufWritePre", {
  group = group,
  buffer = 0,
  callback = function() vim.lsp.buf.format({ async = false }) end,
})

vim.bo.commentstring = "* %s"