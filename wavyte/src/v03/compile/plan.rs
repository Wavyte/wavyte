use crate::foundation::core::{Affine, Rgba8Premul, Vec2};
use crate::v03::foundation::ids::NodeIdx;
use smallvec::SmallVec;
use std::ops::Range;

#[derive(Debug, Clone)]
pub(crate) struct RenderPlan {
    pub(crate) surfaces: Vec<SurfaceDesc>,
    pub(crate) ops: Vec<Op>,
    pub(crate) roots: SmallVec<[SurfaceId; 2]>,
}

impl RenderPlan {
    pub(crate) fn dump(&self) -> String {
        // Deterministic debug dump used by determinism gates.
        // Intentionally avoids printing any addresses or non-deterministic map orderings.
        let mut s = String::new();
        s.push_str("RenderPlan\n");
        s.push_str(&format!("surfaces: {}\n", self.surfaces.len()));
        for (i, surf) in self.surfaces.iter().enumerate() {
            s.push_str(&format!(
                "  S{}: {}x{} {:?}\n",
                i, surf.width, surf.height, surf.format
            ));
        }
        s.push_str(&format!("ops: {}\n", self.ops.len()));
        for (i, op) in self.ops.iter().enumerate() {
            s.push_str(&format!(
                "  O{}: out=S{} deps={:?} inputs={:?} kind={:?}\n",
                i,
                op.output.0,
                op.deps.iter().map(|d| d.0).collect::<SmallVec<[u32; 4]>>(),
                op.inputs
                    .iter()
                    .map(|x| x.0)
                    .collect::<SmallVec<[u32; 4]>>(),
                op.kind
            ));
        }
        s.push_str(&format!(
            "roots: {:?}\n",
            self.roots
                .iter()
                .map(|r| r.0)
                .collect::<SmallVec<[u32; 2]>>()
        ));
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SurfaceId(pub(crate) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OpId(pub(crate) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PixelFormat {
    Rgba8Premul,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceDesc {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: PixelFormat,
}

#[derive(Debug, Clone)]
pub(crate) struct Op {
    pub(crate) id: OpId,
    pub(crate) kind: OpKind,
    pub(crate) inputs: SmallVec<[SurfaceId; 4]>,
    pub(crate) output: SurfaceId,
    pub(crate) deps: SmallVec<[OpId; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitKey {
    Leaf(NodeIdx),
    Group(NodeIdx),
    MaskNode(NodeIdx),
}

#[derive(Debug, Clone)]
pub(crate) enum OpKind {
    /// Rasterize/draw a set of leaves into `output`.
    ///
    /// `transform_post` and `opacity_mul` are compiler-level inline fusions that can be applied
    /// without introducing a full offscreen pass.
    Draw {
        unit: UnitKey,
        leaves: Range<usize>,
        clear_to_transparent: bool,
        transform_post: Affine,
        opacity_mul: f32,
    },
    /// Apply an offscreen effect pass (blur, color matrix, shadow, mask apply).
    ///
    /// Inputs must be explicitly listed in [`Op::inputs`] and are interpreted per-pass:
    /// - Blur, ColorMatrix, DropShadow: `[src]`
    /// - MaskApply: `[src, mask]`
    Pass { fx: PassFx },
    /// Composite multiple inputs into output (over + blend modes, paired transitions).
    Composite {
        clear_to_transparent: bool,
        // Boxed to keep `OpKind` size reasonable; composite lists can be large.
        ops: Box<SmallVec<[CompositeOp; 8]>>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum PassFx {
    Blur {
        radius_px: u32,
        sigma: f32,
    },
    ColorMatrix {
        matrix: [f32; 20],
    },
    MaskApply {
        mode: MaskMode,
        inverted: bool,
    },
    DropShadow {
        offset: Vec2,
        blur_radius_px: u32,
        sigma: f32,
        color: Rgba8Premul,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MaskMode {
    Alpha,
    Luma,
    Stencil { threshold: f32 }, // 0..1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WipeDir {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlideDir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrisShape {
    Circle,
    Rect,
    Diamond,
}

#[derive(Debug, Clone)]
pub(crate) enum CompositeOp {
    Over {
        src: SurfaceId,
        opacity: f32,
        blend: BlendMode,
    },
    Crossfade {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
    },
    Wipe {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
        dir: WipeDir,
        soft_edge: f32,
    },
    Slide {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
        dir: SlideDir,
        push: bool,
    },
    Zoom {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
        origin: Vec2,
        from_scale: f32,
    },
    Iris {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
        origin: Vec2,
        shape: IrisShape,
        soft_edge: f32,
    },
}
