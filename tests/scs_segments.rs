//! Snapshot tests for `.scs` per-section dialect switching
//! (`simulator lang=spice` / `lang=spectre`).
//!
//! - `no_lang.sp` / `lang_spectre_only.scs`: unchanged-behavior guards.
//!   Their snapshots are frozen from the golden-before baseline and must
//!   not change across the segmentation work.
//! - `lang_spice_only.scs` / `lang_mixed.scs`: new-behavior fixtures. Their
//!   snapshots capture the *fixed* per-section formatting and are updated
//!   once the segmentation code lands, then frozen.

use spice_netlist_ls::{FormatOptions, detect_dialect, format_str};
use spice_netlist_ls::linter::{LintOptions, lint_str};
use std::path::Path;

fn load(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/testdata/scs/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn format(name: &str) -> String {
    let input = load(name);
    let kind = detect_dialect(&input);
    format_str(&input, &FormatOptions { dialect: kind, ..Default::default() })
}

fn lint(name: &str) -> String {
    let input = load(name);
    let kind = detect_dialect(&input);
    let dialect = spice_netlist_ls::get_dialect(kind);
    let opts = LintOptions {
        external_subckts: spice_netlist_ls::linter::external_subckts(
            Path::new(&format!(
                "{}/testdata/scs/{name}",
                env!("CARGO_MANIFEST_DIR")
            )),
            &dialect,
        ),
    };
    let diags = lint_str(&input, &dialect, &opts);
    diags
        .iter()
        .map(|d| {
            format!(
                "{}:{}: {} [{}]: {}",
                d.range.start_line + 1,
                d.range.start_col + 1,
                d.severity.as_str(),
                d.code,
                d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------- format snapshots ----------

#[test]
fn snapshot_scs_no_lang_sp() {
    insta::assert_snapshot!("no_lang.sp.fmt", format("no_lang.sp"));
}

#[test]
fn snapshot_scs_lang_spectre_only() {
    insta::assert_snapshot!("lang_spectre_only.scs.fmt", format("lang_spectre_only.scs"));
}

#[test]
fn snapshot_scs_lang_spice_only() {
    insta::assert_snapshot!("lang_spice_only.scs.fmt", format("lang_spice_only.scs"));
}

#[test]
fn snapshot_scs_lang_mixed() {
    insta::assert_snapshot!("lang_mixed.scs.fmt", format("lang_mixed.scs"));
}

// ---------- lint snapshots ----------

#[test]
fn snapshot_scs_no_lang_sp_lint() {
    insta::assert_snapshot!("no_lang.sp.lint", lint("no_lang.sp"));
}

#[test]
fn snapshot_scs_lang_spectre_only_lint() {
    insta::assert_snapshot!("lang_spectre_only.scs.lint", lint("lang_spectre_only.scs"));
}

#[test]
fn snapshot_scs_lang_spice_only_lint() {
    insta::assert_snapshot!("lang_spice_only.scs.lint", lint("lang_spice_only.scs"));
}

#[test]
fn snapshot_scs_lang_mixed_lint() {
    insta::assert_snapshot!("lang_mixed.scs.lint", lint("lang_mixed.scs"));
}

// ---------- idempotency ----------

#[test]
fn idempotent_no_lang_sp() {
    let once = format("no_lang.sp");
    let kind = detect_dialect(&once);
    let twice = format_str(&once, &FormatOptions { dialect: kind, ..Default::default() });
    assert_eq!(once, twice);
}

#[test]
fn idempotent_lang_spectre_only() {
    let once = format("lang_spectre_only.scs");
    let kind = detect_dialect(&once);
    let twice = format_str(&once, &FormatOptions { dialect: kind, ..Default::default() });
    assert_eq!(once, twice);
}

#[test]
fn idempotent_lang_spice_only() {
    let once = format("lang_spice_only.scs");
    let kind = detect_dialect(&once);
    let twice = format_str(&once, &FormatOptions { dialect: kind, ..Default::default() });
    assert_eq!(once, twice);
}

#[test]
fn idempotent_lang_mixed() {
    let once = format("lang_mixed.scs");
    let kind = detect_dialect(&once);
    let twice = format_str(&once, &FormatOptions { dialect: kind, ..Default::default() });
    assert_eq!(once, twice);
}