use crate::dialect::{get_dialect, DialectKind};
use crate::parser::{self};
use crate::segments;
use lsp_types::{
    SemanticToken, SemanticTokenType, SemanticTokens, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensServerCapabilities,
};

/// Legend order defines token type indices for delta encoding and client mapping.
/// Must stay stable; clients cache by index.
pub const LEGEND_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::VARIABLE, // 0 nets, ports
    SemanticTokenType::TYPE,     // 1 subckt names, model names
    SemanticTokenType::FUNCTION, // 2 instance names (R1, XU1)
    SemanticTokenType::KEYWORD,  // 3 directives (.param, .subckt, .ends, simulator)
    SemanticTokenType::PROPERTY, // 4 param keys
    SemanticTokenType::STRING,   // 5 quoted / paths
    SemanticTokenType::NUMBER,   // 6 numeric values
    SemanticTokenType::COMMENT,  // 7 comments
    SemanticTokenType::OPERATOR, // 8 =, (, )
];

const VARIABLE: u32 = 0;
const TYPE: u32 = 1;
const FUNCTION: u32 = 2;
const KEYWORD: u32 = 3;
const PROPERTY: u32 = 4;
const STRING: u32 = 5;
const NUMBER: u32 = 6;
const COMMENT: u32 = 7;
const OPERATOR: u32 = 8;

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: LEGEND_TYPES.to_vec(),
        token_modifiers: vec![],
    }
}

pub fn server_capabilities() -> SemanticTokensServerCapabilities {
    SemanticTokensOptions {
        work_done_progress_options: Default::default(),
        legend: legend(),
        range: Some(true),
        full: Some(lsp_types::SemanticTokensFullOptions::Delta { delta: Some(true) }),
    }
    .into()
}

/// Full-document semantic tokens (delta-encoded) for `text`.
pub fn semantic_tokens_full(text: &str, fallback: DialectKind) -> SemanticTokens {
    let raws = collect_raw_tokens(text, fallback);
    // raws already sorted by (line, col)
    let mut data = Vec::with_capacity(raws.len());
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    for (i, (line, col, len, ty)) in raws.into_iter().enumerate() {
        let delta_line = if i == 0 { line } else { line - prev_line };
        let delta_start = if i == 0 || line != prev_line {
            col
        } else {
            col - prev_col
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: len,
            token_type: ty,
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_col = col;
    }
    SemanticTokens {
        result_id: None,
        data,
    }
}

fn collect_raw_tokens(text: &str, fallback: DialectKind) -> Vec<(u32, u32, u32, u32)> {
    use std::collections::HashSet;
    let global_lines: Vec<&str> = text.lines().collect();
    let secs = segments::segments(text, fallback);
    // header lines set
    let mut header_lines: HashSet<usize> = HashSet::new();
    for sec in &secs {
        if sec.header.is_some() {
            // header is the line immediately before body start
            let hl = sec.line_offset.saturating_sub(1);
            // verify it parses as header to avoid false positives on empty pre-section
            header_lines.insert(hl);
        }
    }

    let mut out: Vec<(u32, u32, u32, u32)> = Vec::new();

    for (line_no, raw) in global_lines.iter().enumerate() {
        if header_lines.contains(&line_no) {
            highlight_header(raw, line_no as u32, &mut out);
            continue;
        }
        // find dialect for this line
        let kind = secs
            .iter()
            .find(|s| line_no >= s.line_offset && line_no < s.line_end)
            .map(|s| s.dialect)
            .unwrap_or(fallback);
        let dialect = get_dialect(kind);
        highlight_line(raw, line_no as u32, dialect.as_ref(), &mut out);
    }
    // also handle case where file ends with newline and last line is empty not in lines()?
    // lines() already yields empty entries for blank lines between, but trailing newline is ignored.
    // That's fine.

    // sort by line, col (already in order because we iterated line_no ascending and pushes are col ascending)
    out.sort_by_key(|(l, c, _, _)| (*l, *c));
    out
}

fn highlight_header(raw: &str, line: u32, out: &mut Vec<(u32, u32, u32, u32)>) {
    // raw is like "simulator lang=spice" possibly with leading spaces and extra tokens
    // highlight "simulator" as keyword, "lang" as property, "=" as operator, value as type
    let lower = raw.to_ascii_lowercase();
    if let Some(pos) = lower.find("simulator") {
        out.push((line, pos as u32, 9, KEYWORD));
        if let Some(lang_pos) = lower[pos..].find("lang") {
            let abs = pos + lang_pos;
            out.push((line, abs as u32, 4, PROPERTY));
            if let Some(eq_rel) = raw[abs..].find('=') {
                let eq_abs = abs + eq_rel;
                out.push((line, eq_abs as u32, 1, OPERATOR));
                // value after =
                let after = &raw[eq_abs + 1..];
                let val_trim = after.trim();
                if !val_trim.is_empty() {
                    // first alnum word is the value (spice/spectre)
                    let word_end = val_trim
                        .bytes()
                        .position(|b| !b.is_ascii_alphanumeric())
                        .unwrap_or(val_trim.len());
                    if word_end > 0 {
                        let val_start_in_after = after.find(val_trim).unwrap();
                        let abs_val = eq_abs + 1 + val_start_in_after;
                        out.push((line, abs_val as u32, word_end as u32, TYPE));
                    }
                }
            }
        }
    } else {
        // fallback: whole line as keyword
        let col = raw.len() - raw.trim_start().len();
        if !raw.trim().is_empty() {
            out.push((line, col as u32, raw.trim().len() as u32, KEYWORD));
        }
    }
}

fn highlight_line(raw: &str, line: u32, dialect: &dyn crate::dialect::Dialect, out: &mut Vec<(u32, u32, u32, u32)>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    // comment line?
    if dialect.is_comment_line(trimmed) {
        let col = (raw.len() - raw.trim_start().len()) as u32;
        let len = raw.trim_start().len() as u32;
        out.push((line, col, len, COMMENT));
        return;
    }
    // continuation line?
    let cont = dialect.continuation_char();
    if trimmed.starts_with(cont) && !dialect.is_comment_line(trimmed) {
        // '+' operator
        let col_plus = (raw.len() - raw.trim_start().len()) as u32;
        out.push((line, col_plus, 1, OPERATOR));
        let after_plus = &trimmed[1..];
        let rest = after_plus.trim();
        if rest.is_empty() {
            return;
        }
        // find rest start col in raw
        // rest's start offset relative to raw
        // compute by locating rest substring after '+'
        let plus_idx = raw.find(cont).unwrap();
        let rest_start_in_raw = raw[plus_idx + 1..].find(rest).map(|p| plus_idx + 1 + p).unwrap_or(plus_idx + 1);
        highlight_param_list(raw, line, rest_start_in_raw as u32, rest, out);
        return;
    }

    // inline comment handling
    let inline = inline_comment_start(raw, dialect);
    let code_part = if let Some(idx) = inline {
        &raw[..idx]
    } else {
        raw
    };
    let _code_trim = code_part.trim();
    if _code_trim.is_empty() {
        if let Some(idx) = inline {
            // entire line is comment after whitespace
            let col = idx as u32;
            let len = (raw.len() - idx) as u32;
            out.push((line, col, len, COMMENT));
        }
        return;
    }

    // Non-continuation code line: parse via logical line for accurate instance/subckt/directive distinction
    // Use trimmed code for parsing
    let code_for_parse = code_part.trim();
    let stmt = parser::parse_logical_line(code_for_parse, dialect);

    // We'll locate tokens by searching in raw sequentially.
    // For comment after code, emit later.

    match stmt {
        crate::ir::Stmt::Blank => {}
        crate::ir::Stmt::Comment(_) => {
            // Should have been caught, but fallback
            let col = (raw.len() - raw.trim_start().len()) as u32;
            out.push((line, col, raw.trim_start().len() as u32, COMMENT));
        }
        crate::ir::Stmt::Subckt(sub) => {
            // ".subckt" keyword
            if let Some(dot) = find_case_insensitive(code_part, ".subckt") {
                out.push((line, dot as u32, 7, KEYWORD));
                let mut cursor = dot + 7;
                // sub.name -> type
                if !sub.name.is_empty() {
                    if let Some(pos) = find_from(raw, &sub.name, cursor) {
                        out.push((line, pos as u32, sub.name.len() as u32, TYPE));
                        cursor = pos + sub.name.len();
                    }
                }
                // ports -> variable
                for p in &sub.ports {
                    if let Some(pos) = find_from(raw, p, cursor) {
                        out.push((line, pos as u32, p.len() as u32, VARIABLE));
                        cursor = pos + p.len();
                    }
                }
                for prm in &sub.params {
                    if let Some(pos) = find_from(raw, &prm.key, cursor) {
                        out.push((line, pos as u32, prm.key.len() as u32, PROPERTY));
                        cursor = pos + prm.key.len();
                        // find '=' after key
                        if let Some(eq) = find_from(raw, "=", cursor) {
                            // ensure eq is before value
                            if let Some(vpos) = find_from(raw, &prm.value, eq + 1) {
                                if eq < vpos {
                                    out.push((line, eq as u32, 1, OPERATOR));
                                    if !prm.value.is_empty() {
                                        let ty = classify_value(&prm.value);
                                        out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                        cursor = vpos + prm.value.len();
                                    } else {
                                        cursor = eq + 1;
                                    }
                                }
                            }
                        } else if !prm.value.is_empty() {
                            if let Some(vpos) = find_from(raw, &prm.value, cursor) {
                                let ty = classify_value(&prm.value);
                                out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                cursor = vpos + prm.value.len();
                            }
                        }
                    }
                }
            }
        }
        crate::ir::Stmt::Directive(d) => {
            // keyword ".<name>"
            let dot = find_case_insensitive(code_part, &format!(".{}", d.name));
            if let Some(pos) = dot {
                out.push((line, pos as u32, (d.name.len() + 1) as u32, KEYWORD));
                let mut cursor = pos + d.name.len() + 1;
                for arg in &d.args {
                    if arg.is_empty() {
                        continue;
                    }
                    if let Some(p) = find_from(raw, arg, cursor) {
                        let ty = if is_quoted(arg) { STRING } else { TYPE };
                        out.push((line, p as u32, arg.len() as u32, ty));
                        cursor = p + arg.len();
                    }
                }
                for prm in &d.params {
                    if let Some(kpos) = find_from(raw, &prm.key, cursor) {
                        out.push((line, kpos as u32, prm.key.len() as u32, PROPERTY));
                        cursor = kpos + prm.key.len();
                        if let Some(eq) = find_from(raw, "=", cursor) {
                            // check eq before value
                            if let Some(vpos) = find_from(raw, &prm.value, eq + 1) {
                                if eq < vpos || prm.value.is_empty() {
                                    out.push((line, eq as u32, 1, OPERATOR));
                                    if !prm.value.is_empty() {
                                        let ty = classify_value(&prm.value);
                                        out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                        cursor = vpos + prm.value.len();
                                    } else {
                                        cursor = eq + 1;
                                    }
                                    continue;
                                }
                            }
                            // fallback: value directly after key without eq search
                        }
                        if !prm.value.is_empty() {
                            if let Some(vpos) = find_from(raw, &prm.value, cursor) {
                                let ty = classify_value(&prm.value);
                                out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                cursor = vpos + prm.value.len();
                            }
                        }
                    }
                }
            } else if !d.name.is_empty() {
                // fallback
                if let Some(p) = find_case_insensitive(raw, &d.name) {
                    out.push((line, p as u32, d.name.len() as u32, KEYWORD));
                }
            }
        }
        crate::ir::Stmt::Instance(inst) => {
            // instance name
            let name = &inst.name;
            if let Some(pos) = find_from(raw, name, 0) {
                out.push((line, pos as u32, name.len() as u32, FUNCTION));
                let mut cursor = pos + name.len();

                // handle spectre parens operators
                let is_spectre = dialect.is_spectre();
                let has_paren = code_part.contains('(') && code_part.contains(')');
                if is_spectre && has_paren {
                    if let Some(open) = raw.find('(') {
                        out.push((line, open as u32, 1, OPERATOR));
                    }
                    if let Some(close) = raw.find(')') {
                        out.push((line, close as u32, 1, OPERATOR));
                    }
                }

                // nodes -> variable
                for n in &inst.nodes {
                    // strip parens for search
                    let clean = n.trim_matches(|c| c == '(' || c == ')' || c == ',');
                    if clean.is_empty() {
                        continue;
                    }
                    if let Some(pos) = find_from(raw, clean, cursor) {
                        // ensure not inside param list (after '=')
                        // we already handle parens, so just push
                        out.push((line, pos as u32, clean.len() as u32, VARIABLE));
                        cursor = pos + clean.len();
                    }
                }
                // model_or_value — handle parenthesized source functions like
                // pulse(0 1.2 ...) / pwl(...) / sin(...) where the '(' and
                // numbers inside should not all be TYPE. The parser currently
                // splits "pulse(0" as model_or_value and the rest as params,
                // so we split on '(' here to give '(' → OPERATOR and the
                // following number → NUMBER (rest of pulse args are params
                // handled below with number-aware fixing).
                if let Some(mv) = &inst.model_or_value {
                    if !mv.is_empty() {
                        let mut handled = false;
                        if mv.contains('(') {
                            if let Some(idx) = mv.find('(') {
                                let prefix = mv[..idx].trim();
                                if !prefix.is_empty() {
                                    if let Some(p) = find_from(raw, prefix, cursor) {
                                        let ty = if is_quoted(prefix) {
                                            STRING
                                        } else {
                                            TYPE
                                        };
                                        out.push((line, p as u32, prefix.len() as u32, ty));
                                        cursor = p + prefix.len();
                                    }
                                }
                                if let Some(open_pos) = find_from(raw, "(", cursor) {
                                    out.push((line, open_pos as u32, 1, OPERATOR));
                                    cursor = open_pos + 1;
                                    let after = mv[idx + 1..].trim();
                                    if !after.is_empty() {
                                        // after may be "0" (truncated) or "0 1.2 ... 2n)" (joined)
                                        // Tokenise by whitespace; handle trailing ')'
                                        let mut inner_cursor = cursor;
                                        let mut first = true;
                                        for tok in after.split(|c| c == ' ' || c == '\t' || c == ',') {
                                            if tok.is_empty() {
                                                continue;
                                            }
                                            let has_close = tok.contains(')');
                                            let clean = tok.trim_matches(|c| c == ')' || c == '(' || c == ',');
                                            if clean.is_empty() {
                                                if has_close {
                                                    if let Some(cp) = find_from(raw, ")", inner_cursor) {
                                                        out.push((line, cp as u32, 1, OPERATOR));
                                                        inner_cursor = cp + 1;
                                                    }
                                                }
                                                continue;
                                            }
                                            // only highlight the first inner token here;
                                            // remaining pulse args are in params and
                                            // will be fixed to NUMBER there
                                            if first {
                                                if let Some(tpos) = find_from(raw, clean, inner_cursor) {
                                                    let ty = if is_number_like(clean) { NUMBER } else { TYPE };
                                                    out.push((line, tpos as u32, clean.len() as u32, ty));
                                                    inner_cursor = tpos + clean.len();
                                                }
                                                first = false;
                                                if has_close {
                                                    if let Some(cp) = find_from(raw, ")", inner_cursor) {
                                                        out.push((line, cp as u32, 1, OPERATOR));
                                                        inner_cursor = cp + 1;
                                                    }
                                                }
                                            } else {
                                                // remaining tokens inside mv (joined case) — highlight them now
                                                if let Some(tpos) = find_from(raw, clean, inner_cursor) {
                                                    let ty = if is_number_like(clean) { NUMBER } else { TYPE };
                                                    out.push((line, tpos as u32, clean.len() as u32, ty));
                                                    inner_cursor = tpos + clean.len();
                                                }
                                                if has_close {
                                                    if let Some(cp) = find_from(raw, ")", inner_cursor) {
                                                        out.push((line, cp as u32, 1, OPERATOR));
                                                        inner_cursor = cp + 1;
                                                    }
                                                }
                                            }
                                        }
                                        cursor = inner_cursor;
                                    }
                                    handled = true;
                                }
                            }
                        }
                        if !handled {
                            if let Some(pos) = find_from(raw, mv, cursor) {
                                let ty = if is_quoted(mv) {
                                    STRING
                                } else if is_number_like(mv) {
                                    NUMBER
                                } else {
                                    TYPE
                                };
                                out.push((line, pos as u32, mv.len() as u32, ty));
                                cursor = pos + mv.len();
                            }
                        }
                    }
                }
                // params
                for prm in &inst.params {
                    // Parser quirk for some device types (e.g. M) leaves key="w=1u" value="" instead of split.
                    // Handle unsplit keys by splitting on '=' here.
                    if prm.key.contains('=') && prm.value.is_empty() {
                        if let Some((k, v)) = prm.key.split_once('=') {
                            if let Some(kpos) = find_from(raw, k, cursor) {
                                out.push((line, kpos as u32, k.len() as u32, PROPERTY));
                                cursor = kpos + k.len();
                                if let Some(eq) = find_from(raw, "=", cursor) {
                                    out.push((line, eq as u32, 1, OPERATOR));
                                    cursor = eq + 1;
                                    if !v.is_empty() {
                                        if let Some(vpos) = find_from(raw, v, cursor) {
                                            let ty = classify_value(v);
                                            out.push((line, vpos as u32, v.len() as u32, ty));
                                            cursor = vpos + v.len();
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    if prm.key.is_empty() && prm.value.is_empty() {
                        continue;
                    }
                    if prm.key.is_empty() {
                        if !prm.value.is_empty() {
                            if let Some(vpos) = find_from(raw, &prm.value, cursor) {
                                let ty = classify_value(&prm.value);
                                out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                cursor = vpos + prm.value.len();
                            }
                        }
                        continue;
                    }
                    if let Some(kpos) = find_from(raw, &prm.key, cursor) {
                        // Pulse/pwl args arrive as bare numbers like "1.2" or "2n)" with empty
                        // value — they should be NUMBER (and ")" → OPERATOR), not PROPERTY
                        // (which maps to Identifier and looks like a net). Handle trailing ')'.
                        let key = &prm.key;
                        if key.ends_with(')') && key.len() > 1 {
                            let without = key.trim_end_matches(')');
                            if !without.is_empty() {
                                if let Some(tpos) = find_from(raw, without, cursor) {
                                    let ty = if is_number_like(without) { NUMBER } else { PROPERTY };
                                    out.push((line, tpos as u32, without.len() as u32, ty));
                                    cursor = tpos + without.len();
                                    if let Some(close_pos) = find_from(raw, ")", cursor) {
                                        // ensure ')' is the one belonging to this token (near tpos)
                                        if close_pos < kpos + key.len() + 1 {
                                            out.push((line, close_pos as u32, 1, OPERATOR));
                                            cursor = close_pos + 1;
                                        } else {
                                            cursor = kpos + key.len();
                                        }
                                    } else {
                                        cursor = kpos + key.len();
                                    }
                                    if prm.value.is_empty() {
                                        // no '=' / value for this pulse arg — done
                                        let eq_pos = find_from(raw, "=", cursor);
                                        if let Some(eq) = eq_pos {
                                            // still check for '=' just in case
                                            if let Some(vpos) = find_from(raw, &prm.value, eq + 1) {
                                                if eq < vpos || prm.value.is_empty() {
                                                    out.push((line, eq as u32, 1, OPERATOR));
                                                    cursor = eq + 1;
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                    // has value — fall through to value handling below with updated cursor
                                    let eq_pos = find_from(raw, "=", cursor);
                                    if let Some(eq) = eq_pos {
                                        if let Some(vpos) = find_from(raw, &prm.value, eq + 1) {
                                            if eq < vpos || prm.value.is_empty() {
                                                out.push((line, eq as u32, 1, OPERATOR));
                                                if !prm.value.is_empty() {
                                                    let ty = classify_value(&prm.value);
                                                    out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                                    cursor = vpos + prm.value.len();
                                                } else {
                                                    cursor = eq + 1;
                                                }
                                                continue;
                                            }
                                        } else if prm.value.is_empty() {
                                            out.push((line, eq as u32, 1, OPERATOR));
                                            cursor = eq + 1;
                                            continue;
                                        }
                                    }
                                    if !prm.value.is_empty() {
                                        if let Some(vpos) = find_from(raw, &prm.value, cursor) {
                                            let ty = classify_value(&prm.value);
                                            out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                            cursor = vpos + prm.value.len();
                                        }
                                    }
                                    continue;
                                }
                            }
                        }
                        let ty = if is_number_like(key) { NUMBER } else { PROPERTY };
                        out.push((line, kpos as u32, key.len() as u32, ty));
                        cursor = kpos + key.len();
                        // operator
                        let eq_pos = find_from(raw, "=", cursor);
                        if let Some(eq) = eq_pos {
                            if let Some(vpos) = find_from(raw, &prm.value, eq + 1) {
                                // only emit eq if it's between key and value
                                if eq < vpos || prm.value.is_empty() {
                                    out.push((line, eq as u32, 1, OPERATOR));
                                    if !prm.value.is_empty() {
                                        let ty = classify_value(&prm.value);
                                        out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                        cursor = vpos + prm.value.len();
                                    } else {
                                        cursor = eq + 1;
                                    }
                                    continue;
                                }
                            } else if prm.value.is_empty() {
                                out.push((line, eq as u32, 1, OPERATOR));
                                cursor = eq + 1;
                                continue;
                            }
                        }
                        if !prm.value.is_empty() {
                            if let Some(vpos) = find_from(raw, &prm.value, cursor) {
                                let ty = classify_value(&prm.value);
                                out.push((line, vpos as u32, prm.value.len() as u32, ty));
                                cursor = vpos + prm.value.len();
                            }
                        }
                    } else if !prm.value.is_empty() {
                        // key not found (maybe value-only param like positional)
                        if let Some(vpos) = find_from(raw, &prm.value, cursor) {
                            let ty = classify_value(&prm.value);
                            out.push((line, vpos as u32, prm.value.len() as u32, ty));
                            cursor = vpos + prm.value.len();
                        }
                    }
                }
            }
        }
    }

    // inline comment
    if let Some(idx) = inline {
        let col = idx as u32;
        let len = (raw.len() - idx) as u32;
        // avoid duplicate if already pushed comment (for comment lines)
        // Check if we already pushed a comment covering this range
        let already = out.iter().any(|(l, c, _, t)| *l == line && *t == COMMENT && *c == col);
        if !already {
            out.push((line, col, len, COMMENT));
        }
    }
}

fn highlight_param_list(_raw: &str, line: u32, start_col: u32, rest: &str, out: &mut Vec<(u32, u32, u32, u32)>) {
    // rest is trimmed param text after '+', need to highlight keys/values inside
    // tokenize respecting quotes
    let tokens = tokenize(rest);
    let mut cursor_in_rest = 0usize;
    // base col in raw where rest starts
    for tok in tokens {
        if tok.is_empty() {
            continue;
        }
        // find tok in rest from cursor
        let rel = rest[cursor_in_rest..].find(tok);
        if let Some(rel_pos) = rel {
            let tok_col = start_col + (cursor_in_rest + rel_pos) as u32;
            if tok == "=" {
                out.push((line, tok_col, 1, OPERATOR));
            } else if tok.contains('=') {
                let (k, v) = tok.split_once('=').unwrap();
                let k_trim = k.trim();
                let v_trim = v.trim();
                if !k_trim.is_empty() {
                    // key part starts at tok_col
                    let k_off = tok.find(k_trim).unwrap();
                    out.push((line, tok_col + k_off as u32, k_trim.len() as u32, PROPERTY));
                    // '=' operator
                    let eq_off = tok.find('=').unwrap();
                    out.push((line, tok_col + eq_off as u32, 1, OPERATOR));
                    if !v_trim.is_empty() {
                        let v_off = tok.find(v_trim).unwrap();
                        let ty = classify_value(v_trim);
                        out.push((line, tok_col + v_off as u32, v_trim.len() as u32, ty));
                    }
                } else if !v_trim.is_empty() {
                    let v_off = tok.find(v_trim).unwrap();
                    let ty = classify_value(v_trim);
                    out.push((line, tok_col + v_off as u32, v_trim.len() as u32, ty));
                }
            } else {
                // standalone token inside param list
                // Could be key without value or key with following "=" token
                // Treat as property
                out.push((line, tok_col, tok.len() as u32, PROPERTY));
            }
            cursor_in_rest += rel_pos + tok.len();
        }
    }
    // handle standalone "=" tokens that were split? tokenize would have "=" as separate token if spaces around "="
    // Already handled
}

fn classify_value(v: &str) -> u32 {
    if is_quoted(v) {
        STRING
    } else if is_number_like(v) {
        NUMBER
    } else {
        // for values like "nch", "pmos", "resistor", "{expr}"
        // treat brace expr as number? but keep as STRING if contains '{'
        if v.contains('{') || v.contains('}') {
            return STRING;
        }
        // check if looks like number with unit suffix: still number
        // is_number_like already checks first char digit; for "1k" true
        // otherwise type
        TYPE
    }
}

fn is_quoted(s: &str) -> bool {
    (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
}

fn is_number_like(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let first = t.chars().next().unwrap();
    if first.is_ascii_digit() || first == '.' || first == '+' || first == '-' {
        return true;
    }
    // hex? not needed
    // also handle like "1.2*1p" is number-like start digit
    false
}

fn find_from(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack[from..].find(needle).map(|p| from + p)
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let h_low = haystack.to_ascii_lowercase();
    let n_low = needle.to_ascii_lowercase();
    h_low.find(&n_low)
}

fn tokenize(s: &str) -> Vec<&str> {
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

fn inline_comment_start(line: &str, dialect: &dyn crate::dialect::Dialect) -> Option<usize> {
    let delim = dialect.inline_comment_delim()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::DialectKind;

    fn tokens(text: &str, kind: DialectKind) -> Vec<(u32, u32, u32, u32)> {
        collect_raw_tokens(text, kind)
    }

    #[test]
    fn nets_are_variable() {
        let t = tokens("R1 n1 n2 1k\n", DialectKind::Hspice);
        // function R1, variable n1 n2, number 1k?
        assert!(t.iter().any(|(_, _, _, ty)| *ty == FUNCTION));
        let vars: Vec<_> = t.iter().filter(|(_, _, _, ty)| *ty == VARIABLE).collect();
        assert_eq!(vars.len(), 2);
    }

    #[test]
    fn subckt_ports_variable_and_name_type() {
        let t = tokens(".subckt inv a b\n", DialectKind::Hspice);
        assert!(t.iter().any(|(_, _, _, ty)| *ty == KEYWORD));
        assert!(t.iter().any(|(_, _, _, ty)| *ty == TYPE));
        let vars: Vec<_> = t.iter().filter(|(_, _, _, ty)| *ty == VARIABLE).collect();
        assert_eq!(vars.len(), 2);
    }

    #[test]
    fn params_property_and_number() {
        let t = tokens("M1 d g s b nch w=1u l=2u\n", DialectKind::Hspice);
        assert!(t.iter().any(|(_, _, _, ty)| *ty == PROPERTY));
        // w and l keys
        let props: Vec<_> = t.iter().filter(|(_, _, _, ty)| *ty == PROPERTY).collect();
        assert!(props.len() >= 2);
    }

    #[test]
    fn comment_highlighted() {
        let t = tokens("* comment\n", DialectKind::Hspice);
        assert!(t.iter().any(|(_, _, _, ty)| *ty == COMMENT));
    }

    #[test]
    fn spectre_paren_nets() {
        let t = tokens("R1 (a b) resistor r=1k\n", DialectKind::Spectre);
        let vars: Vec<_> = t.iter().filter(|(_, _, _, ty)| *ty == VARIABLE).collect();
        assert!(vars.len() >= 2);
        assert!(t.iter().any(|(_, _, _, ty)| *ty == OPERATOR)); // '(' or ')'
    }

    #[test]
    fn delta_encoding_simple() {
        let st = semantic_tokens_full("R1 a b 1k\n", DialectKind::Hspice);
        assert!(!st.data.is_empty());
        // first token delta_line should be 0
        assert_eq!(st.data[0].delta_line, 0);
    }
}
