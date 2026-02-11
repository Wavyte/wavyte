use wavyte::{
    Anim, Asset, Canvas, ClipBuilder, CompositionBuilder, Fps, FrameIndex, FrameRange, TextAsset,
    TrackBuilder, Transform2D, Vec2,
};

fn main() -> anyhow::Result<()> {
    let clip = ClipBuilder::new("c0", "t0", FrameRange::new(FrameIndex(0), FrameIndex(60))?)
        .opacity(Anim::constant(1.0))
        .transform(Anim::constant(Transform2D {
            translate: Vec2::new(10.0, 20.0),
            ..Transform2D::default()
        }))
        .build()?;

    let track = TrackBuilder::new("main").clip(clip).build()?;

    let comp = CompositionBuilder::new(
        Fps::new(30, 1)?,
        Canvas {
            width: 640,
            height: 360,
        },
        FrameIndex(60),
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
    )?
    .track(track)
    .build()?;

    println!("{}", serde_json::to_string_pretty(&comp)?);
    Ok(())
}
