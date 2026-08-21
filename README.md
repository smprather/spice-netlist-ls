# spice-netlist-ls — HSPICE-golden, dialect-extensible formatter

Highly opinionated `gofmt` for SPICE. HSPICE is golden reference (dialect trait pluggable for NGSPICE/Spectre/LTSpice without re-arch).

> Naming: the formatter is **`spicefmt`** (gofmt/rustfmt/shfmt convention). The language server binary is `spice-netlist-ls`. The SPICE *netlist* format predates the SPICE-*the-language* ecosystem — we claim `spicefmt`.

> First draft: formatter + LSP stub in Rust. `spice-lsp` (Python, `spice-lang` .spc) is a different language — we target true SPICE netlists.

## Why

SPICE netlists look like they need an AST — they don't. `File -> Stmt*` where `Stmt = Comment|Directive|Instance|Subckt(Block)` is exact. Complexity is preprocessing (`.param` exprs, `.lib/.include`) and dialect variance.

## HSPICE manual compliance (B-2008.09 Ch.4)

- **Tokens** delimited by `space/tab/,=()` — `,` is whitespace (p40-46)
- **Continuation** `+` as first non-numeric, non-blank char of next line (p40); quoted-string continuation ` \` / ` \\` preserved
- **First char** `*` comment, `+` continuation, `.` keyword, else title (p45)
- **Case-insensitive** except filenames/paths; 1024-char limits (p40)
- **Formatter**: lowercases directives (`.subckt/.ends/.model/.param`), `key = value` with spaces, wraps at 80 with `+ `, idempotent

## Usage

```bash
cargo build --release
./target/release/spicefmt file.sp              # stdout
./target/release/spicefmt --write file.sp      # inplace
./target/release/spicefmt --check file.sp      # CI
./target/release/spicefmt --print-dialect file.sp
echo ".PARAM w=1u" | ./target/release/spicefmt --dialect hspice
./target/release/spice-netlist-ls                # LSP (formatting + diagnostics stub)
```

Dialect is auto-detected per file: dialect-exclusive grammar is decisive (`.control`/`.csparam`/`$&` meas-result refs → ngspice; `.alter`/`.protect` → hspice; `.step`/`.backanno` → ltspice; `//` comments/paren node syntax → spectre); shared markers (`.probe`, `;` vs `$`) add weak evidence; falls back to hspice. Override with `--dialect`.

## Extensibility

```rust
trait Dialect { fn kind(); fn is_comment_line(); fn continuation_char() -> '+'; fn inline_comment_delim() -> '$'|'//'|';'; }
get_dialect(Hspice) // Ngspice -> ';' , Spectre -> '//'
```

Add a dialect: implement `Dialect`, register in `dialect_from_str`, no parser re-arch.

## Speed

1009-line PEX: **Rust 2.6ms** vs Python `spice.lexer` 100ms (40×). Validated vs `ngspice` — formatted netlists simulate identically.

## Roadmap

- [x] HSPICE formatter (idempotent, generic params)
- [x] LSP formatting stub
- [x] LSP go-to-definition: `X` instantiation → `.subckt` (follows `.include`/`.inc`/`.lib` transitively)
- [x] Linter (undefined subckt, arity, floating nodes, duplicate instances, unterminated subckt) — LSP diagnostics + `spicefmt --lint`
- [ ] PyPI wrapper package exposing `spicefmt` (must not collide with the Rust binary's entry point)

## Test

```bash
cargo test
ngspice -b testdata/simple_rc_chain.subckt  # via fmt roundtrip
```
