use crate::expression::bind::{BindCtx, bind_expr};
use crate::expression::error::ExprError;
use crate::expression::lower::lower_to_bytecode;
use crate::expression::parser::parse_expr;
use crate::expression::program::{ExprProgram, PropertyEntry, PropertyProgram, ValueType};
use crate::foundation::ids::{NodeIdx, PropertyId};
use crate::normalize::ir::{ExprSourceIR, NormalizedComposition, ValueTypeIR};
use crate::normalize::property::PropertyKey;
use std::collections::BTreeSet;

#[derive(Debug)]
pub(crate) struct ExprCompileError {
    pub(crate) message: String,
}

impl ExprCompileError {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for ExprCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expr compile error: {}", self.message)
    }
}

impl std::error::Error for ExprCompileError {}

impl From<ExprError> for ExprCompileError {
    fn from(e: ExprError) -> Self {
        Self::new(e.to_string())
    }
}

pub(crate) fn compile_expr_program(
    norm: &NormalizedComposition,
) -> Result<ExprProgram, ExprCompileError> {
    // Stage 1: compile explicit expression outputs.
    let mut entries: Vec<PropertyEntry> = Vec::with_capacity(norm.expr_sources.len());
    for src in &norm.expr_sources {
        entries.push(compile_expr_source(src, norm)?);
    }

    // Collect all required properties (expr outputs + deps).
    let mut required: BTreeSet<PropertyId> = entries.iter().map(|e| e.pid).collect();
    for e in &entries {
        if let PropertyProgram::Expr(bc) = &e.program {
            for op in &bc.ops {
                if let crate::expression::bytecode::Op::LoadProp(pid) = op {
                    required.insert(*pid);
                }
            }
        }
    }

    // Add SampleNodeLane stubs for dependencies that aren't direct expr outputs.
    let existing: BTreeSet<PropertyId> = entries.iter().map(|e| e.pid).collect();
    for pid in required.iter().copied().collect::<Vec<_>>() {
        if existing.contains(&pid) {
            continue;
        }
        let (node, lane) = pid_to_node_lane(pid)?;
        entries.push(PropertyEntry {
            pid,
            owner_node: node,
            value_type: value_type_for_lane(lane),
            program: PropertyProgram::SampleNodeLane { node, lane },
        });
    }

    // Build dense map pid->entry index.
    let max_pid = entries.iter().map(|e| e.pid.0 as usize).max().unwrap_or(0);
    let mut entry_by_pid = vec![None; max_pid + 1];
    for (i, e) in entries.iter().enumerate() {
        entry_by_pid[e.pid.0 as usize] = Some(u32::try_from(i).unwrap());
    }

    // Build dependency graph: dep -> dependent edges.
    let mut indeg = vec![0u32; entries.len()];
    let mut outs: Vec<Vec<u32>> = vec![Vec::new(); entries.len()];
    for (i, e) in entries.iter().enumerate() {
        let deps = scan_deps(&e.program);
        for dep_pid in deps {
            let Some(dep_i) = entry_by_pid.get(dep_pid.0 as usize).and_then(|x| *x) else {
                return Err(ExprCompileError::new(format!(
                    "missing dependency entry for PropertyId({})",
                    dep_pid.0
                )));
            };
            outs[dep_i as usize].push(u32::try_from(i).unwrap());
            indeg[i] = indeg[i].saturating_add(1);
        }
    }

    // Kahn topo with deterministic tie-break on PropertyId.
    let mut ready: BTreeSet<(u32, u32)> = BTreeSet::new(); // (pid.0, entry_idx)
    for (i, &d) in indeg.iter().enumerate() {
        if d == 0 {
            ready.insert((entries[i].pid.0, i as u32));
        }
    }

    let mut eval_order = Vec::with_capacity(entries.len());
    let mut seen = 0usize;
    while let Some((_pid0, i)) = ready.pop_first() {
        seen += 1;
        eval_order.push(entries[i as usize].pid);
        for &j in &outs[i as usize] {
            let dj = &mut indeg[j as usize];
            *dj = dj.saturating_sub(1);
            if *dj == 0 {
                ready.insert((entries[j as usize].pid.0, j));
            }
        }
    }

    if seen != entries.len() {
        let cycle = find_cycle(&entries, &outs);
        if cycle.is_empty() {
            return Err(ExprCompileError::new(
                "expression dependency cycle detected",
            ));
        }
        let mut s = String::new();
        for (i, pid) in cycle.iter().enumerate() {
            if i > 0 {
                s.push_str(" -> ");
            }
            s.push_str(&format!("PropertyId({})", pid.0));
        }
        return Err(ExprCompileError::new(format!(
            "expression dependency cycle detected: {s}"
        )));
    }

    Ok(ExprProgram {
        eval_order,
        entries,
        entry_by_pid,
    })
}

fn compile_expr_source(
    src: &ExprSourceIR,
    norm: &NormalizedComposition,
) -> Result<PropertyEntry, ExprCompileError> {
    let owner_node = pid_to_owner_node(src.target);
    let expr_src = norm.interner.get(src.src);

    let ast = parse_expr(expr_src)?;
    let ctx = BindCtx {
        owner_node,
        interner: &norm.interner,
        node_idx_by_id: &norm.node_idx_by_id,
        var_id_by_key: &norm.var_id_by_key,
    };
    let bound = bind_expr(ast, &ctx)?;
    let bc = lower_to_bytecode(&bound)?;

    Ok(PropertyEntry {
        pid: src.target,
        owner_node,
        value_type: map_value_type(src.value_type),
        program: PropertyProgram::Expr(bc),
    })
}

fn map_value_type(v: ValueTypeIR) -> ValueType {
    match v {
        ValueTypeIR::Bool => ValueType::Bool,
        ValueTypeIR::F64 => ValueType::F64,
        ValueTypeIR::U64 => ValueType::U64,
        ValueTypeIR::Color => ValueType::Color,
    }
}

fn pid_to_owner_node(pid: PropertyId) -> NodeIdx {
    // pid = node * COUNT + key
    let node = pid.0 / PropertyKey::COUNT;
    NodeIdx(node)
}

fn pid_to_node_lane(pid: PropertyId) -> Result<(NodeIdx, PropertyKey), ExprCompileError> {
    let node = pid.0 / PropertyKey::COUNT;
    let key = pid.0 % PropertyKey::COUNT;
    let Some(lane) = property_key_from_u32(key) else {
        return Err(ExprCompileError::new(format!(
            "invalid property key {} in PropertyId({})",
            key, pid.0
        )));
    };
    Ok((NodeIdx(node), lane))
}

fn property_key_from_u32(v: u32) -> Option<PropertyKey> {
    match v {
        0 => Some(PropertyKey::Opacity),
        1 => Some(PropertyKey::TransformTranslateX),
        2 => Some(PropertyKey::TransformTranslateY),
        3 => Some(PropertyKey::TransformRotationRad),
        4 => Some(PropertyKey::TransformScaleX),
        5 => Some(PropertyKey::TransformScaleY),
        6 => Some(PropertyKey::TransformAnchorX),
        7 => Some(PropertyKey::TransformAnchorY),
        8 => Some(PropertyKey::TransformSkewX),
        9 => Some(PropertyKey::TransformSkewY),
        10 => Some(PropertyKey::SwitchActiveIndex),
        11 => Some(PropertyKey::LayoutX),
        12 => Some(PropertyKey::LayoutY),
        13 => Some(PropertyKey::LayoutWidth),
        14 => Some(PropertyKey::LayoutHeight),
        15 => Some(PropertyKey::LayoutGapX),
        16 => Some(PropertyKey::LayoutGapY),
        17 => Some(PropertyKey::LayoutPaddingTopPx),
        18 => Some(PropertyKey::LayoutPaddingRightPx),
        19 => Some(PropertyKey::LayoutPaddingBottomPx),
        20 => Some(PropertyKey::LayoutPaddingLeftPx),
        21 => Some(PropertyKey::LayoutMarginTopPx),
        22 => Some(PropertyKey::LayoutMarginRightPx),
        23 => Some(PropertyKey::LayoutMarginBottomPx),
        24 => Some(PropertyKey::LayoutMarginLeftPx),
        25 => Some(PropertyKey::LayoutFlexGrow),
        26 => Some(PropertyKey::LayoutFlexShrink),
        27 => Some(PropertyKey::LayoutWidthPx),
        28 => Some(PropertyKey::LayoutHeightPx),
        29 => Some(PropertyKey::LayoutMinWidthPx),
        30 => Some(PropertyKey::LayoutMinHeightPx),
        31 => Some(PropertyKey::LayoutMaxWidthPx),
        32 => Some(PropertyKey::LayoutMaxHeightPx),
        _ => None,
    }
}

fn value_type_for_lane(lane: PropertyKey) -> ValueType {
    match lane {
        PropertyKey::SwitchActiveIndex => ValueType::U64,
        _ => ValueType::F64,
    }
}

fn scan_deps(p: &PropertyProgram) -> Vec<PropertyId> {
    match p {
        PropertyProgram::Expr(bc) => bc
            .ops
            .iter()
            .filter_map(|op| match op {
                crate::expression::bytecode::Op::LoadProp(pid) => Some(*pid),
                _ => None,
            })
            .collect(),
        PropertyProgram::SampleNodeLane { .. } => Vec::new(),
    }
}

fn find_cycle(entries: &[PropertyEntry], outs: &[Vec<u32>]) -> Vec<PropertyId> {
    let n = entries.len();
    let mut state = vec![0u8; n]; // 0=unvisited,1=visiting,2=done
    let mut stack: Vec<u32> = Vec::new();

    fn dfs(
        v: u32,
        entries: &[PropertyEntry],
        outs: &[Vec<u32>],
        state: &mut [u8],
        stack: &mut Vec<u32>,
    ) -> Option<Vec<PropertyId>> {
        state[v as usize] = 1;
        stack.push(v);
        for &to in &outs[v as usize] {
            let st = state[to as usize];
            if st == 0 {
                if let Some(c) = dfs(to, entries, outs, state, stack) {
                    return Some(c);
                }
            } else if st == 1 {
                let pos = stack.iter().position(|&x| x == to).unwrap_or(0);
                let mut cycle: Vec<PropertyId> = stack[pos..]
                    .iter()
                    .map(|&i| entries[i as usize].pid)
                    .collect();
                cycle.push(entries[to as usize].pid);
                return Some(cycle);
            }
        }
        stack.pop();
        state[v as usize] = 2;
        None
    }

    for i in 0..n {
        if state[i] == 0
            && let Some(c) = dfs(i as u32, entries, outs, &mut state, &mut stack)
        {
            return c;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::anim::{AnimDef, AnimTaggedDef};
    use crate::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, FpsDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn compile_orders_deps_before_dependents() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 1,
                height: 1,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 10,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Group,
                    children: vec![NodeDef {
                        id: "a".to_owned(),
                        kind: NodeKindDef::Leaf {
                            asset: "a".to_owned(),
                        },
                        range: [0, 10],
                        transform: Default::default(),
                        opacity: AnimDef::Tagged(AnimTaggedDef::Expr("=1+2".to_owned())),
                        blend: Default::default(),
                        layout: None,
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    }],
                },
                range: [0, 10],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                blend: Default::default(),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = crate::normalize::pass::normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        assert!(!program.eval_order.is_empty());
    }
}
