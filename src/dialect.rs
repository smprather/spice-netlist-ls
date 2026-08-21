use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DialectKind {
    Hspice,
    Ngspice,
    Spectre,
    Ltspice,
}

impl DialectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hspice => "hspice",
            Self::Ngspice => "ngspice",
            Self::Spectre => "spectre",
            Self::Ltspice => "ltspice",
        }
    }

    pub fn all() -> &'static [DialectKind] {
        &[Self::Hspice, Self::Ngspice, Self::Spectre, Self::Ltspice]
    }
}

pub trait Dialect: Send + Sync {
    fn kind(&self) -> DialectKind;
    fn name(&self) -> &'static str {
        self.kind().as_str()
    }
    fn continuation_char(&self) -> char {
        '+'
    }
    fn is_comment_line(&self, trimmed: &str) -> bool {
        trimmed.starts_with('*') || trimmed.starts_with('$')
    }
    fn inline_comment_delim(&self) -> Option<char> {
        Some('$')
    }
    fn directive_prefix(&self) -> char {
        '.'
    }
    fn max_line_length(&self) -> usize {
        120
    }
    fn directive_case_lower(&self) -> bool {
        true
    }
    fn continuation_indent(&self) -> &'static str {
        "+ "
    }
    /// `"key = value"` when true (HSPICE house style); `"key=value"` for
    /// spectre, whose own examples and most netlists in the wild run them
    /// together. Only affects formatter output — both parse identically.
    fn space_around_eq(&self) -> bool {
        true
    }
    fn is_spectre(&self) -> bool {
        self.kind() == DialectKind::Spectre
    }
}

#[derive(Clone, Debug, Default)]
pub struct HspiceDialect;
impl Dialect for HspiceDialect {
    fn kind(&self) -> DialectKind {
        DialectKind::Hspice
    }
}

#[derive(Clone, Debug, Default)]
pub struct NgspiceDialect;
impl Dialect for NgspiceDialect {
    fn kind(&self) -> DialectKind {
        DialectKind::Ngspice
    }
    fn inline_comment_delim(&self) -> Option<char> {
        Some(';')
    }
}

#[derive(Clone, Debug, Default)]
pub struct SpectreDialect;
impl Dialect for SpectreDialect {
    fn kind(&self) -> DialectKind {
        DialectKind::Spectre
    }
    fn continuation_char(&self) -> char {
        '+'
    }
    fn is_comment_line(&self, trimmed: &str) -> bool {
        trimmed.starts_with("//") || trimmed.starts_with('*')
    }
    fn inline_comment_delim(&self) -> Option<char> {
        Some('/')
    }
    fn directive_prefix(&self) -> char {
        '.'
    }
    fn space_around_eq(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Default)]
pub struct LtspiceDialect;
impl Dialect for LtspiceDialect {
    fn kind(&self) -> DialectKind {
        DialectKind::Ltspice
    }
    fn inline_comment_delim(&self) -> Option<char> {
        Some(';')
    }
}

pub fn get_dialect(kind: DialectKind) -> Arc<dyn Dialect> {
    match kind {
        DialectKind::Hspice => Arc::new(HspiceDialect),
        DialectKind::Ngspice => Arc::new(NgspiceDialect),
        DialectKind::Spectre => Arc::new(SpectreDialect),
        DialectKind::Ltspice => Arc::new(LtspiceDialect),
    }
}

pub fn dialect_from_str(s: &str) -> Option<DialectKind> {
    match s.to_ascii_lowercase().as_str() {
        "hspice" | "hsp" => Some(DialectKind::Hspice),
        "ngspice" | "ng" => Some(DialectKind::Ngspice),
        "spectre" | "specter" => Some(DialectKind::Spectre),
        "ltspice" | "lt" => Some(DialectKind::Ltspice),
        _ => None,
    }
}
