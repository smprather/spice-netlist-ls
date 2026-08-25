use crate::dialect::Dialect;
use crate::fx::FxHashMap;
use crate::ir::Stmt;
use crate::parser::{logical_line_spans, parse_logical_line};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Machine-readable output formats for `spicefmt --lint --format`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LintReportFormat {
    /// `path:line:col: severity [code]: message` — stable, grep-friendly.
    #[default]
    Human,
    /// Newline-delimited JSON: one diagnostic object per line.
    Json,
    /// Static Analysis Results Interchange Format; GitLab/GitHub Enterprise
    /// merge-request UIs render this natively without any plugin.
    Sarif,
}

/// Bumped on any breaking change to the JSONL record shape; consumers use
/// it to detect format drift instead of silently mis-parsing.
pub const LINT_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct DiagnosticJson<'a> {
    path: &'a str,
    line: u32,
    col: u32,
    severity: &'a str,
    code: &'a str,
    message: &'a str,
    schema_version: u32,
}

pub fn diagnostics_as_json(path: &str, diags: &[Diagnostic]) -> String {
    let mut out = String::new();
    for d in diags {
        let rec = DiagnosticJson {
            path,
            line: d.range.start_line + 1,
            col: d.range.start_col + 1,
            severity: d.severity.as_str(),
            code: d.code,
            message: &d.message,
            schema_version: LINT_JSON_SCHEMA_VERSION,
        };
        out.push_str(&serde_json::to_string(&rec).unwrap_or_default());
        out.push('\n');
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLog<'a> {
    version: &'static str,
    #[serde(rename = "$schema")]
    schema: &'static str,
    runs: Vec<SarifRun<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRun<'a> {
    tool: SarifTool,
    results: Vec<SarifResult<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver {
    name: &'static str,
    version: &'static str,
    information_uri: &'static str,
    rules: Vec<SarifRule>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRule {
    id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult<'a> {
    rule_id: &'a str,
    level: &'static str,
    message: SarifMessage<'a>,
    locations: Vec<SarifLocation<'a>>,
}

#[derive(Serialize)]
struct SarifMessage<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation<'a> {
    physical_location: SarifPhysicalLocation<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation<'a> {
    artifact_location: SarifArtifactLocation<'a>,
    region: SarifRegion,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifArtifactLocation<'a> {
    uri: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    start_line: u32,
    start_column: u32,
}

pub fn diagnostics_as_sarif(path: &str, diags: &[Diagnostic]) -> String {
    let mut rules: Vec<String> = diags.iter().map(|d| d.code.to_string()).collect();
    rules.sort();
    rules.dedup();
    let results = diags
        .iter()
        .map(|d| SarifResult {
            rule_id: d.code,
            level: match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            message: SarifMessage { text: &d.message },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation { uri: path },
                    region: SarifRegion {
                        start_line: d.range.start_line + 1,
                        start_column: d.range.start_col + 1,
                    },
                },
            }],
        })
        .collect();
    let log = SarifLog {
        version: "2.1.0",
        schema: "https://json.schemastore.org/sarif-2.1.0.json",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "spicefmt",
                    version: env!("CARGO_PKG_VERSION"),
                    information_uri: "https://github.com/smprather/spice-netlist-ls",
                    rules: rules.into_iter().map(|id| SarifRule { id }).collect(),
                },
            },
            results,
        }],
    };
    serde_json::to_string_pretty(&log).unwrap_or_default()
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
    let n = node.trim_end_matches('!');
    n == "0" || n.eq_ignore_ascii_case("gnd") || n.eq_ignore_ascii_case("ground")
}

#[derive(Default)]
pub struct LintOptions {
    /// Subckts defined outside this file (includes/libs):
    /// lowercase name -> port count (`None` = defined, arity unknown).
    pub external_subckts: HashMap<String, Option<usize>>,
}

/// Lint a single file's text. Pure: no filesystem access.
///
/// If the file contains `simulator lang=` directives, each section is linted
/// under its own dialect and diagnostic line numbers are offset to global
/// coordinates. `external_subckts` is unioned across sections (subckts
/// defined in a spice section are visible to a spectre section and vice
/// versa — they share a namespace within the file). If the file has no
/// `simulator lang=` line, the fast path lints the whole file under one
/// dialect, byte-identical to today's behavior.
pub fn lint_str(input: &str, dialect: &Arc<dyn Dialect>, opts: &LintOptions) -> Vec<Diagnostic> {
    let fallback = dialect.kind();
    let secs = crate::segments::segments(input, fallback);

    // Fast path: no `simulator lang=` directive → today's code path, unchanged.
    if secs.len() == 1 && secs[0].header.is_none() {
        let empty = FxHashMap::default();
        return lint_str_single(input, dialect, opts, &empty);
    }

    // Sectioned path: lint each body under its dialect, offsetting line
    // numbers to global coordinates. external_subckts from includes is
    // already dialect-agnostic at the file level (it walks the include tree
    // with one dialect); the per-section def map covers same-file defs.
    //
    // Subckts defined in one section are visible to X-instances in any other
    // section (they share a namespace within the file), so pre-scan every
    // section for `.subckt` defs under its own dialect and merge them into a
    // shared map passed to each section's lint pass.
    let mut cross_defs: FxHashMap<String, usize> = FxHashMap::default();
    for sec in &secs {
        let sub_dialect = crate::dialect::get_dialect(sec.dialect);
        let (defs, _incs) = crate::parser::scan_subckt_defs_and_includes(sec.body, sub_dialect.as_ref());
        for (name, ports) in defs {
            cross_defs.insert(name, ports);
        }
    }

    let mut diags = Vec::new();
    for sec in &secs {
        let sub_dialect = crate::dialect::get_dialect(sec.dialect);
        for mut d in lint_str_single(sec.body, &sub_dialect, opts, &cross_defs) {
            d.range.start_line += sec.line_offset as u32;
            d.range.end_line += sec.line_offset as u32;
            diags.push(d);
        }
    }

    diags.sort_by_key(|d| (d.range.start_line, d.range.start_col));
    diags.dedup_by(|a, b| a.code == b.code && a.range == b.range && a.message == b.message);
    diags
}

/// Lint a single section's body text under one dialect. Pure: no filesystem
/// access. Line numbers are within `input` (0-based); the section-aware
/// wrapper offsets them to global coordinates. `cross_defs` carries
/// `.subckt` definitions from *other* sections of the same file so X-instance
/// resolution sees the file-wide namespace (defs found in this section are
/// discovered during the walk and override `cross_defs` for open-subckt
/// tracking, but either way the port count is what matters for arity checks).
fn lint_str_single(
    input: &str,
    dialect: &Arc<dyn Dialect>,
    opts: &LintOptions,
    cross_defs: &FxHashMap<String, usize>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    // Format-derived blank-line rules (ruff-inspired, fixable via formatter).
    // These are emitted here so `spicefmt --lint` reports the same violations
    // the formatter would fix. Codes are the same as `[format] ignore` names.
    // Other format rules (lowercase-directive, eq-spacing, line-wrap, etc.)
    // are formatter-only and not reported as lint to avoid noise; they are
    // still opt-out via [format] ignore for the formatter.
    lint_blank_rules(input, &mut diags);

    // name(lowercase) -> (port_count, def_line)
    let mut subckt_defs: FxHashMap<String, (usize, usize)> = FxHashMap::default();
    let mut open_subckts: Vec<(String, usize)> = Vec::new();

    // instance names per scope (top level + one per open subckt)
    let mut scopes: Vec<FxHashMap<String, usize>> = vec![FxHashMap::default()];

    // Nodes are interned: one hash per mention, all per-node state in
    // id-indexed side tables. Previously this was three HashMap probes and
    // two String clones per node mention.
    let mut nodes = NodeTable::default();

    // instance-node mentions: (node id, line, element type char, e.g.
    // 'R'/'C' for passive-network classification)
    let mut inst_node_mentions: Vec<(u32, u32, char)> = Vec::new();

    // (ref_name, node_count, line)
    let mut xinsts: Vec<(String, usize, usize)> = Vec::new();

    // ngspice `.control` blocks contain simulator command language, not
    // netlist cards; instance and node analysis must skip the interior.
    let mut control_depth = 0usize;

    let logical = logical_line_spans(input, dialect.as_ref());
    // Pre-size to the statement count: every instance lands in the top scope
    // map and every node in the interner, so growing them from empty would
    // rehash ~17 times on large decks.
    scopes[0].reserve(logical.len());
    nodes.map.reserve(logical.len());
    inst_node_mentions.reserve(2 * logical.len());

    for &(start, _, ref line_text) in &logical {
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

        let mut directive_name: Option<String> = None;
        if let Some(rest) = trimmed.strip_prefix('.') {
            directive_name = Some(
                rest.split(|c: char| !c.is_ascii_alphanumeric())
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase(),
            );
        }

        match directive_name.as_deref() {
            Some("control") => {
                control_depth += 1;
                collect_observed(&line_text, &mut nodes);
                continue;
            }
            Some("endc") => {
                control_depth = control_depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }

        if control_depth > 0 {
            // Command language (`let`, `meas`, `if`, ...): only measurement
            // observation matters here, never netlist structure.
            collect_observed(&line_text, &mut nodes);
            continue;
        }

        match parse_logical_line(&line_text, dialect.as_ref()) {
            Stmt::Subckt(s) => {
                if !s.name.is_empty() {
                    subckt_defs.insert(s.name.to_ascii_lowercase(), (s.ports.len(), start));
                    open_subckts.push((s.name.to_ascii_lowercase(), start));
                    scopes.push(FxHashMap::default());
                    // Count ports as node mentions so a port used once in the
                    // body isn't flagged floating.
                    for p in &s.ports {
                        let id = nodes.intern_mention(p, start as u32);
                        nodes.count[id as usize] += 1;
                    }
                }
            }
            Stmt::Directive(d) if d.name.eq_ignore_ascii_case("ends") => {
                // Simulators close a subckt on any `.ends`; a name that does
                // not match the open subckt is almost always a typo.
                if let Some(ends_name) = d.args.first()
                    && !ends_name.is_empty()
                    && let Some((open_name, _)) = open_subckts.last()
                    && !ends_name.eq_ignore_ascii_case(open_name)
                {
                    diags.push(Diagnostic {
                        range: line_range(start as u32, trimmed.len() as u32),
                        severity: Severity::Warning,
                        code: "ends-name-mismatch",
                        message: format!("'.ends {ends_name}' closes subckt '{open_name}'"),
                    });
                }
                if open_subckts.pop().is_none() {
                    diags.push(Diagnostic {
                        range: line_range(start as u32, trimmed.len() as u32),
                        severity: Severity::Warning,
                        code: "stray-ends",
                        message: ".ends without an open .subckt".to_string(),
                    });
                }
                scopes.pop();
                if scopes.is_empty() {
                    scopes.push(FxHashMap::default());
                }
            }
            Stmt::Instance(inst) => {
                let etype = inst.name.chars().next().map(|c| c.to_ascii_uppercase());
                if etype == Some('.') {
                    continue;
                }                // duplicate instance name within the same scope
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
                    let id = nodes.intern_mention(n, start as u32);
                    nodes.count[id as usize] += 1;
                    // First-written spelling per node so a later re-spelling
                    // with different case in a case-insensitive dialect can
                    // be flagged as a likely typo.
                    let (orig, first_line) = nodes.spelling[id as usize].as_ref().unwrap();
                    if orig.as_ref() != n {
                        diags.push(Diagnostic {
                            range: line_range(start as u32, trimmed.len() as u32),
                            severity: Severity::Warning,
                            code: "node-case-collision",
                            message: format!(
                                "node '{n}' differs only by case from '{orig}' \
                                 (first used on line {}); names are case-insensitive",
                                *first_line + 1
                            ),
                        });
                    }
                    inst_node_mentions.push((id, start as u32, etype.unwrap_or('?')));
                }

                if etype == Some('X') {
                    if let Some(r) = &inst.model_or_value {
                        xinsts.push((r.to_string(), inst.nodes.len(), start));
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
            Stmt::Directive(d) if matches!(d.name.to_ascii_lowercase().as_str(), "measure" | "meas" | "probe" | "print" | "plot" | "save") => {
                collect_observed(&line_text, &mut nodes);
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
            None => match cross_defs.get(&key) {
                Some(&ports) => Some(ports),
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

    for (id, line, etype) in &inst_node_mentions {
        // Instance mentions always record a spelling at intern time.
        let spelling = nodes.spelling[*id as usize].as_ref().unwrap().0.as_ref();
        if is_ground(spelling) || nodes.observed[*id as usize] {
            continue;
        }
        if nodes.count[*id as usize] == 1 && nodes.first_line[*id as usize] == *line {
            // A lonely node on a passive element is the signature of an
            // extracted RC/L network endpoint — triage it separately from a
            // dangling device pin.
            let (code, message) = if matches!(etype, 'R' | 'C' | 'L') {
                (
                    "dangling-rc-endpoint",
                    format!(
                        "node '{spelling}' terminates a passive network with no other connection"
                    ),
                )
            } else {
                (
                    "floating-node",
                    format!("node '{spelling}' is connected to only one element"),
                )
            };
            diags.push(Diagnostic {
                range: line_range(*line, spelling.len() as u32),
                severity: Severity::Warning,
                code,
                message,
            });
        }
    }

    diags.sort_by_key(|d| (d.range.start_line, d.range.start_col));
    diags.dedup_by(|a, b| a.code == b.code && a.range == b.range && a.message == b.message);
    diags
}

fn lint_blank_rules(input: &str, diags: &mut Vec<Diagnostic>) {
    let lines: Vec<&str> = input.lines().collect();
    let mut stack: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with(".subckt") {
            // Check next line is blank -> violation blank-after-subckt
            if i + 1 < lines.len() && lines[i + 1].trim().is_empty() {
                diags.push(Diagnostic {
                    range: line_range((i + 1) as u32, lines[i + 1].len() as u32),
                    severity: Severity::Warning,
                    code: "blank-after-subckt",
                    message: "empty line after .subckt is not allowed".to_string(),
                });
            }
            stack.push(i);
        } else if lower.starts_with(".ends") {
            // blank-before-ends: previous line is blank
            if i > 0 && lines[i - 1].trim().is_empty() {
                diags.push(Diagnostic {
                    range: line_range((i - 1) as u32, lines[i - 1].len() as u32),
                    severity: Severity::Warning,
                    code: "blank-before-ends",
                    message: "empty line before .ends is not allowed".to_string(),
                });
            }
            // blank-after-ends: at least one blank after .ends for top-level
            let depth = stack.len();
            if depth == 1 {
                let next_is_blank = i + 1 < lines.len() && lines[i + 1].trim().is_empty();
                let next_is_eof = i + 1 >= lines.len();
                if next_is_eof {
                    // File ends immediately after .ends – need blank line before EOF
                    // Input ending with "\n\n" or "\n\r\n" would have an extra empty line,
                    // but lines() discards trailing empty, so check raw input.
                    let ends_with_blank = input.ends_with("\n\n") || input.ends_with("\r\n\r\n") || input.ends_with("\n \n") ;
                    if !ends_with_blank {
                        diags.push(Diagnostic {
                            range: line_range(i as u32, trimmed.len() as u32),
                            severity: Severity::Warning,
                            code: "blank-after-ends",
                            message: "at least one empty line required after .ends".to_string(),
                        });
                    }
                } else if !next_is_blank {
                    diags.push(Diagnostic {
                        range: line_range(i as u32, trimmed.len() as u32),
                        severity: Severity::Warning,
                        code: "blank-after-ends",
                        message: "at least one empty line required after .ends".to_string(),
                    });
                }
            }
            if !stack.is_empty() {
                stack.pop();
            }
        }
    }
}

/// Node interner. Maps a lowercased node name to a dense id; all per-node
/// state (mention count, first-mention line, first-written spelling,
/// observed-by-measurement flag) lives in id-indexed vectors.
///
/// `spelling` stays `None` for ids created purely by `v(...)` observation so
/// an observed token can never seed a `node-case-collision` report — only a
/// real instance/port mention sets it. This matches the previous
/// two-map design where observed nodes never entered the spelling map.
#[derive(Default)]
struct NodeTable {
    map: FxHashMap<Box<str>, u32>,
    spelling: Vec<Option<(Box<str>, u32)>>,
    count: Vec<u32>,
    first_line: Vec<u32>,
    observed: Vec<bool>,
}

impl NodeTable {
    /// Id of `lower` (already lowercased), interning with `line` on first
    /// sight. Does not bump the mention count or set a spelling.
    fn intern(&mut self, lower: String, line: u32) -> u32 {
        if let Some(&id) = self.map.get(lower.as_str()) {
            return id;
        }
        let id = self.spelling.len() as u32;
        self.map.insert(lower.into_boxed_str(), id);
        self.spelling.push(None);
        self.count.push(0);
        self.first_line.push(line);
        self.observed.push(false);
        id
    }

    /// Intern an instance/port mention: sets the first-written spelling on
    /// first mention. Caller bumps `count` and (for instances) compares the
    /// spelling for case-collision diagnostics.
    fn intern_mention(&mut self, name: &str, line: u32) -> u32 {
        let id = self.intern(name.to_ascii_lowercase(), line);
        if self.spelling[id as usize].is_none() {
            self.spelling[id as usize] = Some((name.into(), line));
        }
        id
    }
}

/// Pull `v(node)` / `v(n1,n2)` / `i(vsrc)` references out of a measurement,
/// probe, save, or `.control` command line. Anything observed by an
/// analysis statement is not floating, no matter how it is driven.
/// Observed tokens are interned (the line is unused for them — observed
/// nodes are skipped before any line comparison) but never set a spelling.
fn collect_observed(line: &str, nodes: &mut NodeTable) {
    let lower = line.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'v' && bytes[i + 1] == b'(' {
            let mut j = i + 2;
            let start = j;
            while j < bytes.len() && bytes[j] != b')' {
                j += 1;
            }
            for token in lower[start..j.min(lower.len())].split(',') {
                let t = token.trim();
                if !t.is_empty()
                    && t.chars().all(|c| {
                        c.is_ascii_alphanumeric() || "[]().$_!#<>|*\\".contains(c)
                    })
                {
                    let id = nodes.intern(t.to_string(), 0);
                    nodes.observed[id as usize] = true;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

/// Collect every subckt visible from `path` — including the file itself and
/// everything reachable via `.include`/`.inc`/`.lib` — with its port count,
/// so arity checks work against PDK/library cells. Cycles guarded.
///
/// If `path` contains `simulator lang=` directives, the file is segmented and
/// each section's includes are walked under that section's dialect, unioning
/// the results (subckts from a spice-section include are visible to a
/// spectre-section instance and vice versa). Plain files take the single-
/// dialect fast path unchanged.
pub fn external_subckts(path: &Path, dialect: &Arc<dyn Dialect>) -> HashMap<String, Option<usize>> {
    let mut out = HashMap::new();
    let mut visited = HashSet::new();

    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    let fallback = dialect.kind();
    let secs = crate::segments::segments(&text, fallback);

    // Fast path: no `simulator lang=` → single-dialect walk, unchanged.
    if secs.len() == 1 && secs[0].header.is_none() {
        walk(path, dialect, &mut visited, &mut out);
        return out;
    }

    // Sectioned: walk includes per section under that section's dialect. The
    // root file's own per-section defs are picked up by `cross_defs` inside
    // `lint_str`; here we only need the *external* (include) defs, but we
    // still seed the visited set with the root so we don't re-walk it.
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    visited.insert(canonical);
    for sec in &secs {
        let sub_dialect = crate::dialect::get_dialect(sec.dialect);
        let (_, includes) = crate::parser::scan_subckt_defs_and_includes(sec.body, sub_dialect.as_ref());
        for inc in includes {
            let inc_path = if Path::new(&inc).is_absolute() {
                PathBuf::from(&inc)
            } else if let Some(parent) = path.parent() {
                parent.join(&inc)
            } else {
                continue;
            };
            walk(&inc_path, &sub_dialect, &mut visited, &mut out);
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
    // One pass per file: subckt defs and include paths together, with a
    // prefix filter so device cards never reach the full parser.
    let (defs, includes) = crate::parser::scan_subckt_defs_and_includes(&text, dialect.as_ref());
    for (name, ports) in defs {
        out.insert(name, Some(ports));
    }
    for inc in includes {
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
Mn y a vss vss nch w = 1u
Mp y a vdd vdd pch w = 2u
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
        let input = ".subckt buf o i\n.ends\n\nX1 a b missing_block\n";
        let diags = lint(input);
        assert!(codes(&diags).contains(&"undefined-subckt"));
        assert_eq!(diags.iter().find(|d| d.code == "undefined-subckt").unwrap().range.start_line, 3);
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
        let input = ".subckt two a b\n.ends\n\nX1 only_one two\n";
        let diags = lint(input);
        let arity = diags.iter().find(|d| d.code == "arity-mismatch").unwrap();
        assert_eq!(arity.range.start_line, 3);
        assert_eq!(arity.severity, Severity::Warning);
    }

    #[test]
    fn flags_floating_node_but_not_ground() {
        let input = "* title\nR1 a b 1k\nR2 c b 2k\n";
        let diags = lint(input);
        // 'a' and 'c' terminate passive networks -> distinct code
        let rcs: Vec<_> = diags.iter().filter(|d| d.code == "dangling-rc-endpoint").collect();
        assert_eq!(rcs.len(), 2);
        // a device pin would still be floating-node
        let input = "* title\nM1 a b c d nch\n";
        let diags = lint(input);
        assert!(diags.iter().any(|d| d.code == "floating-node"));
    }

    #[test]
    fn subckt_port_used_once_in_body_is_not_floating() {
        let input = ".subckt inv a y\nMn y a 0 0 nch w = 1u\n.ends\n\n";
        let diags = lint(input);
        assert_eq!(codes(&diags), Vec::<&'static str>::new());
    }

    #[test]
    fn flags_duplicate_instance_name_in_scope() {
        let input = "R1 a b 1k\nR1 c d 2k\n";
        let diags = lint(input);
        // duplicate reported on the second occurrence; a/b/c/d all terminate
        // passive networks
        assert_eq!(
            codes(&diags),
            vec![
                "dangling-rc-endpoint",
                "dangling-rc-endpoint",
                "duplicate-instance",
                "dangling-rc-endpoint",
                "dangling-rc-endpoint"
            ]
        );
        assert_eq!(
            diags.iter().find(|d| d.code == "duplicate-instance").unwrap().range.start_line,
            1
        );
    }

    #[test]
    fn same_instance_name_in_different_subckts_is_ok() {
        let input = ".subckt a p q\nR1 p q 1k\n.ends\n\n.subckt b p q\nR1 p q 2k\n.ends\n\n";
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
    fn bare_title_like_rc_lowpass_is_flagged_as_invalid_resistor() {
        // "RC lowpass" looks like R device "RC" with single node "lowpass" and no
        // value — not a valid resistor. It must be flagged, not treated as title.
        // The correct title form is "* RC lowpass" (comment). This is the
        // incorrect-format testcase for simple_rc_chain's title line.
        let input = "RC lowpass\nR1 a b 1k\n";
        let diags = lint(input);
        assert!(
            codes(&diags).contains(&"dangling-rc-endpoint"),
            "bare 'RC lowpass' should be flagged as dangling-rc-endpoint, got {:?}",
            codes(&diags)
        );
        // Correct form (comment) must NOT be flagged — use a closed loop so no dangling
        let input_ok = "* RC lowpass\nR1 a b 1k\nR2 b c 1k\nR3 c a 1k\n";
        assert!(
            !codes(&lint(input_ok)).contains(&"dangling-rc-endpoint"),
            "comment title '* RC lowpass' should not be flagged, got {:?}",
            codes(&lint(input_ok))
        );
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

    // ---------- per-section dialect switching (PLAN_scs_segments.md) ----------

    fn spectre_dialect() -> Arc<dyn Dialect> {
        get_dialect(DialectKind::Spectre)
    }

    #[test]
    fn sectioned_lint_offsets_line_numbers_to_global() {
        // A floating node in the *second* (spectre) section must report the
        // global line, not the within-section line. The spice section defines
        // `inv` (4 ports); the spectre section instantiates it across the
        // section boundary via the cross_defs map.
        let input = "\
simulator lang=spice
.subckt inv a y vdd vss
Mn y a vss vss nch w=1u
Mp y a vdd vdd pch w=2u
.ends
simulator lang=spectre
// spectre section
Xinv in out vdd gnd inv
";
        let diags = lint_str(input, &spectre_dialect(), &LintOptions::default());
        // `in` appears once (in the Xinv in the spectre section at global
        // line 7). The within-section line would be 1; the global must be 7.
        let floating = diags
            .iter()
            .find(|d| d.code == "floating-node" && d.message.contains("'in'"));
        assert!(floating.is_some(), "no floating-node 'in': {:?}", codes(&diags));
        assert_eq!(
            floating.unwrap().range.start_line, 7,
            "second-section diagnostic must use global line number"
        );
    }

    #[test]
    fn sectioned_lint_resolves_subckt_across_section_boundary() {
        // A subckt defined in a spice section is visible to an X-instance in
        // a spectre section of the same file (cross_defs union). Arity is
        // checked against the spice def's port count.
        let input = "\
simulator lang=spice
.subckt inv a y vdd vss
Mn y a vss vss nch w=1u
.ends
simulator lang=spectre
X1 p q r s inv
";
        let diags = lint_str(input, &spectre_dialect(), &LintOptions::default());
        // inv is defined in the spice section with 4 ports; X1 passes 4
        // nodes → no undefined-subckt, no arity-mismatch.
        assert!(
            !codes(&diags).contains(&"undefined-subckt"),
            "cross-section def should resolve: {:?}",
            codes(&diags)
        );
        assert!(
            !codes(&diags).contains(&"arity-mismatch"),
            "4-port def vs 4-node instance: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn sectioned_lint_reports_arity_mismatch_across_boundary() {
        let input = "\
simulator lang=spice
.subckt two a b
.ends
simulator lang=spectre
X1 p q r two
";
        let diags = lint_str(input, &spectre_dialect(), &LintOptions::default());
        // `two` has 2 ports; X1 passes 3 nodes → arity-mismatch (not undefined).
        assert!(codes(&diags).contains(&"arity-mismatch"));
        assert!(!codes(&diags).contains(&"undefined-subckt"));
    }

    #[test]
    fn sectioned_lint_simulator_header_not_parsed_as_instance() {
        // Before the fix, `simulator lang=spectre` was parsed as an instance
        // named `simulator`, producing a spurious `duplicate-instance` on the
        // second header. The segmenter excludes headers from bodies, so no
        // instance diagnostic should mention 'simulator'.
        let input = "\
simulator lang=spice
R1 a b 1k
simulator lang=spectre
R2 c d 2k
";
        let diags = lint_str(input, &spectre_dialect(), &LintOptions::default());
        assert!(
            !diags.iter().any(|d| d.message.contains("'simulator'")),
            "header parsed as instance: {:?}",
            codes(&diags)
        );
    }

    #[test]
    fn plain_deck_lint_is_byte_identical_to_pre_segmentation() {
        // Fast path: no `simulator lang=` → single-section, no header. The
        // diagnostic set must match what lint_str_single would produce
        // directly (this is the no-regression guard for the linter).
        let input = ".subckt buf o i\n.ends\n\nX1 p q buf\n";
        let via_lint_str = lint_str(input, &hspice(), &LintOptions::default());
        let empty = crate::fx::FxHashMap::default();
        let via_single = lint_str_single(input, &hspice(), &LintOptions::default(), &empty);
        assert_eq!(
            codes(&via_lint_str),
            codes(&via_single),
            "fast path must match single-section lint exactly"
        );
    }

    #[test]
    fn blank_after_subckt_is_flagged() {
        let input = ".subckt a b\n\nR1 a b 1k\n.ends a\n\n";
        assert!(codes(&lint(input)).contains(&"blank-after-subckt"));
        let clean = ".subckt a b\nR1 a b 1k\n.ends a\n\n";
        assert!(!codes(&lint(clean)).contains(&"blank-after-subckt"));
    }

    #[test]
    fn blank_before_ends_is_flagged() {
        let input = ".subckt a b\nR1 a b 1k\n\n.ends a\n\n";
        assert!(codes(&lint(input)).contains(&"blank-before-ends"));
        let clean = ".subckt a b\nR1 a b 1k\n.ends a\n\n";
        assert!(!codes(&lint(clean)).contains(&"blank-before-ends"));
    }

    #[test]
    fn blank_after_ends_is_flagged() {
        let input = ".subckt a b\nR1 a b 1k\n.ends a\nX1 a b a\n";
        assert!(codes(&lint(input)).contains(&"blank-after-ends"));
        let clean = ".subckt a b\nR1 a b 1k\n.ends a\n\nX1 a b a\n";
        assert!(!codes(&lint(clean)).contains(&"blank-after-ends"));
        // at EOF, missing blank is also flagged
        let eof_no_blank = ".subckt a b\nR1 a b 1k\n.ends a\n";
        assert!(codes(&lint(eof_no_blank)).contains(&"blank-after-ends"));
        let eof_blank = ".subckt a b\nR1 a b 1k\n.ends a\n\n";
        assert!(!codes(&lint(eof_blank)).contains(&"blank-after-ends"));
    }
}
