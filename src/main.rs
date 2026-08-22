use clap::Parser;
use spice_netlist_ls::cli::{Args, LintFormat};
use spice_netlist_ls::config::format_options_for;
use spice_netlist_ls::detect::detect_dialect;
use spice_netlist_ls::dialect::{DialectKind, dialect_from_str};
use std::fs;
use std::path::PathBuf;

fn main() {
    let args = Args::parse();

    if args.list_dialects {
        for k in DialectKind::all() {
            println!("{}", k.as_str());
        }
        println!("auto");
        return;
    }

    let fixed = match args.dialect.as_deref() {
        None | Some("auto") => None,
        Some(s) => match dialect_from_str(s) {
            Some(k) => Some(k),
            None => {
                eprintln!("unknown dialect '{s}' (hspice, ngspice, spectre, ltspice, auto)");
                std::process::exit(2);
            }
        },
    };

    if args.files.is_empty() {
        let input = read_stdin();
        let kind = fixed.unwrap_or_else(|| detect_dialect(&input));
        if args.print_dialect {
            println!("stdin: {}", kind.as_str());
            return;
        }
        if args.lint {
            let has_error = run_lint("<stdin>", &input, kind, None, args.format);
            std::process::exit(if has_error { 1 } else { 0 });
        }
        let mut opts = format_options_for(None, fixed, kind);
        let output = spice_netlist_ls::format_str(&input, &opts);
        if args.check {
            if input != output {
                eprintln!("would format stdin");
                std::process::exit(1);
            }
        } else {
            print!("{output}");
        }
        return;
    }

    let mut exit_code = 0;
    for path in &args.files {
        let input = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                exit_code = 1;
                continue;
            }
        };
        let kind = fixed.unwrap_or_else(|| detect_dialect(&input));
        if args.print_dialect {
            println!("{}: {}", path.display(), kind.as_str());
            continue;
        }
        if args.lint {
            if run_lint(&path.display().to_string(), &input, kind, Some(path), args.format) {
                exit_code = 1;
            }
            continue;
        }
        let mut opts = format_options_for(Some(path), fixed, kind);
        let output = spice_netlist_ls::format_str(&input, &opts);
        if args.check {
            if input != output {
                eprintln!("{}: would be formatted", path.display());
                exit_code = 1;
            }
        } else if args.write {
            if input != output {
                if let Err(e) = fs::write(path, &output) {
                    eprintln!("{}: {e}", path.display());
                    exit_code = 1;
                }
            }
        } else {
            print!("{output}");
        }
    }
    std::process::exit(exit_code);
}

fn run_lint(name: &str, input: &str, kind: DialectKind, path: Option<&PathBuf>, fmt: LintFormat) -> bool {
    let dialect = spice_netlist_ls::get_dialect(kind);
    let opts = match path {
        Some(p) => spice_netlist_ls::linter::LintOptions {
            external_subckts: spice_netlist_ls::linter::external_subckts(p, &dialect),
        },
        None => spice_netlist_ls::linter::LintOptions::default(),
    };
    let diags = spice_netlist_ls::linter::lint_str(input, &dialect, &opts);
    let has_error = diags
        .iter()
        .any(|d| d.severity == spice_netlist_ls::linter::Severity::Error);
    match fmt {
        LintFormat::Human => {
            for d in &diags {
                println!(
                    "{}:{}:{}: {} [{}]: {}",
                    name,
                    d.range.start_line + 1,
                    d.range.start_col + 1,
                    d.severity.as_str(),
                    d.code,
                    d.message
                );
            }
        }
        LintFormat::Json => print!("{}", spice_netlist_ls::linter::diagnostics_as_json(name, &diags)),
        LintFormat::Sarif => println!("{}", spice_netlist_ls::linter::diagnostics_as_sarif(name, &diags)),
    }
    has_error
}

fn read_stdin() -> String {
    use std::io::Read;
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap_or(0);
    s
}
