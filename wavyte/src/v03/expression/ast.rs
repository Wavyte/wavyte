#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Lit(Lit),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        func: String,
        args: Vec<Expr>,
    },
    /// A dotted identifier path: `self.opacity`, `nodes.title.opacity`, `time.frame`.
    Path(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lit {
    F64(f64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}
