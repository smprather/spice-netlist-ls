use crate::dialect::{Dialect, DialectKind};
use crate::ir::{Directive, File, Instance, Param, Stmt, Subckt};
use std::borrow::Cow;

/// Format rule codes – ruff-inspired `select`/`ignore` names.
/// Keep kebab-case like lint codes for a single `ignore` vocabulary.
pub const RULE_BLANK_AFTER_SUBCKT: &str = "blank-after-subckt";
pub const RULE_BLANK_BEFORE_ENDS: &str = "blank-before-ends";
pub const RULE_BLANK_AFTER_ENDS: &str = "blank-after-ends";
pub const RULE_BLANK_BEFORE_SUBCKT: &str = "blank-before-subckt";
pub const RULE_BLANK_COLLAPSE: &str = "blank-collapse";
pub const RULE_LOWERCASE_DIRECTIVE: &str = "lowercase-directive";
pub const RULE_EQ_SPACING: &str = "eq-spacing";
pub const RULE_CONTINUATION_JOIN: &str = "continuation-join";
pub const RULE_LINE_WRAP: &str = "line-wrap";
pub const RULE_SORT_PARAMS: &str = "sort-params";
pub const RULE_COMMENT_NORMALIZE: &str = "comment-normalize";
pub const RULE_TRIM_TRAILING: &str = "trim-trailing-whitespace";
pub const RULE_FINAL_NEWLINE: &str = "insert-final-newline";
pub const RULE_ENDS_NAME: &str = "ends-name";

/// All known format rules – used for validation and docs.
pub const ALL_FORMAT_RULES: &[&str] = &[
    RULE_LOWERCASE_DIRECTIVE,
    RULE_EQ_SPACING,
    RULE_CONTINUATION_JOIN,
    RULE_LINE_WRAP,
    RULE_SORT_PARAMS,
    RULE_BLANK_BEFORE_SUBCKT,
    RULE_BLANK_AFTER_SUBCKT,
    RULE_BLANK_BEFORE_ENDS,
    RULE_BLANK_AFTER_ENDS,
    RULE_BLANK_COLLAPSE,
    RULE_COMMENT_NORMALIZE,
    RULE_TRIM_TRAILING,
    RULE_FINAL_NEWLINE,
    RULE_ENDS_NAME,
];

/// `fmt: off` / `fmt: on` / `fmt: skip` handling – ruff-style `fmt: off` regions.
/// Recognized in any comment style (`*`, `$`, `;`, `//`) and also `spicefmt: off`.
/// Case-insensitive, `:` optional, whitespace flexible: `* fmt: off`, `*fmt:off`, `* FMT OFF`, etc.
///
/// Scans for `fmt` (or `spicefmt`) followed by optional `: `/whitespace and an
/// `off`/`on`/`skip` word, without allocating.
fn fmt_directive_kind(line: &str) -> Option<&'static str> {
    let b = line.as_bytes();
    let kinds: [(&[u8], &str); 3] = [(b"off", "off"), (b"on", "on"), (b"skip", "skip")];
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i].eq_ignore_ascii_case(&b'f')
            && b[i + 1].eq_ignore_ascii_case(&b'm')
            && b[i + 2].eq_ignore_ascii_case(&b't')
        {
            let mut j = i + 3;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t' || b[j] == b':') {
                j += 1;
            }
            let rest = &b[j..];
            for (pat, name) in &kinds {
                if rest.len() >= pat.len()
                    && rest[..pat.len()].eq_ignore_ascii_case(pat)
                    && (rest.len() == pat.len() || !rest[pat.len()].is_ascii_alphanumeric())
                {
                    return Some(name);
                }
            }
        }
        i += 1;
    }
    None
}

fn is_fmt_off(line: &str) -> bool {
    fmt_directive_kind(line) == Some("off")
}
fn is_fmt_on(line: &str) -> bool {
    fmt_directive_kind(line) == Some("on")
}
fn is_fmt_skip(line: &str) -> bool {
    fmt_directive_kind(line) == Some("skip")
}

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
    /// Disabled format rules (ruff `ignore`). Empty = all enabled.
    pub ignore: Vec<String>,
    /// Allowlist – if non-empty, only these rules are enabled (ruff `select`).
    pub select: Vec<String>,
}

impl FormatOptions {
    pub fn is_enabled(&self, rule: &str) -> bool {
        if !self.select.is_empty() && !self.select.iter().any(|c| c.eq_ignore_ascii_case(rule)) {
            return false;
        }
        !self.ignore.iter().any(|c| c.eq_ignore_ascii_case(rule))
    }
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
            ignore: Vec::new(),
            select: Vec::new(),
        }
    }
}

pub fn format_str(input: &str, opts: &FormatOptions) -> String {
    // fmt: off/on/skip handling – ruff-style region suppression.
    // memchr quick reject: no 'f'/'F' byte means no directive can exist, so
    // plain decks pay nothing for the feature.
    if (input.contains('f') || input.contains('F'))
        && input.lines().any(|l| fmt_directive_kind(l).is_some())
    {
        return format_str_with_fmt(input, opts);
    }
    format_str_inner(input, opts)
}

fn format_str_inner(input: &str, opts: &FormatOptions) -> String {
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

/// Statement-level `fmt: off/on/skip` handling. Parses with per-statement
/// source spans and rewrites disabled/skipped statements into `Verbatim`
/// nodes carrying their exact original text, then formats the result
/// through the normal emitter — so directives inside `.subckt` bodies work,
/// unlike a line-level split (which would flush an open subckt as an empty
/// one). The state carries across `simulator lang=` sections.
struct FmtState {
    enabled: bool,
    skip_next: bool,
}

impl Default for FmtState {
    fn default() -> Self {
        Self { enabled: true, skip_next: false }
    }
}

fn format_str_with_fmt(input: &str, opts: &FormatOptions) -> String {
    let secs = crate::segments::segments(input, opts.dialect);
    let mut out = String::with_capacity(input.len() + input.len() / 8);
    let mut state = FmtState::default();
    let mut saw_verbatim = false;
    for sec in &secs {
        if let Some(h) = sec.header {
            out.push_str(h);
            out.push('\n');
        }
        let sub_opts = FormatOptions { dialect: sec.dialect, ..opts.clone() };
        let dialect = crate::dialect::get_dialect(sec.dialect);
        let (file, spans) = crate::parser::parse_str_spanned(sec.body, dialect.clone());
        let file = rewrite_for_fmt(file, &spans, sec.body, &mut state, &mut saw_verbatim);
        emit_statements(&file, &mut out, &sub_opts, dialect.as_ref());
    }
    // Verbatim regions are byte-faithful: skip the trailing-whitespace trim
    // when any were emitted; only the final-newline rule still applies.
    if saw_verbatim {
        apply_final_newline(&mut out, opts);
    } else {
        apply_trailer(&mut out, opts);
    }
    out
}

fn rewrite_for_fmt<'a>(
    file: File<'a>,
    spans: &[(usize, usize)],
    input: &str,
    state: &mut FmtState,
    saw_verbatim: &mut bool,
) -> File<'a> {
    let lines: Vec<&str> = input.split('\n').collect();
    let mut idx = 0;
    let stmts = rewrite_stmts(file.stmts, spans, &mut idx, &lines, state, saw_verbatim);
    File { stmts }
}

fn rewrite_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    spans: &[(usize, usize)],
    idx: &mut usize,
    lines: &[&str],
    state: &mut FmtState,
    saw_verbatim: &mut bool,
) -> Vec<Stmt<'a>> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        let (start, end) = spans[*idx];
        *idx += 1;
        let mut verbatim = |start: usize, end: Option<usize>| -> Stmt<'a> {
            *saw_verbatim = true;
            let text = match end {
                Some(e) => lines[start..=e].join("\n"),
                None => {
                    let slice = &lines[start..];
                    let slice = if slice.last() == Some(&"") {
                        &slice[..slice.len() - 1]
                    } else {
                        slice
                    };
                    slice.join("\n")
                }
            };
            Stmt::Verbatim(Cow::Owned(text))
        };

        // fmt directives act on any standalone comment-style line, whatever
        // the dialect thinks it is (e.g. `; fmt: off` under HSPICE), and on
        // bare directive lines (`spicefmt: off` with no comment marker).
        let first = lines.get(start).copied().unwrap_or("");
        match parse_fmt_directive(first) {
            Some("off") => {
                state.enabled = false;
                out.push(verbatim(start, Some(end)));
                continue;
            }
            Some("on") => {
                state.enabled = true;
                out.push(verbatim(start, Some(end)));
                continue;
            }
            Some("skip") => {
                state.skip_next = true;
                out.push(verbatim(start, Some(end)));
                continue;
            }
            _ => {}
        }
        if is_standalone_comment_line(first) {
            if is_fmt_off(first) {
                state.enabled = false;
                out.push(verbatim(start, Some(end)));
                continue;
            }
            if is_fmt_on(first) {
                state.enabled = true;
                out.push(verbatim(start, Some(end)));
                continue;
            }
            if is_fmt_skip(first) {
                state.skip_next = true;
                out.push(verbatim(start, Some(end)));
                continue;
            }
        }

        if let Stmt::Subckt(mut sub) = stmt {
            // Whole block verbatim when the header is disabled/skipped, or
            // the header carries an inline `fmt: skip`.
            let inline_skip = sub.inline_comment.as_deref().is_some_and(is_fmt_skip);
            if state.skip_next || inline_skip || !state.enabled {
                state.skip_next = false;
                *idx += count_spans(&sub.body);
                let (_, ends_end) = spans[*idx];
                *idx += 1;
                // EOF-closed subckts carry a sentinel span: verbatim to EOF.
                let end = (ends_end != usize::MAX).then_some(ends_end);
                out.push(verbatim(start, end));
                continue;
            }
            // Enabled header: rewrite the body, then decide the `.ends` line.
            let body = rewrite_stmts(sub.body, spans, idx, lines, state, saw_verbatim);
            let (es, ee) = spans[*idx];
            *idx += 1;
            sub.body = body;
            if !state.enabled && es != usize::MAX {
                sub.ends_raw = Some(Cow::Owned(lines[es..=ee].join("\n")));
                *saw_verbatim = true;
            }
            out.push(Stmt::Subckt(sub));
            continue;
        }

        if state.skip_next {
            if matches!(stmt, Stmt::Blank) {
                // Blank lines don't consume a pending skip.
                out.push(Stmt::Blank);
                continue;
            }
            state.skip_next = false;
            out.push(verbatim(start, Some(end)));
            continue;
        }

        let is_blank = matches!(stmt, Stmt::Blank);
        // Inline `$ fmt: skip` disables only the statement it's attached to.
        let inline_skip = stmt_inline_comment(&stmt).is_some_and(is_fmt_skip);
        if !state.enabled || inline_skip {
            if is_blank {
                out.push(Stmt::Blank);
            } else {
                out.push(verbatim(start, Some(end)));
            }
            continue;
        }
        out.push(stmt);
    }
    out
}

/// Number of span slots a statement list occupies in `parse_str_spanned`
/// order: one per statement, plus one for each subckt's `.ends` line.
fn count_spans(stmts: &[Stmt<'_>]) -> usize {
    stmts.iter()
        .map(|s| match s {
            Stmt::Subckt(sub) => 1 + count_spans(&sub.body) + 1,
            _ => 1,
        })
        .sum()
}

fn stmt_inline_comment<'a>(stmt: &'a Stmt<'a>) -> Option<&'a str> {
    match stmt {
        Stmt::Directive(d) => d.inline_comment.as_deref(),
        Stmt::Instance(i) => i.inline_comment.as_deref(),
        Stmt::Subckt(s) => s.inline_comment.as_deref(),
        _ => None,
    }
}

fn is_standalone_comment_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('*') || t.starts_with("//") || t.starts_with(';') || t.starts_with('$')
}

/// Recognize a bare `fmt: off` / `spicefmt: on` line — no comment marker.
/// Returns `off`/`on`/`skip` when the whole trimmed line is the directive.
fn parse_fmt_directive(line: &str) -> Option<&'static str> {
    let lower = line.trim().to_ascii_lowercase();
    let body = lower
        .strip_prefix("spicefmt")
        .or_else(|| lower.strip_prefix("fmt"))?;
    let body = body.trim_start();
    let body = body.strip_prefix(':').unwrap_or(body).trim();
    match body {
        "off" => Some("off"),
        "on" => Some("on"),
        "skip" => Some("skip"),
        _ => None,
    }
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
    let mut prev_was_comment = false;
    for stmt in &file.stmts {
        format_stmt(
            stmt,
            out,
            opts,
            dialect,
            0,
            &mut first,
            &mut prev_was_blank,
            &mut prev_was_comment,
        );
    }
}

/// Apply `trim_trailing_whitespace` and `insert_final_newline` once over the
/// whole output. Running this per section would insert/drop a newline at each
/// segment boundary; the sectioned formatter calls it only at the end.
fn apply_trailer(out: &mut String, opts: &FormatOptions) {
    apply_trim(out, opts);
    apply_final_newline(out, opts);
}

/// Strip trailing whitespace from every line. The emitter only produces
/// trailing whitespace via exotic input tokens, so probe first; the rebuild
/// is a full copy and skips in the common case.
fn apply_trim(out: &mut String, opts: &FormatOptions) {
    if opts.trim_trailing_whitespace
        && opts.is_enabled(RULE_TRIM_TRAILING)
        && has_trailing_whitespace(out)
    {
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
}

fn apply_final_newline(out: &mut String, opts: &FormatOptions) {
    if opts.is_enabled(RULE_FINAL_NEWLINE) {
        if opts.insert_final_newline {
            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
        } else {
            let trimmed_len = out.trim_end_matches('\n').len();
            out.truncate(trimmed_len);
        }
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
    prev_was_comment: &mut bool,
) {
    match stmt {
        Stmt::Blank => {
            if *first {
                return;
            }
            if *prev_was_blank && opts.is_enabled(RULE_BLANK_COLLAPSE) {
                return;
            }
            // When blank-collapse is disabled, we still prevent leading blank
            // at file start, but allow consecutive blanks to be preserved.
            out.push('\n');
            *prev_was_blank = true;
            *prev_was_comment = false;
        }
        Stmt::Comment(c) => {
            let line = if opts.is_enabled(RULE_COMMENT_NORMALIZE) {
                normalize_comment(c)
            } else {
                c.trim().to_string()
            };
            push_line(out, &line, opts, dialect, depth, first, prev_was_blank);
            *prev_was_comment = true;
        }
        Stmt::Directive(d) => {
            let start = out.len();
            format_directive_into(d, dialect, out, opts);
            emit_wrapped(out, start, d.inline_comment.as_deref(), opts, dialect, first, prev_was_blank);
            *prev_was_comment = false;
        }
        Stmt::Instance(inst) => {
            let start = out.len();
            format_instance_into(inst, dialect, out, opts);
            emit_wrapped(out, start, inst.inline_comment.as_deref(), opts, dialect, first, prev_was_blank);
            *prev_was_comment = false;
        }
        Stmt::Verbatim(text) => {
            // fmt: off/on/skip regions: source text preserved byte-for-byte
            // (except the global final-newline rule). Bypasses blank rules.
            out.push_str(text);
            out.push('\n');
            *first = false;
            *prev_was_blank = false;
            *prev_was_comment = false;
        }
        Stmt::Subckt(s) => {
            // Blank before subckt: top-level uses blank-before-subckt, nested
            // uses blank-after-subckt (blank after parent header = blank before
            // child). This matches the three user rules while keeping the
            // existing readability blank before top-level subckts.
            let add_blank_before = if !*first && !*prev_was_blank && !*prev_was_comment {
                if depth == 0 {
                    opts.is_enabled(RULE_BLANK_BEFORE_SUBCKT)
                } else {
                    // nested: blank before child is blank after parent header
                    !opts.is_enabled(RULE_BLANK_AFTER_SUBCKT)
                }
            } else {
                false
            };
            if add_blank_before {
                out.push('\n');
            }
            let start = out.len();
            format_subckt_header_into(s, dialect, out, opts);
            emit_wrapped(out, start, s.inline_comment.as_deref(), opts, dialect, first, prev_was_blank);
            // Apply blank-line rules to the subckt body.
            // - blank-after-subckt: no empty line after .subckt header
            // - blank-before-ends: no empty line before .ends
            let body: Vec<&Stmt> = {
                let mut v: Vec<&Stmt> = s.body.iter().collect();
                if opts.is_enabled(RULE_BLANK_AFTER_SUBCKT) {
                    while matches!(v.first(), Some(Stmt::Blank)) {
                        v.remove(0);
                    }
                }
                if opts.is_enabled(RULE_BLANK_BEFORE_ENDS) {
                    while matches!(v.last(), Some(Stmt::Blank)) {
                        v.pop();
                    }
                }
                v
            };
            for inner in body {
                format_stmt(
                    inner,
                    out,
                    opts,
                    dialect,
                    depth + 1,
                    first,
                    prev_was_blank,
                    prev_was_comment,
                );
            }
            let ends_start = out.len();
            if let Some(raw) = &s.ends_raw {
                // `.ends` line inside a fmt: off/skip region – verbatim.
                out.push_str(raw);
            } else {
                out.push_str(".ends");
                if opts.is_enabled(RULE_ENDS_NAME) {
                    if !s.name.is_empty() {
                        out.push(' ');
                        out.push_str(&s.name);
                    }
                } else if let Some(e) = &s.ends_name {
                    out.push(' ');
                    out.push_str(e);
                } else if !s.name.is_empty() {
                    out.push(' ');
                    out.push_str(&s.name);
                }
            }
            if out.len() > ends_start {
                out.push('\n');
                *first = false;
                *prev_was_blank = false;
            }
            *prev_was_blank = false;
            *prev_was_comment = false;
            // blank-after-ends: at least one empty line after .ends (depth 0 only)
            if opts.is_enabled(RULE_BLANK_AFTER_ENDS) && depth == 0 {
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
    let needs_wrap = body_len > opts.max_width && opts.is_enabled(RULE_LINE_WRAP);

    match inline_comment {
        Some(c) => {
            // Pull the body out so wrap/comment logic can reformat it. This
            // path is rare (inline comments are uncommon), so a copy here is
            // fine.
            let body = out[start..].to_string();
            out.truncate(start);
            let wrapped = if opts.is_enabled(RULE_LINE_WRAP) {
                wrap_line(&body, opts.max_width, dialect.continuation_indent())
            } else {
                std::borrow::Cow::Borrowed(body.as_str())
            };
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

fn format_directive_into(d: &Directive, dialect: &dyn Dialect, out: &mut String, opts: &FormatOptions) {
    out.push('.');
    if opts.is_enabled(RULE_LOWERCASE_DIRECTIVE) {
        out.push_str(&d.name.to_ascii_lowercase());
    } else {
        out.push_str(&d.name);
    }
    for a in &d.args {
        out.push(' ');
        out.push_str(a);
    }
    for p in normalize_params(&d.params, false, opts) {
        out.push(' ');
        out.push_str(&format_param(&p, dialect, opts));
    }
}

fn format_instance_into(inst: &Instance, dialect: &dyn Dialect, out: &mut String, opts: &FormatOptions) {
    out.push_str(&inst.name);
    for n in &inst.nodes {
        out.push(' ');
        out.push_str(n);
    }
    if let Some(m) = &inst.model_or_value {
        out.push(' ');
        out.push_str(m);
    }
    for p in normalize_params(&inst.params, false, opts) {
        out.push(' ');
        out.push_str(&format_param(&p, dialect, opts));
    }
}

fn format_subckt_header_into(s: &Subckt, dialect: &dyn Dialect, out: &mut String, opts: &FormatOptions) {
    if opts.is_enabled(RULE_LOWERCASE_DIRECTIVE) {
        out.push_str(".subckt ");
    } else {
        out.push_str(".subckt ");
    }
    out.push_str(&s.name);
    for p in &s.ports {
        out.push(' ');
        out.push_str(p);
    }
    for param in normalize_params(&s.params, false, opts) {
        out.push(' ');
        out.push_str(&format_param(&param, dialect, opts));
    }
}

fn format_param<'a>(p: &Param<'a>, dialect: &dyn Dialect, opts: &FormatOptions) -> Cow<'a, str> {
    // Values are trim-normalized at parse time (and again in `normalize_params`),
    // so no trim is needed here — which lets the empty-value case borrow the
    // key instead of cloning it.
    if p.value.is_empty() {
        p.key.clone()
    } else if !opts.is_enabled(RULE_EQ_SPACING) {
        // opt-out: preserve dialect-agnostic single-space form
        Cow::Owned(format!("{} = {}", p.key, p.value))
    } else if dialect.space_around_eq() {
        Cow::Owned(format!("{} = {}", p.key, p.value))
    } else {
        Cow::Owned(format!("{}={}", p.key, p.value))
    }
}

fn normalize_params<'a>(params: &[Param<'a>], sort: bool, opts: &FormatOptions) -> Vec<Param<'a>> {
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
    if sort && opts.is_enabled(RULE_SORT_PARAMS) {
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

    #[test]
    fn no_blank_after_subckt() {
        let input = ".subckt a b\n\nR1 a b 1k\n.ends a\n";
        assert_eq!(fmt(input), ".subckt a b\nR1 a b 1k\n.ends a\n\n");
        // also nested
        let nested = ".subckt outer a b\n\n.subckt inner c d\nR1 c d 1k\n.ends inner\n.ends outer\n";
        assert_eq!(
            fmt(nested),
            ".subckt outer a b\n.subckt inner c d\nR1 c d 1k\n.ends inner\n.ends outer\n\n"
        );
    }

    #[test]
    fn no_blank_before_ends() {
        let input = ".subckt a b\nR1 a b 1k\n\n.ends a\n";
        assert_eq!(fmt(input), ".subckt a b\nR1 a b 1k\n.ends a\n\n");
    }

    #[test]
    fn blank_after_ends() {
        let input = ".subckt a b\nR1 a b 1k\n.ends a\nX1 a b a\n";
        assert_eq!(fmt(input), ".subckt a b\nR1 a b 1k\n.ends a\n\nX1 a b a\n");
        // multiple blanks collapse to one
        let multi = ".subckt a b\nR1 a b 1k\n.ends a\n\n\nX1 a b a\n";
        assert_eq!(fmt(multi), ".subckt a b\nR1 a b 1k\n.ends a\n\nX1 a b a\n");
    }

    #[test]
    fn blank_rules_opt_out_via_ignore() {
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_BLANK_AFTER_SUBCKT.to_string());
        let input = ".subckt a b\n\nR1 a b 1k\n.ends a\n";
        // with ignore, blank after subckt is preserved
        assert_eq!(
            format_str(input, &opts),
            ".subckt a b\n\nR1 a b 1k\n.ends a\n\n"
        );

        let mut opts2 = FormatOptions::default();
        opts2.ignore.push(RULE_BLANK_BEFORE_ENDS.to_string());
        let input2 = ".subckt a b\nR1 a b 1k\n\n.ends a\n";
        assert_eq!(
            format_str(input2, &opts2),
            ".subckt a b\nR1 a b 1k\n\n.ends a\n\n"
        );

        let mut opts3 = FormatOptions::default();
        opts3.ignore.push(RULE_BLANK_AFTER_ENDS.to_string());
        let input3 = ".subckt a b\nR1 a b 1k\n.ends a\nX1 a b a\n";
        assert_eq!(
            format_str(input3, &opts3),
            ".subckt a b\nR1 a b 1k\n.ends a\nX1 a b a\n"
        );
    }

    #[test]
    fn blank_rules_select_allowlist() {
        let mut opts = FormatOptions::default();
        opts.select.push(RULE_BLANK_AFTER_SUBCKT.to_string());
        // only blank-after-subckt enabled, others disabled
        let input = ".subckt a b\nR1 a b 1k\n\n.ends a\nX1 a b a\n";
        // blank before .ends should be preserved because that rule is not selected
        // blank after .ends should also be preserved (no insertion)
        // but current input has blank before .ends (violation) and missing blank after .ends
        // with only after-subckt enabled, output should keep those
        let out = format_str(input, &opts);
        assert!(out.contains("R1 a b 1k\n\n.ends a\nX1"), "select allowlist failed: {}", out);
    }

    #[test]
    fn lowercase_directive_opt_out() {
        let input = ".SUBCKT foo a b\n.ENDS\n";
        assert_eq!(fmt(input), ".subckt foo a b\n.ends foo\n\n");
        // Use a plain directive (not subckt) where original case is preserved
        let input2 = ".PARAM foo=1\n";
        assert_eq!(fmt(input2), ".param foo = 1\n");
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_LOWERCASE_DIRECTIVE.to_string());
        assert_eq!(format_str(input2, &opts), ".PARAM foo = 1\n");
    }

    #[test]
    fn eq_spacing_per_dialect() {
        // HSPICE expects spaces
        assert_eq!(fmt("R1 a b 1k tc1=1\n"), "R1 a b 1k tc1 = 1\n");
        // opt-out still produces spaced (dialect-agnostic) – at least not Spectre style
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_EQ_SPACING.to_string());
        assert_eq!(
            format_str("R1 a b 1k tc1=1\n", &opts),
            "R1 a b 1k tc1 = 1\n"
        );
        // Spectre expects no spaces
        let mut spectre_opts = FormatOptions { dialect: crate::dialect::DialectKind::Spectre, ..Default::default() };
        assert_eq!(
            format_str("R1 (a b) resistor r=1k\n", &spectre_opts),
            "R1 (a b) resistor r=1k\n"
        );
    }

    #[test]
    fn comment_normalize_opt_out() {
        assert_eq!(fmt("*foo\n"), "* foo\n");
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_COMMENT_NORMALIZE.to_string());
        assert_eq!(format_str("*foo\n", &opts), "*foo\n");
        assert_eq!(format_str("*   foo\n", &opts), "*   foo\n");
    }

    #[test]
    fn blank_before_subckt() {
        // top-level subckt should have blank before unless previous is comment/first
        let input = "R1 a b 1k\n.subckt foo a b\nR2 c d 1k\n.ends foo\n";
        assert_eq!(
            fmt(input),
            "R1 a b 1k\n\n.subckt foo a b\nR2 c d 1k\n.ends foo\n\n"
        );
        // comment before subckt: no extra blank
        let input2 = "* comment\n.subckt foo a b\nR2 c d 1k\n.ends foo\n";
        assert_eq!(fmt(input2), "* comment\n.subckt foo a b\nR2 c d 1k\n.ends foo\n\n");
        // opt-out
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_BLANK_BEFORE_SUBCKT.to_string());
        assert_eq!(
            format_str(input, &opts),
            "R1 a b 1k\n.subckt foo a b\nR2 c d 1k\n.ends foo\n\n"
        );
    }

    #[test]
    fn blank_collapse() {
        let input = "R1 a b 1k\n\n\nR2 c d 1k\n";
        assert_eq!(fmt(input), "R1 a b 1k\n\nR2 c d 1k\n");
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_BLANK_COLLAPSE.to_string());
        assert_eq!(format_str(input, &opts), "R1 a b 1k\n\n\nR2 c d 1k\n");
    }

    #[test]
    fn trim_and_final_newline() {
        let input = "R1 a b 1k   \n";
        assert_eq!(fmt(input), "R1 a b 1k\n");
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_TRIM_TRAILING.to_string());
        // parser already trims trailing spaces on instances, so even with
        // trim disabled the output is still trimmed – the rule controls only
        // the final trailer pass, not per-statement trimming at parse time
        assert_eq!(format_str(input, &opts), "R1 a b 1k\n");
        let input2 = "R1 a b 1k";
        assert_eq!(fmt(input2), "R1 a b 1k\n");
        let mut opts2 = FormatOptions::default();
        opts2.insert_final_newline = false;
        assert_eq!(format_str(input2, &opts2), "R1 a b 1k");
        // final-newline opt-out: even with insert_final_newline false, when rule disabled, no truncation
        let mut opts3 = FormatOptions::default();
        opts3.insert_final_newline = false;
        opts3.ignore.push(RULE_FINAL_NEWLINE.to_string());
        // input with trailing newline, when final-newline disabled, should keep it
        let input3 = "R1 a b 1k\n";
        assert_eq!(format_str(input3, &opts3), "R1 a b 1k\n");
    }

    #[test]
    fn line_wrap_opt_out() {
        let long = format!("R1 a b {} {}\n", "1k", "tc1 = 1 ".repeat(30));
        let wrapped = fmt(&long);
        assert!(wrapped.contains('\n'), "should wrap");
        let mut opts = FormatOptions::default();
        opts.ignore.push(RULE_LINE_WRAP.to_string());
        let unwrapped = format_str(&long, &opts);
        assert!(!unwrapped.contains("\n+"), "line-wrap disabled should not add continuation");
    }

    // ---------- fmt: off/on/skip ----------

    #[test]
    fn fmt_off_region_is_verbatim() {
        // The `.SUBCKT` header is inside the off region, so the whole block
        // (including its inner `fmt: on`) stays verbatim.
        let input = "* fmt: off\n.SUBCKT badly_formatted    a   b\nR1    a   b    1k\n* fmt: on\n.subckt good a b\nR1 a b 1k\n.ends good\n";
        assert_eq!(fmt(input), input);
        // idempotent: the directives re-apply on the second pass
        assert_eq!(fmt(&fmt(input)), fmt(input));
    }

    #[test]
    fn fmt_off_inside_subckt_body() {
        let input = ".subckt bad a b\n* fmt: off\nR1    a   b   1k\n* fmt: on\n.ends bad\nX1 p bad\n";
        let expected = ".subckt bad a b\n* fmt: off\nR1    a   b   1k\n* fmt: on\n.ends bad\n\nX1 p bad\n";
        assert_eq!(fmt(input), expected);
        assert_eq!(fmt(&expected), expected);
    }

    #[test]
    fn fmt_skip_inside_subckt_body() {
        // regression: the skipped statement must stay inside the subckt
        let input = ".subckt bad a b\n* fmt: skip\nR1    a   b   1k\n.ends bad\nX1 p bad\n";
        let expected = ".subckt bad a b\n* fmt: skip\nR1    a   b   1k\n.ends bad\n\nX1 p bad\n";
        assert_eq!(fmt(input), expected);
        assert_eq!(fmt(&expected), expected);
    }

    #[test]
    fn fmt_skip_before_subckt_skips_whole_block() {
        let input = "* fmt: skip\n.SUBCKT bad    a   b\nR1    a   b   1k\n.ENDS bad\nR2 c d 2k\n";
        let out = fmt(input);
        assert!(out.contains(".SUBCKT bad    a   b\nR1    a   b   1k\n.ENDS bad\n"), "block not verbatim: {out}");
        assert!(out.contains("R2 c d 2k\n"), "following stmt not formatted: {out}");
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn fmt_skip_inline_disables_one_statement() {
        let input = "R1 a b 1k $ fmt: skip\nR2   c   d   2k\n* fmt: skip\nR3 e f 3k\n";
        let expected = "R1 a b 1k $ fmt: skip\nR2 c d 2k\n* fmt: skip\nR3 e f 3k\n";
        assert_eq!(fmt(input), expected);
        assert_eq!(fmt(&expected), expected);
    }

    #[test]
    fn fmt_off_covering_ends_line() {
        let input = ".subckt a b\nR1 a b 1k\n* fmt: off\n.ends a\n* fmt: on\n";
        let expected = ".subckt a b\nR1 a b 1k\n* fmt: off\n.ends a\n\n* fmt: on\n";
        assert_eq!(fmt(input), expected);
        assert_eq!(fmt(&expected), expected);
    }

    #[test]
    fn fmt_directive_syntax_variants() {
        // case-insensitive, `:` optional, `spicefmt:` prefix
        assert_eq!(
            fmt("*FMT OFF\nR1   a   b   1k\n*fmt:on\nR2 c d 2k\n"),
            "*FMT OFF\nR1   a   b   1k\n*fmt:on\nR2 c d 2k\n"
        );
        assert_eq!(
            fmt("spicefmt: off\nR1   a   b   1k\nspicefmt: on\nR2 c d 2k\n"),
            "spicefmt: off\nR1   a   b   1k\nspicefmt: on\nR2 c d 2k\n"
        );
        // non-comment-style directives under the outer dialect
        assert_eq!(
            fmt("; fmt: off\nR1   a   b   1k\n; fmt: on\nR2 c d 2k\n"),
            "; fmt: off\nR1   a   b   1k\n; fmt: on\nR2 c d 2k\n"
        );
    }

    #[test]
    fn fmt_off_to_eof_keeps_rest_verbatim() {
        let input = "* fmt: off\nR1   a   b   1k\n";
        assert_eq!(fmt(input), "* fmt: off\nR1   a   b   1k\n");
    }

    #[test]
    fn fmt_off_preserves_trailing_whitespace() {
        let input = "R1 a b 1k   \n* fmt: off\nR2 c d 2k   \n* fmt: on\n";
        let expected = "R1 a b 1k\n* fmt: off\nR2 c d 2k   \n* fmt: on\n";
        assert_eq!(fmt(input), expected);
    }
}
