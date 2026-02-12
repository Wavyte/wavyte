use crate::v03::expression::bytecode::BytecodeProgram;
use crate::v03::foundation::ids::{NodeIdx, PropertyId};
use crate::v03::normalize::property::PropertyKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueType {
    F64,
    Bool,
    U64,
    Color,
}

#[derive(Debug, Clone)]
pub(crate) enum PropertyProgram {
    Expr(BytecodeProgram),
    SampleNodeLane { node: NodeIdx, lane: PropertyKey },
}

#[derive(Debug, Clone)]
pub(crate) struct PropertyEntry {
    pub(crate) pid: PropertyId,
    pub(crate) owner_node: NodeIdx,
    pub(crate) value_type: ValueType,
    pub(crate) program: PropertyProgram,
}

#[derive(Debug, Clone)]
pub(crate) struct ExprProgram {
    pub(crate) eval_order: Vec<PropertyId>,
    pub(crate) entries: Vec<PropertyEntry>,
    /// Dense map: `pid.0 as usize -> Some(entry_idx)`.
    pub(crate) entry_by_pid: Vec<Option<u32>>,
}
