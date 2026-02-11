use super::*;
use crate::{Anim, BlendMode, Canvas, Clip, ClipProps, Composition, Evaluator, FrameIndex};

fn comp_with_opacity(opacity: f64) -> Composition {
    Composition {
        fps: crate::Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(2),
        assets: std::collections::BTreeMap::from([(
            "p0".to_string(),
            crate::Asset::Path(crate::PathAsset {
                svg_path_d: "M0,0 L10,0 L10,10 Z".to_string(),
            }),
        )]),
        tracks: vec![crate::Track {
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
                asset: "p0".to_string(),
                range: crate::FrameRange::new(FrameIndex(0), FrameIndex(2)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(crate::Transform2D::default()),
                    opacity: Anim::constant(opacity),
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
fn fingerprint_is_deterministic_for_same_eval() {
    let comp = comp_with_opacity(1.0);
    let eval = Evaluator::eval_frame(&comp, FrameIndex(0)).unwrap();
    let a = fingerprint_eval(&eval);
    let b = fingerprint_eval(&eval);
    assert_eq!(a, b);
}

#[test]
fn fingerprint_changes_when_scene_changes() {
    let a_comp = comp_with_opacity(1.0);
    let b_comp = comp_with_opacity(0.5);
    let a_eval = Evaluator::eval_frame(&a_comp, FrameIndex(0)).unwrap();
    let b_eval = Evaluator::eval_frame(&b_comp, FrameIndex(0)).unwrap();
    assert_ne!(fingerprint_eval(&a_eval), fingerprint_eval(&b_eval));
}
