use std::io::Cursor;

use wavyte::{Asset, AssetCache, FsAssetCache, ImageAsset};

fn temp_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "wavyte_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn cache_load_same_asset_only_decodes_once() {
    let tmp = temp_dir("assets_cache_decode_once");
    std::fs::create_dir_all(&tmp).unwrap();

    let png_path = tmp.join("img.png");
    let img = image::RgbaImage::from_raw(1, 1, vec![1u8, 2u8, 3u8, 255u8]).unwrap();
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&png_path, &buf).unwrap();

    let mut cache = FsAssetCache::new(&tmp);
    let asset = Asset::Image(ImageAsset {
        source: "img.png".to_string(),
    });
    let id = cache.id_for(&asset).unwrap();
    cache.get_or_load(&asset).unwrap();
    cache.get_or_load(&asset).unwrap();
    assert_eq!(cache.decode_count(id), 1);

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn cache_cross_platform_keys() {
    let mut cache = FsAssetCache::new(".");
    let a = Asset::Image(ImageAsset {
        source: "a/b.png".to_string(),
    });
    let b = Asset::Image(ImageAsset {
        source: "a\\b.png".to_string(),
    });

    assert_eq!(cache.id_for(&a).unwrap(), cache.id_for(&b).unwrap());
    assert_eq!(cache.normalize_source("a/b.png").unwrap(), "a/b.png");
    assert_eq!(cache.normalize_source("a\\b.png").unwrap(), "a/b.png");
}
