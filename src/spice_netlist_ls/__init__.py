"""Python wrapper package for spice-netlist-ls.

Exposes the Rust-built `spicefmt` and `spice-netlist-ls` binaries as
console entry points. The wrapper never reimplements the tools; it only
locates the real binary and execs it:

  1. ``$SPICEFMT_BIN`` / ``$SPICE_NETLIST_LS_BIN``
  2. binaries bundled into the wheel at build time (``bin/``)
  3. first match on ``PATH`` that is not the wrapper itself
"""

import os
import sys
from pathlib import Path

_PKG_BIN = Path(__file__).resolve().parent / "bin"


def find_binary(name: str) -> str | None:
    env_key = name.upper().replace("-", "_") + "_BIN"
    if env := os.environ.get(env_key):
        return env
    if (_PKG_BIN / name).is_file():
        return str(_PKG_BIN / name)

    me = Path(sys.argv[0]).resolve()
    for d in os.environ.get("PATH", "").split(os.pathsep):
        cand = Path(d) / name
        if not cand.is_file():
            continue
        try:
            if cand.samefile(me):
                continue
        except OSError:
            pass
        return str(cand)
    return None


def main(name: str) -> None:
    binary = find_binary(name)
    if binary is None:
        sys.exit(
            f"error: could not locate the {name} Rust binary.\n"
            f"Build it (cargo build --release), bundle it, or point "
            f"${name.upper().replace('-', '_')}_BIN at it."
        )
    os.execv(binary, [binary, *sys.argv[1:]])


def spicefmt() -> None:
    main("spicefmt")


def lsp() -> None:
    main("spice-netlist-ls")
