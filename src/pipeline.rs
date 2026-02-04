use crate::{
    assets::AssetCache,
    compile::compile_frame,
    core::FrameIndex,
    error::WavyteResult,
    eval::Evaluator,
    model::Composition,
    render::{FrameRGBA, RenderBackend},
    render_passes::execute_plan,
};

pub fn render_frame(
    comp: &Composition,
    frame: FrameIndex,
    backend: &mut dyn RenderBackend,
    assets: &mut dyn AssetCache,
) -> WavyteResult<FrameRGBA> {
    let eval = Evaluator::eval_frame(comp, frame)?;
    let plan = compile_frame(comp, &eval, assets)?;
    execute_plan(backend, &plan, assets)
}
