use crate::foundation::error::WavyteResult;
use crate::v03::compile::plan::RenderPlan;
use crate::v03::eval::evaluator::EvaluatedGraph;
use crate::v03::normalize::intern::StringInterner;
use crate::v03::normalize::ir::CompositionIR;

/// A rendered frame as premultiplied RGBA8 bytes.
#[derive(Clone, Debug)]
pub(crate) struct FrameRGBA {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Vec<u8>,
}

/// Backend contract for executing a v0.3 [`RenderPlan`] into pixels.
pub(crate) trait RenderBackendV03 {
    fn render_plan(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        eval: &EvaluatedGraph,
        plan: &RenderPlan,
    ) -> WavyteResult<FrameRGBA>;
}
