use crate::{
    assets::AssetCache,
    compile::RenderPlan,
    error::{WavyteError, WavyteResult},
    render_passes::{PassBackend, execute_plan},
};

#[derive(Clone, Debug)]
pub struct FrameRGBA {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub premultiplied: bool,
}

pub trait RenderBackend: PassBackend {
    fn render_plan(
        &mut self,
        plan: &RenderPlan,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<FrameRGBA> {
        execute_plan(self, plan, assets)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BackendKind {
    #[cfg(feature = "cpu")]
    Cpu,
    #[cfg(feature = "gpu")]
    Gpu,
}

#[derive(Clone, Debug, Default)]
pub struct RenderSettings {
    pub clear_rgba: Option<[u8; 4]>,
}

pub fn create_backend(
    kind: BackendKind,
    _settings: &RenderSettings,
) -> WavyteResult<Box<dyn RenderBackend>> {
    match kind {
        #[cfg(feature = "cpu")]
        BackendKind::Cpu => Ok(Box::new(crate::render_cpu::CpuBackend::new(
            _settings.clone(),
        ))),
        #[cfg(feature = "gpu")]
        BackendKind::Gpu => Ok(Box::new(crate::render_vello::VelloBackend::new(
            _settings.clone(),
        )?)),
        #[allow(unreachable_patterns)]
        _ => Err(WavyteError::evaluation(
            "requested backend is not available",
        )),
    }
}
