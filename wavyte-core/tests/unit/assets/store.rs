use std::collections::BTreeMap;
use std::io::Cursor;

use super::*;

#[test]
fn normalize_path_slash_normalization() {
    assert_eq!(normalize_rel_path("a/b.png").unwrap(), "a/b.png");
    assert_eq!(normalize_rel_path("a\\b.png").unwrap(), "a/b.png");
    assert!(normalize_rel_path("../x.png").is_err());
    assert!(normalize_rel_path("/abs.png").is_err());
}

#[test]
fn asset_id_stability_same_input() {
    let key = AssetKey::new(
        "assets/img.png".to_string(),
        vec![
            ("dpi".to_string(), "96".to_string()),
            ("colorspace".to_string(), "srgb".to_string()),
        ],
    );

    let a = PreparedAssetStore::hash_id_for_key(b'I', &key);
    let b = PreparedAssetStore::hash_id_for_key(b'I', &key);
    assert_eq!(a, b);
    assert_eq!(a.0, 0xa23b14b8777d9f73);
}

#[test]
fn prepare_path_assets_without_external_io() {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        model::Asset::Path(model::PathAsset {
            svg_path_d: "M0,0 L10,0 L10,10 Z".to_string(),
        }),
    );

    let comp = model::Composition {
        fps: crate::Fps::new(30, 1).unwrap(),
        canvas: crate::Canvas {
            width: 64,
            height: 64,
        },
        duration: crate::FrameIndex(1),
        assets,
        tracks: vec![],
        seed: 1,
    };

    let store = PreparedAssetStore::prepare(&comp, ".").unwrap();
    let id = store.id_for_key("p0").unwrap();
    let prepared = store.get(id).unwrap();
    let PreparedAsset::Path(p) = prepared else {
        panic!("expected prepared path");
    };
    assert!(!p.path.is_empty());
}

#[test]
fn text_layout_smoke_with_local_font_if_present() {
    let font_path = std::path::Path::new("assets/PlayfairDisplay.ttf");
    let Ok(font_bytes) = std::fs::read(font_path) else {
        return;
    };

    let mut engine = TextLayoutEngine::new();
    let layout = engine
        .layout_plain(
            "hello",
            font_bytes.as_slice(),
            48.0,
            TextBrushRgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            None,
        )
        .unwrap();

    assert!(layout.lines().next().is_some());
}

#[test]
fn prepare_single_image_asset() {
    let tmp = std::env::temp_dir().join(format!(
        "wavyte_asset_store_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    let png_path = tmp.join("img.png");
    let img = image::RgbaImage::from_raw(1, 1, vec![1u8, 2u8, 3u8, 255u8]).unwrap();
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&png_path, &buf).unwrap();

    let mut assets = BTreeMap::new();
    assets.insert(
        "img".to_string(),
        model::Asset::Image(model::ImageAsset {
            source: "img.png".to_string(),
        }),
    );

    let comp = model::Composition {
        fps: crate::Fps::new(30, 1).unwrap(),
        canvas: crate::Canvas {
            width: 1,
            height: 1,
        },
        duration: crate::FrameIndex(1),
        assets,
        tracks: vec![],
        seed: 1,
    };

    let store = PreparedAssetStore::prepare(&comp, &tmp).unwrap();
    let id = store.id_for_key("img").unwrap();
    let prepared = store.get(id).unwrap();
    let PreparedAsset::Image(p) = prepared else {
        panic!("expected prepared image");
    };
    assert_eq!(p.width, 1);
    assert_eq!(p.height, 1);

    std::fs::remove_dir_all(&tmp).ok();
}
