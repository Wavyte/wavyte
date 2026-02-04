#[cfg(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu")))]
use std::collections::BTreeMap;

#[cfg(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu")))]
use std::path::PathBuf;

#[cfg(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu")))]
use wavyte::{
    Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, Fps, FrameIndex,
    FrameRange, PathAsset, RenderSettings, RenderToMp4Opts, Track, Transform2D, TransitionSpec,
    Vec2, create_backend, render_frame, render_to_mp4,
};

#[cfg(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu")))]
fn parse_backend() -> BackendKind {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("gpu") => {
            #[cfg(feature = "gpu")]
            {
                BackendKind::Gpu
            }
            #[cfg(not(feature = "gpu"))]
            {
                BackendKind::Cpu
            }
        }
        _ => {
            #[cfg(feature = "cpu")]
            {
                BackendKind::Cpu
            }
            #[cfg(all(not(feature = "cpu"), feature = "gpu"))]
            {
                BackendKind::Gpu
            }
            #[cfg(all(not(feature = "cpu"), not(feature = "gpu")))]
            {
                unreachable!()
            }
        }
    }
}

#[cfg(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu")))]
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
    let duration = FrameIndex(60);

    let a_range = FrameRange::new(FrameIndex(0), FrameIndex(60)).unwrap();
    let b_range = FrameRange::new(FrameIndex(30), FrameIndex(60)).unwrap();

    let tr = TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 30,
        ease: wavyte::Ease::Linear,
        params: serde_json::Value::Null,
    };

    Composition {
        fps,
        canvas: Canvas {
            width: 512,
            height: 512,
        },
        duration,
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

#[cfg(not(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu"))))]
fn main() {
    eprintln!("build with `--features ffmpeg,cpu` (and optionally `gpu`) to run this example");
}

#[cfg(all(feature = "ffmpeg", any(feature = "cpu", feature = "gpu")))]
fn main() -> anyhow::Result<()> {
    let comp = build_comp();
    comp.validate()?;

    let out_dir = PathBuf::from("assets");
    std::fs::create_dir_all(&out_dir)?;
    let out_mp4 = out_dir.join("out.mp4");
    let out_png = out_dir.join("out_frame.png");

    let settings = RenderSettings {
        clear_rgba: Some([18, 20, 28, 255]),
    };
    let mut backend = create_backend(parse_backend(), &settings)?;
    let mut assets = wavyte::FsAssetCache::new(".");

    // Write a single frame PNG for quick sanity checking.
    let frame = render_frame(&comp, FrameIndex(45), backend.as_mut(), &mut assets)?;
    image::save_buffer_with_format(
        &out_png,
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;

    // Render the full composition to MP4.
    render_to_mp4(
        &comp,
        &out_mp4,
        RenderToMp4Opts {
            range: FrameRange::new(FrameIndex(0), comp.duration)?,
            bg_rgba: settings.clear_rgba.unwrap_or([0, 0, 0, 255]),
            overwrite: true,
        },
        backend.as_mut(),
        &mut assets,
    )?;

    eprintln!("wrote {}", out_png.display());
    eprintln!("wrote {}", out_mp4.display());
    Ok(())
}
