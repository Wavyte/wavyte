use crate::{
    assets::AssetCache,
    error::{WavyteError, WavyteResult},
    render::{FrameRGBA, RenderBackend, RenderSettings},
};

pub struct CpuBackend {
    _settings: RenderSettings,
}

impl CpuBackend {
    pub fn new(settings: RenderSettings) -> Self {
        Self {
            _settings: settings,
        }
    }
}

impl RenderBackend for CpuBackend {
    fn render_plan(
        &mut self,
        _plan: &crate::compile::RenderPlan,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<FrameRGBA> {
        Err(WavyteError::evaluation(
            "cpu renderer not implemented yet (phase 4)",
        ))
    }
}
