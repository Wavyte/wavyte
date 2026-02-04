use crate::{
    assets::{AssetCache, AssetId},
    core::{Affine, BezPath, Canvas, Rgba8Premul},
    error::{WavyteError, WavyteResult},
    eval::EvaluatedGraph,
    model::{Asset, BlendMode, Composition},
};

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub canvas: Canvas,
    pub passes: Vec<Pass>,
}

#[derive(Clone, Debug)]
pub enum Pass {
    Scene(ScenePass),
}

#[derive(Clone, Debug)]
pub struct ScenePass {
    pub ops: Vec<DrawOp>,
    pub clear: Option<Rgba8Premul>,
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
    let mut ops = Vec::<DrawOp>::with_capacity(eval.nodes.len());

    for node in &eval.nodes {
        let Some(asset) = comp.assets.get(&node.asset) else {
            return Err(WavyteError::evaluation(format!(
                "evaluated node '{}' references missing asset key '{}'",
                node.clip_id, node.asset
            )));
        };

        let mut opacity = node.opacity as f32;
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

        match asset {
            Asset::Path(a) => {
                let path = parse_svg_path(&a.svg_path_d)?;
                ops.push(DrawOp::FillPath {
                    path,
                    transform: node.transform,
                    color: Rgba8Premul::from_straight_rgba(255, 255, 255, 255),
                    opacity,
                    blend: node.blend,
                    z: node.z,
                });
            }
            Asset::Image(_) => {
                let id = assets.id_for(asset)?;
                ops.push(DrawOp::Image {
                    asset: id,
                    transform: node.transform,
                    opacity,
                    blend: node.blend,
                    z: node.z,
                });
            }
            Asset::Svg(_) => {
                let id = assets.id_for(asset)?;
                ops.push(DrawOp::Svg {
                    asset: id,
                    transform: node.transform,
                    opacity,
                    blend: node.blend,
                    z: node.z,
                });
            }
            Asset::Text(_) => {
                let id = assets.id_for(asset)?;
                ops.push(DrawOp::Text {
                    asset: id,
                    transform: node.transform,
                    opacity,
                    blend: node.blend,
                    z: node.z,
                });
            }
            Asset::Video(_) | Asset::Audio(_) => {
                return Err(WavyteError::evaluation(
                    "video/audio rendering is not supported in v0.1.0 phase 4",
                ));
            }
        }
    }

    Ok(RenderPlan {
        canvas: comp.canvas,
        passes: vec![Pass::Scene(ScenePass { ops, clear: None })],
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
        model::{Asset, BlendMode, Clip, ClipProps, PathAsset, Track, TransitionSpec},
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
                    }),
                    transition_out: None,
                }],
            }],
            seed: 1,
        };

        let eval = Evaluator::eval_frame(&comp, FrameIndex(1)).unwrap();
        let plan = compile_frame(&comp, &eval, &mut NoAssets).unwrap();
        let Pass::Scene(scene) = &plan.passes[0];
        assert_eq!(scene.ops.len(), 1);
        match &scene.ops[0] {
            DrawOp::FillPath { opacity, .. } => {
                assert_eq!(*opacity, 1.0);
            }
            _ => panic!("expected FillPath"),
        }
    }
}
