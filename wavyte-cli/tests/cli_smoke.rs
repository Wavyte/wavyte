use std::path::PathBuf;

#[test]
fn cli_frame_writes_png() {
    let dir = PathBuf::from("target").join("cli_smoke");
    std::fs::create_dir_all(&dir).unwrap();

    let comp_path = dir.join("comp.json");
    let out_path = dir.join("out.png");
    let _ = std::fs::remove_file(&out_path);

    let json = r##"
{
  "version": "0.3",
  "canvas": { "width": 64, "height": 64 },
  "fps": { "num": 30, "den": 1 },
  "duration": 2,
  "assets": {
    "solid": { "solid_rect": { "color": "#ff3366" } }
  },
  "root": {
    "id": "root",
    "kind": { "leaf": { "asset": "solid" } },
    "range": [0, 2]
  }
}
"##;
    std::fs::write(&comp_path, json).unwrap();

    let comp_arg = comp_path.to_string_lossy().to_string();
    let out_arg = out_path.to_string_lossy().to_string();
    let profile_dir = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let direct_bin = std::env::var_os("CARGO_BIN_EXE_wavyte")
        .map(PathBuf::from)
        .or_else(|| {
            let mut p = PathBuf::from("target").join(profile_dir);
            p.push(if cfg!(windows) {
                "wavyte.exe"
            } else {
                "wavyte"
            });
            if p.is_file() { Some(p) } else { None }
        });

    let status = if let Some(exe) = direct_bin {
        std::process::Command::new(exe)
            .args(["frame", "--in", comp_arg.as_str(), "--frame", "0", "--out"])
            .arg(out_arg.as_str())
            .status()
            .unwrap()
    } else {
        // Workspace fallback: invoke Cargo to run the dedicated CLI crate.
        let cargo = std::env::var_os("CARGO")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cargo"));
        std::process::Command::new(cargo)
            .args([
                "run",
                "-p",
                "wavyte-cli",
                "--bin",
                "wavyte",
                "--release",
                "--",
                "frame",
                "--in",
                comp_arg.as_str(),
                "--frame",
                "0",
                "--out",
                out_arg.as_str(),
            ])
            .status()
            .unwrap()
    };

    assert!(status.success());
    assert!(out_path.exists());
}
