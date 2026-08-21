#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub stmts: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Blank,
    Comment(String),
    Directive(Directive),
    Instance(Instance),
    Subckt(Subckt),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    pub name: String,
    pub args: Vec<String>,
    pub params: Vec<Param>,
    pub inline_comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub name: String,
    pub nodes: Vec<String>,
    pub model_or_value: Option<String>,
    pub params: Vec<Param>,
    pub inline_comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subckt {
    pub name: String,
    pub ports: Vec<String>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub inline_comment: Option<String>,
}

impl File {
    pub fn new(stmts: Vec<Stmt>) -> Self {
        Self { stmts }
    }
}
