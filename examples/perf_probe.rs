use spice_netlist_ls::config::format_options_for;
use spice_netlist_ls::detect::detect_dialect;
use spice_netlist_ls::dialect::get_dialect;
use spice_netlist_ls::linter::{external_subckts, lint_str, LintOptions};
use spice_netlist_ls::parser::{include_paths, logical_line_spans, parse_str};
use std::io::Read;
use std::path::Path;
use std::time::Instant;

/// Read a netlist from `path`, transparently decompressing bzip2 when the path
/// ends in `.bz2`. The committed perf deck is `examples/auto_clock_0.sp.bz2`
/// (~435 KB compressed, ~6 MB / 59 K lines expanded), so `cargo run --example
/// perf_probe` works with no arguments.
fn read_netlist(path: &str) -> String {
    if path.ends_with(".bz2") {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut s = String::new();
        bzip2::read::BzDecoder::new(std::io::Cursor::new(bytes))
            .read_to_string(&mut s)
            .unwrap_or_else(|e| panic!("{path}: bzip2 decode failed: {e}"));
        s
    } else {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/auto_clock_0.sp.bz2".into());
    let input = read_netlist(&path);

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
    // external_subckts walks .include/.lib files relative to the deck; for the
    // compressed deck those PDK paths won't resolve, so pass the directory the
    // deck came from (expanded form) when available.
    let ext = external_subckts(Path::new(&path), &dialect);
    println!("external_subckts    {:8.2?}  ({} defs)", t.elapsed(), ext.len());

    let topts = LintOptions { external_subckts: ext };
    let t = Instant::now();
    let diags = lint_str(&input, &dialect, &topts);
    println!("lint_str            {:8.2?}  ({} diags)", t.elapsed(), diags.len());
}