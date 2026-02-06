#[path = "support/mod.rs"]
mod support;

use wavyte::{
    Anim, Asset, ClipBuilder, Composition, CompositionBuilder, Ease, EffectInstance, Fps,
    FrameIndex, FrameRange, ImageAsset, InterpMode, Keyframe, Keyframes, LayoutMode,
    RenderThreading, SvgAsset, TextAsset, TrackBuilder, Transform2D, Vec2,
};

fn pulse_opacity() -> Anim<f64> {
    Anim::Keyframes(Keyframes {
        keys: vec![
            Keyframe {
                frame: FrameIndex(0),
                value: 0.40,
                ease: Ease::OutQuad,
            },
            Keyframe {
                frame: FrameIndex(45),
                value: 1.0,
                ease: Ease::InOutQuad,
            },
            Keyframe {
                frame: FrameIndex(90),
                value: 0.55,
                ease: Ease::InOutQuad,
            },
            Keyframe {
                frame: FrameIndex(135),
                value: 1.0,
                ease: Ease::Linear,
            },
        ],
        mode: InterpMode::Linear,
        default: None,
    })
}

fn build_comp() -> anyhow::Result<Composition> {
    let duration = FrameIndex(150); // 5s @ 30fps
    let range = FrameRange::new(FrameIndex(0), duration)?;

    let bg_track = TrackBuilder::new("bg")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new("soft_bg", "img", range)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(0.0, -10.0),
                    scale: Vec2::new(0.72, 0.72),
                    ..Transform2D::default()
                }))
                .effect(EffectInstance {
                    kind: "blur".to_string(),
                    params: serde_json::json!({ "radius_px": 14, "sigma": 6.0 }),
                })
                .effect(EffectInstance {
                    kind: "opacity_mul".to_string(),
                    params: serde_json::json!({ "value": 0.70 }),
                })
                .build()?,
        )
        .build()?;

    let logo_track = TrackBuilder::new("logo")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new("logo_core", "logo", range)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(420.0, 160.0),
                    scale: Vec2::new(1.35, 1.35),
                    ..Transform2D::default()
                }))
                .opacity(pulse_opacity())
                .effect(EffectInstance {
                    kind: "transform_post".to_string(),
                    params: serde_json::json!({
                        "translate": [0.0, -12.0],
                        "rotate_deg": 4.0,
                        "scale": [1.06, 1.06]
                    }),
                })
                .effect(EffectInstance {
                    kind: "opacity_mul".to_string(),
                    params: serde_json::json!({ "value": 0.92 }),
                })
                .build()?,
        )
        .clip(
            ClipBuilder::new("logo_glow", "logo", range)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(420.0, 160.0),
                    scale: Vec2::new(1.35, 1.35),
                    ..Transform2D::default()
                }))
                .opacity(Anim::constant(0.42))
                .effect(EffectInstance {
                    kind: "blur".to_string(),
                    params: serde_json::json!({ "radius_px": 10, "sigma": 4.5 }),
                })
                .build()?,
        )
        .build()?;

    let text_track = TrackBuilder::new("text")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new("headline", "title", range)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(145.0, 560.0),
                    ..Transform2D::default()
                }))
                .effect(EffectInstance {
                    kind: "opacity_mul".to_string(),
                    params: serde_json::json!({ "value": 0.95 }),
                })
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
    .seed(11)
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
        "title",
        Asset::Text(TextAsset {
            text: "AESTHETIC UNLOCK II: FX STACK".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 56.0,
            max_width_px: Some(1050.0),
            color_rgba8: [245, 245, 255, 255],
        }),
    )?
    .track(bg_track)
    .track(logo_track)
    .track(text_track)
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
        "aesthetic_fx.mp4",
        [22, 18, 28, 255],
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
