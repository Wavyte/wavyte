use crate::{
    assets::{AssetCache, AssetId},
    core::{Affine, BezPath, Canvas, Rgba8Premul},
    error::{WavyteError, WavyteResult},
    eval::EvaluatedGraph,
    fx::{PassFx, normalize_effects, parse_effect},
    model::{Asset, BlendMode, Composition, EffectInstance},
    transitions::WipeDir,
};

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub canvas: Canvas,
    pub surfaces: Vec<SurfaceDesc>,
    pub passes: Vec<Pass>,
    pub final_surface: SurfaceId,
}

#[derive(Clone, Debug)]
pub enum Pass {
    Scene(ScenePass),
    Offscreen(OffscreenPass),
    Composite(CompositePass),
}

#[derive(Clone, Debug)]
pub struct ScenePass {
    pub target: SurfaceId,
    pub ops: Vec<DrawOp>,
    pub clear_to_transparent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8Premul,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceDesc {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

#[derive(Clone, Debug)]
pub struct OffscreenPass {
    pub input: SurfaceId,
    pub output: SurfaceId,
    pub fx: PassFx,
}

#[derive(Clone, Debug)]
pub struct CompositePass {
    pub target: SurfaceId,
    pub ops: Vec<CompositeOp>,
}

#[derive(Clone, Debug)]
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
}

pub fn compile_frame(
    comp: &Composition,
    eval: &EvaluatedGraph,
    assets: &mut dyn AssetCache,
) -> WavyteResult<RenderPlan> {
    let mut surfaces = Vec::<SurfaceDesc>::new();
    surfaces.push(SurfaceDesc {
        width: comp.canvas.width,
        height: comp.canvas.height,
        format: PixelFormat::Rgba8Premul,
    });

    let mut scene_passes = Vec::<Pass>::with_capacity(eval.nodes.len());
    let mut composite_ops = Vec::<CompositeOp>::with_capacity(eval.nodes.len());

    for (idx, node) in eval.nodes.iter().enumerate() {
        let Some(asset) = comp.assets.get(&node.asset) else {
            return Err(WavyteError::evaluation(format!(
                "evaluated node '{}' references missing asset key '{}'",
                node.clip_id, node.asset
            )));
        };

        let mut parsed = Vec::with_capacity(node.effects.len());
        for e in &node.effects {
            let inst = EffectInstance {
                kind: e.kind.clone(),
                params: e.params.clone(),
            };
            parsed.push(parse_effect(&inst)?);
        }
        let fx = normalize_effects(&parsed);

        let mut opacity = (node.opacity as f32) * fx.inline.opacity_mul;
        if let Some(tr) = &node.transition_in {
            opacity *= tr.progress as f32;
        }
        if let Some(tr) = &node.transition_out {
            opacity *= (1.0 - tr.progress) as f32;
        }
        opacity = opacity.clamp(0.0, 1.0);

        if opacity <= 0.0 {
            continue;
        }

        let transform = node.transform * fx.inline.transform_post;

        let op = match asset {
            Asset::Path(a) => {
                let path = parse_svg_path(&a.svg_path_d)?;
                DrawOp::FillPath {
                    path,
                    transform,
                    color: Rgba8Premul::from_straight_rgba(255, 255, 255, 255),
                    opacity,
                    blend: node.blend,
                    z: node.z,
                }
            }
            Asset::Image(_) => {
                let id = assets.id_for(asset)?;
                DrawOp::Image {
                    asset: id,
                    transform,
                    opacity,
                    blend: node.blend,
                    z: node.z,
                }
            }
            Asset::Svg(_) => {
                let id = assets.id_for(asset)?;
                DrawOp::Svg {
                    asset: id,
                    transform,
                    opacity,
                    blend: node.blend,
                    z: node.z,
                }
            }
            Asset::Text(_) => {
                let id = assets.id_for(asset)?;
                DrawOp::Text {
                    asset: id,
                    transform,
                    opacity,
                    blend: node.blend,
                    z: node.z,
                }
            }
            Asset::Video(_) | Asset::Audio(_) => {
                return Err(WavyteError::evaluation(
                    "video/audio rendering is not supported in v0.1.0 phase 4",
                ));
            }
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
        composite_ops.push(CompositeOp::Over {
            src: post_fx,
            opacity: 1.0,
        });
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

fn parse_svg_path(d: &str) -> WavyteResult<BezPath> {
    let d = d.trim();
    if d.is_empty() {
        return Err(WavyteError::validation(
            "path asset svg_path_d must be non-empty",
        ));
    }

    BezPath::from_svg(d).map_err(|e| WavyteError::validation(format!("invalid svg_path_d: {e}")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        anim::Anim,
        core::{Fps, FrameIndex, FrameRange, Transform2D},
        eval::Evaluator,
        model::{
            Asset, BlendMode, Clip, ClipProps, EffectInstance, PathAsset, Track, TransitionSpec,
        },
    };

    struct NoAssets;
    impl AssetCache for NoAssets {
        fn id_for(&mut self, _asset: &Asset) -> WavyteResult<AssetId> {
            Err(WavyteError::evaluation("no assets in this test"))
        }

        fn get_or_load(&mut self, _asset: &Asset) -> WavyteResult<crate::assets::PreparedAsset> {
            Err(WavyteError::evaluation("no assets in this test"))
        }

        fn get_or_load_by_id(
            &mut self,
            _id: AssetId,
        ) -> WavyteResult<crate::assets::PreparedAsset> {
            Err(WavyteError::evaluation("no assets in this test"))
        }
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
        let plan = compile_frame(&comp, &eval, &mut NoAssets).unwrap();
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
        let plan = compile_frame(&comp, &eval, &mut NoAssets).unwrap();
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
        let plan = compile_frame(&comp, &eval, &mut NoAssets).unwrap();

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
}
