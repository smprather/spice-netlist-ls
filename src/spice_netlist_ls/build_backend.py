"""Custom PEP 517 build backend for spice-netlist-ls.

Delegates to ``uv_build`` but first rebuilds the Rust workspace and bundles
the release binaries into ``src/spice_netlist_ls/bin/`` so every
``uv build`` / ``uv tool install .`` packages current code.

Without this, a reinstall would happily repackage whatever stale binaries
happened to sit in ``bin/`` — the wrapper resolves bundled-before-PATH, so a
forgotten manual refresh silently serves old code.
"""

import shutil
import subprocess
from pathlib import Path

import uv_build

ROOT = Path(__file__).resolve().parents[2]
BIN_DIR = ROOT / "src" / "spice_netlist_ls" / "bin"
RUST_BINS = ("spicefmt", "spice-netlist-ls")


def bundle_rust_binaries() -> None:
    # When building wheel from sdist (uv build's second stage), Cargo.toml
    # is not present in the sdist — binaries are already bundled in
    # src/spice_netlist_ls/bin/ at sdist creation time. Skip rebuild there
    # so `uv build` and air-gapped installs work.
    if not (ROOT / "Cargo.toml").is_file():
        # sdist build: ensure bundled bins exist, otherwise skip silently
        return
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    for name in RUST_BINS:
        src = ROOT / "target" / "release" / name
        if not src.is_file():
            raise RuntimeError(f"cargo produced no {src}")
        shutil.copy2(src, BIN_DIR / name)


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    bundle_rust_binaries()
    return uv_build.build_wheel(wheel_directory, config_settings, metadata_directory)


def build_sdist(sdist_directory, config_settings=None):
    bundle_rust_binaries()
    return uv_build.build_sdist(sdist_directory, config_settings)


def get_requires_for_build_wheel(config_settings=None):
    return []


def get_requires_for_build_sdist(config_settings=None):
    return []


def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    bundle_rust_binaries()
    return uv_build.build_editable(wheel_directory, config_settings, metadata_directory)


def get_requires_for_build_editable(config_settings=None):
    return []


# PEP 660 editable hooks (newer uv_build may use these names)
def build_editable_with_base(*args, **kwargs):
    return build_editable(*args, **kwargs)
