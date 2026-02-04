use crate::error::{WavyteError, WavyteResult};

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

    pub fn sample(&self, _ctx: crate::anim::SampleCtx) -> WavyteResult<T> {
        Err(WavyteError::animation(
            "procedural sampling not implemented yet",
        ))
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
