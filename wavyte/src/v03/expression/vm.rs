use crate::v03::expression::bytecode::{BuiltinId, BytecodeProgram, ConstVal, Op, TimeField};
use crate::v03::foundation::ids::{PropertyId, VarId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ValueSlot {
    F64(f64),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub(crate) struct VmError {
    pub(crate) message: String,
}

impl VmError {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vm error: {}", self.message)
    }
}

impl std::error::Error for VmError {}

pub(crate) fn eval_program_noenv(p: &BytecodeProgram) -> Result<ValueSlot, VmError> {
    eval_program(
        p,
        |_pid| Err(VmError::new("LoadProp is not supported in noenv eval")),
        |_vid| Err(VmError::new("LoadVar is not supported in noenv eval")),
        |_tf| Err(VmError::new("LoadTime is not supported in noenv eval")),
    )
}

pub(crate) fn eval_program(
    p: &BytecodeProgram,
    mut load_prop: impl FnMut(PropertyId) -> Result<ValueSlot, VmError>,
    mut load_var: impl FnMut(VarId) -> Result<ValueSlot, VmError>,
    mut load_time: impl FnMut(TimeField) -> Result<ValueSlot, VmError>,
) -> Result<ValueSlot, VmError> {
    let mut stack: Vec<ValueSlot> = Vec::with_capacity(16);

    for &op in &p.ops {
        match op {
            Op::PushConst(idx) => {
                let c = p
                    .consts
                    .get(idx.0 as usize)
                    .ok_or_else(|| VmError::new("const idx out of range"))?;
                stack.push(match *c {
                    ConstVal::F64(v) => ValueSlot::F64(v),
                    ConstVal::Bool(v) => ValueSlot::Bool(v),
                });
            }
            Op::LoadProp(pid) => stack.push(load_prop(pid)?),
            Op::LoadVar(vid) => stack.push(load_var(vid)?),
            Op::LoadTime(tf) => stack.push(load_time(tf)?),

            Op::Neg => {
                let v = pop_f64(&mut stack)?;
                stack.push(ValueSlot::F64(-v));
            }
            Op::Not => {
                let v = pop_bool(&mut stack)?;
                stack.push(ValueSlot::Bool(!v));
            }
            Op::Add => bin_f64(&mut stack, |a, b| a + b)?,
            Op::Sub => bin_f64(&mut stack, |a, b| a - b)?,
            Op::Mul => bin_f64(&mut stack, |a, b| a * b)?,
            Op::Div => bin_f64(&mut stack, |a, b| a / b)?,
            Op::Mod => bin_f64(&mut stack, |a, b| a % b)?,

            Op::Eq => bin_cmp(&mut stack, |a, b| a == b)?,
            Op::Ne => bin_cmp(&mut stack, |a, b| a != b)?,
            Op::Lt => bin_cmp(&mut stack, |a, b| a < b)?,
            Op::Le => bin_cmp(&mut stack, |a, b| a <= b)?,
            Op::Gt => bin_cmp(&mut stack, |a, b| a > b)?,
            Op::Ge => bin_cmp(&mut stack, |a, b| a >= b)?,

            Op::And => {
                let b = pop_bool(&mut stack)?;
                let a = pop_bool(&mut stack)?;
                stack.push(ValueSlot::Bool(a && b));
            }
            Op::Or => {
                let b = pop_bool(&mut stack)?;
                let a = pop_bool(&mut stack)?;
                stack.push(ValueSlot::Bool(a || b));
            }

            Op::CallBuiltin { id, argc } => {
                call_builtin(&mut stack, id, argc)?;
            }
        }
    }

    if stack.len() != 1 {
        return Err(VmError::new(format!(
            "stack has {} values at end of program",
            stack.len()
        )));
    }
    Ok(stack.pop().unwrap())
}

fn pop_f64(stack: &mut Vec<ValueSlot>) -> Result<f64, VmError> {
    match stack.pop() {
        Some(ValueSlot::F64(v)) => Ok(v),
        Some(other) => Err(VmError::new(format!("expected f64, got {other:?}"))),
        None => Err(VmError::new("stack underflow")),
    }
}

fn pop_bool(stack: &mut Vec<ValueSlot>) -> Result<bool, VmError> {
    match stack.pop() {
        Some(ValueSlot::Bool(v)) => Ok(v),
        Some(other) => Err(VmError::new(format!("expected bool, got {other:?}"))),
        None => Err(VmError::new("stack underflow")),
    }
}

fn bin_f64(stack: &mut Vec<ValueSlot>, f: impl FnOnce(f64, f64) -> f64) -> Result<(), VmError> {
    let b = pop_f64(stack)?;
    let a = pop_f64(stack)?;
    stack.push(ValueSlot::F64(f(a, b)));
    Ok(())
}

fn bin_cmp(stack: &mut Vec<ValueSlot>, f: impl FnOnce(f64, f64) -> bool) -> Result<(), VmError> {
    let b = pop_f64(stack)?;
    let a = pop_f64(stack)?;
    stack.push(ValueSlot::Bool(f(a, b)));
    Ok(())
}

fn call_builtin(stack: &mut Vec<ValueSlot>, id: BuiltinId, argc: u8) -> Result<(), VmError> {
    let argc = argc as usize;
    if stack.len() < argc {
        return Err(VmError::new("stack underflow in builtin call"));
    }

    match id {
        BuiltinId::Abs => {
            if argc != 1 {
                return Err(VmError::new("abs expects 1 arg"));
            }
            let x = pop_f64(stack)?;
            stack.push(ValueSlot::F64(x.abs()));
        }
        BuiltinId::Sin => {
            if argc != 1 {
                return Err(VmError::new("sin expects 1 arg"));
            }
            let x = pop_f64(stack)?;
            stack.push(ValueSlot::F64(x.sin()));
        }
        BuiltinId::Cos => {
            if argc != 1 {
                return Err(VmError::new("cos expects 1 arg"));
            }
            let x = pop_f64(stack)?;
            stack.push(ValueSlot::F64(x.cos()));
        }
        BuiltinId::Min => {
            if argc != 2 {
                return Err(VmError::new("min expects 2 args"));
            }
            let b = pop_f64(stack)?;
            let a = pop_f64(stack)?;
            stack.push(ValueSlot::F64(a.min(b)));
        }
        BuiltinId::Max => {
            if argc != 2 {
                return Err(VmError::new("max expects 2 args"));
            }
            let b = pop_f64(stack)?;
            let a = pop_f64(stack)?;
            stack.push(ValueSlot::F64(a.max(b)));
        }
        BuiltinId::Clamp => {
            if argc != 3 {
                return Err(VmError::new("clamp expects 3 args"));
            }
            let hi = pop_f64(stack)?;
            let lo = pop_f64(stack)?;
            let x = pop_f64(stack)?;
            stack.push(ValueSlot::F64(x.clamp(lo, hi)));
        }
        BuiltinId::Lerp => {
            if argc != 3 {
                return Err(VmError::new("lerp expects 3 args"));
            }
            let t = pop_f64(stack)?;
            let b = pop_f64(stack)?;
            let a = pop_f64(stack)?;
            stack.push(ValueSlot::F64(a + (b - a) * t));
        }
    }

    Ok(())
}
