//! End-to-end tests driving the `spicefmt` binary. Unit tests inline in
//! `src/` cover the parser/formatter/linter building blocks; these exercise
//! the CLI surface (stdin/stdout, exit codes, --check/--write/--lint/--print-dialect)
//! and the shipped `testdata/` fixtures.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn spicefmt() -> PathBuf {
    // cargo sets this for integration tests against the workspace binaries.
    PathBuf::from(env!("CARGO_BIN_EXE_spicefmt"))
}

fn run(args: &[&str], stdin: Option<&str>) -> (String, String, i32) {
    let mut cmd = Command::new(spicefmt());
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().expect("spawn spicefmt");
    if let Some(text) = stdin {
        child.stdin.as_mut().unwrap().write_all(text.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap_or(-1),
    )
}

// ---------- formatting ----------

#[test]
fn stdin_lowercases_and_spreads_eq() {
    let (out, _, code) = run(&[], Some(".PARAM w=1u\n"));
    assert_eq!(code, 0);
    assert_eq!(out, ".param w = 1u\n");
}

#[test]
fn stdin_check_clean_input_exits_zero() {
    let (_, _, code) = run(&["--check"], Some(".param w = 1u\n"));
    assert_eq!(code, 0);
}

#[test]
fn stdin_check_dirty_input_exits_one_and_prints_nothing() {
    let (out, err, code) = run(&["--check"], Some(".PARAM w=1u\n"));
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("would format stdin"));
}

#[test]
fn check_and_write_are_mutually_exclusive() {
    let (_, _, code) = run(&["--check", "--write", "/dev/null"], None);
    assert_eq!(code, 2);
}

// ---------- dialect detection ----------

#[test]
fn detects_ngspice_via_control_block() {
    let (out, _, _) = run(&["--print-dialect"], Some("* t\n.control\nrun\n.endc\n"));
    assert!(out.contains("ngspice"), "got {out}");
}

#[test]
fn ngspice_node_with_semicolon_is_not_split() {
    let (out, _, _) = run(&["--dialect", "ngspice"], Some("R1 net;1 0 1k\n"));
    assert_eq!(out, "R1 net;1 0 1k\n");
    // still a comment when the ';' has whitespace on both sides
    let (out, _, _) = run(&["--dialect", "ngspice"], Some("R1 a b 1k ; series\n"));
    assert_eq!(out, "R1 a b 1k ; series\n");
}

// ---------- regression: dangling "=" tokens ----------

#[test]
fn value_after_standalone_eq_is_kept() {
    let (out, _, _) = run(&[], Some("R1 net1 net2 = 10k\n"));
    assert_eq!(out, "R1 net1 net2 10k\n");
}

#[test]
fn trailing_dangling_eq_does_not_drop_the_key() {
    let (out, _, _) = run(&[], Some("R1 a b l =\n"));
    assert_eq!(out, "R1 a b l\n");
}

// ---------- regression: .ends name mismatch ----------

#[test]
fn ends_name_default_rewrites_mismatch() {
    let (out, _, _) = run(&[], Some(".subckt inv a\n.ends wrong\nX1 p inv\n"));
    assert_eq!(out, ".subckt inv a\n.ends inv\n\nX1 p inv\n");
    // idempotent
    let (out2, _, _) = run(&[], Some(&out));
    assert_eq!(out, out2);
}

#[test]
fn ends_name_ignore_preserves_closing_name() {
    let (out, _, _) = run(
        &["--ignore", "ends-name"],
        Some(".subckt inv a\n.ends wrong\nX1 p inv\n"),
    );
    assert_eq!(out, ".subckt inv a\n.ends wrong\n\nX1 p inv\n");
}

#[test]
fn ends_name_mismatch_is_linted() {
    let (out, _, _) = run(&["--lint"], Some(".subckt inv a\n.ends wrong\nX1 p inv\n"));
    assert!(out.contains("ends-name-mismatch"), "got {out}");
}

// ---------- continuation/inline-comment wrapping ----------

#[test]
fn long_comment_is_not_split_across_continuation_lines() {
    let input = "M1 n1 n2 n3 n4 pch ad=1p as=1p pd=1u ps=1u nrd=1 nrs=1 w=10u l=0.13u m=4 region=sat delvto=0.1 mulu0=1.5 $ long long long long long long long long long long comment words here\n";
    let (out, _, _) = run(&[], Some(input));
    // comment moved to its own continuation line, still a comment there
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[1].starts_with("+ $"), "got {}", lines[1]);
    // and both lines stay comments on reformat (idempotent)
    let (out2, _, _) = run(&[], Some(&out));
    assert_eq!(out, out2);
}

#[test]
fn orphan_continuation_is_formatted_verbatim() {
    // Commenting out a parent must not glue a continuation onto an
    // unrelated element above.
    let input = "Rload out gnd 10k\n* Xinv in out inv\n+ w=2u\n";
    let (out, _, _) = run(&[], Some(input));
    assert!(out.contains("Rload out gnd 10k\n"));
    assert!(!out.contains("Rload out gnd 10k w"));
    assert!(out.contains("+ w = 2u"));
}

// ---------- lint ----------

#[test]
fn lint_reports_undefined_subckt_and_exits_nonzero() {
    let (out, _, code) = run(&["--lint"], Some("X1 a b missing\n"));
    assert_eq!(code, 1);
    assert!(out.contains("undefined-subckt"), "got {out}");
}

#[test]
fn lint_clean_netlist_exits_zero() {
    let input = ".subckt buf o i\n.ends\nX1 p q buf\n";
    let (_, _, code) = run(&["--lint"], Some(input));
    assert_eq!(code, 0);
}

#[test]
fn lint_flags_node_case_collision() {
    let (out, _, _) = run(&["--lint"], Some("R1 Net b 1k\nR2 net c 1k\n"));
    assert!(out.contains("node-case-collision"), "got {out}");
}

// ---------- testdata fixtures round-trip ------------------------------------

#[test]
fn testdata_files_format_to_self() {
    for name in [
        "simple_rc_chain.subckt",
        "ngspice_deck.sp",
        "ltspice_deck.cir",
        "spectre_deck.scs",
    ] {
        let path = format!("{}/testdata/{name}", env!("CARGO_MANIFEST_DIR"));
        let input = std::fs::read_to_string(&path).unwrap();
        let (out, _, _) = run(&[path.as_str()], None);
        assert_eq!(out, input, "{name} should already be formatted");
        // and stays stable
        let (out2, _, _) = run(&[], Some(&out));
        assert_eq!(out, out2, "{name} formatting should be idempotent");
    }
}

// ---------- lint output formats ----------

#[test]
fn lint_json_emits_one_object_per_diagnostic() {
    let (out, _, code) = run(&["--lint", "--format", "json"], Some("X1 a b missing\n"));
    assert_eq!(code, 1);
    let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    assert!(!lines.is_empty());
    for l in lines {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        assert_eq!(v["path"], "<stdin>");
        assert!(v["code"].is_string());
        assert!(v["message"].is_string());
    }
}

#[test]
fn lint_sarif_emits_valid_schema() {
    let (out, _, code) = run(&["--lint", "--format", "sarif"], Some("X1 a b missing\n"));
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["version"], "2.1.0");
    assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "spicefmt");
    assert!(!v["runs"][0]["results"].as_array().unwrap().is_empty());
}

#[test]
fn format_flag_requires_lint() {
    let (_, _, code) = run(&["--format", "json"], Some("x\n"));
    assert_eq!(code, 2);
}

// ---------- config file ----------

#[test]
fn spicefmt_toml_sets_max_width() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("spicefmt.toml"), "max_width = 40\n").unwrap();
    let f = dir.path().join("a.sp");
    // Long line, fits at 120 but not at 40
    std::fs::write(&f, "M1 n1 n2 n3 n4 pch w=1u l=1u ad=1p as=1p pd=1u ps=1u nrd=1 nrs=1\n").unwrap();
    let p = f.to_string_lossy().to_string();
    let (out, _, _) = run(&[p.as_str()], None);
    assert!(out.lines().all(|l| l.len() <= 40), "got {out}");
    assert!(out.contains("+"), "long line should wrap: {out}");
}

#[test]
fn spicefmt_toml_dialect_override_changes_format_style() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("spicefmt.toml"), "dialect = \"spectre\"\n").unwrap();
    let f = dir.path().join("a.sp");
    std::fs::write(&f, "R1 a b resistor r=1k\n").unwrap();
    // spectre emits key=value (no spaces)
    let p = f.to_string_lossy().to_string();
    let (out, _, _) = run(&[p.as_str()], None);
    assert!(out.contains("r=1k"), "got {out}");
    assert!(!out.contains("r = 1k"), "got {out}");
}

// ---------- editorconfig ----------

#[test]
fn editorconfig_max_line_length_wraps() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "root = true\n[*.sp]\nmax_line_length = 40\n",
    )
    .unwrap();
    let f = dir.path().join("a.sp");
    std::fs::write(&f, "M1 n1 n2 n3 n4 pch w=1u l=1u ad=1p as=1p pd=1u ps=1u nrd=1 nrs=1\n").unwrap();
    let p = f.to_string_lossy().to_string();
    let (out, _, _) = run(&[p.as_str()], None);
    assert!(out.lines().all(|l| l.len() <= 40), "got {out}");
    assert!(out.contains('+'), "should wrap: {out}");
}

#[test]
fn editorconfig_insert_final_newline_false_drops_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "[*.sp]\ninsert_final_newline = false\n",
    )
    .unwrap();
    let f = dir.path().join("a.sp");
    std::fs::write(&f, ".param w = 1u\n").unwrap();
    let p = f.to_string_lossy().to_string();
    let (out, _, _) = run(&[p.as_str()], None);
    assert_eq!(out, ".param w = 1u");
}

#[test]
fn spicefmt_toml_beats_editorconfig() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("spicefmt.toml"), "max_width = 80\n").unwrap();
    std::fs::write(dir.path().join(".editorconfig"), "[*]\nmax_line_length = 40\n").unwrap();
    // 60 chars: fits at 80, wraps at 40
    let line = format!("R1 {} {} 1k\n", "n".repeat(20), "m".repeat(20));
    let f = dir.path().join("a.sp");
    std::fs::write(&f, &line).unwrap();
    let p = f.to_string_lossy().to_string();
    let (out, _, _) = run(&[p.as_str()], None);
    assert_eq!(out, line, "60-char line must survive at max_width 80");
}

// ---------- enhancement requests from pg-grid-netlist-gen ----------

const CONTROL_DECK: &str = "* control block repro\n\
.param vddr=1.2 vssr=0\n\
.control\nrun\n\
let PG_VDD = vddr\n\
let PG_VSS = vssr\n\
let PG_LVL_50PCT = PG_VSS + (PG_VDD - PG_VSS)*0.5\n\
let pg_t = $&pg_insertion\n\
meas tran T1 WHEN v(CLK_LEAF)=$&PG_LVL_50PCT RISE=5\n\
meas tran T2 WHEN v(CLK_LEAF)=$&PG_LVL_50PCT RISE=6\n\
meas tran NOBS AVG v(nobs)\n\
.endc\n\
B1 nobs 0 V={vddr}\n\
M1 dangling g 0 0 nch w=1u\n\
.end\n";

#[test]
fn control_block_interior_is_not_parsed_as_netlist_cards() {
    let (out, _, code) = run(&["--lint", "--dialect", "ngspice"], Some(CONTROL_DECK));
    assert_eq!(code, 0);
    assert!(!out.contains("duplicate-instance"), "got {out}");
    assert!(!out.contains("PG_VDD"), "control vectors must not float: {out}");
    assert!(!out.contains("nobs"), "measurement-consumed node must not float: {out}");
    // genuine device-pin findings still fire
    assert!(out.contains("'dangling'"), "got {out}");
}

#[test]
fn measure_referenced_node_outside_control_is_observed() {
    let input = ".tran 1n 10n\n.meas tran avg_v AVG v(vmon)\nB1 vmon 0 V=1\n";
    let (out, _, code) = run(&["--lint"], Some(input));
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("floating"), "{out}");
}

#[test]
fn summary_format_counts_by_severity_and_code() {
    let input = "R1 a b 1k\nR1 c d 2k\nX1 p q missing\n";
    let (out, _, code) = run(&["--lint", "--format", "summary"], Some(input));
    assert_eq!(code, 1);
    let dup: usize = out.lines().find(|l| l.contains("duplicate-instance")).unwrap()
        .split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(dup, 1);
    assert!(out.contains("finding(s)"), "totals line missing: {out}");
}

#[test]
fn error_on_warning_fails_on_warnings() {
    let input = "M1 a b c d nch w=1u\n"; // a..d all float
    let (_, _, code) = run(&["--lint", "--error-on", "warning"], Some(input));
    assert_eq!(code, 1);
    let (_, _, code) = run(&["--lint"], Some(input));
    assert_eq!(code, 0);
}

#[test]
fn max_warnings_gate() {
    let input = "M1 a b c d nch w=1u\n"; // 4 warnings
    let (_, _, code) = run(&["--lint", "--max-warnings", "3"], Some(input));
    assert_eq!(code, 1);
    let (_, _, code) = run(&["--lint", "--max-warnings", "4"], Some(input));
    assert_eq!(code, 0);
}

#[test]
fn suppressed_code_hides_from_details_stays_in_summary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spicefmt.toml"),
        "[lint]\nsuppress = [\"dangling-rc-endpoint\"]\n",
    )
    .unwrap();
    let f = dir.path().join("a.sp");
    std::fs::write(&f, "R1 lonely b 1k\nR2 b other 2k\n").unwrap();
    let p = f.to_string_lossy().to_string();
    let (human, _, _) = run(&["--lint", &p], None);
    assert!(!human.contains("lonely"), "suppressed finding leaked: {human}");
    let (summary, _, _) = run(&["--lint", "--format", "summary", p.as_str()], None);
    let line = summary.lines().find(|l| l.contains("dangling-rc-endpoint")).expect("in summary");
    assert!(line.contains("[suppressed]"), "{summary}");
}

#[test]
fn json_records_carry_schema_version() {
    let (out, _, _) = run(
        &["--lint", "--format", "json"],
        Some("X1 a b missing\n"),
    );
    for line in out.lines().filter(|l| !l.is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["schema_version"], 1);
    }
}
