#[cfg(feature = "gpu")]
mod gpu_blur {
    use wavyte::{
        Asset, AssetCache, AssetId, BackendKind, BlendMode, Canvas, CompositeOp, CompositePass,
        DrawOp, Pass, PassFx, PixelFormat, RenderPlan, RenderSettings, ScenePass, SurfaceDesc,
        SurfaceId, WavyteError, WavyteResult, create_backend,
    };

    struct NoAssets;
    impl AssetCache for NoAssets {
        fn id_for(&mut self, _asset: &Asset) -> WavyteResult<AssetId> {
            Err(WavyteError::evaluation("no assets in this test"))
        }

        fn get_or_load(&mut self, _asset: &Asset) -> WavyteResult<wavyte::PreparedAsset> {
            Err(WavyteError::evaluation("no assets in this test"))
        }

        fn get_or_load_by_id(&mut self, _id: AssetId) -> WavyteResult<wavyte::PreparedAsset> {
            Err(WavyteError::evaluation("no assets in this test"))
        }
    }

    #[test]
    fn gpu_blur_smoke() {
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };
        let mut backend = create_backend(BackendKind::Gpu, &settings).unwrap();
        let mut assets = NoAssets;

        let w = 64u32;
        let h = 64u32;
        let desc = SurfaceDesc {
            width: w,
            height: h,
            format: PixelFormat::Rgba8Premul,
        };

        let path = wavyte::BezPath::from_svg("M16,16 L48,16 L48,48 L16,48 Z").unwrap();
        let scene = Pass::Scene(ScenePass {
            target: SurfaceId(1),
            clear_to_transparent: true,
            ops: vec![DrawOp::FillPath {
                path,
                transform: wavyte::Affine::IDENTITY,
                color: wavyte::Rgba8Premul::from_straight_rgba(255, 255, 255, 255),
                opacity: 1.0,
                blend: BlendMode::Normal,
                z: 0,
            }],
        });

        let plan_base = RenderPlan {
            canvas: Canvas {
                width: w,
                height: h,
            },
            surfaces: vec![desc.clone(), desc.clone(), desc.clone()],
            passes: vec![
                scene.clone(),
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

        let base = match backend.render_plan(&plan_base, &mut assets) {
            Ok(v) => v,
            Err(e) if e.to_string().contains("no gpu adapter available") => return,
            Err(e) => panic!("unexpected gpu render error: {e}"),
        };

        let plan_blur = RenderPlan {
            canvas: Canvas {
                width: w,
                height: h,
            },
            surfaces: vec![desc.clone(), desc.clone(), desc],
            passes: vec![
                scene,
                Pass::Offscreen(wavyte::OffscreenPass {
                    input: SurfaceId(1),
                    output: SurfaceId(2),
                    fx: PassFx::Blur {
                        radius_px: 6,
                        sigma: 3.0,
                    },
                }),
                Pass::Composite(CompositePass {
                    target: SurfaceId(0),
                    ops: vec![CompositeOp::Over {
                        src: SurfaceId(2),
                        opacity: 1.0,
                    }],
                }),
            ],
            final_surface: SurfaceId(0),
        };

        let blurred = match backend.render_plan(&plan_blur, &mut assets) {
            Ok(v) => v,
            Err(e) if e.to_string().contains("no gpu adapter available") => return,
            Err(e) => panic!("unexpected gpu render error: {e}"),
        };

        assert_eq!(base.width, w);
        assert_eq!(base.height, h);
        assert!(base.premultiplied);
        assert!(base.data.iter().any(|&x| x != 0));

        assert_eq!(blurred.width, w);
        assert_eq!(blurred.height, h);
        assert!(blurred.premultiplied);
        assert!(blurred.data.iter().any(|&x| x != 0));

        assert_ne!(base.data, blurred.data);
    }
}
