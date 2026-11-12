//! Windowed mode: a thin presentation layer over the same renderer and simulation.
//!
//! The simulation still advances in fixed ticks; wall-clock time only decides how many
//! ticks to run before each frame, never what a tick computes.

use crate::extract::extract;
use crate::gpu::{Gpu, pick_camera};
use crate::renderer::{self, DEPTH_FORMAT, Renderer, Target};
use marionette_core::builtin::{DEFAULT_DT, FixedStep};
use marionette_core::{Diagnostic, Errors, Sim};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// Options for [`run_windowed`].
pub struct WindowOptions {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub camera: Option<u64>,
    pub sample_count: u32,
    /// Close the window after this many ticks (for scripted checks). `None` runs until closed.
    pub max_ticks: Option<u64>,
    pub base_dir: PathBuf,
}

struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    renderer: Renderer,
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
}

struct App {
    sim: Sim,
    opts: WindowOptions,
    running: Option<Running>,
    error: Option<Errors>,
    last: Option<Instant>,
    accumulator: f64,
}

/// Opens a window and runs the simulation until the window closes (or `max_ticks`).
/// Returns the simulation so callers can report its final state.
pub fn run_windowed(sim: Sim, opts: WindowOptions) -> Result<Sim, Errors> {
    let surface_err = |msg: String| Errors::from(Diagnostic::error(renderer::codes::SURFACE, msg));
    let event_loop = EventLoop::new().map_err(|e| surface_err(format!("cannot create event loop: {e}")))?;
    let mut app = App {
        sim,
        opts,
        running: None,
        error: None,
        last: None,
        accumulator: 0.0,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| surface_err(format!("event loop failed: {e}")))?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(app.sim),
    }
}

fn attachments(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    samples: u32,
) -> (Option<wgpu::TextureView>, wgpu::TextureView) {
    let make = |format, samples| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("window attachment"),
                size: wgpu::Extent3d {
                    width: config.width,
                    height: config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    (
        (samples > 1).then(|| make(config.format, samples)),
        make(DEPTH_FORMAT, samples),
    )
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, Errors> {
        let surface_err = |msg: String| Errors::from(Diagnostic::error(renderer::codes::SURFACE, msg));
        let attrs = Window::default_attributes()
            .with_title(self.opts.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(self.opts.width, self.opts.height));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| surface_err(format!("cannot open window: {e}")))?,
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| surface_err(format!("cannot create surface: {e}")))?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| surface_err("the GPU cannot present to this window".into()))?;
        // Prefer an sRGB format so shader output (linear) is encoded correctly.
        let caps = surface.get_capabilities(&gpu.adapter);
        if let Some(f) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = *f;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&gpu.device, &config);
        let renderer = Renderer::new(
            &gpu.device,
            &gpu.queue,
            config.format,
            self.opts.sample_count,
            self.opts.base_dir.clone(),
        );
        let (msaa, depth) = attachments(&gpu.device, &config, self.opts.sample_count);
        Ok(Running {
            window,
            surface,
            config,
            gpu,
            renderer,
            msaa,
            depth,
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, e: Errors) {
        self.error = Some(e);
        event_loop.exit();
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        // Advance the simulation in fixed ticks to catch up with real time.
        let dt = self
            .sim
            .world
            .get_resource::<FixedStep>()
            .map_or(DEFAULT_DT, |s| s.dt) as f64;
        let now = Instant::now();
        if let Some(last) = self.last {
            self.accumulator = (self.accumulator + (now - last).as_secs_f64()).min(dt * 8.0);
        }
        self.last = Some(now);
        while self.accumulator >= dt {
            if self.opts.max_ticks.is_some_and(|m| self.sim.tick() >= m) {
                event_loop.exit();
                return;
            }
            self.sim.step();
            self.accumulator -= dt;
        }

        let frame = extract(&self.sim.world);
        let Some(r) = self.running.as_mut() else { return };
        let cam = match pick_camera(&frame, self.opts.camera) {
            Ok(c) => c,
            Err(e) => return self.fail(event_loop, e),
        };
        let errors = r.renderer.prepare(&frame);
        if !errors.is_empty() {
            return self.fail(event_loop, Errors(errors));
        }
        let texture = match r.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                r.surface.configure(&r.gpu.device, &r.config);
                return;
            }
            _ => return, // timeout or occluded: skip this frame
        };
        let view = texture.texture.create_view(&Default::default());
        let target = Target {
            color: r.msaa.as_ref().unwrap_or(&view),
            resolve: r.msaa.as_ref().map(|_| &view),
            depth: &r.depth,
            width: r.config.width,
            height: r.config.height,
        };
        let mut encoder = r.gpu.device.create_command_encoder(&Default::default());
        r.renderer.render(
            &frame,
            &cam.view(r.config.width as f32 / r.config.height as f32),
            &target,
            &mut encoder,
        );
        r.gpu.queue.submit([encoder.finish()]);
        r.window.pre_present_notify();
        r.gpu.queue.present(texture);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none() {
            match self.start(event_loop) {
                Ok(r) => self.running = Some(r),
                Err(e) => self.fail(event_loop, e),
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.running.as_mut()
                    && size.width > 0
                    && size.height > 0
                {
                    r.config.width = size.width;
                    r.config.height = size.height;
                    r.surface.configure(&r.gpu.device, &r.config);
                    (r.msaa, r.depth) = attachments(&r.gpu.device, &r.config, self.opts.sample_count);
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(r) = &self.running {
            r.window.request_redraw();
        }
    }
}
