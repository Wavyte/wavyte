use crate::foundation::core::{Rgba8Premul, Vec2};
use crate::foundation::ids::{EffectKindId, ParamId};
use serde_json::Value as JsonValue;
use smallvec::SmallVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreEffectKindIR {
    Blur,
    ColorMatrix,
    DropShadow,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreParamKeyIR {
    Value,
    RadiusPx,
    Sigma,
    Matrix,
    Offset,
    Color,
    BlurRadiusPx,
    Unknown,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectBindingIR {
    pub(crate) kind: EffectKindId,
    pub(crate) core_kind: CoreEffectKindIR,
    pub(crate) params: SmallVec<[ResolvedParamIR; 8]>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedParamIR {
    pub(crate) id: ParamId,
    pub(crate) key: CoreParamKeyIR,
    pub(crate) value: ResolvedParamValueIR,
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvedParamValueIR {
    F64(f64),
    Vec2(Vec2),
    Color(Rgba8Premul),
    Bool(bool),
    String(String),
    Matrix20([f32; 20]),
    Json(JsonValue),
}
