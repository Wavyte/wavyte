use crate::assets::decode::{decode_image, parse_svg};
use crate::assets::media::{VideoSourceInfo, decode_video_frame_rgba8, probe_video};
use crate::assets::store::{TextBrushRgba8, TextLayoutEngine, normalize_rel_path};
use crate::compile::plan::{MaskGenSource, OpKind, RenderPlan, SurfaceDesc, SurfaceId, UnitKey};
use crate::eval::evaluator::{EvaluatedGraph, EvaluatedLeaf};
use crate::foundation::core::{Affine, Rgba8Premul};
use crate::foundation::error::{WavyteError, WavyteResult};
use crate::normalize::intern::StringInterner;
use crate::normalize::ir::{AssetIR, CompositionIR, ShapeIR, VarValueIR};
use crate::render::backend::{FrameRGBA, RenderBackendV03};
use crate::render::scheduler::DagScheduler;
use crate::render::surface_pool::{SurfacePool, SurfacePoolOpts};
use kurbo::Shape;
use smallvec::SmallVec;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Options for the v0.3 CPU backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuBackendOpts {
    pub(crate) pool: SurfacePoolOpts,
    pub(crate) clear_rgba: Option<[u8; 4]>,
}

impl CpuBackendOpts {
    /// Return options with a configured clear color for the final output surface.
    pub fn with_clear_rgba(mut self, clear: Option<[u8; 4]>) -> Self {
        self.clear_rgba = clear;
        self
    }
}

#[derive(Clone)]
struct ImagePaint {
    paint: vello_cpu::Image,
    w: u32,
    h: u32,
}

#[derive(Clone)]
struct SvgAssetCache {
    tree: Arc<usvg::Tree>,
}

#[derive(Clone)]
struct TextAssetCache {
    layout: Arc<parley::Layout<TextBrushRgba8>>,
    font: vello_cpu::peniko::FontData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SvgRasterKey {
    asset: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NoiseKey {
    seed: u64,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GradientKey {
    start: [u8; 4],
    end: [u8; 4],
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BlurKernelKey {
    radius_px: u32,
    sigma_bits: u32,
}

struct VideoFrameDecoder {
    info: Arc<VideoSourceInfo>,
    frame_cache: HashMap<u64, vello_cpu::Image>,
    lru: VecDeque<u64>,
    capacity: usize,
}

impl VideoFrameDecoder {
    fn new(info: Arc<VideoSourceInfo>) -> Self {
        let capacity = std::env::var("WAVYTE_VIDEO_CACHE_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(64);
        Self {
            info,
            frame_cache: HashMap::new(),
            lru: VecDeque::new(),
            capacity,
        }
    }

    fn decode_at(&mut self, source_time_s: f64) -> WavyteResult<vello_cpu::Image> {
        let key = self.key_for_time(source_time_s);
        if let Some(img) = self.frame_cache.get(&key).cloned() {
            self.touch(key);
            return Ok(img);
        }

        let rgba = decode_video_frame_rgba8(&self.info, source_time_s)?;
        let image = rgba_straight_to_image_premul(&rgba, self.info.width, self.info.height)?;
        self.insert_frame(key, image.clone());
        Ok(image)
    }

    fn key_for_time(&self, source_time_s: f64) -> u64 {
        ((source_time_s.max(0.0)) * 1000.0).round() as u64
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

/// v0.3 CPU backend powered by `vello_cpu` for vector/text rasterization.
pub(crate) struct CpuBackendV03 {
    assets_root: PathBuf,
    opts: CpuBackendOpts,

    pool: Option<SurfacePool>,
    ctx: Option<vello_cpu::RenderContext>,

    image_cache: Vec<Option<ImagePaint>>,
    svg_cache: Vec<Option<SvgAssetCache>>,
    svg_raster_cache: HashMap<SvgRasterKey, vello_cpu::Image>,
    path_cache: Vec<Option<crate::foundation::core::BezPath>>,
    text_cache: Vec<Option<TextAssetCache>>,
    text_engine: TextLayoutEngine,

    noise_cache: HashMap<NoiseKey, vello_cpu::Image>,
    gradient_cache: HashMap<GradientKey, vello_cpu::Image>,

    video_decoders: Vec<Option<VideoFrameDecoder>>,

    blur_kernel_cache: HashMap<BlurKernelKey, Arc<Vec<u32>>>,
    blur_scratch_a: Vec<u8>,
    blur_scratch_b: Vec<u8>,
}

impl CpuBackendV03 {
    pub(crate) fn new(assets_root: impl Into<PathBuf>, opts: CpuBackendOpts) -> Self {
        Self {
            assets_root: assets_root.into(),
            pool: Some(SurfacePool::new(opts.pool)),
            opts,
            ctx: None,
            image_cache: Vec::new(),
            svg_cache: Vec::new(),
            svg_raster_cache: HashMap::new(),
            path_cache: Vec::new(),
            text_cache: Vec::new(),
            text_engine: TextLayoutEngine::new(),
            noise_cache: HashMap::new(),
            gradient_cache: HashMap::new(),
            video_decoders: Vec::new(),
            blur_kernel_cache: HashMap::new(),
            blur_scratch_a: Vec::new(),
            blur_scratch_b: Vec::new(),
        }
    }

    fn ensure_asset_slots(&mut self, ir: &CompositionIR) {
        let n = ir.assets.len();
        if self.image_cache.len() != n {
            self.image_cache.resize_with(n, || None);
        }
        if self.svg_cache.len() != n {
            self.svg_cache.resize_with(n, || None);
        }
        if self.path_cache.len() != n {
            self.path_cache.resize_with(n, || None);
        }
        if self.text_cache.len() != n {
            self.text_cache.resize_with(n, || None);
        }
        if self.video_decoders.len() != n {
            self.video_decoders.resize_with(n, || None);
        }
    }

    fn with_ctx_mut<R>(
        &mut self,
        width: u16,
        height: u16,
        f: impl FnOnce(&mut Self, &mut vello_cpu::RenderContext) -> WavyteResult<R>,
    ) -> WavyteResult<R> {
        let mut ctx = match self.ctx.take() {
            None => vello_cpu::RenderContext::new(width, height),
            Some(ctx) if ctx.width() == width && ctx.height() == height => ctx,
            Some(_) => vello_cpu::RenderContext::new(width, height),
        };
        ctx.reset();
        let out = f(self, &mut ctx)?;
        self.ctx = Some(ctx);
        Ok(out)
    }

    fn read_bytes(&self, rel: &str) -> WavyteResult<Vec<u8>> {
        let norm = normalize_rel_path(rel)?;
        let p = self.assets_root.join(Path::new(&norm));
        std::fs::read(&p).map_err(|e| {
            WavyteError::evaluation(format!("failed to read asset '{}': {e}", p.display()))
        })
    }

    fn image_paint_for(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        asset_i: usize,
    ) -> WavyteResult<ImagePaint> {
        if let Some(p) = self.image_cache.get(asset_i).and_then(|x| x.clone()) {
            return Ok(p);
        }
        let AssetIR::Image { source } = &ir.assets[asset_i] else {
            return Err(WavyteError::evaluation("asset is not an image"));
        };
        let bytes = self.read_bytes(interner.get(*source))?;
        let prepared = decode_image(&bytes)?;
        let pixmap =
            pixmap_from_premul_bytes(&prepared.rgba8_premul, prepared.width, prepared.height)?;
        let paint = vello_cpu::Image {
            image: vello_cpu::ImageSource::Pixmap(Arc::new(pixmap)),
            sampler: vello_cpu::peniko::ImageSampler::default(),
        };
        let out = ImagePaint {
            paint,
            w: prepared.width,
            h: prepared.height,
        };
        self.image_cache[asset_i] = Some(out.clone());
        Ok(out)
    }

    fn svg_tree_for(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        asset_i: usize,
    ) -> WavyteResult<SvgAssetCache> {
        if let Some(t) = self.svg_cache.get(asset_i).and_then(|x| x.clone()) {
            return Ok(t);
        }
        let AssetIR::Svg { source } = &ir.assets[asset_i] else {
            return Err(WavyteError::evaluation("asset is not an svg"));
        };
        let bytes = self.read_bytes(interner.get(*source))?;
        let prepared = parse_svg(&bytes)?;
        let out = SvgAssetCache {
            tree: prepared.tree,
        };
        self.svg_cache[asset_i] = Some(out.clone());
        Ok(out)
    }

    fn svg_paint_for(
        &mut self,
        tree: &usvg::Tree,
        asset_i: usize,
        transform: Affine,
    ) -> WavyteResult<(vello_cpu::Image, f64, f64, Affine)> {
        let (w, h, transform_adjust) =
            crate::assets::svg_raster::svg_raster_params(tree, transform)?;
        let key = SvgRasterKey {
            asset: asset_i as u32,
            w,
            h,
        };
        if let Some(img) = self.svg_raster_cache.get(&key).cloned() {
            return Ok((img, w as f64, h as f64, transform_adjust));
        }
        let rgba = crate::assets::svg_raster::rasterize_svg_to_premul_rgba8(tree, w, h)?;
        let img = rgba_premul_to_image(&rgba, w, h)?;
        self.svg_raster_cache.insert(key, img.clone());
        Ok((img, w as f64, h as f64, transform_adjust))
    }

    fn path_for(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        asset_i: usize,
    ) -> WavyteResult<crate::foundation::core::BezPath> {
        if let Some(p) = self.path_cache.get(asset_i).and_then(|x| x.clone()) {
            return Ok(p);
        }
        let AssetIR::Path { svg_path_d } = &ir.assets[asset_i] else {
            return Err(WavyteError::evaluation("asset is not a path"));
        };
        let d = interner.get(*svg_path_d);
        let bp = crate::foundation::core::BezPath::from_svg(d.trim())
            .map_err(|e| WavyteError::validation(format!("invalid svg_path_d: {e}")))?;
        self.path_cache[asset_i] = Some(bp.clone());
        Ok(bp)
    }

    fn text_for(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        asset_i: usize,
    ) -> WavyteResult<TextAssetCache> {
        if let Some(t) = self.text_cache.get(asset_i).and_then(|x| x.clone()) {
            return Ok(t);
        }
        let AssetIR::Text {
            text,
            font_source,
            size_px,
            max_width_px,
            color,
        } = &ir.assets[asset_i]
        else {
            return Err(WavyteError::evaluation("asset is not text"));
        };

        let font_bytes = self.read_bytes(interner.get(*font_source))?;
        let brush = match color {
            Some(VarValueIR::Color(c)) => TextBrushRgba8 {
                r: c.r,
                g: c.g,
                b: c.b,
                a: c.a,
            },
            _ => TextBrushRgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        };
        let layout = self.text_engine.layout_plain(
            interner.get(*text),
            &font_bytes,
            *size_px as f32,
            brush,
            max_width_px.map(|v| v as f32),
        )?;

        let font = vello_cpu::peniko::FontData::new(vello_cpu::peniko::Blob::from(font_bytes), 0);
        let out = TextAssetCache {
            layout: Arc::new(layout),
            font,
        };
        self.text_cache[asset_i] = Some(out.clone());
        Ok(out)
    }

    fn noise_paint(&mut self, seed: u64, w: u32, h: u32) -> WavyteResult<vello_cpu::Image> {
        let key = NoiseKey { seed, w, h };
        if let Some(img) = self.noise_cache.get(&key).cloned() {
            return Ok(img);
        }
        let mut bytes = vec![0u8; (w as usize).saturating_mul(h as usize).saturating_mul(4)];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
                let v = hash_u32(seed, x, y) as u8;
                bytes[idx] = v;
                bytes[idx + 1] = v;
                bytes[idx + 2] = v;
                bytes[idx + 3] = 255;
            }
        }
        let img = rgba_premul_to_image(&bytes, w, h)?;
        self.noise_cache.insert(key, img.clone());
        Ok(img)
    }

    fn gradient_paint(
        &mut self,
        start: Rgba8Premul,
        end: Rgba8Premul,
        w: u32,
        h: u32,
    ) -> WavyteResult<vello_cpu::Image> {
        let key = GradientKey {
            start: [start.r, start.g, start.b, start.a],
            end: [end.r, end.g, end.b, end.a],
            w,
            h,
        };
        if let Some(img) = self.gradient_cache.get(&key).cloned() {
            return Ok(img);
        }
        let mut bytes = vec![0u8; (w as usize).saturating_mul(h as usize).saturating_mul(4)];
        let h1 = (h.max(1) - 1) as f32;
        for y in 0..h {
            let t = if h1 <= 0.0 { 0.0 } else { (y as f32) / h1 };
            let lerp = |a: u8, b: u8| -> u8 {
                let af = a as f32;
                let bf = b as f32;
                (af + (bf - af) * t).round().clamp(0.0, 255.0) as u8
            };
            let c = [
                lerp(start.r, end.r),
                lerp(start.g, end.g),
                lerp(start.b, end.b),
                lerp(start.a, end.a),
            ];
            for x in 0..w {
                let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
                bytes[idx..idx + 4].copy_from_slice(&c);
            }
        }
        let img = rgba_premul_to_image(&bytes, w, h)?;
        self.gradient_cache.insert(key, img.clone());
        Ok(img)
    }

    fn video_paint_for(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        asset_i: usize,
        local_frame: u64,
    ) -> WavyteResult<ImagePaint> {
        let AssetIR::Video {
            source,
            trim_start_sec,
            trim_end_sec,
            playback_rate,
            ..
        } = &ir.assets[asset_i]
        else {
            return Err(WavyteError::evaluation("asset is not video"));
        };

        if self.video_decoders[asset_i].is_none() {
            let rel = interner.get(*source);
            let norm = normalize_rel_path(rel)?;
            let p = self.assets_root.join(Path::new(&norm));
            let info = probe_video(&p)?;
            self.video_decoders[asset_i] = Some(VideoFrameDecoder::new(Arc::new(info)));
        }
        let decoder = self.video_decoders[asset_i]
            .as_mut()
            .ok_or_else(|| WavyteError::evaluation("video decoder missing"))?;

        let fps = ir.fps;
        let timeline_t = (local_frame as f64) * (f64::from(fps.den) / f64::from(fps.num));
        let mut src_t = (*trim_start_sec) + timeline_t * (*playback_rate);
        if let Some(end) = trim_end_sec {
            src_t = src_t.min(end.max(*trim_start_sec));
        }
        src_t = src_t.max(0.0);

        let paint = decoder.decode_at(src_t)?;
        let out = ImagePaint {
            paint,
            w: decoder.info.width,
            h: decoder.info.height,
        };
        Ok(out)
    }

    fn leaf_paint_size(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        eval: &EvaluatedGraph,
        leaf: &EvaluatedLeaf,
    ) -> WavyteResult<(f64, f64)> {
        let node = &ir.nodes[leaf.node.0 as usize];
        if node.layout.is_some() {
            let r = eval
                .layout_rects
                .get(leaf.node.0 as usize)
                .copied()
                .unwrap_or_default();
            let w = r.w as f64;
            let h = r.h as f64;
            if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
                return Ok((w, h));
            }
        }

        let a = &ir.assets[leaf.asset.0 as usize];
        match a {
            AssetIR::Image { .. } => {
                let p = self.image_paint_for(ir, interner, leaf.asset.0 as usize)?;
                Ok((p.w as f64, p.h as f64))
            }
            AssetIR::Svg { .. } => {
                let t = self.svg_tree_for(ir, interner, leaf.asset.0 as usize)?;
                let s = t.tree.size();
                Ok((s.width() as f64, s.height() as f64))
            }
            AssetIR::Video { .. } => self
                .video_decoders
                .get(leaf.asset.0 as usize)
                .and_then(|d| d.as_ref())
                .map(|d| (d.info.width as f64, d.info.height as f64))
                .ok_or_else(|| {
                    // Avoid probing/decoding in sizing calls; callers should only ask for size
                    // when the asset is expected to have been prepared already.
                    WavyteError::evaluation(
                        "video intrinsic size unavailable (decoder not prepared)",
                    )
                }),
            AssetIR::SolidRect { .. }
            | AssetIR::Gradient { .. }
            | AssetIR::Noise { .. }
            | AssetIR::Null
            | AssetIR::Audio { .. }
            | AssetIR::Path { .. }
            | AssetIR::Text { .. } => Ok((ir.canvas.width as f64, ir.canvas.height as f64)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_leaf(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        eval: &EvaluatedGraph,
        leaf: &EvaluatedLeaf,
        transform_post: Affine,
        opacity_mul: f32,
        ctx: &mut vello_cpu::RenderContext,
    ) -> WavyteResult<()> {
        let asset_i = leaf.asset.0 as usize;
        let asset = &ir.assets[asset_i];
        let tr = leaf.world_transform * transform_post;
        let opacity = (leaf.opacity * opacity_mul).clamp(0.0, 1.0);

        // Note: v0.3 does not yet have per-leaf blend modes; the plan currently emits Normal.
        ctx.set_blend_mode(vello_cpu::peniko::BlendMode::default());
        ctx.set_paint_transform(vello_cpu::kurbo::Affine::IDENTITY);

        match asset {
            AssetIR::Null | AssetIR::Audio { .. } => Ok(()),
            AssetIR::SolidRect { color } => {
                let c = match color {
                    Some(VarValueIR::Color(c)) => *c,
                    _ => Rgba8Premul::from_straight_rgba(255, 255, 255, 255),
                };
                let (w, h) = self.leaf_paint_size(ir, interner, eval, leaf)?;
                ctx.set_transform(affine_to_cpu(tr));
                ctx.set_paint(vello_cpu::peniko::Color::from_rgba8(c.r, c.g, c.b, c.a));
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, w, h));
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Gradient { start, end } => {
                let s = match start {
                    VarValueIR::Color(c) => *c,
                    _ => Rgba8Premul::transparent(),
                };
                let e = match end {
                    VarValueIR::Color(c) => *c,
                    _ => Rgba8Premul::transparent(),
                };
                let (w, h) = self.leaf_paint_size(ir, interner, eval, leaf)?;
                let iw = w.max(1.0) as u32;
                let ih = h.max(1.0) as u32;
                let img = self.gradient_paint(s, e, iw, ih)?;

                ctx.set_transform(affine_to_cpu(tr));
                ctx.set_paint(img);
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, w, h));
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Noise { seed } => {
                let (w, h) = self.leaf_paint_size(ir, interner, eval, leaf)?;
                let iw = w.max(1.0) as u32;
                let ih = h.max(1.0) as u32;
                let img = self.noise_paint(*seed, iw, ih)?;

                ctx.set_transform(affine_to_cpu(tr));
                ctx.set_paint(img);
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, w, h));
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Image { .. } => {
                let p = self.image_paint_for(ir, interner, asset_i)?;
                ctx.set_transform(affine_to_cpu(tr));
                ctx.set_paint(p.paint);
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                ctx.fill_rect(&vello_cpu::kurbo::Rect::new(
                    0.0, 0.0, p.w as f64, p.h as f64,
                ));
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Svg { .. } => {
                let svg = self.svg_tree_for(ir, interner, asset_i)?;
                let (img, w, h, transform_adjust) = self.svg_paint_for(&svg.tree, asset_i, tr)?;

                ctx.set_transform(affine_to_cpu(transform_adjust));
                ctx.set_paint(img);
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, w, h));
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Path { .. } => {
                let path = self.path_for(ir, interner, asset_i)?;
                ctx.set_transform(affine_to_cpu(tr));
                ctx.set_paint(vello_cpu::peniko::Color::from_rgba8(255, 255, 255, 255));
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                let cpu_path = bezpath_to_cpu(&path);
                ctx.fill_path(&cpu_path);
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Text { .. } => {
                let t = self.text_for(ir, interner, asset_i)?;
                ctx.set_transform(affine_to_cpu(tr));
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
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
                        ctx.glyph_run(&t.font)
                            .font_size(run.run().font_size())
                            .fill_glyphs(glyphs);
                    }
                }
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
            AssetIR::Video { .. } => {
                let p = self.video_paint_for(ir, interner, asset_i, leaf.local_frame)?;
                ctx.set_transform(affine_to_cpu(tr));
                ctx.set_paint(p.paint);
                if opacity < 1.0 {
                    ctx.push_opacity_layer(opacity);
                }
                ctx.fill_rect(&vello_cpu::kurbo::Rect::new(
                    0.0, 0.0, p.w as f64, p.h as f64,
                ));
                if opacity < 1.0 {
                    ctx.pop_layer();
                }
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_draw(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        eval: &EvaluatedGraph,
        unit: UnitKey,
        leaves: std::ops::Range<usize>,
        clear_to_transparent: bool,
        transform_post: Affine,
        opacity_mul: f32,
        surfaces: &mut ExecSurfaces<'_>,
        out: SurfaceId,
    ) -> WavyteResult<()> {
        let desc = surfaces.desc(out);

        let dst_idx = out.0 as usize;
        surfaces.ensure(out, desc)?;
        if clear_to_transparent {
            clear_pixmap_to_transparent(surfaces.pixmaps[dst_idx].as_mut().unwrap());
            self.render_leaves_to(
                ir,
                interner,
                eval,
                leaves,
                transform_post,
                opacity_mul,
                surfaces.pixmaps[dst_idx].as_mut().unwrap(),
            )?;
            return Ok(());
        }

        // Accumulation draw: `vello_cpu` renders into a fresh buffer, so we render into a temp
        // surface and then premul-over onto the destination buffer.
        let mut tmp = surfaces.borrow_temp(desc);
        clear_pixmap_to_transparent(&mut tmp);
        self.render_leaves_to(
            ir,
            interner,
            eval,
            leaves,
            transform_post,
            opacity_mul,
            &mut tmp,
        )?;
        premul_over_in_place(
            surfaces.pixmaps[dst_idx]
                .as_mut()
                .unwrap()
                .data_as_u8_slice_mut(),
            tmp.data_as_u8_slice(),
        )?;
        surfaces.release_temp(desc, tmp);
        let _ = unit;
        Ok(())
    }

    fn exec_mask_gen(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        source: &MaskGenSource,
        surfaces: &mut ExecSurfaces<'_>,
        out: SurfaceId,
    ) -> WavyteResult<()> {
        let desc = surfaces.desc(out);
        surfaces.ensure(out, desc)?;
        let dst_idx = out.0 as usize;
        let dst = surfaces.pixmaps[dst_idx].as_mut().unwrap();
        clear_pixmap_to_transparent(dst);

        // Render mask sources as premul RGBA; mask interpretation is done in MaskApply.
        let width_u16 = dst.width();
        let height_u16 = dst.height();
        self.with_ctx_mut(width_u16, height_u16, |this, ctx| {
            match source {
                MaskGenSource::Asset(a) => {
                    let leaf = EvaluatedLeaf {
                        node: crate::foundation::ids::NodeIdx(0),
                        asset: *a,
                        local_frame: 0,
                        world_transform: Affine::IDENTITY,
                        opacity: 1.0,
                        group_stack: SmallVec::new(),
                    };
                    let dummy = EvaluatedGraph {
                        frame: 0,
                        leaves: Vec::new(),
                        groups: Vec::new(),
                        units: Vec::new(),
                        node_leaf_ranges: Vec::new(),
                        layout_rects: Vec::new(),
                    };
                    this.draw_leaf(ir, interner, &dummy, &leaf, Affine::IDENTITY, 1.0, ctx)?;
                }
                MaskGenSource::Shape(shape) => {
                    ctx.set_transform(vello_cpu::kurbo::Affine::IDENTITY);
                    ctx.set_paint(vello_cpu::peniko::Color::from_rgba8(255, 255, 255, 255));
                    match shape {
                        ShapeIR::Rect { width, height } => {
                            ctx.fill_rect(&vello_cpu::kurbo::Rect::new(0.0, 0.0, *width, *height));
                        }
                        ShapeIR::RoundedRect {
                            width,
                            height,
                            radius,
                        } => {
                            let rr = kurbo::RoundedRect::new(0.0, 0.0, *width, *height, *radius);
                            let mut p = vello_cpu::kurbo::BezPath::new();
                            for el in rr.path_elements(0.1) {
                                p.push(el);
                            }
                            ctx.fill_path(&p);
                        }
                        ShapeIR::Ellipse { rx, ry } => {
                            let e = kurbo::Ellipse::new((*rx, *ry), (*rx, *ry), 0.0);
                            let mut p = vello_cpu::kurbo::BezPath::new();
                            for el in e.path_elements(0.1) {
                                p.push(el);
                            }
                            ctx.fill_path(&p);
                        }
                        ShapeIR::Path { svg_path_d } => {
                            let d = interner.get(*svg_path_d);
                            let bp = crate::foundation::core::BezPath::from_svg(d.trim()).map_err(
                                |e| WavyteError::validation(format!("invalid svg_path_d: {e}")),
                            )?;
                            let cpu_path = bezpath_to_cpu(&bp);
                            ctx.fill_path(&cpu_path);
                        }
                    }
                }
            }

            ctx.flush();
            ctx.render_to_pixmap(dst);
            Ok(())
        })
    }

    fn exec_blur_pass(
        &mut self,
        fx_radius_px: u32,
        fx_sigma: f32,
        surfaces: &mut ExecSurfaces<'_>,
        inputs: &SmallVec<[SurfaceId; 4]>,
        out: SurfaceId,
    ) -> WavyteResult<()> {
        let Some(&src_id) = inputs.first() else {
            return Err(WavyteError::evaluation("Blur pass expects 1 input surface"));
        };
        let desc = surfaces.desc(out);
        surfaces.ensure(out, desc)?;

        let (w, h) = (desc.width, desc.height);
        let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);

        // Kernel and scratch are op-level allocations only (cached after warmup).
        let sigma_bits = fx_sigma.to_bits();
        let key = BlurKernelKey {
            radius_px: fx_radius_px,
            sigma_bits,
        };
        let kernel = if let Some(k) = self.blur_kernel_cache.get(&key).cloned() {
            k
        } else {
            let k = gaussian_kernel_q16(fx_radius_px, fx_sigma)?;
            let a = Arc::new(k);
            self.blur_kernel_cache.insert(key, a.clone());
            a
        };

        self.blur_scratch_a.resize(expected, 0);
        self.blur_scratch_b.resize(expected, 0);

        // Copy src into scratch first to avoid alias/borrow issues. This is still allocation-free
        // after warmup.
        {
            let src_pm = surfaces
                .pixmaps
                .get(src_id.0 as usize)
                .and_then(|x| x.as_ref())
                .ok_or_else(|| WavyteError::evaluation("Blur input surface missing"))?;
            let src_bytes = src_pm.data_as_u8_slice();
            if src_bytes.len() != expected {
                return Err(WavyteError::evaluation(
                    "Blur input surface buffer size mismatch",
                ));
            }
            self.blur_scratch_b.copy_from_slice(src_bytes);
        }

        let dst_pm = surfaces.pixmaps[out.0 as usize]
            .as_mut()
            .ok_or_else(|| WavyteError::evaluation("Blur output surface missing"))?;
        let dst_bytes = dst_pm.data_as_u8_slice_mut();
        if dst_bytes.len() != expected {
            return Err(WavyteError::evaluation(
                "Blur output surface buffer size mismatch",
            ));
        }

        blur_rgba8_premul_q16(
            &self.blur_scratch_b,
            dst_bytes,
            &mut self.blur_scratch_a,
            w,
            h,
            &kernel,
        );

        Ok(())
    }

    fn exec_mask_apply_pass(
        &mut self,
        mode: crate::compile::plan::MaskMode,
        inverted: bool,
        surfaces: &mut ExecSurfaces<'_>,
        inputs: &SmallVec<[SurfaceId; 4]>,
        out: SurfaceId,
    ) -> WavyteResult<()> {
        if inputs.len() != 2 {
            return Err(WavyteError::evaluation(
                "MaskApply pass expects 2 input surfaces",
            ));
        }
        let src_id = inputs[0];
        let mask_id = inputs[1];

        let desc = surfaces.desc(out);
        surfaces.ensure(out, desc)?;
        let expected = (desc.width as usize)
            .saturating_mul(desc.height as usize)
            .saturating_mul(4);

        let src_i = src_id.0 as usize;
        let mask_i = mask_id.0 as usize;
        let dst_i = out.0 as usize;

        // Typical case: distinct output surface. Use slice splitting to avoid borrow conflicts
        // without copying full buffers.
        if dst_i != src_i && dst_i != mask_i {
            let (left, at_and_right) = surfaces.pixmaps.split_at_mut(dst_i);
            let (dst_slot, right) = at_and_right
                .split_first_mut()
                .ok_or_else(|| WavyteError::evaluation("MaskApply output index OOB"))?;

            let get_ref = |i: usize| -> Option<&Option<vello_cpu::Pixmap>> {
                if i < dst_i {
                    left.get(i)
                } else if i == dst_i {
                    None
                } else {
                    right.get(i - dst_i - 1)
                }
            };

            let src_pm = get_ref(src_i)
                .and_then(|x| x.as_ref())
                .ok_or_else(|| WavyteError::evaluation("MaskApply src surface missing"))?;
            let mask_pm = get_ref(mask_i)
                .and_then(|x| x.as_ref())
                .ok_or_else(|| WavyteError::evaluation("MaskApply mask surface missing"))?;
            let dst_pm = dst_slot
                .as_mut()
                .ok_or_else(|| WavyteError::evaluation("MaskApply output surface missing"))?;

            let src = src_pm.data_as_u8_slice();
            let mask = mask_pm.data_as_u8_slice();
            let dst = dst_pm.data_as_u8_slice_mut();
            if src.len() != expected || mask.len() != expected || dst.len() != expected {
                return Err(WavyteError::evaluation(
                    "MaskApply surface buffer size mismatch",
                ));
            }

            mask_apply_rgba8_premul(src, mask, dst, mode, inverted);
            return Ok(());
        }

        // Fallback for aliasing cases after surface canonicalization: copy inputs into scratch.
        self.blur_scratch_a.resize(expected, 0);
        self.blur_scratch_b.resize(expected, 0);
        {
            let src_pm = surfaces
                .pixmaps
                .get(src_i)
                .and_then(|x| x.as_ref())
                .ok_or_else(|| WavyteError::evaluation("MaskApply src surface missing"))?;
            let mask_pm = surfaces
                .pixmaps
                .get(mask_i)
                .and_then(|x| x.as_ref())
                .ok_or_else(|| WavyteError::evaluation("MaskApply mask surface missing"))?;
            self.blur_scratch_a
                .copy_from_slice(src_pm.data_as_u8_slice());
            self.blur_scratch_b
                .copy_from_slice(mask_pm.data_as_u8_slice());
        }
        let dst_pm = surfaces.pixmaps[dst_i]
            .as_mut()
            .ok_or_else(|| WavyteError::evaluation("MaskApply output surface missing"))?;
        let dst = dst_pm.data_as_u8_slice_mut();
        mask_apply_rgba8_premul(
            &self.blur_scratch_a,
            &self.blur_scratch_b,
            dst,
            mode,
            inverted,
        );
        Ok(())
    }

    fn exec_color_matrix_pass(
        &mut self,
        matrix: [f32; 20],
        surfaces: &mut ExecSurfaces<'_>,
        inputs: &SmallVec<[SurfaceId; 4]>,
        out: SurfaceId,
    ) -> WavyteResult<()> {
        let Some(&src_id) = inputs.first() else {
            return Err(WavyteError::evaluation(
                "ColorMatrix pass expects 1 input surface",
            ));
        };
        let desc = surfaces.desc(out);
        surfaces.ensure(out, desc)?;
        let expected = (desc.width as usize)
            .saturating_mul(desc.height as usize)
            .saturating_mul(4);

        self.blur_scratch_b.resize(expected, 0);
        {
            let src_pm = surfaces
                .pixmaps
                .get(src_id.0 as usize)
                .and_then(|x| x.as_ref())
                .ok_or_else(|| WavyteError::evaluation("ColorMatrix input surface missing"))?;
            let src = src_pm.data_as_u8_slice();
            if src.len() != expected {
                return Err(WavyteError::evaluation(
                    "ColorMatrix input buffer size mismatch",
                ));
            }
            self.blur_scratch_b.copy_from_slice(src);
        }

        let dst_pm = surfaces.pixmaps[out.0 as usize]
            .as_mut()
            .ok_or_else(|| WavyteError::evaluation("ColorMatrix output surface missing"))?;
        let dst = dst_pm.data_as_u8_slice_mut();
        if dst.len() != expected {
            return Err(WavyteError::evaluation(
                "ColorMatrix output buffer size mismatch",
            ));
        }

        color_matrix_rgba8_premul(&self.blur_scratch_b, dst, matrix);
        Ok(())
    }

    fn exec_composite(
        &mut self,
        clear_to_transparent: bool,
        cops: &[crate::compile::plan::CompositeOp],
        surfaces: &mut ExecSurfaces<'_>,
        out: SurfaceId,
    ) -> WavyteResult<()> {
        let desc = surfaces.desc(out);
        surfaces.ensure(out, desc)?;
        let dst_i = out.0 as usize;

        let (left, at_and_right) = surfaces.pixmaps.split_at_mut(dst_i);
        let (dst_slot, right) = at_and_right
            .split_first_mut()
            .ok_or_else(|| WavyteError::evaluation("Composite output index OOB"))?;
        let dst_pm = dst_slot
            .as_mut()
            .ok_or_else(|| WavyteError::evaluation("Composite output surface missing"))?;

        if clear_to_transparent {
            clear_pixmap_to_transparent(dst_pm);
        }

        let get_src_noalias = |sid: SurfaceId| -> WavyteResult<&vello_cpu::Pixmap> {
            let i = sid.0 as usize;
            let slot = if i < dst_i {
                left.get(i)
            } else {
                right.get(i - dst_i - 1)
            }
            .and_then(|x| x.as_ref())
            .ok_or_else(|| WavyteError::evaluation("Composite source surface missing"))?;
            Ok(slot)
        };

        let (w, h) = (desc.width, desc.height);
        let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        if dst_pm.data_as_u8_slice().len() != expected {
            return Err(WavyteError::evaluation(
                "Composite output buffer size mismatch",
            ));
        }

        for c in cops {
            match *c {
                crate::compile::plan::CompositeOp::Over {
                    src,
                    opacity,
                    blend,
                } => {
                    let src_bytes: &[u8] = if src == out {
                        // Alias fallback: snapshot current dst into scratch.
                        self.blur_scratch_a.resize(expected, 0);
                        self.blur_scratch_a
                            .copy_from_slice(dst_pm.data_as_u8_slice());
                        &self.blur_scratch_a
                    } else {
                        get_src_noalias(src)?.data_as_u8_slice()
                    };
                    composite_over_rgba8_premul(
                        dst_pm.data_as_u8_slice_mut(),
                        src_bytes,
                        opacity,
                        blend,
                    )?;
                }
                crate::compile::plan::CompositeOp::Crossfade { a, b, t } => {
                    let a_bytes: &[u8] = if a == out {
                        self.blur_scratch_a.resize(expected, 0);
                        self.blur_scratch_a
                            .copy_from_slice(dst_pm.data_as_u8_slice());
                        &self.blur_scratch_a
                    } else {
                        get_src_noalias(a)?.data_as_u8_slice()
                    };
                    let b_bytes: &[u8] = if b == out {
                        if a == out {
                            &self.blur_scratch_a
                        } else {
                            self.blur_scratch_b.resize(expected, 0);
                            self.blur_scratch_b
                                .copy_from_slice(dst_pm.data_as_u8_slice());
                            &self.blur_scratch_b
                        }
                    } else {
                        get_src_noalias(b)?.data_as_u8_slice()
                    };
                    composite_crossfade_over_rgba8_premul(
                        dst_pm.data_as_u8_slice_mut(),
                        a_bytes,
                        b_bytes,
                        t,
                    )?;
                }
                crate::compile::plan::CompositeOp::Wipe {
                    a,
                    b,
                    t,
                    dir,
                    soft_edge,
                } => {
                    let a_bytes: &[u8] = if a == out {
                        self.blur_scratch_a.resize(expected, 0);
                        self.blur_scratch_a
                            .copy_from_slice(dst_pm.data_as_u8_slice());
                        &self.blur_scratch_a
                    } else {
                        get_src_noalias(a)?.data_as_u8_slice()
                    };
                    let b_bytes: &[u8] = if b == out {
                        if a == out {
                            &self.blur_scratch_a
                        } else {
                            self.blur_scratch_b.resize(expected, 0);
                            self.blur_scratch_b
                                .copy_from_slice(dst_pm.data_as_u8_slice());
                            &self.blur_scratch_b
                        }
                    } else {
                        get_src_noalias(b)?.data_as_u8_slice()
                    };
                    composite_wipe_over_rgba8_premul(
                        dst_pm.data_as_u8_slice_mut(),
                        a_bytes,
                        b_bytes,
                        w,
                        h,
                        t,
                        dir,
                        soft_edge,
                    )?;
                }
                crate::compile::plan::CompositeOp::Slide { a, b, t, dir, push } => {
                    let a_bytes: &[u8] = if a == out {
                        self.blur_scratch_a.resize(expected, 0);
                        self.blur_scratch_a
                            .copy_from_slice(dst_pm.data_as_u8_slice());
                        &self.blur_scratch_a
                    } else {
                        get_src_noalias(a)?.data_as_u8_slice()
                    };
                    let b_bytes: &[u8] = if b == out {
                        if a == out {
                            &self.blur_scratch_a
                        } else {
                            self.blur_scratch_b.resize(expected, 0);
                            self.blur_scratch_b
                                .copy_from_slice(dst_pm.data_as_u8_slice());
                            &self.blur_scratch_b
                        }
                    } else {
                        get_src_noalias(b)?.data_as_u8_slice()
                    };
                    composite_slide_over_rgba8_premul(
                        dst_pm.data_as_u8_slice_mut(),
                        a_bytes,
                        b_bytes,
                        w,
                        h,
                        t,
                        dir,
                        push,
                    )?;
                }
                crate::compile::plan::CompositeOp::Zoom {
                    a,
                    b,
                    t,
                    origin,
                    from_scale,
                } => {
                    let a_bytes: &[u8] = if a == out {
                        self.blur_scratch_a.resize(expected, 0);
                        self.blur_scratch_a
                            .copy_from_slice(dst_pm.data_as_u8_slice());
                        &self.blur_scratch_a
                    } else {
                        get_src_noalias(a)?.data_as_u8_slice()
                    };
                    let b_bytes: &[u8] = if b == out {
                        if a == out {
                            &self.blur_scratch_a
                        } else {
                            self.blur_scratch_b.resize(expected, 0);
                            self.blur_scratch_b
                                .copy_from_slice(dst_pm.data_as_u8_slice());
                            &self.blur_scratch_b
                        }
                    } else {
                        get_src_noalias(b)?.data_as_u8_slice()
                    };
                    composite_zoom_over_rgba8_premul(
                        dst_pm.data_as_u8_slice_mut(),
                        a_bytes,
                        b_bytes,
                        w,
                        h,
                        t,
                        origin,
                        from_scale,
                    )?;
                }
                crate::compile::plan::CompositeOp::Iris {
                    a,
                    b,
                    t,
                    origin,
                    shape,
                    soft_edge,
                } => {
                    let a_bytes: &[u8] = if a == out {
                        self.blur_scratch_a.resize(expected, 0);
                        self.blur_scratch_a
                            .copy_from_slice(dst_pm.data_as_u8_slice());
                        &self.blur_scratch_a
                    } else {
                        get_src_noalias(a)?.data_as_u8_slice()
                    };
                    let b_bytes: &[u8] = if b == out {
                        if a == out {
                            &self.blur_scratch_a
                        } else {
                            self.blur_scratch_b.resize(expected, 0);
                            self.blur_scratch_b
                                .copy_from_slice(dst_pm.data_as_u8_slice());
                            &self.blur_scratch_b
                        }
                    } else {
                        get_src_noalias(b)?.data_as_u8_slice()
                    };
                    composite_iris_over_rgba8_premul(
                        dst_pm.data_as_u8_slice_mut(),
                        a_bytes,
                        b_bytes,
                        w,
                        h,
                        t,
                        origin,
                        shape,
                        soft_edge,
                    )?;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn render_leaves_to(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        eval: &EvaluatedGraph,
        leaves: std::ops::Range<usize>,
        transform_post: Affine,
        opacity_mul: f32,
        dst: &mut vello_cpu::Pixmap,
    ) -> WavyteResult<()> {
        let width_u16 = dst.width();
        let height_u16 = dst.height();
        self.with_ctx_mut(width_u16, height_u16, |this, ctx| {
            let end = leaves.end.min(eval.leaves.len());
            let start = leaves.start.min(end);
            for leaf in &eval.leaves[start..end] {
                this.draw_leaf(ir, interner, eval, leaf, transform_post, opacity_mul, ctx)?;
            }
            ctx.flush();
            ctx.render_to_pixmap(dst);
            Ok(())
        })
    }
}

impl RenderBackendV03 for CpuBackendV03 {
    fn render_plan(
        &mut self,
        ir: &CompositionIR,
        interner: &StringInterner,
        eval: &EvaluatedGraph,
        plan: &RenderPlan,
    ) -> WavyteResult<FrameRGBA> {
        self.ensure_asset_slots(ir);

        let mut pool = self
            .pool
            .take()
            .ok_or_else(|| WavyteError::evaluation("cpu backend surface pool missing"))?;
        let mut surfaces = ExecSurfaces::new(&plan.surfaces, &mut pool);
        for &root in &plan.roots {
            surfaces.ensure(root, surfaces.desc(root))?;
        }

        if let Some(clear) = self.opts.clear_rgba
            && let Some(root) = plan.roots.first().copied()
            && let Some(pm) = surfaces.pixmaps[root.0 as usize].as_mut()
        {
            clear_pixmap(pm, premul_rgba8(clear));
        }

        let mut sched = DagScheduler::new(&plan.ops);
        while let Some(next) = sched.pop_ready() {
            let op = &plan.ops[next.0 as usize];

            match &op.kind {
                OpKind::Draw {
                    unit,
                    leaves,
                    clear_to_transparent,
                    transform_post,
                    opacity_mul,
                } => {
                    self.exec_draw(
                        ir,
                        interner,
                        eval,
                        *unit,
                        leaves.clone(),
                        *clear_to_transparent,
                        *transform_post,
                        *opacity_mul,
                        &mut surfaces,
                        op.output,
                    )?;
                }
                OpKind::MaskGen { source } => {
                    self.exec_mask_gen(ir, interner, source, &mut surfaces, op.output)?;
                }
                OpKind::Composite {
                    clear_to_transparent,
                    ops,
                } => {
                    self.exec_composite(
                        *clear_to_transparent,
                        ops.as_ref(),
                        &mut surfaces,
                        op.output,
                    )?;
                }
                OpKind::Pass {
                    fx: crate::compile::plan::PassFx::Blur { radius_px, sigma },
                } => {
                    self.exec_blur_pass(*radius_px, *sigma, &mut surfaces, &op.inputs, op.output)?;
                }
                OpKind::Pass {
                    fx: crate::compile::plan::PassFx::MaskApply { mode, inverted },
                } => {
                    self.exec_mask_apply_pass(
                        *mode,
                        *inverted,
                        &mut surfaces,
                        &op.inputs,
                        op.output,
                    )?;
                }
                OpKind::Pass {
                    fx: crate::compile::plan::PassFx::ColorMatrix { matrix },
                } => {
                    self.exec_color_matrix_pass(*matrix, &mut surfaces, &op.inputs, op.output)?;
                }
                other => {
                    return Err(WavyteError::evaluation(format!(
                        "v0.3 cpu backend: op kind not implemented yet: {other:?}"
                    )));
                }
            }

            sched.mark_done(next);
        }

        let root = plan
            .roots
            .first()
            .copied()
            .ok_or_else(|| WavyteError::evaluation("plan has no roots"))?;
        let root_desc = plan.surfaces[root.0 as usize];
        let root_pixmap = surfaces
            .pixmaps
            .get(root.0 as usize)
            .and_then(|x| x.as_ref())
            .ok_or_else(|| WavyteError::evaluation("root surface missing"))?;

        let out = FrameRGBA {
            width: root_desc.width,
            height: root_desc.height,
            data: root_pixmap.data_as_u8_slice().to_vec(),
        };

        surfaces.release_all();
        drop(surfaces);
        self.pool = Some(pool);
        Ok(out)
    }
}

struct ExecSurfaces<'a> {
    descs: &'a [SurfaceDesc],
    pixmaps: Vec<Option<vello_cpu::Pixmap>>,
    pool: &'a mut SurfacePool,
}

impl<'a> ExecSurfaces<'a> {
    fn new(descs: &'a [SurfaceDesc], pool: &'a mut SurfacePool) -> Self {
        Self {
            descs,
            pixmaps: vec![None; descs.len()],
            pool,
        }
    }

    fn desc(&self, id: SurfaceId) -> SurfaceDesc {
        self.descs[id.0 as usize]
    }

    fn ensure(&mut self, id: SurfaceId, desc: SurfaceDesc) -> WavyteResult<()> {
        let idx = id.0 as usize;
        if self.pixmaps[idx].is_some() {
            return Ok(());
        }
        self.pixmaps[idx] = Some(self.pool.borrow(desc));
        Ok(())
    }

    fn borrow_temp(&mut self, desc: SurfaceDesc) -> vello_cpu::Pixmap {
        self.pool.borrow(desc)
    }

    fn release_temp(&mut self, desc: SurfaceDesc, pixmap: vello_cpu::Pixmap) {
        self.pool.release(desc, pixmap);
    }

    fn release_all(&mut self) {
        for (i, slot) in self.pixmaps.iter_mut().enumerate() {
            let Some(pm) = slot.take() else { continue };
            let desc = self.descs[i];
            self.pool.release(desc, pm);
        }
    }
}

fn clear_pixmap(pixmap: &mut vello_cpu::Pixmap, rgba: [u8; 4]) {
    for px in pixmap.data_as_u8_slice_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&rgba);
    }
}

fn premul_rgba8(rgba: [u8; 4]) -> [u8; 4] {
    let [r, g, b, a] = rgba;
    let a16 = u16::from(a);
    let premul = |c: u8| -> u8 { (((u16::from(c) * a16) + 127) / 255) as u8 };
    [premul(r), premul(g), premul(b), a]
}

fn clear_pixmap_to_transparent(pixmap: &mut vello_cpu::Pixmap) {
    pixmap.data_as_u8_slice_mut().fill(0);
}

fn affine_to_cpu(a: Affine) -> vello_cpu::kurbo::Affine {
    vello_cpu::kurbo::Affine::new(a.as_coeffs())
}

fn bezpath_to_cpu(path: &crate::foundation::core::BezPath) -> vello_cpu::kurbo::BezPath {
    use kurbo::PathEl;

    let mut out = vello_cpu::kurbo::BezPath::new();
    for &el in path.elements() {
        match el {
            PathEl::MoveTo(p) => out.move_to(vello_cpu::kurbo::Point::new(p.x, p.y)),
            PathEl::LineTo(p) => out.line_to(vello_cpu::kurbo::Point::new(p.x, p.y)),
            PathEl::QuadTo(p1, p2) => out.quad_to(
                vello_cpu::kurbo::Point::new(p1.x, p1.y),
                vello_cpu::kurbo::Point::new(p2.x, p2.y),
            ),
            PathEl::CurveTo(p1, p2, p3) => out.curve_to(
                vello_cpu::kurbo::Point::new(p1.x, p1.y),
                vello_cpu::kurbo::Point::new(p2.x, p2.y),
                vello_cpu::kurbo::Point::new(p3.x, p3.y),
            ),
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

fn pixmap_from_premul_bytes(
    bytes: &[u8],
    width: u32,
    height: u32,
) -> WavyteResult<vello_cpu::Pixmap> {
    let w: u16 = width
        .try_into()
        .map_err(|_| WavyteError::evaluation("pixmap width exceeds u16"))?;
    let h: u16 = height
        .try_into()
        .map_err(|_| WavyteError::evaluation("pixmap height exceeds u16"))?;
    if bytes.len()
        != (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4)
    {
        return Err(WavyteError::evaluation("pixmap byte len mismatch"));
    }
    // Pixmap stores PremulRgba8; our bytes are already premultiplied.
    let mut pixels = Vec::<vello_cpu::peniko::color::PremulRgba8>::with_capacity(
        (width as usize) * (height as usize),
    );
    for px in bytes.chunks_exact(4) {
        pixels.push(vello_cpu::peniko::color::PremulRgba8::from_u8_array([
            px[0], px[1], px[2], px[3],
        ]));
    }
    Ok(vello_cpu::Pixmap::from_parts_with_opacity(
        pixels, w, h, true,
    ))
}

fn premultiply_rgba8_in_place(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u16;
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }
        px[0] = ((px[0] as u16 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u16 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u16 * a + 127) / 255) as u8;
    }
}

fn rgba_premul_to_image(
    bytes_premul: &[u8],
    width: u32,
    height: u32,
) -> WavyteResult<vello_cpu::Image> {
    let pixmap = pixmap_from_premul_bytes(bytes_premul, width, height)?;
    Ok(vello_cpu::Image {
        image: vello_cpu::ImageSource::Pixmap(Arc::new(pixmap)),
        sampler: vello_cpu::peniko::ImageSampler::default(),
    })
}

fn rgba_straight_to_image_premul(
    bytes_rgba: &[u8],
    width: u32,
    height: u32,
) -> WavyteResult<vello_cpu::Image> {
    let mut tmp = bytes_rgba.to_vec();
    premultiply_rgba8_in_place(&mut tmp);
    rgba_premul_to_image(&tmp, width, height)
}

fn gaussian_kernel_q16(radius: u32, sigma: f32) -> WavyteResult<Vec<u32>> {
    if radius == 0 {
        return Ok(vec![1 << 16]);
    }
    if !sigma.is_finite() || sigma <= 0.0 {
        return Err(WavyteError::validation("blur sigma must be finite and > 0"));
    }

    let r = radius as i32;
    let mut weights_f = Vec::<f64>::with_capacity((2 * r + 1) as usize);
    let mut sum = 0.0f64;
    let sigma = sigma as f64;
    let denom = 2.0 * sigma * sigma;
    for i in -r..=r {
        let x = i as f64;
        let w = (-x * x / denom).exp();
        weights_f.push(w);
        sum += w;
    }
    if sum <= 0.0 {
        return Err(WavyteError::evaluation("gaussian kernel sum is zero"));
    }

    let mut weights = Vec::<u32>::with_capacity(weights_f.len());
    let mut acc: i64 = 0;
    for &wf in &weights_f {
        let q = ((wf / sum) * 65536.0).round() as i64;
        let q = q.clamp(0, 65536);
        weights.push(q as u32);
        acc += q;
    }
    let target: i64 = 65536;
    let delta = target - acc;
    if delta != 0 {
        let mid = weights.len() / 2;
        let mid_val = i64::from(weights[mid]);
        let new_mid = (mid_val + delta).clamp(0, 65536);
        weights[mid] = new_mid as u32;
    }

    Ok(weights)
}

fn blur_rgba8_premul_q16(
    src: &[u8],
    dst: &mut [u8],
    tmp: &mut [u8],
    width: u32,
    height: u32,
    kernel_q16: &[u32],
) {
    if kernel_q16.len() == 1 {
        dst.copy_from_slice(src);
        return;
    }

    horizontal_blur_q16(src, tmp, width, height, kernel_q16);
    vertical_blur_q16(tmp, dst, width, height, kernel_q16);
}

fn horizontal_blur_q16(src: &[u8], dst: &mut [u8], width: u32, height: u32, k: &[u32]) {
    let radius = (k.len() / 2) as i32;
    let w = width as i32;
    for y in 0..height as i32 {
        for x in 0..w {
            let mut acc = [0u64; 4];
            for (ki, &kw) in k.iter().enumerate() {
                let dx = ki as i32 - radius;
                let sx = (x + dx).clamp(0, w - 1);
                let idx = ((y * w + sx) as usize) * 4;
                for c in 0..4 {
                    acc[c] += (kw as u64) * (src[idx + c] as u64);
                }
            }
            let out_idx = ((y * w + x) as usize) * 4;
            for c in 0..4 {
                dst[out_idx + c] = q16_to_u8(acc[c]);
            }
        }
    }
}

fn vertical_blur_q16(src: &[u8], dst: &mut [u8], width: u32, height: u32, k: &[u32]) {
    let radius = (k.len() / 2) as i32;
    let w = width as i32;
    let h = height as i32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u64; 4];
            for (ki, &kw) in k.iter().enumerate() {
                let dy = ki as i32 - radius;
                let sy = (y + dy).clamp(0, h - 1);
                let idx = ((sy * w + x) as usize) * 4;
                for c in 0..4 {
                    acc[c] += (kw as u64) * (src[idx + c] as u64);
                }
            }
            let out_idx = ((y * w + x) as usize) * 4;
            for c in 0..4 {
                dst[out_idx + c] = q16_to_u8(acc[c]);
            }
        }
    }
}

fn q16_to_u8(acc: u64) -> u8 {
    let v = (acc + 32768) >> 16;
    (v.min(255)) as u8
}

fn mask_apply_rgba8_premul(
    src: &[u8],
    mask: &[u8],
    dst: &mut [u8],
    mode: crate::compile::plan::MaskMode,
    inverted: bool,
) {
    debug_assert_eq!(src.len(), mask.len());
    debug_assert_eq!(src.len(), dst.len());

    match mode {
        crate::compile::plan::MaskMode::Alpha => {
            for ((s, m), d) in src
                .chunks_exact(4)
                .zip(mask.chunks_exact(4))
                .zip(dst.chunks_exact_mut(4))
            {
                let mut w = m[3];
                if inverted {
                    w = 255 - w;
                }
                let w16 = u16::from(w);
                d[0] = mul_div255_u8(u16::from(s[0]), w16);
                d[1] = mul_div255_u8(u16::from(s[1]), w16);
                d[2] = mul_div255_u8(u16::from(s[2]), w16);
                d[3] = mul_div255_u8(u16::from(s[3]), w16);
            }
        }
        crate::compile::plan::MaskMode::Luma => {
            for ((s, m), d) in src
                .chunks_exact(4)
                .zip(mask.chunks_exact(4))
                .zip(dst.chunks_exact_mut(4))
            {
                let r = u16::from(m[0]);
                let g = u16::from(m[1]);
                let b = u16::from(m[2]);
                let luma = ((r * 54 + g * 183 + b * 19 + 128) >> 8) as u8;
                let mut w = luma;
                if inverted {
                    w = 255 - w;
                }
                let w16 = u16::from(w);
                d[0] = mul_div255_u8(u16::from(s[0]), w16);
                d[1] = mul_div255_u8(u16::from(s[1]), w16);
                d[2] = mul_div255_u8(u16::from(s[2]), w16);
                d[3] = mul_div255_u8(u16::from(s[3]), w16);
            }
        }
        crate::compile::plan::MaskMode::Stencil { threshold } => {
            let t = (threshold.clamp(0.0, 1.0) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            for ((s, m), d) in src
                .chunks_exact(4)
                .zip(mask.chunks_exact(4))
                .zip(dst.chunks_exact_mut(4))
            {
                let a = m[3];
                let mut w = if a >= t { 255u8 } else { 0u8 };
                if inverted {
                    w = 255 - w;
                }
                let w16 = u16::from(w);
                d[0] = mul_div255_u8(u16::from(s[0]), w16);
                d[1] = mul_div255_u8(u16::from(s[1]), w16);
                d[2] = mul_div255_u8(u16::from(s[2]), w16);
                d[3] = mul_div255_u8(u16::from(s[3]), w16);
            }
        }
    }
}

fn color_matrix_rgba8_premul(src: &[u8], dst: &mut [u8], m: [f32; 20]) {
    debug_assert_eq!(src.len(), dst.len());
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let pr = s[0] as f32 / 255.0;
        let pg = s[1] as f32 / 255.0;
        let pb = s[2] as f32 / 255.0;
        let pa = s[3] as f32 / 255.0;

        // Convert premul -> straight for matrix application.
        let inv_a = if pa > 0.0 { 1.0 / pa } else { 0.0 };
        let r = pr * inv_a;
        let g = pg * inv_a;
        let b = pb * inv_a;
        let a = pa;

        let out_r = (m[0] * r + m[1] * g + m[2] * b + m[3] * a + m[4]).clamp(0.0, 1.0);
        let out_g = (m[5] * r + m[6] * g + m[7] * b + m[8] * a + m[9]).clamp(0.0, 1.0);
        let out_b = (m[10] * r + m[11] * g + m[12] * b + m[13] * a + m[14]).clamp(0.0, 1.0);
        let out_a = (m[15] * r + m[16] * g + m[17] * b + m[18] * a + m[19]).clamp(0.0, 1.0);

        // Convert straight -> premul.
        let pr = (out_r * out_a).clamp(0.0, 1.0);
        let pg = (out_g * out_a).clamp(0.0, 1.0);
        let pb = (out_b * out_a).clamp(0.0, 1.0);

        d[0] = (pr * 255.0).round().clamp(0.0, 255.0) as u8;
        d[1] = (pg * 255.0).round().clamp(0.0, 255.0) as u8;
        d[2] = (pb * 255.0).round().clamp(0.0, 255.0) as u8;
        d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

fn premul_over_in_place_opacity(dst: &mut [u8], src: &[u8], opacity: f32) -> WavyteResult<()> {
    if dst.len() != src.len() || !dst.len().is_multiple_of(4) {
        return Err(WavyteError::evaluation(
            "premul_over_in_place_opacity expects equal-length rgba8 buffers",
        ));
    }
    let op = ((opacity.clamp(0.0, 1.0) * 255.0).round() as i32).clamp(0, 255) as u16;
    if op == 0 {
        return Ok(());
    }

    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let sa = mul_div255_u8(u16::from(s[3]), op);
        if sa == 0 {
            continue;
        }
        let inv = 255u16 - u16::from(sa);

        d[3] = add_sat_u8(sa, mul_div255_u8(u16::from(d[3]), inv));
        for c in 0..3 {
            let sc = mul_div255_u8(u16::from(s[c]), op);
            let dc = mul_div255_u8(u16::from(d[c]), inv);
            d[c] = add_sat_u8(sc, dc);
        }
    }
    Ok(())
}

fn composite_over_rgba8_premul(
    dst: &mut [u8],
    src: &[u8],
    opacity: f32,
    blend: crate::compile::plan::BlendMode,
) -> WavyteResult<()> {
    if dst.len() != src.len() || !dst.len().is_multiple_of(4) {
        return Err(WavyteError::evaluation(
            "composite_over_rgba8_premul expects equal-length rgba8 buffers",
        ));
    }

    // Perf contract: blend mode dispatch must be chosen once per op (not per pixel).
    // This match is outside the inner loops; each branch monomorphizes a specialized blend kernel.
    match blend {
        crate::compile::plan::BlendMode::Normal => premul_over_in_place_opacity(dst, src, opacity),
        crate::compile::plan::BlendMode::Multiply => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| s * d)
        }
        crate::compile::plan::BlendMode::Screen => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| s + d - s * d)
        }
        crate::compile::plan::BlendMode::Overlay => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| {
                if d <= 0.5 {
                    2.0 * s * d
                } else {
                    1.0 - 2.0 * (1.0 - s) * (1.0 - d)
                }
            })
        }
        crate::compile::plan::BlendMode::Darken => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| s.min(d))
        }
        crate::compile::plan::BlendMode::Lighten => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| s.max(d))
        }
        crate::compile::plan::BlendMode::ColorDodge => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| {
                if s >= 1.0 {
                    1.0
                } else {
                    (d / (1.0 - s)).min(1.0)
                }
            })
        }
        crate::compile::plan::BlendMode::ColorBurn => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| {
                if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - d) / s).min(1.0)
                }
            })
        }
        crate::compile::plan::BlendMode::SoftLight => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| {
                if s <= 0.5 {
                    d - (1.0 - 2.0 * s) * d * (1.0 - d)
                } else {
                    let g = if d <= 0.25 {
                        ((16.0 * d - 12.0) * d + 4.0) * d
                    } else {
                        d.sqrt()
                    };
                    d + (2.0 * s - 1.0) * (g - d)
                }
            })
        }
        crate::compile::plan::BlendMode::HardLight => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| {
                if s <= 0.5 {
                    2.0 * s * d
                } else {
                    1.0 - 2.0 * (1.0 - s) * (1.0 - d)
                }
            })
        }
        crate::compile::plan::BlendMode::Difference => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| (d - s).abs())
        }
        crate::compile::plan::BlendMode::Exclusion => {
            composite_over_rgba8_premul_blend(dst, src, opacity, |s, d| d + s - 2.0 * d * s)
        }
    }
}

#[inline(always)]
fn composite_over_rgba8_premul_blend<F>(
    dst: &mut [u8],
    src: &[u8],
    opacity: f32,
    blend_fn: F,
) -> WavyteResult<()>
where
    F: Fn(f32, f32) -> f32,
{
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return Ok(());
    }

    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        // Porter-Duff "source-over" with blend applied to unpremultiplied channels:
        // out_a = sa + da * (1 - sa)
        // out_p = sp * (1 - da) + dp * (1 - sa) + B(sc, dc) * sa * da

        // Premultiplied src scaled by op.
        let sp_r = (s[0] as f32 / 255.0) * opacity;
        let sp_g = (s[1] as f32 / 255.0) * opacity;
        let sp_b = (s[2] as f32 / 255.0) * opacity;
        let sa = (s[3] as f32 / 255.0) * opacity;

        let dp_r = d[0] as f32 / 255.0;
        let dp_g = d[1] as f32 / 255.0;
        let dp_b = d[2] as f32 / 255.0;
        let da = d[3] as f32 / 255.0;

        let inv_sa = 1.0 - sa;
        let out_a = (sa + da * inv_sa).clamp(0.0, 1.0);

        let sc_r = if sa > 0.0 {
            (sp_r / sa).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sc_g = if sa > 0.0 {
            (sp_g / sa).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sc_b = if sa > 0.0 {
            (sp_b / sa).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let dc_r = if da > 0.0 {
            (dp_r / da).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let dc_g = if da > 0.0 {
            (dp_g / da).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let dc_b = if da > 0.0 {
            (dp_b / da).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let b_r = blend_fn(sc_r, dc_r).clamp(0.0, 1.0);
        let b_g = blend_fn(sc_g, dc_g).clamp(0.0, 1.0);
        let b_b = blend_fn(sc_b, dc_b).clamp(0.0, 1.0);

        let out_p_r = (sp_r * (1.0 - da) + dp_r * (1.0 - sa) + b_r * sa * da).clamp(0.0, 1.0);
        let out_p_g = (sp_g * (1.0 - da) + dp_g * (1.0 - sa) + b_g * sa * da).clamp(0.0, 1.0);
        let out_p_b = (sp_b * (1.0 - da) + dp_b * (1.0 - sa) + b_b * sa * da).clamp(0.0, 1.0);

        d[0] = (out_p_r * 255.0).round().clamp(0.0, 255.0) as u8;
        d[1] = (out_p_g * 255.0).round().clamp(0.0, 255.0) as u8;
        d[2] = (out_p_b * 255.0).round().clamp(0.0, 255.0) as u8;
        d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    Ok(())
}

fn composite_crossfade_over_rgba8_premul(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    t: f32,
) -> WavyteResult<()> {
    if dst.len() != a.len() || dst.len() != b.len() || !dst.len().is_multiple_of(4) {
        return Err(WavyteError::evaluation(
            "composite_crossfade_over_rgba8_premul expects equal-length rgba8 buffers",
        ));
    }
    let t = t.clamp(0.0, 1.0);
    let tt = ((t * 255.0).round() as i32).clamp(0, 255) as u16;
    let it = 255u16 - tt;
    for ((d, ap), bp) in dst
        .chunks_exact_mut(4)
        .zip(a.chunks_exact(4))
        .zip(b.chunks_exact(4))
    {
        let mut src = [0u8; 4];
        for c in 0..4 {
            let av = mul_div255_u8(u16::from(ap[c]), it);
            let bv = mul_div255_u8(u16::from(bp[c]), tt);
            src[c] = add_sat_u8(av, bv);
        }
        // Normal over with opacity=1.
        let sa = src[3] as u16;
        if sa == 0 {
            continue;
        }
        let inv = 255u16 - sa;
        d[3] = add_sat_u8(src[3], mul_div255_u8(u16::from(d[3]), inv));
        for c in 0..3 {
            let dc = mul_div255_u8(u16::from(d[c]), inv);
            d[c] = add_sat_u8(src[c], dc);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn composite_wipe_over_rgba8_premul(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    t: f32,
    dir: crate::compile::plan::WipeDir,
    soft_edge: f32,
) -> WavyteResult<()> {
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if dst.len() != expected || a.len() != expected || b.len() != expected {
        return Err(WavyteError::evaluation(
            "composite_wipe_over_rgba8_premul expects buffers matching width*height*4",
        ));
    }
    let t = t.clamp(0.0, 1.0);
    let soft_edge = soft_edge.max(0.0);

    // Perf contract: direction dispatch is chosen once per op (not per pixel).
    let (axis_len, pos_base, pos_step, axis_is_x) = match dir {
        crate::compile::plan::WipeDir::LeftToRight => (width as f32, 0.0, 1.0, true),
        crate::compile::plan::WipeDir::RightToLeft => {
            (width as f32, (width.saturating_sub(1)) as f32, -1.0, true)
        }
        crate::compile::plan::WipeDir::TopToBottom => (height as f32, 0.0, 1.0, false),
        crate::compile::plan::WipeDir::BottomToTop => (
            height as f32,
            (height.saturating_sub(1)) as f32,
            -1.0,
            false,
        ),
    };

    let soft_px = soft_edge * axis_len.max(0.0);
    let edge = t * (axis_len + 2.0 * soft_px) - soft_px;
    let a_edge = edge - soft_px;
    let b_edge = edge + soft_px;

    // Dispatch the "soft edge" branch outside the pixel loop.
    if soft_px <= 0.0 {
        if axis_is_x {
            composite_wipe_x_hard(dst, a, b, width, height, edge, pos_base, pos_step);
        } else {
            composite_wipe_y_hard(dst, a, b, width, height, edge, pos_base, pos_step);
        }
    } else if axis_is_x {
        composite_wipe_x_soft(dst, a, b, width, height, a_edge, b_edge, pos_base, pos_step);
    } else {
        composite_wipe_y_soft(dst, a, b, width, height, a_edge, b_edge, pos_base, pos_step);
    }

    Ok(())
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_wipe_x_hard(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    edge: f32,
    pos_base: f32,
    pos_step: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let pos = pos_base + pos_step * (x as f32);
            let m_b = if pos < edge { 1.0 } else { 0.0 };
            composite_mix_over_at(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_wipe_x_soft(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    a_edge: f32,
    b_edge: f32,
    pos_base: f32,
    pos_step: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let pos = pos_base + pos_step * (x as f32);
            let m_b = smoothstep(a_edge, b_edge, pos);
            composite_mix_over_at(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_wipe_y_hard(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    edge: f32,
    pos_base: f32,
    pos_step: f32,
) {
    for y in 0..height {
        let pos = pos_base + pos_step * (y as f32);
        let m_b = if pos < edge { 1.0 } else { 0.0 };
        for x in 0..width {
            composite_mix_over_at(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_wipe_y_soft(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    a_edge: f32,
    b_edge: f32,
    pos_base: f32,
    pos_step: f32,
) {
    for y in 0..height {
        let pos = pos_base + pos_step * (y as f32);
        let m_b = smoothstep(a_edge, b_edge, pos);
        for x in 0..width {
            composite_mix_over_at(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
fn composite_mix_over_at(dst: &mut [u8], a: &[u8], b: &[u8], width: u32, x: u32, y: u32, m_b: f32) {
    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
    let ap = &a[idx..idx + 4];
    let bp = &b[idx..idx + 4];

    let tt = ((m_b.clamp(0.0, 1.0) * 255.0).round() as i32).clamp(0, 255) as u16;
    let it = 255u16 - tt;
    let mut src = [0u8; 4];
    for c in 0..4 {
        let av = mul_div255_u8(u16::from(ap[c]), it);
        let bv = mul_div255_u8(u16::from(bp[c]), tt);
        src[c] = add_sat_u8(av, bv);
    }

    let d = &mut dst[idx..idx + 4];
    let sa = src[3] as u16;
    if sa == 0 {
        return;
    }
    let inv = 255u16 - sa;
    d[3] = add_sat_u8(src[3], mul_div255_u8(u16::from(d[3]), inv));
    for c in 0..3 {
        let dc = mul_div255_u8(u16::from(d[c]), inv);
        d[c] = add_sat_u8(src[c], dc);
    }
}

#[allow(clippy::too_many_arguments)]
fn composite_slide_over_rgba8_premul(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    t: f32,
    dir: crate::compile::plan::SlideDir,
    push: bool,
) -> WavyteResult<()> {
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if dst.len() != expected || a.len() != expected || b.len() != expected {
        return Err(WavyteError::evaluation(
            "composite_slide_over_rgba8_premul expects buffers matching width*height*4",
        ));
    }
    let t = t.clamp(0.0, 1.0);
    let w = width as f32;
    let h = height as f32;

    let (b_dx, b_dy) = match dir {
        crate::compile::plan::SlideDir::Left => ((1.0 - t) * w, 0.0),
        crate::compile::plan::SlideDir::Right => (-(1.0 - t) * w, 0.0),
        crate::compile::plan::SlideDir::Up => (0.0, (1.0 - t) * h),
        crate::compile::plan::SlideDir::Down => (0.0, -(1.0 - t) * h),
    };
    let (a_dx, a_dy) = if push {
        match dir {
            crate::compile::plan::SlideDir::Left => (-t * w, 0.0),
            crate::compile::plan::SlideDir::Right => (t * w, 0.0),
            crate::compile::plan::SlideDir::Up => (0.0, -t * h),
            crate::compile::plan::SlideDir::Down => (0.0, t * h),
        }
    } else {
        (0.0, 0.0)
    };

    for y in 0..height {
        for x in 0..width {
            let xf = x as f32;
            let yf = y as f32;
            let ax = (xf - a_dx).round() as i32;
            let ay = (yf - a_dy).round() as i32;
            let bx = (xf - b_dx).round() as i32;
            let by = (yf - b_dy).round() as i32;

            let ap = sample_px(a, width, height, ax, ay);
            let bp = sample_px(b, width, height, bx, by);
            let layer = premul_over_px(ap, bp); // b over a

            let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
            let d = &mut dst[idx..idx + 4];
            let out = premul_over_px([d[0], d[1], d[2], d[3]], layer);
            d.copy_from_slice(&out);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn composite_zoom_over_rgba8_premul(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    t: f32,
    origin: crate::foundation::core::Vec2,
    from_scale: f32,
) -> WavyteResult<()> {
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if dst.len() != expected || a.len() != expected || b.len() != expected {
        return Err(WavyteError::evaluation(
            "composite_zoom_over_rgba8_premul expects buffers matching width*height*4",
        ));
    }
    let t = t.clamp(0.0, 1.0);
    let s = (from_scale + (1.0 - from_scale) * t).max(1e-6);
    let ox = origin.x as f32 * (width as f32);
    let oy = origin.y as f32 * (height as f32);

    for y in 0..height {
        for x in 0..width {
            let xf = x as f32;
            let yf = y as f32;
            let bx = ox + (xf - ox) / s;
            let by = oy + (yf - oy) / s;
            let bp = sample_px(b, width, height, bx.round() as i32, by.round() as i32);
            let ap = sample_px(a, width, height, x as i32, y as i32);

            // Crossfade + zoom.
            let tt = ((t * 255.0).round() as i32).clamp(0, 255) as u16;
            let it = 255u16 - tt;
            let mut layer = [0u8; 4];
            for c in 0..4 {
                let av = mul_div255_u8(u16::from(ap[c]), it);
                let bv = mul_div255_u8(u16::from(bp[c]), tt);
                layer[c] = add_sat_u8(av, bv);
            }

            let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
            let d = &mut dst[idx..idx + 4];
            let out = premul_over_px([d[0], d[1], d[2], d[3]], layer);
            d.copy_from_slice(&out);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn composite_iris_over_rgba8_premul(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    t: f32,
    origin: crate::foundation::core::Vec2,
    shape: crate::compile::plan::IrisShape,
    soft_edge: f32,
) -> WavyteResult<()> {
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if dst.len() != expected || a.len() != expected || b.len() != expected {
        return Err(WavyteError::evaluation(
            "composite_iris_over_rgba8_premul expects buffers matching width*height*4",
        ));
    }
    let t = t.clamp(0.0, 1.0);

    let ox = origin.x as f32 * (width as f32);
    let oy = origin.y as f32 * (height as f32);
    let max_x = ox.max((width as f32) - ox);
    let max_y = oy.max((height as f32) - oy);

    // Perf contract: shape dispatch is chosen once per op (not per pixel).
    let max_dist = match shape {
        crate::compile::plan::IrisShape::Circle => (max_x * max_x + max_y * max_y).sqrt(),
        crate::compile::plan::IrisShape::Rect => max_x.max(max_y),
        crate::compile::plan::IrisShape::Diamond => max_x + max_y,
    }
    .max(1e-6);
    let soft = (soft_edge.max(0.0) * max_dist).max(0.0);
    let edge = t * (max_dist + 2.0 * soft) - soft;

    if soft <= 0.0 {
        match shape {
            crate::compile::plan::IrisShape::Circle => {
                composite_iris_circle_hard(dst, a, b, width, height, ox, oy, edge)
            }
            crate::compile::plan::IrisShape::Rect => {
                composite_iris_rect_hard(dst, a, b, width, height, ox, oy, edge)
            }
            crate::compile::plan::IrisShape::Diamond => {
                composite_iris_diamond_hard(dst, a, b, width, height, ox, oy, edge)
            }
        }
    } else {
        let a_edge = edge - soft;
        let b_edge = edge + soft;
        match shape {
            crate::compile::plan::IrisShape::Circle => {
                composite_iris_circle_soft(dst, a, b, width, height, ox, oy, a_edge, b_edge)
            }
            crate::compile::plan::IrisShape::Rect => {
                composite_iris_rect_soft(dst, a, b, width, height, ox, oy, a_edge, b_edge)
            }
            crate::compile::plan::IrisShape::Diamond => {
                composite_iris_diamond_soft(dst, a, b, width, height, ox, oy, a_edge, b_edge)
            }
        }
    }

    Ok(())
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_iris_circle_hard(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ox: f32,
    oy: f32,
    edge: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = (x as f32) - ox;
            let dy = (y as f32) - oy;
            let dist = (dx * dx + dy * dy).sqrt();
            let m_b = if dist <= edge { 1.0 } else { 0.0 };
            composite_mix_over_px(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_iris_circle_soft(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ox: f32,
    oy: f32,
    a_edge: f32,
    b_edge: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = (x as f32) - ox;
            let dy = (y as f32) - oy;
            let dist = (dx * dx + dy * dy).sqrt();
            let m_b = 1.0 - smoothstep(a_edge, b_edge, dist);
            composite_mix_over_px(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_iris_rect_hard(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ox: f32,
    oy: f32,
    edge: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = ((x as f32) - ox).abs();
            let dy = ((y as f32) - oy).abs();
            let dist = dx.max(dy);
            let m_b = if dist <= edge { 1.0 } else { 0.0 };
            composite_mix_over_px(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_iris_rect_soft(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ox: f32,
    oy: f32,
    a_edge: f32,
    b_edge: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = ((x as f32) - ox).abs();
            let dy = ((y as f32) - oy).abs();
            let dist = dx.max(dy);
            let m_b = 1.0 - smoothstep(a_edge, b_edge, dist);
            composite_mix_over_px(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_iris_diamond_hard(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ox: f32,
    oy: f32,
    edge: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = ((x as f32) - ox).abs();
            let dy = ((y as f32) - oy).abs();
            let dist = dx + dy;
            let m_b = if dist <= edge { 1.0 } else { 0.0 };
            composite_mix_over_px(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn composite_iris_diamond_soft(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    width: u32,
    height: u32,
    ox: f32,
    oy: f32,
    a_edge: f32,
    b_edge: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = ((x as f32) - ox).abs();
            let dy = ((y as f32) - oy).abs();
            let dist = dx + dy;
            let m_b = 1.0 - smoothstep(a_edge, b_edge, dist);
            composite_mix_over_px(dst, a, b, width, x, y, m_b);
        }
    }
}

#[inline(always)]
fn composite_mix_over_px(dst: &mut [u8], a: &[u8], b: &[u8], width: u32, x: u32, y: u32, m_b: f32) {
    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
    let ap = &a[idx..idx + 4];
    let bp = &b[idx..idx + 4];

    let tt = ((m_b.clamp(0.0, 1.0) * 255.0).round() as i32).clamp(0, 255) as u16;
    let it = 255u16 - tt;
    let mut layer = [0u8; 4];
    for c in 0..4 {
        let av = mul_div255_u8(u16::from(ap[c]), it);
        let bv = mul_div255_u8(u16::from(bp[c]), tt);
        layer[c] = add_sat_u8(av, bv);
    }

    let d = &mut dst[idx..idx + 4];
    let out = premul_over_px([d[0], d[1], d[2], d[3]], layer);
    d.copy_from_slice(&out);
}

fn premul_over_px(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = src[3] as u16;
    if sa == 0 {
        return dst;
    }
    let inv = 255u16 - sa;
    let mut out = [0u8; 4];
    out[3] = add_sat_u8(src[3], mul_div255_u8(u16::from(dst[3]), inv));
    for c in 0..3 {
        let dc = mul_div255_u8(u16::from(dst[c]), inv);
        out[c] = add_sat_u8(src[c], dc);
    }
    out
}

fn sample_px(src: &[u8], width: u32, height: u32, x: i32, y: i32) -> [u8; 4] {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return [0, 0, 0, 0];
    }
    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
    [src[idx], src[idx + 1], src[idx + 2], src[idx + 3]]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if x <= a {
        return 0.0;
    }
    if x >= b {
        return 1.0;
    }
    let t = (x - a) / (b - a);
    (t * t * (3.0 - 2.0 * t)).clamp(0.0, 1.0)
}

fn premul_over_in_place(dst: &mut [u8], src: &[u8]) -> WavyteResult<()> {
    if dst.len() != src.len() || !dst.len().is_multiple_of(4) {
        return Err(WavyteError::evaluation(
            "premul_over_in_place expects equal-length rgba8 buffers",
        ));
    }
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let sa = s[3] as u16;
        if sa == 0 {
            continue;
        }
        let inv = 255u16 - sa;
        d[3] = add_sat_u8(sa as u8, mul_div255_u8(d[3] as u16, inv));
        for c in 0..3 {
            let dc = mul_div255_u8(d[c] as u16, inv);
            d[c] = add_sat_u8(s[c], dc);
        }
    }
    Ok(())
}

fn mul_div255_u8(x: u16, y: u16) -> u8 {
    crate::foundation::math::mul_div255_u8(x, y)
}

fn add_sat_u8(a: u8, b: u8) -> u8 {
    a.saturating_add(b)
}

fn hash_u32(seed: u64, x: u32, y: u32) -> u32 {
    let mut h = crate::foundation::math::Fnv1a64::new(
        seed ^ crate::foundation::math::Fnv1a64::OFFSET_BASIS,
    );
    h.write_u64(u64::from(x));
    h.write_u64(u64::from(y));
    (h.finish() & 0xFFFF_FFFF) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::anim::AnimDef;
    use crate::compile::compiler::compile_frame;
    use crate::eval::evaluator::Evaluator;
    use crate::expression::compile::compile_expr_program;
    use crate::normalize::pass::normalize;
    use crate::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, FpsDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn repeat_px(px: [u8; 4], n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n.saturating_mul(4)];
        for c in out.chunks_exact_mut(4) {
            c.copy_from_slice(&px);
        }
        out
    }

    #[test]
    fn v03_cpu_backend_renders_single_image_leaf() {
        let mut assets = BTreeMap::new();
        assets.insert(
            "img".to_owned(),
            AssetDef::Image {
                source: "assets/test_image_1.jpg".to_owned(),
            },
        );

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 64,
                height: 64,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 1,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Group,
                    children: vec![NodeDef {
                        id: "a".to_owned(),
                        kind: NodeKindDef::Leaf {
                            asset: "img".to_owned(),
                        },
                        range: [0, 1],
                        transform: Default::default(),
                        opacity: AnimDef::Constant(1.0),
                        layout: None,
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    }],
                },
                range: [0, 1],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let expr = compile_expr_program(&norm).unwrap();
        let mut eval = Evaluator::new(expr);
        let g = eval.eval_frame(&norm.ir, 0).unwrap();
        let plan = compile_frame(&norm.ir, g);

        let assets_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .to_path_buf();
        let mut backend = CpuBackendV03::new(assets_root, CpuBackendOpts::default());
        let frame = backend
            .render_plan(&norm.ir, &norm.interner, g, &plan)
            .unwrap();

        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 64);
        assert!(frame.data.iter().any(|&b| b != 0));
    }

    #[test]
    fn blur_radius_0_is_identity() {
        let w = 4u32;
        let h = 3u32;
        let mut src = vec![0u8; (w as usize) * (h as usize) * 4];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31);
        }
        let mut dst = vec![0u8; src.len()];
        let mut tmp = vec![0u8; src.len()];
        let k = gaussian_kernel_q16(0, 1.0).unwrap();
        blur_rgba8_premul_q16(&src, &mut dst, &mut tmp, w, h, &k);
        assert_eq!(src, dst);
    }

    #[test]
    fn blur_constant_image_is_identity() {
        let w = 5u32;
        let h = 5u32;
        let mut src = vec![0u8; (w as usize) * (h as usize) * 4];
        for px in src.chunks_exact_mut(4) {
            px.copy_from_slice(&[10, 20, 30, 40]);
        }
        let mut dst = vec![0u8; src.len()];
        let mut tmp = vec![0u8; src.len()];
        let k = gaussian_kernel_q16(2, 1.0).unwrap();
        blur_rgba8_premul_q16(&src, &mut dst, &mut tmp, w, h, &k);
        assert_eq!(src, dst);
    }

    #[test]
    fn mask_apply_alpha_scales_src_by_mask_alpha() {
        let src = vec![10u8, 20, 30, 40];
        let mask = vec![0u8, 0, 0, 128];
        let mut dst = vec![0u8; 4];
        mask_apply_rgba8_premul(
            &src,
            &mask,
            &mut dst,
            crate::compile::plan::MaskMode::Alpha,
            false,
        );
        let w = 128u16;
        assert_eq!(dst[0], mul_div255_u8(u16::from(10u8), w));
        assert_eq!(dst[1], mul_div255_u8(u16::from(20u8), w));
        assert_eq!(dst[2], mul_div255_u8(u16::from(30u8), w));
        assert_eq!(dst[3], mul_div255_u8(u16::from(40u8), w));
    }

    #[test]
    fn mask_apply_stencil_threshold_selects_all_or_nothing() {
        let src = vec![10u8, 20, 30, 200];
        let mask_lo = vec![0u8, 0, 0, 100];
        let mask_hi = vec![0u8, 0, 0, 200];
        let mut dst = vec![0u8; 4];

        mask_apply_rgba8_premul(
            &src,
            &mask_lo,
            &mut dst,
            crate::compile::plan::MaskMode::Stencil { threshold: 0.5 },
            false,
        );
        assert_eq!(dst, vec![0, 0, 0, 0]);

        mask_apply_rgba8_premul(
            &src,
            &mask_hi,
            &mut dst,
            crate::compile::plan::MaskMode::Stencil { threshold: 0.5 },
            false,
        );
        assert_eq!(dst, src);
    }

    #[test]
    fn color_matrix_identity_is_identity() {
        let src = vec![10u8, 20, 30, 40, 50, 60, 70, 80];
        let mut dst = vec![0u8; src.len()];
        let id = [
            1.0, 0.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0, //
        ];
        color_matrix_rgba8_premul(&src, &mut dst, id);
        assert_eq!(src, dst);
    }

    #[test]
    fn color_matrix_zero_alpha_makes_pixels_transparent() {
        let src = vec![10u8, 20, 30, 255];
        let mut dst = vec![0u8; src.len()];
        let zero_a = [
            1.0, 0.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 0.0, 0.0, //
        ];
        color_matrix_rgba8_premul(&src, &mut dst, zero_a);
        assert_eq!(dst, vec![0, 0, 0, 0]);
    }

    #[test]
    fn composite_over_multiply_opaque_is_multiply() {
        let mut dst = vec![128u8, 128, 128, 255];
        let src = vec![128u8, 0, 0, 255];
        composite_over_rgba8_premul(
            &mut dst,
            &src,
            1.0,
            crate::compile::plan::BlendMode::Multiply,
        )
        .unwrap();
        assert_eq!(dst, vec![64, 0, 0, 255]);
    }

    #[test]
    fn composite_crossfade_endpoints() {
        let a = vec![0u8, 0, 0, 255];
        let b = vec![255u8, 0, 0, 255];

        let mut dst = vec![0u8; 4];
        composite_crossfade_over_rgba8_premul(&mut dst, &a, &b, 0.0).unwrap();
        assert_eq!(dst, a);

        let mut dst = vec![0u8; 4];
        composite_crossfade_over_rgba8_premul(&mut dst, &a, &b, 1.0).unwrap();
        assert_eq!(dst, b);
    }

    #[test]
    fn composite_wipe_endpoints_hard_edge() {
        let w = 4u32;
        let h = 1u32;
        let a = repeat_px([0u8, 0, 0, 255], w as usize);
        let b = repeat_px([255u8, 0, 0, 255], w as usize);

        let mut dst = vec![0u8; (w as usize) * 4];
        composite_wipe_over_rgba8_premul(
            &mut dst,
            &a,
            &b,
            w,
            h,
            0.0,
            crate::compile::plan::WipeDir::LeftToRight,
            0.0,
        )
        .unwrap();
        assert_eq!(dst, a);

        let mut dst = vec![0u8; (w as usize) * 4];
        composite_wipe_over_rgba8_premul(
            &mut dst,
            &a,
            &b,
            w,
            h,
            1.0,
            crate::compile::plan::WipeDir::LeftToRight,
            0.0,
        )
        .unwrap();
        assert_eq!(dst, b);
    }

    #[test]
    fn composite_slide_endpoints_no_push() {
        let w = 2u32;
        let h = 1u32;
        let mut a = vec![0u8; (w as usize) * 4];
        a[0..4].copy_from_slice(&[0, 255, 0, 255]);
        let mut b = vec![0u8; (w as usize) * 4];
        b[0..4].copy_from_slice(&[255, 0, 0, 255]);

        let mut dst = vec![0u8; (w as usize) * 4];
        composite_slide_over_rgba8_premul(
            &mut dst,
            &a,
            &b,
            w,
            h,
            0.0,
            crate::compile::plan::SlideDir::Left,
            false,
        )
        .unwrap();
        assert_eq!(dst, a);

        let mut dst = vec![0u8; (w as usize) * 4];
        composite_slide_over_rgba8_premul(
            &mut dst,
            &a,
            &b,
            w,
            h,
            1.0,
            crate::compile::plan::SlideDir::Left,
            false,
        )
        .unwrap();
        assert_eq!(dst, b);
    }

    #[test]
    fn composite_iris_endpoints_circle() {
        let w = 3u32;
        let h = 3u32;
        let a = repeat_px([0u8, 0, 0, 255], (w * h) as usize);
        let b = repeat_px([255u8, 0, 0, 255], (w * h) as usize);

        let mut dst = vec![0u8; (w * h * 4) as usize];
        composite_iris_over_rgba8_premul(
            &mut dst,
            &a,
            &b,
            w,
            h,
            0.0,
            crate::foundation::core::Vec2 { x: 0.5, y: 0.5 },
            crate::compile::plan::IrisShape::Circle,
            0.0,
        )
        .unwrap();
        assert_eq!(dst, a);

        let mut dst = vec![0u8; (w * h * 4) as usize];
        composite_iris_over_rgba8_premul(
            &mut dst,
            &a,
            &b,
            w,
            h,
            1.0,
            crate::foundation::core::Vec2 { x: 0.5, y: 0.5 },
            crate::compile::plan::IrisShape::Circle,
            0.0,
        )
        .unwrap();
        assert_eq!(dst, b);
    }

    #[test]
    fn surface_pool_retained_bytes_plateau_after_warmup() {
        let mut assets = BTreeMap::new();
        assets.insert("solid".to_owned(), AssetDef::SolidRect { color: None });

        let frames = 300u64;
        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 64,
                height: 64,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: frames,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Group,
                    children: vec![NodeDef {
                        id: "a".to_owned(),
                        kind: NodeKindDef::Leaf {
                            asset: "solid".to_owned(),
                        },
                        range: [0, frames],
                        transform: Default::default(),
                        opacity: AnimDef::Constant(1.0),
                        layout: None,
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    }],
                },
                range: [0, frames],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let expr = compile_expr_program(&norm).unwrap();
        let mut eval = Evaluator::new(expr);

        let assets_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .to_path_buf();
        let mut backend = CpuBackendV03::new(assets_root, CpuBackendOpts::default());

        let mut retained = Vec::<usize>::with_capacity(frames as usize);
        for f in 0..frames {
            let g = eval.eval_frame(&norm.ir, f).unwrap();
            let plan = compile_frame(&norm.ir, g);
            let _ = backend
                .render_plan(&norm.ir, &norm.interner, g, &plan)
                .unwrap();
            let st = backend.pool.as_ref().unwrap().stats();
            retained.push(st.retained_bytes);
        }

        let warmup = 10usize.min(retained.len());
        let steady = &retained[warmup..];
        if steady.is_empty() {
            return;
        }
        let min = *steady.iter().min().unwrap();
        let max = *steady.iter().max().unwrap();
        assert_eq!(
            min, max,
            "surface pool retained bytes should plateau after warmup (min={min}, max={max})"
        );
    }
}
