use super::*;
use crate::{Anim, Asset, Canvas, Clip, ClipProps, FrameIndex, FrameRange, PathAsset, Track};

fn comp_for_layout(mode: LayoutMode) -> Composition {
    let mut assets = std::collections::BTreeMap::new();
    assets.insert(
        "a".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 L0,10 Z".to_string(),
        }),
    );
    assets.insert(
        "b".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L20,0 L20,10 L0,10 Z".to_string(),
        }),
    );
    Composition {
        fps: crate::Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 100,
            height: 40,
        },
        duration: FrameIndex(1),
        assets,
        tracks: vec![Track {
            name: "t".to_string(),
            z_base: 0,
            layout_mode: mode,
            layout_gap_px: 5.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Center,
            layout_grid_columns: 2,
            clips: vec![
                Clip {
                    id: "c0".to_string(),
                    asset: "a".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(1)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(crate::Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: crate::BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: None,
                },
                Clip {
                    id: "c1".to_string(),
                    asset: "b".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(1)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(crate::Transform2D::default()),
                        opacity: Anim::constant(1.0),
                        blend: crate::BlendMode::Normal,
                    },
                    z_offset: 1,
                    effects: vec![],
                    transition_in: None,
                    transition_out: None,
                },
            ],
        }],
        seed: 1,
    }
}

#[test]
fn hstack_offsets_are_deterministic() {
    let comp = comp_for_layout(LayoutMode::HStack);
    let store = PreparedAssetStore::prepare(&comp, ".").unwrap();
    let offsets = resolve_layout_offsets(&comp, &store).unwrap();
    assert_eq!(offsets.offset_for(0, 0), Vec2::new(0.0, 15.0));
    assert_eq!(offsets.offset_for(0, 1), Vec2::new(15.0, 15.0));
}

#[test]
fn center_mode_centers_each_clip() {
    let comp = comp_for_layout(LayoutMode::Center);
    let store = PreparedAssetStore::prepare(&comp, ".").unwrap();
    let offsets = resolve_layout_offsets(&comp, &store).unwrap();
    assert_eq!(offsets.offset_for(0, 0), Vec2::new(45.0, 15.0));
    assert_eq!(offsets.offset_for(0, 1), Vec2::new(40.0, 15.0));
}
