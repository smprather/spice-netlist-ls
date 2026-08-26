//! Single-file rename support: nets (instance nodes, `.subckt` ports) and
//! parameter keys (`key=value` on instances/directives/subckt headers).
//! Pure functions here; `src/bin/ls.rs` wires them to `textDocument/rename`
//! and `textDocument/prepareRename`.
//!
//! Positions are `(line, byte column)` in whole-file coordinates, ASCII-safe
//! (netlists are ASCII; byte columns equal UTF-16 columns there).

use crate::dialect::Dialect;

/// One replacement: `line:start_col..end_col` becomes `text`.
pub struct Edit {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Net,
    ParamKey,
}

/// A renameable position within one logical statement: the global logical
/// token index plus the byte range of the renameable part inside that token
/// (`key=value` keys rename only the key; Spectre paren nodes `(a` rename
/// only the name).
struct Candidate {
    role: Role,
    token: usize,
    within_start: usize,
    len: usize,
}

fn net(tokens: &[&str], i: usize) -> Candidate {
    let tok = tokens[i];
    let rest = tok.strip_prefix('(').unwrap_or(tok);
    let name = rest.strip_suffix(')').unwrap_or(rest);
    Candidate { role: Role::Net, token: i, within_start: tok.len() - rest.len(), len: name.len() }
}

fn valid_new_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '=' | '"' | '\'' | '{' | '}'))
}

/// Rename every occurrence of the net/param under `(line, col)` in `text`
/// to `new_name`. `None` when the cursor is not on a net or param key or
/// `new_name` is not a valid name.
pub fn rename_edits(text: &str, line: usize, col: usize, new_name: &str) -> Option<Vec<Edit>> {
    if !valid_new_name(new_name) {
        return None;
    }
    let fallback = crate::detect::detect_dialect(text);
    let secs = crate::segments::segments(text, fallback);
    let idx = crate::segments::section_index_at_line(&secs, line)?;
    let sec = &secs[idx];
    let dialect = crate::dialect::get_dialect(sec.dialect);
    let within = line - sec.line_offset;
    let (target, role) = word_at(sec.body, within, col, dialect.as_ref())?;
    let mut edits = Vec::new();
    for sec in &secs {
        let sub = crate::dialect::get_dialect(sec.dialect);
        collect_edits(sec.body, sec.line_offset, &target, role, sub.as_ref(), new_name, &mut edits);
    }
    Some(edits)
}

/// Range of the renameable word under `(line, col)`, or `None` when the
/// cursor is not on a net or param key.
pub fn prepare_rename(text: &str, line: usize, col: usize) -> Option<(usize, usize, usize)> {
    let fallback = crate::detect::detect_dialect(text);
    let secs = crate::segments::segments(text, fallback);
    let idx = crate::segments::section_index_at_line(&secs, line)?;
    let sec = &secs[idx];
    let dialect = crate::dialect::get_dialect(sec.dialect);
    let within = line - sec.line_offset;
    let (start, end) = word_range_at(sec.body, within, col, dialect.as_ref())?;
    Some((sec.line_offset + within, start, end))
}

/// Classify the tokens of one logical statement. `tokens[0]` is the element
/// or directive name (never renameable). Mirrors the parser's node/param
/// splitting so a token is a net iff the parser treats it as a node/port,
/// and a param key iff the parser treats it as a `key=value` key.
fn classify(tokens: &[&str]) -> Vec<Candidate> {
    let mut out = Vec::new();
    if tokens.is_empty() {
        return out;
    }
    let first = tokens[0];
    if first.starts_with('.') {
        if first.strip_prefix('.').is_some_and(|n| n.eq_ignore_ascii_case("subckt")) {
            // tokens[1] is the subckt name; ports follow until the first param.
            let mut i = 2;
            while i < tokens.len() && !tokens[i].contains('=') {
                out.push(net(tokens, i));
                i += 1;
            }
            mark_param_keys(tokens, i, &mut out);
        } else {
            mark_param_keys(tokens, 1, &mut out);
        }
        return out;
    }
    // Instance: node lists per element type, params after.
    let etype = first.chars().next().unwrap_or(' ');
    match etype {
        'R' | 'C' | 'L' => {
            for i in 1..3.min(tokens.len()) {
                out.push(net(tokens, i));
            }
            mark_param_keys(tokens, 3, &mut out);
        }
        'X' => {
            // Nodes are everything before the model (last non-param token).
            let rel = crate::parser::find_param_start(&tokens[1..]).unwrap_or(tokens.len() - 1);
            let nm_end = (1 + rel).min(tokens.len());
            if nm_end >= 2 {
                for i in 1..nm_end - 1 {
                    out.push(net(tokens, i));
                }
            }
            mark_param_keys(tokens, nm_end, &mut out);
        }
        _ => {
            let rel = crate::parser::find_param_start(&tokens[1..]);
            if let Some(n) = crate::parser::element_node_count(etype) {
                if tokens.len() > 1 + n {
                    for i in 1..=n {
                        out.push(net(tokens, i));
                    }
                    mark_param_keys(tokens, 1 + n, &mut out);
                } else {
                    let ps = 1 + rel.unwrap_or(tokens.len() - 1);
                    for i in 1..ps.min(tokens.len()) {
                        out.push(net(tokens, i));
                    }
                    mark_param_keys(tokens, ps, &mut out);
                }
            } else {
                // Unknown element type: params only, no net guesses.
                mark_param_keys(tokens, 1, &mut out);
            }
        }
    }
    out.sort_by_key(|c| (c.token, c.within_start, c.len));
    out.dedup_by_key(|c| (c.token, c.within_start, c.len));
    out
}

/// Mark `key=value` / `key = value` keys from `start` as renameable params.
fn mark_param_keys(tokens: &[&str], start: usize, out: &mut Vec<Candidate>) {
    let mut i = start;
    while i < tokens.len() {
        let tok = tokens[i];
        if tok == "=" {
            if i > start {
                let prev = tokens[i - 1];
                if !prev.contains('=') && prev != "=" {
                    out.push(Candidate { role: Role::ParamKey, token: i - 1, within_start: 0, len: prev.len() });
                }
            }
            i += 1;
            continue;
        }
        if let Some(eq) = tok.find('=') {
            if eq > 0 {
                out.push(Candidate { role: Role::ParamKey, token: i, within_start: 0, len: eq });
            }
            i += 1;
            continue;
        }
        if i + 1 < tokens.len() && tokens[i + 1] == "=" {
            // `key = value`: key here, skip the '=' and the value.
            out.push(Candidate { role: Role::ParamKey, token: i, within_start: 0, len: tok.len() });
            i += 3;
            continue;
        }
        i += 1;
    }
}

/// The renameable word under `(line, col)` within one section body:
/// its text (the renameable part) and role.
fn word_at(body: &str, line: usize, col: usize, dialect: &dyn Dialect) -> Option<(String, Role)> {
    let spans = crate::parser::logical_line_spans(body, dialect);
    let (start, _, logical) = spans.iter().find(|(s, e, _)| line >= *s && line <= *e)?;
    if dialect.is_comment_line(logical.trim()) {
        return None;
    }
    let candidates = classify(&crate::parser::tokenize(logical));
    if candidates.is_empty() {
        return None;
    }
    let body_lines: Vec<&str> = body.split('\n').collect();
    let mut global_tok = 0;
    for phys in *start..=line {
        let raw = body_lines.get(phys).copied().unwrap_or("");
        let (stripped, prefix) = code_part(raw, phys > *start, dialect);
        for tok in crate::parser::tokenize(stripped) {
            let off = tok.as_ptr() as usize - stripped.as_ptr() as usize;
            let tok_start = prefix + off;
            let tok_end = tok_start + tok.len();
            if phys == line && col >= tok_start && col < tok_end {
                if let Some(c) = candidates.iter().find(|c| c.token == global_tok) {
                    let part_start = tok_start + c.within_start;
                    let part_end = part_start + c.len.min(tok.len() - c.within_start);
                    if col >= part_end {
                        return None; // cursor on the value part of key=value
                    }
                    if col < part_start {
                        return None;
                    }
                    let text = tok[c.within_start..c.within_start + c.len.min(tok.len() - c.within_start)].to_string();
                    return Some((text, c.role));
                }
                return None;
            }
            global_tok += 1;
        }
    }
    None
}

/// Range of the renameable word under `(line, col)`, local to `body`.
fn word_range_at(body: &str, line: usize, col: usize, dialect: &dyn Dialect) -> Option<(usize, usize)> {
    let spans = crate::parser::logical_line_spans(body, dialect);
    let (start, _, logical) = spans.iter().find(|(s, e, _)| line >= *s && line <= *e)?;
    if dialect.is_comment_line(logical.trim()) {
        return None;
    }
    let candidates = classify(&crate::parser::tokenize(logical));
    if candidates.is_empty() {
        return None;
    }
    let body_lines: Vec<&str> = body.split('\n').collect();
    let mut global_tok = 0;
    for phys in *start..=line {
        let raw = body_lines.get(phys).copied().unwrap_or("");
        let (stripped, prefix) = code_part(raw, phys > *start, dialect);
        for tok in crate::parser::tokenize(stripped) {
            let off = tok.as_ptr() as usize - stripped.as_ptr() as usize;
            let tok_start = prefix + off;
            let tok_end = tok_start + tok.len();
            if phys == line && col >= tok_start && col < tok_end {
                if let Some(c) = candidates.iter().find(|c| c.token == global_tok) {
                    let part_start = tok_start + c.within_start;
                    let part_end = part_start + c.len.min(tok.len() - c.within_start);
                    if col >= part_end || col < part_start {
                        return None;
                    }
                    return Some((part_start, part_end));
                }
                return None;
            }
            global_tok += 1;
        }
    }
    None
}

/// Emit edits for every occurrence of `target` with `role` in `body`.
/// `offset` shifts local line numbers to whole-file coordinates.
fn collect_edits(
    body: &str,
    offset: usize,
    target: &str,
    role: Role,
    dialect: &dyn Dialect,
    new_name: &str,
    out: &mut Vec<Edit>,
) {
    let spans = crate::parser::logical_line_spans(body, dialect);
    let body_lines: Vec<&str> = body.split('\n').collect();
    for (start, end, logical) in &spans {
        if dialect.is_comment_line(logical.trim()) {
            continue;
        }
        let candidates = classify(&crate::parser::tokenize(logical));
        if candidates.is_empty() {
            continue;
        }
        let mut global_tok = 0;
        for phys in *start..=*end {
            let raw = body_lines.get(phys).copied().unwrap_or("");
            let (stripped, prefix) = code_part(raw, phys > *start, dialect);
            for tok in crate::parser::tokenize(stripped) {
                if let Some(c) = candidates.iter().find(|c| c.token == global_tok) {
                    let part = &tok[c.within_start..c.within_start + c.len.min(tok.len() - c.within_start)];
                    if c.role == role && part.eq_ignore_ascii_case(target) {
                        let off = tok.as_ptr() as usize - stripped.as_ptr() as usize;
                        out.push(Edit {
                            line: offset + phys,
                            start_col: prefix + off + c.within_start,
                            end_col: prefix + off + c.within_start + c.len,
                            text: new_name.to_string(),
                        });
                    }
                }
                global_tok += 1;
            }
        }
    }
}

/// The code part of a physical line, with the byte prefix consumed by a
/// leading continuation marker (and any whitespace after it) tracked
/// separately so token columns map back to the original line.
fn code_part<'a>(raw: &'a str, is_continuation: bool, dialect: &dyn Dialect) -> (&'a str, usize) {
    let cut = crate::parser::inline_comment_start(raw, dialect).unwrap_or(raw.len());
    let code = &raw[..cut];
    if !is_continuation {
        return (code, 0);
    }
    let t = code.trim_start();
    let rest = &t[1..];
    let lead = rest.len() - rest.trim_start().len();
    (rest.trim_start(), (code.len() - t.len()) + 1 + lead)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_tuples(edits: &[Edit]) -> Vec<(usize, usize, usize, String)> {
        let mut v: Vec<_> = edits
            .iter()
            .map(|e| (e.line, e.start_col, e.end_col, e.text.clone()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn renames_net_on_instances() {
        let text = "R1 a b 1k\nR2 c b 2k\n";
        let edits = rename_edits(text, 0, 5, "new_net").unwrap();
        assert_eq!(
            edit_tuples(&edits),
            vec![(0, 5, 6, "new_net".into()), (1, 5, 6, "new_net".into())]
        );
    }

    #[test]
    fn renames_net_in_subckt_ports_and_x_pins() {
        let text = ".subckt inv a b\nR1 a b 1k\n.ends inv\nX1 p a inv\n";
        let edits = rename_edits(text, 0, 12, "new_net").unwrap();
        assert_eq!(
            edit_tuples(&edits),
            vec![
                (0, 12, 13, "new_net".into()),
                (1, 3, 4, "new_net".into()),
                (3, 5, 6, "new_net".into()),
            ]
        );
        // same rename from an instance usage, case-insensitive
        let text2 = ".subckt inv A b\nR1 A b 1k\n.ends inv\nX1 p a inv\n";
        let edits2 = rename_edits(text2, 1, 3, "new_net").unwrap();
        assert_eq!(edit_tuples(&edits2), edit_tuples(&edits));
    }

    #[test]
    fn renames_param_keys() {
        let text = ".param w=1u\nR1 a b 1k w=2u\n.subckt inv a b w=3u\nR2 a b 1k\n.ends inv\n";
        let edits = rename_edits(text, 0, 7, "width").unwrap();
        assert_eq!(
            edit_tuples(&edits),
            vec![
                (0, 7, 8, "width".into()),
                (1, 10, 11, "width".into()),
                (2, 16, 17, "width".into()),
            ]
        );
    }

    #[test]
    fn renames_param_key_with_spaced_eq() {
        let text = "R1 a b 1k w = 2u\n";
        let edits = rename_edits(text, 0, 10, "width").unwrap();
        assert_eq!(edit_tuples(&edits), vec![(0, 10, 11, "width".into())]);
    }

    #[test]
    fn rename_does_not_touch_model_or_instance_names() {
        let text = "R1 a b 1k\nX1 a b inv\n";
        assert!(rename_edits(text, 0, 1, "R2").is_none());
        assert!(rename_edits(text, 0, 7, "1meg").is_none());
        assert!(rename_edits(text, 1, 9, "buf").is_none());
    }

    #[test]
    fn prepare_rename_returns_word_range() {
        let text = "R1 a b 1k w=2u\n";
        assert_eq!(prepare_rename(text, 0, 3), Some((0, 3, 4)));
        assert_eq!(prepare_rename(text, 0, 10), Some((0, 10, 11)));
        // value part of key=value is not renameable
        assert_eq!(prepare_rename(text, 0, 12), None);
        // comment and blank are not renameable
        assert_eq!(prepare_rename("R1 a b 1k $ a\n", 0, 12), None);
        assert_eq!(prepare_rename("\n", 0, 0), None);
    }

    #[test]
    fn rename_works_across_continuation_lines() {
        let text = "M1 a b c d nch w=1u\n+ l=2u\n";
        let edits = rename_edits(text, 1, 2, "len").unwrap();
        assert_eq!(edit_tuples(&edits), vec![(1, 2, 3, "len".into())]);
    }

    #[test]
    fn rename_works_with_inline_comments() {
        let text = "R1 a b 1k $ c b\nR2 c b 2k\n";
        let edits = rename_edits(text, 1, 5, "new_net").unwrap();
        // both code occurrences renamed; the "b" inside the comment untouched
        assert_eq!(
            edit_tuples(&edits),
            vec![(0, 5, 6, "new_net".into()), (1, 5, 6, "new_net".into())]
        );
    }

    #[test]
    fn rename_across_simulator_lang_sections() {
        let text = "R1 a b 1k\nsimulator lang=spectre\nR2 (a b) resistor r=1k\n";
        let edits = rename_edits(text, 0, 3, "new_net").unwrap();
        // Spectre paren nodes rename the name only: `(a` -> `(new_net`
        assert_eq!(
            edit_tuples(&edits),
            vec![
                (0, 3, 4, "new_net".into()),
                (2, 4, 5, "new_net".into()),
            ]
        );
    }

    #[test]
    fn invalid_new_name_returns_none() {
        let text = "R1 a b 1k\n";
        assert!(rename_edits(text, 0, 3, "a b").is_none());
        assert!(rename_edits(text, 0, 3, ".param").is_none());
        assert!(rename_edits(text, 0, 3, "x=1").is_none());
        assert!(rename_edits(text, 0, 3, "").is_none());
    }
}
