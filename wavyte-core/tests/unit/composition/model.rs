use super::*;
use crate::foundation::core::Vec2;

fn basic_comp() -> Composition {
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
            width: 1920,
            height: 1080,
        },
        duration: FrameIndex(60),
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            layout_mode: LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: Edges::default(),
            layout_align_x: LayoutAlignX::Start,
            layout_align_y: LayoutAlignY::Start,
            layout_grid_columns: default_layout_grid_columns(),
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "t0".to_string(),
                range: FrameRange::new(FrameIndex(0), FrameIndex(60)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D {
                        translate: Vec2::new(10.0, 20.0),
                        ..Transform2D::default()
                    }),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![EffectInstance {
                    kind: "noop".to_string(),
                    params: serde_json::Value::Null,
                }],
                transition_in: Some(TransitionSpec {
                    kind: "crossfade".to_string(),
                    duration_frames: 10,
                    ease: Ease::Linear,
                    params: serde_json::Value::Null,
                }),
                transition_out: None,
            }],
        }],
        seed: 123,
    }
}

#[test]
fn json_roundtrip() {
    let comp = basic_comp();
    let s = serde_json::to_string_pretty(&comp).unwrap();
    let de: Composition = serde_json::from_str(&s).unwrap();
    assert_eq!(de.canvas.width, 1920);
    assert_eq!(de.assets.len(), 1);
}

#[test]
fn validate_rejects_missing_asset() {
    let mut comp = basic_comp();
    comp.tracks[0].clips[0].asset = "missing".to_string();
    assert!(comp.validate().is_err());
}

#[test]
fn validate_rejects_out_of_bounds_range() {
    let mut comp = basic_comp();
    comp.tracks[0].clips[0].range = FrameRange {
        start: FrameIndex(0),
        end: FrameIndex(999),
    };
    assert!(comp.validate().is_err());
}

#[test]
fn validate_rejects_bad_fps() {
    let mut comp = basic_comp();
    comp.fps = Fps { num: 30, den: 0 };
    assert!(comp.validate().is_err());
}

#[test]
fn media_assets_serde_defaults_and_validation() {
    let json = r#"{
        "fps": {"num": 30, "den": 1},
        "canvas": {"width": 640, "height": 360},
        "duration": 30,
        "assets": {
            "v0": {"Video": {"source": "assets/a.mp4"}},
            "a0": {"Audio": {"source": "assets/b.wav"}}
        },
        "tracks": [],
        "seed": 1
    }"#;
    let comp: Composition = serde_json::from_str(json).unwrap();
    comp.validate().unwrap();

    let Asset::Video(v) = comp.assets.get("v0").unwrap() else {
        panic!("expected video asset");
    };
    assert_eq!(v.trim_start_sec, 0.0);
    assert_eq!(v.playback_rate, 1.0);
    assert_eq!(v.volume, 1.0);
    assert!(!v.muted);
}

#[test]
fn media_validation_rejects_non_positive_playback_rate() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "v0".to_string(),
        Asset::Video(VideoAsset {
            source: "assets/a.mp4".to_string(),
            trim_start_sec: 0.0,
            trim_end_sec: None,
            playback_rate: 0.0,
            volume: 1.0,
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            muted: false,
        }),
    );
    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(10),
        assets,
        tracks: vec![],
        seed: 1,
    };
    assert!(comp.validate().is_err());
}
