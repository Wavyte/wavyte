use crate::foundation::ids::{PropertyId, VarId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConstIdx(pub(crate) u32);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConstVal {
    F64(f64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuiltinId {
    Min,
    Max,
    Clamp,
    Abs,
    Sin,
    Cos,
    Lerp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeField {
    Frame,
    Fps,
    Duration,
    Progress,
    Seconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    PushConst(ConstIdx),

    // Resolved in later stages (Phase 2.3+).
    LoadProp(PropertyId),
    LoadVar(VarId),
    LoadTime(TimeField),

    Neg,
    Not,
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

    CallBuiltin { id: BuiltinId, argc: u8 },
}

#[derive(Debug, Clone)]
pub(crate) struct BytecodeProgram {
    pub(crate) ops: Vec<Op>,
    pub(crate) consts: Vec<ConstVal>,
}

impl BytecodeProgram {
    pub(crate) fn new() -> Self {
        Self {
            ops: Vec::new(),
            consts: Vec::new(),
        }
    }

    pub(crate) fn push_const(&mut self, c: ConstVal) -> ConstIdx {
        let idx = ConstIdx(self.consts.len() as u32);
        self.consts.push(c);
        idx
    }
}
