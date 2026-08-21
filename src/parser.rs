use crate::dialect::Dialect;
use crate::ir::{Directive, File, Instance, Param, Stmt, Subckt};
use std::sync::Arc;

pub fn parse_str(input: &str, dialect: Arc<dyn Dialect>) -> File {
    let logical = join_continuations(input, dialect.as_ref());
    let mut stmts: Vec<Stmt> = Vec::new();
    let mut stack: Vec<Subckt> = Vec::new();

    for line in logical {
        let stmt = parse_logical_line(&line, dialect.as_ref());
        match &stmt {
            Stmt::Subckt(s) => {
                if let Some(open) = stack.last_mut() {
                    let sub = s.clone();
                    open.body.push(Stmt::Subckt(sub));
                } else {
                    stack.push(s.clone());
                    continue;
                }
            }
            Stmt::Directive(d) if d.name.eq_ignore_ascii_case("ends") => {
                if let Some(mut sub) = stack.pop() {
                    if let Some(ends_name) = d.args.first()
                        && !ends_name.is_empty()
                        && !ends_name.eq_ignore_ascii_case(&sub.name)
                    {
                        sub.body.push(Stmt::Directive(d.clone()));
                        stack.push(sub);
                        continue;
                    }
                    let stmt = Stmt::Subckt(sub);
                    if let Some(parent) = stack.last_mut() {
                        parent.body.push(stmt);
                    } else {
                        stmts.push(stmt);
                    }
                    continue;
                } else {
                    stmts.push(stmt);
                    continue;
                }
            }
            _ => {}
        }

        if let Some(top) = stack.last_mut() {
            if matches!(stmt, Stmt::Blank) && top.body.is_empty() {
                continue;
            }
            top.body.push(stmt);
        } else {
            stmts.push(stmt);
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

fn join_continuations(input: &str, dialect: &dyn Dialect) -> Vec<String> {
    logical_line_spans(input, dialect)
        .into_iter()
        .map(|(_, _, s)| s)
        .collect()
}

/// Logical lines as `(start_line, end_line_inclusive, text)` spans, 0-based
/// physical line numbers. Continuation lines merge into the preceding
/// statement and extend its span.
pub fn logical_line_spans(input: &str, dialect: &dyn Dialect) -> Vec<(usize, usize, String)> {
    let mut out: Vec<(usize, usize, String)> = Vec::new();
    let cont = dialect.continuation_char();
    for (lineno, raw) in input.lines().enumerate() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with(cont) {
            let rest = trimmed[1..].trim();
            let mut attached = false;
            for idx in (0..out.len()).rev() {
                let candidate = out[idx].2.trim();
                if candidate.is_empty() || dialect.is_comment_line(candidate) {
                    continue;
                }
                if !rest.is_empty() {
                    out[idx].2.push(' ');
                    out[idx].2.push_str(rest);
                }
                out[idx].1 = lineno;
                attached = true;
                break;
            }
            if !attached {
                out.push((lineno, lineno, raw.to_string()));
            }
        } else {
            out.push((lineno, lineno, raw.to_string()));
        }
    }
    out
}

/// All `.subckt NAME` definitions in the file as `(name, 0-based start line)`.
pub fn subckt_definitions(input: &str, dialect: &dyn Dialect) -> Vec<(String, usize)> {
    let mut defs = Vec::new();
    for (start, _, line) in logical_line_spans(input, dialect) {
        let trimmed = line.trim();
        if trimmed.to_ascii_uppercase().starts_with(".SUBCKT") {
            let tokens = tokenize(&trimmed[1 + 6..]);
            if let Some(name) = tokens.first() {
                defs.push((name.clone(), start));
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
            inst.model_or_value
        }
        _ => None,
    }
}

/// File paths referenced by `.include`/`.inc`/`.lib` directives, in order.
/// Surrounding quotes are stripped; `.lib` takes the path as its first arg.
pub fn include_paths(input: &str, dialect: &dyn Dialect) -> Vec<String> {
    let mut out = Vec::new();
    for (_, _, line) in logical_line_spans(input, dialect) {
        if let Stmt::Directive(d) = parse_logical_line(&line, dialect)
            && matches!(d.name.as_str(), "include" | "inc" | "lib")
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

fn parse_logical_line(line: &str, dialect: &dyn Dialect) -> Stmt {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Stmt::Blank;
    }
    if dialect.is_comment_line(trimmed) {
        return Stmt::Comment(trimmed.to_string());
    }

    let (code, inline_comment) = split_inline_comment(line, dialect);

    let code_trim = code.trim();
    if code_trim.is_empty() {
        if let Some(c) = inline_comment {
            return Stmt::Comment(c);
        }
        return Stmt::Blank;
    }

    let upper = code_trim.to_ascii_uppercase();
    if upper.starts_with(".SUBCKT") {
        return parse_subckt(code_trim, inline_comment);
    }
    if upper.starts_with(".ENDS") {
        let args = tokenize(&code_trim[5..]);
        return Stmt::Directive(Directive {
            name: "ends".to_string(),
            args,
            params: Vec::new(),
            inline_comment,
        });
    }

    if code_trim.starts_with(dialect.directive_prefix()) {
        return parse_directive(code_trim, inline_comment, dialect);
    }

    parse_instance(code_trim, inline_comment)
}

fn split_inline_comment<'a>(line: &'a str, dialect: &dyn Dialect) -> (String, Option<String>) {
    let delim = match dialect.inline_comment_delim() {
        Some(c) => c,
        None => return (line.to_string(), None),
    };

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
            if delim == '/' && i + 1 < bytes.len() && bytes[i + 1] != b'/' as u8 {
                continue;
            }
            if delim == '/' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'/' as u8 {
                    let code = line[..i].to_string();
                    let comment = line[i..].trim().to_string();
                    return (code, Some(comment));
                }
                continue;
            }
            let code = line[..i].to_string();
            let comment = line[i..].trim().to_string();
            return (code, Some(comment));
        }
        if delim == '$' && !in_single && !in_double && ch == '$' {
            let code = line[..i].to_string();
            let comment = line[i..].trim().to_string();
            return (code, Some(comment));
        }
        if delim == ';' && !in_single && !in_double && ch == ';' {
            let code = line[..i].to_string();
            let comment = line[i..].trim().to_string();
            return (code, Some(comment));
        }
    }
    (line.to_string(), None)
}

fn parse_directive(line: &str, inline_comment: Option<String>, _dialect: &dyn Dialect) -> Stmt {
    let inner = line[1..].trim();
    let mut tokens = tokenize(inner);
    if tokens.is_empty() {
        return Stmt::Directive(Directive {
            name: String::new(),
            args: Vec::new(),
            params: Vec::new(),
            inline_comment,
        });
    }
    let name = tokens.remove(0).to_ascii_lowercase();
    let (args, params) = split_args_params(tokens);
    Stmt::Directive(Directive {
        name,
        args,
        params,
        inline_comment,
    })
}

fn parse_subckt(line: &str, inline_comment: Option<String>) -> Stmt {
    let inner = if line[1..].trim().to_ascii_uppercase().starts_with("SUBCKT") {
        line[1 + 6..].trim()
    } else {
        line.trim()
    };
    let mut tokens = tokenize(inner);
    if tokens.is_empty() {
        return Stmt::Subckt(Subckt {
            name: String::new(),
            ports: Vec::new(),
            params: Vec::new(),
            body: Vec::new(),
            inline_comment,
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
            ports.push(tok);
        }
    }
    let (_, params) = split_args_params(param_tokens);
    Stmt::Subckt(Subckt {
        name,
        ports,
        params,
        body: Vec::new(),
        inline_comment,
    })
}

fn parse_instance(line: &str, inline_comment: Option<String>) -> Stmt {
    let tokens = tokenize(line);
    if tokens.is_empty() {
        return Stmt::Blank;
    }
    let name = tokens[0].clone();
    let etype = name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or(' ');
    let rest = &tokens[1..];

    if matches!(etype, 'R' | 'C' | 'L') && rest.len() >= 2 {
        let nodes = vec![rest[0].clone(), rest[1].clone()];
        let tail = &rest[2..];
        let (model_or_value, params) = parse_tail_params(tail, &etype.to_string());
        return Stmt::Instance(Instance {
            name,
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
            let m = nodes_and_model.last().cloned();
            let n = nodes_and_model[..nodes_and_model.len() - 1].to_vec();
            (n, m)
        } else if nodes_and_model.len() == 1 {
            (Vec::new(), Some(nodes_and_model[0].clone()))
        } else {
            (Vec::new(), None)
        };
        let (_, params) = split_args_params(param_part.to_vec());
        return Stmt::Instance(Instance {
            name,
            nodes,
            model_or_value: model,
            params,
            inline_comment,
        });
    }

    if name.starts_with('.') {
        return Stmt::Instance(Instance {
            name,
            nodes: Vec::new(),
            model_or_value: None,
            params: Vec::new(),
            inline_comment,
        });
    }

    let node_count = element_node_count(etype);
    let (nodes, tail) = match node_count {
        Some(n) if rest.len() > n => (rest[..n].to_vec(), &rest[n..]),
        _ => {
            let param_start = find_param_start(rest).unwrap_or(rest.len());
            (rest[..param_start.min(rest.len())].to_vec(), &rest[param_start.min(rest.len())..])
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
        let m = Some(tail[0].clone());
        let p = tail[1..]
            .iter()
            .map(|t| Param {
                key: t.clone(),
                value: String::new(),
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
        name,
        nodes,
        model_or_value,
        params,
        inline_comment,
    })
}

fn parse_tail_params(tail: &[String], etype: &str) -> (Option<String>, Vec<Param>) {
    if tail.is_empty() {
        return (None, Vec::new());
    }
    let has_eq = tail.iter().any(|t| t == "=" || t.contains('='));
    if has_eq {
        let is_first_val = !tail[0].contains('=') && tail[0] != "=" && !(tail.len() > 1 && tail[1] == "=");
        if is_first_val {
            let first = tail[0].clone();
            if first.eq_ignore_ascii_case(etype) && tail.len() > 1 {
                let (_, params) = split_args_params(tail[1..].to_vec());
                return (Some(first), params);
            }
            let has_param_eq = tail[1..].iter().any(|t| t == "=" || t.contains('='));
            if has_param_eq {
                let (_, params) = split_args_params(tail[1..].to_vec());
                if !params.is_empty() && params[0].key.eq_ignore_ascii_case(etype) {
                    return (Some(params[0].value.clone()), params[1..].to_vec());
                }
                return (Some(first), params);
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
        return (Some(tail[0].clone()), Vec::new());
    }
    (Some(tail[0].clone()), Vec::new())
}

fn split_args_params(tokens: Vec<String>) -> (Vec<String>, Vec<Param>) {
    let mut args = Vec::new();
    let mut params = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if tok == "=" {
            if let Some(prev) = args.pop() {
                let val = if i + 1 < tokens.len() {
                    i += 1;
                    tokens[i].clone()
                } else {
                    String::new()
                };
                params.push(Param {
                    key: prev,
                    value: val.trim().to_string(),
                });
            }
            i += 1;
            continue;
        }
        if tok.contains('=') {
            let (k, v) = tok.split_once('=').unwrap();
            let key = k.trim().to_string();
            let mut value = v.trim().to_string();
            if value.is_empty() && i + 1 < tokens.len() && tokens[i + 1] != "=" && !tokens[i + 1].contains('=') {
                i += 1;
                value = tokens[i].trim().to_string();
            }
            if key.is_empty() {
                if let Some(prev) = args.pop() {
                    params.push(Param { key: prev, value });
                }
            } else {
                params.push(Param { key, value });
            }
            i += 1;
            continue;
        }
        if !params.is_empty() {
            if i + 1 < tokens.len() && tokens[i + 1] == "=" {
                let key = tok.clone();
                let val = if i + 2 < tokens.len() {
                    tokens[i + 2].clone()
                } else {
                    String::new()
                };
                params.push(Param {
                    key,
                    value: val.trim().to_string(),
                });
                i += 3;
                continue;
            }
            if i + 1 < tokens.len() && tokens[i + 1].contains('=') {
                args.push(tok.clone());
                i += 1;
                continue;
            }
            if tok.contains('=') {
                i += 1;
                continue;
            }
            params.push(Param {
                key: tok.clone(),
                value: String::new(),
            });
            i += 1;
            continue;
        }
        if i + 1 < tokens.len() && tokens[i + 1] == "=" {
            let key = tok.clone();
            let val = if i + 2 < tokens.len() {
                tokens[i + 2].clone()
            } else {
                String::new()
            };
            params.push(Param {
                key,
                value: val.trim().to_string(),
            });
            i += 3;
            continue;
        }
        args.push(tok.clone());
        i += 1;
    }
    (args, params)
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_double {
            in_single = !in_single;
            cur.push(ch);
        } else if ch == '"' && !in_single {
            in_double = !in_double;
            cur.push(ch);
        } else if !in_single && !in_double && ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
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

fn find_param_start(tokens: &[String]) -> Option<usize> {
    for i in 0..tokens.len() {
        let tok = &tokens[i];
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
}
