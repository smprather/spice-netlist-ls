//! CLI argument definitions shared between the `spicefmt` binary and
//! `build.rs` (which renders the man page from the same struct via
//! `clap_mangen`).

use clap::{Parser, ValueEnum};

/// What severity should fail `--lint`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ErrorOn {
    /// Exit non-zero only when error-severity findings exist.
    #[default]
    Error,
    /// Exit non-zero on any error *or* warning finding.
    Warning,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum LintFormat {
    /// `path:line:col: severity [code]: message` — stable, grep-friendly.
    #[default]
    Human,
    /// Newline-delimited JSON: **one object per line**, not an array —
    /// stream-processable, and one malformed line costs one finding rather
    /// than the whole report.
    Json,
    /// Static Analysis Results Interchange Format; GitLab/GitHub Enterprise
    /// merge-request UIs render this natively without any plugin.
    Sarif,
    /// Counts by (severity, code), descending, plus totals. Suppressed codes
    /// still appear here so acknowledgements stay visible.
    Summary,
}

#[derive(Parser, Debug)]
#[command(
    name = "spicefmt",
    version,
    about = "Opinionated SPICE netlist formatter and linter — dialect-extensible (hspice, ngspice, spectre, ltspice)",
    after_help = "In .scs files, `simulator lang=spice`/`lang=spectre` switch the active dialect per section; --dialect sets only the fallback for the implicit pre-switch section."
)]
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

    #[arg(long, help = "Lint only: print diagnostics, exit code governed by --error-on/--max-warnings")]
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

    #[arg(
        long,
        value_name = "SEVERITY",
        value_enum,
        default_value_t = ErrorOn::Error,
        requires = "lint",
        help = "Lowest severity that fails the lint run"
    )]
    pub error_on: ErrorOn,

    #[arg(
        long,
        value_name = "N",
        requires = "lint",
        help = "Fail when more than N (non-suppressed) warnings are reported"
    )]
    pub max_warnings: Option<usize>,

    #[arg(long, help = "Print dialect list and exit")]
    pub list_dialects: bool,
}
