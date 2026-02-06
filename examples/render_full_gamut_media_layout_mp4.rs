#[path = "support/mod.rs"]
mod support;

use std::path::Path;

use wavyte::{
    Anim, Asset, AudioAsset, ClipBuilder, Composition, CompositionBuilder, Fps, FrameIndex,
    FrameRange, ImageAsset, LayoutAlignX, LayoutAlignY, LayoutMode, PathAsset, RenderThreading,
    SvgAsset, TextAsset, TrackBuilder, Transform2D, Vec2, VideoAsset,
};

fn build_comp() -> anyhow::Result<Composition> {
    let duration = FrameIndex(150); // 5s @ 30fps
    let full = FrameRange::new(FrameIndex(0), duration)?;

    let media_grid = TrackBuilder::new("media_grid")
        .layout_mode(LayoutMode::Grid)
        .layout_grid_columns(2)
        .layout_gap_px(20.0)
        .layout_padding(wavyte::Edges {
            left: 40.0,
            right: 40.0,
            top: 120.0,
            bottom: 100.0,
        })
        .layout_align(LayoutAlignX::Center, LayoutAlignY::Center)
        .clip(
            ClipBuilder::new("video_card", "test_video", full)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(1.0, 1.0),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("image_card", "still", full)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(0.34, 0.34),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("logo_card", "logo", full)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(0.9, 0.9),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("badge_card", "badge", full)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(2.0, 2.0),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .build()?;

    let title_stack = TrackBuilder::new("title_stack")
        .layout_mode(LayoutMode::VStack)
        .layout_gap_px(4.0)
        .layout_padding(wavyte::Edges {
            left: 0.0,
            right: 0.0,
            top: 22.0,
            bottom: 0.0,
        })
        .layout_align(LayoutAlignX::Center, LayoutAlignY::Start)
        .clip(ClipBuilder::new("headline_clip", "headline", full).build()?)
        .clip(ClipBuilder::new("subtitle_clip", "subtitle", full).build()?)
        .build()?;

    let accents = TrackBuilder::new("accents")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new("accent_bar", "bar", full)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(80.0, 82.0),
                    scale: Vec2::new(1.0, 1.0),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("accent_stamp", "stamp", full)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(840.0, 650.0),
                    rotation_rad: -0.08,
                    ..Transform2D::default()
                }))
                .opacity(Anim::constant(0.86))
                .build()?,
        )
        .build()?;

    let audio_track = TrackBuilder::new("audio")
        .layout_mode(LayoutMode::Absolute)
        .clip(
            ClipBuilder::new("music_clip", "music", full)
                .opacity(Anim::constant(1.0))
                .transform(Anim::constant(Transform2D::default()))
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
    .seed(41)
    .asset(
        "test_video",
        Asset::Video(VideoAsset {
            source: "assets/test_video.mp4".to_string(),
            trim_start_sec: 0.0,
            trim_end_sec: Some(4.8),
            playback_rate: 1.0,
            volume: 1.0,
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            muted: false,
        }),
    )?
    .asset(
        "music",
        Asset::Audio(AudioAsset {
            source: "assets/test_audio.mp3".to_string(),
            trim_start_sec: 0.0,
            trim_end_sec: Some(4.9),
            playback_rate: 1.0,
            volume: 0.45,
            fade_in_sec: 0.3,
            fade_out_sec: 0.45,
            muted: false,
        }),
    )?
    .asset(
        "still",
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
        "headline",
        Asset::Text(TextAsset {
            text: "FULL CREATIVE GAMUT".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 52.0,
            max_width_px: Some(1200.0),
            color_rgba8: [245, 245, 242, 255],
        }),
    )?
    .asset(
        "subtitle",
        Asset::Text(TextAsset {
            text: "Video + Audio + Layout + Overlays (v0.2)".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 28.0,
            max_width_px: Some(1200.0),
            color_rgba8: [240, 220, 145, 255],
        }),
    )?
    .asset(
        "badge",
        Asset::Path(PathAsset {
            svg_path_d: "M0,30 C0,13.4 13.4,0 30,0 L310,0 C326.6,0 340,13.4 340,30 C340,46.6 326.6,60 310,60 L30,60 C13.4,60 0,46.6 0,30 Z".to_string(),
        }),
    )?
    .asset(
        "bar",
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L1120,0 L1120,8 L0,8 Z".to_string(),
        }),
    )?
    .asset(
        "stamp",
        Asset::Text(TextAsset {
            text: "MEDIA-FFMPEG DEMO".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 30.0,
            max_width_px: Some(420.0),
            color_rgba8: [250, 197, 92, 255],
        }),
    )?
    .track(media_grid)
    .track(title_stack)
    .track(accents)
    .track(audio_track)
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
    if !cfg!(feature = "media-ffmpeg") {
        anyhow::bail!(
            "this example requires the 'media-ffmpeg' feature; run with: cargo run --features media-ffmpeg --example render_full_gamut_media_layout_mp4"
        );
    }

    support::ensure_ffmpeg_tools()?;
    support::ensure_test_video_mp4(
        Path::new("assets/test_video.mp4"),
        Path::new("assets/test_image_1.jpg"),
    )?;

    let comp = build_comp()?;
    let out = support::render_comp_to_assets_mp4(
        &comp,
        "full_gamut_media_layout.mp4",
        [14, 16, 24, 255],
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
