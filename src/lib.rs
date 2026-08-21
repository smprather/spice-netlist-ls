pub mod detect;
pub mod dialect;
pub mod formatter;
pub mod ir;
pub mod parser;

pub use detect::{DialectScores, detect_dialect, score_dialect};
pub use dialect::{Dialect, DialectKind, get_dialect};
pub use formatter::{FormatOptions, format_file, format_str};
pub use ir::{File, Stmt};
pub use parser::parse_str;
