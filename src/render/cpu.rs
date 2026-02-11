use std::collections::{HashMap, VecDeque};

use crate::{
    asset_store::{AssetId, PreparedAsset, PreparedAssetStore},
    compile::{CompositeOp, DrawOp, SurfaceDesc, SurfaceId},
    error::{WavyteError, WavyteResult},
    media,
    render::{FrameRGBA, RenderBackend, RenderSettings},
    render_passes::PassBackend,
    svg_raster::{SvgRasterKey, rasterize_svg_to_premul_rgba8, svg_raster_params},
};

pub struct CpuBackend {
    settings: RenderSettings,
    image_cache: HashMap<AssetId, vello_cpu::Image>,
    svg_cache: HashMap<SvgRasterKey, vello_cpu::Image>,
    font_cache: HashMap<AssetId, vello_cpu::peniko::FontData>,
    video_decoders: HashMap<AssetId, VideoFrameDecoder>,
    surfaces: HashMap<SurfaceId, CpuSurface>,
}

struct CpuSurface {
    width: u16,
    height: u16,
    pixmap: vello_cpu::Pixmap,
}

struct VideoFrameDecoder {
    info: std::sync::Arc<media::VideoSourceInfo>,
    frame_cache: HashMap<u64, vello_cpu::Image>,
    lru: VecDeque<u64>,
    capacity: usize,
    prefetch_frames: u32,
}

impl VideoFrameDecoder {
    fn new(info: std::sync::Arc<media::VideoSourceInfo>) -> Self {
        let capacity = std::env::var("WAVYTE_VIDEO_CACHE_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(64);
        let prefetch_frames = std::env::var("WAVYTE_VIDEO_PREFETCH_FRAMES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(12);
        Self {
            info,
            frame_cache: HashMap::new(),
            lru: VecDeque::new(),
            capacity,
            prefetch_frames,
        }
    }

    fn decode_at(&mut self, source_time_s: f64) -> WavyteResult<vello_cpu::Image> {
        let key = self.key_for_time(source_time_s);
        if let Some(img) = self.frame_cache.get(&key).cloned() {
            self.touch(key);
            return Ok(img);
        }

        if self.prefetch_for_key(key).is_ok()
            && let Some(img) = self.frame_cache.get(&key).cloned()
        {
            self.touch(key);
            return Ok(img);
        }

        // Fallback for sparse decode requests where batch prefetch didn't include the key.
        let rgba = media::decode_video_frame_rgba8(&self.info, source_time_s)?;
        let image = self.rgba_to_image(&rgba)?;
        self.insert_frame(key, image.clone());
        Ok(image)
    }

    fn key_for_time(&self, source_time_s: f64) -> u64 {
        ((source_time_s.max(0.0)) * 1000.0).round() as u64
    }

    fn prefetch_for_key(&mut self, key_ms: u64) -> WavyteResult<()> {
        let source_fps = self.info.source_fps();
        let step_ms = if source_fps.is_finite() && source_fps > 0.0 {
            1000.0 / source_fps
        } else {
            1.0
        };
        let window_ms = (step_ms * self.prefetch_frames as f64).max(step_ms);
        let bucket = ((key_ms as f64) / window_ms).floor();
        let start_key_ms = (bucket * window_ms).round().max(0.0) as u64;
        let start_time_s = (start_key_ms as f64) / 1000.0;
        let frames =
            media::decode_video_frames_rgba8(&self.info, start_time_s, self.prefetch_frames)?;

        for (offset, rgba) in frames.iter().enumerate() {
            let key = ((start_key_ms as f64) + ((offset as f64) * step_ms)).round() as u64;
            if self.frame_cache.contains_key(&key) {
                self.touch(key);
                continue;
            }
            let image = self.rgba_to_image(rgba)?;
            self.insert_frame(key, image);
        }
        Ok(())
    }

    fn rgba_to_image(&self, rgba: &[u8]) -> WavyteResult<vello_cpu::Image> {
        let pixmap = image_premul_bytes_to_pixmap(rgba, self.info.width, self.info.height)?;
        Ok(vello_cpu::Image {
            image: vello_cpu::ImageSource::Pixmap(std::sync::Arc::new(pixmap)),
            sampler: vello_cpu::peniko::ImageSampler::default(),
        })
    }

    fn insert_frame(&mut self, key: u64, image: vello_cpu::Image) {
        self.frame_cache.insert(key, image);
        self.touch(key);
        while self.lru.len() > self.capacity {
            if let Some(old) = self.lru.pop_front() {
                self.frame_cache.remove(&old);
            }
        }
    }

    fn touch(&mut self, key: u64) {
        if let Some(pos) = self.lru.iter().position(|x| *x == key) {
            self.lru.remove(pos);
        }
        self.lru.push_back(key);
    }
}

impl CpuBackend {
    pub fn new(settings: RenderSettings) -> Self {
        Self {
            settings,
            image_cache: HashMap::new(),
            svg_cache: HashMap::new(),
            font_cache: HashMap::new(),
            video_decoders: HashMap::new(),
            surfaces: HashMap::new(),
        }
    }
}

impl PassBackend for CpuBackend {
    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()> {
        let width_u16: u16 = desc
            .width
            .try_into()
            .map_err(|_| WavyteError::evaluation("surface width exceeds u16"))?;
        let height_u16: u16 = desc
            .height
            .try_into()
            .map_err(|_| WavyteError::evaluation("surface height exceeds u16"))?;

        match self.surfaces.get_mut(&id) {
            Some(surface) => {
                if surface.width != width_u16 || surface.height != height_u16 {
                    *surface = CpuSurface {
                        width: width_u16,
                        height: height_u16,
                        pixmap: vello_cpu::Pixmap::new(width_u16, height_u16),
                    };
                }
            }
            None => {
                self.surfaces.insert(
                    id,
                    CpuSurface {
                        width: width_u16,
                        height: height_u16,
                        pixmap: vello_cpu::Pixmap::new(width_u16, height_u16),
                    },
                );
            }
        }

        if id == SurfaceId(0) {
            let premul = self
                .settings
                .clear_rgba
                .map(|[r, g, b, a]| premul_rgba8(r, g, b, a))
                .unwrap_or([0, 0, 0, 0]);
            let s = self
                .surfaces
                .get_mut(&SurfaceId(0))
                .ok_or_else(|| WavyteError::evaluation("surface 0 missing"))?;
            clear_pixmap(&mut s.pixmap, premul);
        }
        Ok(())
    }

    fn exec_scene(
        &mut self,
        pass: &crate::compile::ScenePass,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<()> {
        let mut surface = self.surfaces.remove(&pass.target).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "scene target surface {:?} was not initialized",
                pass.target
            ))
        })?;

        if pass.clear_to_transparent {
            clear_pixmap(&mut surface.pixmap, [0, 0, 0, 0]);
        }

        let mut ctx = vello_cpu::RenderContext::new(surface.width, surface.height);
        for op in &pass.ops {
            draw_op(self, &mut ctx, op, assets)?;
        }
        ctx.flush();
        ctx.render_to_pixmap(&mut surface.pixmap);
        self.surfaces.insert(pass.target, surface);
        Ok(())
    }

    fn exec_offscreen(
        &mut self,
        pass: &crate::compile::OffscreenPass,
        _assets: &PreparedAssetStore,
    ) -> WavyteResult<()> {
        let mut output = self.surfaces.remove(&pass.output).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "offscreen output surface {:?} was not initialized",
                pass.output
            ))
        })?;

        let (w, h) = (u32::from(output.width), u32::from(output.height));
        let input_bytes = if pass.input == pass.output {
            output.pixmap.data_as_u8_slice().to_vec()
        } else {
            let input = self.surfaces.get(&pass.input).ok_or_else(|| {
                WavyteError::evaluation(format!(
                    "offscreen input surface {:?} was not initialized",
                    pass.input
                ))
            })?;
            if input.width != output.width || input.height != output.height {
                return Err(WavyteError::evaluation(
                    "offscreen input/output surface size mismatch",
                ));
            }
            input.pixmap.data_as_u8_slice().to_vec()
        };

        match pass.fx {
            crate::fx::PassFx::Blur { radius_px, sigma } => {
                let blurred =
                    crate::blur_cpu::blur_rgba8_premul(&input_bytes, w, h, radius_px, sigma)?;
                output
                    .pixmap
                    .data_as_u8_slice_mut()
                    .copy_from_slice(&blurred);
            }
        }

        self.surfaces.insert(pass.output, output);
        Ok(())
    }

    fn exec_composite(
        &mut self,
        pass: &crate::compile::CompositePass,
        _assets: &PreparedAssetStore,
    ) -> WavyteResult<()> {
        let mut dst = self.surfaces.remove(&pass.target).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "composite target surface {:?} was not initialized",
                pass.target
            ))
        })?;

        for op in &pass.ops {
            match *op {
                CompositeOp::Over { src, opacity } => {
                    let src = self.surfaces.get(&src).ok_or_else(|| {
                        WavyteError::evaluation(format!(
                            "composite src surface {:?} was not initialized",
                            src
                        ))
                    })?;
                    crate::composite_cpu::over_in_place(
                        dst.pixmap.data_as_u8_slice_mut(),
                        src.pixmap.data_as_u8_slice(),
                        opacity,
                    )?;
                }
                CompositeOp::Crossfade { a, b, t } => {
                    let a = self.surfaces.get(&a).ok_or_else(|| {
                        WavyteError::evaluation(format!(
                            "composite src surface {:?} was not initialized",
                            a
                        ))
                    })?;
                    let b = self.surfaces.get(&b).ok_or_else(|| {
                        WavyteError::evaluation(format!(
                            "composite src surface {:?} was not initialized",
                            b
                        ))
                    })?;
                    crate::composite_cpu::crossfade_over_in_place(
                        dst.pixmap.data_as_u8_slice_mut(),
                        a.pixmap.data_as_u8_slice(),
                        b.pixmap.data_as_u8_slice(),
                        t,
                    )?;
                }
                CompositeOp::Wipe {
                    a,
                    b,
                    t,
                    dir,
                    soft_edge,
                } => {
                    let a = self.surfaces.get(&a).ok_or_else(|| {
                        WavyteError::evaluation(format!(
                            "composite src surface {:?} was not initialized",
                            a
                        ))
                    })?;
                    let b = self.surfaces.get(&b).ok_or_else(|| {
                        WavyteError::evaluation(format!(
                            "composite src surface {:?} was not initialized",
                            b
                        ))
                    })?;
                    crate::composite_cpu::wipe_over_in_place(
                        dst.pixmap.data_as_u8_slice_mut(),
                        a.pixmap.data_as_u8_slice(),
                        b.pixmap.data_as_u8_slice(),
                        crate::composite_cpu::WipeParams {
                            width: u32::from(dst.width),
                            height: u32::from(dst.height),
                            t,
                            dir,
                            soft_edge,
                        },
                    )?;
                }
            }
        }
        self.surfaces.insert(pass.target, dst);
        Ok(())
    }

    fn readback_rgba8(
        &mut self,
        surface: SurfaceId,
        plan: &crate::compile::RenderPlan,
        _assets: &PreparedAssetStore,
    ) -> WavyteResult<FrameRGBA> {
        let s = self.surfaces.get(&surface).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "readback surface {:?} was not initialized",
                surface
            ))
        })?;
        let frame_data = s.pixmap.data_as_u8_slice().to_vec();
        let surface_cap = plan.surfaces.len() as u32;
        self.surfaces.retain(|id, _| id.0 < surface_cap);

        Ok(FrameRGBA {
            width: plan.canvas.width,
            height: plan.canvas.height,
            data: frame_data,
            premultiplied: true,
        })
    }
}

impl RenderBackend for CpuBackend {
    fn worker_render_settings(&self) -> Option<RenderSettings> {
        Some(self.settings.clone())
    }
}

fn premul_rgba8(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    let af = (a as u16) + 1;
    let premul = |c: u8| -> u8 { (((c as u16) * af) >> 8) as u8 };
    [premul(r), premul(g), premul(b), a]
}

fn clear_pixmap(pixmap: &mut vello_cpu::Pixmap, rgba: [u8; 4]) {
    let data = pixmap.data_as_u8_slice_mut();
    for px in data.chunks_exact_mut(4) {
        px.copy_from_slice(&rgba);
    }
}

fn draw_op(
    backend: &mut CpuBackend,
    ctx: &mut vello_cpu::RenderContext,
    op: &DrawOp,
    assets: &PreparedAssetStore,
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
            let prepared = assets.get(*asset)?;
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
            let (svg_paint, w, h, transform_adjust) =
                backend.svg_paint_for(*asset, *transform, assets)?;

            ctx.set_transform(affine_to_cpu(transform_adjust));
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
        DrawOp::Video {
            asset,
            source_time_s,
            transform,
            opacity,
            blend: _,
            z: _,
        } => {
            let video_paint = backend.video_paint_for(*asset, *source_time_s, assets)?;
            let (w, h) = image_paint_size(&video_paint)?;

            ctx.set_transform(affine_to_cpu(*transform));
            ctx.set_paint(video_paint);
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
        assets: &PreparedAssetStore,
    ) -> WavyteResult<vello_cpu::Image> {
        if let Some(paint) = self.image_cache.get(&id) {
            return Ok(paint.clone());
        }

        let prepared = assets.get(id)?;
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
        assets: &PreparedAssetStore,
    ) -> WavyteResult<vello_cpu::peniko::FontData> {
        if let Some(font) = self.font_cache.get(&id) {
            return Ok(font.clone());
        }

        let prepared = assets.get(id)?;
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
        transform: crate::core::Affine,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<(vello_cpu::Image, f64, f64, crate::core::Affine)> {
        let prepared = assets.get(id)?;
        let PreparedAsset::Svg(svg) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedSvg"));
        };

        let (w, h, transform_adjust) = svg_raster_params(&svg.tree, transform)?;
        let key = SvgRasterKey {
            asset: id,
            width: w,
            height: h,
        };
        if let Some(paint) = self.svg_cache.get(&key) {
            return Ok((paint.clone(), w as f64, h as f64, transform_adjust));
        }

        let rgba8_premul = rasterize_svg_to_premul_rgba8(&svg.tree, w, h)?;
        let pixmap = image_premul_bytes_to_pixmap(rgba8_premul.as_slice(), w, h)?;

        let paint = vello_cpu::Image {
            image: vello_cpu::ImageSource::Pixmap(std::sync::Arc::new(pixmap)),
            sampler: vello_cpu::peniko::ImageSampler::default(),
        };

        self.svg_cache.insert(key, paint.clone());
        Ok((paint, w as f64, h as f64, transform_adjust))
    }

    fn video_paint_for(
        &mut self,
        id: AssetId,
        source_time_s: f64,
        assets: &PreparedAssetStore,
    ) -> WavyteResult<vello_cpu::Image> {
        let prepared = assets.get(id)?;
        let PreparedAsset::Video(video) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedVideo"));
        };
        let decoder = self
            .video_decoders
            .entry(id)
            .or_insert_with(|| VideoFrameDecoder::new(video.info.clone()));
        decoder.decode_at(source_time_s)
    }
}
