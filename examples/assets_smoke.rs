use std::path::Path;

use wavyte::{Asset, AssetCache, FsAssetCache, ImageAsset, PreparedAsset, SvgAsset, TextAsset};

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

    let mut cache = FsAssetCache::new(".");

    let img = Asset::Image(ImageAsset {
        source: "assets/test_image_1.jpg".to_string(),
    });
    let svg = Asset::Svg(SvgAsset {
        source: "assets/logo.svg".to_string(),
    });
    let text = Asset::Text(TextAsset {
        text: "Hello, Wavyte!".to_string(),
        font_source: "assets/PlayfairDisplay.ttf".to_string(),
        size_px: 48.0,
        max_width_px: Some(512.0),
        color_rgba8: [255, 255, 255, 255],
    });

    for (name, asset) in [("image", img), ("svg", svg), ("text", text)] {
        let prepared = cache.get_or_load(&asset)?;
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
        }
    }

    Ok(())
}
