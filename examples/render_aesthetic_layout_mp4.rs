#[path = "support/mod.rs"]
mod support;

use wavyte::{
    Anim, Asset, ClipBuilder, Composition, CompositionBuilder, Fps, FrameIndex, FrameRange,
    ImageAsset, LayoutAlignX, LayoutAlignY, LayoutMode, PathAsset, RenderThreading, SvgAsset,
    TextAsset, TrackBuilder, Transform2D, Vec2,
};

fn build_comp() -> anyhow::Result<Composition> {
    let duration = FrameIndex(150); // 5s @ 30fps
    let range = FrameRange::new(FrameIndex(0), duration)?;

    let grid_track = TrackBuilder::new("grid")
        .layout_mode(LayoutMode::Grid)
        .layout_grid_columns(2)
        .layout_gap_px(24.0)
        .layout_padding(wavyte::Edges {
            left: 70.0,
            right: 70.0,
            top: 120.0,
            bottom: 200.0,
        })
        .layout_align(LayoutAlignX::Center, LayoutAlignY::Center)
        .clip(
            ClipBuilder::new("grid_img", "img", range)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(0.30, 0.30),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("grid_logo", "logo", range)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(0.88, 0.88),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("grid_badge", "badge", range)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(1.75, 1.75),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .clip(
            ClipBuilder::new("grid_title", "title", range)
                .transform(Anim::constant(Transform2D {
                    scale: Vec2::new(1.0, 1.0),
                    ..Transform2D::default()
                }))
                .build()?,
        )
        .build()?;

    let row_track = TrackBuilder::new("row")
        .layout_mode(LayoutMode::HStack)
        .layout_gap_px(18.0)
        .layout_padding(wavyte::Edges {
            left: 96.0,
            right: 96.0,
            top: 620.0,
            bottom: 24.0,
        })
        .layout_align(LayoutAlignX::Center, LayoutAlignY::Center)
        .clip(ClipBuilder::new("row_a", "chip_a", range).build()?)
        .clip(ClipBuilder::new("row_b", "chip_b", range).build()?)
        .clip(ClipBuilder::new("row_c", "chip_c", range).build()?)
        .build()?;

    let center_track = TrackBuilder::new("center")
        .layout_mode(LayoutMode::Center)
        .clip(
            ClipBuilder::new("center_stamp", "stamp", range)
                .transform(Anim::constant(Transform2D {
                    translate: Vec2::new(0.0, -290.0),
                    ..Transform2D::default()
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
    .seed(19)
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
            text: "AESTHETIC UNLOCK III: LAYOUT SYSTEMS".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 48.0,
            max_width_px: Some(520.0),
            color_rgba8: [240, 238, 232, 255],
        }),
    )?
    .asset(
        "badge",
        Asset::Path(PathAsset {
            svg_path_d: "M0,30 C0,13.4 13.4,0 30,0 L220,0 C236.6,0 250,13.4 250,30 C250,46.6 236.6,60 220,60 L30,60 C13.4,60 0,46.6 0,30 Z".to_string(),
        }),
    )?
    .asset(
        "chip_a",
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L180,0 L180,42 L0,42 Z".to_string(),
        }),
    )?
    .asset(
        "chip_b",
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L180,0 L180,42 L0,42 Z".to_string(),
        }),
    )?
    .asset(
        "chip_c",
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L180,0 L180,42 L0,42 Z".to_string(),
        }),
    )?
    .asset(
        "stamp",
        Asset::Text(TextAsset {
            text: "GRID + HSTACK + CENTER".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 34.0,
            max_width_px: Some(840.0),
            color_rgba8: [252, 214, 101, 255],
        }),
    )?
    .track(grid_track)
    .track(row_track)
    .track(center_track)
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
        "aesthetic_layout.mp4",
        [10, 18, 26, 255],
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
