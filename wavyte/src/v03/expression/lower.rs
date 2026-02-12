use crate::v03::expression::ast::{BinaryOp, Expr, Lit, UnaryOp};
use crate::v03::expression::bytecode::{BuiltinId, BytecodeProgram, ConstVal, Op};
use crate::v03::expression::error::ExprError;

pub(crate) fn lower_to_bytecode(expr: &Expr) -> Result<BytecodeProgram, ExprError> {
    let mut p = BytecodeProgram::new();
    lower_expr(expr, &mut p)?;
    Ok(p)
}

fn lower_expr(e: &Expr, out: &mut BytecodeProgram) -> Result<(), ExprError> {
    match e {
        Expr::Lit(Lit::F64(v)) => {
            let idx = out.push_const(ConstVal::F64(*v));
            out.ops.push(Op::PushConst(idx));
            Ok(())
        }
        Expr::Lit(Lit::Bool(v)) => {
            let idx = out.push_const(ConstVal::Bool(*v));
            out.ops.push(Op::PushConst(idx));
            Ok(())
        }
        Expr::Unary { op, expr } => {
            lower_expr(expr, out)?;
            out.ops.push(match op {
                UnaryOp::Neg => Op::Neg,
                UnaryOp::Not => Op::Not,
            });
            Ok(())
        }
        Expr::Binary { op, left, right } => {
            lower_expr(left, out)?;
            lower_expr(right, out)?;
            out.ops.push(match op {
                BinaryOp::Add => Op::Add,
                BinaryOp::Sub => Op::Sub,
                BinaryOp::Mul => Op::Mul,
                BinaryOp::Div => Op::Div,
                BinaryOp::Mod => Op::Mod,
                BinaryOp::Eq => Op::Eq,
                BinaryOp::Ne => Op::Ne,
                BinaryOp::Lt => Op::Lt,
                BinaryOp::Le => Op::Le,
                BinaryOp::Gt => Op::Gt,
                BinaryOp::Ge => Op::Ge,
                BinaryOp::And => Op::And,
                BinaryOp::Or => Op::Or,
            });
            Ok(())
        }
        Expr::Call { func, args } => {
            for a in args {
                lower_expr(a, out)?;
            }
            let id = match func.as_str() {
                "min" => BuiltinId::Min,
                "max" => BuiltinId::Max,
                "clamp" => BuiltinId::Clamp,
                "abs" => BuiltinId::Abs,
                "sin" => BuiltinId::Sin,
                "cos" => BuiltinId::Cos,
                "lerp" => BuiltinId::Lerp,
                other => {
                    return Err(ExprError::new(
                        0,
                        format!("unknown builtin function \"{other}\""),
                    ));
                }
            };
            out.ops.push(Op::CallBuiltin {
                id,
                argc: u8::try_from(args.len()).unwrap_or(u8::MAX),
            });
            Ok(())
        }
        Expr::Path(_p) => Err(ExprError::new(
            0,
            "unresolved path in lowering; bind refs before lowering",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::expression::parser::parse_expr;
    use crate::v03::expression::vm::{ValueSlot, eval_program_noenv};

    #[test]
    fn lowers_and_evaluates_arithmetic() {
        let ast = parse_expr("=(1+2)*3").unwrap();
        let bc = lower_to_bytecode(&ast).unwrap();
        let v = eval_program_noenv(&bc).unwrap();
        assert_eq!(v, ValueSlot::F64(9.0));
    }

    #[test]
    fn lowers_and_evaluates_builtins() {
        let ast = parse_expr("clamp(-1, 0, 10)").unwrap();
        let bc = lower_to_bytecode(&ast).unwrap();
        let v = eval_program_noenv(&bc).unwrap();
        assert_eq!(v, ValueSlot::F64(0.0));
    }
}
