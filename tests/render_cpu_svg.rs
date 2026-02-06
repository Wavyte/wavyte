mod cpu_svg {
    use std::{collections::BTreeMap, fs};

    use wavyte::{
        Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, FrameIndex,
        FrameRange, PreparedAssetStore, RenderSettings, SvgAsset, Track, Transform2D,
        create_backend, render_frame,
    };

    fn mix64(mut z: u64) -> u64 {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn digest_u64(bytes: &[u8]) -> u64 {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for chunk in bytes.chunks(8) {
            let mut v = 0u64;
            for (i, &b) in chunk.iter().enumerate() {
                v |= (b as u64) << (i * 8);
            }
            state = mix64(state ^ v);
        }
        state
    }

    fn simple_svg_comp(source: &str) -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "s0".to_string(),
            Asset::Svg(SvgAsset {
                source: source.to_string(),
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
                    asset: "s0".to_string(),
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
    fn cpu_svg_render_is_deterministic_and_nonempty() {
        let tmp = std::env::temp_dir().join(format!(
            "wavyte_svg_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let svg_path = tmp.join("test.svg");
        fs::write(
            &svg_path,
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
  <rect x="0" y="0" width="64" height="64" fill="#ff00ff"/>
</svg>"##,
        )
        .unwrap();

        let comp = simple_svg_comp("test.svg");
        let assets = PreparedAssetStore::prepare(&comp, &tmp).unwrap();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 0]),
        };
        let mut backend = create_backend(BackendKind::Cpu, &settings).unwrap();

        let a = render_frame(&comp, FrameIndex(0), backend.as_mut(), &assets).unwrap();
        let b = render_frame(&comp, FrameIndex(0), backend.as_mut(), &assets).unwrap();

        assert_eq!(a.width, 64);
        assert_eq!(a.height, 64);
        assert!(a.premultiplied);
        assert_eq!(digest_u64(&a.data), digest_u64(&b.data));
        assert!(a.data.iter().any(|&x| x != 0));

        fs::remove_dir_all(&tmp).ok();
    }
}
