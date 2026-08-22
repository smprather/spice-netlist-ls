# spice-netlist-ls — a formatter and language server for SPICE netlists

`gofmt` for SPICE: an opinionated formatter, linter, and LSP server for the classic
SPICE circuit-simulation netlist format — with pluggable support for the dialects
that grew out of it (HSPICE, NGSPICE, Spectre-SPICE, LTspice).

> **Not that SPICE.** This tool formats *netlists* for the SPICE circuit simulator
> created at UC Berkeley in 1972. It has nothing to do with the SPICE programming
> language announced in 2021 (`spice-lang`, `.spc` files) — different language,
> different ecosystem.

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

## Configuration

A `spicefmt.toml` in the file's directory (or any ancestor, or your user config dir)
overrides defaults; CLI flags beat the config file:

```toml
dialect = "spectre"   # or omit/auto for per-file detection
max_width = 100
sort_params = true    # sort key=value params after positional args
```

`.editorconfig` files are honored too ([editorconfig.org](https://editorconfig.org)):
`max_line_length`, `insert_final_newline`, and `trim_trailing_whitespace` apply to
your netlist files via their usual glob sections. Precedence, loosest to tightest:

1. built-in defaults
2. `.editorconfig` (walked up to `root = true`)
3. `spicefmt.toml`
4. CLI flags

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
- [x] Linter (undefined subckt, arity, floating nodes, duplicate instances, unterminated subckt, `.ends` name mismatch, stray `.ends`, node case-collision, orphan continuation) — LSP diagnostics + `spicefmt --lint`
  - ngspice `.control`/`.endc` interiors are command language, not netlist cards: no instance/node analysis inside
  - nodes referenced by `.measure`/`.meas`, `.probe`, `.print`, `.plot`, `save`, or `v(...)` in a `let`/`meas` are *observed* and never reported floating
  - one-terminal passive-network endpoints get their own code, `dangling-rc-endpoint`, distinct from a dangling device pin (`floating-node`)
- [x] Lint ergonomics: `--format human|json|sarif|summary`, `--error-on warning|error`, `--max-warnings N`; `[lint]` table in `spicefmt.toml` for per-code `suppress` (hidden from detail, still counted in summary) and severity overrides; JSONL output is **one object per line** with a `schema_version` field on every record
- [x] PyPI wrapper package exposing `spicefmt` (`uv tool install .`; entry points exec the Rust binaries, bundled at build time or resolved from `$SPICEFMT_BIN`/PATH)

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
