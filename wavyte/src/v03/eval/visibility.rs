use crate::v03::animation::anim::{Anim, SampleCtx};
use crate::v03::eval::context::NodeTimeCtx;
use crate::v03::eval::properties::PropertyValues;
use crate::v03::expression::vm::VmError;
use crate::v03::foundation::ids::NodeIdx;
use crate::v03::normalize::ir::{CollectionModeIR, CompositionIR, NodeKindIR};

#[derive(Debug, Default)]
pub(crate) struct VisibilityState {
    pub(crate) node_visible: Vec<bool>,
    /// Only meaningful for `Switch` nodes.
    pub(crate) switch_active_child: Vec<Option<NodeIdx>>,
}

pub(crate) fn compute_visibility(
    ir: &CompositionIR,
    time_ctxs: &[NodeTimeCtx],
    props: Option<&PropertyValues>,
    out: &mut VisibilityState,
) -> Result<(), VmError> {
    let n = ir.nodes.len();
    out.node_visible.clear();
    out.node_visible.resize(n, false);
    out.switch_active_child.clear();
    out.switch_active_child.resize(n, None);

    let mut stack: Vec<(NodeIdx, bool)> = Vec::with_capacity(64);
    stack.push((ir.root, true));

    while let Some((idx, parent_visible)) = stack.pop() {
        let t = time_ctxs
            .get(idx.0 as usize)
            .copied()
            .ok_or_else(|| VmError::new("node time ctx out of range"))?;
        let in_range = t.is_in_range();
        let visible_here = parent_visible && in_range;
        out.node_visible[idx.0 as usize] = visible_here;

        let node = &ir.nodes[idx.0 as usize];
        let NodeKindIR::Collection { mode, children, .. } = &node.kind else {
            continue;
        };

        match mode {
            CollectionModeIR::Group | CollectionModeIR::Stack | CollectionModeIR::Sequence => {
                for &c in children.iter().rev() {
                    stack.push((c, visible_here));
                }
            }
            CollectionModeIR::Switch => {
                // Only the active child participates in layout and DFS traversal.
                let active_i = if visible_here {
                    switch_active_index(ir, time_ctxs, idx, props)?
                } else {
                    0
                };
                let active_child = children.get(active_i as usize).copied();
                out.switch_active_child[idx.0 as usize] = active_child;

                for &c in children.iter().rev() {
                    stack.push((c, visible_here && Some(c) == active_child));
                }
            }
        }
    }

    Ok(())
}

fn switch_active_index(
    ir: &CompositionIR,
    time_ctxs: &[NodeTimeCtx],
    switch_node: NodeIdx,
    props: Option<&PropertyValues>,
) -> Result<u64, VmError> {
    let node = ir
        .nodes
        .get(switch_node.0 as usize)
        .ok_or_else(|| VmError::new("switch node idx out of range"))?;
    let Some(anim) = node.props.switch_active.as_ref() else {
        return Ok(0);
    };
    let t = time_ctxs
        .get(switch_node.0 as usize)
        .copied()
        .ok_or_else(|| VmError::new("switch node time ctx out of range"))?;
    let frame = t.sample_frame_u64();

    match anim {
        Anim::Constant(v) => Ok(*v),
        Anim::Keyframes(k) => Ok(k.sample(frame)),
        Anim::Procedural(p) => Ok(p.sample(SampleCtx {
            fps: ir.fps,
            frame,
            seed: ir.seed,
        })),
        Anim::Reference(pid) => {
            let Some(props) = props else {
                return Err(VmError::new(
                    "switch.active references an expression property but no properties were provided",
                ));
            };
            props.get(*pid)?.as_u64_floor()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::{AnimDef, AnimTaggedDef};
    use crate::v03::eval::properties::{PropertyEvalScratch, eval_expr_program_frame};
    use crate::v03::eval::time::compute_node_time_ctxs;
    use crate::v03::expression::compile::compile_expr_program;
    use crate::v03::normalize::pass::normalize;
    use crate::v03::normalize::property::{PropertyIndex, PropertyKey};
    use crate::v03::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, FpsDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn switch_selects_only_active_child_for_visibility() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let child0 = NodeDef {
            id: "c0".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };
        let child1 = NodeDef {
            id: "c1".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 1,
                height: 1,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 20,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Switch {
                        active: AnimDef::Constant(1),
                    },
                    children: vec![child0, child1],
                },
                range: [0, 20],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();

        let mut t = Vec::new();
        compute_node_time_ctxs(&norm.ir, 0, &mut t);

        let mut vis = VisibilityState::default();
        compute_visibility(&norm.ir, &t, None, &mut vis).unwrap();

        let c0_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("c0").unwrap())
            .unwrap();
        let c1_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("c1").unwrap())
            .unwrap();

        assert!(!vis.node_visible[c0_idx.0 as usize]);
        assert!(vis.node_visible[c1_idx.0 as usize]);
    }

    #[test]
    fn switch_active_expr_is_resolved_via_property_values() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let child0 = NodeDef {
            id: "c0".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };
        let child1 = NodeDef {
            id: "c1".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 1,
                height: 1,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 20,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Switch {
                        active: AnimDef::Tagged(AnimTaggedDef::Expr("=1".to_owned())),
                    },
                    children: vec![child0, child1],
                },
                range: [0, 20],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut values = PropertyValues::new(&program);
        let mut scratch = PropertyEvalScratch::new();

        let mut t = Vec::new();
        compute_node_time_ctxs(&norm.ir, 0, &mut t);
        eval_expr_program_frame(&norm.ir, &t, &program, &mut values, &mut scratch).unwrap();

        // Sanity: ensure we actually evaluated the switch.active property.
        let root_pid = PropertyIndex::property_id(norm.ir.root, PropertyKey::SwitchActiveIndex);
        assert_eq!(values.get(root_pid).unwrap().as_u64_floor().unwrap(), 1);

        let mut vis = VisibilityState::default();
        compute_visibility(&norm.ir, &t, Some(&values), &mut vis).unwrap();

        let c0_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("c0").unwrap())
            .unwrap();
        let c1_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("c1").unwrap())
            .unwrap();

        assert!(!vis.node_visible[c0_idx.0 as usize]);
        assert!(vis.node_visible[c1_idx.0 as usize]);
    }
}
