use std::borrow::Cow;

/// Parsed netlist. Borrows token text from the input wherever possible;
/// only synthesized text (lowercased directive names, continuation-joined
/// lines) is owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File<'a> {
    pub stmts: Vec<Stmt<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt<'a> {
    Blank,
    Comment(Cow<'a, str>),
    Directive(Directive<'a>),
    Instance(Instance<'a>),
    Subckt(Subckt<'a>),
    /// Source text to emit verbatim (fmt: off/on/skip regions). Produced
    /// only by the formatter's fmt-directive rewrite pass.
    Verbatim(Cow<'a, str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive<'a> {
    pub name: Cow<'a, str>,
    pub args: Vec<Cow<'a, str>>,
    pub params: Vec<Param<'a>>,
    pub inline_comment: Option<Cow<'a, str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance<'a> {
    pub name: Cow<'a, str>,
    pub nodes: Vec<Cow<'a, str>>,
    pub model_or_value: Option<Cow<'a, str>>,
    pub params: Vec<Param<'a>>,
    pub inline_comment: Option<Cow<'a, str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param<'a> {
    pub key: Cow<'a, str>,
    pub value: Cow<'a, str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subckt<'a> {
    pub name: Cow<'a, str>,
    pub ports: Vec<Cow<'a, str>>,
    pub params: Vec<Param<'a>>,
    pub body: Vec<Stmt<'a>>,
    pub inline_comment: Option<Cow<'a, str>>,
    /// Name token carried by the original `.ends <name>` line when it differs
    /// from `name`. When `Some`, the formatter emits `.ends <ends_name>` so
    /// a mismatch is preserved across round-trips (the linter warns).
    pub ends_name: Option<Cow<'a, str>>,
    /// Original `.ends ...` line text, emitted verbatim when the line falls
    /// inside a `fmt: off`/`fmt: skip` region. Set only by the formatter's
    /// fmt-directive rewrite pass.
    pub ends_raw: Option<Cow<'a, str>>,
}

impl<'a> File<'a> {
    pub fn new(stmts: Vec<Stmt<'a>>) -> Self {
        Self { stmts }
    }
}

/// Deep-convert to an owned (`'static`) tree. Used for statements parsed
/// from continuation-joined text, which does not live in the input.
fn owned(c: Cow<'_, str>) -> Cow<'static, str> {
    Cow::Owned(c.into_owned())
}

impl Stmt<'_> {
    pub fn into_owned(self) -> Stmt<'static> {
        match self {
            Stmt::Blank => Stmt::Blank,
            Stmt::Comment(c) => Stmt::Comment(owned(c)),
            Stmt::Directive(d) => Stmt::Directive(d.into_owned()),
            Stmt::Instance(i) => Stmt::Instance(i.into_owned()),
            Stmt::Subckt(s) => Stmt::Subckt(s.into_owned()),
            Stmt::Verbatim(c) => Stmt::Verbatim(owned(c)),
        }
    }
}

impl Directive<'_> {
    pub fn into_owned(self) -> Directive<'static> {
        Directive {
            name: owned(self.name),
            args: self.args.into_iter().map(owned).collect(),
            params: self.params.into_iter().map(Param::into_owned).collect(),
            inline_comment: self.inline_comment.map(owned),
        }
    }
}

impl Instance<'_> {
    pub fn into_owned(self) -> Instance<'static> {
        Instance {
            name: owned(self.name),
            nodes: self.nodes.into_iter().map(owned).collect(),
            model_or_value: self.model_or_value.map(owned),
            params: self.params.into_iter().map(Param::into_owned).collect(),
            inline_comment: self.inline_comment.map(owned),
        }
    }
}

impl Param<'_> {
    pub fn into_owned(self) -> Param<'static> {
        Param { key: owned(self.key), value: owned(self.value) }
    }
}

impl Subckt<'_> {
    pub fn into_owned(self) -> Subckt<'static> {
        Subckt {
            name: owned(self.name),
            ports: self.ports.into_iter().map(owned).collect(),
            params: self.params.into_iter().map(Param::into_owned).collect(),
            body: self.body.into_iter().map(Stmt::into_owned).collect(),
            inline_comment: self.inline_comment.map(owned),
            ends_name: self.ends_name.map(owned),
            ends_raw: self.ends_raw.map(owned),
        }
    }
}
