use spice_netlist_ls::config::format_options_for;
use spice_netlist_ls::detect::detect_dialect;
use spice_netlist_ls::dialect::get_dialect;
use spice_netlist_ls::linter::{external_subckts, lint_str, LintOptions};
use spice_netlist_ls::parser::{include_paths, logical_line_spans, parse_str};
use std::path::Path;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "testdata/auto_clock_0.sp".into());
    let input = std::fs::read_to_string(&path).unwrap();

    let t = Instant::now();
    let kind = detect_dialect(&input);
    println!("detect_dialect      {:8.2?}  ({kind:?})", t.elapsed());

    let dialect = get_dialect(kind);
    let t = Instant::now();
    let spans = logical_line_spans(&input, dialect.as_ref());
    println!("logical_line_spans  {:8.2?}  ({} spans)", t.elapsed(), spans.len());

    let t = Instant::now();
    let file = parse_str(&input, dialect.clone());
    println!("parse_str           {:8.2?}  ({} stmts)", t.elapsed(), file.stmts.len());

    let mut opts = format_options_for(Some(Path::new(&path)), None, kind);
    opts.sort_params = false;
    let t = Instant::now();
    let out = spice_netlist_ls::format_str(&input, &opts);
    println!("format_str(total)   {:8.2?}  ({} bytes)", t.elapsed(), out.len());

    let t = Instant::now();
    let inc = include_paths(&input, dialect.as_ref());
    println!("include_paths       {:8.2?}  ({} includes)", t.elapsed(), inc.len());

    let t = Instant::now();
    let ext = external_subckts(Path::new(&path), &dialect);
    println!("external_subckts    {:8.2?}  ({} defs)", t.elapsed(), ext.len());

    let topts = LintOptions { external_subckts: ext };
    let t = Instant::now();
    let diags = lint_str(&input, &dialect, &topts);
    println!("lint_str            {:8.2?}  ({} diags)", t.elapsed(), diags.len());
}
