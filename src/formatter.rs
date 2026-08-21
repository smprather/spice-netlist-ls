use crate::dialect::{Dialect, DialectKind};
use crate::ir::{Directive, File, Instance, Param, Stmt, Subckt};

#[derive(Clone, Debug)]
pub struct FormatOptions {
    pub dialect: DialectKind,
    pub max_width: usize,
    pub indent: &'static str,
    pub sort_params: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            dialect: DialectKind::Hspice,
            max_width: 120,
            indent: "",
            sort_params: false,
        }
    }
}

pub fn format_str(input: &str, opts: &FormatOptions) -> String {
    let dialect = crate::dialect::get_dialect(opts.dialect);
    let file = crate::parser::parse_str(input, dialect.clone());
    format_file(&file, opts, dialect.as_ref())
}

pub fn format_file(file: &File, opts: &FormatOptions, dialect: &dyn Dialect) -> String {
    let mut out = String::new();
    let mut first = true;
    let mut prev_was_blank = false;

    for stmt in &file.stmts {
        format_stmt(stmt, &mut out, opts, dialect, 0, &mut first, &mut prev_was_blank);
    }

    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

fn format_stmt(
    stmt: &Stmt,
    out: &mut String,
    opts: &FormatOptions,
    dialect: &dyn Dialect,
    depth: usize,
    first: &mut bool,
    prev_was_blank: &mut bool,
) {
    match stmt {
        Stmt::Blank => {
            if *first || *prev_was_blank {
                return;
            }
            out.push('\n');
            *prev_was_blank = true;
        }
        Stmt::Comment(c) => {
            let line = normalize_comment(c);
            push_line(out, &line, opts, dialect, depth, first, prev_was_blank);
        }
        Stmt::Directive(d) => {
            let line = format_directive(d);
            push_line_wrapped(out, &line, d.inline_comment.as_deref(), opts, dialect, depth, first, prev_was_blank);
        }
        Stmt::Instance(inst) => {
            let line = format_instance(inst);
            push_line_wrapped(out, &line, inst.inline_comment.as_deref(), opts, dialect, depth, first, prev_was_blank);
        }
        Stmt::Subckt(s) => {
            if !*first && !*prev_was_blank {
                out.push('\n');
            }
            let header = format_subckt_header(s);
            push_line_wrapped(out, &header, s.inline_comment.as_deref(), opts, dialect, depth, first, prev_was_blank);
            for inner in &s.body {
                format_stmt(inner, out, opts, dialect, depth + 1, first, prev_was_blank);
            }
            let ends = if s.name.is_empty() {
                ".ends".to_string()
            } else {
                format!(".ends {}", s.name)
            };
            push_line(out, &ends, opts, dialect, depth, first, prev_was_blank);
            *prev_was_blank = false;
            if depth == 0 {
                out.push('\n');
                *prev_was_blank = true;
                *first = false;
            }
        }
    }
}

fn push_line(
    out: &mut String,
    line: &str,
    _opts: &FormatOptions,
    _dialect: &dyn Dialect,
    _depth: usize,
    first: &mut bool,
    prev_was_blank: &mut bool,
) {
    if line.is_empty() {
        return;
    }
    out.push_str(line);
    out.push('\n');
    *first = false;
    *prev_was_blank = false;
}

fn push_line_wrapped(
    out: &mut String,
    line: &str,
    inline_comment: Option<&str>,
    opts: &FormatOptions,
    dialect: &dyn Dialect,
    _depth: usize,
    first: &mut bool,
    prev_was_blank: &mut bool,
) {
    if line.is_empty() {
        return;
    }
    let full = if let Some(c) = inline_comment {
        format!("{line} {c}")
    } else {
        line.to_string()
    };

    let wrapped = wrap_line(&full, opts.max_width, dialect.continuation_indent());
    out.push_str(&wrapped);
    if !wrapped.ends_with('\n') {
        out.push('\n');
    }
    *first = false;
    *prev_was_blank = false;
}

fn wrap_line(line: &str, max_width: usize, cont: &str) -> String {
    if line.len() <= max_width {
        return line.to_string();
    }
    let tokens = tokenize_for_wrap(line);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for tok in tokens {
        let sep = if cur.is_empty() { "" } else { " " };
        let candidate_len = cur.len() + sep.len() + tok.len();
        let limit = if lines.is_empty() { max_width } else { max_width - cont.len() };
        if candidate_len > limit && !cur.is_empty() {
            lines.push(cur);
            cur = tok;
        } else {
            cur.push_str(sep);
            cur.push_str(&tok);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines[0].clone();
    for l in lines.iter().skip(1) {
        out.push('\n');
        out.push_str(cont);
        out.push_str(l);
    }
    out
}

fn tokenize_for_wrap(line: &str) -> Vec<String> {
    let raw: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == "=" && !out.is_empty() && i + 1 < raw.len() {
            let prev = out.pop().unwrap();
            let next = raw[i + 1].clone();
            out.push(format!("{prev} = {next}"));
            i += 2;
        } else if raw[i].ends_with('=') && i + 1 < raw.len() {
            let tok = format!("{} {}", raw[i], raw[i + 1]);
            out.push(tok);
            i += 2;
        } else if i + 1 < raw.len() && raw[i + 1] == "=" {
            if i + 2 < raw.len() {
                out.push(format!("{} = {}", raw[i], raw[i + 2]));
                i += 3;
            } else {
                out.push(raw[i].clone());
                i += 1;
            }
        } else {
            out.push(raw[i].clone());
            i += 1;
        }
    }
    out
}

fn format_directive(d: &Directive) -> String {
    let mut s = format!(".{}", d.name.to_ascii_lowercase());
    for a in &d.args {
        s.push(' ');
        s.push_str(a);
    }
    for p in normalize_params(&d.params, false) {
        s.push(' ');
        s.push_str(&format_param(&p));
    }
    s
}

fn format_instance(inst: &Instance) -> String {
    let mut s = inst.name.clone();
    for n in &inst.nodes {
        s.push(' ');
        s.push_str(n);
    }
    if let Some(m) = &inst.model_or_value {
        s.push(' ');
        s.push_str(m);
    }
    for p in normalize_params(&inst.params, false) {
        s.push(' ');
        s.push_str(&format_param(&p));
    }
    s
}

fn format_subckt_header(s: &Subckt) -> String {
    let mut out = format!(".subckt {}", s.name);
    for p in &s.ports {
        out.push(' ');
        out.push_str(p);
    }
    for param in normalize_params(&s.params, false) {
        out.push(' ');
        out.push_str(&format_param(&param));
    }
    out
}

fn format_param(p: &Param) -> String {
    let v = p.value.trim();
    if v.is_empty() {
        p.key.clone()
    } else if needs_quotes(v) {
        format!("{} = {}", p.key, v)
    } else {
        format!("{} = {}", p.key, v)
    }
}

fn normalize_params(params: &[Param], sort: bool) -> Vec<Param> {
    let mut out: Vec<Param> = params
        .iter()
        .map(|p| Param {
            key: p.key.clone(),
            value: normalize_value(&p.value),
        })
        .collect();
    if sort {
        out.sort_by(|a, b| a.key.cmp(&b.key));
    }
    out
}

fn normalize_value(v: &str) -> String {
    let t = v.trim();
    if (t.starts_with('\'') && t.ends_with('\'')) || (t.starts_with('"') && t.ends_with('"')) {
        return t.to_string();
    }
    t.to_string()
}

fn needs_quotes(_v: &str) -> bool {
    false
}

fn normalize_comment(c: &str) -> String {
    let t = c.trim();
    if t.starts_with('*') {
        let inner = t[1..].trim_start();
        if inner.is_empty() {
            "*".to_string()
        } else {
            format!("* {}", inner)
        }
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(input: &str) -> String {
        format_str(input, &FormatOptions::default())
    }

    #[test]
    fn lowercases_directive() {
        assert_eq!(fmt(".SUBCKT foo a b\n.ENDS\n"), ".subckt foo a b\n.ends foo\n\n");
    }

    #[test]
    fn normalizes_eq() {
        assert_eq!(fmt("R1 a b 1k tc1=1 tc2=2\n"), "R1 a b 1k tc1 = 1 tc2 = 2\n");
    }

    #[test]
    fn joins_continuation() {
        assert_eq!(fmt("R1 a b 1k\n+ tc=1\n"), "R1 a b 1k tc = 1\n");
    }

    #[test]
    fn preserves_positional_value_tail() {
        assert_eq!(fmt("V1 a 0 DC {vssr}\n"), "V1 a 0 DC {vssr}\n");
        assert_eq!(fmt("V2 a b pulse(0 1 1n)\n"), "V2 a b pulse(0 1 1n)\n");
    }
}
