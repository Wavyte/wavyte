use crate::{
    assets::store::PreparedAssetStore,
    compile::{CompositePass, OffscreenPass, Pass, RenderPlan, ScenePass, SurfaceDesc, SurfaceId},
    foundation::error::{WavyteError, WavyteResult},
    render::FrameRGBA,
};

pub trait PassBackend {
    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()>;

    fn exec_scene(&mut self, pass: &ScenePass, assets: &PreparedAssetStore) -> WavyteResult<()>;

    fn exec_offscreen(
        &mut self,
        pass: &OffscreenPass,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<()>;

    fn exec_composite(
        &mut self,
        pass: &CompositePass,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<()>;

    fn readback_rgba8(
        &mut self,
        surface: SurfaceId,
        plan: &RenderPlan,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<FrameRGBA>;
}

pub fn execute_plan<B: PassBackend + ?Sized>(
    backend: &mut B,
    plan: &RenderPlan,
    assets: &PreparedAssetStore,
) -> WavyteResult<FrameRGBA> {
    for (idx, desc) in plan.surfaces.iter().enumerate() {
        let id = SurfaceId(
            idx.try_into()
                .map_err(|_| WavyteError::evaluation("surface id overflow"))?,
        );
        backend.ensure_surface(id, desc)?;
    }

    for pass in &plan.passes {
        match pass {
            Pass::Scene(p) => backend.exec_scene(p, assets)?,
            Pass::Offscreen(p) => backend.exec_offscreen(p, assets)?,
            Pass::Composite(p) => backend.exec_composite(p, assets)?,
        }
    }

    backend.readback_rgba8(plan.final_surface, plan, assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::store::PreparedAssetStore,
        compile::{CompositeOp, PixelFormat},
        foundation::core::{Canvas, Rgba8Premul},
        foundation::error::WavyteResult,
    };

    #[derive(Default)]
    struct MockBackend {
        calls: Vec<&'static str>,
    }

    impl PassBackend for MockBackend {
        fn ensure_surface(&mut self, _id: SurfaceId, _desc: &SurfaceDesc) -> WavyteResult<()> {
            self.calls.push("ensure_surface");
            Ok(())
        }

        fn exec_scene(
            &mut self,
            _pass: &ScenePass,
            _assets: &PreparedAssetStore,
        ) -> WavyteResult<()> {
            self.calls.push("exec_scene");
            Ok(())
        }

        fn exec_offscreen(
            &mut self,
            _pass: &OffscreenPass,
            _assets: &PreparedAssetStore,
        ) -> WavyteResult<()> {
            self.calls.push("exec_offscreen");
            Ok(())
        }

        fn exec_composite(
            &mut self,
            _pass: &CompositePass,
            _assets: &PreparedAssetStore,
        ) -> WavyteResult<()> {
            self.calls.push("exec_composite");
            Ok(())
        }

        fn readback_rgba8(
            &mut self,
            _surface: SurfaceId,
            plan: &RenderPlan,
            _assets: &PreparedAssetStore,
        ) -> WavyteResult<FrameRGBA> {
            self.calls.push("readback_rgba8");
            Ok(FrameRGBA {
                width: plan.canvas.width,
                height: plan.canvas.height,
                data: vec![0; (plan.canvas.width * plan.canvas.height * 4) as usize],
                premultiplied: true,
            })
        }
    }

    #[test]
    fn execute_plan_calls_in_expected_order() {
        let plan = RenderPlan {
            canvas: Canvas {
                width: 4,
                height: 3,
            },
            surfaces: vec![
                SurfaceDesc {
                    width: 4,
                    height: 3,
                    format: PixelFormat::Rgba8Premul,
                },
                SurfaceDesc {
                    width: 4,
                    height: 3,
                    format: PixelFormat::Rgba8Premul,
                },
            ],
            passes: vec![
                Pass::Scene(ScenePass {
                    target: SurfaceId(1),
                    ops: vec![],
                    clear_to_transparent: true,
                }),
                Pass::Offscreen(OffscreenPass {
                    input: SurfaceId(1),
                    output: SurfaceId(1),
                    fx: crate::effects::fx::PassFx::Blur {
                        radius_px: 0,
                        sigma: 1.0,
                    },
                }),
                Pass::Composite(CompositePass {
                    target: SurfaceId(0),
                    ops: vec![CompositeOp::Over {
                        src: SurfaceId(1),
                        opacity: 1.0,
                    }],
                }),
            ],
            final_surface: SurfaceId(0),
        };

        let mut backend = MockBackend::default();
        let comp = crate::Composition {
            fps: crate::Fps::new(30, 1).unwrap(),
            canvas: crate::Canvas {
                width: 1,
                height: 1,
            },
            duration: crate::FrameIndex(1),
            assets: std::collections::BTreeMap::new(),
            tracks: vec![],
            seed: 0,
        };
        let store = PreparedAssetStore::prepare(&comp, ".").unwrap();
        let out = execute_plan(&mut backend, &plan, &store).unwrap();
        assert_eq!(out.width, 4);
        assert_eq!(out.height, 3);
        assert!(out.premultiplied);
        assert_eq!(
            backend.calls,
            vec![
                "ensure_surface",
                "ensure_surface",
                "exec_scene",
                "exec_offscreen",
                "exec_composite",
                "readback_rgba8",
            ]
        );
    }

    #[test]
    fn execute_plan_returns_final_frame() {
        let plan = RenderPlan {
            canvas: Canvas {
                width: 2,
                height: 2,
            },
            surfaces: vec![SurfaceDesc {
                width: 2,
                height: 2,
                format: PixelFormat::Rgba8Premul,
            }],
            passes: vec![Pass::Scene(ScenePass {
                target: SurfaceId(0),
                ops: vec![crate::compile::DrawOp::FillPath {
                    path: crate::foundation::core::BezPath::new(),
                    transform: crate::foundation::core::Affine::IDENTITY,
                    color: Rgba8Premul::from_straight_rgba(0, 0, 0, 0),
                    opacity: 1.0,
                    blend: crate::composition::model::BlendMode::Normal,
                    z: 0,
                }],
                clear_to_transparent: true,
            })],
            final_surface: SurfaceId(0),
        };

        let mut backend = MockBackend::default();
        let comp = crate::Composition {
            fps: crate::Fps::new(30, 1).unwrap(),
            canvas: crate::Canvas {
                width: 1,
                height: 1,
            },
            duration: crate::FrameIndex(1),
            assets: std::collections::BTreeMap::new(),
            tracks: vec![],
            seed: 0,
        };
        let store = PreparedAssetStore::prepare(&comp, ".").unwrap();
        let out = execute_plan(&mut backend, &plan, &store).unwrap();
        assert_eq!(out.data.len(), 16);
    }
}
