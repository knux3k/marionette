//! Windowed mode: a thin presentation layer over the same renderer and simulation.
//!
//! The simulation still advances in fixed ticks; wall-clock time only decides how many
//! ticks to run before each frame, never what a tick computes.
//!
//! This crate knows nothing about input or audio modules. Live keyboard and mouse input,
//! `UiButton` clicks and per-frame work (playing sounds) reach the caller through
//! [`WindowHooks`], which queues them for the next tick so they enter the simulation the same
//! way scripted input does.

use crate::extract::extract;
use crate::gpu::{Gpu, pick_camera};
use crate::renderer::{self, DEPTH_FORMAT, Renderer, Target};
use marionette_core::builtin::{DEFAULT_DT, FixedStep};
use marionette_core::scene::SceneId;
use marionette_core::ui::{UiButton, UiNode, layout};
use marionette_core::{Diagnostic, Errors, Sim};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey, PhysicalKey};
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

/// What the window hands back to its caller. Every method has a default that does nothing.
pub trait WindowHooks {
    /// A device input changed (`key:KeyW`, `mouse:Left`; value 1 pressed, 0 released). Queue
    /// it for tick `sim.tick()`, the next tick to run.
    fn device(&mut self, _sim: &mut Sim, _input: &str, _value: f32) {}
    /// An action is set directly (a `UiButton` click sets it to 1, then to 0 one tick later).
    fn action(&mut self, _sim: &mut Sim, _tick: u64, _action: &str, _value: f32) {}
    /// Called once per frame after the simulation caught up (for example to play new sounds).
    fn after_ticks(&mut self, _sim: &mut Sim) {}
}

/// Hooks that do nothing (a plain viewer).
pub struct NoHooks;
impl WindowHooks for NoHooks {}

/// Input name of a key: `key:` plus winit's key code name (`KeyW`, `Space`, `ArrowLeft`).
pub fn key_name(key: PhysicalKey) -> Option<String> {
    match key {
        PhysicalKey::Code(code) => Some(format!("key:{code:?}")),
        PhysicalKey::Unidentified(_) => None,
    }
}

/// Pixels of mouse movement per unit of the `axis:MouseX` / `axis:MouseY` value.
pub const MOUSE_PIXELS_PER_UNIT: f32 = 100.0;

/// Mouse movement in pixels as the values of `axis:MouseX` (right is +) and `axis:MouseY`
/// (down is +) for one tick: pixels / [`MOUSE_PIXELS_PER_UNIT`], clamped to -1..1.
pub fn mouse_axes(pixels: (f32, f32)) -> (f32, f32) {
    let f = |p: f32| (p / MOUSE_PIXELS_PER_UNIT).clamp(-1.0, 1.0);
    (f(pixels.0), f(pixels.1))
}

/// Input name of a mouse button: `mouse:Left`, `mouse:Right`, `mouse:Middle`.
pub fn mouse_name(button: MouseButton) -> Option<&'static str> {
    match button {
        MouseButton::Left => Some("mouse:Left"),
        MouseButton::Right => Some("mouse:Right"),
        MouseButton::Middle => Some("mouse:Middle"),
        _ => None,
    }
}

/// The action of the top-most `UiButton` under screen point (`x`, `y`) on a `w` x `h` screen.
/// Ties on layer go to the higher scene id (drawn later, so on top).
pub fn button_at(world: &mut bevy_ecs::world::World, x: f32, y: f32, w: f32, h: f32) -> Option<String> {
    let mut q = world.query::<(&SceneId, &UiNode, &UiButton)>();
    q.iter(world)
        .filter(|(_, n, _)| {
            let (bx, by, bw, bh) = layout(n, w, h);
            x >= bx && x < bx + bw && y >= by && y < by + bh
        })
        .max_by_key(|(id, n, _)| (n.layer, id.0))
        .map(|(_, _, b)| b.action.clone())
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

struct App<'h> {
    sim: Sim,
    hooks: &'h mut dyn WindowHooks,
    cursor: (f32, f32),
    /// Mouse movement in pixels since it was last fed to the simulation (only while focused).
    mouse: (f32, f32),
    focused: bool,
    opts: WindowOptions,
    running: Option<Running>,
    error: Option<Errors>,
    last: Option<Instant>,
    accumulator: f64,
}

/// Opens a window and runs the simulation until the window closes (or `max_ticks`).
/// Returns the simulation so callers can report its final state.
pub fn run_windowed(sim: Sim, opts: WindowOptions, hooks: &mut dyn WindowHooks) -> Result<Sim, Errors> {
    let surface_err = |msg: String| Errors::from(Diagnostic::error(renderer::codes::SURFACE, msg));
    let event_loop = EventLoop::new().map_err(|e| surface_err(format!("cannot create event loop: {e}")))?;
    let mut app = App {
        sim,
        hooks,
        cursor: (0.0, 0.0),
        mouse: (0.0, 0.0),
        focused: true,
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

impl App<'_> {
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
        let mut renderer = Renderer::new(
            &gpu.device,
            &gpu.queue,
            config.format,
            self.opts.sample_count,
            self.opts.base_dir.clone(),
        );
        // Models first seen mid-game decode in the background, not in the frame (decision 0068).
        renderer.set_streaming(true);
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
        let mut moved = false;
        while self.accumulator >= dt {
            if self.opts.max_ticks.is_some_and(|m| self.sim.tick() >= m) {
                event_loop.exit();
                return;
            }
            // Mouse movement since the last tick drives the mouse axes for one tick.
            if self.mouse != (0.0, 0.0) {
                let (x, y) = mouse_axes(self.mouse);
                self.hooks.device(&mut self.sim, "axis:MouseX", x);
                self.hooks.device(&mut self.sim, "axis:MouseY", y);
                self.mouse = (0.0, 0.0);
                moved = true;
            }
            self.sim.step();
            self.accumulator -= dt;
            if moved {
                self.hooks.device(&mut self.sim, "axis:MouseX", 0.0);
                self.hooks.device(&mut self.sim, "axis:MouseY", 0.0);
                moved = false;
            }
        }
        self.hooks.after_ticks(&mut self.sim);

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

impl ApplicationHandler for App<'_> {
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
            WindowEvent::KeyboardInput { event, .. } => {
                if !event.repeat
                    && let Some(name) = key_name(event.physical_key)
                {
                    let value = if event.state == ElementState::Pressed {
                        1.0
                    } else {
                        0.0
                    };
                    self.hooks.device(&mut self.sim, &name, value);
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.mouse = (0.0, 0.0);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(name) = mouse_name(button) {
                    let pressed = state == ElementState::Pressed;
                    self.hooks
                        .device(&mut self.sim, name, if pressed { 1.0 } else { 0.0 });
                    if pressed
                        && button == MouseButton::Left
                        && let Some(r) = self.running.as_ref()
                    {
                        let (w, h) = (r.config.width as f32, r.config.height as f32);
                        let (x, y) = self.cursor;
                        if let Some(action) = button_at(&mut self.sim.world, x, y, w, h) {
                            let tick = self.sim.tick();
                            self.hooks.action(&mut self.sim, tick, &action, 1.0);
                            self.hooks.action(&mut self.sim, tick + 1, &action, 0.0);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.focused
        {
            self.mouse.0 += delta.0 as f32;
            self.mouse.1 += delta.1 as f32;
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(r) = &self.running {
            r.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;
    use marionette_core::spatial::Vec2;
    use marionette_core::ui::Anchor;
    use winit::keyboard::KeyCode;

    #[test]
    fn device_names_match_the_action_map_format() {
        assert_eq!(
            key_name(PhysicalKey::Code(KeyCode::KeyW)).as_deref(),
            Some("key:KeyW")
        );
        assert_eq!(
            key_name(PhysicalKey::Code(KeyCode::Space)).as_deref(),
            Some("key:Space")
        );
        assert_eq!(
            key_name(PhysicalKey::Code(KeyCode::ArrowLeft)).as_deref(),
            Some("key:ArrowLeft")
        );
        assert_eq!(mouse_name(MouseButton::Left), Some("mouse:Left"));
        assert_eq!(mouse_axes((50.0, -250.0)), (0.5, -1.0));
        assert_eq!(mouse_name(MouseButton::Back), None);
    }

    fn button(world: &mut World, id: u64, offset: (f32, f32), layer: i32, action: &str) {
        world.spawn((
            SceneId(id),
            UiNode {
                anchor: Anchor::TopLeft,
                offset: Vec2 {
                    x: offset.0,
                    y: offset.1,
                },
                size: Vec2 { x: 100.0, y: 40.0 },
                layer,
            },
            UiButton {
                action: action.into(),
            },
        ));
    }

    #[test]
    fn clicks_hit_the_top_most_button() {
        let mut world = World::new();
        button(&mut world, 1, (10.0, 10.0), 0, "start");
        button(&mut world, 2, (60.0, 30.0), 1, "options");
        button(&mut world, 3, (300.0, 300.0), 0, "quit");
        let at = |w: &mut World, x, y| button_at(w, x, y, 800.0, 600.0);
        assert_eq!(at(&mut world, 20.0, 20.0).as_deref(), Some("start"));
        assert_eq!(at(&mut world, 80.0, 40.0).as_deref(), Some("options"));
        assert_eq!(at(&mut world, 350.0, 320.0).as_deref(), Some("quit"));
        assert_eq!(at(&mut world, 5.0, 5.0), None);
        assert_eq!(at(&mut world, 110.0, 10.0), None, "right edge is exclusive");
    }
}
