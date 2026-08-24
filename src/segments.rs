//! Per-section dialect switching for `.scs` files.
//!
//! A `.scs` (Spectre netlist) file is a sequence of *language segments*. A
//! `simulator lang=spice` line means "parse the following lines as SPICE until
//! the next `simulator lang=spectre`"; `simulator lang=spectre` switches back.
//! This module splits the input into sections by `simulator lang=` directives
//! so the formatter/linter can route each section to the right existing
//! dialect. It adds no new grammar — segmentation only dispatches.
//!
//! Design invariants:
//! - **No `simulator lang=` lines ⇒ single section, `header: None`** (the
//!   fast path — callers take today's code path verbatim, guaranteeing no
//!   regression on plain decks).
//! - **`simulator lang=…` lines are structural, not comments**, and pass
//!   through the formatter verbatim.
//! - **`spice` is the only switch Spectre documents in practice**; an unknown
//!   value does NOT start a new section (the line stays in the current body
//!   and passes through verbatim under the current dialect).
//! - Continuation (`+`) lines never span a `simulator lang=` line: the
//!   segmenter operates on physical lines only; `logical_line_spans` runs
//!   *within* each section, so a `+` at the top of a section orphans —
//!   which is correct, because a `simulator lang=` line severs continuation
//!   attachment.

use crate::dialect::DialectKind;

/// A run of physical lines parsed under one dialect, plus the directive
/// line that started it (if any — the file's first section has none).
pub struct Section<'a> {
    /// The `simulator lang=…` line verbatim, or `None` for the implicit
    /// pre-switch section. Emitted verbatim by the formatter.
    pub header: Option<&'a str>,
    pub dialect: DialectKind,
    /// Physical line index of the first body line (0-based). Used to offset
    /// diagnostic line numbers to global coordinates.
    pub line_offset: usize,
    /// One past the last body line's physical index (0-based). Used to locate
    /// which section a global line falls in (LSP definition requests).
    pub line_end: usize,
    /// The body text: physical lines [line_offset, line_end) with their
    /// original line terminators, as a slice of the input.
    pub body: &'a str,
}

/// Split `input` into sections by `simulator lang=` directives.
///
/// If the file has no `simulator lang=` line anywhere, returns a single
/// section covering the whole file with `fallback` and `header: None`.
pub fn segments(input: &str, fallback: DialectKind) -> Vec<Section<'_>> {
    let bytes = input.as_bytes();

    // Collect each physical line's text and byte offsets so body slices can
    // borrow directly from `input`. `str::lines()` strips `\n` and `\r\n`;
    // we track the terminator to compute each line's full byte span.
    let mut line_starts: Vec<usize> = Vec::new();
    let mut line_next: Vec<usize> = Vec::new(); // start of the next line (after terminator)
    let mut line_texts: Vec<&str> = Vec::new();
    let mut pos = 0;
    for text in input.lines() {
        line_starts.push(pos);
        let text_end = pos + text.len();
        let next = if text_end < bytes.len() && bytes[text_end] == b'\n' {
            text_end + 1
        } else if text_end + 1 <= bytes.len()
            && bytes.get(text_end) == Some(&b'\r')
            && bytes.get(text_end + 1) == Some(&b'\n')
        {
            text_end + 2
        } else {
            text_end
        };
        line_next.push(next);
        line_texts.push(text);
        pos = next;
    }
    let total = line_texts.len();

    // Find header lines: `^\s*simulator\s+lang\s*=\s*(spice|spectre)\b`
    // (case-insensitive ASCII). Unknown values do not start a section.
    let headers: Vec<(usize, DialectKind)> = line_texts
        .iter()
        .enumerate()
        .filter_map(|(i, t)| parse_header(t).map(|k| (i, k)))
        .collect();

    // Fast path: no headers → single whole-file section.
    if headers.is_empty() {
        return vec![Section {
            header: None,
            dialect: fallback,
            line_offset: 0,
            line_end: total,
            body: input,
        }];
    }

    let mut out = Vec::with_capacity(headers.len() + 1);

    // Implicit pre-section (lines before the first header).
    let first_h = headers[0].0;
    if first_h > 0 {
        let bs = line_starts[0];
        let be = line_starts[first_h];
        out.push(Section {
            header: None,
            dialect: fallback,
            line_offset: 0,
            line_end: first_h,
            body: &input[bs..be],
        });
    }

    // Sections starting at each header.
    for (i, &(h_idx, kind)) in headers.iter().enumerate() {
        let body_start = h_idx + 1;
        let body_end = if i + 1 < headers.len() {
            headers[i + 1].0
        } else {
            total
        };
        let body = if body_start < total {
            let bs = line_starts[body_start];
            let be = if body_end < total {
                line_starts[body_end]
            } else {
                input.len()
            };
            &input[bs..be]
        } else {
            ""
        };
        out.push(Section {
            header: Some(line_texts[h_idx]),
            dialect: kind,
            line_offset: body_start,
            line_end: body_end,
            body,
        });
    }

    out
}

/// Find which section a 0-based global `line` falls in, returning its index.
pub fn section_index_at_line(secs: &[Section<'_>], line: usize) -> Option<usize> {
    secs.iter().position(|s| line >= s.line_offset && line < s.line_end)
}

/// Parse a `simulator lang=VALUE` line. Returns the dialect for `spice` or
/// `spectre`; `None` for non-matching lines or unknown values.
///
/// `spice` maps to [`DialectKind::Ngspice`] — the existing dialect whose
/// `;` inline-comment delim and `key = value` spacing match generic SPICE.
/// `spectre` maps to [`DialectKind::Spectre`].
fn parse_header(line: &str) -> Option<DialectKind> {
    let bytes = line.as_bytes();
    let mut i = 0;

    // Skip leading whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }

    // Match "simulator" (case-insensitive ASCII).
    let kw = b"simulator";
    if i + kw.len() > bytes.len() || !bytes[i..i + kw.len()].eq_ignore_ascii_case(kw) {
        return None;
    }
    i += kw.len();

    // Must be followed by whitespace.
    if i >= bytes.len() || !(bytes[i] == b' ' || bytes[i] == b'\t') {
        return None;
    }
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }

    // Match "lang" (case-insensitive ASCII).
    let kw = b"lang";
    if i + kw.len() > bytes.len() || !bytes[i..i + kw.len()].eq_ignore_ascii_case(kw) {
        return None;
    }
    i += kw.len();

    // Optional whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }

    // Match '='.
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;

    // Optional whitespace before the value.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }

    // Value: the first alphanumeric word (case-insensitive). A word boundary
    // (non-alphanumeric char or EOL) must follow for `spice`/`spectre` to
    // match — `spicex` does not trigger a switch.
    let value = &line[i..];
    let word_end = value
        .bytes()
        .position(|b| !b.is_ascii_alphanumeric())
        .unwrap_or(value.len());
    let word = &value[..word_end];
    if word.eq_ignore_ascii_case("spice") {
        Some(DialectKind::Ngspice)
    } else if word.eq_ignore_ascii_case("spectre") {
        Some(DialectKind::Spectre)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(input: &str) -> Vec<(Option<&str>, DialectKind, usize, usize)> {
        segments(input, DialectKind::Spectre)
            .into_iter()
            .map(|s| (s.header, s.dialect, s.line_offset, s.line_end))
            .collect()
    }

    #[test]
    fn no_headers_returns_single_whole_file_section() {
        let input = "R1 a b 1k\nC1 b 0 1p\n";
        let s = segments(input, DialectKind::Hspice);
        assert_eq!(s.len(), 1);
        assert!(s[0].header.is_none());
        assert_eq!(s[0].dialect, DialectKind::Hspice);
        assert_eq!(s[0].line_offset, 0);
        assert_eq!(s[0].line_end, 2);
        assert_eq!(s[0].body, input);
    }

    #[test]
    fn spice_only_single_section() {
        let input = "simulator lang=spice\nR1 a b 1k\n";
        let s = secs(input);
        assert_eq!(s, vec![(Some("simulator lang=spice"), DialectKind::Ngspice, 1, 2)]);
    }

    #[test]
    fn spectre_only_single_section() {
        let input = "simulator lang=spectre\nR1 (a b) resistor r=1k\n";
        let s = secs(input);
        assert_eq!(s, vec![(Some("simulator lang=spectre"), DialectKind::Spectre, 1, 2)]);
    }

    #[test]
    fn mixed_three_sections() {
        let input = "\
simulator lang=spice
R1 a b 1k
simulator lang=spectre
R2 (a b) resistor r=2k
simulator lang=spice
R3 c d 3k
";
        let s = secs(input);
        assert_eq!(
            s,
            vec![
                (Some("simulator lang=spice"), DialectKind::Ngspice, 1, 2),
                (Some("simulator lang=spectre"), DialectKind::Spectre, 3, 4),
                (Some("simulator lang=spice"), DialectKind::Ngspice, 5, 6),
            ]
        );
    }

    #[test]
    fn pre_section_before_first_header_uses_fallback() {
        let input = "* comment\nsimulator lang=spice\nR1 a b 1k\n";
        let s = secs(input);
        assert_eq!(
            s,
            vec![
                (None, DialectKind::Spectre, 0, 1),
                (Some("simulator lang=spice"), DialectKind::Ngspice, 2, 3),
            ]
        );
    }

    #[test]
    fn file_starting_with_header_drops_empty_pre_section() {
        let input = "simulator lang=spice\nR1 a b 1k\n";
        let s = secs(input);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].0, Some("simulator lang=spice"));
        assert_eq!(s[0].2, 1); // line_offset
    }

    #[test]
    fn unknown_lang_value_does_not_start_a_section() {
        let input = "simulator lang=verilog\nR1 a b 1k\n";
        let s = segments(input, DialectKind::Hspice);
        assert_eq!(s.len(), 1);
        assert!(s[0].header.is_none());
        assert_eq!(s[0].dialect, DialectKind::Hspice);
    }

    #[test]
    fn case_insensitive_header_matching() {
        let input = "Simulator Lang=SPICE\nR1 a b 1k\n";
        let s = secs(input);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].0, Some("Simulator Lang=SPICE"));
        assert_eq!(s[0].1, DialectKind::Ngspice);
    }

    #[test]
    fn whitespace_around_equals_is_tolerated() {
        let input = "simulator lang = spice\nR1 a b 1k\n";
        let s = secs(input);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].1, DialectKind::Ngspice);
    }

    #[test]
    fn extra_text_after_value_is_part_of_header() {
        // `simulator lang=spice section=foo` — the whole line is the header.
        let input = "simulator lang=spice section=foo\nR1 a b 1k\n";
        let s = segments(input, DialectKind::Spectre);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].header, Some("simulator lang=spice section=foo"));
        assert_eq!(s[0].dialect, DialectKind::Ngspice);
    }

    #[test]
    fn spicex_does_not_match() {
        let input = "simulator lang=spicex\nR1 a b 1k\n";
        let s = segments(input, DialectKind::Hspice);
        assert_eq!(s.len(), 1);
        assert!(s[0].header.is_none());
    }

    #[test]
    fn leading_blanks_before_first_header_are_pre_section() {
        let input = "\n\nsimulator lang=spice\nR1 a b 1k\n";
        let s = secs(input);
        assert_eq!(
            s,
            vec![
                (None, DialectKind::Spectre, 0, 2),
                (Some("simulator lang=spice"), DialectKind::Ngspice, 3, 4),
            ]
        );
    }

    #[test]
    fn continuation_after_header_orphans() {
        // A `+` immediately after a `simulator lang=` line must be in the new
        // section's body, not attached to the previous section. The segmenter
        // does NOT merge continuations — that happens per-section via
        // `logical_line_spans`, so a `+` at the top of a section can't reach
        // into the previous section.
        let input = "R1 a b 1k\nsimulator lang=spice\n+ w=2u\n";
        let s = segments(input, DialectKind::Spectre);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].header, None);
        assert_eq!(s[0].dialect, DialectKind::Spectre);
        assert_eq!(s[0].line_offset, 0);
        assert_eq!(s[0].line_end, 1);
        assert_eq!(s[1].header, Some("simulator lang=spice"));
        assert_eq!(s[1].dialect, DialectKind::Ngspice);
        assert_eq!(s[1].line_offset, 2);
        assert_eq!(s[1].line_end, 3);
        // The `+ w=2u` is in the spice section's body, not the pre-section.
        assert!(s[1].body.contains("+ w=2u"));
        assert!(!s[0].body.contains("+ w=2u"));
    }

    #[test]
    fn trailing_header_with_empty_body() {
        let input = "simulator lang=spice\nR1 a b 1k\nsimulator lang=spectre\n";
        let s = segments(input, DialectKind::Spectre);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].body, "");
        assert_eq!(s[1].line_offset, 3); // one past the spectre header (line 2)
        assert_eq!(s[1].line_end, 3);
    }

    #[test]
    fn section_index_at_line_finds_right_section() {
        let input = "\
simulator lang=spice
R1 a b 1k
simulator lang=spectre
R2 (a b) resistor r=2k
";
        let s = segments(input, DialectKind::Spectre);
        assert_eq!(section_index_at_line(&s, 0), None); // header line — not in any body
        assert_eq!(section_index_at_line(&s, 1), Some(0)); // spice body
        assert_eq!(section_index_at_line(&s, 2), None); // header line
        assert_eq!(section_index_at_line(&s, 3), Some(1)); // spectre body
    }

    #[test]
    fn blank_lines_between_header_and_next_header_belong_to_current_body() {
        let input = "simulator lang=spice\n\n\nsimulator lang=spectre\nR1 (a b) resistor r=1k\n";
        let s = segments(input, DialectKind::Spectre);
        // The blank lines belong to the spice section's body.
        assert_eq!(s[0].line_offset, 1);
        assert_eq!(s[0].line_end, 3); // up to (not including) the spectre header
        assert_eq!(s[0].body, "\n\n");
        assert_eq!(s[1].line_offset, 4);
        assert_eq!(s[1].line_end, 5);
    }
}