/// Easing functions used to map normalized animation progress.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum Ease {
    /// Linear interpolation.
    Linear,
    /// Quadratic ease-in.
    InQuad,
    /// Quadratic ease-out.
    OutQuad,
    /// Quadratic ease-in/out.
    InOutQuad,
    /// Cubic ease-in.
    InCubic,
    /// Cubic ease-out.
    OutCubic,
    /// Cubic ease-in/out.
    InOutCubic,
}

impl Ease {
    /// Apply this easing function to normalized progress `t` in `[0, 1]`.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::InQuad => t * t,
            Self::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Self::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - ((-2.0 * t + 2.0).powi(2) / 2.0)
                }
            }
            Self::InCubic => t * t * t,
            Self::OutCubic => 1.0 - (1.0 - t).powi(3),
            Self::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - ((-2.0 * t + 2.0).powi(3) / 2.0)
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/animation/ease.rs"]
mod tests;
