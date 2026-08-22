//! Build-time artifacts: generate the man page from a duplicate of the
//! `spicefmt` CLI definition so `man spicefmt` is in sync with `--help`.
//! NOTE: if you add a flag, update both `src/cli.rs` and the copy below.

use clap::Parser;
use clap_mangen::Man;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
#[allow(dead_code)]
enum LintFormat {
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
struct Args {
    #[arg(value_name = "FILE", help = "Input file (stdin if omitted)")]
    files: Vec<PathBuf>,

    #[arg(long, help = "Check only, exit 1 if not formatted")]
    check: bool,

    #[arg(long, help = "Write back to file in-place")]
    write: bool,

    #[arg(long, value_name = "DIALECT", help = "Dialect: hspice, ngspice, spectre, ltspice, or auto (default: auto)")]
    dialect: Option<String>,

    #[arg(long, help = "Detect and print dialect per input, no formatting")]
    print_dialect: bool,

    #[arg(long, help = "Lint only: print diagnostics, exit 1 on error-severity findings")]
    lint: bool,

    #[arg(
        long,
        value_name = "FORMAT",
        value_enum,
        default_value_t = LintFormat::Human,
        requires = "lint",
        help = "Diagnostic output format (used with --lint)"
    )]
    format: LintFormat,

    #[arg(long, help = "Print dialect list and exit")]
    list_dialects: bool,
}

fn main() {
    let out_dir = std::env::var("OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    let man_dir = out_dir.join("man");
    std::fs::create_dir_all(&man_dir).expect("mkdir man");

    let cmd = <Args as clap::CommandFactory>::command();
    let mut buf = Vec::new();
    Man::new(cmd).render(&mut buf).expect("render man");
    std::fs::write(man_dir.join("spicefmt.1"), &buf).expect("write man page");
}
