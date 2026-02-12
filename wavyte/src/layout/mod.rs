//! v0.3 layout bridge (Taffy).
//!
//! Phase 4 wires this into the evaluator to compute per-frame layout rectangles and inject them
//! into the world transform chain.

pub(crate) mod cache;
pub(crate) mod taffy_bridge;

/// Pixel-space rectangle produced by the layout solver.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RectPx {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}
