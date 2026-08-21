use clap::Parser;
use spice_netlist_ls::detect::detect_dialect;
use spice_netlist_ls::dialect::{dialect_from_str, DialectKind};
use spice_netlist_ls::formatter::FormatOptions;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "spicefmt", about = "Highly opinionated SPICE formatter — HSPICE golden, dialect-extensible")]
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

    #[arg(long, help = "Print dialect list and exit")]
    list_dialects: bool,
}

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
        let opts = FormatOptions {
            dialect: kind,
            ..Default::default()
        };
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
        let opts = FormatOptions {
            dialect: kind,
            ..Default::default()
        };
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

fn read_stdin() -> String {
    use std::io::Read;
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap_or(0);
    s
}
