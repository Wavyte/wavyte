use super::*;

#[test]
fn normalize_path_slash_normalization() {
    assert_eq!(normalize_rel_path("a/b.png").unwrap(), "a/b.png");
    assert_eq!(normalize_rel_path("a\\b.png").unwrap(), "a/b.png");
    assert!(normalize_rel_path("../x.png").is_err());
    assert!(normalize_rel_path("/abs.png").is_err());
    assert!(normalize_rel_path("").is_err());
    assert!(normalize_rel_path(".").is_err());
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
