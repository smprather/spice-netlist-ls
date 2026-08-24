use crate::dialect::Dialect;
use crate::ir::{Directive, File, Instance, Param, Stmt, Subckt};
use crate::starts_with_ci;
use std::borrow::Cow;
use std::sync::Arc;

pub fn parse_str(input: &str, dialect: Arc<dyn Dialect>) -> File<'_> {
    let logical = logical_line_spans(input, dialect.as_ref());
    let mut stmts: Vec<Stmt> = Vec::with_capacity(logical.len());
    let mut stack: Vec<Subckt> = Vec::new();

    for (_, _, line) in &logical {
        // Statements borrow their token text from the input. A logical line
        // assembled from continuations is owned by the span table, so a
        // statement parsed from one must be deep-copied to outlive it —
        // cheap, since continuations are rare next to plain lines.
        let stmt = match line {
            Cow::Borrowed(s) => parse_logical_line(s, dialect.as_ref()),
            Cow::Owned(s) => parse_logical_line(s, dialect.as_ref()).into_owned(),
        };
        match stmt {
            Stmt::Subckt(s) => {
                stack.push(s);
                continue;
            }
            Stmt::Directive(d) if d.name.eq_ignore_ascii_case("ends") => {
                if let Some(mut sub) = stack.pop() {
                    // Simulators close a subckt on any `.ends`; carry a
                    // mismatching name so the formatter emits it verbatim
                    // and the linter can warn.
                    if let Some(ends_name) = d.args.first()
                        && !ends_name.is_empty()
                        && !ends_name.eq_ignore_ascii_case(&sub.name)
                    {
                        sub.ends_name = Some(ends_name.clone());
                    }
                    let stmt = Stmt::Subckt(sub);
                    if let Some(parent) = stack.last_mut() {
                        parent.body.push(stmt);
                    } else {
                        stmts.push(stmt);
                    }
                    continue;
                } else {
                    stmts.push(Stmt::Directive(d));
                    continue;
                }
            }
            stmt => {
                if let Some(top) = stack.last_mut() {
                    if matches!(stmt, Stmt::Blank) && top.body.is_empty() {
                        continue;
                    }
                    top.body.push(stmt);
                } else {
                    stmts.push(stmt);
                }
            }
        }
    }

    while let Some(sub) = stack.pop() {
        if let Some(parent) = stack.last_mut() {
            parent.body.push(Stmt::Subckt(sub));
        } else {
            stmts.push(Stmt::Subckt(sub));
        }
    }

    File::new(stmts)
}

/// Logical lines as `(start_line, end_line_inclusive, text)` spans, 0-based
/// physical line numbers. Continuation lines merge into the preceding
/// statement and extend its span. Blank lines are transparent, but a comment
/// severs attachment: commenting out a statement must not silently re-attach
/// its continuations to an unrelated element above (data corruption). An
/// unattached continuation is kept verbatim — it is not a parse error and
/// the linter flags it (`orphan-continuation`).
///
/// Span text borrows from `input`; only a line that actually absorbs a
/// continuation is copied (the rare case), so large decks parse without a
/// per-line `String` allocation.
pub fn logical_line_spans<'a>(input: &'a str, dialect: &dyn Dialect) -> Vec<(usize, usize, Cow<'a, str>)> {
    let mut out: Vec<(usize, usize, Cow<'a, str>)> = Vec::new();
    let cont = dialect.continuation_char();
    for (lineno, raw) in input.lines().enumerate() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with(cont) && !dialect.is_comment_line(trimmed) {
            let rest = trimmed[1..].trim();
            // Attach to the most recent non-blank, non-comment logical line;
            // a comment between parent and continuation breaks the chain so
            // the continuation lands as an orphan rather than merging with an
            // unrelated statement above.
            let mut attached = false;
            for idx in (0..out.len()).rev() {
                let candidate = out[idx].2.trim();
                if candidate.is_empty() {
                    continue;
                }
                if dialect.is_comment_line(candidate) {
                    break;
                }
                if !rest.is_empty() {
                    let parent = out[idx].2.to_mut();
                    parent.push(' ');
                    parent.push_str(rest);
                }
                out[idx].1 = lineno;
                attached = true;
                break;
            }
            if !attached {
                out.push((lineno, lineno, Cow::Borrowed(raw)));
            }
        } else {
            out.push((lineno, lineno, Cow::Borrowed(raw)));
        }
    }
    out
}

/// All `.subckt NAME` definitions in the file as `(name, 0-based start line)`.
pub fn subckt_definitions(input: &str, dialect: &dyn Dialect) -> Vec<(String, usize)> {
    let mut defs = Vec::new();
    for (start, _, line) in logical_line_spans(input, dialect) {
        let trimmed = line.trim();
        if starts_with_ci(trimmed, ".subckt") {
            let tokens = tokenize(&trimmed[1 + 6..]);
            if let Some(name) = tokens.first() {
                defs.push((name.to_string(), start));
            }
        }
    }
    defs
}

/// If the logical line covering 0-based `line` is a subckt instantiation
/// (`X` instance), return the referenced subckt name.
pub fn subckt_ref_at_line(input: &str, line: usize, dialect: &dyn Dialect) -> Option<String> {
    let (_, _, text) = logical_line_spans(input, dialect)
        .into_iter()
        .find(|(s, e, _)| line >= *s && line <= *e)?;
    match parse_logical_line(&text, dialect) {
        Stmt::Instance(inst)
            if inst.name.chars().next().is_some_and(|c| c.eq_ignore_ascii_case(&'X')) =>
        {
            inst.model_or_value.map(|c| c.into_owned())
        }
        _ => None,
    }
}

/// File paths referenced by `.include`/`.inc`/`.lib` directives, in order.
/// Surrounding quotes are stripped; `.lib` takes the path as its first arg.
pub fn include_paths(input: &str, dialect: &dyn Dialect) -> Vec<String> {
    let mut out = Vec::new();
    for (_, _, line) in logical_line_spans(input, dialect) {
        let trimmed = line.trim();
        // Cheap prefix filter so device cards never reach the full parser.
        let Some(rest) = trimmed.strip_prefix(dialect.directive_prefix()) else {
            continue;
        };
        let word = rest.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("");
        if !(word.eq_ignore_ascii_case("include")
            || word.eq_ignore_ascii_case("inc")
            || word.eq_ignore_ascii_case("lib"))
        {
            continue;
        }
        if let Stmt::Directive(d) = parse_logical_line(trimmed, dialect)
            && matches!(d.name.as_ref(), "include" | "inc" | "lib")
            && let Some(first) = d.args.first()
        {
            let p = first.trim_matches(['"', '\'']);
            if !p.is_empty() {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// One pass collecting both `.subckt` definitions (lowercased name, port
/// count) and `.include`/`.inc`/`.lib` paths, fully parsing only candidate
/// lines. Cheaper than `subckt_definitions` + `include_paths` when both are
/// needed (the linter's include walk).
pub(crate) fn scan_subckt_defs_and_includes(
    input: &str,
    dialect: &dyn Dialect,
) -> (Vec<(String, usize)>, Vec<String>) {
    let mut defs = Vec::new();
    let mut incs = Vec::new();
    for (_, _, line) in logical_line_spans(input, dialect) {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(dialect.directive_prefix()) else {
            continue;
        };
        let word = rest.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("");
        let is_subckt = word.eq_ignore_ascii_case("subckt");
        let is_include = word.eq_ignore_ascii_case("include")
            || word.eq_ignore_ascii_case("inc")
            || word.eq_ignore_ascii_case("lib");
        if !is_subckt && !is_include {
            continue;
        }
        match parse_logical_line(trimmed, dialect) {
            Stmt::Subckt(s) if is_subckt && !s.name.is_empty() => {
                defs.push((s.name.to_ascii_lowercase(), s.ports.len()));
            }
            Stmt::Directive(d)
                if is_include && matches!(d.name.as_ref(), "include" | "inc" | "lib") =>
            {
                if let Some(first) = d.args.first() {
                    let p = first.trim_matches(['"', '\'']);
                    if !p.is_empty() {
                        incs.push(p.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    (defs, incs)
}

pub fn parse_logical_line<'a>(line: &'a str, dialect: &dyn Dialect) -> Stmt<'a> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Stmt::Blank;
    }
    if dialect.is_comment_line(trimmed) {
        return Stmt::Comment(Cow::Borrowed(trimmed));
    }

    let (code, inline_comment) = match inline_comment_start(line, dialect) {
        Some(i) => (&line[..i], Some(Cow::Borrowed(line[i..].trim()))),
        None => (line, None),
    };

    let code_trim = code.trim();
    if code_trim.is_empty() {
        if let Some(c) = inline_comment {
            return Stmt::Comment(c);
        }
        return Stmt::Blank;
    }

    if code_trim.starts_with('.') {
        if starts_with_ci(code_trim, ".subckt") {
            return parse_subckt(code_trim, inline_comment);
        }
        if starts_with_ci(code_trim, ".ends") {
            let args = tokenize(&code_trim[5..]).into_iter().map(Cow::Borrowed).collect();
            return Stmt::Directive(Directive {
                name: Cow::Borrowed("ends"),
                args,
                params: Vec::new(),
                inline_comment,
            });
        }
        return parse_directive(code_trim, inline_comment);
    }

    parse_instance(code_trim, inline_comment)
}

/// Byte offset where the inline comment starts, if the line has one.
fn inline_comment_start(line: &str, dialect: &dyn Dialect) -> Option<usize> {
    let delim = dialect.inline_comment_delim()?;
    // SIMD fast reject: a line without the delimiter byte can never match
    // (`str::contains(char)` lowers to memchr).
    if !line.contains(delim) {
        return None;
    }

    let mut in_single = false;
    let mut in_double = false;
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let ch = b as char;
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            in_double = !in_double;
        }
        if !in_single && !in_double && ch == delim {
            // `$` (HSPICE) and `;` (ngspice/LTspice) only start a comment when
            // preceded by whitespace/`=`/`,`/start-of-line or followed by
            // whitespace/end-of-line — otherwise they are inside a node name
            // like `net;1` or a value like `a;b`.
            let prev_ok = i == 0
                || matches!(bytes[i - 1] as char, c if c.is_whitespace() || c == '=' || c == ',');
            let next_ok = i + 1 >= bytes.len() || (bytes[i + 1] as char).is_whitespace();
            if !(prev_ok || next_ok) {
                continue;
            }
            if delim == '/' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    return Some(i);
                }
                continue;
            }
            return Some(i);
        }
    }
    None
}

fn parse_directive<'a>(line: &'a str, inline_comment: Option<Cow<'a, str>>) -> Stmt<'a> {
    let inner = line[1..].trim();
    let mut tokens = tokenize(inner);
    if tokens.is_empty() {
        return Stmt::Directive(Directive {
            name: Cow::Borrowed(""),
            args: Vec::new(),
            params: Vec::new(),
            inline_comment,
        });
    }
    let name = tokens.remove(0).to_ascii_lowercase();
    let (args, params) = split_args_params(tokens);
    Stmt::Directive(Directive {
        name: Cow::Owned(name),
        args,
        params,
        inline_comment,
    })
}

fn parse_subckt<'a>(line: &'a str, inline_comment: Option<Cow<'a, str>>) -> Stmt<'a> {
    let inner = if starts_with_ci(line[1..].trim(), "subckt") {
        line[1 + 6..].trim()
    } else {
        line.trim()
    };
    let mut tokens = tokenize(inner);
    if tokens.is_empty() {
        return Stmt::Subckt(Subckt {
            name: Cow::Borrowed(""),
            ports: Vec::new(),
            params: Vec::new(),
            body: Vec::new(),
            inline_comment,
            ends_name: None,
        });
    }
    let name = tokens.remove(0);
    let mut ports = Vec::new();
    let mut param_tokens = Vec::new();
    let mut in_params = false;
    for tok in tokens {
        if !in_params && tok.contains('=') {
            in_params = true;
        }
        if in_params {
            param_tokens.push(tok);
        } else {
            ports.push(Cow::Borrowed(tok));
        }
    }
    let (_, params) = split_args_params(param_tokens);
    Stmt::Subckt(Subckt {
        name: Cow::Borrowed(name),
        ports,
        params,
        body: Vec::new(),
        inline_comment,
        ends_name: None,
    })
}

fn parse_instance<'a>(line: &'a str, inline_comment: Option<Cow<'a, str>>) -> Stmt<'a> {
    let tokens = tokenize(line);
    if tokens.is_empty() {
        return Stmt::Blank;
    }
    let name = tokens[0];
    let etype = name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or(' ');
    let rest = &tokens[1..];

    if matches!(etype, 'R' | 'C' | 'L') && rest.len() >= 2 {
        let nodes = vec![Cow::Borrowed(rest[0]), Cow::Borrowed(rest[1])];
        let tail = &rest[2..];
        // Spectre writes `R1 (a b) resistor r=1k` — after tokenization the
        // paren node list lands in a single token "(a b)". Keep the model
        // name when the tail starts with a non-param token.
        let etype_str = match etype {
            'R' => "R",
            'C' => "C",
            _ => "L",
        };
        let (model_or_value, params) = parse_tail_params(tail, etype_str);
        return Stmt::Instance(Instance {
            name: Cow::Borrowed(name),
            nodes,
            model_or_value,
            params,
            inline_comment,
        });
    }

    if etype == 'X' {
        let param_start = find_param_start(rest).unwrap_or(rest.len());
        let (nodes_and_model, param_part) = rest.split_at(param_start);
        let (nodes, model) = if nodes_and_model.len() >= 2 {
            let m = nodes_and_model.last().map(|&s| Cow::Borrowed(s));
            let n = nodes_and_model[..nodes_and_model.len() - 1]
                .iter()
                .map(|&s| Cow::Borrowed(s))
                .collect();
            (n, m)
        } else if nodes_and_model.len() == 1 {
            (Vec::new(), Some(Cow::Borrowed(nodes_and_model[0])))
        } else {
            (Vec::new(), None)
        };
        let (_, params) = split_args_params(param_part.to_vec());
        return Stmt::Instance(Instance {
            name: Cow::Borrowed(name),
            nodes,
            model_or_value: model,
            params,
            inline_comment,
        });
    }

    if name.starts_with('.') {
        return Stmt::Instance(Instance {
            name: Cow::Borrowed(name),
            nodes: Vec::new(),
            model_or_value: None,
            params: Vec::new(),
            inline_comment,
        });
    }

    let node_count = element_node_count(etype);
    let (nodes, tail): (Vec<Cow<'a, str>>, &[&'a str]) = match node_count {
        Some(n) if rest.len() > n => (
            rest[..n].iter().map(|&s| Cow::Borrowed(s)).collect(),
            &rest[n..],
        ),
        _ => {
            let param_start = find_param_start(rest).unwrap_or(rest.len());
            (
                rest[..param_start.min(rest.len())]
                    .iter()
                    .map(|&s| Cow::Borrowed(s))
                    .collect(),
                &rest[param_start.min(rest.len())..],
            )
        }
    };

    let (model_or_value, params) = if tail.is_empty() {
        (None, Vec::new())
    } else if tail[0].contains('=') || tail[0] == "=" || (tail.len() > 1 && tail[1] == "=") {
        let (_, p) = split_args_params(tail.to_vec());
        (None, p)
    } else {
        // Positional value tail (e.g. `V1 a 0 DC {vssr}`): keep every token;
        // tokens after the first become valueless params, which round-trip
        // through the formatter unchanged.
        let m = Some(Cow::Borrowed(tail[0]));
        let p = tail[1..]
            .iter()
            .map(|&t| Param {
                key: Cow::Borrowed(t),
                value: Cow::Borrowed(""),
            })
            .collect();
        (m, p)
    };

    let nodes = if !nodes.is_empty() && nodes.iter().any(|n| n.contains('=')) {
        Vec::new()
    } else {
        nodes
    };

    Stmt::Instance(Instance {
        name: Cow::Borrowed(name),
        nodes,
        model_or_value,
        params,
        inline_comment,
    })
}

fn parse_tail_params<'a>(tail: &[&'a str], etype: &str) -> (Option<Cow<'a, str>>, Vec<Param<'a>>) {
    if tail.is_empty() {
        return (None, Vec::new());
    }
    // `R1 a b = 10k` / `R1 a b =` — a bare "=" tail is a value with an empty
    // key; keep it as the value rather than dropping it (or worse, dropping
    // the whole line's value). Strip leading "=" tokens and re-process so
    // `= 10k` becomes the value and `= 10k tc=1` keeps both.
    if tail[0] == "=" {
        let rest = &tail[1..];
        if rest.is_empty() {
            return (None, Vec::new());
        }
        return parse_tail_params(rest, etype);
    }
    let has_eq = tail.iter().any(|t| *t == "=" || t.contains('='));
    if has_eq {
        let is_first_val = !tail[0].contains('=') && tail[0] != "=" && !(tail.len() > 1 && tail[1] == "=");
        if is_first_val {
            let first = tail[0];
            if first.eq_ignore_ascii_case(etype) && tail.len() > 1 {
                let (_, params) = split_args_params(tail[1..].to_vec());
                return (Some(Cow::Borrowed(first)), params);
            }
            let has_param_eq = tail[1..].iter().any(|t| *t == "=" || t.contains('='));
            if has_param_eq {
                // The leading positional token is the model/value (`resistor`
                // in `R1 a b resistor r=1k`); keep it and treat the rest as
                // params. The special case `R1 a b r=5` (no model name, the
                // param key *is* the canonical param) has first == tail's
                // param key, handled below.
                let (_, params) = split_args_params(tail[1..].to_vec());
                return (Some(Cow::Borrowed(first)), params);
            }
        }
        let (_, params) = split_args_params(tail.to_vec());
        if params.len() == 1 && params[0].key.eq_ignore_ascii_case(etype) {
            return (Some(params[0].value.clone()), Vec::new());
        }
        if !params.is_empty() && params[0].key.eq_ignore_ascii_case(etype) {
            return (Some(params[0].value.clone()), params[1..].to_vec());
        }
        return (None, params);
    }
    if tail.len() == 1 {
        return (Some(Cow::Borrowed(tail[0])), Vec::new());
    }
    (Some(Cow::Borrowed(tail[0])), Vec::new())
}

fn split_args_params<'a>(tokens: Vec<&'a str>) -> (Vec<Cow<'a, str>>, Vec<Param<'a>>) {
    let mut args: Vec<Cow<'a, str>> = Vec::new();
    let mut params: Vec<Param<'a>> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        if tok == "=" {
            // Standalone "=" re-attaches the previous positional arg as a
            // param key; a dangling trailing "=" yields an empty value, which
            // is still better than silently discarding the token.
            if let Some(prev) = args.pop() {
                if i + 1 < tokens.len() && tokens[i + 1] != "=" {
                    i += 1;
                    let value = tokens[i].trim();
                    params.push(Param { key: prev, value: Cow::Borrowed(value) });
                } else {
                    params.push(Param {
                        key: prev,
                        value: Cow::Borrowed(""),
                    });
                }
            }
            i += 1;
            continue;
        }
        if tok.contains('=') {
            let (k, v) = tok.split_once('=').unwrap();
            let key = k.trim();
            let mut value = v.trim();
            if value.is_empty() && i + 1 < tokens.len() && tokens[i + 1] != "=" && !tokens[i + 1].contains('=') {
                i += 1;
                value = tokens[i].trim();
            }
            if key.is_empty() {
                if let Some(prev) = args.pop() {
                    params.push(Param { key: prev, value: Cow::Borrowed(value) });
                }
            } else {
                params.push(Param { key: Cow::Borrowed(key), value: Cow::Borrowed(value) });
            }
            i += 1;
            continue;
        }
        if !params.is_empty() {
            if i + 1 < tokens.len() && tokens[i + 1] == "=" {
                let val = if i + 2 < tokens.len() { tokens[i + 2] } else { "" };
                params.push(Param {
                    key: Cow::Borrowed(tok),
                    value: Cow::Borrowed(val.trim()),
                });
                i += 3;
                continue;
            }
            if i + 1 < tokens.len() && tokens[i + 1].contains('=') {
                args.push(Cow::Borrowed(tok));
                i += 1;
                continue;
            }
            params.push(Param {
                key: Cow::Borrowed(tok),
                value: Cow::Borrowed(""),
            });
            i += 1;
            continue;
        }
        if i + 1 < tokens.len() && tokens[i + 1] == "=" {
            let val = if i + 2 < tokens.len() { tokens[i + 2] } else { "" };
            params.push(Param {
                key: Cow::Borrowed(tok),
                value: Cow::Borrowed(val.trim()),
            });
            i += 3;
            continue;
        }
        args.push(Cow::Borrowed(tok));
        i += 1;
    }
    (args, params)
}

/// Whitespace-split tokens as borrowed slices; quoted spans keep whitespace
/// and their surrounding quotes intact. No per-token allocation.
fn tokenize(s: &str) -> Vec<&str> {
    // Fast path: ASCII-only, quote-free lines need no quote tracking —
    // `split_ascii_whitespace` is exact there (Unicode whitespace and quotes
    // are both non-ASCII-or-quote, so the checks are exhaustive).
    if s.is_ascii() && !s.contains('\'') && !s.contains('"') {
        return s.split_ascii_whitespace().collect();
    }
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut in_single = false;
    let mut in_double = false;
    for (i, ch) in s.char_indices() {
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            in_double = !in_double;
        } else if !in_single && !in_double && ch.is_whitespace() {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
            continue;
        }
        if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

fn element_node_count(etype: char) -> Option<usize> {
    match etype {
        'M' => Some(4),
        'Q' => Some(3),
        'D' => Some(2),
        'J' => Some(3),
        'Z' => Some(3),
        'L' => Some(2),
        'V' => Some(2),
        'I' => Some(2),
        'E' => Some(4),
        'F' => Some(2),
        'G' => Some(4),
        'H' => Some(2),
        'B' => Some(2),
        'S' => Some(4),
        'W' => Some(2),
        'T' => Some(4),
        'O' => Some(4),
        'K' => Some(0),
        _ => None,
    }
}

fn find_param_start(tokens: &[&str]) -> Option<usize> {
    for i in 0..tokens.len() {
        let tok = tokens[i];
        if tok.contains('=') && tok != "=" {
            return Some(i);
        }
        if tok == "=" {
            return Some(i.saturating_sub(1));
        }
        if i + 1 < tokens.len() && tokens[i + 1] == "=" {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::get_dialect;
    use crate::dialect::DialectKind;

    fn hspice() -> std::sync::Arc<dyn Dialect> {
        get_dialect(DialectKind::Hspice)
    }

    #[test]
    fn finds_subckt_definitions_with_spans() {
        let input = "* title\n.subckt inv a b\n+ w=1u\n.ends\nX1 a b inv\n.subckt buf o i\n.ends\n";
        let defs = subckt_definitions(input, hspice().as_ref());
        assert_eq!(
            defs,
            vec![("inv".to_string(), 1), ("buf".to_string(), 5)]
        );
    }

    #[test]
    fn finds_ref_on_instantiation_and_continuation_lines() {
        let input = ".subckt inv a b\n.ends\nX1 a b inv\n+ w=2u\n";
        let d = hspice();
        assert_eq!(
            subckt_ref_at_line(input, 2, d.as_ref()).as_deref(),
            Some("inv")
        );
        assert_eq!(
            subckt_ref_at_line(input, 3, d.as_ref()).as_deref(),
            Some("inv")
        );
    }

    #[test]
    fn no_ref_on_definition_or_element_lines() {
        let input = ".subckt inv a b\n.ends\nR1 a b 1k\n";
        let d = hspice();
        assert_eq!(subckt_ref_at_line(input, 0, d.as_ref()), None);
        assert_eq!(subckt_ref_at_line(input, 2, d.as_ref()), None);
    }

    #[test]
    fn ref_case_insensitive_match() {
        let input = ".subckt INV a b\n.ends\nx1 a b inv\n";
        let defs = subckt_definitions(input, hspice().as_ref());
        let name = subckt_ref_at_line(input, 2, hspice().as_ref()).unwrap();
        assert!(defs.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)));
    }

    #[test]
    fn extracts_include_paths() {
        let input = "\
.INCLUDE \"/abs/path/a.spice\"\n\
.inc rel/b.spice\n\
.lib 'libfile.l' section\n\
X1 a b inv\n\
.model n nmos\n\
.include \"/path with spaces/c.spice\"\n";
        let paths = include_paths(input, hspice().as_ref());
        assert_eq!(
            paths,
            vec![
                "/abs/path/a.spice".to_string(),
                "rel/b.spice".to_string(),
                "libfile.l".to_string(),
                "/path with spaces/c.spice".to_string(),
            ]
        );
    }

    #[test]
    fn ignores_directive_lookalikes() {
        let input = ".include2 x\n.incident y\n";
        assert!(include_paths(input, hspice().as_ref()).is_empty());
    }

    #[test]
    fn comment_severs_continuation_attachment() {
        let input = "Rload out gnd 10k\n* Xinv in out inv\n+ w=2u\n";
        let spans = logical_line_spans(input, hspice().as_ref());
        // Rload unchanged; the + line is orphaned, NOT merged into Rload
        assert_eq!(spans[0].2, "Rload out gnd 10k");
        assert_eq!(spans[2].2, "+ w=2u");
    }

    #[test]
    fn blank_lines_do_not_sever_continuation() {
        let input = "R1 a b 1k\n\n+ tc=1\n";
        let spans = logical_line_spans(input, hspice().as_ref());
        assert_eq!(spans[0].2, "R1 a b 1k tc=1");
    }

    #[test]
    fn adjacent_continuation_still_attaches() {
        let input = "M1 a b c d nch w=1u\n+ ad=0.5p\n";
        let spans = logical_line_spans(input, hspice().as_ref());
        assert_eq!(spans.len(), 1);
        assert!(spans[0].2.contains("ad=0.5p"));
    }
}
