use crate::dialect::DialectKind;
use crate::starts_with_ci;

/// Per-dialect evidence tallies from a netlist scan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DialectScores {
    pub hspice: u32,
    pub ngspice: u32,
    pub spectre: u32,
    pub ltspice: u32,
}

impl DialectScores {
    /// If exactly one dialect has DECISIVE evidence, it wins outright.
    /// Otherwise (no decisive evidence, or conflicting decisive markers from
    /// a mixed-dialect file) compare the non-decisive remainder; ties (and
    /// all-zero) fall back to HSPICE (golden reference).
    /// Priority: hspice > ngspice > spectre > ltspice.
    pub fn winner(&self) -> DialectKind {
        let raw = [
            (DialectKind::Hspice, self.hspice),
            (DialectKind::Ngspice, self.ngspice),
            (DialectKind::Spectre, self.spectre),
            (DialectKind::Ltspice, self.ltspice),
        ];
        if raw.iter().filter(|(_, s)| *s >= DECISIVE).count() == 1 {
            return raw.iter().find(|(_, s)| *s >= DECISIVE).unwrap().0;
        }
        let mut best = DialectKind::Hspice;
        let mut best_score = self.hspice % DECISIVE;
        for (kind, score) in raw.iter().skip(1) {
            let rem = score % DECISIVE;
            if rem > best_score {
                best = *kind;
                best_score = rem;
            }
        }
        best
    }
}

const INLINE_CAP: u32 = 5;
const SPECTRE_SYNTAX_CAP: u32 = 6;

/// Score for grammar that exists in exactly one supported dialect (e.g.
/// `.control`/`.csparam` are ngspice-only, `.backanno` is LTspice-only).
/// Normal evidence can never outvote it; conflicting decisive markers
/// (a file genuinely mixing dialects) fall back to the weighted scores.
const DECISIVE: u32 = 100_000;

/// Detect the dialect of a SPICE netlist by scoring dialect-exclusive markers.
/// Grammar found in exactly one dialect scores DECISIVE and cannot be outvoted:
/// `.control`/`.csparam`/`$&` meas-result refs → ngspice; `.alter`/`.protect`/
/// `.unprotect`/`.data`/`.dalo`/`.graph` → hspice; `.step`/`.backanno` →
/// ltspice; `//` comments, paren node syntax, `simulator lang` → spectre.
/// Markers shared across simulators (`.probe`, `.measure`, `;` vs `$`, ...)
/// score nothing or weakly. `.probe` is excluded — ngspice 37+ supports it.
///
/// Files genuinely mixing dialects (conflicting decisive markers) fall back
/// to the weighted remainder; use `--dialect` to override.
pub fn detect_dialect(input: &str) -> DialectKind {
    score_dialect(input).winner()
}

pub fn score_dialect(input: &str) -> DialectScores {
    let mut s = DialectScores::default();
    let mut semis = 0;
    let mut dollars = 0;
    let mut slashes = 0;
    let mut parens = 0;
    let mut meas_refs = 0;

    for raw in input.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('*') {
            continue;
        }

        if starts_with_ci(trimmed, "//") {
            if slashes < SPECTRE_SYNTAX_CAP {
                s.spectre += DECISIVE;
                slashes += 1;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            s.spectre += 1;
            continue;
        }

        // Continuation lines contribute inline-comment evidence only.
        let code = trimmed.strip_prefix('+').map(str::trim).unwrap_or(trimmed);

        // First-byte dispatch: the keyword probes below all match a known
        // leading byte, so a line starting with anything else (the vast
        // majority — device cards) skips every `starts_with_ci` call.
        match code.as_bytes().first().copied().map(|b| b.to_ascii_lowercase()) {
            Some(b's') => {
                if starts_with_ci(code, "simulator lang") {
                    s.spectre += DECISIVE;
                }
                for kw in ["section ", "statistics "] {
                    if starts_with_ci(code, kw) {
                        s.spectre += DECISIVE;
                        break;
                    }
                }
            }
            Some(b'a') => {
                if starts_with_ci(code, "ahdl_include") {
                    s.spectre += DECISIVE;
                }
            }
            Some(b'e') => {
                if starts_with_ci(code, "endsection ") {
                    s.spectre += DECISIVE;
                }
            }
            // Bare `parameters`/`variables`/`include "..."` could be title
            // lines in SPICE dialects, so they stay weak evidence.
            Some(b'p') => {
                if starts_with_ci(code, "parameters ") {
                    s.spectre += 2;
                }
            }
            Some(b'v') => {
                if starts_with_ci(code, "variables ") {
                    s.spectre += 2;
                }
            }
            Some(b'i') => {
                if starts_with_ci(code, "include \"") {
                    s.spectre += 2;
                }
            }
            _ => {}
        }

        if let Some(rest) = code.strip_prefix('.') {
            let word = rest.split([',', ' ', '\t']).next().unwrap_or("");
            // ngspice-exclusive
            if word.eq_ignore_ascii_case("control") || word.eq_ignore_ascii_case("csparam") {
                s.ngspice += DECISIVE;
            } else if word.eq_ignore_ascii_case("endc")
                || word.eq_ignore_ascii_case("options")
                || word.eq_ignore_ascii_case("meas")
            {
                s.ngspice += 1;
            } else if word.eq_ignore_ascii_case("alter")
                || word.eq_ignore_ascii_case("protect")
                || word.eq_ignore_ascii_case("unprotect")
                || word.eq_ignore_ascii_case("data")
                || word.eq_ignore_ascii_case("dalo")
                || word.eq_ignore_ascii_case("graph")
            {
                // hspice-exclusive (among supported dialects)
                s.hspice += DECISIVE;
            } else if word.eq_ignore_ascii_case("measure") || word.eq_ignore_ascii_case("option") {
                // shared flavor: ngspice accepts .measure/.option too
                s.hspice += 1;
            } else if word.eq_ignore_ascii_case("step") || word.eq_ignore_ascii_case("backanno") {
                // ltspice-exclusive (.step is also PSpice, unsupported here)
                s.ltspice += DECISIVE;
            }
        } else if has_leading_paren_nodes(code) && parens < SPECTRE_SYNTAX_CAP {
            s.spectre += DECISIVE;
            parens += 1;
        }

        // Inline-comment / measurement-deref evidence, one fused quote-aware
        // pass per line. Pre-filter with `contains` (SIMD memchr): marker-free
        // lines — the common case in generated decks — never enter the scan,
        // and the caps keep fully-saturated streams out of it too.
        //
        // ngspice measurement-result dereference (`FROM $&t1 TO=$&t2`):
        // HSPICE would treat the `$` as the start of an inline comment, so
        // `$` evidence must not fire on `$&` derefs.
        if (semis < INLINE_CAP
            || dollars < INLINE_CAP
            || meas_refs < INLINE_CAP
            || slashes < SPECTRE_SYNTAX_CAP)
            && (code.contains(';') || code.contains('$') || code.contains('/'))
        {
            let ev = scan_inline_evidence(code);
            if ev.meas_ref && meas_refs < INLINE_CAP {
                s.ngspice += DECISIVE;
                meas_refs += 1;
            }
            if ev.semi && semis < INLINE_CAP {
                s.ngspice += 1;
                s.ltspice += 1;
                semis += 1;
            }
            if ev.dollar && dollars < INLINE_CAP {
                s.hspice += 1;
                dollars += 1;
            }
            if ev.slashes && slashes < SPECTRE_SYNTAX_CAP {
                s.spectre += DECISIVE;
                slashes += 1;
            }
        }
    }
    s
}

/// `Name (nodes ...) model ...` — Spectre instance syntax (paren directly
/// after the first token; HSPICE subckt param parens `(w=1)` come later in
/// the line and don't match).
fn has_leading_paren_nodes(code: &str) -> bool {
    match code.find(char::is_whitespace) {
        Some(idx) => code[idx..].trim_start().starts_with('('),
        None => false,
    }
}

/// Inline-evidence flags collected by one quote-aware pass over a line.
#[derive(Default)]
struct InlineEvidence {
    semi: bool,
    dollar: bool,
    meas_ref: bool,
    slashes: bool,
}

/// Single quote-aware byte pass collecting every inline-evidence marker at
/// once (`;`, unquoted `$` that is not a `$&` deref, `$&`, `//`) — replacing
/// what used to be four independent scans per line. Byte-oriented: UTF-8
/// continuation bytes are >= 0x80 and can never equal an ASCII quote or
/// marker byte, so scanning bytes is exact, not an approximation.
fn scan_inline_evidence(code: &str) -> InlineEvidence {
    let bytes = code.as_bytes();
    let mut ev = InlineEvidence::default();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' && !in_double {
            in_single = !in_single;
        } else if b == b'"' && !in_single {
            in_double = !in_double;
        } else if !in_single && !in_double {
            match b {
                b';' => ev.semi = true,
                b'$' => {
                    if bytes.get(i + 1) == Some(&b'&') {
                        ev.meas_ref = true;
                        i += 1;
                    } else {
                        ev.dollar = true;
                    }
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    ev.slashes = true;
                    i += 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    ev
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::DialectKind;

    const HSPICE: &str = "\
* hspice deck
.temp 70
.option post=2
.probe v(out)
.alter
.param vdd='1.2*2'
R1 a b 1k $ feedback
.tran 1n 100n
";

    const NGSPICE: &str = "\
* ngspice deck
.options reltol=1e-4
.control
run
plot v(out)
.endc
V1 in 0 pulse(0 1 1n 1n)
R1 in out 1k ; series
";

    const LTSPICE: &str = "\
* ltspice deck
.step param R list 1k 2k 5k
.tran 10u
R1 a b {R} ; gain
";

    const SPECTRE: &str = "\
// spectre netlist
parameters vdd=1.2
Vdd (vdd 0) vsource dc=vdd
M1 (d g 0 0) nch w=1u l=1u
";

    #[test]
    fn detects_hspice() {
        assert_eq!(detect_dialect(HSPICE), DialectKind::Hspice);
    }

    #[test]
    fn detects_ngspice() {
        assert_eq!(detect_dialect(NGSPICE), DialectKind::Ngspice);
    }

    #[test]
    fn detects_ltspice() {
        assert_eq!(detect_dialect(LTSPICE), DialectKind::Ltspice);
    }

    #[test]
    fn detects_spectre() {
        assert_eq!(detect_dialect(SPECTRE), DialectKind::Spectre);
    }

    #[test]
    fn plain_defaults_to_hspice() {
        assert_eq!(
            detect_dialect("RC lowpass\nR1 a b 1k\nC1 b 0 1p\n.tran 1n 1u\n"),
            DialectKind::Hspice
        );
    }

    #[test]
    fn semicolon_comment_prefers_ngspice() {
        assert_eq!(detect_dialect("R1 a b 1k ; series\n"), DialectKind::Ngspice);
    }

    #[test]
    fn hspice_subckt_param_parens_not_spectre() {
        assert_eq!(
            detect_dialect("X1 a b inv (w=1u l=1u)\n"),
            DialectKind::Hspice
        );
    }

    #[test]
    fn quoted_delims_ignored() {
        assert_eq!(detect_dialect(".param msg=\"a;b\"\n"), DialectKind::Hspice);
    }

    #[test]
    fn detects_csparam() {
        assert_eq!(
            detect_dialect(".csparam vddr = {vddr}\n"),
            DialectKind::Ngspice
        );
    }

    #[test]
    fn control_block_is_decisive_over_weak_majority() {
        let deck = ".option post\n".repeat(20) + &".probe v(out)\n".repeat(20) + ".control\nrun\n.endc\n";
        assert_eq!(detect_dialect(&deck), DialectKind::Ngspice);
    }

    #[test]
    fn weak_majority_still_wins_without_decisive_evidence() {
        let deck = ".option post\n".repeat(10) + ".meas tran td rise=1\n";
        assert_eq!(detect_dialect(&deck), DialectKind::Hspice);
    }

    #[test]
    fn meas_dollar_amp_dereference_is_decisive_ngspice() {
        let deck = ".option post\n".repeat(20)
            + &".probe v(out)\n".repeat(20)
            + ".meas tran ir AVG v(n) FROM $&t1 TO=$&t2\n";
        assert_eq!(detect_dialect(&deck), DialectKind::Ngspice);
    }

    #[test]
    fn dollar_amp_is_not_hspice_comment_evidence() {
        assert_eq!(detect_dialect("B1 x 0 V=a-$&b\n"), DialectKind::Ngspice);
    }

    #[test]
    fn conflicting_decisive_falls_back_to_weights() {
        // .control (ngspice) vs .backanno (ltspice); weak evidence breaks it.
        let deck = ".control\nrun\n.endc\n; tune\n.backanno\n";
        assert_eq!(detect_dialect(deck), DialectKind::Ngspice);
    }
}
