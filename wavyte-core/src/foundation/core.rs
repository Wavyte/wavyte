use crate::foundation::error::{WavyteError, WavyteResult};

pub use kurbo::{Affine, BezPath, Point, Rect, Vec2};

/// Absolute 0-based frame index in composition timeline space.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct FrameIndex(pub u64);

/// Half-open frame range `[start, end)` in timeline space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrameRange {
    /// Inclusive range start.
    pub start: FrameIndex,
    /// Exclusive range end.
    pub end: FrameIndex, // exclusive
}

impl FrameRange {
    /// Create a validated range with `start <= end`.
    pub fn new(start: FrameIndex, end: FrameIndex) -> WavyteResult<Self> {
        if start.0 > end.0 {
            return Err(WavyteError::validation("FrameRange start must be <= end"));
        }
        Ok(Self { start, end })
    }

    /// Number of frames contained in the range.
    pub fn len_frames(self) -> u64 {
        self.end.0.saturating_sub(self.start.0)
    }

    /// Return `true` when the range has no frames.
    pub fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    /// Return `true` when `f` is inside `[start, end)`.
    pub fn contains(self, f: FrameIndex) -> bool {
        self.start.0 <= f.0 && f.0 < self.end.0
    }

    /// Clamp a frame index into this range.
    ///
    /// Empty ranges clamp to `start`.
    pub fn clamp(self, f: FrameIndex) -> FrameIndex {
        if self.is_empty() {
            return self.start;
        }
        let max_inclusive = self.end.0.saturating_sub(1);
        FrameIndex(f.0.clamp(self.start.0, max_inclusive))
    }

    /// Shift both bounds by `delta` frames using saturating arithmetic.
    pub fn shift(self, delta: i64) -> Self {
        fn shift_idx(v: u64, delta: i64) -> u64 {
            if delta >= 0 {
                v.saturating_add(delta as u64)
            } else {
                v.saturating_sub(delta.unsigned_abs())
            }
        }

        Self {
            start: FrameIndex(shift_idx(self.start.0, delta)),
            end: FrameIndex(shift_idx(self.end.0, delta)),
        }
    }
}

/// Frames-per-second represented as a rational `num/den`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fps {
    /// Numerator (frames).
    pub num: u32,
    /// Denominator (seconds), must be non-zero.
    pub den: u32, // must be > 0
}

impl Fps {
    /// Create a validated FPS value.
    pub fn new(num: u32, den: u32) -> WavyteResult<Self> {
        if den == 0 {
            return Err(WavyteError::validation("Fps den must be > 0"));
        }
        if num == 0 {
            return Err(WavyteError::validation("Fps num must be > 0"));
        }
        Ok(Self { num, den })
    }

    /// Convert to floating-point FPS.
    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Duration of one frame in seconds.
    pub fn frame_duration_secs(self) -> f64 {
        f64::from(self.den) / f64::from(self.num)
    }

    /// Convert frame count to seconds.
    pub fn frames_to_secs(self, frames: u64) -> f64 {
        (frames as f64) * self.frame_duration_secs()
    }

    /// Convert seconds to frame count using floor semantics.
    pub fn secs_to_frames_floor(self, secs: f64) -> u64 {
        (secs * self.as_f64()).floor().max(0.0) as u64
    }
}

/// Output canvas dimensions in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Canvas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Premultiplied RGBA8 (r,g,b already multiplied by a).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rgba8Premul {
    /// Red channel premultiplied by alpha.
    pub r: u8,
    /// Green channel premultiplied by alpha.
    pub g: u8,
    /// Blue channel premultiplied by alpha.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

impl Rgba8Premul {
    /// Fully transparent black.
    pub fn transparent() -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }

    /// Convert straight-alpha RGBA8 into premultiplied RGBA8.
    pub fn from_straight_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        fn premul(c: u8, a: u8) -> u8 {
            let c = u16::from(c);
            let a = u16::from(a);
            (((c * a) + 127) / 255) as u8
        }

        Self {
            r: premul(r, a),
            g: premul(g, a),
            b: premul(b, a),
            a,
        }
    }
}

/// Authoring-space transform components for a clip.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform2D {
    /// Translation in pixels.
    pub translate: Vec2,
    /// Rotation in radians around `anchor`.
    pub rotation_rad: f64,
    /// Non-uniform scale.
    pub scale: Vec2, // default (1,1)
    /// Pivot point in local clip space.
    pub anchor: Vec2, // pivot in local space
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            translate: Vec2::ZERO,
            rotation_rad: 0.0,
            scale: Vec2::new(1.0, 1.0),
            anchor: Vec2::ZERO,
        }
    }
}

impl Transform2D {
    /// Convert this decomposed transform into a single affine matrix.
    ///
    /// Order is `T(translate) * T(anchor) * R(rotation) * S(scale) * T(-anchor)`.
    pub fn to_affine(self) -> kurbo::Affine {
        let t_translate = kurbo::Affine::translate(self.translate);
        let t_anchor = kurbo::Affine::translate(self.anchor);
        let t_unanchor = kurbo::Affine::translate(-self.anchor);
        let t_rotate = kurbo::Affine::rotate(self.rotation_rad);
        let t_scale = kurbo::Affine::scale_non_uniform(self.scale.x, self.scale.y);

        // Canonical order:
        // T(translate) * T(anchor) * R(rot) * S(scale) * T(-anchor)
        t_translate * t_anchor * t_rotate * t_scale * t_unanchor
    }
}

#[cfg(test)]
#[path = "../../tests/unit/foundation/core.rs"]
mod tests;
