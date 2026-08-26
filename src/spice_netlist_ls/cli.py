"""Rich-click CLI wrappers for spicefmt and spice-netlist-ls.

The Python package is a thin wrapper that execs the bundled Rust binaries
(`src/spice_netlist_ls/bin/` or `$SPICEFMT_BIN` / `$SPICE_NETLIST_LS_BIN`).
Rich-click is used only for `--help` rendering — all other invocations are
passed through to Rust unchanged, so the wrapper never reimplements formatting
or LSP logic.

Offline note: `dependencies = ["rich-click"]` is a hard runtime dep so
`uv tool install .` in a release tarball/clone gets rich help by default.
The lockfile (`uv.lock`) pins the exact version; an air-gapped
`uv tool install . --offline` works after a single `uv sync` that populates
the cache, or from a wheel built with `uv build` which bundles no extra
network fetch at install time beyond the already-locked `rich-click` + `rich`.
If `rich-click` is somehow absent, the wrapper falls back to plain `click`
and finally to execing the Rust binary's own `--help`.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

# Prefer rich_click for styled help, fall back to click, then to Rust passthrough.
try:
    import rich_click as click  # type: ignore[import-not-found]
    from rich_click import RichGroup, RichCommand  # type: ignore[import-not-found] # noqa: F401

    click.rich_click.USE_RICH_MARKUP = True  # type: ignore[attr-defined]
    click.rich_click.USE_MARKDOWN = False  # type: ignore[attr-defined]
    click.rich_click.SHOW_ARGUMENTS = True  # type: ignore[attr-defined]
    click.rich_click.GROUPED_DISPLAY = False  # type: ignore[attr-defined]
    click.rich_click.MAX_WIDTH = 100  # type: ignore[attr-defined]
    click.rich_click.STYLE_HELPTEXT_FIRST_LINE = "bold"  # type: ignore[attr-defined]
except ImportError:  # pragma: no cover - fallback for minimal offline without rich-click
    try:
        import click  # type: ignore[import-not-found]
    except ImportError:  # pragma: no cover - no click at all, delegate to Rust
        click = None  # type: ignore[assignment]

# Reuse binary resolution from the package.
from . import find_binary

_PKG_VERSION = "0.6.0"


# ---------------------------------------------------------------------------
# spicefmt — mirrors src/cli.rs Args for help only; execution is Rust.
# ---------------------------------------------------------------------------

if click is not None:
    @click.command(
        context_settings=dict(help_option_names=["-h", "--help"], max_content_width=100),
        help="Opinionated SPICE netlist formatter and linter — dialect-extensible (hspice, ngspice, spectre, ltspice)",
        epilog="In .scs files, `simulator lang=spice`/`lang=spectre` switch the active dialect per section; --dialect sets only the fallback for the implicit pre-switch section.",
    )
    @click.argument("files", nargs=-1, type=click.Path(path_type=Path), metavar="FILE")
    @click.option("--check", is_flag=True, help="Check only, exit 1 if not formatted")
    @click.option("--write", is_flag=True, help="Write back to file in-place")
    @click.option(
        "--dialect",
        type=click.Choice(["hspice", "ngspice", "spectre", "ltspice", "auto"], case_sensitive=False),
        default=None,
        help="Dialect: hspice, ngspice, spectre, ltspice, or auto (default: auto)",
    )
    @click.option("--print-dialect", is_flag=True, help="Detect and print dialect per input, no formatting")
    @click.option("--lint", is_flag=True, help="Lint only: print diagnostics, exit code governed by --error-on/--max-warnings")
    @click.option(
        "--format",
        "lint_format",
        type=click.Choice(["human", "json", "sarif", "summary"], case_sensitive=False),
        default="human",
        show_default=True,
        help="Diagnostic output format (used with --lint)",
    )
    @click.option(
        "--error-on",
        type=click.Choice(["error", "warning"], case_sensitive=False),
        default="error",
        show_default=True,
        help="Lowest severity that fails the lint run",
    )
    @click.option("--max-warnings", type=int, default=None, metavar="N", help="Fail when more than N (non-suppressed) warnings are reported")
    @click.option("--list-dialects", is_flag=True, help="Print dialect list and exit")
    @click.version_option(_PKG_VERSION, "-V", "--version", prog_name="spicefmt")
    def _spicefmt_click(files, check, write, dialect, print_dialect, lint, lint_format, error_on, max_warnings, list_dialects):  # pragma: no cover
        # This body is never used for real execution — the wrapper execs Rust.
        # It exists so `rich_click` can render help. If someone invokes the
        # Python entry point via `python -m` without the Rust binary, give a hint.
        click.echo("spicefmt: Rust binary not invoked directly via Python CLI help. Use `spicefmt --help` for usage.", err=True)  # type: ignore[attr-defined]

    @click.command(
        context_settings=dict(help_option_names=["-h", "--help"]),
        help="SPICE language server (formatting + diagnostics + semanticTokens + go-to-definition)",
        epilog="Run with no arguments; communicates via LSP stdio. Editors launch it as:\n\n  vim.lsp.config[\"spicefmt\"] = { cmd = { \"spice-netlist-ls\" }, filetypes = { \"spice\", \"cir\", \"scs\", \"subckt\" } }",
    )
    @click.version_option(_PKG_VERSION, "-V", "--version", prog_name="spice-netlist-ls")
    def _lsp_click():  # pragma: no cover
        # click handles --help/--version; this body is fallback for plain invoke
        click.echo("spice-netlist-ls: use --help for usage", err=True)  # type: ignore[attr-defined]

else:  # no click available
    _spicefmt_click = None  # type: ignore
    _lsp_click = None  # type: ignore


def _exec_rust(name: str) -> None:
    """Exec the Rust binary, preserving args and exit code via execv."""
    from . import main as _main  # reuse find_binary + execv

    _main(name)


def spicefmt() -> None:
    """Entry point for `spicefmt` (rich help, Rust exec)."""
    # Fast path for --help: render with rich-click without needing Rust binary
    if click is not None and any(a in sys.argv for a in ("--help", "-h", "--version", "-V", "--list-dialects")):
        # For --version, click's version_option handles it; for --help, same
        # Use standalone_mode=False to avoid sys.exit in tests, but in cli we want normal exit
        try:
            # Re-invoke the click command with original args (skip prog name)
            # This will print help/version and exit via SystemExit
            _spicefmt_click.main(args=sys.argv[1:], prog_name="spicefmt", standalone_mode=True)  # type: ignore
        except SystemExit as e:
            # click already printed help/version; propagate exit code
            sys.exit(e.code)
        return  # pragma: no cover

    # Fallback if click not available but --help requested: delegate to Rust which has clap help
    # (Rust's --help is plain, but better than nothing)
    _exec_rust("spicefmt")


def lsp() -> None:
    """Entry point for `spice-netlist-ls` (rich help, Rust exec)."""
    if click is not None and any(a in sys.argv for a in ("--help", "-h", "--version", "-V")):
        try:
            _lsp_click.main(args=sys.argv[1:], prog_name="spice-netlist-ls", standalone_mode=True)  # type: ignore
        except SystemExit as e:
            sys.exit(e.code)
        return  # pragma: no cover
    _exec_rust("spice-netlist-ls")


# Backwards-compat: `python -m spice_netlist_ls` etc. (if needed)
if __name__ == "__main__":
    # Determine prog by argv[0] basename
    prog = Path(sys.argv[0]).name
    if "spice-netlist-ls" in prog or "lsp" in prog:
        lsp()
    else:
        spicefmt()
