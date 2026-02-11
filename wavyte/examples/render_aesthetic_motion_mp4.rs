#[path = "support/mod.rs"]
mod support;

use wavyte::{
    Anim, Asset, ClipBuilder, Composition, CompositionBuilder, Ease, EffectInstance, Fps,
    FrameIndex, FrameRange, ImageAsset, InterpMode, Keyframe, Keyframes, LayoutMode, PathAsset,
    RenderThreading, SvgAsset, TextAsset, TrackBuilder, Transform2D, TransitionSpec, Vec2,
};

fn linear_transform(
    start: u64,
    end: u64,
    from: Transform2D,
    to: Transform2D,
    ease: Ease,
) -> Anim<Transform2D> {
    Anim::Keyframes(Keyframes {
        keys: vec![
            Keyframe {
                frame: FrameIndex(start),
                value: from,
                ease,
            },
            Keyframe {
                frame: FrameIndex(end),
                value: to,
                ease: Ease::Linear,
            },
        ],
        mode: InterpMode::Linear,
        default: None,
    })
}

fn build_comp() -> anyhow::Result<Composition> {
    let duration = FrameIndex(180); // 6s @ 30fps

    let crossfade = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 18,
        ease: Ease::InOutQuad,
        params: serde_json::Value::Null,
    };
    let wipe = TransitionSpec {
        kind: "wipe".to_string(),
        duration_frames: 18,
        ease: Ease::InOutCubic,
        params: serde_json::json!({ "dir": "left_to_right", "soft_edge": 0.08 }),
    };

    let hero = TrackBuilder::new("hero")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new(
                "hero_img",
                "img",
                FrameRange::new(FrameIndex(0), FrameIndex(84))?,
            )
            .transform(linear_transform(
                0,
                83,
                Transform2D {
                    translate: Vec2::new(-120.0, 30.0),
                    scale: Vec2::new(0.56, 0.56),
                    ..Transform2D::default()
                },
                Transform2D {
                    translate: Vec2::new(120.0, 30.0),
                    scale: Vec2::new(0.60, 0.60),
                    ..Transform2D::default()
                },
                Ease::InOutCubic,
            ))
            .transition_out(crossfade.clone())
            .build()?,
        )
        .clip(
            ClipBuilder::new(
                "hero_logo",
                "logo",
                FrameRange::new(FrameIndex(66), FrameIndex(144))?,
            )
            .transform(linear_transform(
                0,
                77,
                Transform2D {
                    translate: Vec2::new(880.0, 110.0),
                    scale: Vec2::new(0.8, 0.8),
                    ..Transform2D::default()
                },
                Transform2D {
                    translate: Vec2::new(500.0, 110.0),
                    scale: Vec2::new(1.1, 1.1),
                    ..Transform2D::default()
                },
                Ease::OutCubic,
            ))
            .transition_in(crossfade)
            .transition_out(wipe.clone())
            .build()?,
        )
        .clip(
            ClipBuilder::new(
                "hero_orbit",
                "ring",
                FrameRange::new(FrameIndex(126), duration)?,
            )
            .transform(linear_transform(
                0,
                53,
                Transform2D {
                    translate: Vec2::new(460.0, 470.0),
                    rotation_rad: -0.6,
                    scale: Vec2::new(2.0, 2.0),
                    ..Transform2D::default()
                },
                Transform2D {
                    translate: Vec2::new(460.0, 200.0),
                    rotation_rad: 0.45,
                    scale: Vec2::new(3.2, 3.2),
                    ..Transform2D::default()
                },
                Ease::InOutQuad,
            ))
            .transition_in(wipe)
            .effect(EffectInstance {
                kind: "opacity_mul".to_string(),
                params: serde_json::json!({ "value": 0.8 }),
            })
            .build()?,
        )
        .build()?;

    let title = TrackBuilder::new("title")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new(
                "title_clip",
                "title",
                FrameRange::new(FrameIndex(0), duration)?,
            )
            .transform(Anim::constant(Transform2D {
                translate: Vec2::new(70.0, 46.0),
                ..Transform2D::default()
            }))
            .opacity(Anim::Keyframes(Keyframes {
                keys: vec![
                    Keyframe {
                        frame: FrameIndex(0),
                        value: 0.0,
                        ease: Ease::OutCubic,
                    },
                    Keyframe {
                        frame: FrameIndex(16),
                        value: 1.0,
                        ease: Ease::Linear,
                    },
                    Keyframe {
                        frame: FrameIndex(150),
                        value: 1.0,
                        ease: Ease::Linear,
                    },
                    Keyframe {
                        frame: FrameIndex(179),
                        value: 0.0,
                        ease: Ease::Linear,
                    },
                ],
                mode: InterpMode::Linear,
                default: None,
            }))
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
    .seed(7)
    .asset(
        "img",
        Asset::Image(ImageAsset {
            source: "assets/test_image_1.jpg".to_string(),
        }),
    )?
    .asset(
        "logo",
        Asset::Svg(SvgAsset {
            source: "assets/logo.svg".to_string(),
        }),
    )?
    .asset(
        "ring",
        Asset::Path(PathAsset {
            svg_path_d: "M60,0 A60,60 0 1 1 59.9,0 M60,22 A38,38 0 1 0 60.1,22 Z".to_string(),
        }),
    )?
    .asset(
        "title",
        Asset::Text(TextAsset {
            text: "AESTHETIC UNLOCK I: MOTION".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 66.0,
            max_width_px: Some(1100.0),
            color_rgba8: [244, 244, 236, 255],
        }),
    )?
    .track(hero)
    .track(title)
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
    let out = support::render_comp_to_assets_mp4(
        &comp,
        "aesthetic_motion.mp4",
        [12, 14, 20, 255],
        RenderThreading {
            parallel: true,
            chunk_size: 48,
            threads: Some(4),
            static_frame_elision: false,
        },
    )?;
    eprintln!("wrote {}", out.display());
    Ok(())
}
