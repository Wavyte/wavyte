use crate::foundation::core::{Fps, Rgba8Premul};
use crate::v03::foundation::ids::PropertyId;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

// ----------------------------
// Boundary (serde) structures
// ----------------------------

#[derive(Debug, Clone, Serialize)]
pub(crate) struct KeyframesDef<T> {
    pub(crate) keys: Vec<KeyframeDef<T>>,
    #[serde(default)]
    pub(crate) mode: InterpModeDef,
    #[serde(default)]
    pub(crate) default: Option<T>,
}

impl<'de, T> Deserialize<'de> for KeyframesDef<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr<T> {
            List(Vec<KeyframeDef<T>>),
            Obj {
                keys: Option<Vec<KeyframeDef<T>>>,
                mode: Option<InterpModeDef>,
                default: Option<T>,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::List(keys) => Ok(Self {
                keys,
                mode: InterpModeDef::default(),
                default: None,
            }),
            Repr::Obj {
                keys,
                mode,
                default,
            } => Ok(Self {
                keys: keys.unwrap_or_default(),
                mode: mode.unwrap_or_default(),
                default,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct KeyframeDef<T> {
    pub(crate) frame: u64,
    pub(crate) value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
pub(crate) enum InterpModeDef {
    Hold,
    #[default]
    Linear,
    CubicBezier {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    Spring {
        stiffness: f64,
        damping: f64,
        mass: f64,
    },
    EaseIn,
    EaseOut,
    EaseInOut,
    ElasticOut,
    BounceOut,
}

impl<'de> Deserialize<'de> for InterpModeDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            CubicBezier { cubic_bezier: [f64; 4] },
            Spring { spring: SpringParams },
        }

        #[derive(Deserialize)]
        struct SpringParams {
            stiffness: f64,
            damping: f64,
            mass: f64,
        }

        let v = Repr::deserialize(deserializer)?;
        match v {
            Repr::Str(s) => match s.as_str() {
                "hold" => Ok(Self::Hold),
                "linear" => Ok(Self::Linear),
                "ease_in" => Ok(Self::EaseIn),
                "ease_out" => Ok(Self::EaseOut),
                "ease_in_out" => Ok(Self::EaseInOut),
                "elastic_out" => Ok(Self::ElasticOut),
                "bounce_out" => Ok(Self::BounceOut),
                other => Err(serde::de::Error::custom(format!(
                    "unknown interp mode \"{other}\""
                ))),
            },
            Repr::CubicBezier { cubic_bezier } => Ok(Self::CubicBezier {
                x1: cubic_bezier[0],
                y1: cubic_bezier[1],
                x2: cubic_bezier[2],
                y2: cubic_bezier[3],
            }),
            Repr::Spring { spring } => Ok(Self::Spring {
                stiffness: spring.stiffness,
                damping: spring.damping,
                mass: spring.mass,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnimDef<T> {
    /// JSON shorthand: a bare value is a constant animation.
    Constant(T),
    /// Full form: tagged object.
    Tagged(AnimTaggedDef<T>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnimTaggedDef<T> {
    Keyframes(KeyframesDef<T>),
    Procedural(ProceduralDef),
    /// Raw expression string (prefixed with `=`). Compiled into `Reference(PropertyId)`.
    Expr(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "params", rename_all = "snake_case")]
pub(crate) enum ProceduralDef {
    Scalar(ProcScalarDef),
    Vec2 { x: ProcScalarDef, y: ProcScalarDef },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcScalarDef {
    Sine {
        amp: f64,
        freq_hz: f64,
        phase: f64,
        offset: f64,
    },
    Noise1d {
        amp: f64,
        freq_hz: f64,
        offset: f64,
    },
}

// ----------------------------
// Runtime structures
// ----------------------------

#[derive(Debug, Clone)]
pub(crate) enum Anim<T> {
    Constant(T),
    Keyframes(Keyframes<T>),
    Procedural(Procedural<T>),
    Reference(PropertyId),
}

#[derive(Debug, Clone)]
pub(crate) struct Keyframes<T> {
    pub(crate) keys: Vec<Keyframe<T>>,
    pub(crate) mode: InterpMode,
    pub(crate) default: Option<T>,
}

#[derive(Debug, Clone)]
pub(crate) struct Keyframe<T> {
    pub(crate) frame: u64,
    pub(crate) value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum InterpMode {
    Hold,
    #[default]
    Linear,
    CubicBezier {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    Spring {
        stiffness: f64,
        damping: f64,
        mass: f64,
    },
    EaseIn,
    EaseOut,
    EaseInOut,
    ElasticOut,
    BounceOut,
}

impl From<InterpModeDef> for InterpMode {
    fn from(v: InterpModeDef) -> Self {
        match v {
            InterpModeDef::Hold => Self::Hold,
            InterpModeDef::Linear => Self::Linear,
            InterpModeDef::CubicBezier { x1, y1, x2, y2 } => Self::CubicBezier { x1, y1, x2, y2 },
            InterpModeDef::Spring {
                stiffness,
                damping,
                mass,
            } => Self::Spring {
                stiffness,
                damping,
                mass,
            },
            InterpModeDef::EaseIn => Self::EaseIn,
            InterpModeDef::EaseOut => Self::EaseOut,
            InterpModeDef::EaseInOut => Self::EaseInOut,
            InterpModeDef::ElasticOut => Self::ElasticOut,
            InterpModeDef::BounceOut => Self::BounceOut,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Procedural<T> {
    pub(crate) kind: ProceduralKind,
    _marker: PhantomData<T>,
}

impl<T> Procedural<T> {
    pub(crate) fn new(kind: ProceduralKind) -> Self {
        Self {
            kind,
            _marker: PhantomData,
        }
    }
}

pub(crate) trait ProcValue: Sized {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> Self;
}

impl<T> Procedural<T>
where
    T: ProcValue,
{
    pub(crate) fn sample(&self, ctx: SampleCtx) -> T {
        T::from_procedural(&self.kind, ctx)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ProceduralKind {
    Scalar(ProcScalar),
    Vec2 { x: ProcScalar, y: ProcScalar },
}

#[derive(Debug, Clone)]
pub(crate) enum ProcScalar {
    Sine {
        amp: f64,
        freq_hz: f64,
        phase: f64,
        offset: f64,
    },
    Noise1d {
        amp: f64,
        freq_hz: f64,
        offset: f64,
    },
}

impl From<ProceduralDef> for ProceduralKind {
    fn from(v: ProceduralDef) -> Self {
        match v {
            ProceduralDef::Scalar(s) => Self::Scalar(s.into()),
            ProceduralDef::Vec2 { x, y } => Self::Vec2 {
                x: x.into(),
                y: y.into(),
            },
        }
    }
}

impl From<ProcScalarDef> for ProcScalar {
    fn from(v: ProcScalarDef) -> Self {
        match v {
            ProcScalarDef::Sine {
                amp,
                freq_hz,
                phase,
                offset,
            } => Self::Sine {
                amp,
                freq_hz,
                phase,
                offset,
            },
            ProcScalarDef::Noise1d {
                amp,
                freq_hz,
                offset,
            } => Self::Noise1d {
                amp,
                freq_hz,
                offset,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SampleCtx {
    pub(crate) fps: Fps,
    pub(crate) frame: u64,
    pub(crate) seed: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng64 {
    state: u64,
}

impl Rng64 {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        // SplitMix64
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn next_f64_01(&mut self) -> f64 {
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
        ProcScalar::Noise1d {
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
    }
}

impl ProcValue for f64 {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> Self {
        match kind {
            ProceduralKind::Scalar(s) => sample_scalar(s, ctx.fps, ctx.frame, ctx.seed),
            ProceduralKind::Vec2 { .. } => 0.0,
        }
    }
}

impl ProcValue for (f64, f64) {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> Self {
        match kind {
            ProceduralKind::Scalar(s) => {
                let v = sample_scalar(s, ctx.fps, ctx.frame, ctx.seed);
                (v, v)
            }
            ProceduralKind::Vec2 { x, y } => (
                sample_scalar(x, ctx.fps, ctx.frame, ctx.seed),
                sample_scalar(y, ctx.fps, ctx.frame, ctx.seed),
            ),
        }
    }
}

impl ProcValue for u64 {
    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> Self {
        match kind {
            ProceduralKind::Scalar(s) => {
                let v = sample_scalar(s, ctx.fps, ctx.frame, ctx.seed);
                if !v.is_finite() {
                    return 0;
                }
                v.floor().max(0.0) as u64
            }
            ProceduralKind::Vec2 { .. } => 0,
        }
    }
}

pub(crate) trait Lerp: Sized {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self;
}

impl Lerp for f64 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        a + (b - a) * t
    }
}

impl Lerp for Rgba8Premul {
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

impl Lerp for u64 {
    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        // Used for keyframes on discrete lanes like `switch.active`. v0.3 intends
        // these to be `hold` in most cases, but we define linear interpolation to
        // avoid panics when sampling.
        let a = *a as f64;
        let b = *b as f64;
        (a + (b - a) * t).round().max(0.0) as u64
    }
}

impl<T> Keyframes<T>
where
    T: Lerp + Clone,
{
    pub(crate) fn sample(&self, frame: u64) -> T {
        if self.keys.is_empty() {
            return self.default.clone().unwrap_or_else(|| {
                // Default should be validated at schema/normalize layer for animation-bearing fields.
                panic!("Keyframes has no keys and no default");
            });
        }

        let idx = self.keys.partition_point(|k| k.frame <= frame);
        if idx == 0 {
            return self.keys[0].value.clone();
        }
        if idx >= self.keys.len() {
            return self.keys[self.keys.len() - 1].value.clone();
        }

        let a = &self.keys[idx - 1];
        let b = &self.keys[idx];
        let denom = b.frame.saturating_sub(a.frame);
        if denom == 0 {
            return a.value.clone();
        }
        let t = ((frame - a.frame) as f64) / (denom as f64);
        let te = self.mode.apply(t);
        match self.mode {
            InterpMode::Hold => a.value.clone(),
            _ => T::lerp(&a.value, &b.value, te),
        }
    }
}

impl InterpMode {
    pub(crate) fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Hold => 0.0,
            Self::Linear => t,
            Self::CubicBezier { x1, y1, x2, y2 } => cubic_bezier_ease(t, x1, y1, x2, y2),
            Self::Spring {
                stiffness,
                damping,
                mass,
            } => spring_step(t, stiffness, damping, mass),
            Self::EaseIn => cubic_bezier_ease(t, 0.42, 0.0, 1.0, 1.0),
            Self::EaseOut => cubic_bezier_ease(t, 0.0, 0.0, 0.58, 1.0),
            Self::EaseInOut => cubic_bezier_ease(t, 0.42, 0.0, 0.58, 1.0),
            Self::ElasticOut => elastic_out(t),
            Self::BounceOut => bounce_out(t),
        }
    }
}

fn cubic_bezier_ease(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // CSS cubic-bezier: given x in [0,1], solve u such that bx(u)=x, then return by(u).
    fn sample_curve(a1: f64, a2: f64, t: f64) -> f64 {
        let omt = 1.0 - t;
        3.0 * omt * omt * t * a1 + 3.0 * omt * t * t * a2 + t * t * t
    }
    fn sample_curve_derivative(a1: f64, a2: f64, t: f64) -> f64 {
        let omt = 1.0 - t;
        3.0 * omt * omt * a1 + 6.0 * omt * t * (a2 - a1) + 3.0 * t * t * (1.0 - a2)
    }

    // Newton-Raphson with bisection fallback (fixed iterations, no adaptive loops).
    let mut t = x;
    for _ in 0..8 {
        let x_t = sample_curve(x1, x2, t) - x;
        let d = sample_curve_derivative(x1, x2, t);
        if d.abs() < 1e-7 {
            break;
        }
        t = (t - x_t / d).clamp(0.0, 1.0);
    }

    // Refine with a few bisection steps to avoid edge cases.
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..8 {
        let x_t = sample_curve(x1, x2, t);
        if x_t < x {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }

    sample_curve(y1, y2, t)
}

fn spring_step(t: f64, stiffness: f64, damping: f64, mass: f64) -> f64 {
    // Step response from 0 to 1 with x(0)=0, v(0)=0.
    let k = stiffness.max(0.0);
    let c = damping.max(0.0);
    let m = mass.max(1e-9);

    let w0 = (k / m).sqrt();
    if w0 == 0.0 {
        return t;
    }
    let zeta = c / (2.0 * (k * m).sqrt()).max(1e-9);

    if (zeta - 1.0).abs() < 1e-6 {
        // Critically damped.
        let e = (-w0 * t).exp();
        1.0 - e * (1.0 + w0 * t)
    } else if zeta < 1.0 {
        // Underdamped.
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        let e = (-zeta * w0 * t).exp();
        let c1 = (wd * t).cos();
        let s1 = (wd * t).sin();
        let k = zeta / (1.0 - zeta * zeta).sqrt();
        1.0 - e * (c1 + k * s1)
    } else {
        // Overdamped.
        let z2 = (zeta * zeta - 1.0).sqrt();
        let r1 = -w0 * (zeta - z2);
        let r2 = -w0 * (zeta + z2);
        let c2 = (zeta + z2) / (2.0 * z2);
        let c1 = (zeta - z2) / (2.0 * z2);
        1.0 - (c2 * (r1 * t).exp() - c1 * (r2 * t).exp())
    }
}

fn elastic_out(t: f64) -> f64 {
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let p = 0.3;
    (2f64).powf(-10.0 * t) * ((t - p / 4.0) * (2.0 * std::f64::consts::PI) / p).sin() + 1.0
}

fn bounce_out(t: f64) -> f64 {
    // Standard piecewise bounce.
    let n1 = 7.5625;
    let d1 = 2.75;

    if t < 1.0 / d1 {
        n1 * t * t
    } else if t < 2.0 / d1 {
        let t = t - 1.5 / d1;
        n1 * t * t + 0.75
    } else if t < 2.5 / d1 {
        let t = t - 2.25 / d1;
        n1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / d1;
        n1 * t * t + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interp_mode_def_parses_strings_and_objects() {
        let v: InterpModeDef = serde_json::from_str("\"ease_in_out\"").unwrap();
        assert_eq!(v, InterpModeDef::EaseInOut);
        let v: InterpModeDef =
            serde_json::from_str("{\"cubic_bezier\": [0.25, 0.1, 0.25, 1.0]}").unwrap();
        assert!(matches!(v, InterpModeDef::CubicBezier { .. }));
    }

    #[test]
    fn cubic_bezier_endpoints() {
        let y0 = cubic_bezier_ease(0.0, 0.25, 0.1, 0.25, 1.0);
        let y1 = cubic_bezier_ease(1.0, 0.25, 0.1, 0.25, 1.0);
        assert!((y0 - 0.0).abs() < 1e-9);
        assert!((y1 - 1.0).abs() < 1e-9);
    }
}
