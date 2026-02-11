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
