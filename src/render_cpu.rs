use std::collections::HashMap;

use crate::{
    assets::{AssetCache, AssetId, PreparedAsset},
    compile::{DrawOp, SurfaceDesc, SurfaceId},
    error::{WavyteError, WavyteResult},
    render::{FrameRGBA, RenderBackend, RenderSettings},
    render_passes::PassBackend,
};

pub struct CpuBackend {
    settings: RenderSettings,
    image_cache: HashMap<AssetId, vello_cpu::Image>,
    svg_cache: HashMap<AssetId, vello_cpu::Image>,
    font_cache: HashMap<AssetId, vello_cpu::peniko::FontData>,
    ctx: Option<vello_cpu::RenderContext>,
}

impl CpuBackend {
    pub fn new(settings: RenderSettings) -> Self {
        Self {
            settings,
            image_cache: HashMap::new(),
            svg_cache: HashMap::new(),
            font_cache: HashMap::new(),
            ctx: None,
        }
    }
}

impl PassBackend for CpuBackend {
    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()> {
        if id != SurfaceId(0) {
            return Ok(());
        }

        let width_u16: u16 = desc
            .width
            .try_into()
            .map_err(|_| WavyteError::evaluation("canvas width exceeds u16"))?;
        let height_u16: u16 = desc
            .height
            .try_into()
            .map_err(|_| WavyteError::evaluation("canvas height exceeds u16"))?;

        let mut ctx = vello_cpu::RenderContext::new(width_u16, height_u16);
        if let Some([r, g, b, a]) = self.settings.clear_rgba {
            ctx.set_paint(vello_cpu::peniko::Color::from_rgba8(r, g, b, a));
            ctx.fill_rect(&vello_cpu::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(width_u16),
                f64::from(height_u16),
            ));
        }

        self.ctx = Some(ctx);
        Ok(())
    }

    fn exec_scene(
        &mut self,
        pass: &crate::compile::ScenePass,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<()> {
        let mut ctx = self
            .ctx
            .take()
            .ok_or_else(|| WavyteError::evaluation("cpu backend surface 0 was not initialized"))?;

        let _ = pass.target;
        let _ = pass.clear_to_transparent;
        for op in &pass.ops {
            draw_op(self, &mut ctx, op, assets)?;
        }
        self.ctx = Some(ctx);
        Ok(())
    }

    fn exec_offscreen(
        &mut self,
        _pass: &crate::compile::OffscreenPass,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<()> {
        Ok(())
    }

    fn exec_composite(
        &mut self,
        _pass: &crate::compile::CompositePass,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<()> {
        Ok(())
    }

    fn readback_rgba8(
        &mut self,
        surface: SurfaceId,
        plan: &crate::compile::RenderPlan,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<FrameRGBA> {
        if surface != SurfaceId(0) {
            return Err(WavyteError::evaluation(
                "cpu backend readback is only supported for surface 0 in this phase",
            ));
        }

        let mut ctx = self
            .ctx
            .take()
            .ok_or_else(|| WavyteError::evaluation("cpu backend surface 0 was not initialized"))?;

        let width_u16: u16 = plan
            .canvas
            .width
            .try_into()
            .map_err(|_| WavyteError::evaluation("canvas width exceeds u16"))?;
        let height_u16: u16 = plan
            .canvas
            .height
            .try_into()
            .map_err(|_| WavyteError::evaluation("canvas height exceeds u16"))?;

        ctx.flush();
        let mut pixmap = vello_cpu::Pixmap::new(width_u16, height_u16);
        ctx.render_to_pixmap(&mut pixmap);

        Ok(FrameRGBA {
            width: plan.canvas.width,
            height: plan.canvas.height,
            data: pixmap.data_as_u8_slice().to_vec(),
            premultiplied: true,
        })
    }
}

impl RenderBackend for CpuBackend {}

fn draw_op(
    backend: &mut CpuBackend,
    ctx: &mut vello_cpu::RenderContext,
    op: &DrawOp,
    assets: &mut dyn AssetCache,
) -> WavyteResult<()> {
    ctx.set_paint_transform(vello_cpu::kurbo::Affine::IDENTITY);

    match op {
        DrawOp::FillPath {
            path,
            transform,
            color,
            opacity,
            blend: _,
            z: _,
        } => {
            ctx.set_transform(affine_to_cpu(*transform));
            ctx.set_paint(vello_cpu::peniko::Color::from_rgba8(
                color.r, color.g, color.b, color.a,
            ));
            if *opacity < 1.0 {
                ctx.push_opacity_layer(*opacity);
            }
            let cpu_path = bezpath_to_cpu(path);
            ctx.fill_path(&cpu_path);
            if *opacity < 1.0 {
                ctx.pop_layer();
            }
            Ok(())
        }
        DrawOp::Image {
            asset,
            transform,
            opacity,
            blend: _,
            z: _,
        } => {
            let image_paint = backend.image_paint_for(*asset, assets)?;
            let (w, h) = image_paint_size(&image_paint)?;

            ctx.set_transform(affine_to_cpu(*transform));
            ctx.set_paint(image_paint);

            if *opacity < 1.0 {
                ctx.push_opacity_layer(*opacity);
            }
            ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, w, h));
            if *opacity < 1.0 {
                ctx.pop_layer();
            }

            Ok(())
        }
        DrawOp::Text {
            asset,
            transform,
            opacity,
            blend: _,
            z: _,
        } => {
            let prepared = assets.get_or_load_by_id(*asset)?;
            let PreparedAsset::Text(t) = prepared else {
                return Err(WavyteError::evaluation("AssetId is not a PreparedText"));
            };

            let font = backend.font_for_text_asset(*asset, assets)?;
            ctx.set_transform(affine_to_cpu(*transform));

            if *opacity < 1.0 {
                ctx.push_opacity_layer(*opacity);
            }

            for line in t.layout.lines() {
                for item in line.items() {
                    let parley::layout::PositionedLayoutItem::GlyphRun(run) = item else {
                        continue;
                    };

                    let brush = run.style().brush;
                    ctx.set_paint(vello_cpu::peniko::Color::from_rgba8(
                        brush.r, brush.g, brush.b, brush.a,
                    ));

                    let glyphs = run.glyphs().map(|g| vello_cpu::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    });
                    ctx.glyph_run(&font)
                        .font_size(run.run().font_size())
                        .fill_glyphs(glyphs);
                }
            }

            if *opacity < 1.0 {
                ctx.pop_layer();
            }

            Ok(())
        }
        DrawOp::Svg {
            asset,
            transform,
            opacity,
            blend: _,
            z: _,
        } => {
            let svg_paint = backend.svg_paint_for(*asset, assets)?;
            let (w, h) = image_paint_size(&svg_paint)?;

            ctx.set_transform(affine_to_cpu(*transform));
            ctx.set_paint(svg_paint);

            if *opacity < 1.0 {
                ctx.push_opacity_layer(*opacity);
            }
            ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, w, h));
            if *opacity < 1.0 {
                ctx.pop_layer();
            }

            Ok(())
        }
    }
}

fn affine_to_cpu(a: crate::core::Affine) -> vello_cpu::kurbo::Affine {
    vello_cpu::kurbo::Affine::new(a.as_coeffs())
}

fn point_to_cpu(p: crate::core::Point) -> vello_cpu::kurbo::Point {
    vello_cpu::kurbo::Point::new(p.x, p.y)
}

fn bezpath_to_cpu(path: &crate::core::BezPath) -> vello_cpu::kurbo::BezPath {
    use kurbo::PathEl;

    let mut out = vello_cpu::kurbo::BezPath::new();
    for &el in path.elements() {
        match el {
            PathEl::MoveTo(p) => out.move_to(point_to_cpu(p)),
            PathEl::LineTo(p) => out.line_to(point_to_cpu(p)),
            PathEl::QuadTo(p1, p2) => out.quad_to(point_to_cpu(p1), point_to_cpu(p2)),
            PathEl::CurveTo(p1, p2, p3) => {
                out.curve_to(point_to_cpu(p1), point_to_cpu(p2), point_to_cpu(p3));
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

fn image_premul_bytes_to_pixmap(
    rgba8_premul: &[u8],
    width: u32,
    height: u32,
) -> WavyteResult<vello_cpu::Pixmap> {
    let w: u16 = width
        .try_into()
        .map_err(|_| WavyteError::evaluation("image width exceeds u16"))?;
    let h: u16 = height
        .try_into()
        .map_err(|_| WavyteError::evaluation("image height exceeds u16"))?;
    if rgba8_premul.len() != width as usize * height as usize * 4 {
        return Err(WavyteError::evaluation(
            "prepared image byte length mismatch",
        ));
    }

    let mut may_have_opacities = false;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for px in rgba8_premul.chunks_exact(4) {
        let a = px[3];
        may_have_opacities |= a != 255;
        pixels.push(vello_cpu::peniko::color::PremulRgba8 {
            r: px[0],
            g: px[1],
            b: px[2],
            a,
        });
    }

    Ok(vello_cpu::Pixmap::from_parts_with_opacity(
        pixels,
        w,
        h,
        may_have_opacities,
    ))
}

fn image_paint_size(image: &vello_cpu::Image) -> WavyteResult<(f64, f64)> {
    match &image.image {
        vello_cpu::ImageSource::Pixmap(p) => Ok((f64::from(p.width()), f64::from(p.height()))),
        vello_cpu::ImageSource::OpaqueId(_) => Err(WavyteError::evaluation(
            "cpu backend does not support opaque image ids",
        )),
    }
}

impl CpuBackend {
    fn image_paint_for(
        &mut self,
        id: AssetId,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<vello_cpu::Image> {
        if let Some(paint) = self.image_cache.get(&id) {
            return Ok(paint.clone());
        }

        let prepared = assets.get_or_load_by_id(id)?;
        let PreparedAsset::Image(img) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedImage"));
        };

        let pixmap =
            image_premul_bytes_to_pixmap(img.rgba8_premul.as_slice(), img.width, img.height)?;
        let paint = vello_cpu::Image {
            image: vello_cpu::ImageSource::Pixmap(std::sync::Arc::new(pixmap)),
            sampler: vello_cpu::peniko::ImageSampler::default(),
        };

        self.image_cache.insert(id, paint.clone());
        Ok(paint)
    }

    fn font_for_text_asset(
        &mut self,
        id: AssetId,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<vello_cpu::peniko::FontData> {
        if let Some(font) = self.font_cache.get(&id) {
            return Ok(font.clone());
        }

        let prepared = assets.get_or_load_by_id(id)?;
        let PreparedAsset::Text(t) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedText"));
        };

        let font_bytes = t.font_bytes.as_ref().clone();
        let font = vello_cpu::peniko::FontData::new(vello_cpu::peniko::Blob::from(font_bytes), 0);
        self.font_cache.insert(id, font.clone());
        Ok(font)
    }

    fn svg_paint_for(
        &mut self,
        id: AssetId,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<vello_cpu::Image> {
        if let Some(paint) = self.svg_cache.get(&id) {
            return Ok(paint.clone());
        }

        let prepared = assets.get_or_load_by_id(id)?;
        let PreparedAsset::Svg(svg) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedSvg"));
        };

        let (w, h, rgba8_premul) = rasterize_svg_to_premul_rgba8(&svg.tree)?;
        let pixmap = image_premul_bytes_to_pixmap(rgba8_premul.as_slice(), w, h)?;

        let paint = vello_cpu::Image {
            image: vello_cpu::ImageSource::Pixmap(std::sync::Arc::new(pixmap)),
            sampler: vello_cpu::peniko::ImageSampler::default(),
        };

        self.svg_cache.insert(id, paint.clone());
        Ok(paint)
    }
}

fn rasterize_svg_to_premul_rgba8(tree: &usvg::Tree) -> WavyteResult<(u32, u32, Vec<u8>)> {
    fn to_px(v: f32) -> WavyteResult<u32> {
        if !v.is_finite() || v <= 0.0 {
            return Err(WavyteError::evaluation("svg has invalid width/height"));
        }
        let px = v.ceil() as u32;
        Ok(px.max(1))
    }

    let size = tree.size();
    let width = to_px(size.width())?;
    let height = to_px(size.height())?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| WavyteError::evaluation("failed to allocate svg pixmap"))?;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    Ok((width, height, pixmap.data().to_vec()))
}
