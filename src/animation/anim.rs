use crate::{
    anim_ease::Ease,
    anim_proc::{ProcValue, Procedural},
    core::{FrameIndex, Transform2D, Vec2},
    error::{WavyteError, WavyteResult},
};

#[derive(Clone, Copy, Debug)]
pub struct SampleCtx {
    pub frame: FrameIndex,      // global frame
    pub fps: crate::core::Fps,  // global fps
    pub clip_local: FrameIndex, // frame - clip.start
    pub seed: u64,              // deterministic seed for procs
}

pub trait Lerp: Sized {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self;
}

impl Lerp for f64 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        a + (b - a) * t
    }
}

impl Lerp for f32 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        (*a as f64 + ((*b as f64 - *a as f64) * t)) as f32
    }
}

impl Lerp for Vec2 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    }
}

impl Lerp for Transform2D {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        Self {
            translate: <Vec2 as Lerp>::lerp(&a.translate, &b.translate, t),
            rotation_rad: a.rotation_rad + (b.rotation_rad - a.rotation_rad) * t,
            scale: <Vec2 as Lerp>::lerp(&a.scale, &b.scale, t),
            anchor: <Vec2 as Lerp>::lerp(&a.anchor, &b.anchor, t),
        }
    }
}

impl Lerp for crate::core::Rgba8Premul {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
            let a = f64::from(a);
            let b = f64::from(b);
            (a + (b - a) * t).round().clamp(0.0, 255.0) as u8
        }

        Self {
            r: lerp_u8(a.r, b.r, t),
            g: lerp_u8(a.g, b.g, t),
            b: lerp_u8(a.b, b.b, t),
            a: lerp_u8(a.a, b.a, t),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Anim<T> {
    Keyframes(Keyframes<T>),
    Procedural(Procedural<T>),
    Expr(Expr<T>),
}

impl<T> Anim<T>
where
    T: Lerp + Clone + ProcValue,
{
    pub fn constant(value: T) -> Self {
        Self::Keyframes(Keyframes {
            keys: vec![Keyframe {
                frame: FrameIndex(0),
                value,
                ease: Ease::Linear,
            }],
            mode: InterpMode::Hold,
            default: None,
        })
    }

    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
        match self {
            Self::Keyframes(kf) => kf.sample(ctx),
            Self::Procedural(proc) => proc.sample(ctx),
            Self::Expr(expr) => expr.sample(ctx),
        }
    }

    pub fn validate(&self) -> WavyteResult<()> {
        match self {
            Self::Keyframes(kf) => kf.validate(),
            Self::Procedural(_proc) => Ok(()),
            Self::Expr(expr) => expr.validate(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Keyframes<T> {
    pub keys: Vec<Keyframe<T>>, // sorted by frame
    pub mode: InterpMode,       // linear/hold
    pub default: Option<T>,     // value when no keys exist
}

impl<T> Keyframes<T>
where
    T: Lerp + Clone,
{
    pub fn validate(&self) -> WavyteResult<()> {
        if self.keys.is_empty() && self.default.is_none() {
            return Err(WavyteError::animation(
                "Keyframes must have at least one key or a default value",
            ));
        }
        if !self.keys.windows(2).all(|w| w[0].frame.0 <= w[1].frame.0) {
            return Err(WavyteError::animation(
                "Keyframes keys must be sorted by frame",
            ));
        }
        Ok(())
    }

    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
        if self.keys.is_empty() {
            return self
                .default
                .clone()
                .ok_or_else(|| WavyteError::animation("Keyframes has no keys and no default"));
        }

        let f = ctx.clip_local.0;
        let idx = self.keys.partition_point(|k| k.frame.0 <= f);

        if idx == 0 {
            return Ok(self.keys[0].value.clone());
        }
        if idx >= self.keys.len() {
            return Ok(self.keys[self.keys.len() - 1].value.clone());
        }

        let a = &self.keys[idx - 1];
        let b = &self.keys[idx];
        let denom = b.frame.0.saturating_sub(a.frame.0);
        if denom == 0 {
            return Ok(a.value.clone());
        }

        let t = ((f - a.frame.0) as f64) / (denom as f64);
        let te = a.ease.apply(t);
        match self.mode {
            InterpMode::Hold => Ok(a.value.clone()),
            InterpMode::Linear => Ok(T::lerp(&a.value, &b.value, te)),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Keyframe<T> {
    pub frame: FrameIndex,
    pub value: T,
    pub ease: Ease, // ease applied toward next key
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum InterpMode {
    Hold,
    Linear,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Expr<T> {
    Delay {
        inner: Box<Anim<T>>,
        by: u64,
    },
    Speed {
        inner: Box<Anim<T>>,
        factor: f64,
    }, // factor>0
    Reverse {
        inner: Box<Anim<T>>,
        duration: u64,
    }, // duration in frames
    Loop {
        inner: Box<Anim<T>>,
        period: u64,
        mode: LoopMode,
    },
    Mix {
        a: Box<Anim<T>>,
        b: Box<Anim<T>>,
        t: Box<Anim<f64>>,
    },
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum LoopMode {
    Repeat,
    PingPong,
}

impl<T> Expr<T>
where
    T: Lerp + Clone + ProcValue,
{
    pub fn validate(&self) -> WavyteResult<()> {
        match self {
            Self::Delay { inner, by: _ } => inner.validate(),
            Self::Speed { inner, factor } => {
                if *factor <= 0.0 {
                    return Err(WavyteError::animation("Speed factor must be > 0"));
                }
                inner.validate()
            }
            Self::Reverse { inner, duration } => {
                if *duration == 0 {
                    return Err(WavyteError::animation("Reverse duration must be > 0"));
                }
                inner.validate()
            }
            Self::Loop {
                inner,
                period,
                mode: _,
            } => {
                if *period == 0 {
                    return Err(WavyteError::animation("Loop period must be > 0"));
                }
                inner.validate()
            }
            Self::Mix { a, b, t } => {
                a.validate()?;
                b.validate()?;
                t.validate()
            }
        }
    }

    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
        fn with_clip_local(mut ctx: SampleCtx, clip_local: FrameIndex) -> SampleCtx {
            let delta = clip_local.0 as i128 - ctx.clip_local.0 as i128;
            let new_frame = if delta >= 0 {
                ctx.frame.0.saturating_add(delta as u64)
            } else {
                ctx.frame.0.saturating_sub((-delta) as u64)
            };
            ctx.frame = FrameIndex(new_frame);
            ctx.clip_local = clip_local;
            ctx
        }

        match self {
            Self::Delay { inner, by } => {
                let f = ctx.clip_local.0;
                let mapped = FrameIndex(if f < *by { 0 } else { f - by });
                inner.sample(with_clip_local(ctx, mapped))
            }
            Self::Speed { inner, factor } => {
                if *factor <= 0.0 {
                    return Err(WavyteError::animation("Speed factor must be > 0"));
                }
                let f = ctx.clip_local.0 as f64;
                let mapped = FrameIndex((f * factor).floor().max(0.0) as u64);
                inner.sample(with_clip_local(ctx, mapped))
            }
            Self::Reverse { inner, duration } => {
                if *duration == 0 {
                    return Err(WavyteError::animation("Reverse duration must be > 0"));
                }
                let max = duration - 1;
                let f = ctx.clip_local.0.min(max);
                let mapped = FrameIndex(max - f);
                inner.sample(with_clip_local(ctx, mapped))
            }
            Self::Loop {
                inner,
                period,
                mode,
            } => {
                if *period == 0 {
                    return Err(WavyteError::animation("Loop period must be > 0"));
                }
                let f = ctx.clip_local.0;
                let mapped = match mode {
                    LoopMode::Repeat => FrameIndex(f % period),
                    LoopMode::PingPong => {
                        if *period == 1 {
                            FrameIndex(0)
                        } else {
                            let cycle = 2 * (period - 1);
                            let pos = f % cycle;
                            if pos < *period {
                                FrameIndex(pos)
                            } else {
                                FrameIndex(cycle - pos)
                            }
                        }
                    }
                };
                inner.sample(with_clip_local(ctx, mapped))
            }
            Self::Mix { a, b, t } => {
                let tt = t.sample(ctx)?.clamp(0.0, 1.0);
                let av = a.sample(ctx)?;
                let bv = b.sample(ctx)?;
                Ok(T::lerp(&av, &bv, tt))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Fps;

    fn ctx(frame: u64) -> SampleCtx {
        SampleCtx {
            frame: FrameIndex(frame),
            fps: Fps::new(30, 1).unwrap(),
            clip_local: FrameIndex(frame),
            seed: 0,
        }
    }

    #[test]
    fn keyframes_hold_is_constant_between_keys() {
        let anim = Anim::Keyframes(Keyframes {
            keys: vec![
                Keyframe {
                    frame: FrameIndex(0),
                    value: 1.0,
                    ease: Ease::Linear,
                },
                Keyframe {
                    frame: FrameIndex(10),
                    value: 3.0,
                    ease: Ease::Linear,
                },
            ],
            mode: InterpMode::Hold,
            default: None,
        });
        assert_eq!(anim.sample(ctx(5)).unwrap(), 1.0);
        assert_eq!(anim.sample(ctx(10)).unwrap(), 3.0);
    }

    #[test]
    fn keyframes_linear_interpolates() {
        let anim = Anim::Keyframes(Keyframes {
            keys: vec![
                Keyframe {
                    frame: FrameIndex(0),
                    value: 0.0,
                    ease: Ease::Linear,
                },
                Keyframe {
                    frame: FrameIndex(10),
                    value: 10.0,
                    ease: Ease::Linear,
                },
            ],
            mode: InterpMode::Linear,
            default: None,
        });
        assert_eq!(anim.sample(ctx(5)).unwrap(), 5.0);
    }

    #[test]
    fn expr_reverse_maps_frames() {
        let inner = Anim::Keyframes(Keyframes {
            keys: vec![
                Keyframe {
                    frame: FrameIndex(0),
                    value: 0.0,
                    ease: Ease::Linear,
                },
                Keyframe {
                    frame: FrameIndex(9),
                    value: 9.0,
                    ease: Ease::Linear,
                },
            ],
            mode: InterpMode::Hold,
            default: None,
        });
        let rev = Anim::Expr(Expr::Reverse {
            inner: Box::new(inner),
            duration: 10,
        });
        assert_eq!(rev.sample(ctx(0)).unwrap(), 9.0);
        assert_eq!(rev.sample(ctx(9)).unwrap(), 0.0);
    }
}
