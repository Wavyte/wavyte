use std::sync::Arc;

use anyhow::Context;

use crate::{
    WavyteResult,
    assets::store::{PreparedImage, PreparedSvg},
};

/// Decode encoded image bytes and convert to premultiplied RGBA8.
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

/// Parse SVG bytes into a prepared `usvg` tree.
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
#[path = "../../tests/unit/assets/decode.rs"]
mod tests;
