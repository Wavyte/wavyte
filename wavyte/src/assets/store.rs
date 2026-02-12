use std::sync::Arc;

use crate::foundation::error::{WavyteError, WavyteResult};

#[derive(Clone, Debug)]
/// Prepared raster image in premultiplied RGBA8 form.
pub(crate) struct PreparedImage {
    /// Width in pixels.
    pub(crate) width: u32,
    /// Height in pixels.
    pub(crate) height: u32,
    /// Pixel bytes in row-major premultiplied RGBA8.
    pub(crate) rgba8_premul: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
/// Prepared SVG asset represented as a parsed `usvg` tree.
pub(crate) struct PreparedSvg {
    /// Parsed SVG tree.
    pub(crate) tree: Arc<usvg::Tree>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// RGBA8 brush color used by Parley text layout.
pub(crate) struct TextBrushRgba8 {
    /// Red channel.
    pub(crate) r: u8,
    /// Green channel.
    pub(crate) g: u8,
    /// Blue channel.
    pub(crate) b: u8,
    /// Alpha channel.
    pub(crate) a: u8,
}

/// Normalize and validate composition-relative asset paths.
///
/// The normalized result uses `/` separators, removes `.` segments, and rejects absolute paths or
/// parent traversals (`..`).
pub(crate) fn normalize_rel_path(source: &str) -> WavyteResult<String> {
    let s = source.replace('\\', "/");
    if s.starts_with('/') {
        return Err(WavyteError::validation("asset paths must be relative"));
    }
    if s.is_empty() {
        return Err(WavyteError::validation("asset path must be non-empty"));
    }

    let mut out = Vec::<&str>::new();
    for part in s.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(WavyteError::validation("asset paths must not contain '..'"));
        }
        out.push(part);
    }

    if out.is_empty() {
        return Err(WavyteError::validation(
            "asset path must contain a file name",
        ));
    }

    Ok(out.join("/"))
}

/// Stateful helper for building Parley text layouts from raw font bytes.
pub(crate) struct TextLayoutEngine {
    font_ctx: parley::FontContext,
    layout_ctx: parley::LayoutContext<TextBrushRgba8>,
}

impl Default for TextLayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextLayoutEngine {
    /// Construct a new layout engine with fresh Parley contexts.
    pub(crate) fn new() -> Self {
        Self {
            font_ctx: parley::FontContext::default(),
            layout_ctx: parley::LayoutContext::new(),
        }
    }

    /// Shape and lay out plain text using provided font bytes and styling.
    pub(crate) fn layout_plain(
        &mut self,
        text: &str,
        font_bytes: &[u8],
        size_px: f32,
        brush: TextBrushRgba8,
        max_width_px: Option<f32>,
    ) -> WavyteResult<parley::Layout<TextBrushRgba8>> {
        if !size_px.is_finite() || size_px <= 0.0 {
            return Err(WavyteError::validation(
                "text size_px must be finite and > 0",
            ));
        }

        let families = self
            .font_ctx
            .collection
            .register_fonts(parley::fontique::Blob::from(font_bytes.to_vec()), None);
        let family_id = families.first().map(|(id, _)| *id).ok_or_else(|| {
            WavyteError::validation("no font families registered from font bytes")
        })?;

        let family_name = self
            .font_ctx
            .collection
            .family_name(family_id)
            .ok_or_else(|| WavyteError::validation("registered font family has no name"))?
            .to_string();

        let mut builder = self
            .layout_ctx
            .ranged_builder(&mut self.font_ctx, text, 1.0, true);
        builder.push_default(parley::style::StyleProperty::FontStack(
            parley::style::FontStack::Source(std::borrow::Cow::Owned(family_name)),
        ));
        builder.push_default(parley::style::StyleProperty::FontSize(size_px));
        builder.push_default(parley::style::StyleProperty::Brush(brush));

        let mut layout: parley::Layout<TextBrushRgba8> = builder.build(text);
        if let Some(w) = max_width_px {
            layout.break_all_lines(Some(w));
            layout.align(
                Some(w),
                parley::Alignment::Start,
                parley::AlignmentOptions::default(),
            );
        } else {
            layout.break_all_lines(None);
        }

        Ok(layout)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/assets/store.rs"]
mod tests;
