//! Non-linear transform utilities.

#[inline]
/// Clamp scalar value to normalized range `[0, 1]`.
pub fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}
