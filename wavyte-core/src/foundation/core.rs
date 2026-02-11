use crate::foundation::error::{WavyteError, WavyteResult};

pub use kurbo::{Affine, BezPath, Point, Rect, Vec2};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct FrameIndex(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrameRange {
    pub start: FrameIndex,
    pub end: FrameIndex, // exclusive
}

impl FrameRange {
    pub fn new(start: FrameIndex, end: FrameIndex) -> WavyteResult<Self> {
        if start.0 > end.0 {
            return Err(WavyteError::validation("FrameRange start must be <= end"));
        }
        Ok(Self { start, end })
    }

    pub fn len_frames(self) -> u64 {
        self.end.0.saturating_sub(self.start.0)
    }

    pub fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    pub fn contains(self, f: FrameIndex) -> bool {
        self.start.0 <= f.0 && f.0 < self.end.0
    }

    pub fn clamp(self, f: FrameIndex) -> FrameIndex {
        if self.is_empty() {
            return self.start;
        }
        let max_inclusive = self.end.0.saturating_sub(1);
        FrameIndex(f.0.clamp(self.start.0, max_inclusive))
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fps {
    pub num: u32,
    pub den: u32, // must be > 0
}

impl Fps {
    pub fn new(num: u32, den: u32) -> WavyteResult<Self> {
        if den == 0 {
            return Err(WavyteError::validation("Fps den must be > 0"));
        }
        if num == 0 {
            return Err(WavyteError::validation("Fps num must be > 0"));
        }
        Ok(Self { num, den })
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    pub fn frame_duration_secs(self) -> f64 {
        f64::from(self.den) / f64::from(self.num)
    }

    pub fn frames_to_secs(self, frames: u64) -> f64 {
        (frames as f64) * self.frame_duration_secs()
    }

    pub fn secs_to_frames_floor(self, secs: f64) -> u64 {
        (secs * self.as_f64()).floor().max(0.0) as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

/// Premultiplied RGBA8 (r,g,b already multiplied by a).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rgba8Premul {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8Premul {
    pub fn transparent() -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }

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

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform2D {
    pub translate: Vec2,
    pub rotation_rad: f64,
    pub scale: Vec2,  // default (1,1)
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
mod tests {
    use super::*;

    #[test]
    fn frame_range_contains_boundaries() {
        let r = FrameRange::new(FrameIndex(2), FrameIndex(5)).unwrap();
        assert!(!r.contains(FrameIndex(1)));
        assert!(r.contains(FrameIndex(2)));
        assert!(r.contains(FrameIndex(4)));
        assert!(!r.contains(FrameIndex(5)));
    }

    #[test]
    fn fps_frames_secs_roundtrip_floor() {
        let fps = Fps::new(30000, 1001).unwrap();
        let secs = fps.frames_to_secs(123);
        assert_eq!(fps.secs_to_frames_floor(secs), 123);
    }

    #[test]
    fn transform_to_affine_identity_and_translation() {
        let t = Transform2D::default();
        assert_eq!(t.to_affine(), kurbo::Affine::IDENTITY);

        let t = Transform2D {
            translate: Vec2::new(10.0, -2.5),
            ..Transform2D::default()
        };
        assert_eq!(
            t.to_affine(),
            kurbo::Affine::translate(Vec2::new(10.0, -2.5))
        );
    }
}
