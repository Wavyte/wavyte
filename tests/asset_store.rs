use std::io::Cursor;

use wavyte::{
    Asset, Canvas, Composition, Fps, FrameIndex, ImageAsset, PathAsset, PreparedAsset,
    PreparedAssetStore, normalize_rel_path,
};

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
fn prepare_loads_image_asset() {
    let tmp = temp_dir("asset_store_prepare_image");
    std::fs::create_dir_all(&tmp).unwrap();

    let png_path = tmp.join("img.png");
    let img = image::RgbaImage::from_raw(1, 1, vec![1u8, 2u8, 3u8, 255u8]).unwrap();
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&png_path, &buf).unwrap();

    let mut assets = std::collections::BTreeMap::new();
    assets.insert(
        "img".to_string(),
        Asset::Image(ImageAsset {
            source: "img.png".to_string(),
        }),
    );
    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 1,
            height: 1,
        },
        duration: FrameIndex(1),
        assets,
        tracks: vec![],
        seed: 1,
    };

    let store = PreparedAssetStore::prepare(&comp, &tmp).unwrap();
    let id = store.id_for_key("img").unwrap();
    let prepared = store.get(id).unwrap();
    let PreparedAsset::Image(image) = prepared else {
        panic!("expected image asset");
    };
    assert_eq!(image.width, 1);
    assert_eq!(image.height, 1);

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn normalize_path_cross_platform() {
    assert_eq!(normalize_rel_path("a/b.png").unwrap(), "a/b.png");
    assert_eq!(normalize_rel_path("a\\b.png").unwrap(), "a/b.png");
    assert!(normalize_rel_path("../x.png").is_err());
}

#[test]
fn prepare_loads_inline_path_without_external_io() {
    let mut assets = std::collections::BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 Z".to_string(),
        }),
    );
    let comp = Composition {
        fps: Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 32,
            height: 32,
        },
        duration: FrameIndex(1),
        assets,
        tracks: vec![],
        seed: 1,
    };

    let store = PreparedAssetStore::prepare(&comp, ".").unwrap();
    let id = store.id_for_key("p0").unwrap();
    let prepared = store.get(id).unwrap();
    let PreparedAsset::Path(path) = prepared else {
        panic!("expected path asset");
    };
    assert!(!path.path.is_empty());
}
