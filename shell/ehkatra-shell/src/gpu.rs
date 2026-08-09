//! The wgpu compositor: one instanced-quad pipeline (ADR-021, docs/31).
//!
//! docs/31 specifies *"instanced fills/borders/gridlines, damage-rect repaint
//! only"*. Everything a grid draws before text — cell backgrounds, gridlines,
//! header shading, selection — is an axis-aligned rectangle, so it is one
//! pipeline and one draw call with a per-instance rect and colour. There is no
//! per-cell vertex buffer to rebuild, which is what keeps a scroll frame inside
//! docs/31's 8.3 ms.
//!
//! Rendering is **offscreen by default**, and that is deliberate rather than a
//! limitation: a render that targets a texture can be read back, hashed and
//! committed as evidence (`demo/`), and it runs on a machine with no display.
//! The windowed path presents the same scene to a surface.

use std::borrow::Cow;

/// One rectangle. `rect` is `(x, y, w, h)` in logical pixels with the origin at
/// the viewport's top-left; `color` is linear RGBA; `uv` is the atlas rect in
/// texels.
///
/// A cell fill and a glyph are **the same instance**, differing only in which
/// part of the atlas they sample — a fill points at the reserved white texel.
/// That is why adding text added no pipeline, no pass and no draw call.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub uv: [f32; 4],
}

/// Casts a slice of quads to bytes. Sound because `Quad` is `#[repr(C)]` with
/// only `f32` fields: no padding, no niches, every bit pattern valid.
pub fn quads_as_bytes(quads: &[Quad]) -> &[u8] {
    // SAFETY: `Quad` is `#[repr(C)]` and composed solely of `f32` arrays, so it
    // has no padding bytes and no invalid bit patterns. The lifetime is tied to
    // the input slice.
    unsafe {
        core::slice::from_raw_parts(quads.as_ptr() as *const u8, core::mem::size_of_val(quads))
    }
}

const SHADER: &str = r#"
struct Viewport { size: vec2<f32>, atlas: vec2<f32> };
@group(0) @binding(0) var<uniform> viewport: Viewport;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    // A unit quad from two triangles, expanded per instance. No vertex buffer:
    // the corner is derived from the index, so the only per-cell data on the
    // bus is the instance itself.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vi];
    let px = inst.rect.xy + corner * inst.rect.zw;
    // Pixels (origin top-left, y down) to clip space (origin centre, y up).
    let ndc = vec2<f32>(
        px.x / viewport.size.x * 2.0 - 1.0,
        1.0 - px.y / viewport.size.y * 2.0,
    );
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = inst.color;
    // Atlas texels to normalised coordinates. Sampling the *centre* of the
    // reserved white texel matters: at its corner a linear filter would blend
    // with the empty texel beside it and every solid fill would come out
    // half-transparent.
    out.uv = (inst.uv.xy + corner * inst.uv.zw) / viewport.atlas;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // The atlas is coverage, not colour: it modulates alpha so a glyph takes
    // the run's colour and antialiases against whatever is behind it.
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
"#;

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<(wgpu::TextureView, u32)>,
}

impl Renderer {
    /// Creates a headless renderer.
    ///
    /// `None` when no adapter exists at all — a CI container with no GPU and no
    /// software fallback. Reported rather than panicked on: a machine without a
    /// GPU is a fact about the machine, not a bug in the shell (DP-A10).
    pub fn headless() -> Option<Renderer> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("ehkatra"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;
        Some(Self::with_device(device, queue))
    }

    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: core::mem::size_of::<Quad>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            // Linear, so a glyph edge is smooth; the atlas leaves a texel of
            // padding between entries so filtering cannot bleed across glyphs.
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Renderer {
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            atlas: None,
        }
    }

    /// Uploads the glyph atlas. Called once; the atlas grows by rasterising
    /// into the same texture, so re-uploading is a whole-texture write rather
    /// than a per-frame cost.
    pub fn upload_atlas(&mut self, size: u32, coverage: &[u8]) {
        let extent = wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            coverage,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(size),
                rows_per_image: Some(size),
            },
            extent,
        );
        self.atlas = Some((texture.create_view(&Default::default()), size));
    }

    /// Renders a scene to an RGBA8 buffer of `width * height * 4` bytes.
    pub fn render_to_rgba(&self, width: u32, height: u32, quads: &[Quad]) -> Vec<u8> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());

        // A 1x1 opaque stand-in, so a scene rendered before any atlas exists
        // draws solid quads correctly instead of failing to bind.
        let blank = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &blank,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(1),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let blank_view = blank.create_view(&Default::default());

        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas_size = self.atlas.as_ref().map_or(1.0, |(_, n)| *n as f32);
        let dims = [width as f32, height as f32, atlas_size, atlas_size];
        self.queue
            .write_buffer(&uniform, 0, quads_as_bytes_f32(&dims));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        self.atlas.as_ref().map(|(v, _)| v).unwrap_or(&blank_view),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let instances = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: (core::mem::size_of::<Quad>() * quads.len().max(1)) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !quads.is_empty() {
            self.queue
                .write_buffer(&instances, 0, quads_as_bytes(quads));
        }

        // Readback rows must be aligned to 256 bytes, so the buffer is padded
        // and the padding stripped after mapping.
        let unpadded = width as usize * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * height as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            // One draw call for the whole grid.
            pass.draw(0..6, 0..quads.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let mapped = slice.get_mapped_range();
        let mut out = Vec::with_capacity(unpadded * height as usize);
        for row in 0..height as usize {
            let start = row * padded;
            out.extend_from_slice(&mapped[start..start + unpadded]);
        }
        drop(mapped);
        readback.unmap();
        out
    }
}

fn quads_as_bytes_f32(v: &[f32]) -> &[u8] {
    // SAFETY: `f32` has no padding and no invalid bit patterns.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}
