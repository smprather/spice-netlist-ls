//! Configuration file support.
//!
//! Hierarchy (first match wins for each key):
//!   1. CLI flags
//!   2. `spicefmt.toml` in the file's directory or any ancestor
//!   3. `$XDG_CONFIG_HOME/spicefmt/spicefmt.toml` (or platform equivalent)
//!
//! The LSP applies the same search so the editor and the CI runner agree.

use crate::dialect::{DialectKind, dialect_from_str};
use crate::formatter::FormatOptions;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpicefmtConfig {
    /// Dialect override; "auto" means detect per file.
    pub dialect: Option<String>,
    /// Wrap width for the formatter.
    pub max_width: Option<usize>,
    /// Sort `key = value` params after positional args.
    pub sort_params: Option<bool>,
}

impl SpicefmtConfig {
    pub fn apply_to(&self, opts: &mut FormatOptions) {
        if let Some(w) = self.max_width {
            opts.max_width = w;
        }
        if let Some(s) = self.sort_params {
            opts.sort_params = s;
        }
        if let Some(d) = &self.dialect
            && let Some(k) = dialect_from_str(d)
        {
            opts.dialect = k;
        }
    }
}

/// Locate the nearest `spicefmt.toml` walking up from `start` (a file or a
/// directory), then fall back to the per-user config.
pub fn config_path(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        let candidate = dir.join("spicefmt.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    directories::ProjectDirs::from("", "", "spicefmt")
        .map(|p| p.config_dir().join("spicefmt.toml"))
        .filter(|p| p.is_file())
}

pub fn load_config(start: &Path) -> Option<SpicefmtConfig> {
    let path = config_path(start)?;
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

/// CLI flags beat config; config beats auto-detection.
pub fn format_options_for(
    file_path: Option<&Path>,
    cli_dialect: Option<DialectKind>,
    detected: DialectKind,
) -> FormatOptions {
    let mut opts = FormatOptions {
        dialect: detected,
        ..FormatOptions::default()
    };
    if let Some(p) = file_path
        && let Some(cfg) = load_config(p)
    {
        cfg.apply_to(&mut opts);
    }
    if let Some(d) = cli_dialect {
        opts.dialect = d;
    }
    opts
}
