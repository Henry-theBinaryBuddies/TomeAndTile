//! The wgpu backend: turns a `DrawList` into one render pass. One pipeline,
//! one growing vertex buffer, one bind group (texture + sampler) per image.
//! WebGPU is used when the browser has it, WebGL2 otherwise. wasm32 only.

use std::collections::HashMap;
use std::sync::Arc;

use web_sys::{HtmlCanvasElement, ImageBitmap};

use crate::draw::{DrawItem, DrawList};

/// f32s per vertex: pos(2) uv(2) colour(4) mode(1) param(1).
const FLOATS_PER_VERTEX: usize = 10;
const MODE_FLAT: f32 = 0.0;
const MODE_IMAGE: f32 = 1.0;
const MODE_DISC: f32 = 2.0;
const MODE_RING: f32 = 3.0;
const MODE_IMAGE_DISC: f32 = 4.0;
const INITIAL_VERTEX_BYTES: u64 = 64 * 1024;

/// A run of vertices drawn with one texture (`None` = the white texture).
type Batch = (Option<String>, std::ops::Range<u32>);

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::BindGroup,
    images: HashMap<String, wgpu::BindGroup>,
    vertex_buf: wgpu::Buffer,
    vertex_cap: u64,
    max_texture: u32,
    canvas: HtmlCanvasElement,
    css_w: f64,
    css_h: f64,
}

fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> String + '_ {
    move |e| format!("{what}: {e}")
}

impl Gpu {
    pub async fn new(canvas: HtmlCanvasElement) -> Result<Gpu, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL;
        let instance = wgpu::util::new_instance_with_webgpu_detection(desc).await;
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(err("create surface"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .map_err(err("no usable GPU adapter (neither WebGPU nor WebGL2)"))?;
        // Ask for everything the adapter has: large maps need big textures.
        let limits = adapter.limits();
        let max_texture = limits.max_texture_dimension_2d;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("tt-renderer"),
                required_limits: limits,
                ..Default::default()
            })
            .await
            .map_err(err("request device"))?;
        device.on_uncaptured_error(Arc::new(|e| {
            web_sys::console::error_1(&format!("tt-renderer GPU error: {e}").into());
        }));

        // A non-sRGB surface format keeps colours as authored (no gamma step).
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("surface reports no formats")?;
        let w = canvas.width().max(1);
        let h = canvas.height().max(1);
        let mut config = surface
            .get_default_config(&adapter, w, h)
            .ok_or("surface not supported by adapter")?;
        config.format = format;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tt-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tt-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tt-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x2
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tt-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (FLOATS_PER_VERTEX * 4) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tt-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // A 1x1 white texture stands in for "no image" so one layout fits all.
        let white_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tt-white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white_tex.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white = bind_group(&device, &bind_layout, &sampler, &white_tex);
        let vertex_buf = new_vertex_buffer(&device, INITIAL_VERTEX_BYTES);

        Ok(Gpu {
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_layout,
            sampler,
            white,
            images: HashMap::new(),
            vertex_buf,
            vertex_cap: INITIAL_VERTEX_BYTES,
            max_texture,
            canvas,
            css_w: 1.0,
            css_h: 1.0,
        })
    }

    /// Resize the canvas backing store to `css * dpr` device pixels.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        let dpr = if dpr.is_finite() && dpr > 0.0 {
            dpr
        } else {
            1.0
        };
        // A NaN or infinite size from the host must not reach the surface.
        let sane = |v: f64| if v.is_finite() { v.max(1.0) } else { 1.0 };
        let (css_w, css_h) = (sane(css_w), sane(css_h));
        let max = f64::from(self.max_texture);
        let w = (css_w * dpr).round().clamp(1.0, max) as u32;
        let h = (css_h * dpr).round().clamp(1.0, max) as u32;
        self.css_w = css_w.max(1.0);
        self.css_h = css_h.max(1.0);
        self.canvas.set_width(w);
        self.canvas.set_height(h);
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    /// Copy a decoded bitmap into a new texture. Fails (so the caller marks
    /// the image failed) if it is larger than the GPU allows.
    pub fn upload_image(&mut self, url: &str, bitmap: &ImageBitmap) -> Result<(), String> {
        let (w, h) = (bitmap.width(), bitmap.height());
        if w == 0 || h == 0 || w > self.max_texture || h > self.max_texture {
            return Err(format!(
                "image {w}x{h} does not fit the GPU limit of {}",
                self.max_texture
            ));
        }
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tt-image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.queue.copy_external_image_to_texture(
            &wgpu::CopyExternalImageSourceInfo {
                source: wgpu::ExternalImageSource::ImageBitmap(bitmap.clone()),
                origin: wgpu::Origin2d::ZERO,
                flip_y: false,
            },
            wgpu::CopyExternalImageDestInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
                color_space: wgpu::PredefinedColorSpace::Srgb,
                premultiplied_alpha: false,
            },
            size,
        );
        let group = bind_group(&self.device, &self.bind_layout, &self.sampler, &tex);
        self.images.insert(url.to_owned(), group);
        Ok(())
    }

    /// Draw one frame.
    pub fn draw(&mut self, list: &DrawList) -> Result<(), String> {
        let (verts, batches) = self.build_vertices(list);
        let bytes: &[u8] = bytemuck::cast_slice(&verts);
        if bytes.len() as u64 > self.vertex_cap {
            let mut cap = self.vertex_cap;
            while cap < bytes.len() as u64 {
                cap *= 2;
            }
            self.vertex_buf = new_vertex_buffer(&self.device, cap);
            self.vertex_cap = cap;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.vertex_buf, 0, bytes);
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                // Reconfigure and skip this frame; the next one draws.
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface texture validation error".into());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tt-frame"),
            });
        {
            let bg = list.background;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tt-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(bg[0]),
                            g: f64::from(bg[1]),
                            b: f64::from(bg[2]),
                            a: f64::from(bg[3]),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            for (key, range) in batches {
                let group = key
                    .as_deref()
                    .and_then(|k| self.images.get(k))
                    .unwrap_or(&self.white);
                pass.set_bind_group(0, group, &[]);
                pass.draw(range, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }

    /// Flatten the draw list into vertices, merging runs that use the same
    /// texture into one draw call.
    fn build_vertices(&self, list: &DrawList) -> (Vec<f32>, Vec<Batch>) {
        let mut v: Vec<f32> = Vec::with_capacity(list.items.len() * 6 * FLOATS_PER_VERTEX);
        let mut batches: Vec<Batch> = Vec::new();
        let (w, h) = (self.css_w as f32, self.css_h as f32);
        for item in &list.items {
            let before = (v.len() / FLOATS_PER_VERTEX) as u32;
            let key: Option<String> = match item {
                DrawItem::Image {
                    url,
                    x0,
                    y0,
                    x1,
                    y1,
                } => {
                    if !self.images.contains_key(url) {
                        continue;
                    }
                    quad(
                        &mut v,
                        w,
                        h,
                        (*x0, *y0, *x1, *y1),
                        (0.0, 0.0, 1.0, 1.0),
                        [1.0; 4],
                        MODE_IMAGE,
                        0.0,
                    );
                    Some(url.clone())
                }
                DrawItem::Rect {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                } => {
                    quad(
                        &mut v,
                        w,
                        h,
                        (*x0, *y0, *x1, *y1),
                        (0.0, 0.0, 0.0, 0.0),
                        *color,
                        MODE_FLAT,
                        0.0,
                    );
                    None
                }
                DrawItem::Disc {
                    cx,
                    cy,
                    r,
                    color,
                    image,
                } => {
                    let textured = image.as_ref().filter(|u| self.images.contains_key(*u));
                    disc_quad(
                        &mut v,
                        w,
                        h,
                        *cx,
                        *cy,
                        *r,
                        *color,
                        if textured.is_some() {
                            MODE_IMAGE_DISC
                        } else {
                            MODE_DISC
                        },
                        0.0,
                    );
                    textured.cloned()
                }
                DrawItem::Ring {
                    cx,
                    cy,
                    r,
                    thickness,
                    color,
                } => {
                    let inner = ((*r - *thickness) / r.max(0.001)).clamp(0.0, 1.0);
                    disc_quad(&mut v, w, h, *cx, *cy, *r, *color, MODE_RING, inner);
                    None
                }
            };
            let after = (v.len() / FLOATS_PER_VERTEX) as u32;
            match batches.last_mut() {
                Some((k, range)) if *k == key => range.end = after,
                _ => batches.push((key, before..after)),
            }
        }
        (v, batches)
    }
}

fn new_vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tt-vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    tex: &wgpu::Texture,
) -> wgpu::BindGroup {
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tt-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Push one vertex, converting CSS-pixel `(x, y)` to clip space.
#[allow(clippy::too_many_arguments)]
fn vertex(
    v: &mut Vec<f32>,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    u: f32,
    t: f32,
    color: [f32; 4],
    mode: f32,
    param: f32,
) {
    v.extend_from_slice(&[
        x / w * 2.0 - 1.0,
        1.0 - y / h * 2.0,
        u,
        t,
        color[0],
        color[1],
        color[2],
        color[3],
        mode,
        param,
    ]);
}

/// Two triangles covering `rect` with texture corners `uv`.
#[allow(clippy::too_many_arguments)]
fn quad(
    v: &mut Vec<f32>,
    w: f32,
    h: f32,
    rect: (f32, f32, f32, f32),
    uv: (f32, f32, f32, f32),
    color: [f32; 4],
    mode: f32,
    param: f32,
) {
    let (x0, y0, x1, y1) = rect;
    let (u0, t0, u1, t1) = uv;
    for (x, y, u, t) in [
        (x0, y0, u0, t0),
        (x1, y0, u1, t0),
        (x0, y1, u0, t1),
        (x1, y0, u1, t0),
        (x1, y1, u1, t1),
        (x0, y1, u0, t1),
    ] {
        vertex(v, w, h, x, y, u, t, color, mode, param);
    }
}

/// A quad around a disc, one pixel larger than its radius for antialiasing,
/// with uv = -1..1 scaled so the disc edge sits at length 1.
#[allow(clippy::too_many_arguments)]
fn disc_quad(
    v: &mut Vec<f32>,
    w: f32,
    h: f32,
    cx: f32,
    cy: f32,
    r: f32,
    color: [f32; 4],
    mode: f32,
    param: f32,
) {
    let r = r.max(0.001);
    let half = r + 1.0;
    let k = half / r;
    quad(
        v,
        w,
        h,
        (cx - half, cy - half, cx + half, cy + half),
        (-k, -k, k, k),
        color,
        mode,
        param,
    );
}
