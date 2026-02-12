use crate::foundation::error::{WavyteError, WavyteResult};

/// Compute a conservative raster size for an SVG given the draw transform.
///
/// The returned `(width, height, transform_adjust)` are used as:
///
/// - rasterize the SVG into a pixmap of `(width, height)`
/// - draw the resulting image with `transform_adjust` (not the original transform)
///
/// This avoids blurry upscaling when the SVG is scaled up in the scene.
pub fn svg_raster_params(
    tree: &usvg::Tree,
    transform: crate::foundation::core::Affine,
) -> WavyteResult<(u32, u32, crate::foundation::core::Affine)> {
    fn to_px(v: f32) -> WavyteResult<u32> {
        if !v.is_finite() || v <= 0.0 {
            return Err(WavyteError::evaluation("svg has invalid width/height"));
        }
        Ok((v.ceil() as u32).max(1))
    }

    let size = tree.size();
    let base_w = to_px(size.width())?;
    let base_h = to_px(size.height())?;

    let [a, b, c, d, _e, _f] = transform.as_coeffs();
    let sx = (a * a + b * b).sqrt().max(1e-6);
    let sy = (c * c + d * d).sqrt().max(1e-6);

    let w = ((base_w as f64) * sx).ceil().max(1.0) as u32;
    let h = ((base_h as f64) * sy).ceil().max(1.0) as u32;

    // Avoid pathological allocations. If the caller needs very large SVGs they should change the
    // render strategy and caching policy explicitly.
    const MAX_DIM: u32 = 16_384;
    if w > MAX_DIM || h > MAX_DIM {
        return Err(WavyteError::evaluation(format!(
            "svg raster size too large: {w}x{h} (max {MAX_DIM}x{MAX_DIM})"
        )));
    }

    // The SVG was rasterized with a scale applied. Adjust the draw transform so that pixel-space
    // coordinates map back into the SVG's logical coordinate space before the original transform.
    let inv = crate::foundation::core::Affine::scale_non_uniform(1.0 / sx, 1.0 / sy);
    let transform_adjust = transform * inv;

    Ok((w, h, transform_adjust))
}

pub fn rasterize_svg_to_premul_rgba8(
    tree: &usvg::Tree,
    width: u32,
    height: u32,
) -> WavyteResult<Vec<u8>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| WavyteError::evaluation("failed to allocate svg pixmap"))?;

    let sx = (width as f32) / tree.size().width();
    let sy = (height as f32) / tree.size().height();
    let xform = resvg::tiny_skia::Transform::from_scale(sx, sy);

    resvg::render(tree, xform, &mut pixmap.as_mut());
    Ok(pixmap.data().to_vec())
}
