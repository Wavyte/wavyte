use std::collections::HashMap;

use crate::{
    assets::{AssetCache, AssetId, PreparedAsset},
    compile::{CompositeOp, DrawOp, SurfaceDesc, SurfaceId},
    error::{WavyteError, WavyteResult},
    render::{FrameRGBA, RenderBackend, RenderSettings},
    render_passes::PassBackend,
};

struct GpuSurface {
    width: u32,
    height: u32,
    texture: vello::wgpu::Texture,
    view: vello::wgpu::TextureView,
}

struct Compositor {
    pipeline: vello::wgpu::RenderPipeline,
    bind_group_layout: vello::wgpu::BindGroupLayout,
    sampler: vello::wgpu::Sampler,
    params: vello::wgpu::Buffer,
}

pub struct VelloBackend {
    settings: RenderSettings,

    device: Option<vello::wgpu::Device>,
    queue: Option<vello::wgpu::Queue>,
    renderer: Option<vello::Renderer>,
    scene: vello::Scene,

    readback: Option<vello::wgpu::Buffer>,
    readback_bytes_per_row: u32,
    width: u32,
    height: u32,

    surfaces: HashMap<SurfaceId, GpuSurface>,
    compositor: Option<Compositor>,

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
            readback: None,
            readback_bytes_per_row: 0,
            width: 0,
            height: 0,
            surfaces: HashMap::new(),
            compositor: None,
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
        self.readback = Some(readback);
        self.readback_bytes_per_row = bytes_per_row;
        self.width = width;
        self.height = height;
        self.surfaces.clear();
        self.compositor = None;
        self.image_cache.clear();
        self.font_cache.clear();
        Ok(())
    }

    fn ensure_compositor(&mut self) -> WavyteResult<()> {
        if self.compositor.is_some() {
            return Ok(());
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;

        let sampler = device.create_sampler(&vello::wgpu::SamplerDescriptor {
            label: Some("wavyte_composite_sampler"),
            address_mode_u: vello::wgpu::AddressMode::ClampToEdge,
            address_mode_v: vello::wgpu::AddressMode::ClampToEdge,
            address_mode_w: vello::wgpu::AddressMode::ClampToEdge,
            mag_filter: vello::wgpu::FilterMode::Nearest,
            min_filter: vello::wgpu::FilterMode::Nearest,
            mipmap_filter: vello::wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let params = device.create_buffer(&vello::wgpu::BufferDescriptor {
            label: Some("wavyte_composite_params"),
            size: 16,
            usage: vello::wgpu::BufferUsages::UNIFORM | vello::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&vello::wgpu::BindGroupLayoutDescriptor {
                label: Some("wavyte_composite_bgl"),
                entries: &[
                    vello::wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: vello::wgpu::ShaderStages::FRAGMENT,
                        ty: vello::wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: vello::wgpu::TextureViewDimension::D2,
                            sample_type: vello::wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    vello::wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: vello::wgpu::ShaderStages::FRAGMENT,
                        ty: vello::wgpu::BindingType::Sampler(
                            vello::wgpu::SamplerBindingType::Filtering,
                        ),
                        count: None,
                    },
                    vello::wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: vello::wgpu::ShaderStages::FRAGMENT,
                        ty: vello::wgpu::BindingType::Buffer {
                            ty: vello::wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: Some(std::num::NonZeroU64::new(16).unwrap()),
                        },
                        count: None,
                    },
                ],
            });

        let shader = device.create_shader_module(vello::wgpu::ShaderModuleDescriptor {
            label: Some("wavyte_composite_shader"),
            source: vello::wgpu::ShaderSource::Wgsl(
                r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
  var p = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 3.0, -1.0),
    vec2<f32>(-1.0,  3.0),
  );
  let pos = p[vi];
  var o: VsOut;
  o.pos = vec4<f32>(pos, 0.0, 1.0);
  o.uv = (pos + vec2<f32>(1.0, 1.0)) * 0.5;
  return o;
}

@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;
@group(0) @binding(2) var<uniform> params: vec4<f32>;

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let c = textureSample(t_src, s_src, in.uv);
  return c * params.x;
}
"#
                .into(),
            ),
        });

        let pipeline_layout =
            device.create_pipeline_layout(&vello::wgpu::PipelineLayoutDescriptor {
                label: Some("wavyte_composite_pl"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = device.create_render_pipeline(&vello::wgpu::RenderPipelineDescriptor {
            label: Some("wavyte_composite_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: vello::wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: vello::wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(vello::wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: vello::wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(vello::wgpu::ColorTargetState {
                    format: vello::wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(vello::wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: vello::wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: vello::wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: vello::wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        self.compositor = Some(Compositor {
            pipeline,
            bind_group_layout,
            sampler,
            params,
        });

        Ok(())
    }
}

impl PassBackend for VelloBackend {
    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()> {
        self.ensure_init(desc.width, desc.height)?;
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;

        let entry = self.surfaces.get(&id);
        let needs_create = entry
            .map(|s| s.width != desc.width || s.height != desc.height)
            .unwrap_or(true);

        if needs_create {
            let texture = device.create_texture(&vello::wgpu::TextureDescriptor {
                label: Some("wavyte_surface"),
                size: vello::wgpu::Extent3d {
                    width: desc.width,
                    height: desc.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: vello::wgpu::TextureDimension::D2,
                format: vello::wgpu::TextureFormat::Rgba8Unorm,
                usage: vello::wgpu::TextureUsages::STORAGE_BINDING
                    | vello::wgpu::TextureUsages::TEXTURE_BINDING
                    | vello::wgpu::TextureUsages::RENDER_ATTACHMENT
                    | vello::wgpu::TextureUsages::COPY_SRC
                    | vello::wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&vello::wgpu::TextureViewDescriptor::default());
            self.surfaces.insert(
                id,
                GpuSurface {
                    width: desc.width,
                    height: desc.height,
                    texture,
                    view,
                },
            );
        }
        Ok(())
    }

    fn exec_scene(
        &mut self,
        pass: &crate::compile::ScenePass,
        assets: &mut dyn AssetCache,
    ) -> WavyteResult<()> {
        self.scene.reset();
        for op in &pass.ops {
            encode_op(self, op, self.width, self.height, assets)?;
        }

        let device = self
            .device
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let target_view = self.surfaces.get(&pass.target).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "scene target surface {:?} was not initialized",
                pass.target
            ))
        })?;

        let base_color = if pass.clear_to_transparent {
            vello::peniko::Color::from_rgba8(0, 0, 0, 0)
        } else {
            match self.settings.clear_rgba {
                Some([r, g, b, a]) => vello::peniko::Color::from_rgba8(r, g, b, a),
                None => vello::peniko::Color::from_rgba8(0, 0, 0, 0),
            }
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
                &target_view.view,
                &vello::RenderParams {
                    base_color,
                    width: target_view.width,
                    height: target_view.height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .map_err(|e| WavyteError::evaluation(format!("vello render failed: {e:?}")))?;
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
        pass: &crate::compile::CompositePass,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<()> {
        self.ensure_compositor()?;

        let device = self
            .device
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let compositor = self
            .compositor
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu compositor not initialized"))?;

        let target = self.surfaces.get(&pass.target).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "composite target surface {:?} was not initialized",
                pass.target
            ))
        })?;

        let clear = match self.settings.clear_rgba {
            Some([r, g, b, a]) => vello::wgpu::Color {
                r: (r as f64) / 255.0,
                g: (g as f64) / 255.0,
                b: (b as f64) / 255.0,
                a: (a as f64) / 255.0,
            },
            None => vello::wgpu::Color::TRANSPARENT,
        };

        let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
            label: Some("wavyte_composite_encoder"),
        });

        {
            let mut rp = encoder.begin_render_pass(&vello::wgpu::RenderPassDescriptor {
                label: Some("wavyte_composite_rp"),
                color_attachments: &[Some(vello::wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: vello::wgpu::Operations {
                        load: vello::wgpu::LoadOp::Clear(clear),
                        store: vello::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rp.set_pipeline(&compositor.pipeline);

            for op in &pass.ops {
                match *op {
                    CompositeOp::Over { src, opacity } => {
                        let src = self.surfaces.get(&src).ok_or_else(|| {
                            WavyteError::evaluation(format!(
                                "composite src surface {:?} was not initialized",
                                src
                            ))
                        })?;

                        let opacity = opacity.clamp(0.0, 1.0);
                        let mut params = [0u8; 16];
                        params[0..4].copy_from_slice(&opacity.to_le_bytes());
                        queue.write_buffer(&compositor.params, 0, &params);

                        let bind_group =
                            device.create_bind_group(&vello::wgpu::BindGroupDescriptor {
                                label: Some("wavyte_composite_bg"),
                                layout: &compositor.bind_group_layout,
                                entries: &[
                                    vello::wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: vello::wgpu::BindingResource::TextureView(
                                            &src.view,
                                        ),
                                    },
                                    vello::wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: vello::wgpu::BindingResource::Sampler(
                                            &compositor.sampler,
                                        ),
                                    },
                                    vello::wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: compositor.params.as_entire_binding(),
                                    },
                                ],
                            });
                        rp.set_bind_group(0, &bind_group, &[]);
                        rp.draw(0..3, 0..1);
                    }
                    CompositeOp::Crossfade { .. } | CompositeOp::Wipe { .. } => {
                        return Err(WavyteError::evaluation(
                            "gpu composite crossfade/wipe is not implemented yet (phase 5)",
                        ));
                    }
                }
            }
        }

        queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn readback_rgba8(
        &mut self,
        surface: SurfaceId,
        plan: &crate::compile::RenderPlan,
        _assets: &mut dyn AssetCache,
    ) -> WavyteResult<FrameRGBA> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;
        let readback = self
            .readback
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("gpu backend not initialized"))?;

        let surface = self.surfaces.get(&surface).ok_or_else(|| {
            WavyteError::evaluation(format!(
                "readback surface {:?} was not initialized",
                surface
            ))
        })?;

        let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
            label: Some("wavyte_readback_encoder"),
        });
        encoder.copy_texture_to_buffer(
            vello::wgpu::TexelCopyTextureInfo {
                texture: &surface.texture,
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
