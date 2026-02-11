use super::*;
use crate::{
    animation::ease::Ease,
    composition::model::{Asset, TextAsset},
    foundation::core::{Fps, Vec2},
};

#[test]
fn builders_create_expected_structure() {
    let clip = ClipBuilder::new(
        "c0",
        "t0",
        FrameRange::new(
            crate::foundation::core::FrameIndex(0),
            crate::foundation::core::FrameIndex(30),
        )
        .unwrap(),
    )
    .opacity(Anim::constant(0.5))
    .transform(Anim::constant(Transform2D {
        translate: Vec2::new(1.0, 2.0),
        ..Transform2D::default()
    }))
    .transition_in(TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 10,
        ease: Ease::Linear,
        params: serde_json::Value::Null,
    })
    .build()
    .unwrap();

    let track = TrackBuilder::new("main").clip(clip).build().unwrap();

    let comp = CompositionBuilder::new(
        Fps::new(30, 1).unwrap(),
        Canvas {
            width: 640,
            height: 360,
        },
        FrameIndex(30),
    )
    .asset(
        "t0",
        Asset::Text(TextAsset {
            text: "hello".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 48.0,
            max_width_px: None,
            color_rgba8: [255, 255, 255, 255],
        }),
    )
    .unwrap()
    .track(track)
    .build()
    .unwrap();

    assert_eq!(comp.assets.len(), 1);
    assert_eq!(comp.tracks.len(), 1);
}

#[test]
fn duplicate_asset_key_is_rejected() {
    let builder = CompositionBuilder::new(
        Fps::new(30, 1).unwrap(),
        Canvas {
            width: 640,
            height: 360,
        },
        FrameIndex(1),
    )
    .asset(
        "t0",
        Asset::Text(TextAsset {
            text: "a".into(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 48.0,
            max_width_px: None,
            color_rgba8: [255, 255, 255, 255],
        }),
    )
    .unwrap();
    assert!(
        builder
            .asset(
                "t0",
                Asset::Text(TextAsset {
                    text: "b".into(),
                    font_source: "assets/PlayfairDisplay.ttf".to_string(),
                    size_px: 48.0,
                    max_width_px: None,
                    color_rgba8: [255, 255, 255, 255],
                }),
            )
            .is_err()
    );
}
