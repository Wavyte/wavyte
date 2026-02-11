#[path = "support/mod.rs"]
mod support;

use wavyte::{
    Anim, Asset, ClipBuilder, Composition, CompositionBuilder, Ease, Fps, FrameIndex, FrameRange,
    InterpMode, Keyframe, Keyframes, LayoutMode, PathAsset, RenderThreading, SvgAsset, TextAsset,
    TrackBuilder, Transform2D, Vec2,
};

fn scalar_keys(keys: &[(u64, f64, Ease)]) -> Anim<f64> {
    Anim::Keyframes(Keyframes {
        keys: keys
            .iter()
            .map(|(frame, value, ease)| Keyframe {
                frame: FrameIndex(*frame),
                value: *value,
                ease: *ease,
            })
            .collect(),
        mode: InterpMode::Linear,
        default: None,
    })
}

fn transform_keys(keys: &[(u64, Transform2D, Ease)]) -> Anim<Transform2D> {
    Anim::Keyframes(Keyframes {
        keys: keys
            .iter()
            .map(|(frame, value, ease)| Keyframe {
                frame: FrameIndex(*frame),
                value: *value,
                ease: *ease,
            })
            .collect(),
        mode: InterpMode::Linear,
        default: None,
    })
}

fn build_comp() -> anyhow::Result<Composition> {
    let duration = FrameIndex(120); // 4s @ 30fps

    let glow_track = TrackBuilder::new("glow")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new("glow_orb", "orb", FrameRange::new(FrameIndex(0), duration)?)
                .transform(transform_keys(&[
                    (
                        0,
                        Transform2D {
                            translate: Vec2::new(730.0, 290.0),
                            scale: Vec2::new(3.0, 3.0),
                            ..Transform2D::default()
                        },
                        Ease::OutCubic,
                    ),
                    (
                        119,
                        Transform2D {
                            translate: Vec2::new(690.0, 310.0),
                            scale: Vec2::new(3.4, 3.4),
                            ..Transform2D::default()
                        },
                        Ease::Linear,
                    ),
                ]))
                .opacity(scalar_keys(&[
                    (0, 0.06, Ease::Linear),
                    (119, 0.2, Ease::Linear),
                ]))
                .effect(wavyte::EffectInstance {
                    kind: "blur".to_string(),
                    params: serde_json::json!({ "radius_px": 40, "sigma": 16.0 }),
                })
                .build()?,
        )
        .build()?;

    let logo_track = TrackBuilder::new("logo")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new(
                "logo_hero",
                "logo",
                FrameRange::new(FrameIndex(0), duration)?,
            )
            .transform(transform_keys(&[
                (
                    0,
                    Transform2D {
                        translate: Vec2::new(500.0, -120.0),
                        scale: Vec2::new(0.72, 0.72),
                        rotation_rad: -0.16,
                        ..Transform2D::default()
                    },
                    Ease::OutCubic,
                ),
                (
                    26,
                    Transform2D {
                        translate: Vec2::new(500.0, 84.0),
                        scale: Vec2::new(1.0, 1.0),
                        rotation_rad: 0.0,
                        ..Transform2D::default()
                    },
                    Ease::OutCubic,
                ),
                (
                    119,
                    Transform2D {
                        translate: Vec2::new(500.0, 84.0),
                        scale: Vec2::new(1.0, 1.0),
                        rotation_rad: 0.0,
                        ..Transform2D::default()
                    },
                    Ease::Linear,
                ),
            ]))
            .opacity(scalar_keys(&[
                (0, 0.0, Ease::Linear),
                (8, 1.0, Ease::OutCubic),
                (110, 1.0, Ease::Linear),
                (119, 0.0, Ease::InCubic),
            ]))
            .build()?,
        )
        .build()?;

    let title_track = TrackBuilder::new("title")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new(
                "hello_title",
                "title",
                FrameRange::new(FrameIndex(0), duration)?,
            )
            .transform(transform_keys(&[
                (
                    0,
                    Transform2D {
                        translate: Vec2::new(184.0, 448.0),
                        scale: Vec2::new(0.95, 0.95),
                        ..Transform2D::default()
                    },
                    Ease::OutCubic,
                ),
                (
                    28,
                    Transform2D {
                        translate: Vec2::new(184.0, 392.0),
                        scale: Vec2::new(1.0, 1.0),
                        ..Transform2D::default()
                    },
                    Ease::OutCubic,
                ),
                (
                    119,
                    Transform2D {
                        translate: Vec2::new(184.0, 392.0),
                        scale: Vec2::new(1.0, 1.0),
                        ..Transform2D::default()
                    },
                    Ease::Linear,
                ),
            ]))
            .opacity(scalar_keys(&[
                (0, 0.0, Ease::Linear),
                (10, 1.0, Ease::OutCubic),
                (112, 1.0, Ease::Linear),
                (119, 0.0, Ease::InCubic),
            ]))
            .build()?,
        )
        .build()?;

    let subtitle_track = TrackBuilder::new("subtitle")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new(
                "hello_subtitle",
                "subtitle",
                FrameRange::new(FrameIndex(0), duration)?,
            )
            .transform(transform_keys(&[
                (
                    0,
                    Transform2D {
                        translate: Vec2::new(216.0, 500.0),
                        ..Transform2D::default()
                    },
                    Ease::OutCubic,
                ),
                (
                    36,
                    Transform2D {
                        translate: Vec2::new(216.0, 466.0),
                        ..Transform2D::default()
                    },
                    Ease::OutCubic,
                ),
                (
                    119,
                    Transform2D {
                        translate: Vec2::new(216.0, 466.0),
                        ..Transform2D::default()
                    },
                    Ease::Linear,
                ),
            ]))
            .opacity(scalar_keys(&[
                (0, 0.0, Ease::Linear),
                (24, 0.0, Ease::Linear),
                (42, 1.0, Ease::OutCubic),
                (112, 1.0, Ease::Linear),
                (119, 0.0, Ease::InCubic),
            ]))
            .build()?,
        )
        .build()?;

    let comp = CompositionBuilder::new(
        Fps::new(30, 1)?,
        wavyte::Canvas {
            width: 1280,
            height: 720,
        },
        duration,
    )
    .seed(42)
    .asset(
        "logo",
        Asset::Svg(SvgAsset {
            source: "assets/logo.svg".to_string(),
        }),
    )?
    .asset(
        "title",
        Asset::Text(TextAsset {
            text: "Hello, Wavyte".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 96.0,
            max_width_px: Some(1000.0),
            color_rgba8: [244, 246, 255, 255],
        }),
    )?
    .asset(
        "subtitle",
        Asset::Text(TextAsset {
            text: "A timeline-first renderer, in Rust".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 42.0,
            max_width_px: Some(900.0),
            color_rgba8: [205, 220, 255, 255],
        }),
    )?
    .asset(
        "orb",
        Asset::Path(PathAsset {
            svg_path_d: "M60,0 A60,60 0 1 1 59.9,0".to_string(),
        }),
    )?
    .track(glow_track)
    .track(logo_track)
    .track(title_track)
    .track(subtitle_track)
    .build()?;

    Ok(comp)
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn try_main() -> anyhow::Result<()> {
    let comp = build_comp()?;
    let threads = std::env::var("WAVYTE_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4);
    let out = support::render_comp_to_assets_mp4(
        &comp,
        &format!("remotion_hello_world_style_t{threads}.mp4"),
        [16, 23, 47, 255],
        RenderThreading {
            parallel: true,
            chunk_size: 48,
            threads: Some(threads),
            static_frame_elision: false,
        },
    )?;
    eprintln!("threads={threads}");
    eprintln!("wrote {}", out.display());
    Ok(())
}
