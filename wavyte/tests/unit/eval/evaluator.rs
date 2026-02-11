use super::*;
use crate::{
    animation::anim::Anim,
    animation::ease::Ease,
    composition::model::{Asset, ClipProps, TextAsset, Track},
    foundation::core::{Canvas, Fps, Transform2D, Vec2},
};
use std::collections::BTreeMap;

fn basic_comp(
    opacity: Anim<f64>,
    tr_in: Option<TransitionSpec>,
    tr_out: Option<TransitionSpec>,
) -> Composition {
    let mut assets = BTreeMap::new();
    assets.insert(
        "t0".to_string(),
        Asset::Text(TextAsset {
            text: "hello".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 48.0,
            max_width_px: None,
            color_rgba8: [255, 255, 255, 255],
        }),
    );
    Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 640,
            height: 360,
        },
        duration: FrameIndex(20),
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "t0".to_string(),
                range: FrameRange::new(FrameIndex(5), FrameIndex(15)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D {
                        translate: Vec2::new(1.0, 2.0),
                        ..Transform2D::default()
                    }),
                    opacity,
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![],
                transition_in: tr_in,
                transition_out: tr_out,
            }],
        }],
        seed: 1,
    }
}

#[test]
fn visibility_respects_frame_range() {
    let comp = basic_comp(Anim::constant(1.0), None, None);
    assert_eq!(
        Evaluator::eval_frame(&comp, FrameIndex(4))
            .unwrap()
            .nodes
            .len(),
        0
    );
    assert_eq!(
        Evaluator::eval_frame(&comp, FrameIndex(5))
            .unwrap()
            .nodes
            .len(),
        1
    );
    assert_eq!(
        Evaluator::eval_frame(&comp, FrameIndex(14))
            .unwrap()
            .nodes
            .len(),
        1
    );
    assert_eq!(
        Evaluator::eval_frame(&comp, FrameIndex(15))
            .unwrap()
            .nodes
            .len(),
        0
    );
}

#[test]
fn opacity_is_clamped() {
    let opacity = Anim::constant(2.0);
    let comp = basic_comp(opacity, None, None);
    let g = Evaluator::eval_frame(&comp, FrameIndex(5)).unwrap();
    assert_eq!(g.nodes[0].opacity, 1.0);
}

#[test]
fn transition_progress_boundaries() {
    let tr = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 3,
        ease: Ease::Linear,
        params: serde_json::Value::Null,
    };
    let comp = basic_comp(Anim::constant(1.0), Some(tr.clone()), Some(tr));

    // In transition at clip start frame.
    let g0 = Evaluator::eval_frame(&comp, FrameIndex(5)).unwrap();
    assert_eq!(g0.nodes[0].transition_in.as_ref().unwrap().progress, 0.0);

    // Last in-transition frame hits progress 1.0 (dur=3 => denom=2).
    let g_last_in = Evaluator::eval_frame(&comp, FrameIndex(7)).unwrap();
    assert_eq!(
        g_last_in.nodes[0].transition_in.as_ref().unwrap().progress,
        1.0
    );

    // Out transition starts at end-dur.
    let g_out0 = Evaluator::eval_frame(&comp, FrameIndex(12)).unwrap();
    assert_eq!(
        g_out0.nodes[0].transition_out.as_ref().unwrap().progress,
        0.0
    );

    let g_out_last = Evaluator::eval_frame(&comp, FrameIndex(14)).unwrap();
    assert_eq!(
        g_out_last.nodes[0]
            .transition_out
            .as_ref()
            .unwrap()
            .progress,
        1.0
    );
}
