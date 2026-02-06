use std::collections::BTreeMap;
use std::path::PathBuf;

use wavyte::{
    Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, Fps, FrameIndex,
    FrameRange, InterpMode, Keyframe, Keyframes, RenderSettings, RenderToMp4Opts, SvgAsset, Track,
    Transform2D, Vec2, create_backend, render_to_mp4,
};

fn parse_backend() -> anyhow::Result<BackendKind> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("cpu") | None => Ok(BackendKind::Cpu),
        Some(other) => anyhow::bail!("unknown backend '{other}', only 'cpu' is supported"),
    }
}

fn build_comp() -> Composition {
    let fps = Fps::new(30, 1).unwrap();
    let duration = FrameIndex(60); // 2s @ 30fps

    let mut assets = BTreeMap::<String, Asset>::new();
    assets.insert(
        "logo".to_string(),
        Asset::Svg(SvgAsset {
            source: "assets/logo.svg".to_string(),
        }),
    );

    let fade = Anim::Keyframes(Keyframes {
        // fade-in: 0..15, hold: 15..45, fade-out: 45..60
        keys: vec![
            Keyframe {
                frame: FrameIndex(0),
                value: 0.0,
                ease: wavyte::Ease::Linear,
            },
            Keyframe {
                frame: FrameIndex(15),
                value: 1.0,
                ease: wavyte::Ease::Linear,
            },
            Keyframe {
                frame: FrameIndex(45),
                value: 1.0,
                ease: wavyte::Ease::Linear,
            },
            Keyframe {
                frame: FrameIndex(60),
                value: 0.0,
                ease: wavyte::Ease::Linear,
            },
        ],
        mode: InterpMode::Linear,
        default: None,
    });

    Composition {
        fps,
        canvas: Canvas {
            width: 1280,
            height: 720,
        },
        duration,
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            clips: vec![Clip {
                id: "logo".to_string(),
                asset: "logo".to_string(),
                range: FrameRange::new(FrameIndex(0), duration).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D {
                        // Center a 512x512 SVG on a 1280x720 canvas.
                        translate: Vec2::new(384.0, 104.0),
                        scale: Vec2::new(1.0, 1.0),
                        ..Transform2D::default()
                    }),
                    opacity: fade,
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![],
                transition_in: None,
                transition_out: None,
            }],
        }],
        seed: 1,
    }
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("{err:#}");
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

    let out_dir = PathBuf::from("assets");
    std::fs::create_dir_all(&out_dir)?;
    let out_mp4 = out_dir.join("svg_fade_sample.mp4");

    render_to_mp4(
        &comp,
        &out_mp4,
        RenderToMp4Opts {
            range: FrameRange::new(FrameIndex(0), comp.duration)?,
            bg_rgba: settings.clear_rgba.unwrap_or([0, 0, 0, 255]),
            overwrite: true,
        },
        backend.as_mut(),
        &assets,
    )?;

    eprintln!("wrote {}", out_mp4.display());
    Ok(())
}
