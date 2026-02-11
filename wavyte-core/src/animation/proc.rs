use crate::{
    animation::anim::SampleCtx,
    foundation::core::{Fps, Transform2D, Vec2},
    foundation::error::{WavyteError, WavyteResult},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Procedural<T> {
    pub kind: ProceduralKind,
    #[serde(skip)]
    _marker: std::marker::PhantomData<T>,
}

impl<T> Procedural<T> {
    pub fn new(kind: ProceduralKind) -> Self {
        Self {
            kind,
            _marker: std::marker::PhantomData,
        }
    }
}

pub trait ProcValue: Sized {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self>;
}

impl<T> Procedural<T>
where
    T: ProcValue,
{
    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
        T::from_procedural(&self.kind, ctx)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "params")]
pub enum ProceduralKind {
    Scalar(ProcScalar),
    Vec2 { x: ProcScalar, y: ProcScalar },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ProcScalar {
    Sine {
        amp: f64,
        freq_hz: f64,
        phase: f64,
        offset: f64,
    },
    Noise1D {
        amp: f64,
        freq_hz: f64,
        offset: f64,
    },
    Envelope {
        attack: u64,
        decay: u64,
        sustain: f64,
        release: u64,
    },
    Spring {
        stiffness: f64,
        damping: f64,
        target: f64,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Rng64 {
    state: u64,
}

impl Rng64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        // SplitMix64
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_f64_01(&mut self) -> f64 {
        // 53 bits of precision.
        let v = self.next_u64() >> 11;
        (v as f64) * (1.0 / ((1u64 << 53) as f64))
    }
}

fn noise01(seed: u64, x: u64) -> f64 {
    let mut rng = Rng64::new(seed ^ x.wrapping_mul(0xD6E8_FEB8_6659_FD93));
    rng.next_f64_01()
}

fn sample_scalar(s: &ProcScalar, fps: Fps, frame: u64, seed: u64) -> f64 {
    let secs = fps.frames_to_secs(frame);
    match *s {
        ProcScalar::Sine {
            amp,
            freq_hz,
            phase,
            offset,
        } => offset + amp * (std::f64::consts::TAU * freq_hz * secs + phase).sin(),
        ProcScalar::Noise1D {
            amp,
            freq_hz,
            offset,
        } => {
            let x = secs * freq_hz;
            let i0 = x.floor();
            let t = x - i0;
            let i0u = i0.max(0.0) as u64;
            let i1u = i0u + 1;

            let a = noise01(seed, i0u) * 2.0 - 1.0;
            let b = noise01(seed, i1u) * 2.0 - 1.0;
            let v = a + (b - a) * t;
            offset + amp * v
        }
        ProcScalar::Envelope {
            attack,
            decay,
            sustain,
            release,
        } => {
            let f = frame;
            if attack > 0 && f < attack {
                f as f64 / attack as f64
            } else if decay > 0 && f < attack.saturating_add(decay) {
                let u = (f - attack) as f64 / decay as f64;
                1.0 + (sustain - 1.0) * u
            } else if release > 0 && f < attack.saturating_add(decay).saturating_add(release) {
                let u = (f - attack - decay) as f64 / release as f64;
                sustain * (1.0 - u)
            } else {
                0.0
            }
        }
        ProcScalar::Spring {
            stiffness,
            damping,
            target,
        } => {
            let omega = stiffness.max(0.0);
            let d = damping.max(0.0);
            let rate = (omega / (1.0 + d)).max(1e-6);
            let e = (-rate * secs).exp();
            // Critically-damped-like response.
            target * (1.0 - e * (1.0 + rate * secs))
        }
    }
}

impl ProcValue for f64 {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self> {
        match kind {
            ProceduralKind::Scalar(s) => Ok(sample_scalar(s, ctx.fps, ctx.clip_local.0, ctx.seed)),
            ProceduralKind::Vec2 { .. } => Err(WavyteError::animation(
                "procedural kind Vec2 cannot be sampled as f64",
            )),
        }
    }
}

impl ProcValue for f32 {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self> {
        Ok(f64::from_procedural(kind, ctx)? as f32)
    }
}

impl ProcValue for Vec2 {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self> {
        match kind {
            ProceduralKind::Scalar(_) => Err(WavyteError::animation(
                "procedural kind Scalar cannot be sampled as Vec2",
            )),
            ProceduralKind::Vec2 { x, y } => Ok(Vec2::new(
                sample_scalar(x, ctx.fps, ctx.clip_local.0, ctx.seed),
                sample_scalar(y, ctx.fps, ctx.clip_local.0, ctx.seed),
            )),
        }
    }
}

impl ProcValue for Transform2D {
    fn from_procedural(_kind: &ProceduralKind, _ctx: SampleCtx) -> WavyteResult<Self> {
        Err(WavyteError::animation(
            "procedural sampling for Transform2D is not supported in v0.2",
        ))
    }
}

impl ProcValue for crate::foundation::core::Rgba8Premul {
    fn from_procedural(_kind: &ProceduralKind, _ctx: SampleCtx) -> WavyteResult<Self> {
        Err(WavyteError::animation(
            "procedural sampling for Rgba8Premul is not supported in v0.2",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::core::{Fps, FrameIndex};

    fn ctx(frame: u64, seed: u64) -> SampleCtx {
        SampleCtx {
            frame: FrameIndex(frame),
            fps: Fps::new(30, 1).unwrap(),
            clip_local: FrameIndex(frame),
            seed,
        }
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng64::new(123);
        let mut b = Rng64::new(123);
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn noise_is_bounded_and_deterministic() {
        let proc = Procedural::<f64>::new(ProceduralKind::Scalar(ProcScalar::Noise1D {
            amp: 2.0,
            freq_hz: 1.0,
            offset: 0.5,
        }));
        let v0 = proc.sample(ctx(0, 7)).unwrap();
        let v1 = proc.sample(ctx(1, 7)).unwrap();
        assert_ne!(v0, v1);
        for v in [v0, v1] {
            assert!(v >= -1.5);
            assert!(v <= 2.5);
        }
        assert_eq!(v0, proc.sample(ctx(0, 7)).unwrap());
    }

    #[test]
    fn envelope_basic_boundaries() {
        let proc = Procedural::<f64>::new(ProceduralKind::Scalar(ProcScalar::Envelope {
            attack: 10,
            decay: 10,
            sustain: 0.25,
            release: 10,
        }));
        assert_eq!(proc.sample(ctx(0, 0)).unwrap(), 0.0);
        assert!((proc.sample(ctx(10, 0)).unwrap() - 1.0).abs() < 1e-9);
        let at_sustain = proc.sample(ctx(20, 0)).unwrap();
        assert!((at_sustain - 0.25).abs() < 1e-9);
        let released = proc.sample(ctx(30, 0)).unwrap();
        assert!((released - 0.0).abs() < 1e-9);
    }
}
