use crate::foundation::core::Affine;
use crate::foundation::math::Fnv1a64;
use crate::v03::animation::anim::{Anim, SampleCtx};
use crate::v03::eval::context::NodeTimeCtx;
use crate::v03::eval::properties::{PropertyEvalScratch, PropertyValues, eval_expr_program_frame};
use crate::v03::eval::time::compute_node_time_ctxs;
use crate::v03::eval::visibility::{VisibilityState, compute_visibility};
use crate::v03::expression::program::ExprProgram;
use crate::v03::expression::vm::VmError;
use crate::v03::foundation::ids::{AssetIdx, NodeIdx};
use crate::v03::normalize::intern::InternId;
use crate::v03::normalize::ir::{
    CollectionModeIR, CompositionIR, NodeIR, NodeKindIR, NodePropsIR, TransitionSpecIR,
};
use crate::v03::normalize::property::PropertyKey;
use smallvec::SmallVec;
use std::ops::Range;

#[derive(Debug, Clone)]
pub(crate) struct EvaluatedGraph {
    pub(crate) frame: u64,
    pub(crate) leaves: Vec<EvaluatedLeaf>,
    pub(crate) groups: Vec<EvaluatedGroup>,
    pub(crate) units: Vec<RenderUnit>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvaluatedLeaf {
    pub(crate) node: NodeIdx,
    pub(crate) asset: AssetIdx,
    pub(crate) world_transform: Affine,
    pub(crate) opacity: f32,
    pub(crate) group_stack: SmallVec<[NodeIdx; 4]>,
}

#[derive(Debug, Clone)]
pub(crate) struct EvaluatedGroup {
    pub(crate) node: NodeIdx,
    pub(crate) leaf_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderUnitKind {
    Leaf(NodeIdx),
    Group(NodeIdx),
}

#[derive(Debug, Clone)]
pub(crate) struct RenderUnit {
    pub(crate) kind: RenderUnitKind,
    pub(crate) leaf_range: Range<usize>,
    pub(crate) transition_in: Option<ResolvedTransition>,
    pub(crate) transition_out: Option<ResolvedTransition>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedTransition {
    pub(crate) kind: InternId,
    pub(crate) progress: f32, // eased, in [0, 1]
}

#[derive(Debug)]
pub(crate) struct Evaluator {
    expr_program: ExprProgram,

    time_ctxs: Vec<NodeTimeCtx>,
    props: PropertyValues,
    props_scratch: PropertyEvalScratch,
    vis: VisibilityState,

    graph: EvaluatedGraph,

    group_stack: Vec<NodeIdx>,
    group_frames: Vec<GroupFrame>,
    isolated_group_stack: Vec<NodeIdx>,
}

#[derive(Debug, Clone, Copy)]
struct GroupFrame {
    idx: NodeIdx,
    start_leaf: usize,
    isolate: bool,
}

impl Evaluator {
    pub(crate) fn new(expr_program: ExprProgram) -> Self {
        let props = PropertyValues::new(&expr_program);
        Self {
            expr_program,
            time_ctxs: Vec::new(),
            props,
            props_scratch: PropertyEvalScratch::new(),
            vis: VisibilityState::default(),
            graph: EvaluatedGraph {
                frame: 0,
                leaves: Vec::new(),
                groups: Vec::new(),
                units: Vec::new(),
            },
            group_stack: Vec::with_capacity(8),
            group_frames: Vec::with_capacity(8),
            isolated_group_stack: Vec::with_capacity(4),
        }
    }

    pub(crate) fn eval_frame(
        &mut self,
        ir: &CompositionIR,
        global_frame: u64,
    ) -> Result<&EvaluatedGraph, VmError> {
        compute_node_time_ctxs(ir, global_frame, &mut self.time_ctxs);

        eval_expr_program_frame(
            ir,
            &self.time_ctxs,
            &self.expr_program,
            &mut self.props,
            &mut self.props_scratch,
        )?;

        compute_visibility(ir, &self.time_ctxs, Some(&self.props), &mut self.vis)?;

        self.graph.frame = global_frame;
        self.graph.leaves.clear();
        self.graph.groups.clear();
        self.graph.units.clear();
        self.group_stack.clear();
        self.group_frames.clear();
        self.isolated_group_stack.clear();

        self.dfs_emit_leaves(ir)?;

        Ok(&self.graph)
    }

    fn dfs_emit_leaves(&mut self, ir: &CompositionIR) -> Result<(), VmError> {
        #[derive(Clone, Copy)]
        enum Work {
            Enter {
                idx: NodeIdx,
                parent_world: Affine,
                parent_opacity: f64,
            },
            ExitGroup {
                idx: NodeIdx,
            },
        }

        let mut stack: Vec<Work> = Vec::with_capacity(64);
        stack.push(Work::Enter {
            idx: ir.root,
            parent_world: Affine::IDENTITY,
            parent_opacity: 1.0,
        });

        while let Some(w) = stack.pop() {
            match w {
                Work::ExitGroup { idx: _ } => {
                    let frame = self
                        .group_frames
                        .pop()
                        .ok_or_else(|| VmError::new("group frame underflow"))?;
                    let popped = self.group_stack.pop();
                    debug_assert_eq!(popped, Some(frame.idx));

                    if frame.isolate {
                        let end_leaf = self.graph.leaves.len();
                        self.graph.groups.push(EvaluatedGroup {
                            node: frame.idx,
                            leaf_range: frame.start_leaf..end_leaf,
                        });

                        let popped_iso = self.isolated_group_stack.pop();
                        debug_assert_eq!(popped_iso, Some(frame.idx));

                        if self.isolated_group_stack.is_empty() {
                            let node_ir = &ir.nodes[frame.idx.0 as usize];
                            let t = self.time_ctxs[frame.idx.0 as usize];
                            let (tin, tout) = resolve_transitions(node_ir, t);
                            self.graph.units.push(RenderUnit {
                                kind: RenderUnitKind::Group(frame.idx),
                                leaf_range: frame.start_leaf..end_leaf,
                                transition_in: tin,
                                transition_out: tout,
                            });
                        }
                    }
                }
                Work::Enter {
                    idx,
                    parent_world,
                    parent_opacity,
                } => {
                    if !self.vis.node_visible[idx.0 as usize] {
                        continue;
                    }

                    let node = &ir.nodes[idx.0 as usize];
                    let t = self.time_ctxs[idx.0 as usize];
                    let frame = t.sample_frame_u64();

                    let (local_affine, local_opacity) = sample_local_transform_and_opacity(
                        &node.props,
                        ir,
                        idx,
                        frame,
                        &self.props,
                    )?;
                    let world = parent_world * local_affine;
                    let opacity = (parent_opacity * local_opacity).clamp(0.0, 1.0);

                    match &node.kind {
                        NodeKindIR::Leaf { asset } => {
                            let leaf_i = self.graph.leaves.len();
                            let mut gs: SmallVec<[NodeIdx; 4]> = SmallVec::new();
                            gs.extend(self.group_stack.iter().copied());
                            self.graph.leaves.push(EvaluatedLeaf {
                                node: idx,
                                asset: *asset,
                                world_transform: world,
                                opacity: opacity as f32,
                                group_stack: gs,
                            });

                            if self.isolated_group_stack.is_empty() {
                                let (tin, tout) = resolve_transitions(node, t);
                                self.graph.units.push(RenderUnit {
                                    kind: RenderUnitKind::Leaf(idx),
                                    leaf_range: leaf_i..(leaf_i + 1),
                                    transition_in: tin,
                                    transition_out: tout,
                                });
                            }
                        }
                        NodeKindIR::CompRef { .. } => {
                            return Err(VmError::new(
                                "CompRef is not implemented in v0.3 evaluator yet",
                            ));
                        }
                        NodeKindIR::Collection { mode, children, .. } => {
                            if matches!(mode, CollectionModeIR::Group) {
                                let isolate = group_requires_isolation(node);
                                let start_leaf = self.graph.leaves.len();
                                self.group_stack.push(idx);
                                self.group_frames.push(GroupFrame {
                                    idx,
                                    start_leaf,
                                    isolate,
                                });
                                if isolate {
                                    self.isolated_group_stack.push(idx);
                                }
                                stack.push(Work::ExitGroup { idx });
                            }

                            match mode {
                                CollectionModeIR::Switch => {
                                    let active = self.vis.switch_active_child[idx.0 as usize];
                                    if let Some(c) = active {
                                        stack.push(Work::Enter {
                                            idx: c,
                                            parent_world: world,
                                            parent_opacity: opacity,
                                        });
                                    }
                                }
                                _ => {
                                    // Push in reverse to preserve painter-order traversal.
                                    for &c in children.iter().rev() {
                                        if self.vis.node_visible[c.0 as usize] {
                                            stack.push(Work::Enter {
                                                idx: c,
                                                parent_world: world,
                                                parent_opacity: opacity,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

fn group_requires_isolation(node: &NodeIR) -> bool {
    node.mask.is_some()
        || !node.effects.is_empty()
        || node.transition_in.is_some()
        || node.transition_out.is_some()
}

fn resolve_transitions(
    node: &NodeIR,
    t: NodeTimeCtx,
) -> (Option<ResolvedTransition>, Option<ResolvedTransition>) {
    let local = t.sample_frame_u64();
    let dur = t.duration_frames_u64();

    let tin = node
        .transition_in
        .as_ref()
        .and_then(|spec| resolve_transition_in(spec, local, dur));
    let tout = node
        .transition_out
        .as_ref()
        .and_then(|spec| resolve_transition_out(spec, local, dur));

    (tin, tout)
}

fn resolve_transition_in(
    spec: &TransitionSpecIR,
    local_frame: u64,
    node_dur_frames: u64,
) -> Option<ResolvedTransition> {
    if node_dur_frames == 0 {
        return None;
    }
    let d = (spec.duration_frames as u64).min(node_dur_frames);
    if d == 0 {
        return None;
    }
    if local_frame >= d {
        return None;
    }

    let denom = d.saturating_sub(1);
    let t = if denom == 0 {
        1.0
    } else {
        (local_frame as f64) / (denom as f64)
    };
    let p = spec.ease.apply(t).clamp(0.0, 1.0);

    Some(ResolvedTransition {
        kind: spec.kind,
        progress: p as f32,
    })
}

fn resolve_transition_out(
    spec: &TransitionSpecIR,
    local_frame: u64,
    node_dur_frames: u64,
) -> Option<ResolvedTransition> {
    if node_dur_frames == 0 {
        return None;
    }
    let d = (spec.duration_frames as u64).min(node_dur_frames);
    if d == 0 {
        return None;
    }
    let start = node_dur_frames.saturating_sub(d);
    if local_frame < start {
        return None;
    }

    let offset = local_frame - start;
    let denom = d.saturating_sub(1);
    let t = if denom == 0 {
        1.0
    } else {
        (offset as f64) / (denom as f64)
    };
    let p = spec.ease.apply(t).clamp(0.0, 1.0);

    Some(ResolvedTransition {
        kind: spec.kind,
        progress: p as f32,
    })
}

fn sample_local_transform_and_opacity(
    props: &NodePropsIR,
    ir: &CompositionIR,
    node: NodeIdx,
    frame: u64,
    values: &PropertyValues,
) -> Result<(Affine, f64), VmError> {
    let opacity = sample_anim_f64(
        &props.opacity,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::Opacity),
        },
        values,
    )?
    .clamp(0.0, 1.0);

    let tx = sample_anim_f64(
        &props.translate_x,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformTranslateX),
        },
        values,
    )?;
    let ty = sample_anim_f64(
        &props.translate_y,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformTranslateY),
        },
        values,
    )?;
    let rot = sample_anim_f64(
        &props.rotation_rad,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformRotationRad),
        },
        values,
    )?;
    let sx = sample_anim_f64(
        &props.scale_x,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformScaleX),
        },
        values,
    )?;
    let sy = sample_anim_f64(
        &props.scale_y,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformScaleY),
        },
        values,
    )?;
    let ax = sample_anim_f64(
        &props.anchor_x,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformAnchorX),
        },
        values,
    )?;
    let ay = sample_anim_f64(
        &props.anchor_y,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformAnchorY),
        },
        values,
    )?;
    let skew_x_deg = sample_anim_f64(
        &props.skew_x_deg,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformSkewX),
        },
        values,
    )?;
    let skew_y_deg = sample_anim_f64(
        &props.skew_y_deg,
        SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, PropertyKey::TransformSkewY),
        },
        values,
    )?;

    let local = local_affine_from_components(tx, ty, rot, sx, sy, ax, ay, skew_x_deg, skew_y_deg);
    Ok((local, opacity))
}

fn sample_anim_f64(a: &Anim<f64>, ctx: SampleCtx, values: &PropertyValues) -> Result<f64, VmError> {
    match a {
        Anim::Constant(v) => Ok(*v),
        Anim::Keyframes(k) => Ok(k.sample(ctx.frame)),
        Anim::Procedural(p) => Ok(p.sample(ctx)),
        Anim::Reference(pid) => Ok(values.get(*pid)?.as_f64()?),
    }
}

fn mixed_seed(base: u64, node: NodeIdx, lane: PropertyKey) -> u64 {
    let mut h = Fnv1a64::new(base);
    h.write_u64(node.0 as u64);
    h.write_u64(lane.as_u32() as u64);
    h.finish()
}

#[allow(clippy::too_many_arguments)]
fn local_affine_from_components(
    tx: f64,
    ty: f64,
    rotation_rad: f64,
    scale_x: f64,
    scale_y: f64,
    anchor_x: f64,
    anchor_y: f64,
    skew_x_deg: f64,
    skew_y_deg: f64,
) -> Affine {
    let t_translate = Affine::translate((tx, ty));
    let t_anchor = Affine::translate((anchor_x, anchor_y));
    let t_unanchor = Affine::translate((-anchor_x, -anchor_y));
    let t_rotate = Affine::rotate(rotation_rad);
    let t_scale = Affine::scale_non_uniform(scale_x, scale_y);

    let skew_x = (skew_x_deg.to_radians()).tan();
    let skew_y = (skew_y_deg.to_radians()).tan();
    // x' = x + skew_x * y
    let shear_x = Affine::new([1.0, 0.0, skew_x, 1.0, 0.0, 0.0]);
    // y' = y + skew_y * x
    let shear_y = Affine::new([1.0, skew_y, 0.0, 1.0, 0.0, 0.0]);

    // Canonical order:
    // T(translate) * T(anchor) * Skew * R(rot) * S(scale) * T(-anchor)
    t_translate * t_anchor * (shear_y * shear_x) * t_rotate * t_scale * t_unanchor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::{AnimDef, AnimTaggedDef};
    use crate::v03::expression::compile::compile_expr_program;
    use crate::v03::normalize::pass::normalize;
    use crate::v03::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, EffectInstanceDef, FpsDef, NodeDef,
        NodeKindDef, TransitionSpecDef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn inactive_switch_child_does_not_emit_leaf() {
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
        let mut eval = Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        assert_eq!(g.leaves.len(), 1);
    }

    #[test]
    fn group_effect_triggers_isolated_group_unit_and_suppresses_leaf_units() {
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

        let group = NodeDef {
            id: "g".to_owned(),
            kind: NodeKindDef::Collection {
                mode: CollectionModeDef::Group,
                children: vec![child0, child1],
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![EffectInstanceDef {
                kind: "blur".to_owned(),
                params: {
                    let mut m = BTreeMap::new();
                    m.insert("radius".to_owned(), serde_json::json!(3));
                    m
                },
            }],
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
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();

        assert_eq!(g.leaves.len(), 2);
        assert_eq!(g.groups.len(), 1);
        assert_eq!(g.units.len(), 1);
        assert!(matches!(g.units[0].kind, RenderUnitKind::Group(_)));
    }

    #[test]
    fn sequence_overlap_emits_both_children_in_overlap_window() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let a = NodeDef {
            id: "a".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: Some(TransitionSpecDef {
                kind: "fade".to_owned(),
                duration_frames: 3,
                ease: None,
                params: BTreeMap::new(),
            }),
        };

        let b = NodeDef {
            id: "b".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: Some(TransitionSpecDef {
                kind: "fade".to_owned(),
                duration_frames: 3,
                ease: None,
                params: BTreeMap::new(),
            }),
            transition_out: None,
        };

        // Total duration is 10 + 10 - 3 = 17.
        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 1,
                height: 1,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 17,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Sequence,
                    children: vec![a, b],
                },
                range: [0, 17],
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
        let mut eval = Evaluator::new(program);

        let g6 = eval.eval_frame(&norm.ir, 6).unwrap();
        assert_eq!(g6.leaves.len(), 1);
        assert_eq!(g6.units.len(), 1);

        let g7 = eval.eval_frame(&norm.ir, 7).unwrap();
        assert_eq!(g7.leaves.len(), 2);
        assert_eq!(g7.units.len(), 2);

        // In the overlap window, one child has `transition_out` and the other has `transition_in`.
        assert!(
            g7.units.iter().any(|u| u.transition_out.is_some())
                && g7.units.iter().any(|u| u.transition_in.is_some())
        );
    }
}
