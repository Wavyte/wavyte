mod render_parallel_parity {
    use std::collections::BTreeMap;

    use wavyte::{
        Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, FrameIndex,
        FrameRange, Keyframe, Keyframes, PreparedAssetStore, RenderSettings, RenderThreading,
        Track, Transform2D, Vec2, create_backend, render_frames_with_stats,
    };

    fn moving_comp() -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "p0".to_string(),
            Asset::Path(wavyte::PathAsset {
                svg_path_d: "M0,0 L30,0 L30,30 L0,30 Z".to_string(),
            }),
        );

        let duration = FrameIndex(12);
        let transform = Anim::Keyframes(Keyframes {
            keys: vec![
                Keyframe {
                    frame: FrameIndex(0),
                    value: Transform2D {
                        translate: Vec2::new(4.0, 16.0),
                        ..Transform2D::default()
                    },
                    ease: wavyte::Ease::Linear,
                },
                Keyframe {
                    frame: FrameIndex(11),
                    value: Transform2D {
                        translate: Vec2::new(24.0, 16.0),
                        ..Transform2D::default()
                    },
                    ease: wavyte::Ease::Linear,
                },
            ],
            mode: wavyte::InterpMode::Linear,
            default: None,
        });

        Composition {
            fps: wavyte::Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 64,
                height: 64,
            },
            duration,
            assets,
            tracks: vec![Track {
                name: "main".to_string(),
                z_base: 0,
                layout_mode: wavyte::LayoutMode::Absolute,
                layout_gap_px: 0.0,
                layout_padding: wavyte::Edges::default(),
                layout_align_x: wavyte::LayoutAlignX::Start,
                layout_align_y: wavyte::LayoutAlignY::Start,
                layout_grid_columns: 2,
                clips: vec![Clip {
                    id: "c0".to_string(),
                    asset: "p0".to_string(),
                    range: FrameRange::new(FrameIndex(0), duration).unwrap(),
                    props: ClipProps {
                        transform,
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: None,
                }],
            }],
            seed: 7,
        }
    }

    fn static_comp() -> Composition {
        let mut comp = moving_comp();
        comp.duration = FrameIndex(8);
        comp.tracks[0].clips[0].range = FrameRange::new(FrameIndex(0), FrameIndex(8)).unwrap();
        comp.tracks[0].clips[0].props.transform = Anim::constant(Transform2D {
            translate: Vec2::new(8.0, 8.0),
            ..Transform2D::default()
        });
        comp
    }

    #[test]
    fn sequential_and_parallel_match_for_multiple_chunk_sizes() {
        let comp = moving_comp();
        let range = FrameRange::new(FrameIndex(0), comp.duration).unwrap();
        let assets = PreparedAssetStore::prepare(&comp, ".").unwrap();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };

        let mut seq_backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let (seq_frames, _) = render_frames_with_stats(
            &comp,
            range,
            seq_backend.as_mut(),
            &assets,
            &RenderThreading::default(),
        )
        .unwrap();

        for chunk_size in [1usize, 3, 8] {
            let mut par_backend = create_backend(BackendKind::Cpu, &settings).unwrap();
            let opts = RenderThreading {
                parallel: true,
                chunk_size,
                threads: Some(4),
                static_frame_elision: false,
            };
            let (par_frames, stats) =
                render_frames_with_stats(&comp, range, par_backend.as_mut(), &assets, &opts)
                    .unwrap();

            assert_eq!(stats.frames_elided, 0);
            assert_eq!(seq_frames.len(), par_frames.len());
            for (a, b) in seq_frames.iter().zip(par_frames.iter()) {
                assert_eq!(a.width, b.width);
                assert_eq!(a.height, b.height);
                assert_eq!(a.premultiplied, b.premultiplied);
                assert_eq!(a.data, b.data);
            }
        }
    }

    #[test]
    fn static_frame_elision_reports_expected_counts() {
        let comp = static_comp();
        let range = FrameRange::new(FrameIndex(0), comp.duration).unwrap();
        let assets = PreparedAssetStore::prepare(&comp, ".").unwrap();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };

        let mut backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let opts = RenderThreading {
            parallel: true,
            chunk_size: range.len_frames() as usize,
            threads: Some(4),
            static_frame_elision: true,
        };
        let (frames, stats) =
            render_frames_with_stats(&comp, range, backend.as_mut(), &assets, &opts).unwrap();

        assert_eq!(stats.frames_total, 8);
        assert_eq!(stats.frames_rendered, 1);
        assert_eq!(stats.frames_elided, 7);
        for frame in frames.iter().skip(1) {
            assert_eq!(frame.data, frames[0].data);
        }
    }
}
