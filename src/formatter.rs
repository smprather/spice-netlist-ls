use crate::dialect::{Dialect, DialectKind};
use crate::ir::{Directive, File, Instance, Param, Stmt, Subckt};
use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct FormatOptions {
    pub dialect: DialectKind,
    pub max_width: usize,
    pub indent: &'static str,
    pub sort_params: bool,
    /// Ensure the output ends with exactly one `\n`.
    pub insert_final_newline: bool,
    /// Strip trailing whitespace from every emitted line.
    pub trim_trailing_whitespace: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            dialect: DialectKind::Hspice,
            max_width: 120,
            indent: "",
            sort_params: false,
            insert_final_newline: true,
            trim_trailing_whitespace: true,
        }
    }
}

pub fn format_str(input: &str, opts: &FormatOptions) -> String {
    let secs = crate::segments::segments(input, opts.dialect);

    // Fast path: no `simulator lang=` directive anywhere → today's code path,
    // unchanged, guaranteeing byte-identical output for plain decks.
    if secs.len() == 1 && secs[0].header.is_none() {
        let dialect = crate::dialect::get_dialect(opts.dialect);
        let file = crate::parser::parse_str(input, dialect.clone());
        let mut out = String::with_capacity(input.len() + input.len() / 8);
        format_into(&file, &mut out, opts, dialect.as_ref());
        return out;
    }

    // Sectioned path: emit each header verbatim, then its body under the
    // section's dialect. The trailer (trim_trailing_whitespace /
    // insert_final_newline) runs once over the whole output, not per
    // section, so we don't insert/drop a newline at each segment boundary.
    let mut out = String::with_capacity(input.len() + input.len() / 8);
    for sec in &secs {
        if let Some(h) = sec.header {
            out.push_str(h);
            out.push('\n');
        }
        let sub_opts = FormatOptions { dialect: sec.dialect, ..opts.clone() };
        let dialect = crate::dialect::get_dialect(sec.dialect);
        let file = crate::parser::parse_str(sec.body, dialect.clone());
        emit_statements(&file, &mut out, &sub_opts, dialect.as_ref());
    }
    apply_trailer(&mut out, opts);
    out
}

pub fn format_file(file: &File, opts: &FormatOptions, dialect: &dyn Dialect) -> String {
    let mut out = String::new();
    format_into(file, &mut out, opts, dialect);
    out
}

fn format_into(file: &File, out: &mut String, opts: &FormatOptions, dialect: &dyn Dialect) {
    emit_statements(file, out, opts, dialect);
    apply_trailer(out, opts);
}

/// Emit every statement of `file` into `out` without applying the trailer
/// (trim/final-newline). The sectioned formatter calls this per section and
/// runs the trailer once at the end.
fn emit_statements(file: &File, out: &mut String, opts: &FormatOptions, dialect: &dyn Dialect) {
    let mut first = true;
    let mut prev_was_blank = false;
    for stmt in &file.stmts {
        format_stmt(stmt, out, opts, dialect, 0, &mut first, &mut prev_was_blank);
    }
}

/// Apply `trim_trailing_whitespace` and `insert_final_newline` once over the
/// whole output. Running this per section would insert/drop a newline at each
/// segment boundary; the sectioned formatter calls it only at the end.
fn apply_trailer(out: &mut String, opts: &FormatOptions) {
    // The emitter only produces trailing whitespace via exotic input tokens,
    // so probe first; the rebuild is a full copy and skips in the common case.
    if opts.trim_trailing_whitespace && has_trailing_whitespace(out) {
        let mut trimmed = String::with_capacity(out.len());
        for line in out.lines() {
            trimmed.push_str(line.trim_end());
            trimmed.push('\n');
        }
        if !out.ends_with('\n') {
            trimmed.truncate(trimmed.trim_end_matches('\n').len());
        }
        out.clear();
        out.push_str(&trimmed);
    }

    if opts.insert_final_newline {
        if !out.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
    } else {
        let trimmed_len = out.trim_end_matches('\n').len();
        out.truncate(trimmed_len);
    }
}

fn has_trailing_whitespace(s: &str) -> bool {
    s.split('\n').any(|line| line != line.trim_end())
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
            let start = out.len();
            format_directive_into(d, dialect, out);
            emit_wrapped(out, start, d.inline_comment.as_deref(), opts, dialect, first, prev_was_blank);
        }
        Stmt::Instance(inst) => {
            let start = out.len();
            format_instance_into(inst, dialect, out);
            emit_wrapped(out, start, inst.inline_comment.as_deref(), opts, dialect, first, prev_was_blank);
        }
        Stmt::Subckt(s) => {
            if !*first && !*prev_was_blank {
                out.push('\n');
            }
            let start = out.len();
            format_subckt_header_into(s, dialect, out);
            emit_wrapped(out, start, s.inline_comment.as_deref(), opts, dialect, first, prev_was_blank);
            for inner in &s.body {
                format_stmt(inner, out, opts, dialect, depth + 1, first, prev_was_blank);
            }
            let ends_start = out.len();
            out.push_str(".ends");
            if let Some(e) = &s.ends_name {
                out.push(' ');
                out.push_str(e);
            } else if !s.name.is_empty() {
                out.push(' ');
                out.push_str(&s.name);
            }
            if out.len() > ends_start {
                out.push('\n');
                *first = false;
                *prev_was_blank = false;
            }
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

/// Finish a statement body already written into `out` starting at `start`:
/// wrap if it overflows, attach the inline comment, and terminate the line.
/// The common (fits, no comment) case appends nothing but a newline — the
/// body is already in place, so no intermediate `String` is built.
fn emit_wrapped(
    out: &mut String,
    start: usize,
    inline_comment: Option<&str>,
    opts: &FormatOptions,
    dialect: &dyn Dialect,
    first: &mut bool,
    prev_was_blank: &mut bool,
) {
    if out.len() == start {
        return; // empty body — nothing to emit
    }

    let body_len = out.len() - start;
    let needs_wrap = body_len > opts.max_width;

    match inline_comment {
        Some(c) => {
            // Pull the body out so wrap/comment logic can reformat it. This
            // path is rare (inline comments are uncommon), so a copy here is
            // fine.
            let body = out[start..].to_string();
            out.truncate(start);
            let wrapped = wrap_line(&body, opts.max_width, dialect.continuation_indent());
            let wrapped_len = wrapped.rsplit('\n').next().map(str::len).unwrap_or(0);
            let full = if wrapped.contains('\n') {
                format!("{wrapped}\n{}{}", dialect.continuation_indent(), c)
            } else if wrapped_len + 1 + c.len() > opts.max_width {
                // Body fits but the comment would push past the margin —
                // give the comment its own continuation line, prefixed with
                // the dialect's inline-comment delimiter so it stays a
                // comment on the continuation.
                let delim = dialect
                    .inline_comment_delim()
                    .map(|c| if c == '/' { "//".to_string() } else { c.to_string() })
                    .unwrap_or_else(|| "$".to_string());
                let text = c
                    .trim_start_matches(|ch: char| ch == '$' || ch == ';' || ch == '/' || ch.is_whitespace())
                    .trim_start();
                format!("{wrapped}\n{}{delim} {text}", dialect.continuation_indent())
            } else {
                format!("{wrapped} {c}")
            };
            out.push_str(&full);
            if !full.ends_with('\n') {
                out.push('\n');
            }
        }
        None => {
            if needs_wrap {
                // Over-width with no comment: lift the body, wrap, put back.
                let body = out[start..].to_string();
                out.truncate(start);
                let wrapped = wrap_line(&body, opts.max_width, dialect.continuation_indent());
                out.push_str(&wrapped);
            }
            // The body is already in `out`; just terminate the line. `wrap_line`
            // never produces a trailing newline, and neither does the direct
            // writer, so one newline is always correct.
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    *first = false;
    *prev_was_blank = false;
}

/// Wrap at `max_width`, borrowing the input when it already fits (the common
/// case) so short lines are emitted with zero copies.
fn wrap_line<'a>(line: &'a str, max_width: usize, cont: &str) -> Cow<'a, str> {
    if line.len() <= max_width {
        return Cow::Borrowed(line);
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
        return Cow::Borrowed("");
    }
    let mut out = lines[0].clone();
    for l in lines.iter().skip(1) {
        out.push('\n');
        out.push_str(cont);
        out.push_str(l);
    }
    Cow::Owned(out)
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

fn format_directive_into(d: &Directive, dialect: &dyn Dialect, out: &mut String) {
    out.push('.');
    out.push_str(&d.name.to_ascii_lowercase());
    for a in &d.args {
        out.push(' ');
        out.push_str(a);
    }
    for p in normalize_params(&d.params, false) {
        out.push(' ');
        out.push_str(&format_param(&p, dialect));
    }
}

fn format_instance_into(inst: &Instance, dialect: &dyn Dialect, out: &mut String) {
    out.push_str(&inst.name);
    for n in &inst.nodes {
        out.push(' ');
        out.push_str(n);
    }
    if let Some(m) = &inst.model_or_value {
        out.push(' ');
        out.push_str(m);
    }
    for p in normalize_params(&inst.params, false) {
        out.push(' ');
        out.push_str(&format_param(&p, dialect));
    }
}

fn format_subckt_header_into(s: &Subckt, dialect: &dyn Dialect, out: &mut String) {
    out.push_str(".subckt ");
    out.push_str(&s.name);
    for p in &s.ports {
        out.push(' ');
        out.push_str(p);
    }
    for param in normalize_params(&s.params, false) {
        out.push(' ');
        out.push_str(&format_param(&param, dialect));
    }
}

fn format_param<'a>(p: &Param<'a>, dialect: &dyn Dialect) -> Cow<'a, str> {
    // Values are trim-normalized at parse time (and again in `normalize_params`),
    // so no trim is needed here — which lets the empty-value case borrow the
    // key instead of cloning it.
    if p.value.is_empty() {
        p.key.clone()
    } else if dialect.space_around_eq() {
        Cow::Owned(format!("{} = {}", p.key, p.value))
    } else {
        Cow::Owned(format!("{}={}", p.key, p.value))
    }
}

fn normalize_params<'a>(params: &[Param<'a>], sort: bool) -> Vec<Param<'a>> {
    let mut out: Vec<Param<'a>> = params
        .iter()
        .map(|p| {
            // Trim while preserving the borrow: a borrowed value stays borrowed
            // (trim returns a subslice), an owned value is copied.
            let value = match &p.value {
                Cow::Borrowed(b) => Cow::Borrowed(b.trim()),
                Cow::Owned(o) => Cow::Owned(o.trim().to_string()),
            };
            Param { key: p.key.clone(), value }
        })
        .collect();
    if sort {
        out.sort_by(|a, b| a.key.cmp(&b.key));
    }
    out
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

    #[test]
    fn commenting_a_line_does_not_steal_its_continuation() {
        let input = "Rload out gnd 10k\n* Xinv in out inv\n+ w=2u\n";
        let output = fmt(input);
        // Rload must not absorb w=2u; the orphan continuation survives verbatim
        assert!(output.contains("Rload out gnd 10k\n"));
        assert!(!output.contains("Rload out gnd 10k w"));
        assert!(output.contains("+ w = 2u"));
        // idempotent
        assert_eq!(fmt(&output), output);
    }
}
