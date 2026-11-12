//! GPU setup and offscreen rendering with readback.

use crate::extract::{CameraData, FrameData};
use crate::renderer::{self, DEPTH_FORMAT, Renderer, Target};
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("marionette"),
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

/// Renders frames into textures with no window and reads them back.
pub struct Offscreen {
    pub renderer: Renderer,
    width: u32,
    height: u32,
    msaa: Option<wgpu::TextureView>,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_row: u32,
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
            attach | wgpu::TextureUsages::COPY_SRC,
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
            width,
            height,
            msaa,
            resolve,
            resolve_view,
            depth,
            readback,
            padded_row,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Renders the frame from the camera with stable id `camera` (or the first camera) and
    /// waits for the GPU to finish. Asset problems and a missing camera come back as data.
    pub fn render(&mut self, gpu: &Gpu, frame: &FrameData, camera: Option<u64>) -> Result<(), Errors> {
        let cam = pick_camera(frame, camera)?;
        let errors = self.renderer.prepare(frame);
        if !errors.is_empty() {
            return Err(Errors(errors));
        }
        let view = cam.view(self.width as f32 / self.height as f32);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = Target {
            color: self.msaa.as_ref().unwrap_or(&self.resolve_view),
            resolve: self.msaa.as_ref().map(|_| &self.resolve_view),
            depth: &self.depth,
            width: self.width,
            height: self.height,
        };
        self.renderer.render(frame, &view, &target, &mut encoder);
        gpu.queue.submit([encoder.finish()]);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| device_error(format!("GPU wait failed: {e}")))?;
        Ok(())
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
