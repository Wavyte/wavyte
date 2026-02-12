use crate::foundation::core::Affine;
use crate::v03::compile::plan::{
    BlendMode, CompositeOp, MaskGenSource, MaskMode, Op, OpId, OpKind, PassFx, PixelFormat,
    RenderPlan, SurfaceDesc, SurfaceId, UnitKey,
};
use crate::v03::eval::evaluator::{EvaluatedGraph, RenderUnitKind};
use crate::v03::normalize::ir::{CompositionIR, MaskModeIR, MaskSourceIR};
use smallvec::{SmallVec, smallvec};

pub(crate) fn compile_frame(ir: &CompositionIR, eval: &EvaluatedGraph) -> RenderPlan {
    let mut surfaces = Vec::<SurfaceDesc>::new();
    let mut ops = Vec::<Op>::new();

    // Surface 0 is the final canvas surface.
    let root = SurfaceId(0);
    surfaces.push(SurfaceDesc {
        width: ir.canvas.width,
        height: ir.canvas.height,
        format: PixelFormat::Rgba8Premul,
    });

    // Last writer per surface to generate explicit deps.
    let mut last_write: Vec<Option<OpId>> = vec![None; surfaces.len()];

    let mut root_written = false;

    for unit in &eval.units {
        let unit_node = match unit.kind {
            RenderUnitKind::Leaf(n) => n,
            RenderUnitKind::Group(n) => n,
        };
        let node = &ir.nodes[unit_node.0 as usize];

        let has_transition = unit.transition_in.is_some() || unit.transition_out.is_some();
        let needs_isolation = matches!(unit.kind, RenderUnitKind::Group(_))
            || node.mask.is_some()
            || !node.effects.is_empty()
            || has_transition;

        if needs_isolation {
            // Draw into a dedicated unit surface; then composite onto root.
            let unit_surface = alloc_surface_like(&mut surfaces, &mut last_write, root);

            push_op(
                &mut ops,
                &mut last_write,
                OpKind::Draw {
                    unit: map_unit_key(unit.kind),
                    leaves: unit.leaf_range.clone(),
                    clear_to_transparent: true,
                    transform_post: Affine::IDENTITY,
                    opacity_mul: 1.0,
                },
                smallvec![],
                unit_surface,
            );

            let mut current_surface = unit_surface;
            if let Some(mask) = node.mask.as_ref() {
                let mask_surface = compile_mask_source(
                    ir,
                    eval,
                    &mut surfaces,
                    &mut ops,
                    &mut last_write,
                    root,
                    &mask.source,
                );
                let masked_surface = alloc_surface_like(&mut surfaces, &mut last_write, root);
                push_op(
                    &mut ops,
                    &mut last_write,
                    OpKind::Pass {
                        fx: PassFx::MaskApply {
                            mode: mask_mode(&mask.mode),
                            inverted: mask.inverted,
                        },
                    },
                    smallvec![current_surface, mask_surface],
                    masked_surface,
                );
                current_surface = masked_surface;
            }

            let mut cop = SmallVec::<[CompositeOp; 8]>::new();
            cop.push(CompositeOp::Over {
                src: current_surface,
                opacity: 1.0,
                blend: BlendMode::Normal,
            });

            let mut inputs = SmallVec::<[SurfaceId; 4]>::new();
            if root_written {
                inputs.push(root);
            }
            inputs.push(current_surface);

            push_op(
                &mut ops,
                &mut last_write,
                OpKind::Composite {
                    clear_to_transparent: !root_written,
                    ops: Box::new(cop),
                },
                inputs,
                root,
            );

            root_written = true;
        } else {
            // Direct draw into root (no intermediate surface).
            let mut inputs = SmallVec::<[SurfaceId; 4]>::new();
            if root_written {
                inputs.push(root);
            }

            push_op(
                &mut ops,
                &mut last_write,
                OpKind::Draw {
                    unit: map_unit_key(unit.kind),
                    leaves: unit.leaf_range.clone(),
                    clear_to_transparent: !root_written,
                    transform_post: Affine::IDENTITY,
                    opacity_mul: 1.0,
                },
                inputs,
                root,
            );
            root_written = true;
        }
    }

    RenderPlan {
        surfaces,
        ops,
        roots: smallvec![root],
    }
}

fn map_unit_key(k: RenderUnitKind) -> UnitKey {
    match k {
        RenderUnitKind::Leaf(n) => UnitKey::Leaf(n),
        RenderUnitKind::Group(n) => UnitKey::Group(n),
    }
}

fn mask_mode(m: &MaskModeIR) -> MaskMode {
    match *m {
        MaskModeIR::Alpha => MaskMode::Alpha,
        MaskModeIR::Luma => MaskMode::Luma,
        MaskModeIR::Stencil { threshold } => MaskMode::Stencil { threshold },
    }
}

fn compile_mask_source(
    ir: &CompositionIR,
    eval: &EvaluatedGraph,
    surfaces: &mut Vec<SurfaceDesc>,
    ops: &mut Vec<Op>,
    last_write: &mut Vec<Option<OpId>>,
    root: SurfaceId,
    src: &MaskSourceIR,
) -> SurfaceId {
    let mask_surface = alloc_surface_like(surfaces, last_write, root);
    match src {
        MaskSourceIR::Node(id) => {
            let node_idx = ir.node_idx_by_intern.get(id.0 as usize).and_then(|x| *x);
            let (node_idx, range) = match node_idx {
                Some(n) => {
                    let r = eval
                        .node_leaf_ranges
                        .get(n.0 as usize)
                        .cloned()
                        .unwrap_or(0..0);
                    (n, r)
                }
                None => (crate::v03::foundation::ids::NodeIdx(0), 0..0),
            };
            push_op(
                ops,
                last_write,
                OpKind::Draw {
                    unit: UnitKey::MaskNode(node_idx),
                    leaves: range,
                    clear_to_transparent: true,
                    transform_post: Affine::IDENTITY,
                    opacity_mul: 1.0,
                },
                smallvec![],
                mask_surface,
            );
        }
        MaskSourceIR::Asset(key) => {
            if let Some(asset) = ir.asset_idx_by_intern.get(key.0 as usize).and_then(|x| *x) {
                push_op(
                    ops,
                    last_write,
                    OpKind::MaskGen {
                        source: MaskGenSource::Asset(asset),
                    },
                    smallvec![],
                    mask_surface,
                );
            } else {
                push_op(
                    ops,
                    last_write,
                    OpKind::Draw {
                        unit: UnitKey::MaskNode(crate::v03::foundation::ids::NodeIdx(0)),
                        leaves: 0..0,
                        clear_to_transparent: true,
                        transform_post: Affine::IDENTITY,
                        opacity_mul: 1.0,
                    },
                    smallvec![],
                    mask_surface,
                );
            }
        }
        MaskSourceIR::Shape(shape) => {
            push_op(
                ops,
                last_write,
                OpKind::MaskGen {
                    source: MaskGenSource::Shape(shape.clone()),
                },
                smallvec![],
                mask_surface,
            );
        }
    }
    mask_surface
}

fn alloc_surface_like(
    surfaces: &mut Vec<SurfaceDesc>,
    last_write: &mut Vec<Option<OpId>>,
    template: SurfaceId,
) -> SurfaceId {
    let desc = surfaces[template.0 as usize];
    let id = SurfaceId(u32::try_from(surfaces.len()).unwrap());
    surfaces.push(desc);
    last_write.push(None);
    id
}

fn push_op(
    ops: &mut Vec<Op>,
    last_write: &mut [Option<OpId>],
    kind: OpKind,
    inputs: SmallVec<[SurfaceId; 4]>,
    output: SurfaceId,
) -> OpId {
    let id = OpId(u32::try_from(ops.len()).unwrap());
    let mut deps = SmallVec::<[OpId; 4]>::new();

    // Depend on the last writer of each input surface.
    for s in &inputs {
        if let Some(w) = last_write[s.0 as usize]
            && !deps.contains(&w)
        {
            deps.push(w);
        }
    }

    // For in-place accumulation (`clear_to_transparent == false`), the output surface is also an
    // implicit input; we already include `root` in inputs in those cases.
    if let Some(w) = last_write[output.0 as usize]
        && !deps.contains(&w)
    {
        deps.push(w);
    }

    ops.push(Op {
        id,
        kind,
        inputs,
        output,
        deps,
    });
    last_write[output.0 as usize] = Some(id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::AnimDef;
    use crate::v03::expression::compile::compile_expr_program;
    use crate::v03::normalize::pass::normalize;
    use crate::v03::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, EffectInstanceDef, FpsDef, MaskDef,
        MaskModeDef, MaskSourceDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn compile_emits_direct_root_draw_ops_for_simple_leaf_units() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 8,
                height: 8,
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
                    children: vec![
                        NodeDef {
                            id: "a".to_owned(),
                            kind: NodeKindDef::Leaf {
                                asset: "a".to_owned(),
                            },
                            range: [0, 10],
                            transform: Default::default(),
                            opacity: AnimDef::Constant(1.0),
                            layout: None,
                            effects: vec![],
                            mask: None,
                            transition_in: None,
                            transition_out: None,
                        },
                        NodeDef {
                            id: "b".to_owned(),
                            kind: NodeKindDef::Leaf {
                                asset: "a".to_owned(),
                            },
                            range: [0, 10],
                            transform: Default::default(),
                            opacity: AnimDef::Constant(1.0),
                            layout: None,
                            effects: vec![],
                            mask: None,
                            transition_in: None,
                            transition_out: None,
                        },
                    ],
                },
                range: [0, 10],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::v03::eval::evaluator::Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        let plan = compile_frame(&norm.ir, g);

        assert_eq!(plan.surfaces.len(), 1);
        assert_eq!(plan.roots.len(), 1);
        assert_eq!(plan.roots[0], SurfaceId(0));
        assert_eq!(plan.ops.len(), 2);
        assert!(matches!(plan.ops[0].kind, OpKind::Draw { .. }));
        assert!(matches!(plan.ops[1].kind, OpKind::Draw { .. }));
    }

    #[test]
    fn compile_allocates_isolation_surface_for_isolated_group_unit() {
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
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let group = NodeDef {
            id: "g".to_owned(),
            kind: NodeKindDef::Collection {
                mode: CollectionModeDef::Group,
                children: vec![child0],
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            layout: None,
            effects: vec![EffectInstanceDef {
                kind: "blur".to_owned(),
                params: BTreeMap::new(),
            }],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 8,
                height: 8,
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
                    children: vec![group],
                },
                range: [0, 10],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::v03::eval::evaluator::Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        assert_eq!(g.units.len(), 1);
        assert!(matches!(g.units[0].kind, RenderUnitKind::Group(_)));

        let plan = compile_frame(&norm.ir, g);
        assert_eq!(plan.surfaces.len(), 2);
        assert_eq!(plan.ops.len(), 2);
        assert!(matches!(plan.ops[0].kind, OpKind::Draw { .. }));
        assert!(matches!(plan.ops[1].kind, OpKind::Composite { .. }));
    }

    #[test]
    fn compile_emits_mask_surface_and_mask_apply_pass_for_group_mask() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let mask_node = NodeDef {
            id: "mask".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };
        let content = NodeDef {
            id: "content".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let masked_group = NodeDef {
            id: "g".to_owned(),
            kind: NodeKindDef::Collection {
                mode: CollectionModeDef::Group,
                children: vec![mask_node, content],
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            layout: None,
            effects: vec![],
            mask: Some(MaskDef {
                source: MaskSourceDef::Node("mask".to_owned()),
                mode: MaskModeDef::Alpha,
                inverted: false,
            }),
            transition_in: None,
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 8,
                height: 8,
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
                    children: vec![masked_group],
                },
                range: [0, 10],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::v03::eval::evaluator::Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        let plan = compile_frame(&norm.ir, g);

        assert!(
            plan.ops
                .iter()
                .any(|op| matches!(op.kind, OpKind::Pass { .. }))
        );
    }
}
