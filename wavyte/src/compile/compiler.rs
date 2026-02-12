use crate::compile::plan::{
    BlendMode, CompositeOp, MaskGenSource, MaskMode, Op, OpId, OpKind, PassFx, PixelFormat,
    RenderPlan, SurfaceDesc, SurfaceId, UnitKey,
};
use crate::effects::binding::{
    CoreEffectKindIR, CoreParamKeyIR, EffectBindingIR, ResolvedParamValueIR,
};
use crate::eval::evaluator::{EvaluatedGraph, RenderUnitKind, ResolvedTransition};
use crate::foundation::core::Affine;
use crate::normalize::ir::{
    BlendModeIR, CompositionIR, IrisShapeIR, MaskModeIR, MaskSourceIR, SlideDirIR,
    TransitionKindIR, WipeDirIR,
};
use smallvec::{SmallVec, smallvec};

const TRANSITION_PROGRESS_TOLERANCE: f32 = 1.0 / 1024.0;

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

    // Pass 1: ensure all units that need isolation produce a standalone surface.
    let mut unit_surfaces = Vec::<Option<SurfaceId>>::with_capacity(eval.units.len());
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
            || unit.blend != BlendModeIR::Normal
            || has_transition;

        if !needs_isolation {
            unit_surfaces.push(None);
            continue;
        }

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
        current_surface = compile_effect_chain(
            node.effects.as_slice(),
            current_surface,
            root,
            &mut surfaces,
            &mut ops,
            &mut last_write,
        );

        unit_surfaces.push(Some(current_surface));
    }

    // Pass 2: composite units into the root surface in painter order, applying transition pairing.
    let mut root_written = false;
    let mut i = 0usize;
    while i < eval.units.len() {
        if let Some(pair) = try_pair_transition(eval, &unit_surfaces, i) {
            let (kind, a_surf, b_surf, t) = pair;
            let pair_surface = alloc_surface_like(&mut surfaces, &mut last_write, root);

            let mut cop = SmallVec::<[CompositeOp; 8]>::new();
            cop.push(transition_composite(kind, a_surf, b_surf, t));

            push_op(
                &mut ops,
                &mut last_write,
                OpKind::Composite {
                    clear_to_transparent: true,
                    ops: Box::new(cop),
                },
                smallvec![a_surf, b_surf],
                pair_surface,
            );

            composite_over_root(
                &mut ops,
                &mut last_write,
                root,
                &mut root_written,
                pair_surface,
                1.0,
                map_blend_mode(eval.units[i + 1].blend),
            );

            i += 2;
            continue;
        }

        let unit = &eval.units[i];
        if let Some(surf) = unit_surfaces[i] {
            let opacity = unit_transition_opacity(unit);
            composite_over_root(
                &mut ops,
                &mut last_write,
                root,
                &mut root_written,
                surf,
                opacity,
                map_blend_mode(unit.blend),
            );
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

        i += 1;
    }

    let mut plan = RenderPlan {
        surfaces,
        ops,
        roots: smallvec![root],
    };
    crate::compile::fuse::fuse_plan(&mut plan);
    plan
}

fn unit_transition_opacity(unit: &crate::eval::evaluator::RenderUnit) -> f32 {
    let mut o = 1.0f32;
    if let Some(t) = unit.transition_in.as_ref() {
        o *= t.progress.clamp(0.0, 1.0);
    }
    if let Some(t) = unit.transition_out.as_ref() {
        o *= (1.0 - t.progress.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    }
    o
}

fn try_pair_transition(
    eval: &EvaluatedGraph,
    unit_surfaces: &[Option<SurfaceId>],
    i: usize,
) -> Option<(TransitionKindIR, SurfaceId, SurfaceId, f32)> {
    let a = eval.units.get(i)?;
    let b = eval.units.get(i + 1)?;
    if a.blend != b.blend {
        return None;
    }

    let a_out = a.transition_out.as_ref()?;
    let b_in = b.transition_in.as_ref()?;

    if !can_pair(a_out, b_in) {
        return None;
    }

    let a_surf = unit_surfaces.get(i).and_then(|x| *x)?;
    let b_surf = unit_surfaces.get(i + 1).and_then(|x| *x)?;

    Some((a_out.kind, a_surf, b_surf, b_in.progress))
}

fn can_pair(a_out: &ResolvedTransition, b_in: &ResolvedTransition) -> bool {
    if a_out.kind != b_in.kind {
        return false;
    }
    (a_out.progress - b_in.progress).abs() <= TRANSITION_PROGRESS_TOLERANCE
}

fn composite_over_root(
    ops: &mut Vec<Op>,
    last_write: &mut [Option<OpId>],
    root: SurfaceId,
    root_written: &mut bool,
    src: SurfaceId,
    opacity: f32,
    blend: BlendMode,
) {
    let mut cop = SmallVec::<[CompositeOp; 8]>::new();
    cop.push(CompositeOp::Over {
        src,
        opacity,
        blend,
    });

    let mut inputs = SmallVec::<[SurfaceId; 4]>::new();
    if *root_written {
        inputs.push(root);
    }
    inputs.push(src);

    push_op(
        ops,
        last_write,
        OpKind::Composite {
            clear_to_transparent: !*root_written,
            ops: Box::new(cop),
        },
        inputs,
        root,
    );
    *root_written = true;
}

fn transition_composite(kind: TransitionKindIR, a: SurfaceId, b: SurfaceId, t: f32) -> CompositeOp {
    match kind {
        TransitionKindIR::Crossfade => CompositeOp::Crossfade { a, b, t },
        TransitionKindIR::Wipe { dir, soft_edge } => CompositeOp::Wipe {
            a,
            b,
            t,
            dir: map_wipe_dir(dir),
            soft_edge,
        },
        TransitionKindIR::Slide { dir, push } => CompositeOp::Slide {
            a,
            b,
            t,
            dir: map_slide_dir(dir),
            push,
        },
        TransitionKindIR::Zoom { origin, from_scale } => CompositeOp::Zoom {
            a,
            b,
            t,
            origin,
            from_scale,
        },
        TransitionKindIR::Iris {
            origin,
            shape,
            soft_edge,
        } => CompositeOp::Iris {
            a,
            b,
            t,
            origin,
            shape: map_iris_shape(shape),
            soft_edge,
        },
    }
}

fn compile_effect_chain(
    effects: &[EffectBindingIR],
    mut current_surface: SurfaceId,
    root: SurfaceId,
    surfaces: &mut Vec<SurfaceDesc>,
    ops: &mut Vec<Op>,
    last_write: &mut Vec<Option<OpId>>,
) -> SurfaceId {
    for effect in effects {
        let Some(fx) = effect_to_pass(effect) else {
            continue;
        };
        let out = alloc_surface_like(surfaces, last_write, root);
        push_op(
            ops,
            last_write,
            OpKind::Pass { fx },
            smallvec![current_surface],
            out,
        );
        current_surface = out;
    }
    current_surface
}

fn effect_to_pass(effect: &EffectBindingIR) -> Option<PassFx> {
    match effect.core_kind {
        CoreEffectKindIR::Blur => {
            let radius = read_u32_param(effect, &[CoreParamKeyIR::RadiusPx, CoreParamKeyIR::Value])
                .unwrap_or(8);
            let sigma = read_f32_param(effect, &[CoreParamKeyIR::Sigma])
                .unwrap_or_else(|| default_blur_sigma(radius));
            Some(PassFx::Blur {
                radius_px: radius,
                sigma,
            })
        }
        CoreEffectKindIR::ColorMatrix => {
            let matrix =
                read_matrix_param(effect, &[CoreParamKeyIR::Matrix, CoreParamKeyIR::Value])
                    .unwrap_or_else(identity_color_matrix);
            Some(PassFx::ColorMatrix { matrix })
        }
        CoreEffectKindIR::DropShadow => {
            let offset = read_vec2_param(effect, &[CoreParamKeyIR::Offset])
                .unwrap_or_else(|| crate::foundation::core::Vec2::new(4.0, 4.0));
            let blur_radius_px = read_u32_param(
                effect,
                &[CoreParamKeyIR::BlurRadiusPx, CoreParamKeyIR::RadiusPx],
            )
            .or_else(|| read_u32_param(effect, &[CoreParamKeyIR::Value]))
            .unwrap_or(8);
            let sigma = read_f32_param(effect, &[CoreParamKeyIR::Sigma])
                .unwrap_or_else(|| default_blur_sigma(blur_radius_px));
            let color = read_color_param(effect, &[CoreParamKeyIR::Color]).unwrap_or_else(|| {
                crate::foundation::core::Rgba8Premul::from_straight_rgba(0, 0, 0, 192)
            });
            Some(PassFx::DropShadow {
                offset,
                blur_radius_px,
                sigma,
                color,
            })
        }
        CoreEffectKindIR::Unknown => None,
    }
}

fn read_u32_param(effect: &EffectBindingIR, keys: &[CoreParamKeyIR]) -> Option<u32> {
    read_f64_param(effect, keys).map(|v| v.max(0.0).round() as u32)
}

fn read_f32_param(effect: &EffectBindingIR, keys: &[CoreParamKeyIR]) -> Option<f32> {
    read_f64_param(effect, keys).map(|v| v as f32)
}

fn read_f64_param(effect: &EffectBindingIR, keys: &[CoreParamKeyIR]) -> Option<f64> {
    for p in &effect.params {
        if !keys.contains(&p.key) {
            continue;
        }
        match p.value {
            ResolvedParamValueIR::F64(v) => return Some(v),
            _ => continue,
        }
    }
    None
}

fn read_vec2_param(
    effect: &EffectBindingIR,
    keys: &[CoreParamKeyIR],
) -> Option<crate::foundation::core::Vec2> {
    for p in &effect.params {
        if !keys.contains(&p.key) {
            continue;
        }
        match p.value {
            ResolvedParamValueIR::Vec2(v) => return Some(v),
            _ => continue,
        }
    }
    None
}

fn read_color_param(
    effect: &EffectBindingIR,
    keys: &[CoreParamKeyIR],
) -> Option<crate::foundation::core::Rgba8Premul> {
    for p in &effect.params {
        if !keys.contains(&p.key) {
            continue;
        }
        match p.value {
            ResolvedParamValueIR::Color(v) => return Some(v),
            _ => continue,
        }
    }
    None
}

fn read_matrix_param(effect: &EffectBindingIR, keys: &[CoreParamKeyIR]) -> Option<[f32; 20]> {
    for p in &effect.params {
        if !keys.contains(&p.key) {
            continue;
        }
        match p.value {
            ResolvedParamValueIR::Matrix20(v) => return Some(v),
            _ => continue,
        }
    }
    None
}

fn default_blur_sigma(radius: u32) -> f32 {
    // Gaussian radius/sigma heuristic that keeps blur visually stable for small radii.
    let r = radius as f32;
    (r / 3.0).max(0.1)
}

fn identity_color_matrix() -> [f32; 20] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, 0.0, //
    ]
}

fn map_wipe_dir(dir: WipeDirIR) -> crate::compile::plan::WipeDir {
    match dir {
        WipeDirIR::LeftToRight => crate::compile::plan::WipeDir::LeftToRight,
        WipeDirIR::RightToLeft => crate::compile::plan::WipeDir::RightToLeft,
        WipeDirIR::TopToBottom => crate::compile::plan::WipeDir::TopToBottom,
        WipeDirIR::BottomToTop => crate::compile::plan::WipeDir::BottomToTop,
    }
}

fn map_slide_dir(dir: SlideDirIR) -> crate::compile::plan::SlideDir {
    match dir {
        SlideDirIR::Left => crate::compile::plan::SlideDir::Left,
        SlideDirIR::Right => crate::compile::plan::SlideDir::Right,
        SlideDirIR::Up => crate::compile::plan::SlideDir::Up,
        SlideDirIR::Down => crate::compile::plan::SlideDir::Down,
    }
}

fn map_iris_shape(shape: IrisShapeIR) -> crate::compile::plan::IrisShape {
    match shape {
        IrisShapeIR::Circle => crate::compile::plan::IrisShape::Circle,
        IrisShapeIR::Rect => crate::compile::plan::IrisShape::Rect,
        IrisShapeIR::Diamond => crate::compile::plan::IrisShape::Diamond,
    }
}

fn map_blend_mode(v: BlendModeIR) -> BlendMode {
    match v {
        BlendModeIR::Normal => BlendMode::Normal,
        BlendModeIR::Multiply => BlendMode::Multiply,
        BlendModeIR::Screen => BlendMode::Screen,
        BlendModeIR::Overlay => BlendMode::Overlay,
        BlendModeIR::Darken => BlendMode::Darken,
        BlendModeIR::Lighten => BlendMode::Lighten,
        BlendModeIR::ColorDodge => BlendMode::ColorDodge,
        BlendModeIR::ColorBurn => BlendMode::ColorBurn,
        BlendModeIR::SoftLight => BlendMode::SoftLight,
        BlendModeIR::HardLight => BlendMode::HardLight,
        BlendModeIR::Difference => BlendMode::Difference,
        BlendModeIR::Exclusion => BlendMode::Exclusion,
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
                None => (crate::foundation::ids::NodeIdx(0), 0..0),
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
                        unit: UnitKey::MaskNode(crate::foundation::ids::NodeIdx(0)),
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
    use crate::animation::anim::AnimDef;
    use crate::expression::compile::compile_expr_program;
    use crate::normalize::pass::normalize;
    use crate::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, EffectInstanceDef, FpsDef, MaskDef,
        MaskModeDef, MaskSourceDef, NodeDef, NodeKindDef, TransitionSpecDef,
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
                            blend: Default::default(),
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
                            blend: Default::default(),
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
                blend: Default::default(),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::eval::evaluator::Evaluator::new(program);
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
            blend: Default::default(),
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
            blend: Default::default(),
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
                blend: Default::default(),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::eval::evaluator::Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        assert_eq!(g.units.len(), 1);
        assert!(matches!(g.units[0].kind, RenderUnitKind::Group(_)));

        let plan = compile_frame(&norm.ir, g);
        assert_eq!(plan.surfaces.len(), 3);
        assert_eq!(plan.ops.len(), 3);
        assert!(matches!(plan.ops[0].kind, OpKind::Draw { .. }));
        assert!(matches!(plan.ops[1].kind, OpKind::Pass { .. }));
        assert!(matches!(plan.ops[2].kind, OpKind::Composite { .. }));
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
            blend: Default::default(),
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
            blend: Default::default(),
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
            blend: Default::default(),
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
                blend: Default::default(),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::eval::evaluator::Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        let plan = compile_frame(&norm.ir, g);

        assert!(
            plan.ops
                .iter()
                .any(|op| matches!(op.kind, OpKind::Pass { .. }))
        );
    }

    #[test]
    fn compile_pairs_crossfade_when_progress_is_aligned() {
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
            blend: Default::default(),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: Some(TransitionSpecDef {
                kind: "crossfade".to_owned(),
                duration_frames: 2,
                ease: None,
                params: BTreeMap::new(),
            }),
        };
        let b = NodeDef {
            id: "b".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            // Offset so at global_frame=9, a is in its last 2 frames and b is in its first 2.
            range: [8, 18],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            blend: Default::default(),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: Some(TransitionSpecDef {
                kind: "crossfade".to_owned(),
                duration_frames: 2,
                ease: None,
                params: BTreeMap::new(),
            }),
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 8,
                height: 8,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 20,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Group,
                    children: vec![a, b],
                },
                range: [0, 20],
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

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::eval::evaluator::Evaluator::new(program);

        // At frame 9: a_out.progress=1, b_in.progress=1 => pair.
        let g = eval.eval_frame(&norm.ir, 9).unwrap();
        let plan = compile_frame(&norm.ir, g);

        let mut saw_crossfade = false;
        for op in &plan.ops {
            if let OpKind::Composite { ops, .. } = &op.kind
                && ops
                    .iter()
                    .any(|c| matches!(c, CompositeOp::Crossfade { .. }))
            {
                saw_crossfade = true;
            }
        }
        assert!(saw_crossfade);
    }

    #[test]
    fn compile_does_not_pair_when_progress_is_not_aligned() {
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
            blend: Default::default(),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: Some(TransitionSpecDef {
                kind: "crossfade".to_owned(),
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
            // At global_frame=9: b_in.progress=0 while a_out.progress=1 => not pairable.
            range: [9, 19],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            blend: Default::default(),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: Some(TransitionSpecDef {
                kind: "crossfade".to_owned(),
                duration_frames: 2,
                ease: None,
                params: BTreeMap::new(),
            }),
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 8,
                height: 8,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 20,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Group,
                    children: vec![a, b],
                },
                range: [0, 20],
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

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::eval::evaluator::Evaluator::new(program);

        let g = eval.eval_frame(&norm.ir, 9).unwrap();
        let plan = compile_frame(&norm.ir, g);

        for op in &plan.ops {
            if let OpKind::Composite { ops, .. } = &op.kind {
                assert!(
                    !ops.iter()
                        .any(|c| matches!(c, CompositeOp::Crossfade { .. }))
                );
            }
        }
    }

    #[test]
    fn plan_dump_is_deterministic_for_same_frame() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 16,
                height: 16,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 20,
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
                            blend: Default::default(),
                            layout: None,
                            effects: vec![],
                            mask: None,
                            transition_in: Some(TransitionSpecDef {
                                kind: "crossfade".to_owned(),
                                duration_frames: 2,
                                ease: None,
                                params: BTreeMap::new(),
                            }),
                            transition_out: None,
                        },
                        NodeDef {
                            id: "b".to_owned(),
                            kind: NodeKindDef::Leaf {
                                asset: "a".to_owned(),
                            },
                            range: [2, 12],
                            transform: Default::default(),
                            opacity: AnimDef::Constant(1.0),
                            blend: Default::default(),
                            layout: None,
                            effects: vec![],
                            mask: None,
                            transition_in: None,
                            transition_out: Some(TransitionSpecDef {
                                kind: "crossfade".to_owned(),
                                duration_frames: 2,
                                ease: None,
                                params: BTreeMap::new(),
                            }),
                        },
                    ],
                },
                range: [0, 20],
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

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut eval = crate::eval::evaluator::Evaluator::new(program);
        let g = eval.eval_frame(&norm.ir, 2).unwrap();

        let p1 = compile_frame(&norm.ir, g).dump();
        let p2 = compile_frame(&norm.ir, g).dump();
        assert_eq!(p1, p2);
    }
}
