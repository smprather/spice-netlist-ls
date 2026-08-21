use crate::dialect::Dialect;
use crate::parser::{logical_line_spans, parse_logical_line};
use crate::ir::Stmt;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// 0-based line/col span; col is in UTF-16 code units (LSP convention).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LintRange {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: LintRange,
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

fn line_range(line: u32, len: u32) -> LintRange {
    LintRange { start_line: line, start_col: 0, end_line: line, end_col: len }
}

/// Node names that mean ground and never count as floating.
fn is_ground(node: &str) -> bool {
    let n = node.trim_end_matches('!').to_ascii_lowercase();
    matches!(n.as_str(), "0" | "gnd" | "ground")
}

#[derive(Default)]
pub struct LintOptions {
    /// Subckts defined outside this file (includes/libs):
    /// lowercase name -> port count (`None` = defined, arity unknown).
    pub external_subckts: HashMap<String, Option<usize>>,
}

/// Lint a single file's text. Pure: no filesystem access.
pub fn lint_str(input: &str, dialect: &Arc<dyn Dialect>, opts: &LintOptions) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    // name(lowercase) -> (port_count, def_line)
    let mut subckt_defs: HashMap<String, (usize, usize)> = HashMap::new();
    let mut open_subckts: Vec<(String, usize)> = Vec::new();

    // instance names per scope (top level + one per open subckt)
    let mut scopes: Vec<HashMap<String, usize>> = vec![HashMap::new()];

    // node(lowercase) -> (mention_count, first_line)
    let mut nodes: HashMap<String, (usize, usize)> = HashMap::new();

    // instance-node mentions as written: (lowercase, original spelling, line)
    let mut inst_node_mentions: Vec<(String, String, usize)> = Vec::new();

    // (ref_name, node_count, line)
    let mut xinsts: Vec<(String, usize, usize)> = Vec::new();

    for (start, _, line_text) in logical_line_spans(input, dialect.as_ref()) {
        let trimmed = line_text.trim();
        if trimmed.is_empty() || dialect.is_comment_line(trimmed) {
            continue;
        }
        // Attached continuations are merged into their parent's span, so a
        // logical line still starting with '+' has no parent statement.
        if trimmed.starts_with(dialect.continuation_char()) {
            diags.push(Diagnostic {
                range: line_range(start as u32, trimmed.len() as u32),
                severity: Severity::Warning,
                code: "orphan-continuation",
                message: "continuation line has no parent statement \
                          (was the line above commented out?)"
                    .to_string(),
            });
            continue;
        }
        match parse_logical_line(&line_text, dialect.as_ref()) {
            Stmt::Subckt(s) => {
                if !s.name.is_empty() {
                    subckt_defs.insert(s.name.to_ascii_lowercase(), (s.ports.len(), start));
                    open_subckts.push((s.name.to_ascii_lowercase(), start));
                    scopes.push(HashMap::new());
                    // Count ports as node mentions so a port used once in the
                    // body isn't flagged floating.
                    for p in &s.ports {
                        bump(&mut nodes, p, start);
                    }
                }
            }
            Stmt::Directive(d) if d.name == "ends" => {
                open_subckts.pop();
                scopes.pop();
                if scopes.is_empty() {
                    scopes.push(HashMap::new());
                }
            }
            Stmt::Instance(inst) => {
                let etype = inst.name.chars().next().map(|c| c.to_ascii_uppercase());
                if etype == Some('.') {
                    continue;
                }
                // duplicate instance name within the same scope
                let scope = scopes.last_mut().unwrap();
                let key = inst.name.to_ascii_lowercase();
                if let Some(prev) = scope.get(&key) {
                    diags.push(Diagnostic {
                        range: line_range(start as u32, trimmed.len() as u32),
                        severity: Severity::Warning,
                        code: "duplicate-instance",
                        message: format!(
                            "duplicate instance '{}' (first defined on line {})",
                            inst.name,
                            prev + 1
                        ),
                    });
                } else {
                    scope.insert(key, start);
                }

                for n in &inst.nodes {
                    bump(&mut nodes, n, start);
                    inst_node_mentions.push((n.to_ascii_lowercase(), n.clone(), start));
                }

                if etype == Some('X') {
                    if let Some(r) = &inst.model_or_value {
                        xinsts.push((r.clone(), inst.nodes.len(), start));
                    } else {
                        diags.push(Diagnostic {
                            range: line_range(start as u32, trimmed.len() as u32),
                            severity: Severity::Error,
                            code: "missing-subckt-ref",
                            message: format!("subckt instantiation '{}' has no subckt name", inst.name),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    for (name, start) in open_subckts.iter() {
        diags.push(Diagnostic {
            range: line_range(*start as u32, 6),
            severity: Severity::Error,
            code: "unterminated-subckt",
            message: format!("subckt '{name}' is never closed with .ends"),
        });
    }

    for (r, node_count, line) in &xinsts {
        let key = r.to_ascii_lowercase();
        let known_ports: Option<usize> = match subckt_defs.get(&key) {
            Some(&(ports, _)) => Some(ports),
            None => match opts.external_subckts.get(&key) {
                None => {
                    diags.push(Diagnostic {
                        range: line_range(*line as u32, r.len() as u32),
                        severity: Severity::Error,
                        code: "undefined-subckt",
                        message: format!("subckt '{r}' is not defined in this file or its includes"),
                    });
                    continue;
                }
                Some(ports) => *ports,
            },
        };
        if let Some(ports) = known_ports
            && ports != *node_count
        {
            diags.push(Diagnostic {
                range: line_range(*line as u32, r.len() as u32),
                severity: Severity::Warning,
                code: "arity-mismatch",
                message: format!("'{r}' expects {ports} node(s), got {node_count}"),
            });
        }
    }

    for (lower, spelling, line) in &inst_node_mentions {
        if is_ground(spelling) {
            continue;
        }
        if let Some(&(count, first_line)) = nodes.get(lower)
            && count == 1
            && first_line == *line
        {
            diags.push(Diagnostic {
                range: line_range(*line as u32, spelling.len() as u32),
                severity: Severity::Warning,
                code: "floating-node",
                message: format!("node '{spelling}' is connected to only one element"),
            });
        }
    }

    diags.sort_by_key(|d| (d.range.start_line, d.range.start_col));
    diags.dedup_by(|a, b| a.code == b.code && a.range == b.range && a.message == b.message);
    diags
}

fn bump(map: &mut HashMap<String, (usize, usize)>, node: &str, line: usize) {
    map.entry(node.to_ascii_lowercase())
        .and_modify(|(c, l)| {
            *c += 1;
            *l = (*l).min(line);
        })
        .or_insert((1, line));
}

/// Collect every subckt visible from `path` — including the file itself and
/// everything reachable via `.include`/`.inc`/`.lib` — with its port count,
/// so arity checks work against PDK/library cells. Cycles guarded.
pub fn external_subckts(path: &Path, dialect: &Arc<dyn Dialect>) -> HashMap<String, Option<usize>> {
    let mut out = HashMap::new();
    let mut visited = HashSet::new();
    walk(path, dialect, &mut visited, &mut out);
    out
}

/// Subckt name -> port count for all `.subckt` definitions in `text`.
fn subckt_port_counts(text: &str, dialect: &Arc<dyn Dialect>) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for (_, _, line) in logical_line_spans(text, dialect.as_ref()) {
        if let Stmt::Subckt(s) = parse_logical_line(&line, dialect.as_ref())
            && !s.name.is_empty()
        {
            out.push((s.name.to_ascii_lowercase(), s.ports.len()));
        }
    }
    out
}

fn walk(
    path: &Path,
    dialect: &Arc<dyn Dialect>,
    visited: &mut HashSet<PathBuf>,
    out: &mut HashMap<String, Option<usize>>,
) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for (name, ports) in subckt_port_counts(&text, dialect) {
        out.insert(name, Some(ports));
    }
    for inc in crate::parser::include_paths(&text, dialect.as_ref()) {
        let inc_path = if Path::new(&inc).is_absolute() {
            PathBuf::from(&inc)
        } else if let Some(parent) = path.parent() {
            parent.join(&inc)
        } else {
            continue;
        };
        walk(&inc_path, dialect, visited, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::{get_dialect, DialectKind};

    fn hspice() -> Arc<dyn Dialect> {
        get_dialect(DialectKind::Hspice)
    }

    fn lint(input: &str) -> Vec<Diagnostic> {
        lint_str(input, &hspice(), &LintOptions::default())
    }

    fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
        diags.iter().map(|d| d.code).collect()
    }

    #[test]
    fn clean_netlist_has_no_diagnostics() {
        let input = "\
* title
.subckt inv a y vdd vss
Mn y a vss vss nch w=1u
Mp y a vdd vdd pch w=2u
.ends
Xinv in out vdd gnd inv
Cload out gnd 10f
Vdd vdd gnd 1.2
Vin in gnd pulse(0 1.2 0 100p 100p 1n 2n)
";
        assert_eq!(codes(&lint(input)), Vec::<&'static str>::new());
    }

    #[test]
    fn flags_undefined_subckt() {
        let input = ".subckt buf o i\n.ends\nX1 a b missing_block\n";
        let diags = lint(input);
        assert!(codes(&diags).contains(&"undefined-subckt"));
        assert_eq!(diags[0].range.start_line, 2);
    }

    #[test]
    fn external_defs_suppress_undefined() {
        let input = "X1 a b ext\n";
        let mut opts = LintOptions::default();
        // defined externally, arity unknown -> no undefined, no arity check
        opts.external_subckts.insert("ext".to_string(), None);
        let diags = lint_str(input, &hspice(), &opts);
        assert!(!codes(&diags).contains(&"undefined-subckt"));
        assert!(!codes(&diags).contains(&"arity-mismatch"));
    }

    #[test]
    fn external_port_count_enables_arity_check() {
        let input = "X1 a b sky130_inv\n";
        let mut opts = LintOptions::default();
        opts.external_subckts.insert("sky130_inv".to_string(), Some(4));
        let diags = lint_str(input, &hspice(), &opts);
        assert!(codes(&diags).contains(&"arity-mismatch"));
        assert!(!codes(&diags).contains(&"undefined-subckt"));
    }

    #[test]
    fn external_port_count_match_is_clean() {
        let input = "X1 a b c d sky130_buf\n";
        let mut opts = LintOptions::default();
        opts.external_subckts.insert("sky130_buf".to_string(), Some(4));
        let diags = lint_str(input, &hspice(), &opts);
        assert!(!codes(&diags).contains(&"arity-mismatch"));
        assert!(!codes(&diags).contains(&"undefined-subckt"));
    }

    #[test]
    fn flags_arity_mismatch() {
        let input = ".subckt two a b\n.ends\nX1 only_one two\n";
        let diags = lint(input);
        let arity = diags.iter().find(|d| d.code == "arity-mismatch").unwrap();
        assert_eq!(arity.range.start_line, 2);
        assert_eq!(arity.severity, Severity::Warning);
    }

    #[test]
    fn flags_floating_node_but_not_ground() {
        let input = "* title\nR1 a b 1k\nR2 c b 2k\n";
        let diags = lint(input);
        let floats: Vec<_> = diags.iter().filter(|d| d.code == "floating-node").collect();
        assert_eq!(floats.len(), 2); // 'a' and 'c'
    }

    #[test]
    fn subckt_port_used_once_in_body_is_not_floating() {
        let input = ".subckt inv a y\nMn y a 0 0 nch w=1u\n.ends\n";
        let diags = lint(input);
        assert_eq!(codes(&diags), Vec::<&'static str>::new());
    }

    #[test]
    fn flags_duplicate_instance_name_in_scope() {
        let input = "R1 a b 1k\nR1 c d 2k\n";
        let diags = lint(input);
        // duplicate reported on the second occurrence; a/b/c/d each float
        assert_eq!(
            codes(&diags),
            vec![
                "floating-node",
                "floating-node",
                "duplicate-instance",
                "floating-node",
                "floating-node"
            ]
        );
        assert_eq!(
            diags.iter().find(|d| d.code == "duplicate-instance").unwrap().range.start_line,
            1
        );
    }

    #[test]
    fn same_instance_name_in_different_subckts_is_ok() {
        let input = ".subckt a p q\nR1 p q 1k\n.ends\n.subckt b p q\nR1 p q 2k\n.ends\n";
        assert!(!codes(&lint(input)).contains(&"duplicate-instance"));
    }

    #[test]
    fn flags_unterminated_subckt() {
        let input = "* t\n.subckt inv a y\nMn y a 0 0 nch w=1u\n";
        let diags = lint(input);
        let unterm = diags.iter().find(|d| d.code == "unterminated-subckt").unwrap();
        assert_eq!(unterm.range.start_line, 1);
    }

    #[test]
    fn flags_orphan_continuation_after_commented_parent() {
        let input = "Rload out gnd 10k\n* Xinv in out inv\n+ w=2u\n";
        let diags = lint(input);
        let orphan = diags.iter().find(|d| d.code == "orphan-continuation").unwrap();
        assert_eq!(orphan.range.start_line, 2);
        // and no false floating/other noise from the orphan being parsed as an instance
    }

    #[test]
    fn attached_continuation_is_not_orphan() {
        let input = "M1 a b c d nch\n+ w=1u ad=0.5p\n";
        assert!(!codes(&lint(input)).contains(&"orphan-continuation"));
    }

    #[test]
    fn diagnostics_are_sorted_by_position() {
        let input = "X1 a b nope\nR1 a b 1k\nR1 c b 2k\n";
        let lines: Vec<u32> = lint(input).iter().map(|d| d.range.start_line).collect();
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted);
    }
}
