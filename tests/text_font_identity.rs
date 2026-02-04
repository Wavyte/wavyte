#[test]
fn prepared_text_exposes_font_family_from_bytes() {
    let font_bytes = std::fs::read("tests/data/fonts/Inconsolata-Regular.ttf").unwrap();

    let mut engine = wavyte::TextLayoutEngine::new();
    let _layout = engine
        .layout_plain(
            "wavyte",
            font_bytes.as_slice(),
            32.0,
            wavyte::TextBrushRgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            None,
        )
        .unwrap();

    let family = engine.last_family_name().unwrap();
    assert!(!family.trim().is_empty());
}
