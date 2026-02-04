use crate::{
    assets::AssetCache,
    compile::RenderPlan,
    error::WavyteResult,
    render_passes::{PassBackend, execute_plan},
};

/// A rendered frame as RGBA8 pixels.
///
/// In Wavyte v0.1.0, frames are **premultiplied alpha** by default. The `premultiplied` flag is
/// included to make this explicit at API boundaries.
#[derive(Clone, Debug)]
pub struct FrameRGBA {
    pub width: u32,
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
    fn render_plan(
        &mut self,
        plan: &RenderPlan,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<FrameRGBA> {
        execute_plan(self, plan, assets)
    }
}

/// Available backend kinds.
///
/// - `Cpu` is always available.
/// - `Gpu` requires building the crate with `--features gpu`.
#[derive(Clone, Copy, Debug)]
pub enum BackendKind {
    Cpu,
    Gpu,
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
/// - `BackendKind::Gpu` is only available when built with `--features gpu`.
pub fn create_backend(
    kind: BackendKind,
    _settings: &RenderSettings,
) -> WavyteResult<Box<dyn RenderBackend>> {
    match kind {
        BackendKind::Cpu => Ok(Box::new(crate::render_cpu::CpuBackend::new(
            _settings.clone(),
        ))),
        BackendKind::Gpu => {
            #[cfg(feature = "gpu")]
            {
                Ok(Box::new(crate::render_vello::VelloBackend::new(
                    _settings.clone(),
                )?))
            }
            #[cfg(not(feature = "gpu"))]
            {
                let _ = _settings;
                Err(crate::error::WavyteError::evaluation(
                    "requested backend is not available (built without `gpu` feature)",
                ))
            }
        }
    }
}
