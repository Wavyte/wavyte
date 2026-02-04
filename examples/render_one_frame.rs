#[cfg(any(feature = "cpu", feature = "gpu"))]
use std::collections::BTreeMap;

#[cfg(any(feature = "cpu", feature = "gpu"))]
use wavyte::{
    Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, FrameIndex,
    FrameRange, FsAssetCache, ImageAsset, PathAsset, RenderSettings, SvgAsset, TextAsset, Track,
    Transform2D, create_backend, render_frame,
};

#[cfg(any(feature = "cpu", feature = "gpu"))]
fn first_asset_path_with_ext(ext: &str) -> Option<String> {
    let dir = std::path::Path::new("assets");
    let rd = std::fs::read_dir(dir).ok()?;

    let mut names = Vec::<String>::new();
    for ent in rd {
        let ent = ent.ok()?;
        if !ent.file_type().ok()?.is_file() {
            continue;
        }
        let path = ent.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.first().map(|n| format!("assets/{n}"))
}

#[cfg(any(feature = "cpu", feature = "gpu"))]
fn build_comp() -> Composition {
    let mut assets = BTreeMap::<String, Asset>::new();

    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L120,0 L120,120 L0,120 Z".to_string(),
        }),
    );

    if let Some(jpg) = first_asset_path_with_ext("jpg").or_else(|| first_asset_path_with_ext("png"))
    {
        assets.insert("img0".to_string(), Asset::Image(ImageAsset { source: jpg }));
    }

    if let Some(svg) = first_asset_path_with_ext("svg") {
        assets.insert("svg0".to_string(), Asset::Svg(SvgAsset { source: svg }));
    }

    if let Some(font) =
        first_asset_path_with_ext("ttf").or_else(|| first_asset_path_with_ext("otf"))
    {
        assets.insert(
            "txt0".to_string(),
            Asset::Text(TextAsset {
                text: "wavyte v0.1.0".to_string(),
                font_source: font,
                size_px: 42.0,
                max_width_px: Some(480.0),
                color_rgba8: [255, 255, 255, 255],
            }),
        );
    }

    let dur = FrameIndex(1);
    let range = FrameRange::new(FrameIndex(0), dur).unwrap();

    let mut clips = Vec::<Clip>::new();

    clips.push(Clip {
        id: "c_path".to_string(),
        asset: "p0".to_string(),
        range,
        props: ClipProps {
            transform: Anim::constant(Transform2D {
                translate: wavyte::Vec2::new(64.0, 64.0),
                scale: wavyte::Vec2::new(3.0, 3.0),
                ..Transform2D::default()
            }),
            opacity: Anim::constant(0.9),
            blend: BlendMode::Normal,
        },
        z_offset: 0,
        effects: vec![],
        transition_in: None,
        transition_out: None,
    });

    if assets.contains_key("img0") {
        clips.push(Clip {
            id: "c_img".to_string(),
            asset: "img0".to_string(),
            range,
            props: ClipProps {
                transform: Anim::constant(Transform2D {
                    translate: wavyte::Vec2::new(16.0, 16.0),
                    scale: wavyte::Vec2::new(1.0, 1.0),
                    ..Transform2D::default()
                }),
                opacity: Anim::constant(1.0),
                blend: BlendMode::Normal,
            },
            z_offset: 10,
            effects: vec![],
            transition_in: None,
            transition_out: None,
        });
    }

    if assets.contains_key("svg0") {
        clips.push(Clip {
            id: "c_svg".to_string(),
            asset: "svg0".to_string(),
            range,
            props: ClipProps {
                transform: Anim::constant(Transform2D {
                    translate: wavyte::Vec2::new(280.0, 40.0),
                    scale: wavyte::Vec2::new(1.0, 1.0),
                    ..Transform2D::default()
                }),
                opacity: Anim::constant(1.0),
                blend: BlendMode::Normal,
            },
            z_offset: 20,
            effects: vec![],
            transition_in: None,
            transition_out: None,
        });
    }

    if assets.contains_key("txt0") {
        clips.push(Clip {
            id: "c_txt".to_string(),
            asset: "txt0".to_string(),
            range,
            props: ClipProps {
                transform: Anim::constant(Transform2D {
                    translate: wavyte::Vec2::new(24.0, 440.0),
                    ..Transform2D::default()
                }),
                opacity: Anim::constant(1.0),
                blend: BlendMode::Normal,
            },
            z_offset: 30,
            effects: vec![],
            transition_in: None,
            transition_out: None,
        });
    }

    Composition {
        fps: wavyte::Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 512,
            height: 512,
        },
        duration: dur,
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            clips,
        }],
        seed: 1,
    }
}

#[cfg(any(feature = "cpu", feature = "gpu"))]
fn parse_backend() -> Option<&'static str> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("cpu") => Some("cpu"),
        Some("gpu") => Some("gpu"),
        _ => None,
    }
}

#[cfg(not(any(feature = "cpu", feature = "gpu")))]
fn main() {
    eprintln!("build with `--features cpu` and/or `--features gpu`");
}

#[cfg(any(feature = "cpu", feature = "gpu"))]
fn main() -> anyhow::Result<()> {
    let comp = build_comp();
    comp.validate()?;

    let settings = RenderSettings {
        clear_rgba: Some([18, 20, 28, 255]),
    };

    let kind = match parse_backend() {
        Some("cpu") => {
            #[cfg(feature = "cpu")]
            {
                BackendKind::Cpu
            }
            #[cfg(not(feature = "cpu"))]
            {
                anyhow::bail!("built without `cpu` feature")
            }
        }
        Some("gpu") => {
            #[cfg(feature = "gpu")]
            {
                BackendKind::Gpu
            }
            #[cfg(not(feature = "gpu"))]
            {
                anyhow::bail!("built without `gpu` feature")
            }
        }
        None => {
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
        _ => unreachable!(),
    };

    let mut backend = create_backend(kind, &settings)?;
    let mut assets = FsAssetCache::new(".");
    let frame = render_frame(&comp, FrameIndex(0), backend.as_mut(), &mut assets)?;

    let out_path = std::path::Path::new("target").join("render_one_frame.png");
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
