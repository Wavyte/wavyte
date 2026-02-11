use crate::{
    assets::store::PreparedAssetStore,
    compile::{CompositePass, OffscreenPass, Pass, RenderPlan, ScenePass, SurfaceDesc, SurfaceId},
    foundation::error::{WavyteError, WavyteResult},
    render::FrameRGBA,
};

pub trait PassBackend {
    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()>;

    fn exec_scene(&mut self, pass: &ScenePass, assets: &PreparedAssetStore) -> WavyteResult<()>;

    fn exec_offscreen(
        &mut self,
        pass: &OffscreenPass,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<()>;

    fn exec_composite(
        &mut self,
        pass: &CompositePass,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<()>;

    fn readback_rgba8(
        &mut self,
        surface: SurfaceId,
        plan: &RenderPlan,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<FrameRGBA>;
}

pub fn execute_plan<B: PassBackend + ?Sized>(
    backend: &mut B,
    plan: &RenderPlan,
    assets: &PreparedAssetStore,
) -> WavyteResult<FrameRGBA> {
    for (idx, desc) in plan.surfaces.iter().enumerate() {
        let id = SurfaceId(
            idx.try_into()
                .map_err(|_| WavyteError::evaluation("surface id overflow"))?,
        );
        backend.ensure_surface(id, desc)?;
    }

    for pass in &plan.passes {
        match pass {
            Pass::Scene(p) => backend.exec_scene(p, assets)?,
            Pass::Offscreen(p) => backend.exec_offscreen(p, assets)?,
            Pass::Composite(p) => backend.exec_composite(p, assets)?,
        }
    }

    backend.readback_rgba8(plan.final_surface, plan, assets)
}

#[cfg(test)]
#[path = "../../tests/unit/render/passes.rs"]
mod tests;
