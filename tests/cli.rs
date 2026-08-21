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
fn ends_name_mismatch_preserves_closing_name() {
    let (out, _, _) = run(&[], Some(".subckt inv a\n.ends wrong\nX1 p inv\n"));
    assert_eq!(out, ".subckt inv a\n.ends wrong\n\nX1 p inv\n");
    // idempotent
    let (out2, _, _) = run(&[], Some(&out));
    assert_eq!(out, out2);
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
