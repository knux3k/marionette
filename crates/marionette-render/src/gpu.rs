//! GPU setup and offscreen rendering with readback.

use crate::extract::{CameraData, FrameData, View};
use crate::renderer::{self, DEPTH_FORMAT, RenderStats, Renderer, Target};
use marionette_core::{Diagnostic, Errors};
use std::path::PathBuf;

/// The offscreen color format. Screenshots are 8-bit sRGB.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Counts of live GPU objects (see [`Gpu::live_objects`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuObjects {
    pub buffers: usize,
    pub textures: usize,
    pub texture_views: usize,
    pub bind_groups: usize,
    pub samplers: usize,
    pub render_pipelines: usize,
}

fn device_error(msg: String) -> Errors {
    Diagnostic::error(renderer::codes::DEVICE, msg).into()
}

impl Gpu {
    /// Picks a high-performance adapter. `surface` makes it compatible with a window.
    /// The backend can be forced with `WGPU_BACKEND` and the adapter with `WGPU_ADAPTER_NAME`
    /// (substring, e.g. "Basic Render" for the WARP software rasterizer on Windows).
    pub fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Self, Errors> {
        let adapter = pollster::block_on(async {
            match wgpu::util::initialize_adapter_from_env(&instance, surface).await {
                Ok(adapter) => Ok(adapter),
                Err(_) => {
                    instance
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::HighPerformance,
                            compatible_surface: surface,
                            ..Default::default()
                        })
                        .await
                }
            }
        })
        .map_err(|e| {
            Errors::from(
                Diagnostic::error(renderer::codes::NO_ADAPTER, format!("no usable GPU adapter: {e}")).fix(
                    "install a Vulkan, DirectX 12 or Metal driver; on headless Linux install Mesa's lavapipe",
                ),
            )
        })?;
        // Ray queries (decision 0093) where the adapter has them, unless turned off with
        // MARIONETTE_RAY_TRACING=0. Scenes use them only with the `RayTracing` resource.
        let ray_query = wgpu::Features::EXPERIMENTAL_RAY_QUERY;
        let rt = adapter.features().contains(ray_query)
            && std::env::var("MARIONETTE_RAY_TRACING").map_or(true, |v| v != "0");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("marionette"),
            // BC7 model textures under `TextureCompression` (decision 0283) where supported.
            // Both eyes in one pass (decision 0285) where supported; MSAA two-layer targets
            // need multisampled arrays.
            required_features: (adapter.features()
                & (wgpu::Features::TEXTURE_COMPRESSION_BC
                    | wgpu::Features::MULTIVIEW
                    | wgpu::Features::MULTISAMPLE_ARRAY))
                | if rt { ray_query } else { wgpu::Features::empty() },
            required_limits: wgpu::Limits {
                max_multiview_view_count: adapter.limits().max_multiview_view_count.min(2),
                ..if rt {
                    wgpu::Limits::default().using_acceleration_structure_values(adapter.limits())
                } else {
                    wgpu::Limits::default()
                }
            },
            // SAFETY: wgpu marks ray queries experimental: its API may change and may have
            // bugs. Only ray query use is enabled; it runs only when a scene asks for
            // `RayTracing`, and MARIONETTE_RAY_TRACING=0 turns it off entirely.
            experimental_features: if rt {
                unsafe { wgpu::ExperimentalFeatures::enabled() }
            } else {
                wgpu::ExperimentalFeatures::disabled()
            },
            ..Default::default()
        }))
        .map_err(|e| device_error(format!("cannot open GPU device: {e}")))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn headless() -> Result<Self, Errors> {
        Self::new(
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env()),
            None,
        )
    }

    /// Live GPU objects the application holds, counted by wgpu. Leak tests compare these
    /// before and after repeated work (decision 0032). `None` on backends without counts.
    pub fn live_objects(&self) -> Option<GpuObjects> {
        let hub = self.instance.generate_report()?.hub;
        Some(GpuObjects {
            buffers: hub.buffers.num_kept_from_user,
            textures: hub.textures.num_kept_from_user,
            texture_views: hub.texture_views.num_kept_from_user,
            bind_groups: hub.bind_groups.num_kept_from_user,
            samplers: hub.samplers.num_kept_from_user,
            render_pipelines: hub.render_pipelines.num_kept_from_user,
        })
    }

    /// True when the device can trace rays (decision 0093).
    pub fn ray_tracing(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
    }

    /// GPU name and backend, e.g. `<adapter name> (Vulkan)`.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }
}

/// Picks the camera with stable id `camera`, or the first camera (lowest id).
pub fn pick_camera(frame: &FrameData, camera: Option<u64>) -> Result<CameraData, Errors> {
    let found = match camera {
        Some(id) => frame.cameras.iter().find(|c| c.entity == id),
        None => frame.cameras.first(),
    };
    found.copied().ok_or_else(|| {
        let ids: Vec<String> = frame.cameras.iter().map(|c| c.entity.to_string()).collect();
        let msg = match camera {
            Some(id) => format!("entity {id} has no Camera"),
            None => "the scene has no Camera".into(),
        };
        let fix = if ids.is_empty() {
            "add an entity with Camera and Transform components".to_string()
        } else {
            format!("use one of the camera entities: {}", ids.join(", "))
        };
        let mut d = Diagnostic::error(renderer::codes::NO_CAMERA, msg).fix(fix);
        if let Some(id) = camera {
            d = d.entity(id);
        }
        Errors::from(d)
    })
}

/// Where the time of one offscreen render went (wall clock, milliseconds).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTiming {
    /// CPU: prepare, build per-frame data, record and submit commands.
    pub record_ms: f64,
    /// Waiting for the GPU to finish after submit.
    pub gpu_wait_ms: f64,
}

/// Renders frames into textures with no window and reads them back.
pub struct Offscreen {
    pub renderer: Renderer,
    /// Wall-clock split of the last render, for benchmarks.
    pub last_timing: FrameTiming,
    width: u32,
    height: u32,
    msaa: Option<wgpu::TextureView>,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_row: u32,
    sample_count: u32,
    /// Two-layer targets for single-pass stereo (decision 0285), made when turned on.
    stereo: Option<StereoTargets>,
}

/// Color, resolve and depth with two layers: each as one array view and per layer.
struct StereoTargets {
    color: wgpu::Texture,
    msaa: Option<[wgpu::TextureView; 3]>,
    resolve: [wgpu::TextureView; 3],
    depth: [wgpu::TextureView; 3],
}

impl StereoTargets {
    fn new(device: &wgpu::Device, (width, height): (u32, u32), samples: u32) -> Self {
        let texture = |label, format, samples| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        // [both layers, layer 0, layer 1]
        let views = |t: &wgpu::Texture| {
            let view = |base: u32, count: u32| {
                t.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    base_array_layer: base,
                    array_layer_count: Some(count),
                    ..Default::default()
                })
            };
            let layer = |base: u32| {
                t.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: base,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            };
            [view(0, 2), layer(0), layer(1)]
        };
        let color = texture("color (stereo)", OFFSCREEN_FORMAT, 1);
        let msaa = (samples > 1).then(|| views(&texture("color msaa (stereo)", OFFSCREEN_FORMAT, samples)));
        let resolve = views(&color);
        let depth = views(&texture("depth (stereo)", DEPTH_FORMAT, samples));
        Self {
            color,
            msaa,
            resolve,
            depth,
        }
    }

    /// Target `k`: 0 both layers, 1 and 2 one eye each.
    fn target(&self, k: usize, (width, height): (u32, u32)) -> Target<'_> {
        Target {
            color: self.msaa.as_ref().map_or(&self.resolve[k], |m| &m[k]),
            resolve: self.msaa.as_ref().map(|_| &self.resolve[k]),
            depth: &self.depth[k],
            width,
            height,
        }
    }
}

impl Offscreen {
    pub fn new(gpu: &Gpu, width: u32, height: u32, sample_count: u32, base_dir: PathBuf) -> Self {
        let renderer = Renderer::new(&gpu.device, &gpu.queue, OFFSCREEN_FORMAT, sample_count, base_dir);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = |label, format, samples, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let resolve = texture(
            "color",
            OFFSCREEN_FORMAT,
            1,
            // COPY_DST: single-pass stereo copies the right eye in.
            attach | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        );
        let msaa = (sample_count > 1).then(|| {
            texture("color msaa", OFFSCREEN_FORMAT, sample_count, attach).create_view(&Default::default())
        });
        let depth = texture("depth", DEPTH_FORMAT, sample_count, attach).create_view(&Default::default());
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = (width * 4).div_ceil(align) * align;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let resolve_view = resolve.create_view(&Default::default());
        Self {
            renderer,
            last_timing: FrameTiming::default(),
            width,
            height,
            msaa,
            resolve,
            resolve_view,
            depth,
            readback,
            padded_row,
            sample_count,
            stereo: None,
        }
    }

    /// Draws two-eye renders ([`Offscreen::render_eyes`] with 2 eyes) into two-layer targets
    /// with [`Renderer::render_stereo`] (decision 0285): one pass for both eyes where the
    /// device and frame allow it. Returns whether the device has multiview.
    pub fn set_single_pass_stereo(&mut self, gpu: &Gpu, on: bool) -> bool {
        let can = self.renderer.single_pass_stereo();
        self.stereo = (on && can)
            .then(|| StereoTargets::new(&gpu.device, (self.width, self.height), self.sample_count));
        can
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// See [`Renderer::set_culling`].
    pub fn set_culling(&mut self, on: bool) {
        self.renderer.set_culling(on);
    }

    /// See [`Renderer::set_streaming`].
    pub fn set_streaming(&mut self, on: bool) {
        self.renderer.set_streaming(on);
    }

    /// See [`Renderer::models_loading`].
    pub fn models_loading(&self) -> usize {
        self.renderer.models_loading()
    }

    /// See [`Renderer::finish_loading`].
    pub fn finish_loading(&mut self) {
        self.renderer.finish_loading();
    }

    /// See [`Renderer::trim`].
    pub fn trim(&mut self, frame: &FrameData) -> usize {
        self.renderer.trim(frame)
    }

    /// See [`Renderer::cached_assets`].
    pub fn cached_assets(&self) -> usize {
        self.renderer.cached_assets()
    }

    /// Renders the frame from the camera with stable id `camera` (or the first camera) and
    /// waits for the GPU to finish. Asset problems and a missing camera come back as data.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        frame: &FrameData,
        camera: Option<u64>,
    ) -> Result<RenderStats, Errors> {
        self.render_eyes(gpu, frame, camera, 1, 0.0)
    }

    /// Like [`Offscreen::render`], with `eyes` views of the camera spread `ipd` meters apart
    /// along its right axis (two for a stereo headset), all recorded in one call. Each eye
    /// is a full render pass into the same target (the last eye is what stays in the image),
    /// so the GPU and CPU cost matches a stereo frame at this per-eye size.
    pub fn render_eyes(
        &mut self,
        gpu: &Gpu,
        frame: &FrameData,
        camera: Option<u64>,
        eyes: u32,
        ipd: f32,
    ) -> Result<RenderStats, Errors> {
        let cam = pick_camera(frame, camera)?;
        let base = cam.view(self.width as f32 / self.height as f32);
        self.render_from(gpu, frame, base, eyes, ipd)
    }

    /// Renders the frame from an explicit view (an editor camera, not a scene camera) and
    /// waits for the GPU to finish.
    pub fn render_view(&mut self, gpu: &Gpu, frame: &FrameData, view: View) -> Result<RenderStats, Errors> {
        self.render_from(gpu, frame, view, 1, 0.0)
    }

    fn render_from(
        &mut self,
        gpu: &Gpu,
        frame: &FrameData,
        base: View,
        eyes: u32,
        ipd: f32,
    ) -> Result<RenderStats, Errors> {
        let started = std::time::Instant::now();
        let errors = self.renderer.prepare(frame);
        if !errors.is_empty() {
            return Err(Errors(errors));
        }
        let eyes = eyes.clamp(1, crate::MAX_VIEWS as u32);
        let target = Target {
            color: self.msaa.as_ref().unwrap_or(&self.resolve_view),
            resolve: self.msaa.as_ref().map(|_| &self.resolve_view),
            depth: &self.depth,
            width: self.width,
            height: self.height,
        };
        let views: Vec<(View, &Target)> = (0..eyes)
            .map(|i| {
                // Eye offset in view space: -ipd/2 .. +ipd/2 along x.
                let k = if eyes == 1 {
                    0.0
                } else {
                    i as f32 / (eyes - 1) as f32 - 0.5
                };
                let shift = glam::Mat4::from_translation(glam::Vec3::new(-k * ipd, 0.0, 0.0));
                let view = shift * base.view;
                (
                    View {
                        view,
                        eye: view.inverse().w_axis.truncate(),
                        ..base
                    },
                    &target,
                )
            })
            .collect();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let stats = match (&self.stereo, views.as_slice()) {
            (Some(st), [(a, _), (b, _)]) => {
                let size = (self.width, self.height);
                let stats = self.renderer.render_stereo(
                    frame,
                    [*a, *b],
                    &st.target(0, size),
                    [&st.target(1, size), &st.target(2, size)],
                    &mut encoder,
                );
                // The last eye stays in the image, as with separate passes.
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &st.color,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: 1 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    self.resolve.as_image_copy(),
                    wgpu::Extent3d {
                        width: self.width,
                        height: self.height,
                        depth_or_array_layers: 1,
                    },
                );
                stats
            }
            _ => self.renderer.render_views(frame, &views, &mut encoder),
        };
        gpu.queue.submit([encoder.finish()]);
        let submitted = std::time::Instant::now();
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| device_error(format!("GPU wait failed: {e}")))?;
        // Measurement only (benchmarks); never read by the simulation.
        self.last_timing = FrameTiming {
            record_ms: (submitted - started).as_secs_f64() * 1000.0,
            gpu_wait_ms: submitted.elapsed().as_secs_f64() * 1000.0,
        };
        Ok(stats)
    }

    /// Copies the last rendered image to the CPU as tightly packed RGBA8 (sRGB).
    pub fn read_pixels(&self, gpu: &Gpu) -> Result<Vec<u8>, Errors> {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.resolve.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| device_error(format!("GPU wait failed: {e}")))?;
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        {
            let data = slice
                .get_mapped_range()
                .map_err(|e| device_error(format!("readback failed: {e}")))?;
            for row in data.chunks_exact(self.padded_row as usize) {
                pixels.extend_from_slice(&row[..(self.width * 4) as usize]);
            }
        }
        self.readback.unmap();
        Ok(pixels)
    }
}
