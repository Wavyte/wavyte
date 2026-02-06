use std::collections::BTreeMap;

use std::path::PathBuf;

use wavyte::{
    Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, EffectInstance, Fps,
    FrameIndex, FrameRange, PathAsset, RenderSettings, Track, Transform2D, Vec2, create_backend,
    render_frame,
};

fn parse_backend() -> anyhow::Result<BackendKind> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("cpu") | None => Ok(BackendKind::Cpu),
        Some(other) => anyhow::bail!("unknown backend '{other}', only 'cpu' is supported"),
    }
}

fn build_comp() -> Composition {
    let mut assets = BTreeMap::<String, Asset>::new();
    assets.insert(
        "rect".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L120,0 L120,120 L0,120 Z".to_string(),
        }),
    );

    let fps = Fps::new(30, 1).unwrap();
    let dur = FrameIndex(1);
    let range = FrameRange::new(FrameIndex(0), dur).unwrap();

    Composition {
        fps,
        canvas: Canvas {
            width: 512,
            height: 512,
        },
        duration: dur,
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            layout_mode: wavyte::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: wavyte::Edges::default(),
            layout_align_x: wavyte::LayoutAlignX::Start,
            layout_align_y: wavyte::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: vec![Clip {
                id: "blurred_rect".to_string(),
                asset: "rect".to_string(),
                range,
                props: ClipProps {
                    transform: Anim::constant(Transform2D {
                        translate: Vec2::new(140.0, 140.0),
                        scale: Vec2::new(2.0, 2.0),
                        ..Transform2D::default()
                    }),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![EffectInstance {
                    kind: "blur".to_string(),
                    params: serde_json::json!({ "radius_px": 10, "sigma": 6.0 }),
                }],
                transition_in: None,
                transition_out: None,
            }],
        }],
        seed: 1,
    }
}

fn main() {
    if let Err(e) = try_main() {
        eprintln!("{e:?}");
        std::process::exit(1);
    }
}

fn try_main() -> anyhow::Result<()> {
    let comp = build_comp();
    comp.validate()?;

    let settings = RenderSettings {
        clear_rgba: Some([18, 20, 28, 255]),
    };
    let mut backend = create_backend(parse_backend()?, &settings)?;
    let assets = wavyte::PreparedAssetStore::prepare(&comp, ".")?;

    let frame = render_frame(&comp, FrameIndex(0), backend.as_mut(), &assets)?;

    let out_dir = PathBuf::from("assets");
    std::fs::create_dir_all(&out_dir)?;
    let out_path = out_dir.join("out_blur.png");

    image::save_buffer_with_format(
        &out_path,
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;

    eprintln!("wrote {}", out_path.display());
    Ok(())
}
