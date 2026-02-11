//! Linear transform helpers.

use crate::foundation::core::Vec2;

#[inline]
/// Linearly interpolate between two vectors with clamped parameter `t`.
pub fn lerp_vec2(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    let t = t.clamp(0.0, 1.0);
    Vec2::new(a.x + ((b.x - a.x) * t), a.y + ((b.y - a.y) * t))
}
