use crate::v03::compile::plan::{
    BlendMode, CompositeOp, MaskMode, Op, OpId, OpKind, PassFx, RenderPlan, SurfaceId,
};
use smallvec::{SmallVec, smallvec};

pub(crate) fn fuse_plan(plan: &mut RenderPlan) {
    // This pass is intentionally conservative: it must preserve deterministic semantics.
    //
    // Strategy:
    // 1. Canonicalize surfaces via aliasing when removing identity passes.
    // 2. Fold simple composite opacity into the producing draw op when safe (single use).
    // 3. Recompute op deps since ops are removed/rewritten.

    let surface_count = plan.surfaces.len();
    let mut alias_of: Vec<SurfaceId> = (0..surface_count).map(|i| SurfaceId(i as u32)).collect();

    let mut use_count: Vec<u32> = vec![0; surface_count];
    for op in &plan.ops {
        for s in &op.inputs {
            use_count[s.0 as usize] = use_count[s.0 as usize].saturating_add(1);
        }
    }

    // Producer op index per surface (only the first writer is relevant for safe folding checks).
    let mut producer: Vec<Option<usize>> = vec![None; surface_count];
    for (i, op) in plan.ops.iter().enumerate() {
        let out = op.output.0 as usize;
        producer[out].get_or_insert(i);
    }

    // Accumulated opacity multipliers to apply to draw ops (indexed by op index).
    let mut draw_opacity_mul: Vec<f32> = vec![1.0; plan.ops.len()];

    // Pass 1: fold composite opacity into producer draws when safe.
    for op in &plan.ops {
        let OpKind::Composite { ops, .. } = &op.kind else {
            continue;
        };
        if ops.len() != 1 {
            continue;
        }
        let CompositeOp::Over {
            src,
            opacity,
            blend,
        } = ops[0]
        else {
            continue;
        };
        if blend != BlendMode::Normal {
            continue;
        }
        if opacity == 1.0 {
            continue;
        }
        let src_i = src.0 as usize;
        if use_count.get(src_i).copied().unwrap_or(0) != 1 {
            continue;
        }
        let Some(prod_i) = producer.get(src_i).and_then(|x| *x) else {
            continue;
        };
        let Some(prod) = plan.ops.get(prod_i) else {
            continue;
        };
        if !matches!(prod.kind, OpKind::Draw { .. }) {
            continue;
        }
        draw_opacity_mul[prod_i] *= opacity;
    }

    // Pass 2: rebuild ops with:
    // - surface aliasing for identity passes
    // - draw opacity folded
    // - composite over opacity reset when folded
    let mut rebuilt: Vec<Op> = Vec::with_capacity(plan.ops.len());
    let mut rebuilt_producer: Vec<Option<usize>> = vec![None; surface_count];
    for (i, op) in plan.ops.iter().enumerate() {
        let inputs = op
            .inputs
            .iter()
            .copied()
            .map(|s| canon_surface(s, &mut alias_of))
            .collect::<SmallVec<[SurfaceId; 4]>>();
        let mut output = op.output;

        // If an upstream identity-eliminated op aliased this output surface, canonicalize it.
        output = canon_surface(output, &mut alias_of);

        let mut kind = op.kind.clone();
        match &mut kind {
            OpKind::Draw {
                opacity_mul,
                transform_post: _,
                ..
            } => {
                *opacity_mul *= draw_opacity_mul[i];
            }
            OpKind::Pass { fx } => {
                // Pass identity elimination (alias output -> input).
                match fx {
                    PassFx::Blur { radius_px, sigma } => {
                        if (*radius_px == 0 || *sigma == 0.0)
                            && let Some(&src) = inputs.first()
                        {
                            alias_surface(op.output, src, &mut alias_of);
                            continue;
                        }
                    }
                    PassFx::ColorMatrix { matrix } => {
                        // Fold consecutive color matrices when the intermediate is single-use.
                        if let Some(&src) = inputs.first() {
                            let src_i = src.0 as usize;
                            if use_count.get(src_i).copied().unwrap_or(0) == 1
                                && let Some(prod_i) = rebuilt_producer.get(src_i).and_then(|x| *x)
                                && let OpKind::Pass {
                                    fx: PassFx::ColorMatrix { matrix: prev },
                                } = &mut rebuilt[prod_i].kind
                            {
                                *prev = mul_color_matrix(*matrix, *prev);
                                alias_surface(op.output, src, &mut alias_of);
                                continue;
                            }
                        }

                        if is_identity_color_matrix(matrix)
                            && let Some(&src) = inputs.first()
                        {
                            alias_surface(op.output, src, &mut alias_of);
                            continue;
                        }
                    }
                    PassFx::MaskApply { mode, inverted } => {
                        if is_noop_mask_apply(*mode, *inverted)
                            && let Some(&src) = inputs.first()
                        {
                            alias_surface(op.output, src, &mut alias_of);
                            continue;
                        }
                    }
                    PassFx::DropShadow { .. } => {}
                }
            }
            OpKind::Composite { ops, .. } => {
                // Canonicalize embedded surface ids and apply opacity folding (set opacity to 1).
                for c in ops.iter_mut() {
                    match c {
                        CompositeOp::Over { src, .. } => {
                            *src = canon_surface(*src, &mut alias_of);
                        }
                        CompositeOp::Crossfade { a, b, .. } => {
                            *a = canon_surface(*a, &mut alias_of);
                            *b = canon_surface(*b, &mut alias_of);
                        }
                        CompositeOp::Wipe { a, b, .. } => {
                            *a = canon_surface(*a, &mut alias_of);
                            *b = canon_surface(*b, &mut alias_of);
                        }
                        CompositeOp::Slide { a, b, .. } => {
                            *a = canon_surface(*a, &mut alias_of);
                            *b = canon_surface(*b, &mut alias_of);
                        }
                        CompositeOp::Zoom { a, b, .. } => {
                            *a = canon_surface(*a, &mut alias_of);
                            *b = canon_surface(*b, &mut alias_of);
                        }
                        CompositeOp::Iris { a, b, .. } => {
                            *a = canon_surface(*a, &mut alias_of);
                            *b = canon_surface(*b, &mut alias_of);
                        }
                    }
                }

                if ops.len() == 1
                    && let CompositeOp::Over {
                        src,
                        opacity,
                        blend,
                    } = &mut ops[0]
                    && *blend == BlendMode::Normal
                {
                    let src_i = src.0 as usize;
                    if use_count.get(src_i).copied().unwrap_or(0) == 1
                        && let Some(prod_i) = producer.get(src_i).and_then(|x| *x)
                        && draw_opacity_mul[prod_i] != 1.0
                    {
                        // The producer draw absorbed this opacity.
                        *opacity = 1.0;
                    }
                }
            }
            OpKind::MaskGen { .. } => {}
        }

        // Remove composite ops that have no effective operations.
        if let OpKind::Composite { ops, .. } = &kind
            && ops.is_empty()
        {
            continue;
        }

        // Rebuild op with placeholder deps; recomputed later.
        let new_i = rebuilt.len();
        rebuilt.push(Op {
            id: OpId(u32::try_from(new_i).unwrap()),
            kind,
            inputs,
            output,
            deps: smallvec![],
        });
        rebuilt_producer[output.0 as usize] = Some(new_i);
    }

    // Recompute deps for rebuilt ops (surface last-writer scheduling).
    let mut last_write: Vec<Option<OpId>> = vec![None; surface_count];
    for op in &mut rebuilt {
        let mut deps = SmallVec::<[OpId; 4]>::new();
        for &s in &op.inputs {
            if let Some(w) = last_write.get(s.0 as usize).and_then(|x| *x)
                && !deps.contains(&w)
            {
                deps.push(w);
            }
        }
        if let Some(w) = last_write.get(op.output.0 as usize).and_then(|x| *x)
            && !deps.contains(&w)
        {
            deps.push(w);
        }
        op.deps = deps;
        last_write[op.output.0 as usize] = Some(op.id);
    }

    // Canonicalize roots.
    for r in &mut plan.roots {
        *r = canon_surface(*r, &mut alias_of);
    }

    plan.ops = rebuilt;
}

fn canon_surface(s: SurfaceId, alias_of: &mut [SurfaceId]) -> SurfaceId {
    let mut cur = s;
    // Path compress.
    while alias_of[cur.0 as usize] != cur {
        cur = alias_of[cur.0 as usize];
    }
    let mut x = s;
    while alias_of[x.0 as usize] != x {
        let p = alias_of[x.0 as usize];
        alias_of[x.0 as usize] = cur;
        x = p;
    }
    cur
}

fn alias_surface(dst: SurfaceId, src: SurfaceId, alias_of: &mut [SurfaceId]) {
    let src = canon_surface(src, alias_of);
    alias_of[dst.0 as usize] = src;
}

fn is_identity_color_matrix(m: &[f32; 20]) -> bool {
    let id = [
        1.0, 0.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, 0.0, //
    ];
    let eps = 1.0e-6;
    m.iter().zip(id.iter()).all(|(a, b)| (*a - *b).abs() <= eps)
}

fn mul_color_matrix(a: [f32; 20], b: [f32; 20]) -> [f32; 20] {
    // 4x5 affine color matrices, applied as:
    // out = M * [r,g,b,a,1]
    //
    // This composes as a(b(x)).
    let mut out = [0.0f32; 20];
    for row in 0..4 {
        let base = row * 5;
        for col in 0..4 {
            let mut v = 0.0f32;
            for k in 0..4 {
                v += a[row * 5 + k] * b[k * 5 + col];
            }
            out[row * 5 + col] = v;
        }
        // bias term: a * b_bias + a_bias
        out[base + 4] = a[base + 4]
            + a[base] * b[4]
            + a[base + 1] * b[9]
            + a[base + 2] * b[14]
            + a[base + 3] * b[19];
    }
    out
}

fn is_noop_mask_apply(mode: MaskMode, inverted: bool) -> bool {
    if inverted {
        return false;
    }
    match mode {
        MaskMode::Alpha | MaskMode::Luma => false,
        MaskMode::Stencil { threshold } => threshold <= 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::core::Affine;
    use crate::v03::compile::plan::{PixelFormat, SurfaceDesc, UnitKey};

    #[test]
    fn fuse_folds_over_opacity_into_producer_draw_when_single_use() {
        let mut plan = RenderPlan {
            surfaces: vec![
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
            ],
            ops: vec![
                Op {
                    id: OpId(0),
                    kind: OpKind::Draw {
                        unit: UnitKey::Leaf(crate::v03::foundation::ids::NodeIdx(0)),
                        leaves: 0..0,
                        clear_to_transparent: true,
                        transform_post: Affine::IDENTITY,
                        opacity_mul: 1.0,
                    },
                    inputs: smallvec![],
                    output: SurfaceId(1),
                    deps: smallvec![],
                },
                Op {
                    id: OpId(1),
                    kind: OpKind::Composite {
                        clear_to_transparent: true,
                        ops: Box::new({
                            let mut v = SmallVec::<[CompositeOp; 8]>::new();
                            v.push(CompositeOp::Over {
                                src: SurfaceId(1),
                                opacity: 0.25,
                                blend: BlendMode::Normal,
                            });
                            v
                        }),
                    },
                    inputs: smallvec![SurfaceId(1)],
                    output: SurfaceId(0),
                    deps: smallvec![],
                },
            ],
            roots: smallvec![SurfaceId(0)],
        };

        fuse_plan(&mut plan);

        let draw = &plan.ops[0];
        let OpKind::Draw { opacity_mul, .. } = &draw.kind else {
            panic!("expected draw");
        };
        assert!((*opacity_mul - 0.25).abs() < 1e-6);

        let comp = &plan.ops[1];
        let OpKind::Composite { ops, .. } = &comp.kind else {
            panic!("expected composite");
        };
        let CompositeOp::Over { opacity, .. } = &ops[0] else {
            panic!("expected over");
        };
        assert!((*opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fuse_eliminates_stencil_threshold_0_mask_apply() {
        let mut plan = RenderPlan {
            surfaces: vec![
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
            ],
            ops: vec![
                Op {
                    id: OpId(0),
                    kind: OpKind::Pass {
                        fx: PassFx::MaskApply {
                            mode: MaskMode::Stencil { threshold: 0.0 },
                            inverted: false,
                        },
                    },
                    inputs: smallvec![SurfaceId(1), SurfaceId(2)],
                    output: SurfaceId(0),
                    deps: smallvec![],
                },
                Op {
                    id: OpId(1),
                    kind: OpKind::Composite {
                        clear_to_transparent: true,
                        ops: Box::new({
                            let mut v = SmallVec::<[CompositeOp; 8]>::new();
                            v.push(CompositeOp::Over {
                                src: SurfaceId(0),
                                opacity: 1.0,
                                blend: BlendMode::Normal,
                            });
                            v
                        }),
                    },
                    inputs: smallvec![SurfaceId(0)],
                    output: SurfaceId(0),
                    deps: smallvec![],
                },
            ],
            roots: smallvec![SurfaceId(0)],
        };

        fuse_plan(&mut plan);

        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(&op.kind, OpKind::Pass { .. }))
        );
    }

    #[test]
    fn fuse_eliminates_blur_radius_0() {
        let mut plan = RenderPlan {
            surfaces: vec![
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
            ],
            ops: vec![Op {
                id: OpId(0),
                kind: OpKind::Pass {
                    fx: PassFx::Blur {
                        radius_px: 0,
                        sigma: 0.0,
                    },
                },
                inputs: smallvec![SurfaceId(1)],
                output: SurfaceId(0),
                deps: smallvec![],
            }],
            roots: smallvec![SurfaceId(0)],
        };

        fuse_plan(&mut plan);
        assert!(plan.ops.is_empty());
    }

    #[test]
    fn fuse_folds_consecutive_color_matrices() {
        let m_id = [
            1.0, 0.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0, //
        ];
        let mut m2 = m_id;
        m2[4] = 0.25; // add bias to red

        let mut plan = RenderPlan {
            surfaces: vec![
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgba8Premul,
                },
            ],
            ops: vec![
                Op {
                    id: OpId(0),
                    kind: OpKind::Pass {
                        fx: PassFx::ColorMatrix { matrix: m2 },
                    },
                    inputs: smallvec![SurfaceId(2)],
                    output: SurfaceId(1),
                    deps: smallvec![],
                },
                Op {
                    id: OpId(1),
                    kind: OpKind::Pass {
                        fx: PassFx::ColorMatrix { matrix: m2 },
                    },
                    inputs: smallvec![SurfaceId(1)],
                    output: SurfaceId(0),
                    deps: smallvec![],
                },
            ],
            roots: smallvec![SurfaceId(0)],
        };

        fuse_plan(&mut plan);
        // Second matrix should have folded into first (and the second op should be removed/aliased).
        assert_eq!(plan.ops.len(), 1);
        assert!(matches!(&plan.ops[0].kind, OpKind::Pass { .. }));
    }
}
