use std::collections::HashMap;

use crate::{
    assets::{AssetCache, AssetId, PreparedAsset},
    compile::{DrawOp, SurfaceDesc, SurfaceId},
    error::{WavyteError, WavyteResult},
    render::{FrameRGBA, RenderBackend, RenderSettings},
    render_passes::PassBackend,
};

pub struct VelloBackend {
    settings: RenderSettings,

    device: Option<vello::wgpu::Device>,
    queue: Option<vello::wgpu::Queue>,
    renderer: Option<vello::Renderer>,
    scene: vello::Scene,

    target_texture: Option<vello::wgpu::Texture>,
    target_view: Option<vello::wgpu::TextureView>,
    readback: Option<vello::wgpu::Buffer>,
    readback_bytes_per_row: u32,
    width: u32,
    height: u32,

    image_cache: HashMap<AssetId, vello::peniko::ImageData>,
    font_cache: HashMap<AssetId, vello::peniko::FontData>,
}

impl VelloBackend {
    pub fn new(settings: RenderSettings) -> WavyteResult<Self> {
        Ok(Self {
            settings,
            device: None,
            queue: None,
            renderer: None,
            scene: vello::Scene::new(),
            target_texture: None,
            target_view: None,
            readback: None,
            readback_bytes_per_row: 0,
            width: 0,
            height: 0,
            image_cache: HashMap::new(),
            font_cache: HashMap::new(),
        })
    }

    fn ensure_init(&mut self, width: u32, height: u32) -> WavyteResult<()> {
        if self.device.is_some() && self.width == width && self.height == height {
            return Ok(());
        }

        let instance = vello::wgpu::Instance::new(&vello::wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(
            &vello::wgpu::RequestAdapterOptions {
                power_preference: vello::wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            },
        ))
        .map_err(|e| match e {
            vello::wgpu::RequestAdapterError::NotFound { .. } => {
                WavyteError::evaluation("no gpu adapter available")
            }
            other => WavyteError::evaluation(format!("wgpu request_adapter failed: {other:?}")),
        })?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&vello::wgpu::DeviceDescriptor {
                label: None,
                required_features: vello::wgpu::Features::empty(),
                required_limits: vello::wgpu::Limits::default(),
                experimental_features: vello::wgpu::ExperimentalFeatures::default(),
                memory_hints: vello::wgpu::MemoryHints::Performance,
                trace: vello::wgpu::Trace::Off,
            }))
            .map_err(|e| WavyteError::evaluation(format!("wgpu request_device failed: {e:?}")))?;

        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(|e| WavyteError::evaluation(format!("vello renderer init failed: {e:?}")))?;

        let texture = device.create_texture(&vello::wgpu::TextureDescriptor {
            label: Some("wavyte_render_target"),
            size: vello::wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: vello::wgpu::TextureDimension::D2,
            format: vello::wgpu::TextureFormat::Rgba8Unorm,
            usage: vello::wgpu::TextureUsages::STORAGE_BINDING
                | vello::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&vello::wgpu::TextureViewDescriptor::default());

        let bytes_per_row_unpadded = width
            .checked_mul(4)
            .ok_or_else(|| WavyteError::evaluation("render target width overflow"))?;
        let bytes_per_row = align_to(
            bytes_per_row_unpadded,
            vello::wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
        );
        let buffer_size = (bytes_per_row as u64)
            .checked_mul(height as u64)
            .ok_or_else(|| WavyteError::evaluation("readback buffer size overflow"))?;

        let readback = device.create_buffer(&vello::wgpu::BufferDescriptor {
            label: Some("wavyte_readback"),
            size: buffer_size,
            usage: vello::wgpu::BufferUsages::MAP_READ | vello::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.device = Some(device);
        self.queue = Some(queue);
        self.renderer = Some(renderer);
        self.target_texture = Some(texture);
        self.target_view = Some(view);
        self.readback = Some(readback);
        self.readback_bytes_per_row = bytes_per_row;
        self.width = width;
        self.height = height;
        self.image_cache.clear();
        self.font_cache.clear();
        Ok(())
    }
}

impl PassBackend for VelloBackend {
    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()> {
        if id != SurfaceId(0) {
            return Ok(());
        }
        self.ensure_init(desc.width, desc.height)?;
        self.scene.reset();
        Ok(())
    }

    fn exec_scene(
        &mut self,
        pass: &crate::compile::ScenePass,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<()> {
        let _ = pass.target;
        let _ = pass.clear_to_transparent;
        for op in &pass.ops {
            encode_op(self, op, self.width, self.height, assets)?;
        }
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
                "gpu backend readback is only supported for surface 0 in this phase",
            ));
        }

        let device = self
            .device
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let target_view = self
            .target_view
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let readback = self
            .readback
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let target_tex = self
            .target_texture
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;

        let base_color = match self.settings.clear_rgba {
            Some([r, g, b, a]) => vello::peniko::Color::from_rgba8(r, g, b, a),
            None => vello::peniko::Color::from_rgba8(0, 0, 0, 0),
        };

        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        renderer
            .render_to_texture(
                device,
                queue,
                &self.scene,
                target_view,
                &vello::RenderParams {
                    base_color,
                    width: plan.canvas.width,
                    height: plan.canvas.height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .map_err(|e| WavyteError::evaluation(format!("vello render failed: {e:?}")))?;

        let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
            label: Some("wavyte_readback_encoder"),
        });
        encoder.copy_texture_to_buffer(
            vello::wgpu::TexelCopyTextureInfo {
                texture: target_tex,
                mip_level: 0,
                origin: vello::wgpu::Origin3d::ZERO,
                aspect: vello::wgpu::TextureAspect::All,
            },
            vello::wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: vello::wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.readback_bytes_per_row),
                    rows_per_image: Some(plan.canvas.height),
                },
            },
            vello::wgpu::Extent3d {
                width: plan.canvas.width,
                height: plan.canvas.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let buffer_slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(vello::wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        device
            .poll(vello::wgpu::PollType::wait_indefinitely())
            .map_err(|e| WavyteError::evaluation(format!("wgpu poll failed: {e:?}")))?;
        rx.recv()
            .map_err(|_| WavyteError::evaluation("readback channel closed"))?
            .map_err(|e| WavyteError::evaluation(format!("readback map failed: {e:?}")))?;

        let mapped = buffer_slice.get_mapped_range();
        let row_bytes = (plan.canvas.width as usize) * 4;
        let padded_row_bytes = self.readback_bytes_per_row as usize;
        let mut out = Vec::with_capacity(row_bytes * plan.canvas.height as usize);
        for row in 0..plan.canvas.height as usize {
            let start = row * padded_row_bytes;
            out.extend_from_slice(&mapped[start..start + row_bytes]);
        }
        drop(mapped);
        readback.unmap();

        Ok(FrameRGBA {
            width: plan.canvas.width,
            height: plan.canvas.height,
            data: out,
            premultiplied: true,
        })
    }
}

impl RenderBackend for VelloBackend {}

fn align_to(value: u32, alignment: u32) -> u32 {
    let mask = alignment - 1;
    (value + mask) & !mask
}

fn clip_rect(width: u32, height: u32) -> kurbo::Rect {
    kurbo::Rect::new(0.0, 0.0, width as f64, height as f64)
}

fn encode_op(
    backend: &mut VelloBackend,
    op: &DrawOp,
    canvas_w: u32,
    canvas_h: u32,
    assets: &mut dyn AssetCache,
) -> WavyteResult<()> {
    use vello::peniko::{BlendMode, Fill};

    match op {
        DrawOp::FillPath {
            path,
            transform,
            color,
            opacity,
            blend: _,
            z: _,
        } => {
            if *opacity < 1.0 {
                backend.scene.push_layer(
                    Fill::NonZero,
                    BlendMode::default(),
                    *opacity,
                    kurbo::Affine::IDENTITY,
                    &clip_rect(canvas_w, canvas_h),
                );
            }
            backend.scene.fill(
                Fill::NonZero,
                *transform,
                vello::peniko::Color::from_rgba8(color.r, color.g, color.b, color.a),
                None,
                path,
            );
            if *opacity < 1.0 {
                backend.scene.pop_layer();
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
            if *opacity < 1.0 {
                backend.scene.push_layer(
                    Fill::NonZero,
                    BlendMode::default(),
                    *opacity,
                    kurbo::Affine::IDENTITY,
                    &clip_rect(canvas_w, canvas_h),
                );
            }

            let img = backend.image_for(*asset, assets)?;
            backend.scene.draw_image(&img, *transform);

            if *opacity < 1.0 {
                backend.scene.pop_layer();
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
            if *opacity < 1.0 {
                backend.scene.push_layer(
                    Fill::NonZero,
                    BlendMode::default(),
                    *opacity,
                    kurbo::Affine::IDENTITY,
                    &clip_rect(canvas_w, canvas_h),
                );
            }

            let prepared = assets.get_or_load_by_id(*asset)?;
            let PreparedAsset::Svg(svg) = prepared else {
                return Err(WavyteError::evaluation("AssetId is not a PreparedSvg"));
            };
            let svg_scene = vello_svg::render_tree(&svg.tree);
            backend.scene.append(&svg_scene, Some(*transform));

            if *opacity < 1.0 {
                backend.scene.pop_layer();
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

            if *opacity < 1.0 {
                backend.scene.push_layer(
                    Fill::NonZero,
                    BlendMode::default(),
                    *opacity,
                    kurbo::Affine::IDENTITY,
                    &clip_rect(canvas_w, canvas_h),
                );
            }

            for line in t.layout.lines() {
                for item in line.items() {
                    let parley::layout::PositionedLayoutItem::GlyphRun(run) = item else {
                        continue;
                    };
                    let brush = run.style().brush;
                    backend
                        .scene
                        .draw_glyphs(&font)
                        .transform(*transform)
                        .font_size(run.run().font_size())
                        .brush(vello::peniko::Color::from_rgba8(
                            brush.r, brush.g, brush.b, brush.a,
                        ))
                        .draw(
                            Fill::NonZero,
                            run.glyphs().map(|g| vello::Glyph {
                                id: g.id,
                                x: g.x,
                                y: g.y,
                            }),
                        );
                }
            }

            if *opacity < 1.0 {
                backend.scene.pop_layer();
            }

            Ok(())
        }
    }
}

impl VelloBackend {
    fn image_for(
        &mut self,
        id: AssetId,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<vello::peniko::ImageData> {
        if let Some(img) = self.image_cache.get(&id) {
            return Ok(img.clone());
        }
        let prepared = assets.get_or_load_by_id(id)?;
        let PreparedAsset::Image(img) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedImage"));
        };

        let data = vello::peniko::Blob::from(img.rgba8_premul.as_ref().clone());
        let image = vello::peniko::ImageData {
            data,
            format: vello::peniko::ImageFormat::Rgba8,
            alpha_type: vello::peniko::ImageAlphaType::AlphaPremultiplied,
            width: img.width,
            height: img.height,
        };
        self.image_cache.insert(id, image.clone());
        Ok(image)
    }

    fn font_for_text_asset(
        &mut self,
        id: AssetId,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<vello::peniko::FontData> {
        if let Some(font) = self.font_cache.get(&id) {
            return Ok(font.clone());
        }

        let prepared = assets.get_or_load_by_id(id)?;
        let PreparedAsset::Text(t) = prepared else {
            return Err(WavyteError::evaluation("AssetId is not a PreparedText"));
        };

        let bytes: Vec<u8> = t.font_bytes.as_ref().clone();
        let font = vello::peniko::FontData::new(vello::peniko::Blob::from(bytes), 0);
        self.font_cache.insert(id, font.clone());
        Ok(font)
    }
}
