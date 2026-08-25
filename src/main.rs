use clap::Parser;
use spice_netlist_ls::cli::{Args, ErrorOn, LintFormat};
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
            let has_error = run_lint("<stdin>", &input, kind, None, args.format, args.error_on, args.max_warnings);
            std::process::exit(if has_error { 1 } else { 0 });
        }
        let opts = format_options_for(None, fixed, kind);
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
            if run_lint(&path.display().to_string(), &input, kind, Some(path), args.format, args.error_on, args.max_warnings) {
                exit_code = 1;
            }
            continue;
        }
        let opts = format_options_for(Some(path), fixed, kind);
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

fn run_lint(
    name: &str,
    input: &str,
    kind: DialectKind,
    path: Option<&PathBuf>,
    fmt: LintFormat,
    error_on: ErrorOn,
    max_warnings: Option<usize>,
) -> bool {
    use spice_netlist_ls::linter::{LintOptions, Severity};
    use std::collections::HashMap;

    let dialect = spice_netlist_ls::get_dialect(kind);
    let mut diags = {
        let opts = match path {
            Some(p) => LintOptions {
                external_subckts: spice_netlist_ls::linter::external_subckts(p, &dialect),
            },
            None => LintOptions::default(),
        };
        spice_netlist_ls::linter::lint_str(input, &dialect, &opts)
    };

    // Project policy from [lint] in spicefmt.toml (CLI beats nothing here —
    // this *is* the project's voice; --error-on/--max-warnings are the CI's).
    let policy = path.map(|p| spice_netlist_ls::config::lint_config_for(p)).unwrap_or_default();
    for d in &mut diags {
        if let Some(s) = policy.severity.get(d.code) {
            match s.as_str() {
                "error" => d.severity = Severity::Error,
                "warning" => d.severity = Severity::Warning,
                _ => {}
            }
        }
    }
    let active: Vec<_> = diags.iter().filter(|d| !policy.is_suppressed(d.code)).collect();

    match fmt {
        LintFormat::Human => {
            for d in &active {
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
        LintFormat::Json => print!(
            "{}",
            spice_netlist_ls::linter::diagnostics_as_json(
                name,
                &active.iter().map(|d| (*d).clone()).collect::<Vec<_>>()
            )
        ),
        LintFormat::Sarif => println!(
            "{}",
            spice_netlist_ls::linter::diagnostics_as_sarif(
                name,
                &active.iter().map(|d| (*d).clone()).collect::<Vec<_>>()
            )
        ),
        LintFormat::Summary => {
            let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
            for d in &diags {
                *counts.entry((d.severity.as_str(), d.code)).or_default() += 1;
            }
            let mut rows: Vec<(usize, &str, &str)> = counts
                .into_iter()
                .map(|((sev, code), n)| (n, sev, code))
                .collect();
            rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(b.2)));
            for (n, sev, code) in &rows {
                let mark = if policy.is_suppressed(code) { "  [suppressed]" } else { "" };
                println!("{n:>9}  {sev:<7}  {code}{mark}");
            }
            let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
            let warnings = diags.len() - errors;
            println!(
                "{:>9}  finding(s): {} error(s), {} warning(s)",
                diags.len(),
                errors,
                warnings
            );
        }
    }

    // Exit decision counts only non-suppressed findings.
    let errors = active.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = active.iter().filter(|d| d.severity == Severity::Warning).count();
    let over_max = max_warnings.is_some_and(|n| warnings > n);
    match error_on {
        ErrorOn::Warning => errors > 0 || warnings > 0 || over_max,
        ErrorOn::Error => errors > 0 || over_max,
    }
}

fn read_stdin() -> String {
    use std::io::Read;
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap_or(0);
    s
}
