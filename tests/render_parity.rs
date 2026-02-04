#[cfg(feature = "gpu")]
mod parity {
    use std::collections::BTreeMap;

    use wavyte::{
        Anim, Asset, AssetCache, AssetId, BackendKind, BlendMode, Canvas, Clip, ClipProps,
        Composition, FrameIndex, FrameRange, PathAsset, RenderSettings, Track, Transform2D,
        create_backend, render_frame,
    };

    struct NoAssets;
    impl AssetCache for NoAssets {
        fn id_for(&mut self, _asset: &Asset) -> wavyte::WavyteResult<AssetId> {
            Err(wavyte::WavyteError::evaluation("no assets in this test"))
        }

        fn get_or_load(&mut self, _asset: &Asset) -> wavyte::WavyteResult<wavyte::PreparedAsset> {
            Err(wavyte::WavyteError::evaluation("no assets in this test"))
        }

        fn get_or_load_by_id(
            &mut self,
            _id: AssetId,
        ) -> wavyte::WavyteResult<wavyte::PreparedAsset> {
            Err(wavyte::WavyteError::evaluation("no assets in this test"))
        }
    }

    fn full_cover_path_comp() -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "p0".to_string(),
            Asset::Path(PathAsset {
                svg_path_d: "M-100,-100 L1000,-100 L1000,1000 L-100,1000 Z".to_string(),
            }),
        );

        Composition {
            fps: wavyte::Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 64,
                height: 64,
            },
            duration: FrameIndex(1),
            assets,
            tracks: vec![Track {
                name: "main".to_string(),
                z_base: 0,
                clips: vec![Clip {
                    id: "c0".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(1)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: None,
                }],
            }],
            seed: 1,
        }
    }

    #[test]
    fn cpu_and_gpu_match_on_solid_fill() {
        let comp = full_cover_path_comp();

        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };

        let mut cpu = create_backend(BackendKind::Cpu, &settings).unwrap();
        let mut gpu = create_backend(BackendKind::Gpu, &settings).unwrap();
        let mut assets = NoAssets;

        let a = render_frame(&comp, FrameIndex(0), cpu.as_mut(), &mut assets).unwrap();
        let b = match render_frame(&comp, FrameIndex(0), gpu.as_mut(), &mut assets) {
            Ok(v) => v,
            Err(e) if e.to_string().contains("no gpu adapter available") => return,
            Err(e) => panic!("unexpected gpu render error: {e}"),
        };

        assert_eq!(a.data.len(), b.data.len());
        assert_eq!(a.data, b.data);
        assert!(a.data.iter().all(|&x| x == 255));
    }
}
