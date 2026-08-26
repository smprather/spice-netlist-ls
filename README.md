# spice-netlist-ls — a formatter and language server for SPICE netlists

> **Current status (2026-08-26):** `v2026.8.0` on `main` (calver: `year.month.patch`). Formatter is 14-rule opinionated (`src/formatter.rs:7` `ALL_FORMAT_RULES`) with ruff `ignore`/`select` (`spicefmt.toml` `[format]` + `--ignore`/`--select`). Releases always include `## What's Changed` (`.github/workflows/release.yml:139`). `fmt: off/on/skip` pragmas (statement-level, subckt-safe) and `ends-name` (always add `s.name` after `.ends`) are implemented.

An opinionated formatter, linter, and LSP server for the classic for the
SPICE circuit-simulation netlist format — with pluggable support for the dialects
that grew out of it (HSPICE, NGSPICE, Spectre-SPICE, LTspice).

> This tool formats *netlists* for the SPICE circuit simulator
> created at UC Berkeley in 1972. Note the Spice programming
> language announced in 2021 (`spice-lang`, `.spc` files).

## Neovim setup — copy-paste, works for everyone (no plugins required)

> **One binary, zero plugins.** `spice-netlist-ls` provides formatting, diagnostics, go-to-definition, and semantic highlights (nets!) — no `:TSInstall`, no tree-sitter grammar.

**Requires:** Neovim ≥0.11 for zero-config (≥0.10 with `nvim-lspconfig`, `<0.10` gets offline regex highlights only). `spice-netlist-ls` on `$PATH`.

### 1. Install the binary

```bash
cargo install --path .                          # from source
# or grab a static tarball from Releases and put spice-netlist-ls + spicefmt on $PATH
# https://github.com/smprather/spice-netlist-ls/releases
cargo install --path . --locked                 # reproducible build
```

Verify: `spice-netlist-ls --help` and `spicefmt --help` print.

### 2. Tell Neovim which files are SPICE (required)

`after/ftplugin/spice.lua` only fires *after* `filetype=spice` is set — without this step `ft` stays `conf`/`""` and the server never attaches.

**Easiest (works on every Neovim, no lua):**

```bash
mkdir -p ~/.config/nvim/ftdetect
cp contrib/vim/ftdetect/spice.vim ~/.config/nvim/ftdetect/spice.vim  # covers *.sp,*.cir,*.ckt,*.net,*.scs,*.subckt,*.sub — see contrib/vim/ftdetect/spice.vim:8
```

**Alternative — lua in `init.lua`:**

```lua
vim.filetype.add({
  extension = {
    sp = "spice", cir = "spice", ckt = "spice", net = "spice",
    scs = "spice", subckt = "spice", sub = "spice",
    spice = "spice", cdl = "spice", pex = "spice",
  },
})
```

Either one is enough. Verify: `:e foo.sp` then `:set ft?` → `spice` (not `conf`).

### 3. Connect the language server — pick ONE

Air-gapped / tarball not on `$PATH`? `export SPICEFMT_LS_CMD=/path/to/spice-netlist-ls` — all configs below respect it.

#### Option A — Zero-config, no plugin manager (recommended for everyone, Neovim ≥0.11)

```bash
mkdir -p ~/.config/nvim/after/ftplugin
cp after/ftplugin/spice.lua ~/.config/nvim/after/ftplugin/spice.lua
# restart nvim, open any SPICE file
```

What you copied (`after/ftplugin/spice.lua:1`): registers `cmd = { "spice-netlist-ls" }`, `filetypes = { "spice" }`, `root_markers = { ".git", "spicefmt.toml" }`, enables the server for this buffer, sets `commentstring`, adds **format-on-save** (`BufWritePre → vim.lsp.buf.format`), and links per-filetype semantic highlights (`@lsp.type.variable.spice → Identifier` for nets — the fix for “nets show no color”).

Remove the `BufWritePre` block in that file if you prefer manual `:lua vim.lsp.buf.format()`.

#### Option B — Native `lsp/` directory (structured configs, Neovim ≥0.11, no lspconfig)

For configs that keep servers in `~/.config/nvim/lsp/` and enable them in `init.lua` (like this repo’s `lua/user/init.lua:22`):

```bash
mkdir -p ~/.config/nvim/lsp
cp contrib/lspconfig-spicefmt.lua ~/.config/nvim/lsp/spicefmt.lua
# or cp to spice_netlist_ls.lua if you prefer that name — just enable the same name
```

```lua
-- in init.lua
vim.lsp.enable("spicefmt")  -- or "spice_netlist_ls" if you copied to that name
```

`contrib/lspconfig-spicefmt.lua:1` is already a `vim.lsp.Config` ( `---@type vim.lsp.Config` ) — no wrapper needed for native `lsp/` . `after/ftplugin/spice.lua` still supplies the highlight links + format-on-save even in this mode; keep it (Option A) *or* add the highlight snippet from Troubleshooting below to `init.lua`.

#### Option C — nvim-lspconfig (Neovim ≥0.10)

If you already use `neovim/nvim-lspconfig`:

```lua
-- init.lua with lspconfig
require("lspconfig").spicefmt.setup({
  cmd = { "spice-netlist-ls" },
  filetypes = { "spice" },
  root_markers = { ".git", "spicefmt.toml" },
})
-- or via lazy.nvim
{ "neovim/nvim-lspconfig", opts = { servers = { spicefmt = {} } } }
```

`contrib/lspconfig-spicefmt.lua:1` is the upstream-ready config for this path. With lspconfig, copy `ftdetect` (step 2) and add the highlight snippet from Troubleshooting below — `after/ftplugin/spice.lua`’s autocmd won’t run unless you also copy it.

### 4. What you get

* **Format on save** — via `textDocument/formatting` (included in `after/ftplugin`; remove autocmd for manual).
* **Diagnostics** — undefined subckt, arity, floating nodes, orphan `+` continuations, `.ends` name mismatch, etc. (same as `spicefmt --lint`).
* **Go-to-definition** — `gd` on an `X` line jumps to its `.subckt` (follows `.include`/`.lib` transitively).
* **Rename** — `F2` on a net (instance nodes, `.subckt` ports, `X` pins) or param key (`key=value`, `.param` definitions) renames every occurrence in the file; model/instance names and values are not renameable (`src/rename.rs`).
* **Semantic highlighting** — nets (`variable`), subckt names (`type`), instance names (`function`), param keys (`property`) etc via `semanticTokensProvider` (`src/semantic_tokens.rs:40`). Dialect-aware (Spectre `(a b)` parens `src/dialect.rs:82`, `+` continuations `src/parser.rs:84`). No tree-sitter grammar needed. See [Syntax highlighting](#syntax-highlighting) for offline regex fallback.

### 5. Verify — prove nets are colored

Open the test file and check:

```bash
nvim testdata/simple_rc_chain.subckt
```

```vim
:set ft?                     " spice  — if not, step 2 failed
:LspInfo                     " spicefmt (or spice_netlist_ls) attached
:checkhealth vim.lsp
:hi @lsp.type.variable.spice " links to Identifier (not 'cleared' — if cleared, re-copy after/ftplugin/spice.lua:32)
:Inspect                     " on `in` in `R1 in mid {rser}` → @lsp.type.variable.spice
:lua vim.lsp.buf.format()    " should format
gd                           " on `X1 in n1 rcstage` → jumps to `.subckt rcstage`
```

If `LspInfo` empty: `:echo exepath("spice-netlist-ls")` should print a path — if empty, binary not on `$PATH` or set `$SPICEFMT_LS_CMD`. If `ft` is wrong, see step 2. If highlight is `cleared`, see Troubleshooting.

### 6. Troubleshooting

**Nets (or other tokens) show no color?** Neovim uses per-filetype groups (`@lsp.type.variable.spice`), not the generic `@lsp.type.variable` most colorschemes define. `after/ftplugin/spice.lua:32` already links them for Option A/B-native. If you used lspconfig (Option C) or a colorscheme that clears them, add to `init.lua` (sourced *after* your colorscheme):

```lua
local links = {
  ["@lsp.type.variable"] = "Identifier", -- nets / ports — the "net names" group
  ["@lsp.type.type"]     = "Type",
  ["@lsp.type.function"] = "Function",
  ["@lsp.type.keyword"]  = "Keyword",
  ["@lsp.type.property"] = "Identifier",
  ["@lsp.type.string"]   = "String",
  ["@lsp.type.number"]   = "Number",
  ["@lsp.type.comment"]  = "Comment",
  ["@lsp.type.operator"] = "Operator",
}
for base, target in pairs(links) do
  for _, ft in ipairs({ "spice", "cir", "scs", "subckt", "sp", "ckt", "net" }) do
    vim.api.nvim_set_hl(0, base .. "." .. ft, { link = target, default = true })
  end
  vim.api.nvim_set_hl(0, base, { link = target, default = true })
end
vim.api.nvim_create_autocmd("ColorScheme", {
  callback = function()
    for base, target in pairs(links) do
      for _, ft in ipairs({ "spice", "cir", "scs", "subckt", "sp", "ckt", "net" }) do
        vim.api.nvim_set_hl(0, base .. "." .. ft, { link = target, default = true })
      end
    end
  end,
})
```

Then `:hi @lsp.type.variable.spice` should `links to Identifier` and `:Inspect` on a net shows that group. Re-copying `after/ftplugin/spice.lua` does the same for Option A.

**Filetype mismatch** (`.sp` → `conf`): `*.sp` is claimed by `conf` in stock Neovim. Our `ftdetect` (step 2) overrides it — without it, `after/ftplugin` never fires. Check `:set ft?` and see [Filetype detection](#filetype-detection) above.

**Old Neovim (<0.10):** semantic tokens unsupported — nets fall back to regex `contrib/vim/syntax/spice.vim:1` (ports → `Identifier` but no precise net/param split — that needs the LSP).

**No formatting:** remove the `BufWritePre` to debug, then `:lua vim.lsp.buf.format()` manually; check `spicefmt` on CLI: `echo "* title\nR1 a b 1k" | spicefmt --dialect hspice`.

### 7. Mason (optional)

`contrib/mason-package.yaml:1` is a draft registry entry. Until upstreamed, install the binary manually (step 1) — Mason will pick it up from `$PATH`. Air-gapped sites can mirror the GitHub Release tarballs and point `mason` `registries` at their mirror.

---

## Why

SPICE netlists look like they need an AST — they don't. `File -> Stmt*` where `Stmt = Comment|Directive|Instance|Subckt(Block)` is exact. Complexity is preprocessing (`.param` exprs, `.lib/.include`) and dialect variance.

## Dialects

Dialect is auto-detected per file: dialect-exclusive grammar is decisive
(`.control`/`.csparam`/`$&` meas-result refs → ngspice; `.alter`/`.protect` → hspice;
`.step`/`.backanno` → ltspice; `//` comments/paren node syntax → spectre); shared
markers (`.probe`, `;` vs `$`) add weak evidence; falls back to hspice. Override with
`--dialect` or a `spicefmt.toml`.

The parser core is shared; each dialect only supplies its own grammar quirks via a
small trait (see [Extensibility](#extensibility)). HSPICE served as the reference
implementation of the trait because its manual pins the base syntax down precisely:

> **Why "Spectre-SPICE"?** The `spectre` dialect here means *SPICE syntax as accepted
> by Cadence Spectre* (parenthesized nodes, `//` comments, `key=value` params). It is
> **not** Cadence's native Spectre netlist language, which is an entirely different
> format and not supported.

- **Tokens** delimited by `space/tab/,=()` — `,` is whitespace
- **Continuation** `+` as first non-numeric, non-blank char of next line; quoted-string continuation ` \` / ` \\` preserved
- **First char** `*` comment, `+` continuation, `.` keyword, else title
- **Case-insensitive** except filenames/paths

Other dialects override these defaults where they differ (Spectre-SPICE emits
`key=value`, ngspice and ltspice use `;` inline comments, Spectre-SPICE uses `//`).

### `.scs` per-section dialect switching

A `.scs` (Spectre netlist) file can mix dialects with `simulator lang=spice` /
`simulator lang=spectre` directives: each directive switches the active dialect
for the lines that follow, and the formatter/linter routes each section to the
right existing dialect (`spice` → ngspice grammar, `spectre` → Spectre-SPICE
grammar). The directive lines themselves are structural (not comments) and pass
through the formatter verbatim. Subckts defined in one section are visible to
instantiations in any other section of the same file; includes are walked under
each section's dialect. `--dialect`/`spicefmt.toml` set only the *fallback*
dialect for the implicit pre-switch section; explicit `simulator lang=` lines in
the file always win per-section. Plain files with no `simulator lang=` directive
are unaffected — they take the single-dialect fast path byte-identical to before.

## Usage

```bash
cargo build --release
./target/release/spicefmt file.sp              # stdout
./target/release/spicefmt --write file.sp      # inplace
./target/release/spicefmt --check file.sp      # CI
./target/release/spicefmt --print-dialect file.sp
echo ".PARAM w=1u" | ./target/release/spicefmt --dialect hspice
./target/release/spice-netlist-ls                # LSP (formatting + diagnostics)
```

## Formatter invariants

- Output is **idempotent** — `spicefmt | spicefmt` is a fixed point.
- A `+` continuation joined to a parent is merged; an **orphan** continuation (no parent statement, e.g. the parent was commented out) is preserved verbatim and flagged by the linter as `orphan-continuation`.
- Inline comments stay comments: a comment bumped past `max_width` moves to its own `+ <delim> text` continuation line instead of being split mid-comment into what would become code.
- `key = value` spacing follows the dialect: HSPICE-style spacing by default, Spectre-SPICE emits `key=value`.
- `.ends <name>` that names a different subckt than the one it closes is kept as written and flagged as `ends-name-mismatch`.
- Blank line before a top-level `.subckt` is preserved but **not forced** when the previous line is a comment — `* comment` directly before `.subckt` stays `* comment\n.subckt` (no extra blank), while `param → subckt` keeps the separating blank for readability (`src/formatter.rs:159`).

## Configuration

A `spicefmt.toml` in the file's directory (or any ancestor, or your user config dir)
overrides defaults; CLI flags beat the config file:

```toml
dialect = "spectre"   # or omit/auto for per-file detection
max_width = 100
sort_params = true    # sort key=value params after positional args

# Formatter is opinionated but ruff-inspired opt-out is available:
[format]
# Disable specific formatting rules (all enabled by default)
# Available: lowercase-directive, eq-spacing, continuation-join,
#   line-wrap, sort-params, blank-before-subckt, blank-after-subckt,
#   blank-before-ends, blank-after-ends, blank-collapse,
#   comment-normalize, trim-trailing-whitespace, insert-final-newline,
#   ends-name
ignore = ["blank-after-subckt"]   # keep your empty line after .subckt
# select = ["blank-after-ends"]   # allowlist – only these run

[lint]
# Like ruff's `select`/`ignore` – control which diagnostics are reported.
# `suppress` is kept for backwards compat (alias for `ignore`).
# Available codes: undefined-subckt, arity-mismatch, floating-node,
# dangling-rc-endpoint, duplicate-instance, unterminated-subckt,
# stray-ends, ends-name-mismatch, node-case-collision, orphan-continuation,
# plus the three blank-line codes above (reported by `spicefmt --lint`).
# Format-only rules (lowercase-directive, eq-spacing, etc.) are not reported
# as lint – they are fixed by the formatter and opt-out via [format] ignore.
ignore = ["blank-before-ends"]    # don't warn about blank before .ends
# select = ["blank-after-ends"]   # only warn about that one
# severity overrides: "code" = "error" | "warning"
[lint.severity]
# "blank-after-ends" = "error"
```

CLI also supports ruff-style `--ignore`/`--select` (comma-separated or repeated):

```bash
spicefmt --ignore blank-after-subckt,blank-before-ends file.sp
spicefmt --lint --ignore blank-after-ends --format summary
spicefmt --ignore lowercase-directive,eq-spacing --write file.sp
```

Formatter rules (all fixable via `spicefmt` or `spicefmt --write`):

- `lowercase-directive`          – `.SUBCKT` → `.subckt` (`src/formatter.rs:457`)
- `eq-spacing`                   – `k=v` ↔ `k = v` per dialect (`src/formatter.rs:499`)
- `continuation-join`            – `+` lines joined (`src/parser.rs:84`)
- `line-wrap`                    – wrap at `max_width` (`src/formatter.rs:335`)
- `sort-params`                  – sort `key=value` when `sort_params` (`src/formatter.rs:523`)
- `blank-before-subckt`          – blank before top-level `.subckt` (`src/formatter.rs:230`)
- `blank-after-subckt`           – no empty after `.subckt` (nesting-aware)
- `blank-before-ends`            – no empty before `.ends`
- `blank-after-ends`             – ≥1 empty after `.ends` (top-level, collapsed)
- `blank-collapse`               – collapse consecutive blanks (`src/formatter.rs:192`)
- `comment-normalize`            – `*foo` → `* foo` (`src/formatter.rs:531`)
- `trim-trailing-whitespace`     – strip trailing spaces (`src/formatter.rs:145`)
- `insert-final-newline`         – ensure `\n` at EOF (`src/formatter.rs:161`)
- `ends-name`                    – always add `s.name` after `.ends` (`src/formatter.rs:493`)

### `fmt: off/on/skip` (ruff-style pragmas)

Any comment line switches verbatim regions: `* fmt: off` … `* fmt: on`, or
`// fmt: off`, `$ fmt: off`, `; fmt: off`, or a bare `fmt: off` / `spicefmt: off`
line — case-insensitive, the `:` is optional (`*fmt:off`, `* FMT OFF`). Lines
between `off` and `on` (and the pragma lines themselves) are emitted exactly
as written; no rule touches them, not even trailing-whitespace trimming.

`fmt: skip` keeps the next statement verbatim (`* fmt: skip` on its own line —
skips the next non-blank statement, including a whole `.subckt` block), or,
written inline (`R1 a b 1k $ fmt: skip`), keeps just that statement. Pragmas
work anywhere, including inside `.subckt` bodies, and are idempotent: running
`spicefmt` again leaves the verbatim regions untouched.

```spice
* fmt: off
R1 a b 1k     tc=2     $ hand-tuned layout, do not touch
* fmt: on
.subckt inv a b
* fmt: skip
R1    a   b   1k
.ends inv
```

`.editorconfig` files are honored too ([editorconfig.org](https://editorconfig.org)):
`max_line_length`, `insert_final_newline`, and `trim_trailing_whitespace` apply to
your netlist files via their usual glob sections. Precedence, loosest to tightest:

1. built-in defaults
2. `.editorconfig` (walked up to `root = true`)
3. `spicefmt.toml`
4. CLI flags (`--dialect`, `--ignore`, `--select`, etc.)

The LSP applies the same search so your editor and CI agree.

## Extensibility

```rust
trait Dialect { fn kind(); fn is_comment_line(); fn continuation_char() -> '+'; fn inline_comment_delim() -> '$'|'//'|';'; }
get_dialect(Hspice) // Ngspice -> ';' , Spectre -> '//'
```

Add a dialect: implement `Dialect`, register in `dialect_from_str`, no parser re-arch.

## Speed

1009-line PEX netlist: **2.6ms**. Validated against `ngspice` — formatted netlists simulate identically.

## Roadmap

- [x] Formatter (idempotent, generic params, all four dialects)
- [x] Dialect auto-detection + config-file overrides
- [x] LSP formatting, go-to-definition (`X` instantiation → `.subckt`, follows `.include`/`.inc`/`.lib` transitively), diagnostics
- [x] LSP rename (`F2`) for nets (nodes, `.subckt` ports, `X` pins) and param keys, single-file
- [x] Linter (undefined subckt, arity, floating nodes, duplicate instances, unterminated subckt, `.ends` name mismatch, stray `.ends`, node case-collision, orphan continuation) — LSP diagnostics + `spicefmt --lint`
  - ngspice `.control`/`.endc` interiors are command language, not netlist cards: no instance/node analysis inside
  - nodes referenced by `.measure`/`.meas`, `.probe`, `.print`, `.plot`, `save`, or `v(...)` in a `let`/`meas` are *observed* and never reported floating
  - one-terminal passive-network endpoints get their own code, `dangling-rc-endpoint`, distinct from a dangling device pin (`floating-node`)
- [x] Lint ergonomics: `--format human|json|sarif|summary`, `--error-on warning|error`, `--max-warnings N`; `[lint]` table in `spicefmt.toml` for per-code `suppress` (hidden from detail, still counted in summary) and severity overrides; JSONL output is **one object per line** with a `schema_version` field on every record
- [x] PyPI wrapper package exposing `spicefmt` (`uv tool install .`; entry points exec the Rust binaries, bundled at build time or resolved from `$SPICEFMT_BIN`/PATH)

## Syntax highlighting

Nets (`variable`), subckt names (`type`), instance names (`function`), param keys (`property`) etc are colored via **LSP semanticTokens** — no tree-sitter grammar required. `spice-netlist-ls` advertises `semanticTokensProvider` (`src/semantic_tokens.rs:40`, `src/bin/ls.rs:10`); Neovim ≥0.10, Helix ≥23.10, VS Code, Zed render them automatically when the server is attached. For net names this is dialect- and arity-accurate (uses the same `element_node_count` `src/parser.rs:655` the linter does, including Spectre `(a b)` parens `src/dialect.rs:82` and `+` continuations `src/parser.rs:84`). Independent-source functions (`pulse(0 1.2 …)`, `pwl(…)`, `sin(…)`) are split: the function name → `type`, `(` / `)` → `operator`, inner numbers → `number` (`src/semantic_tokens.rs:358` — fixes `pulse(0` where `0` was previously the same color as `(`).

- **Neovim**: `after/ftplugin/spice.lua` registers the server; semantic tokens are enabled automatically. No `:TSInstall` needed. Optional regex fallback for offline viewing: `contrib/vim/syntax/spice.vim` + `ftdetect/spice.vim`.
- **Helix**: add `contrib/helix/languages.toml` to `~/.config/helix/languages.toml`; nets are highlighted via the LSP. Copy `queries/spice/highlights.scm` to `runtime/queries/spice/highlights.scm` only if you also install a `tree-sitter-spice` parser for offline highlighting.
- **Vim8 / bare vim**: copy `contrib/vim/ftdetect/spice.vim`, `contrib/vim/syntax/spice.vim`, `contrib/vim/ftplugin/spice.vim` into `~/.vim/` (regex fallback highlights directives/params/numbers/strings/comments; precise net coloring needs the LSP).
- **Do we need tree-sitter?** No for accurate colors — LSP already does. Tree-sitter is only for offline highlighting without the server (heavier to maintain, needs a separate C grammar kept in sync with the Rust parser). Hybrid is shipped: LSP primary, regex/queries fallback.

## Install

Releases ship static binaries for Linux (musl), macOS, and Windows via
[cargo-dist](https://opensource.axo.dev/cargo-dist/) — see the
[releases page](https://github.com/smprather/spice-netlist-ls/releases). A man page
(`spicefmt.1`) is included in each tarball. For Neovim, `contrib/` has an
nvim-lspconfig snippet and a mason package manifest; `after/ftplugin/spice.lua`
wires formatting up on save.

## Test

```bash
cargo test                                        # unit + CLI integration + insta snapshots
INSTA_UPDATE=always cargo test --test snapshots   # accept snapshot updates
ls testdata/                                      # dialect fixtures; snapshot-tested
```

`testdata/simple_rc_chain.subckt:3` is `* RC lowpass` (comment title) — the bare form `RC lowpass` would look like `R` device `RC` with single node `lowpass` and no value, which is not a valid resistor and is intentionally flagged as `dangling-rc-endpoint` (`src/linter.rs:862` testcase `bare_title_like_rc_lowpass_is_flagged_as_invalid_resistor`). Keep the `*` in the file; use the bare form to test the linter's detection of a malformed `R` card.
