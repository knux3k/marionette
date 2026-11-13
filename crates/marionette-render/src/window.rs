//! Windowed mode: a thin presentation layer over the same renderer and simulation.
//!
//! The simulation still advances in fixed ticks; wall-clock time only decides how many
//! ticks to run before each frame, never what a tick computes.
//!
//! This crate knows nothing about input or audio modules. Live keyboard and mouse input,
//! `UiButton` clicks and per-frame work (playing sounds) reach the caller through
//! [`WindowHooks`], which queues them for the next tick so they enter the simulation the same
//! way scripted input does.
//!
//! Play feel (decisions 0170, 0171, 0173): the present mode, frame queue depth and an optional
//! frame cap are options; F11 or Alt+Enter toggles borderless fullscreen; losing focus releases
//! every held key and button; a minimized or hidden window sleeps instead of spinning; every run
//! measures its frame pacing and input latency into a [`FrameStats`].

use crate::extract::extract;
use crate::frame_stats::FrameStats;
use crate::gpu::{Gpu, pick_camera};
use crate::renderer::{self, DEPTH_FORMAT, Renderer, Target};
use marionette_core::builtin::{DEFAULT_DT, FixedStep};
use marionette_core::scene::SceneId;
use marionette_core::ui::{UiButton, UiNode, layout};
use marionette_core::{Diagnostic, Errors, Sim};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

/// How finished frames reach the screen (decision 0170).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Present {
    /// Wait for the display refresh (no tearing; the default).
    #[default]
    Vsync,
    /// Newest frame wins at each refresh (no tearing, lower latency, more GPU work). Falls
    /// back to `Vsync` where the driver lacks it.
    Mailbox,
    /// Show frames as soon as they are done (tearing, lowest latency). Falls back to
    /// `Mailbox`, then `Vsync`.
    Immediate,
}

impl Present {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vsync" => Some(Self::Vsync),
            "mailbox" => Some(Self::Mailbox),
            "immediate" => Some(Self::Immediate),
            _ => None,
        }
    }
}

/// The first mode of the preference order for `want` that the surface supports.
pub fn choose_present(supported: &[wgpu::PresentMode], want: Present) -> wgpu::PresentMode {
    use wgpu::PresentMode as M;
    let order: &[M] = match want {
        Present::Vsync => &[M::Fifo],
        Present::Mailbox => &[M::Mailbox, M::Fifo],
        Present::Immediate => &[M::Immediate, M::Mailbox, M::Fifo],
    };
    order
        .iter()
        .copied()
        .find(|m| supported.contains(m))
        .unwrap_or(M::Fifo)
}

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
    /// How frames reach the screen.
    pub present: Present,
    /// Frames the GPU may queue ahead of the display: 1 is the lowest input latency, 2 the
    /// default, 3 the smoothest under uneven load.
    pub frame_latency: u32,
    /// Most frames per second when the present mode does not wait for the display.
    pub fps_cap: Option<f64>,
    /// Start in borderless fullscreen.
    pub fullscreen: bool,
    /// F11 and Alt+Enter toggle borderless fullscreen (those keys are then not passed on).
    pub fullscreen_keys: bool,
    /// Escape closes the window. A game with its own pause menu turns this off and binds
    /// `key:Escape`.
    pub escape_exits: bool,
    /// Hide and lock the cursor while the window has focus (mouse look).
    pub grab_cursor: bool,
    /// Keep simulating while minimized or hidden (no frames are drawn then).
    pub run_in_background: bool,
    /// Close after this much wall time (scripted checks on real screens).
    pub max_seconds: Option<f64>,
}

impl WindowOptions {
    /// A 1280x720 window with 4x MSAA, vsync and every option at its default.
    pub fn new(title: impl Into<String>, base_dir: PathBuf) -> Self {
        Self {
            title: title.into(),
            width: 1280,
            height: 720,
            camera: None,
            sample_count: 4,
            max_ticks: None,
            base_dir,
            present: Present::Vsync,
            frame_latency: 2,
            fps_cap: None,
            fullscreen: false,
            fullscreen_keys: true,
            escape_exits: true,
            grab_cursor: false,
            run_in_background: false,
            max_seconds: None,
        }
    }
}

/// What [`run_windowed`] hands back.
pub struct WindowRun {
    pub sim: Sim,
    pub stats: FrameStats,
}

/// What the window hands back to its caller. Every method has a default that does nothing.
pub trait WindowHooks {
    /// A device input changed (`key:KeyW`, `mouse:Left`; value 1 pressed, 0 released). Queue
    /// it for tick `sim.tick()`, the next tick to run.
    fn device(&mut self, _sim: &mut Sim, _input: &str, _value: f32) {}
    /// An action is set directly (a `UiButton` click sets it to 1, then to 0 one tick later).
    fn action(&mut self, _sim: &mut Sim, _tick: u64, _action: &str, _value: f32) {}
    /// Called once per frame before the simulation catches up (for example to poll gamepads
    /// and queue their inputs). Returns true when it queued input, for the latency numbers.
    fn before_ticks(&mut self, _sim: &mut Sim) -> bool {
        false
    }
    /// Called once per frame after the simulation caught up (for example to play new sounds).
    fn after_ticks(&mut self, _sim: &mut Sim) {}
    /// The window gained or lost focus. Keys and mouse buttons the window saw held are
    /// already released through [`WindowHooks::device`]; release what else the hooks hold.
    fn focus(&mut self, _sim: &mut Sim, _focused: bool) {}
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
    vsync: bool,
    refresh: Option<f64>,
}

struct App<'h> {
    sim: Sim,
    hooks: &'h mut dyn WindowHooks,
    cursor: (f32, f32),
    /// Mouse movement in pixels since it was last fed to the simulation (only while focused).
    mouse: (f32, f32),
    focused: bool,
    /// Minimized or fully hidden: no frames are drawn.
    hidden: bool,
    /// The OS said the window is fully covered (`Occluded`).
    occluded: bool,
    /// The window has zero width or height (minimized on some platforms).
    zero_size: bool,
    modifiers: ModifiersState,
    /// Keys and mouse buttons currently held, released on focus loss.
    held: BTreeSet<String>,
    opts: WindowOptions,
    running: Option<Running>,
    error: Option<Errors>,
    last: Option<Instant>,
    accumulator: f64,
    started: Instant,
    last_present: Option<Instant>,
    /// When the oldest input not yet simulated reached the window.
    pending_input: Option<Instant>,
    /// Earliest start of the next frame under `fps_cap`.
    next_frame: Option<Instant>,
    presented: u64,
    stats: FrameStats,
}

/// Frames at the start of a run left out of the frame numbers (pipeline compiles, first
/// model uploads, the window manager's first resizes).
pub const WARMUP_FRAMES: u64 = 30;

/// Opens a window and runs the simulation until the window closes (or `max_ticks`,
/// `max_seconds`). Returns the simulation and the frame numbers.
pub fn run_windowed(sim: Sim, opts: WindowOptions, hooks: &mut dyn WindowHooks) -> Result<WindowRun, Errors> {
    let surface_err = |msg: String| Errors::from(Diagnostic::error(renderer::codes::SURFACE, msg));
    let event_loop = EventLoop::new().map_err(|e| surface_err(format!("cannot create event loop: {e}")))?;
    let mut app = App {
        sim,
        hooks,
        cursor: (0.0, 0.0),
        mouse: (0.0, 0.0),
        focused: true,
        hidden: false,
        occluded: false,
        zero_size: false,
        modifiers: ModifiersState::empty(),
        held: BTreeSet::new(),
        opts,
        running: None,
        error: None,
        last: None,
        accumulator: 0.0,
        started: Instant::now(),
        last_present: None,
        pending_input: None,
        next_frame: None,
        presented: 0,
        stats: FrameStats::default(),
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| surface_err(format!("event loop failed: {e}")))?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(WindowRun {
            sim: app.sim,
            stats: app.stats,
        }),
    }
}

/// Snaps a measured frame time to the display refresh when it is within 5% of it, so timer
/// noise around a refresh equal to the tick rate does not alternate 0- and 2-tick frames
/// (decision 0171). Only used while presenting with vsync.
pub fn snap_to_refresh(elapsed: f64, refresh_hz: Option<f64>) -> f64 {
    match refresh_hz {
        Some(hz) if hz > 0.0 => {
            let period = 1.0 / hz;
            if (elapsed - period).abs() < period * 0.05 {
                period
            } else {
                elapsed
            }
        }
        _ => elapsed,
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
        let mut attrs = Window::default_attributes()
            .with_title(self.opts.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(self.opts.width, self.opts.height));
        if self.opts.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
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
        config.present_mode = choose_present(&caps.present_modes, self.opts.present);
        config.desired_maximum_frame_latency = self.opts.frame_latency.clamp(1, 3);
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
        let refresh = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| f64::from(mhz) / 1000.0);
        let vsync = matches!(
            config.present_mode,
            wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed
        );
        self.stats.present_mode = format!("{:?}", config.present_mode).to_lowercase();
        self.stats.frame_latency = config.desired_maximum_frame_latency;
        self.stats.refresh_hz = refresh;
        self.stats.size = (config.width, config.height);
        self.stats.fullscreen = self.opts.fullscreen;
        if self.opts.grab_cursor {
            grab(&window, true);
        }
        Ok(Running {
            window,
            surface,
            config,
            gpu,
            renderer,
            msaa,
            depth,
            vsync,
            refresh,
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, e: Errors) {
        self.error = Some(e);
        event_loop.exit();
    }

    /// Runs the fixed ticks that real time owes; returns how many ran, or `None` when the run
    /// is over (`max_ticks`).
    fn advance(&mut self, now: Instant) -> Option<u32> {
        let dt = self
            .sim
            .world
            .get_resource::<FixedStep>()
            .map_or(DEFAULT_DT, |s| s.dt) as f64;
        if let Some(last) = self.last {
            let mut elapsed = (now - last).as_secs_f64();
            if let Some(r) = self.running.as_ref()
                && r.vsync
                && !self.hidden
            {
                elapsed = snap_to_refresh(elapsed, r.refresh);
            }
            self.accumulator = (self.accumulator + elapsed).min(dt * 8.0);
        }
        self.last = Some(now);
        let mut ran = 0;
        let mut moved = false;
        while self.accumulator >= dt {
            if self.opts.max_ticks.is_some_and(|m| self.sim.tick() >= m) {
                return None;
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
            ran += 1;
            self.accumulator -= dt;
            if moved {
                self.hooks.device(&mut self.sim, "axis:MouseX", 0.0);
                self.hooks.device(&mut self.sim, "axis:MouseY", 0.0);
                moved = false;
            }
        }
        Some(ran)
    }

    fn timed_out(&self) -> bool {
        self.opts
            .max_seconds
            .is_some_and(|s| self.started.elapsed().as_secs_f64() >= s)
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let start = Instant::now();
        if self.timed_out() {
            return event_loop.exit();
        }
        if self.hooks.before_ticks(&mut self.sim) {
            self.pending_input.get_or_insert(start);
        }
        let Some(ran) = self.advance(start) else {
            return event_loop.exit();
        };
        self.stats.ticks(ran);
        let input_at = if ran > 0 { self.pending_input.take() } else { None };
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
                self.stats.skipped += 1;
                return;
            }
            _ => {
                // Timeout or occluded: skip this frame.
                self.stats.skipped += 1;
                return;
            }
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
        let done = Instant::now();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        self.presented += 1;
        // The first frames compile pipelines and load models: counted apart (decision 0173).
        if self.presented > WARMUP_FRAMES {
            self.stats.cpu.add(ms(done - start));
            if let Some(at) = input_at {
                self.stats.input_latency.add(ms(done - at));
            }
            if let Some(prev) = self.last_present {
                self.stats.interval.add(ms(done - prev));
            }
        } else {
            self.stats.warmup_ms += ms(done - start);
        }
        self.last_present = Some(done);
        if let Some(cap) = self.opts.fps_cap.filter(|c| *c > 0.0 && !r.vsync) {
            let period = Duration::from_secs_f64(1.0 / cap);
            // Keep a steady cadence; restart it after a long stall instead of catching up.
            let next = self.next_frame.map_or(start + period, |n| n + period);
            self.next_frame = Some(if next < done { done + period / 2 } else { next });
        }
    }

    fn input_now(&mut self) {
        self.pending_input.get_or_insert_with(Instant::now);
    }

    fn device(&mut self, name: &str, pressed: bool) {
        let changed = if pressed {
            self.held.insert(name.to_string())
        } else {
            self.held.remove(name)
        };
        if changed {
            self.hooks
                .device(&mut self.sim, name, if pressed { 1.0 } else { 0.0 });
            self.input_now();
        }
    }

    fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
        self.mouse = (0.0, 0.0);
        if !focused {
            self.stats.focus_losses += 1;
            // Keys released while another window had focus never arrive: release them now.
            for name in std::mem::take(&mut self.held) {
                self.hooks.device(&mut self.sim, &name, 0.0);
            }
        }
        self.hooks.focus(&mut self.sim, focused);
        if self.opts.grab_cursor
            && let Some(r) = &self.running
        {
            grab(&r.window, focused);
        }
    }

    fn toggle_fullscreen(&mut self) {
        if let Some(r) = &self.running {
            let on = r.window.fullscreen().is_none();
            r.window
                .set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
            self.stats.fullscreen = on;
        }
    }

    fn set_hidden(&mut self, hidden: bool) {
        if hidden != self.hidden {
            self.hidden = hidden;
            if !self.opts.run_in_background {
                // Paused while hidden: do not owe the hidden time on return.
                self.last = None;
                self.accumulator = 0.0;
            }
        }
    }
}

/// Hides and locks (or confines, where locking is unsupported) the cursor, or frees it.
fn grab(window: &Window, on: bool) {
    if on {
        let _ = window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
    } else {
        let _ = window.set_cursor_grab(CursorGrabMode::None);
    }
    window.set_cursor_visible(!on);
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::Resized(size) => {
                self.zero_size = size.width == 0 || size.height == 0;
                if let Some(r) = self.running.as_mut()
                    && size.width > 0
                    && size.height > 0
                    && (size.width, size.height) != (r.config.width, r.config.height)
                {
                    r.config.width = size.width;
                    r.config.height = size.height;
                    r.surface.configure(&r.gpu.device, &r.config);
                    (r.msaa, r.depth) = attachments(&r.gpu.device, &r.config, self.opts.sample_count);
                    self.stats.resizes += 1;
                    self.stats.size = (size.width, size.height);
                }
            }
            WindowEvent::Occluded(hidden) => self.occluded = hidden,
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if pressed && self.opts.escape_exits && event.logical_key == Key::Named(NamedKey::Escape) {
                    return event_loop.exit();
                }
                let toggle = event.logical_key == Key::Named(NamedKey::F11)
                    || (self.modifiers.alt_key() && event.logical_key == Key::Named(NamedKey::Enter));
                if self.opts.fullscreen_keys && toggle {
                    if pressed && !event.repeat {
                        self.toggle_fullscreen();
                    }
                    return;
                }
                if !event.repeat
                    && let Some(name) = key_name(event.physical_key)
                {
                    self.device(&name, pressed);
                }
            }
            WindowEvent::Focused(focused) => self.set_focus(focused),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(name) = mouse_name(button) {
                    let pressed = state == ElementState::Pressed;
                    self.device(name, pressed);
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
            WindowEvent::RedrawRequested if !self.hidden => self.frame(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.focused
        {
            self.mouse.0 += delta.0 as f32;
            self.mouse.1 += delta.1 as f32;
            self.input_now();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none() {
            return;
        }
        if self.timed_out() {
            return event_loop.exit();
        }
        // Windows reports minimizing neither as a zero size nor as `Occluded`: ask the window.
        let minimized = self
            .running
            .as_ref()
            .and_then(|r| r.window.is_minimized())
            .unwrap_or(false);
        self.set_hidden(minimized || self.occluded || self.zero_size);
        if self.hidden {
            // Nothing to draw: sleep. Keep ticking at the tick rate if asked to.
            if self.opts.run_in_background {
                self.hooks.before_ticks(&mut self.sim);
                if self.advance(Instant::now()).is_none() {
                    return event_loop.exit();
                }
                self.hooks.after_ticks(&mut self.sim);
                let dt = self
                    .sim
                    .world
                    .get_resource::<FixedStep>()
                    .map_or(DEFAULT_DT, |s| s.dt);
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_secs_f32(dt),
                ));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            return;
        }
        if let Some(next) = self.next_frame
            && Instant::now() < next
        {
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
            return;
        }
        event_loop.set_control_flow(ControlFlow::Poll);
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

    #[test]
    fn present_modes_fall_back_in_order() {
        use wgpu::PresentMode as M;
        let all = [M::Fifo, M::Mailbox, M::Immediate];
        assert_eq!(choose_present(&all, Present::Vsync), M::Fifo);
        assert_eq!(choose_present(&all, Present::Mailbox), M::Mailbox);
        assert_eq!(choose_present(&all, Present::Immediate), M::Immediate);
        assert_eq!(
            choose_present(&[M::Fifo, M::Mailbox], Present::Immediate),
            M::Mailbox
        );
        assert_eq!(
            choose_present(&[M::Fifo, M::Immediate], Present::Mailbox),
            M::Fifo
        );
        assert_eq!(choose_present(&[], Present::Immediate), M::Fifo);
        assert_eq!(Present::parse("mailbox"), Some(Present::Mailbox));
        assert_eq!(Present::parse("fast"), None);
    }

    #[test]
    fn frame_times_snap_to_the_refresh_only_when_close() {
        let p60 = 1.0 / 60.0;
        assert_eq!(snap_to_refresh(p60 * 1.03, Some(60.0)), p60);
        assert_eq!(snap_to_refresh(p60 * 0.97, Some(60.0)), p60);
        assert_eq!(snap_to_refresh(p60 * 1.2, Some(60.0)), p60 * 1.2);
        assert_eq!(
            snap_to_refresh(p60 * 2.0, Some(60.0)),
            p60 * 2.0,
            "a missed refresh stays missed"
        );
        assert_eq!(snap_to_refresh(0.0071, None), 0.0071);
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
