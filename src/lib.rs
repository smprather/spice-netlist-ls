pub mod cli;
pub mod config;
pub mod detect;
pub mod dialect;
pub mod formatter;
mod fx;
pub mod ir;
pub mod linter;
pub mod parser;

pub use detect::{DialectScores, detect_dialect, score_dialect};
pub use dialect::{Dialect, DialectKind, get_dialect};
pub use formatter::{FormatOptions, format_file, format_str};
pub use ir::{File, Stmt};
pub use parser::parse_str;

/// Case-insensitive ASCII prefix check without allocating an uppercased copy.
pub(crate) fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}
