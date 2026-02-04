use std::path::PathBuf;

use wavyte::{
    Anim, Asset, BlendMode, Canvas, Clip, ClipProps, Composition, Fps, FrameIndex, FrameRange,
    PathAsset, Track, Transform2D,
};

#[test]
fn cli_frame_writes_png() {
    let dir = PathBuf::from("target").join("cli_smoke");
    std::fs::create_dir_all(&dir).unwrap();

    let comp_path = dir.join("comp.json");
    let out_path = dir.join("out.png");
    let _ = std::fs::remove_file(&out_path);

    let mut assets = std::collections::BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L40,0 L40,40 L0,40 Z".to_string(),
        }),
    );

    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 64,
            height: 64,
        },
        duration: FrameIndex(2),
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "p0".to_string(),
                range: FrameRange::new(FrameIndex(0), FrameIndex(2)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D::default()),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![],
                transition_in: None,
                transition_out: None,
            }],
        }],
        seed: 1,
    };

    let f = std::fs::File::create(&comp_path).unwrap();
    serde_json::to_writer_pretty(f, &comp).unwrap();

    let exe = std::env::var_os("CARGO_BIN_EXE_wavyte")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = PathBuf::from("target").join("debug");
            p.push(if cfg!(windows) {
                "wavyte.exe"
            } else {
                "wavyte"
            });
            p
        });

    let comp_arg = comp_path.to_string_lossy().to_string();
    let out_arg = out_path.to_string_lossy().to_string();

    let status = std::process::Command::new(exe)
        .args(["frame", "--in", comp_arg.as_str(), "--frame", "0", "--out"])
        .arg(out_arg.as_str())
        .status()
        .unwrap();

    assert!(status.success());
    assert!(out_path.exists());
}
