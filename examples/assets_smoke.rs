use std::path::Path;

use wavyte::{
    Asset, Canvas, Composition, Fps, FrameIndex, PreparedAsset, PreparedAssetStore, SvgAsset,
    TextAsset,
};

fn main() -> anyhow::Result<()> {
    let required = [
        "assets/logo.svg",
        "assets/test_image_1.jpg",
        "assets/PlayfairDisplay.ttf",
    ];
    let missing: Vec<&str> = required
        .into_iter()
        .filter(|p| !Path::new(p).exists())
        .collect();
    if !missing.is_empty() {
        eprintln!("assets_smoke: missing local files: {missing:?}");
        eprintln!("assets_smoke: skipping (assets/ is intentionally untracked)");
        return Ok(());
    }

    let mut assets = std::collections::BTreeMap::new();
    assets.insert(
        "image".to_string(),
        Asset::Image(wavyte::ImageAsset {
            source: "assets/test_image_1.jpg".to_string(),
        }),
    );
    assets.insert(
        "svg".to_string(),
        Asset::Svg(SvgAsset {
            source: "assets/logo.svg".to_string(),
        }),
    );
    assets.insert(
        "text".to_string(),
        Asset::Text(TextAsset {
            text: "Hello, Wavyte!".to_string(),
            font_source: "assets/PlayfairDisplay.ttf".to_string(),
            size_px: 48.0,
            max_width_px: Some(512.0),
            color_rgba8: [255, 255, 255, 255],
        }),
    );

    let comp = Composition {
        fps: Fps::new(30, 1)?,
        canvas: Canvas {
            width: 640,
            height: 360,
        },
        duration: FrameIndex(1),
        assets,
        tracks: vec![],
        seed: 1,
    };
    let store = PreparedAssetStore::prepare(&comp, ".")?;

    for name in ["image", "svg", "text"] {
        let id = store.id_for_key(name)?;
        let prepared = store.get(id)?;
        match prepared {
            PreparedAsset::Image(i) => {
                println!(
                    "{name}: {}x{} ({} bytes)",
                    i.width,
                    i.height,
                    i.rgba8_premul.len()
                );
            }
            PreparedAsset::Svg(s) => {
                let _ = s;
                println!("{name}: svg tree OK");
            }
            PreparedAsset::Text(t) => {
                let lines = t.layout.lines().count();
                println!("{name}: layout OK ({lines} lines)");
            }
            PreparedAsset::Path(_) => {
                println!("{name}: path asset");
            }
        }
    }

    Ok(())
}
