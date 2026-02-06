use crate::{
    asset_store::{AssetId, PreparedAsset, PreparedAssetStore},
    core::{Affine, BezPath, Canvas, Rgba8Premul},
    error::WavyteResult,
    eval::EvaluatedGraph,
    fx::{PassFx, normalize_effects, parse_effect},
    model::{BlendMode, Composition, EffectInstance},
    transitions::{TransitionKind, WipeDir, parse_transition_kind_params},
};

#[derive(Clone, Debug)]
/// Backend-agnostic render plan for a single frame.
///
/// A plan consists of:
/// - surface declarations (`surfaces`)
/// - a sequence of passes (`passes`)
/// - a declared final surface (`final_surface`)
///
/// The plan is designed to be executable by multiple backends (CPU and GPU) with the same
/// semantics.
pub struct RenderPlan {
    pub canvas: Canvas,
    pub surfaces: Vec<SurfaceDesc>,
    pub passes: Vec<Pass>,
    pub final_surface: SurfaceId,
}

#[derive(Clone, Debug)]
/// A single pass in a [`RenderPlan`].
pub enum Pass {
    Scene(ScenePass),
    Offscreen(OffscreenPass),
    Composite(CompositePass),
}

#[derive(Clone, Debug)]
/// Draw operations into a surface.
pub struct ScenePass {
    pub target: SurfaceId,
    pub ops: Vec<DrawOp>,
    pub clear_to_transparent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Identifier for a render surface declared in [`RenderPlan::surfaces`].
pub struct SurfaceId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Supported pixel formats for render surfaces.
pub enum PixelFormat {
    Rgba8Premul,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Surface declaration: dimensions + pixel format.
pub struct SurfaceDesc {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

#[derive(Clone, Debug)]
/// Run a post-processing effect producing a new surface from an input surface.
pub struct OffscreenPass {
    pub input: SurfaceId,
    pub output: SurfaceId,
    pub fx: PassFx,
}

#[derive(Clone, Debug)]
/// Composite multiple surfaces into a target surface.
pub struct CompositePass {
    pub target: SurfaceId,
    pub ops: Vec<CompositeOp>,
}

#[derive(Clone, Debug)]
/// A compositing operation between surfaces.
pub enum CompositeOp {
    Over {
        src: SurfaceId,
        opacity: f32,
    },
    Crossfade {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
    },
    Wipe {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
        dir: WipeDir,
        soft_edge: f32,
    },
}

#[derive(Clone, Debug)]
/// Draw operation emitted by the compiler.
pub enum DrawOp {
    FillPath {
        path: BezPath,
        transform: Affine,
        color: Rgba8Premul,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Image {
        asset: AssetId,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Svg {
        asset: AssetId,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Text {
        asset: AssetId,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Video {
        asset: AssetId,
        source_time_s: f64,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
}

pub fn compile_frame(
    comp: &Composition,
    eval: &EvaluatedGraph,
    assets: &PreparedAssetStore,
) -> WavyteResult<RenderPlan> {
    #[derive(Clone, Debug)]
    struct Layer {
        surface: SurfaceId,
        transition_in: Option<crate::eval::ResolvedTransition>,
        transition_out: Option<crate::eval::ResolvedTransition>,
    }

    let mut surfaces = Vec::<SurfaceDesc>::new();
    surfaces.push(SurfaceDesc {
        width: comp.canvas.width,
        height: comp.canvas.height,
        format: PixelFormat::Rgba8Premul,
    });

    let mut scene_passes = Vec::<Pass>::with_capacity(eval.nodes.len());
    let mut layers = Vec::<Layer>::with_capacity(eval.nodes.len());

    for (idx, node) in eval.nodes.iter().enumerate() {
        let mut parsed = Vec::with_capacity(node.effects.len());
        for e in &node.effects {
            let inst = EffectInstance {
                kind: e.kind.clone(),
                params: e.params.clone(),
            };
            parsed.push(parse_effect(&inst)?);
        }
        let fx = normalize_effects(&parsed);

        // Transitions are handled during composition. Keep DrawOp opacity for "intrinsic" opacity
        // only (clip opacity + inline opacity effect).
        let opacity = ((node.opacity as f32) * fx.inline.opacity_mul).clamp(0.0, 1.0);

        if opacity <= 0.0 {
            continue;
        }

        let transform = node.transform * fx.inline.transform_post;

        let asset_id = assets.id_for_key(&node.asset)?;
        let op = match assets.get(asset_id)? {
            PreparedAsset::Path(a) => DrawOp::FillPath {
                path: a.path.clone(),
                transform,
                color: Rgba8Premul::from_straight_rgba(255, 255, 255, 255),
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Image(_) => DrawOp::Image {
                asset: asset_id,
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Svg(_) => DrawOp::Svg {
                asset: asset_id,
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Text(_) => DrawOp::Text {
                asset: asset_id,
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Video(_) => DrawOp::Video {
                asset: asset_id,
                source_time_s: node.source_time_s.unwrap_or(0.0),
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Audio(_) => continue,
        };

        let surf_id = SurfaceId((surfaces.len()) as u32);
        surfaces.push(SurfaceDesc {
            width: comp.canvas.width,
            height: comp.canvas.height,
            format: PixelFormat::Rgba8Premul,
        });

        scene_passes.push(Pass::Scene(ScenePass {
            target: surf_id,
            ops: vec![op],
            clear_to_transparent: true,
        }));

        let mut post_fx = surf_id;
        for fx in &fx.passes {
            let out_id = SurfaceId((surfaces.len()) as u32);
            surfaces.push(SurfaceDesc {
                width: comp.canvas.width,
                height: comp.canvas.height,
                format: PixelFormat::Rgba8Premul,
            });
            scene_passes.push(Pass::Offscreen(OffscreenPass {
                input: post_fx,
                output: out_id,
                fx: fx.clone(),
            }));
            post_fx = out_id;
        }

        let _ = idx;
        layers.push(Layer {
            surface: post_fx,
            transition_in: node.transition_in.clone(),
            transition_out: node.transition_out.clone(),
        });
    }

    let mut composite_ops = Vec::<CompositeOp>::with_capacity(layers.len());
    let mut i = 0usize;
    while i < layers.len() {
        let layer = &layers[i];

        let mut paired = false;
        if i + 1 < layers.len() {
            let next = &layers[i + 1];

            if let (Some(out_tr), Some(in_tr)) =
                (layer.transition_out.as_ref(), next.transition_in.as_ref())
            {
                let out_kind = parse_transition_kind_params(&out_tr.kind, &out_tr.params).ok();
                let in_kind = parse_transition_kind_params(&in_tr.kind, &in_tr.params).ok();

                if let (Some(out_kind), Some(in_kind)) = (out_kind, in_kind) {
                    let t_in = (in_tr.progress as f32).clamp(0.0, 1.0);
                    let t_out = (out_tr.progress as f32).clamp(0.0, 1.0);

                    // Explicit v0.2 pairing rule: the Out and In edges must agree on progress
                    // (same duration/ease and overlapping window).
                    let progress_close = (t_in - t_out).abs() <= 0.05;

                    if progress_close {
                        match (out_kind, in_kind) {
                            (TransitionKind::Crossfade, TransitionKind::Crossfade) => {
                                composite_ops.push(CompositeOp::Crossfade {
                                    a: layer.surface,
                                    b: next.surface,
                                    t: t_in,
                                });
                                paired = true;
                            }
                            (
                                TransitionKind::Wipe {
                                    dir: dir_a,
                                    soft_edge: soft_a,
                                },
                                TransitionKind::Wipe {
                                    dir: dir_b,
                                    soft_edge: soft_b,
                                },
                            ) => {
                                if dir_a == dir_b && (soft_a - soft_b).abs() <= 1e-6 {
                                    composite_ops.push(CompositeOp::Wipe {
                                        a: layer.surface,
                                        b: next.surface,
                                        t: t_in,
                                        dir: dir_a,
                                        soft_edge: soft_a,
                                    });
                                    paired = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if paired {
            i += 2;
            continue;
        }

        let mut layer_opacity = 1.0f32;
        if let Some(tr) = &layer.transition_in {
            layer_opacity *= tr.progress as f32;
        }
        if let Some(tr) = &layer.transition_out {
            layer_opacity *= (1.0 - tr.progress) as f32;
        }
        layer_opacity = layer_opacity.clamp(0.0, 1.0);

        if layer_opacity > 0.0 {
            composite_ops.push(CompositeOp::Over {
                src: layer.surface,
                opacity: layer_opacity,
            });
        }

        i += 1;
    }

    Ok(RenderPlan {
        canvas: comp.canvas,
        surfaces,
        passes: {
            let mut out = scene_passes;
            out.push(Pass::Composite(CompositePass {
                target: SurfaceId(0),
                ops: composite_ops,
            }));
            out
        },
        final_surface: SurfaceId(0),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        anim::Anim,
        anim_ease::Ease,
        asset_store::PreparedAssetStore,
        core::{Fps, FrameIndex, FrameRange, Transform2D},
        eval::Evaluator,
        model::{
            Asset, BlendMode, Clip, ClipProps, EffectInstance, PathAsset, Track, TransitionSpec,
        },
    };

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
                    crate::fx::PassFx::Blur {
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
}
