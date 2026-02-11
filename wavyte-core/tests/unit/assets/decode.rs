use std::io::Cursor;

use super::*;

#[test]
fn decode_image_png_dimensions_and_premul() {
    let src_rgba = vec![100u8, 50u8, 200u8, 128u8];
    let img = image::RgbaImage::from_raw(1, 1, src_rgba).unwrap();

    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();

    let prepared = decode_image(&buf).unwrap();
    assert_eq!(prepared.width, 1);
    assert_eq!(prepared.height, 1);
    assert_eq!(
        prepared.rgba8_premul.as_slice(),
        &[
            ((100u16 * 128 + 127) / 255) as u8,
            ((50u16 * 128 + 127) / 255) as u8,
            ((200u16 * 128 + 127) / 255) as u8,
            128u8
        ]
    );
}

#[test]
fn decode_svg_parse_ok_and_err() {
    let ok = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#;
    parse_svg(ok).unwrap();

    let bad = br#"<svg"#;
    assert!(parse_svg(bad).is_err());
}
