use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PreparedImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8, row-major, tightly packed.
    pub rgba8_premul: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct PreparedSvg {
    pub tree: Arc<usvg::Tree>,
}
