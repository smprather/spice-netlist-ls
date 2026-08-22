//! CLI argument definitions shared between the `spicefmt` binary and
//! `build.rs` (which renders the man page from the same struct via
//! `clap_mangen`).

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum LintFormat {
    #[default]
    Human,
    Json,
    Sarif,
}

#[derive(Parser, Debug)]
#[command(name = "spicefmt", about = "Highly opinionated SPICE formatter — HSPICE golden, dialect-extensible")]
#[command(group(
    clap::ArgGroup::new("mode")
        .args(["check", "write", "lint", "print_dialect"])
        .multiple(false)
))]
pub struct Args {
    #[arg(value_name = "FILE", help = "Input file (stdin if omitted)")]
    pub files: Vec<std::path::PathBuf>,

    #[arg(long, help = "Check only, exit 1 if not formatted")]
    pub check: bool,

    #[arg(long, help = "Write back to file in-place")]
    pub write: bool,

    #[arg(long, value_name = "DIALECT", help = "Dialect: hspice, ngspice, spectre, ltspice, or auto (default: auto)")]
    pub dialect: Option<String>,

    #[arg(long, help = "Detect and print dialect per input, no formatting")]
    pub print_dialect: bool,

    #[arg(long, help = "Lint only: print diagnostics, exit 1 on error-severity findings")]
    pub lint: bool,

    #[arg(
        long,
        value_name = "FORMAT",
        value_enum,
        default_value_t = LintFormat::Human,
        requires = "lint",
        help = "Diagnostic output format (used with --lint)"
    )]
    pub format: LintFormat,

    #[arg(long, help = "Print dialect list and exit")]
    pub list_dialects: bool,
}
