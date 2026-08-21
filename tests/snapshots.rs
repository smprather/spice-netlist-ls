//! Snapshot tests over the shipped testdata files using `insta`. Run with
//! `cargo insta review` (or set `INSTA_UPDATE=always`) to accept updates.

use spice_netlist_ls::{FormatOptions, detect_dialect, format_str};

fn load(name: &str) -> String {
    std::fs::read_to_string(format!("{}/testdata/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn snapshot_rc_chain() {
    let input = load("simple_rc_chain.subckt");
    let kind = detect_dialect(&input);
    insta::assert_snapshot!(format_str(&input, &FormatOptions { dialect: kind, ..Default::default() }));
}

#[test]
fn snapshot_ngspice_deck() {
    let input = load("ngspice_deck.sp");
    let kind = detect_dialect(&input);
    insta::assert_snapshot!(format_str(&input, &FormatOptions { dialect: kind, ..Default::default() }));
}

#[test]
fn snapshot_ltspice_deck() {
    let input = load("ltspice_deck.cir");
    let kind = detect_dialect(&input);
    insta::assert_snapshot!(format_str(&input, &FormatOptions { dialect: kind, ..Default::default() }));
}

#[test]
fn snapshot_spectre_deck() {
    let input = load("spectre_deck.scs");
    let kind = detect_dialect(&input);
    insta::assert_snapshot!(format_str(&input, &FormatOptions { dialect: kind, ..Default::default() }));
}
