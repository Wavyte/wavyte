mod svg_text {
    use std::collections::BTreeMap;

    use usvg::Node;
    use wavyte::{
        Anim, Asset, AssetCache as _, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition,
        FrameIndex, FrameRange, FsAssetCache, RenderSettings, SvgAsset, Track, Transform2D, Vec2,
        create_backend, render_frame,
    };

    fn count_text_nodes(group: &usvg::Group) -> usize {
        let mut n = 0usize;
        for child in group.children() {
            match child {
                Node::Group(g) => n += count_text_nodes(g.as_ref()),
                Node::Text(_) => n += 1,
                Node::Path(_) | Node::Image(_) => {}
            }
        }
        n
    }

    fn comp_with_svg_text() -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "s0".to_string(),
            Asset::Svg(SvgAsset {
                source: "svg_with_text.svg".to_string(),
            }),
        );

        Composition {
            fps: wavyte::Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 512,
                height: 192,
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
                        transform: Anim::constant(Transform2D {
                            translate: Vec2::new(0.0, 0.0),
                            scale: Vec2::new(2.0, 2.0),
                            ..Transform2D::default()
                        }),
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

    fn comp_with_svg_text_missing_font_stack() -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "s0".to_string(),
            Asset::Svg(SvgAsset {
                source: "svg_missing_font_fallback.svg".to_string(),
            }),
        );

        Composition {
            fps: wavyte::Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 512,
                height: 192,
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
                        transform: Anim::constant(Transform2D {
                            translate: Vec2::new(0.0, 0.0),
                            scale: Vec2::new(2.0, 2.0),
                            ..Transform2D::default()
                        }),
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

    fn assert_svg_fixture_fontdb_contains_inconsolata(
        assets: &mut FsAssetCache,
        comp: &Composition,
    ) {
        let Some(asset) = comp.assets.get("s0") else {
            panic!("missing svg asset 's0' in test composition");
        };
        let prepared = assets.get_or_load(asset).unwrap();
        let wavyte::PreparedAsset::Svg(p) = prepared else {
            panic!("expected prepared svg asset");
        };

        let has_inconsolata = p
            .tree
            .fontdb()
            .faces()
            .any(|f| f.families.iter().any(|(name, _)| name == "Inconsolata"));
        assert!(
            has_inconsolata,
            "expected SVG fontdb to contain the vendored test font family 'Inconsolata'"
        );
    }

    #[test]
    fn cpu_svg_text_renders_nonempty() {
        let comp = comp_with_svg_text();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 0]),
        };
        let mut backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let mut assets = FsAssetCache::new("tests/data");

        assert_svg_fixture_fontdb_contains_inconsolata(&mut assets, &comp);

        let frame = render_frame(&comp, FrameIndex(0), backend.as_mut(), &mut assets).unwrap();
        assert_eq!(frame.width, 512);
        assert_eq!(frame.height, 192);
        assert!(frame.premultiplied);
        assert!(
            frame.data.iter().any(|&b| b != 0),
            "expected non-empty pixels; svg fixture contains only <text> on transparent background"
        );
    }

    #[test]
    fn cpu_svg_text_renders_even_when_font_family_is_missing() {
        let comp = comp_with_svg_text_missing_font_stack();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 0]),
        };
        let mut backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let mut assets = FsAssetCache::new("tests/data");

        // The fixture references nonexistent family names. This should still render text by falling
        // back to any available face (vendored font in tests/data/fonts).
        let Some(asset) = comp.assets.get("s0") else {
            panic!("missing svg asset 's0' in test composition");
        };
        let prepared = assets.get_or_load(asset).unwrap();
        let wavyte::PreparedAsset::Svg(p) = prepared else {
            panic!("expected prepared svg asset");
        };
        assert_eq!(count_text_nodes(p.tree.root()), 1);

        let frame = render_frame(&comp, FrameIndex(0), backend.as_mut(), &mut assets).unwrap();
        assert!(frame.data.iter().any(|&b| b != 0));
    }
}
