use std::collections::BTreeMap;

use std::path::PathBuf;

use wavyte::{
    Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, Fps, FrameIndex,
    FrameRange, PathAsset, RenderSettings, Track, Transform2D, TransitionSpec, Vec2,
    create_backend, render_frame,
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
    assets.insert(
        "tri".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M60,0 L120,120 L0,120 Z".to_string(),
        }),
    );

    let fps = Fps::new(30, 1).unwrap();
    let dur = FrameIndex(16);

    let a_range = FrameRange::new(FrameIndex(0), FrameIndex(10)).unwrap();
    let b_range = FrameRange::new(FrameIndex(5), FrameIndex(15)).unwrap();

    let tr = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 5,
        ease: wavyte::Ease::Linear,
        params: serde_json::Value::Null,
    };

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
            clips: vec![
                Clip {
                    id: "a_rect".to_string(),
                    asset: "rect".to_string(),
                    range: a_range,
                    props: ClipProps {
                        transform: Anim::constant(Transform2D {
                            translate: Vec2::new(80.0, 180.0),
                            scale: Vec2::new(2.5, 2.5),
                            ..Transform2D::default()
                        }),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: Some(tr.clone()),
                },
                Clip {
                    id: "b_tri".to_string(),
                    asset: "tri".to_string(),
                    range: b_range,
                    props: ClipProps {
                        transform: Anim::constant(Transform2D {
                            translate: Vec2::new(260.0, 120.0),
                            scale: Vec2::new(2.5, 2.5),
                            ..Transform2D::default()
                        }),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 1,
                    effects: vec![],
                    transition_in: Some(tr),
                    transition_out: None,
                },
            ],
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

    // Middle of the overlap between A (out) and B (in): should pair into Crossfade op.
    let frame = render_frame(&comp, FrameIndex(7), backend.as_mut(), &assets)?;

    let out_dir = PathBuf::from("assets");
    std::fs::create_dir_all(&out_dir)?;
    let out_path = out_dir.join("out_frame.png");

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
