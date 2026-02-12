use crate::v03::expression::bytecode::{BytecodeProgram, ConstVal, Op};
use crate::v03::expression::program::{ExprProgram, PropertyEntry, PropertyProgram, ValueType};
use crate::v03::expression::vm::{ValueSlot, eval_program};
use crate::v03::foundation::ids::{NodeIdx, PropertyId, VarId};
use std::time::Instant;

#[allow(clippy::needless_range_loop)]
fn make_chain_program(n: usize) -> ExprProgram {
    // pid i depends on pid i-1. Each program is: load(prev) + 1.
    let mut entries = Vec::with_capacity(n);
    let mut entry_by_pid = vec![None; n];
    let mut eval_order = Vec::with_capacity(n);

    for i in 0..n {
        let pid = PropertyId(i as u32);
        let program = if i == 0 {
            let mut bc = BytecodeProgram::new();
            let c1 = bc.push_const(ConstVal::F64(1.0));
            bc.ops.push(Op::PushConst(c1));
            PropertyProgram::Expr(bc)
        } else {
            let mut bc = BytecodeProgram::new();
            let c1 = bc.push_const(ConstVal::F64(1.0));
            bc.ops.push(Op::LoadProp(PropertyId((i - 1) as u32)));
            bc.ops.push(Op::PushConst(c1));
            bc.ops.push(Op::Add);
            PropertyProgram::Expr(bc)
        };

        entries.push(PropertyEntry {
            pid,
            owner_node: NodeIdx(0),
            value_type: ValueType::F64,
            program,
        });
        entry_by_pid[i] = Some(i as u32);
        eval_order.push(pid);
    }

    ExprProgram {
        eval_order,
        entries,
        entry_by_pid,
    }
}

fn eval_chain(program: &ExprProgram, iters: usize) -> f64 {
    let mut values = vec![ValueSlot::F64(0.0); program.entry_by_pid.len()];
    let mut acc = 0.0f64;

    for _ in 0..iters {
        for &pid in &program.eval_order {
            let entry_idx = program.entry_by_pid[pid.0 as usize].unwrap() as usize;
            let entry = &program.entries[entry_idx];
            match &entry.program {
                PropertyProgram::Expr(bc) => {
                    let v = eval_program(
                        bc,
                        |dep| Ok(values[dep.0 as usize]),
                        |_var: VarId| {
                            Err(crate::v03::expression::vm::VmError {
                                message: "vars not supported in perf microbench".to_owned(),
                            })
                        },
                        |_tf| {
                            Err(crate::v03::expression::vm::VmError {
                                message: "time not supported in perf microbench".to_owned(),
                            })
                        },
                    )
                    .unwrap();
                    values[pid.0 as usize] = v;
                }
                PropertyProgram::SampleNodeLane { .. } => {
                    // Not used in this microbench.
                }
            }
        }
        if let ValueSlot::F64(v) = values[values.len() - 1] {
            acc += v;
        }
    }

    std::hint::black_box(acc)
}

#[test]
fn expr_vm_perf_microbench_gate() {
    // A conservative perf gate meant to catch pathological regressions (O(N^2), allocations, etc).
    // Thresholds should be tuned for this VPS; they’re intentionally loose to avoid flakiness.
    let p500 = make_chain_program(500);
    let p2000 = make_chain_program(2000);

    // Warmup
    let _ = eval_chain(&p500, 10);
    let _ = eval_chain(&p2000, 2);

    let t0 = Instant::now();
    let _ = eval_chain(&p500, 200);
    let d500 = t0.elapsed();

    let t1 = Instant::now();
    let _ = eval_chain(&p2000, 80);
    let d2000 = t1.elapsed();

    // These should be revisited after a few runs on the actual VPS to tighten.
    assert!(
        d500.as_millis() < 2500,
        "expr microbench 500 too slow: {d500:?}"
    );
    assert!(
        d2000.as_millis() < 4500,
        "expr microbench 2000 too slow: {d2000:?}"
    );
}
