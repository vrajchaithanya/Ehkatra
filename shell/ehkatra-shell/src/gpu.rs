//! The wgpu compositor: one instanced-quad pipeline (ADR-021, docs/31).
//!
//! docs/31 specifies *"instanced fills/borders/gridlines, damage-rect repaint
//! only"*. Everything a grid draws before text — cell backgrounds, gridlines,
//! header shading, selection — is an axis-aligned rectangle, so it is one
//! pipeline and one draw call with a per-instance rect and colour. There is no
//! per-cell vertex buffer to rebuild, which is what keeps a scroll frame inside
//! docs/31's 8.3 ms.
//!
//! Rendering has **two targets and one path**: a texture that can be read
//! back, hashed and committed as evidence (`demo/`) on a machine with no
//! display, and a window surface that is presented to the compositor
//! ([`Present`]). [`Renderer::encode_scene`] is the single place a scene
//! becomes GPU commands, so the two cannot drift — the presented frame is the
//! frame the PNG shows, by construction rather than by discipline.

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

/// The offscreen colour format. sRGB, so the hardware does the linear→sRGB
/// encode on write and the theme can be authored in the space a designer picks
/// colours in.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A window surface configured for presentation, and the DPI scale the frames
/// drawn into it are laid out at.
///
/// Held apart from [`Renderer`] because the renderer outlives any particular
/// window and a headless one never has a surface at all.
pub struct Present {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Display scale factor. The scene is authored in logical pixels; this is
    /// the only place the conversion to device pixels happens.
    pub scale: f32,
}

impl Present {
    /// Logical size of the drawable area — the coordinate space a scene is
    /// built in.
    pub fn logical_size(&self) -> (f32, f32) {
        (
            self.config.width as f32 / self.scale,
            self.config.height as f32 / self.scale,
        )
    }

    pub fn physical_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }
}

/// What one presented frame cost, split so the number means something.
///
/// Under `Fifo`, `get_current_texture` **blocks until the display is ready for
/// another image**. Timing the whole call therefore measures the refresh
/// interval and not the renderer: on a 120 Hz panel every frame comes out at
/// about 8.3 ms whether the scene took 0.2 ms or 5 ms to build. So the wait is
/// reported apart from the work, and docs/31's 8.3 ms scroll-frame budget is
/// judged against the work — the part the application controls, and the only
/// part that can actually drop a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    /// Time blocked in `get_current_texture`. Vsync, not cost.
    pub acquire_ms: f64,
    /// Encoding the pass and submitting it.
    pub submit_ms: f64,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<(wgpu::TextureView, u32)>,
    format: wgpu::TextureFormat,
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
        let (device, queue) = Self::open_device(&adapter)?;
        Some(Self::with_device(device, queue, OFFSCREEN_FORMAT))
    }

    fn open_device(adapter: &wgpu::Adapter) -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("ehkatra"),
                required_features: wgpu::Features::empty(),
                // `downlevel_defaults` and not `default()`: the grid needs
                // nothing a 2015 integrated GPU lacks, and asking for more
                // would refuse to open on hardware that can run the product
                // perfectly well.
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()
    }

    /// Creates a renderer bound to a window surface, and the surface itself.
    ///
    /// The adapter is requested **with the surface as `compatible_surface`**,
    /// which is not a formality: on a multi-GPU laptop the adapter that can
    /// present to this window is not always the one a headless request would
    /// pick, and the failure mode is a device that creates textures happily and
    /// cannot show them.
    ///
    /// `scale` is the display's DPI factor; frames are authored in logical
    /// pixels and stretched to the physical surface by the projection, so a
    /// caller never converts coordinates itself.
    pub fn for_surface<T>(
        target: T,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<(Renderer, Present), String>
    where
        T: Into<wgpu::SurfaceTarget<'static>>,
    {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(target)
            .map_err(|e| format!("creating the window surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // A grid is not a game: the integrated GPU draws it inside budget
            // and the discrete one costs battery, which docs/31 budgets
            // explicitly ("<8% per hour of active editing").
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or("no GPU adapter can present to this window")?;
        let (device, queue) =
            Self::open_device(&adapter).ok_or("the GPU adapter refused to open a device")?;

        let caps = surface.get_capabilities(&adapter);
        // An sRGB target if one exists, so the same theme constants produce the
        // same colours as the offscreen path. If none does, the first format is
        // taken and the frame is a shade off rather than absent — reported
        // through `Present::format_is_srgb` rather than silently.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or_else(|| caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            // Fifo is vsync and is the only mode guaranteed present everywhere.
            // docs/31's budget is a *frame* budget, not a frame *rate* target:
            // a grid that redraws only when something changed has no reason to
            // spin the GPU faster than the display.
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let renderer = Self::with_device(device, queue, format);
        Ok((
            renderer,
            Present {
                surface,
                config,
                scale,
            },
        ))
    }

    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Renderer {
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
                    format,
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
            format,
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Whether the presented format encodes sRGB in hardware. `false` means the
    /// theme's colours are being written to a linear target and will look
    /// washed out — a fact worth surfacing rather than shipping quietly.
    pub fn format_is_srgb(&self) -> bool {
        self.format.is_srgb()
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

    /// Resizes the surface. A zero dimension is ignored: Windows reports one
    /// when the window is minimised, and configuring a zero-sized surface is a
    /// validation error rather than a no-op.
    pub fn reconfigure(&self, present: &mut Present, width: u32, height: u32, scale: f32) {
        if width == 0 || height == 0 {
            return;
        }
        present.config.width = width;
        present.config.height = height;
        present.scale = scale;
        present.surface.configure(&self.device, &present.config);
    }

    /// Draws a scene and hands the frame to the compositor.
    ///
    /// `Lost`/`Outdated` are recovered by reconfiguring, because they are what
    /// a resize or a monitor change looks like from here and neither is an
    /// error the caller can do anything else about.
    pub fn present(
        &self,
        present: &mut Present,
        quads: &[Quad],
    ) -> Result<FrameTiming, wgpu::SurfaceError> {
        let acquire = std::time::Instant::now();
        let frame = match present.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                present.surface.configure(&self.device, &present.config);
                present.surface.get_current_texture()?
            }
            Err(other) => return Err(other),
        };
        let acquire_ms = acquire.elapsed().as_secs_f64() * 1000.0;

        let work = std::time::Instant::now();
        let view = frame.texture.create_view(&Default::default());
        // Logical, not physical: the projection maps the logical box onto the
        // whole surface, so every coordinate in the scene stays DPI-independent
        // and there is exactly one place the scale factor is applied.
        let (lw, lh) = present.logical_size();
        let encoder = self.encode_scene(&view, lw, lh, quads);
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(FrameTiming {
            acquire_ms,
            submit_ms: work.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Builds the command buffer for one frame. The **only** place a scene
    /// becomes GPU commands; both the presented and the read-back paths go
    /// through it, so a frame on screen and a frame in a PNG cannot differ.
    fn encode_scene(
        &self,
        view: &wgpu::TextureView,
        width: f32,
        height: f32,
        quads: &[Quad],
    ) -> wgpu::CommandEncoder {
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
        let dims = [width, height, atlas_size, atlas_size];
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

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
        encoder
    }

    /// Renders a scene to an RGBA8 buffer of `width * height * 4` bytes.
    ///
    /// Device pixels: this path has no display and so no scale factor, and the
    /// caller that wants a 2× frame asks for a 2× buffer.
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
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());

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

        let mut encoder = self.encode_scene(&view, width as f32, height as f32, quads);
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
