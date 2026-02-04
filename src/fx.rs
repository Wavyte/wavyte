use crate::{
    core::Affine,
    error::{WavyteError, WavyteResult},
    model::EffectInstance,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpacityMul { value: f32 },
    TransformPost { value: Affine },
    Blur { radius_px: u32, sigma: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineFx {
    pub opacity_mul: f32,
    pub transform_post: Affine,
}

impl Default for InlineFx {
    fn default() -> Self {
        Self {
            opacity_mul: 1.0,
            transform_post: Affine::IDENTITY,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PassFx {
    Blur { radius_px: u32, sigma: f32 },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FxPipeline {
    pub inline: InlineFx,
    pub passes: Vec<PassFx>,
}

pub fn parse_effect(inst: &EffectInstance) -> WavyteResult<Effect> {
    let kind = inst.kind.trim().to_ascii_lowercase();
    if kind.is_empty() {
        return Err(WavyteError::validation("effect kind must be non-empty"));
    }

    match kind.as_str() {
        "opacitymul" | "opacity_mul" | "opacity-mul" => {
            let value = get_f32(&inst.params, "value")?;
            if !value.is_finite() || value < 0.0 {
                return Err(WavyteError::validation(
                    "OpacityMul.value must be finite and >= 0",
                ));
            }
            Ok(Effect::OpacityMul { value })
        }
        "transformpost" | "transform_post" | "transform-post" => {
            let value = parse_affine(&inst.params)?;
            Ok(Effect::TransformPost { value })
        }
        "blur" => {
            let radius_px = get_u32(&inst.params, "radius_px")?;
            if radius_px > 256 {
                return Err(WavyteError::validation(
                    "Blur.radius_px must be <= 256 in v0.1.0",
                ));
            }
            let sigma = match inst.params.get("sigma") {
                Some(v) => {
                    let s = v
                        .as_f64()
                        .ok_or_else(|| WavyteError::validation("Blur.sigma must be a number"))?
                        as f32;
                    if !s.is_finite() || s <= 0.0 {
                        return Err(WavyteError::validation("Blur.sigma must be finite and > 0"));
                    }
                    s
                }
                None => (radius_px as f32) / 2.0,
            };
            Ok(Effect::Blur { radius_px, sigma })
        }
        _ => Err(WavyteError::validation(format!(
            "unknown effect kind '{kind}'"
        ))),
    }
}

pub fn normalize_effects(effects: &[Effect]) -> FxPipeline {
    let mut inline = InlineFx::default();
    let mut passes = Vec::<PassFx>::new();

    for e in effects {
        match *e {
            Effect::OpacityMul { value } => inline.opacity_mul *= value,
            Effect::TransformPost { value } => inline.transform_post *= value,
            Effect::Blur { radius_px, sigma } => {
                if radius_px == 0 {
                    continue;
                }
                passes.push(PassFx::Blur { radius_px, sigma });
            }
        }
    }

    if !inline.opacity_mul.is_finite() || inline.opacity_mul < 0.0 {
        inline.opacity_mul = 0.0;
    }

    if inline.opacity_mul == 1.0 && inline.transform_post == Affine::IDENTITY && passes.is_empty() {
        FxPipeline::default()
    } else {
        FxPipeline { inline, passes }
    }
}

fn get_u32(obj: &serde_json::Value, key: &str) -> WavyteResult<u32> {
    let Some(v) = obj.get(key) else {
        return Err(WavyteError::validation(format!(
            "missing effect param '{key}'"
        )));
    };
    let Some(n) = v.as_u64() else {
        return Err(WavyteError::validation(format!(
            "effect param '{key}' must be an integer"
        )));
    };
    u32::try_from(n)
        .map_err(|_| WavyteError::validation(format!("effect param '{key}' is out of range")))
}

fn get_f32(obj: &serde_json::Value, key: &str) -> WavyteResult<f32> {
    let Some(v) = obj.get(key) else {
        return Err(WavyteError::validation(format!(
            "missing effect param '{key}'"
        )));
    };
    let Some(n) = v.as_f64() else {
        return Err(WavyteError::validation(format!(
            "effect param '{key}' must be a number"
        )));
    };
    let n = n as f32;
    if !n.is_finite() {
        return Err(WavyteError::validation(format!(
            "effect param '{key}' must be finite"
        )));
    }
    Ok(n)
}

fn parse_affine(params: &serde_json::Value) -> WavyteResult<Affine> {
    if let Some(a) = params.get("affine") {
        let Some(arr) = a.as_array() else {
            return Err(WavyteError::validation(
                "transform_post.affine must be an array",
            ));
        };
        if arr.len() != 6 {
            return Err(WavyteError::validation(
                "transform_post.affine must have length 6",
            ));
        }
        let mut coeffs = [0.0f64; 6];
        for (i, v) in arr.iter().enumerate() {
            coeffs[i] = v.as_f64().ok_or_else(|| {
                WavyteError::validation("transform_post.affine entries must be numbers")
            })?;
        }
        return Ok(Affine::new(coeffs));
    }

    // Structured fallback.
    let t = match params.get("translate") {
        Some(v) => {
            let a = v
                .as_array()
                .ok_or_else(|| WavyteError::validation("transform_post.translate must be [x,y]"))?;
            if a.len() != 2 {
                return Err(WavyteError::validation(
                    "transform_post.translate must be [x,y]",
                ));
            }
            let x = a[0].as_f64().ok_or_else(|| {
                WavyteError::validation("transform_post.translate x must be number")
            })?;
            let y = a[1].as_f64().ok_or_else(|| {
                WavyteError::validation("transform_post.translate y must be number")
            })?;
            Affine::translate((x, y))
        }
        None => Affine::IDENTITY,
    };

    let rot = match (params.get("rotation_rad"), params.get("rotate_deg")) {
        (Some(v), _) => {
            let r = v.as_f64().ok_or_else(|| {
                WavyteError::validation("transform_post.rotation_rad must be number")
            })?;
            Affine::rotate(r)
        }
        (None, Some(v)) => {
            let deg = v.as_f64().ok_or_else(|| {
                WavyteError::validation("transform_post.rotate_deg must be number")
            })?;
            Affine::rotate(deg.to_radians())
        }
        (None, None) => Affine::IDENTITY,
    };

    let scale = match params.get("scale") {
        Some(v) => {
            let a = v
                .as_array()
                .ok_or_else(|| WavyteError::validation("transform_post.scale must be [sx,sy]"))?;
            if a.len() != 2 {
                return Err(WavyteError::validation(
                    "transform_post.scale must be [sx,sy]",
                ));
            }
            let sx = a[0]
                .as_f64()
                .ok_or_else(|| WavyteError::validation("transform_post.scale sx must be number"))?;
            let sy = a[1]
                .as_f64()
                .ok_or_else(|| WavyteError::validation("transform_post.scale sy must be number"))?;
            Affine::scale_non_uniform(sx, sy)
        }
        None => Affine::IDENTITY,
    };

    Ok(t * rot * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(kind: &str, params: serde_json::Value) -> EffectInstance {
        EffectInstance {
            kind: kind.to_string(),
            params,
        }
    }

    #[test]
    fn parse_opacity_mul() {
        let e = parse_effect(&inst("opacity_mul", serde_json::json!({ "value": 0.5 }))).unwrap();
        assert_eq!(e, Effect::OpacityMul { value: 0.5 });
    }

    #[test]
    fn normalize_folds_opacity_and_drops_noop_blur() {
        let fx = vec![
            Effect::OpacityMul { value: 0.5 },
            Effect::OpacityMul { value: 0.25 },
            Effect::Blur {
                radius_px: 0,
                sigma: 1.0,
            },
        ];
        let p = normalize_effects(&fx);
        assert_eq!(p.inline.opacity_mul, 0.125);
        assert!(p.passes.is_empty());
    }
}
