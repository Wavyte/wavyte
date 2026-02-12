use crate::compile::fingerprint::FrameFingerprint;
use crate::v03::compile::plan::{
    BlendMode, CompositeOp, IrisShape, MaskGenSource, MaskMode, OpKind, PassFx, PixelFormat,
    RenderPlan, SlideDir, SurfaceDesc, SurfaceId, UnitKey, WipeDir,
};
use crate::v03::normalize::intern::InternId;
use crate::v03::normalize::ir::ShapeIR;
use xxhash_rust::xxh3::Xxh3;

const XXH3_SEED: u64 = 0x8b5ad4a0c7d8e9f1;

pub(crate) fn fingerprint_plan(plan: &RenderPlan) -> FrameFingerprint {
    let mut h = StableHasher::new();
    write_plan(&mut h, plan);
    h.finish()
}

struct StableHasher {
    inner: Xxh3,
}

impl StableHasher {
    fn new() -> Self {
        Self {
            inner: Xxh3::with_seed(XXH3_SEED),
        }
    }

    fn write_bytes(&mut self, b: &[u8]) {
        self.inner.update(b);
    }

    fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    fn write_bool(&mut self, v: bool) {
        self.write_u8(u8::from(v));
    }

    fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }

    fn write_f64(&mut self, v: f64) {
        self.write_u64(v.to_bits());
    }

    fn finish(self) -> FrameFingerprint {
        let v = self.inner.digest128();
        FrameFingerprint {
            hi: (v >> 64) as u64,
            lo: v as u64,
        }
    }
}

fn write_plan(h: &mut StableHasher, plan: &RenderPlan) {
    h.write_u32(plan.surfaces.len() as u32);
    for s in &plan.surfaces {
        write_surface_desc(h, s);
    }

    h.write_u32(plan.ops.len() as u32);
    for op in &plan.ops {
        write_op(h, op.output, &op.kind, &op.inputs);
    }

    h.write_u32(plan.roots.len() as u32);
    for r in &plan.roots {
        write_surface_id(h, *r);
    }
}

fn write_surface_desc(h: &mut StableHasher, s: &SurfaceDesc) {
    h.write_u32(s.width);
    h.write_u32(s.height);
    match s.format {
        PixelFormat::Rgba8Premul => h.write_u8(0),
    }
}

fn write_surface_id(h: &mut StableHasher, s: SurfaceId) {
    h.write_u32(s.0);
}

fn write_unit_key(h: &mut StableHasher, u: UnitKey) {
    match u {
        UnitKey::Leaf(n) => {
            h.write_u8(0);
            h.write_u32(n.0);
        }
        UnitKey::Group(n) => {
            h.write_u8(1);
            h.write_u32(n.0);
        }
        UnitKey::MaskNode(n) => {
            h.write_u8(2);
            h.write_u32(n.0);
        }
    }
}

fn write_mask_mode(h: &mut StableHasher, m: MaskMode) {
    match m {
        MaskMode::Alpha => h.write_u8(0),
        MaskMode::Luma => h.write_u8(1),
        MaskMode::Stencil { threshold } => {
            h.write_u8(2);
            h.write_f32(threshold);
        }
    }
}

fn write_blend_mode(h: &mut StableHasher, b: BlendMode) {
    h.write_u8(match b {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::SoftLight => 8,
        BlendMode::HardLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
    });
}

fn write_wipe_dir(h: &mut StableHasher, d: WipeDir) {
    h.write_u8(match d {
        WipeDir::LeftToRight => 0,
        WipeDir::RightToLeft => 1,
        WipeDir::TopToBottom => 2,
        WipeDir::BottomToTop => 3,
    });
}

fn write_slide_dir(h: &mut StableHasher, d: SlideDir) {
    h.write_u8(match d {
        SlideDir::Left => 0,
        SlideDir::Right => 1,
        SlideDir::Up => 2,
        SlideDir::Down => 3,
    });
}

fn write_iris_shape(h: &mut StableHasher, s: IrisShape) {
    h.write_u8(match s {
        IrisShape::Circle => 0,
        IrisShape::Rect => 1,
        IrisShape::Diamond => 2,
    });
}

fn write_intern_id(h: &mut StableHasher, id: InternId) {
    h.write_u32(id.0);
}

fn write_shape(h: &mut StableHasher, s: &ShapeIR) {
    match s {
        ShapeIR::Rect { width, height } => {
            h.write_u8(0);
            h.write_f64(*width);
            h.write_f64(*height);
        }
        ShapeIR::RoundedRect {
            width,
            height,
            radius,
        } => {
            h.write_u8(1);
            h.write_f64(*width);
            h.write_f64(*height);
            h.write_f64(*radius);
        }
        ShapeIR::Ellipse { rx, ry } => {
            h.write_u8(2);
            h.write_f64(*rx);
            h.write_f64(*ry);
        }
        ShapeIR::Path { svg_path_d } => {
            h.write_u8(3);
            write_intern_id(h, *svg_path_d);
        }
    }
}

fn write_pass_fx(h: &mut StableHasher, fx: &PassFx) {
    match fx {
        PassFx::Blur { radius_px, sigma } => {
            h.write_u8(0);
            h.write_u32(*radius_px);
            h.write_f32(*sigma);
        }
        PassFx::ColorMatrix { matrix } => {
            h.write_u8(1);
            for v in matrix {
                h.write_f32(*v);
            }
        }
        PassFx::MaskApply { mode, inverted } => {
            h.write_u8(2);
            write_mask_mode(h, *mode);
            h.write_bool(*inverted);
        }
        PassFx::DropShadow {
            offset,
            blur_radius_px,
            sigma,
            color,
        } => {
            h.write_u8(3);
            h.write_f64(offset.x);
            h.write_f64(offset.y);
            h.write_u32(*blur_radius_px);
            h.write_f32(*sigma);
            h.write_u8(color.r);
            h.write_u8(color.g);
            h.write_u8(color.b);
            h.write_u8(color.a);
        }
    }
}

fn write_mask_gen_source(h: &mut StableHasher, src: &MaskGenSource) {
    match src {
        MaskGenSource::Asset(a) => {
            h.write_u8(0);
            h.write_u32(a.0);
        }
        MaskGenSource::Shape(s) => {
            h.write_u8(1);
            write_shape(h, s);
        }
    }
}

fn write_composite_op(h: &mut StableHasher, c: &CompositeOp) {
    match c {
        CompositeOp::Over {
            src,
            opacity,
            blend,
        } => {
            h.write_u8(0);
            write_surface_id(h, *src);
            h.write_f32(*opacity);
            write_blend_mode(h, *blend);
        }
        CompositeOp::Crossfade { a, b, t } => {
            h.write_u8(1);
            write_surface_id(h, *a);
            write_surface_id(h, *b);
            h.write_f32(*t);
        }
        CompositeOp::Wipe {
            a,
            b,
            t,
            dir,
            soft_edge,
        } => {
            h.write_u8(2);
            write_surface_id(h, *a);
            write_surface_id(h, *b);
            h.write_f32(*t);
            write_wipe_dir(h, *dir);
            h.write_f32(*soft_edge);
        }
        CompositeOp::Slide { a, b, t, dir, push } => {
            h.write_u8(3);
            write_surface_id(h, *a);
            write_surface_id(h, *b);
            h.write_f32(*t);
            write_slide_dir(h, *dir);
            h.write_bool(*push);
        }
        CompositeOp::Zoom {
            a,
            b,
            t,
            origin,
            from_scale,
        } => {
            h.write_u8(4);
            write_surface_id(h, *a);
            write_surface_id(h, *b);
            h.write_f32(*t);
            h.write_f64(origin.x);
            h.write_f64(origin.y);
            h.write_f32(*from_scale);
        }
        CompositeOp::Iris {
            a,
            b,
            t,
            origin,
            shape,
            soft_edge,
        } => {
            h.write_u8(5);
            write_surface_id(h, *a);
            write_surface_id(h, *b);
            h.write_f32(*t);
            h.write_f64(origin.x);
            h.write_f64(origin.y);
            write_iris_shape(h, *shape);
            h.write_f32(*soft_edge);
        }
    }
}

fn write_op(h: &mut StableHasher, output: SurfaceId, kind: &OpKind, inputs: &[SurfaceId]) {
    // Output surface and inputs are part of the op signature.
    write_surface_id(h, output);
    h.write_u32(inputs.len() as u32);
    for s in inputs {
        write_surface_id(h, *s);
    }

    match kind {
        OpKind::Draw {
            unit,
            leaves,
            clear_to_transparent,
            transform_post,
            opacity_mul,
        } => {
            h.write_u8(0);
            write_unit_key(h, *unit);
            h.write_u64(leaves.start as u64);
            h.write_u64(leaves.end as u64);
            h.write_bool(*clear_to_transparent);
            for c in transform_post.as_coeffs() {
                h.write_f64(c);
            }
            h.write_f32(*opacity_mul);
        }
        OpKind::Pass { fx } => {
            h.write_u8(1);
            write_pass_fx(h, fx);
        }
        OpKind::MaskGen { source } => {
            h.write_u8(2);
            write_mask_gen_source(h, source);
        }
        OpKind::Composite {
            clear_to_transparent,
            ops,
        } => {
            h.write_u8(3);
            h.write_bool(*clear_to_transparent);
            h.write_u32(ops.len() as u32);
            for c in ops.iter() {
                write_composite_op(h, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::compile::plan::{Op, OpId, SurfaceDesc};
    use smallvec::smallvec;

    #[test]
    fn fingerprint_is_deterministic_for_same_plan() {
        let plan = RenderPlan {
            surfaces: vec![SurfaceDesc {
                width: 1,
                height: 1,
                format: PixelFormat::Rgba8Premul,
            }],
            ops: vec![Op {
                id: OpId(0),
                kind: OpKind::Composite {
                    clear_to_transparent: true,
                    ops: Box::new({
                        let mut v = smallvec::SmallVec::<[CompositeOp; 8]>::new();
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
            }],
            roots: smallvec![SurfaceId(0)],
        };

        let a = fingerprint_plan(&plan);
        let b = fingerprint_plan(&plan);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_plan_changes() {
        let base = RenderPlan {
            surfaces: vec![SurfaceDesc {
                width: 1,
                height: 1,
                format: PixelFormat::Rgba8Premul,
            }],
            ops: vec![Op {
                id: OpId(0),
                kind: OpKind::Composite {
                    clear_to_transparent: true,
                    ops: Box::new({
                        let mut v = smallvec::SmallVec::<[CompositeOp; 8]>::new();
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
            }],
            roots: smallvec![SurfaceId(0)],
        };

        let mut changed = base.clone();
        if let OpKind::Composite { ops, .. } = &mut changed.ops[0].kind
            && let CompositeOp::Over { opacity, .. } = &mut ops[0]
        {
            *opacity = 0.5;
        }

        assert_ne!(fingerprint_plan(&base), fingerprint_plan(&changed));
    }
}
