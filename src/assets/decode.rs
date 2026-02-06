use std::sync::Arc;

use anyhow::Context;

use crate::{
    WavyteResult,
    asset_store::{PreparedImage, PreparedSvg},
};

pub fn decode_image(bytes: &[u8]) -> WavyteResult<PreparedImage> {
    let dyn_img = image::load_from_memory(bytes).context("decode image from memory")?;
    let rgba = dyn_img.to_rgba8();
    let (width, height) = rgba.dimensions();

    let mut rgba8_premul = rgba.into_raw();
    premultiply_rgba8_in_place(&mut rgba8_premul);

    Ok(PreparedImage {
        width,
        height,
        rgba8_premul: Arc::new(rgba8_premul),
    })
}

pub fn parse_svg(bytes: &[u8]) -> WavyteResult<PreparedSvg> {
    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opts).context("parse svg tree")?;
    Ok(PreparedSvg {
        tree: Arc::new(tree),
    })
}

fn premultiply_rgba8_in_place(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u16;
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }
        px[0] = ((px[0] as u16 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u16 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u16 * a + 127) / 255) as u8;
    }
}

#[cfg(test)]
mod tests {
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
}
