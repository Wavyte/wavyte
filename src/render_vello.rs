use crate::{
    assets::AssetCache,
    error::{WavyteError, WavyteResult},
    render::{FrameRGBA, RenderBackend, RenderSettings},
};

pub struct VelloBackend {
    _settings: RenderSettings,
}

impl VelloBackend {
    pub fn new(settings: RenderSettings) -> WavyteResult<Self> {
        Ok(Self {
            _settings: settings,
        })
    }
}

impl RenderBackend for VelloBackend {
    fn render_plan(
        &mut self,
        _plan: &crate::compile::RenderPlan,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<FrameRGBA> {
        Err(WavyteError::evaluation(
            "gpu renderer not implemented yet (phase 4)",
        ))
    }
}
