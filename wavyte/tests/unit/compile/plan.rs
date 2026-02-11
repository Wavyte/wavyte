use std::collections::BTreeMap;

use super::*;
use crate::{
    animation::anim::Anim,
    animation::ease::Ease,
    assets::store::PreparedAssetStore,
    composition::model::{
        Asset, BlendMode, Clip, ClipProps, EffectInstance, PathAsset, Track, TransitionSpec,
    },
    eval::evaluator::Evaluator,
    foundation::core::{Fps, FrameIndex, FrameRange, Transform2D},
};

#[test]
fn parse_effect_cache_key_is_order_insensitive_for_object_params() {
    let mut cache = CompileCache::default();
    let a = crate::eval::evaluator::ResolvedEffect {
        kind: "blur".to_string(),
        params: serde_json::json!({ "radius_px": 3, "sigma": 2.0 }),
    };
    let b = crate::eval::evaluator::ResolvedEffect {
        kind: "blur".to_string(),
        params: serde_json::json!({ "sigma": 2.0, "radius_px": 3 }),
    };

    let pa = parse_effect_cached(&mut cache, &a).unwrap();
    let pb = parse_effect_cached(&mut cache, &b).unwrap();
    assert_eq!(pa, pb);
    assert_eq!(cache.effect_cache.len(), 1);
    assert_eq!(cache.effect_cache.values().next().unwrap().len(), 1);
}

#[test]
fn parse_transition_cache_key_is_order_insensitive_for_object_params() {
    let mut cache = CompileCache::default();
    let a = crate::eval::evaluator::ResolvedTransition {
        kind: "wipe".to_string(),
        progress: 0.5,
        params: serde_json::json!({ "dir": "ttb", "soft_edge": 0.2 }),
    };
    let b = crate::eval::evaluator::ResolvedTransition {
        kind: "wipe".to_string(),
        progress: 0.5,
        params: serde_json::json!({ "soft_edge": 0.2, "dir": "ttb" }),
    };

    let pa = parse_transition_cached(&mut cache, &a).unwrap();
    let pb = parse_transition_cached(&mut cache, &b).unwrap();
    assert_eq!(pa, pb);
    assert_eq!(cache.transition_cache.len(), 1);
    assert_eq!(cache.transition_cache.values().next().unwrap().len(), 1);
}

fn store_for(comp: &Composition) -> PreparedAssetStore {
    PreparedAssetStore::prepare(comp, ".").expect("prepare asset store for test composition")
}

#[test]
fn compile_path_emits_fillpath_without_asset_cache() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(10),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "p0".to_string(),
                range: FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D::default()),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![],
                transition_in: Some(TransitionSpec {
                    kind: "fade".to_string(),
                    duration_frames: 2,
                    ease: crate::Ease::Linear,
                    params: serde_json::Value::Null,
                }),
                transition_out: None,
            }],
        }],
        seed: 1,
    };

    let eval = Evaluator::eval_frame(&comp, FrameIndex(1)).unwrap();
    let store = store_for(&comp);
    let plan = compile_frame(&comp, &eval, &store).unwrap();
    let Pass::Scene(scene) = &plan.passes[0] else {
        panic!("expected Scene pass");
    };
    assert_eq!(scene.ops.len(), 1);
    match &scene.ops[0] {
        DrawOp::FillPath { opacity, .. } => {
            assert_eq!(*opacity, 1.0);
        }
        _ => panic!("expected FillPath"),
    }
}

#[test]
fn compile_applies_inline_effects_to_opacity_and_transform() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(10),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "p0".to_string(),
                range: FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D::default()),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![
                    EffectInstance {
                        kind: "opacity_mul".to_string(),
                        params: serde_json::json!({ "value": 0.5 }),
                    },
                    EffectInstance {
                        kind: "transform_post".to_string(),
                        params: serde_json::json!({ "translate": [3.0, 4.0] }),
                    },
                ],
                transition_in: None,
                transition_out: None,
            }],
        }],
        seed: 1,
    };

    let eval = Evaluator::eval_frame(&comp, FrameIndex(0)).unwrap();
    let store = store_for(&comp);
    let plan = compile_frame(&comp, &eval, &store).unwrap();
    let Pass::Scene(scene) = &plan.passes[0] else {
        panic!("expected Scene pass");
    };
    let DrawOp::FillPath {
        transform, opacity, ..
    } = &scene.ops[0]
    else {
        panic!("expected FillPath");
    };
    assert_eq!(*opacity, 0.5);

    let coeffs = transform.as_coeffs();
    assert_eq!(coeffs[4], 3.0);
    assert_eq!(coeffs[5], 4.0);
}

#[test]
fn compile_emits_offscreen_blur_pass_and_composites_blurred_surface() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(10),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "p0".to_string(),
                range: FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D::default()),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![EffectInstance {
                    kind: "blur".to_string(),
                    params: serde_json::json!({ "radius_px": 3, "sigma": 2.0 }),
                }],
                transition_in: None,
                transition_out: None,
            }],
        }],
        seed: 1,
    };

    let eval = Evaluator::eval_frame(&comp, FrameIndex(0)).unwrap();
    let store = store_for(&comp);
    let plan = compile_frame(&comp, &eval, &store).unwrap();

    assert_eq!(plan.surfaces.len(), 3);
    assert_eq!(plan.final_surface, SurfaceId(0));

    match &plan.passes[0] {
        Pass::Scene(s) => assert_eq!(s.target, SurfaceId(1)),
        _ => panic!("expected Scene pass"),
    }

    match &plan.passes[1] {
        Pass::Offscreen(p) => {
            assert_eq!(p.input, SurfaceId(1));
            assert_eq!(p.output, SurfaceId(2));
            assert_eq!(
                p.fx,
                crate::effects::fx::PassFx::Blur {
                    radius_px: 3,
                    sigma: 2.0
                }
            );
        }
        _ => panic!("expected Offscreen pass"),
    }

    match &plan.passes[2] {
        Pass::Composite(p) => {
            assert_eq!(p.target, SurfaceId(0));
            assert_eq!(p.ops.len(), 1);
            match p.ops[0] {
                CompositeOp::Over { src, opacity } => {
                    assert_eq!(src, SurfaceId(2));
                    assert_eq!(opacity, 1.0);
                }
                _ => panic!("expected Over composite op"),
            }
        }
        _ => panic!("expected Composite pass"),
    }
}

#[test]
fn compile_pairs_crossfade_into_single_composite_op() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );

    let tr = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 3,
        ease: Ease::Linear,
        params: serde_json::Value::Null,
    };

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(20),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![
                Clip {
                    id: "a".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: Some(tr.clone()),
                },
                Clip {
                    id: "b".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(7), FrameIndex(17)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 1,
                    effects: vec![],
                    transition_in: Some(tr),
                    transition_out: None,
                },
            ],
        }],
        seed: 1,
    };

    let eval = Evaluator::eval_frame(&comp, FrameIndex(8)).unwrap();
    let store = store_for(&comp);
    let plan = compile_frame(&comp, &eval, &store).unwrap();
    let Pass::Composite(p) = plan.passes.last().unwrap() else {
        panic!("expected Composite pass");
    };
    assert_eq!(p.ops.len(), 1);

    match &p.ops[0] {
        CompositeOp::Crossfade { a, b, t } => {
            assert_eq!(*a, SurfaceId(1));
            assert_eq!(*b, SurfaceId(2));
            assert!((*t - 0.5).abs() <= 1e-6);
        }
        other => panic!("expected Crossfade op, got {other:?}"),
    }
}

#[test]
fn compile_pairs_wipe_into_single_composite_op() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );

    let tr = TransitionSpec {
        kind: "wipe".to_string(),
        duration_frames: 3,
        ease: Ease::Linear,
        params: serde_json::json!({ "dir": "ttb", "soft_edge": 0.2 }),
    };

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(20),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![
                Clip {
                    id: "a".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: Some(tr.clone()),
                },
                Clip {
                    id: "b".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(7), FrameIndex(17)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 1,
                    effects: vec![],
                    transition_in: Some(tr),
                    transition_out: None,
                },
            ],
        }],
        seed: 1,
    };

    let eval = Evaluator::eval_frame(&comp, FrameIndex(8)).unwrap();
    let store = store_for(&comp);
    let plan = compile_frame(&comp, &eval, &store).unwrap();
    let Pass::Composite(p) = plan.passes.last().unwrap() else {
        panic!("expected Composite pass");
    };
    assert_eq!(p.ops.len(), 1);

    match &p.ops[0] {
        CompositeOp::Wipe {
            a,
            b,
            t,
            dir,
            soft_edge,
        } => {
            assert_eq!(*a, SurfaceId(1));
            assert_eq!(*b, SurfaceId(2));
            assert!((*t - 0.5).abs() <= 1e-6);
            assert_eq!(*dir, WipeDir::TopToBottom);
            assert!((*soft_edge - 0.2).abs() <= 1e-6);
        }
        other => panic!("expected Wipe op, got {other:?}"),
    }
}

#[test]
fn compile_does_not_pair_transitions_when_progress_is_not_aligned() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );

    let out_tr = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 4,
        ease: Ease::Linear,
        params: serde_json::Value::Null,
    };
    let in_tr = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 3,
        ease: Ease::Linear,
        params: serde_json::Value::Null,
    };

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(20),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![
                Clip {
                    id: "a".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: Some(out_tr),
                },
                Clip {
                    id: "b".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(7), FrameIndex(17)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 1,
                    effects: vec![],
                    transition_in: Some(in_tr),
                    transition_out: None,
                },
            ],
        }],
        seed: 1,
    };

    let eval = Evaluator::eval_frame(&comp, FrameIndex(8)).unwrap();
    let store = store_for(&comp);
    let plan = compile_frame(&comp, &eval, &store).unwrap();
    let Pass::Composite(p) = plan.passes.last().unwrap() else {
        panic!("expected Composite pass");
    };
    assert_eq!(p.ops.len(), 2);

    let CompositeOp::Over {
        src: src0,
        opacity: op0,
    } = p.ops[0]
    else {
        panic!("expected Over op 0");
    };
    let CompositeOp::Over {
        src: src1,
        opacity: op1,
    } = p.ops[1]
    else {
        panic!("expected Over op 1");
    };

    assert_eq!(src0, SurfaceId(1));
    assert_eq!(src1, SurfaceId(2));
    assert!((op0 - (1.0 / 3.0)).abs() <= 0.02);
    assert!((op1 - 0.5).abs() <= 1e-6);
}
