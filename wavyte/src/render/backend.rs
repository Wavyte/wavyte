use crate::{
    assets::store::PreparedAssetStore,
    compile::plan::RenderPlan,
    foundation::error::WavyteResult,
    render::passes::{PassBackend, execute_plan},
};

/// A rendered frame as RGBA8 pixels.
///
/// In Wavyte v0.2.1, frames are **premultiplied alpha** by default. The `premultiplied` flag is
/// included to make this explicit at API boundaries.
#[derive(Clone, Debug)]
pub struct FrameRGBA {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// RGBA8 bytes, tightly packed, row-major.
    pub data: Vec<u8>,
    /// Whether the `data` is premultiplied alpha.
    pub premultiplied: bool,
}

/// A renderer that can execute a compiled [`RenderPlan`] into a [`FrameRGBA`].
///
/// Most users do not call [`RenderBackend::render_plan`] directly; prefer [`crate::render_frame`]
/// and friends, which handle evaluation and compilation.
pub trait RenderBackend: PassBackend {
    /// Execute a backend-agnostic [`RenderPlan`] and read back final frame.
    fn render_plan(
        &mut self,
        plan: &RenderPlan,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<FrameRGBA> {
        execute_plan(self, plan, assets)
    }

    /// Return backend settings required to construct equivalent worker backends.
    ///
    /// This is used by parallel rendering paths.
    fn worker_render_settings(&self) -> Option<RenderSettings> {
        None
    }
}

/// Available backend kinds.
///
/// - `Cpu` is always available.
#[derive(Clone, Copy, Debug)]
pub enum BackendKind {
    /// CPU raster backend powered by `vello_cpu`.
    Cpu,
}

/// Backend-agnostic settings.
#[derive(Clone, Debug, Default)]
pub struct RenderSettings {
    /// If set, backends clear the final target to this RGBA8 color before drawing.
    pub clear_rgba: Option<[u8; 4]>,
}

/// Create a rendering backend implementation.
///
/// - `BackendKind::Cpu` is always available.
pub fn create_backend(
    kind: BackendKind,
    _settings: &RenderSettings,
) -> WavyteResult<Box<dyn RenderBackend>> {
    match kind {
        BackendKind::Cpu => Ok(Box::new(crate::render::cpu::CpuBackend::new(
            _settings.clone(),
        ))),
    }
}
