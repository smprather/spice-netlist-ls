//! Configuration file support.
//!
//! Hierarchy (first match wins for each key):
//!   1. CLI flags
//!   2. `spicefmt.toml` in the file's directory or any ancestor
//!   3. `.editorconfig` files between the file and its `root = true` marker
//!      or the filesystem root ([editorconfig.org](https://editorconfig.org))
//!   4. `$XDG_CONFIG_HOME/spicefmt/spicefmt.toml` (or platform equivalent)
//!
//! The LSP applies the same search so the editor and the CI runner agree.

use crate::dialect::{DialectKind, dialect_from_str};
use crate::formatter::FormatOptions;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Project lint policy from `spicefmt.toml`'s `[lint]` table.
///
/// Suppressed codes vanish from the detail listing but **still appear in
/// summary counts** — a suppression that makes a finding disappear entirely
/// is how a real problem gets lost when circumstances change.
///
/// Ruff-inspired: `ignore` is the preferred name (`suppress` kept for
/// backwards compat). `select` allowlists which codes are enabled; if `select`
/// is empty all codes are enabled minus `ignore`/`suppress`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LintConfig {
    /// Diagnostic codes to hide from detail output (`duplicate-instance`,
    /// `floating-node`, ...). Counted in `--format summary`, marked as
    /// suppressed, and excluded from `--max-warnings`/`--error-on` math.
    pub suppress: Vec<String>,
    /// Ruff-style alias for `suppress` – `ignore = ["code"]`.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Allowlist – if non-empty, only these codes are enabled.
    #[serde(default)]
    pub select: Vec<String>,
    /// Per-code severity override: `"code" = "error" | "warning"`.
    pub severity: std::collections::HashMap<String, String>,
}

impl LintConfig {
    pub fn is_suppressed(&self, code: &str) -> bool {
        self.suppress.iter().any(|c| c == code) || self.ignore.iter().any(|c| c == code)
    }

    pub fn is_enabled(&self, code: &str) -> bool {
        if !self.select.is_empty() && !self.select.iter().any(|c| c == code) {
            return false;
        }
        !self.is_suppressed(code)
    }
}

/// Formatter policy from `spicefmt.toml`'s `[format]` table.
///
/// Ruff north-star: `select`/`ignore` mirror `ruff`'s `lint.select`/`lint.ignore`.
/// All format rules are enabled by default; `ignore` disables, `select`
/// allowlists. Available rules:
/// - `blank-after-subckt`  – no empty line after `.subckt` (default enabled)
/// - `blank-before-ends`   – no empty line before `.ends`
/// - `blank-after-ends`    – at least one empty line after `.ends`
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FormatConfig {
    /// Disable specific format rules (e.g. `ignore = ["blank-after-subckt"]`).
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Allowlist – if non-empty, only these rules are enabled.
    #[serde(default)]
    pub select: Vec<String>,
}

impl FormatConfig {
    pub fn is_ignored(&self, code: &str) -> bool {
        self.ignore.iter().any(|c| c.eq_ignore_ascii_case(code))
    }

    pub fn is_enabled(&self, code: &str) -> bool {
        if !self.select.is_empty() && !self.select.iter().any(|c| c.eq_ignore_ascii_case(code)) {
            return false;
        }
        !self.is_ignored(code)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpicefmtConfig {
    /// Dialect override; "auto" means detect per file.
    pub dialect: Option<String>,
    /// Wrap width for the formatter.
    pub max_width: Option<usize>,
    /// Sort `key = value` params after positional args.
    pub sort_params: Option<bool>,
    /// Ensure output ends with a newline.
    pub insert_final_newline: Option<bool>,
    /// Strip trailing whitespace from every line.
    pub trim_trailing_whitespace: Option<bool>,
    /// Project lint policy.
    pub lint: LintConfig,
    /// Formatter policy (ruff-inspired `select`/`ignore`).
    #[serde(default)]
    pub format: FormatConfig,
}

impl SpicefmtConfig {
    pub fn apply_to(&self, opts: &mut FormatOptions) {
        if let Some(w) = self.max_width {
            opts.max_width = w;
        }
        if let Some(s) = self.sort_params {
            opts.sort_params = s;
        }
        if let Some(b) = self.insert_final_newline {
            opts.insert_final_newline = b;
        }
        if let Some(b) = self.trim_trailing_whitespace {
            opts.trim_trailing_whitespace = b;
        }
        if let Some(d) = &self.dialect
            && let Some(k) = dialect_from_str(d)
        {
            opts.dialect = k;
        }
        // Ruff-inspired format rule selection – config file wins over defaults,
        // CLI flags (applied later) win over config.
        if !self.format.ignore.is_empty() {
            opts.ignore = self.format.ignore.clone();
        }
        if !self.format.select.is_empty() {
            opts.select = self.format.select.clone();
        }
    }
}

// ---------- EditorConfig ----------

/// The subset of [EditorConfig](https://editorconfig.org) properties this
/// formatter understands. Unknown properties are ignored, as the spec allows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EditorConfig {
    /// `max_line_length` — becomes the formatter wrap width when
    /// `spicefmt.toml` does not set `max_width`.
    pub max_line_length: Option<usize>,
    pub insert_final_newline: Option<bool>,
    pub trim_trailing_whitespace: Option<bool>,
}

impl EditorConfig {
    fn set(&mut self, key: &str, value: &str) -> bool {
        match key {
            "max_line_length" => value.trim().parse().ok().map(|n| self.max_line_length = Some(n)).is_some(),
            "insert_final_newline" => parse_bool(value).inspect(|b| self.insert_final_newline = Some(*b)).is_some(),
            "trim_trailing_whitespace" => parse_bool(value).inspect(|b| self.trim_trailing_whitespace = Some(*b)).is_some(),
            _ => false,
        }
    }

    pub fn apply_to(&self, opts: &mut FormatOptions) {
        if let Some(w) = self.max_line_length {
            opts.max_width = w;
        }
        if let Some(b) = self.insert_final_newline {
            opts.insert_final_newline = b;
        }
        if let Some(b) = self.trim_trailing_whitespace {
            opts.trim_trailing_whitespace = b;
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parse one `.editorconfig` file; return its properties for `file_name`
/// plus whether it declares `root = true`. Later matching sections win,
/// per spec. Globs follow the EditorConfig subset handled by `globset`
/// (`*`, `**`, `?`, `{a,b}`, `[..]`).
pub(crate) fn parse_editorconfig(text: &str, rel_path: &str) -> (EditorConfig, bool) {
    let mut cfg = EditorConfig::default();
    let mut is_root = false;
    // (glob, key, value) rules from all sections, in file order.
    let mut rules: Vec<(globset::Glob, String, String)> = Vec::new();
    let mut current: Option<globset::Glob> = None;
    let base = rel_path.rsplit('/').next().unwrap_or(rel_path);

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(pat) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = globset::GlobBuilder::new(pat)
                .literal_separator(true)
                .build()
                .ok();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            match (&current, k.to_ascii_lowercase().as_str()) {
                (None, "root") => {
                    if parse_bool(v) == Some(true) {
                        is_root = true;
                    }
                }
                (Some(glob), _) => {
                    rules.push((glob.clone(), k.to_ascii_lowercase(), v.to_string()));
                }
                (None, _) => {}
            }
        }
    }

    // Last matching rule wins per property.
    for (glob, key, value) in rules {
        let matcher = glob.compile_matcher();
        let matches = if glob.glob().contains('/') {
            matcher.is_match(rel_path)
        } else {
            matcher.is_match(base)
        };
        if matches {
            cfg.set(&key, &value);
        }
    }

    (cfg, is_root)
}

/// Apply every `.editorconfig` between `start`'s directory and either a
/// `root = true` marker or the filesystem root. Closer files override
/// farther ones, so files are applied farthest-first.
pub fn load_editorconfig(start: &Path) -> Option<EditorConfig> {
    let mut dir = start.parent()?.to_path_buf();
    let file_name = start.file_name()?.to_string_lossy().into_owned();

    let mut chain: Vec<(PathBuf, EditorConfig)> = Vec::new();
    loop {
        let candidate = dir.join(".editorconfig");
        if candidate.is_file()
            && let Ok(text) = std::fs::read_to_string(&candidate)
        {
            let rel = start
                .strip_prefix(&dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| file_name.clone());
            let (cfg, root) = parse_editorconfig(&text, &rel);
            chain.push((candidate, cfg));
            if root {
                break;
            }
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }

    let mut result = EditorConfig::default();
    for (_, cfg) in chain.iter().rev() {
        if cfg.max_line_length.is_some() {
            result.max_line_length = cfg.max_line_length;
        }
        if cfg.insert_final_newline.is_some() {
            result.insert_final_newline = cfg.insert_final_newline;
        }
        if cfg.trim_trailing_whitespace.is_some() {
            result.trim_trailing_whitespace = cfg.trim_trailing_whitespace;
        }
    }
    Some(result)
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

/// The `[lint]` policy table applying to `path` (default if no config).
pub fn lint_config_for(path: &Path) -> LintConfig {
    load_config(path).map(|c| c.lint).unwrap_or_default()
}

/// The `[format]` policy table applying to `path` (default if no config).
pub fn format_config_for(path: &Path) -> FormatConfig {
    load_config(path).map(|c| c.format).unwrap_or_default()
}

/// CLI flags beat `spicefmt.toml`; `spicefmt.toml` beats `.editorconfig`;
/// `.editorconfig` beats defaults. Auto-detected dialect fills the gap left
/// by all of the above.
pub fn format_options_for(
    file_path: Option<&Path>,
    cli_dialect: Option<DialectKind>,
    detected: DialectKind,
) -> FormatOptions {
    let mut opts = FormatOptions {
        dialect: detected,
        ..FormatOptions::default()
    };
    if let Some(p) = file_path {
        if let Some(ec) = load_editorconfig(p) {
            ec.apply_to(&mut opts);
        }
        if let Some(cfg) = load_config(p) {
            cfg.apply_to(&mut opts);
        }
    }
    if let Some(d) = cli_dialect {
        opts.dialect = d;
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_basename_for_bare_patterns() {
        assert!(editorconfig_match("[*.sp]", "a.sp"));
        assert!(editorconfig_match("[*.{sp,cir}]", "deck.cir"));
        assert!(!editorconfig_match("[*.sp]", "a.cir"));
    }

    #[test]
    fn glob_matches_path_only_when_pattern_has_separator() {
        assert!(!editorconfig_match("[sim/*.sp]", "a.sp"));
        assert!(editorconfig_match("[**/*.scs]", "sim/top.scs"));
    }

    fn editorconfig_match(section: &str, rel_path: &str) -> bool {
        let text = format!("{section}\nmax_line_length = 40\n");
        let (cfg, _) = parse_editorconfig(&text, rel_path);
        cfg.max_line_length == Some(40)
    }

    #[test]
    fn later_sections_win() {
        let text = "[*.sp]\ninsert_final_newline = false\n[*]\ninsert_final_newline = true\n";
        let (cfg, _) = parse_editorconfig(text, "a.sp");
        assert_eq!(cfg.insert_final_newline, Some(true));
    }

    #[test]
    fn root_marker_is_detected() {
        let text = "root = true\n[*.sp]\ntrim_trailing_whitespace = false\n";
        let (cfg, root) = parse_editorconfig(text, "a.sp");
        assert!(root);
        assert_eq!(cfg.trim_trailing_whitespace, Some(false));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = "[*]\nindent_style = space\nend_of_line = lf\n";
        let (cfg, _) = parse_editorconfig(text, "a.sp");
        assert_eq!(cfg, EditorConfig::default());
    }

    #[test]
    fn format_ignore_parses() {
        let text = "[format]\nignore = [\"blank-after-subckt\", \"blank-before-ends\"]\n";
        let cfg: SpicefmtConfig = toml::from_str(text).unwrap();
        assert!(cfg.format.is_ignored("blank-after-subckt"));
        assert!(cfg.format.is_ignored("blank-before-ends"));
        assert!(!cfg.format.is_ignored("blank-after-ends"));
        assert!(cfg.format.is_enabled("blank-after-ends"));
        assert!(!cfg.format.is_enabled("blank-after-subckt"));
    }

    #[test]
    fn format_select_allowlist() {
        let text = "[format]\nselect = [\"blank-after-ends\"]\n";
        let cfg: SpicefmtConfig = toml::from_str(text).unwrap();
        assert!(cfg.format.is_enabled("blank-after-ends"));
        assert!(!cfg.format.is_enabled("blank-after-subckt"));
        assert!(!cfg.format.is_enabled("blank-before-ends"));
    }

    #[test]
    fn lint_ignore_alias_for_suppress() {
        let text = "[lint]\nignore = [\"blank-after-subckt\"]\n";
        let cfg: SpicefmtConfig = toml::from_str(text).unwrap();
        assert!(cfg.lint.is_suppressed("blank-after-subckt"));
        assert!(cfg.lint.is_enabled("blank-before-ends"));
        let text2 = "[lint]\nsuppress = [\"floating-node\"]\n";
        let cfg2: SpicefmtConfig = toml::from_str(text2).unwrap();
        assert!(cfg2.lint.is_suppressed("floating-node"));
    }

    #[test]
    fn format_options_respect_config() {
        let text = "[format]\nignore = [\"blank-after-ends\"]\n";
        let cfg: SpicefmtConfig = toml::from_str(text).unwrap();
        let mut opts = crate::formatter::FormatOptions::default();
        cfg.apply_to(&mut opts);
        assert!(!opts.is_enabled("blank-after-ends"));
        assert!(opts.is_enabled("blank-after-subckt"));
    }
}
