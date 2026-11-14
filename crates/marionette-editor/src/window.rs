//! The editor window: turns winit events into [`Editor`] commands and draws each frame
//! through the renderer's shared [`Presenter`].

use crate::overlay::Layout;
use crate::state::{Editor, Host};
use glam::Vec3;
use marionette_core::builtin::{DEFAULT_DT, FixedStep};
use marionette_core::{Diagnostic, Errors};
use marionette_render::extract;
use marionette_render::window::{Presenter, key_name, mouse_name};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

/// Options for [`run`].
pub struct EditorOptions {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub sample_count: u32,
    pub base_dir: PathBuf,
    /// Close after this many frames (for scripted checks); `None` runs until closed.
    pub max_frames: Option<u64>,
}

/// Opens the editor window and runs until it is closed. Returns the editor (its last state)
/// so the caller can report it.
pub fn run(editor: Editor, opts: EditorOptions, host: &mut dyn Host) -> Result<Editor, Errors> {
    let err = |m: String| Errors::from(Diagnostic::error(marionette_render::renderer::codes::SURFACE, m));
    let event_loop = EventLoop::new().map_err(|e| err(format!("cannot create event loop: {e}")))?;
    let mut app = App {
        editor,
        host,
        opts,
        presenter: None,
        error: None,
        held: BTreeSet::new(),
        mods: ModifiersState::empty(),
        look: false,
        pan: false,
        cursor: (0.0, 0.0),
        mouse: (0.0, 0.0),
        last: None,
        accumulator: 0.0,
        frames: 0,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| err(format!("event loop failed: {e}")))?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(app.editor),
    }
}

struct App<'h> {
    editor: Editor,
    host: &'h mut dyn Host,
    opts: EditorOptions,
    presenter: Option<Presenter>,
    error: Option<Errors>,
    held: BTreeSet<KeyCode>,
    mods: ModifiersState,
    /// Right mouse button held: mouse turns the camera.
    look: bool,
    /// Middle mouse button held: mouse drags the view.
    pan: bool,
    cursor: (f32, f32),
    /// Mouse movement in pixels since the last frame.
    mouse: (f32, f32),
    last: Option<Instant>,
    accumulator: f64,
    frames: u64,
}

impl App<'_> {
    /// Keys go to the game while it plays through its own camera.
    fn game_input(&self) -> bool {
        self.editor.playing && self.editor.game_view
    }

    fn layout(&self) -> Layout {
        let (w, h) = self.presenter.as_ref().map_or((1, 1), |p| p.size());
        Layout::new(w as f32, h as f32, self.editor.panels)
    }

    fn key(&mut self, code: KeyCode) {
        let ctrl = self.mods.control_key();
        let shift = self.mods.shift_key();
        let step = if shift { 1.0 } else { 0.25 };
        let rows = self.layout().list_rows();
        let e = &mut self.editor;
        let host = &mut *self.host;
        match code {
            KeyCode::KeyP | KeyCode::Space => e.toggle_play(),
            KeyCode::KeyN | KeyCode::Period => e.step(host),
            KeyCode::KeyR if !ctrl => e.reset(host),
            KeyCode::KeyC if !ctrl => e.game_view = !e.game_view,
            KeyCode::KeyG => e.grid = !e.grid,
            KeyCode::Tab => e.panels = !e.panels,
            KeyCode::KeyF => {
                if let Some(p) = &self.presenter {
                    let frame = extract(&e.sim.world);
                    let items = e.items(&frame, &|a| p.renderer.model_bounds(a));
                    e.frame_selection(&items);
                    e.game_view = false;
                }
            }
            KeyCode::BracketRight => e.select_step(1, rows),
            KeyCode::BracketLeft => e.select_step(-1, rows),
            KeyCode::Escape => e.select(None, rows),
            KeyCode::ArrowLeft => e.nudge(host, Vec3::new(-step, 0.0, 0.0)),
            KeyCode::ArrowRight => e.nudge(host, Vec3::new(step, 0.0, 0.0)),
            KeyCode::ArrowUp => e.nudge(host, Vec3::new(0.0, 0.0, -step)),
            KeyCode::ArrowDown => e.nudge(host, Vec3::new(0.0, 0.0, step)),
            KeyCode::PageUp => e.nudge(host, Vec3::new(0.0, step, 0.0)),
            KeyCode::PageDown => e.nudge(host, Vec3::new(0.0, -step, 0.0)),
            KeyCode::KeyD if ctrl => e.duplicate(host),
            KeyCode::Delete => e.delete(host),
            KeyCode::KeyZ if ctrl => e.undo(host),
            _ => {}
        }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt_real = self.last.map_or(0.0, |l| (now - l).as_secs_f64());
        self.last = Some(now);
        // The camera flies on wall-clock time; it never touches the simulation.
        let ctrl = self.mods.control_key();
        if !self.game_input() && !ctrl {
            let held = |k| self.held.contains(&k);
            let axis = |a, b| (held(a) as i32 - held(b) as i32) as f32;
            let dir = Vec3::new(
                axis(KeyCode::KeyD, KeyCode::KeyA),
                axis(KeyCode::KeyE, KeyCode::KeyQ),
                axis(KeyCode::KeyW, KeyCode::KeyS),
            );
            if dir != Vec3::ZERO {
                let fast = self.mods.shift_key();
                self.editor.camera.fly(dir, fast, dt_real.min(0.1) as f32);
            }
        }
        let (dx, dy) = std::mem::take(&mut self.mouse);
        if self.look {
            self.editor.camera.look(dx, dy);
        }
        if self.pan {
            let h = self.layout().h;
            let depth = self.editor.camera.speed.max(1.0);
            self.editor.camera.pan(dx, dy, h, depth);
        }
        // Playing: fixed ticks to catch up with real time, as in the game window.
        if self.editor.playing {
            let dt = self
                .editor
                .sim
                .world
                .get_resource::<FixedStep>()
                .map_or(DEFAULT_DT, |s| s.dt) as f64;
            self.accumulator = (self.accumulator + dt_real).min(dt * 8.0);
            let before = self.editor.entities().len();
            while self.accumulator >= dt {
                self.editor.sim.step();
                self.accumulator -= dt;
            }
            self.host.after_ticks(&mut self.editor.sim);
            // Spawns and removals change the list; a count check keeps this cheap.
            if inspect_count(&self.editor) != before {
                self.editor.refresh();
            }
        } else {
            self.accumulator = 0.0;
        }

        let Some(p) = self.presenter.as_mut() else { return };
        let mut frame = extract(&self.editor.sim.world);
        let (w, h) = p.size();
        let layout = Layout::new(w as f32, h as f32, self.editor.panels);
        let view = self.editor.view(&frame, w as f32 / h as f32);
        let items = self.editor.items(&frame, &|a| p.renderer.model_bounds(a));
        self.editor.overlay(&mut frame, &items, &view, layout);
        match p.draw(&frame, &view) {
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
            Ok(_) if p.lost() => event_loop.exit(),
            Ok(_) => {}
        }
        self.frames += 1;
        if self.opts.max_frames.is_some_and(|m| self.frames >= m) {
            event_loop.exit();
        }
    }
}

fn inspect_count(e: &Editor) -> usize {
    e.sim
        .world
        .try_query::<&marionette_core::scene::SceneId>()
        .map_or(0, |mut q| q.iter(&e.sim.world).count())
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.presenter.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(self.opts.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(self.opts.width, self.opts.height));
        let made = event_loop
            .create_window(attrs)
            .map_err(|e| {
                Errors::from(Diagnostic::error(
                    marionette_render::renderer::codes::SURFACE,
                    format!("cannot open window: {e}"),
                ))
            })
            .and_then(|w| Presenter::new(Arc::new(w), self.opts.sample_count, self.opts.base_dir.clone()));
        match made {
            Ok(p) => self.presenter = Some(p),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(p) = self.presenter.as_mut() {
                    p.resize(size.width, size.height);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::Focused(false) => {
                self.held.clear();
                self.look = false;
                self.pan = false;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                if self.game_input() && !matches!(code, KeyCode::KeyP | KeyCode::KeyC | KeyCode::Escape) {
                    if !event.repeat
                        && let Some(name) = key_name(event.physical_key)
                    {
                        self.host
                            .device(&mut self.editor.sim, &name, if pressed { 1.0 } else { 0.0 });
                    }
                    return;
                }
                if pressed {
                    self.held.insert(code);
                    if !event.repeat
                        || matches!(
                            code,
                            KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::ArrowUp
                                | KeyCode::ArrowDown
                                | KeyCode::PageUp
                                | KeyCode::PageDown
                        )
                    {
                        self.key(code);
                    }
                } else {
                    self.held.remove(&code);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                let layout = self.layout();
                if layout.list_row_at(self.cursor.0, self.cursor.1).is_some() {
                    self.editor.scroll_list((-steps * 3.0) as i64, layout.list_rows());
                } else {
                    self.editor.camera.scroll(steps);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if self.game_input() {
                    if let Some(name) = mouse_name(button) {
                        self.host
                            .device(&mut self.editor.sim, name, if pressed { 1.0 } else { 0.0 });
                    }
                    return;
                }
                match button {
                    MouseButton::Right => self.look = pressed,
                    MouseButton::Middle => self.pan = pressed,
                    MouseButton::Left if pressed => {
                        if let Some(p) = &self.presenter {
                            let frame = extract(&self.editor.sim.world);
                            let (w, h) = p.size();
                            let layout = Layout::new(w as f32, h as f32, self.editor.panels);
                            let view = self.editor.view(&frame, w as f32 / h as f32);
                            let items = self.editor.items(&frame, &|a| p.renderer.model_bounds(a));
                            self.editor
                                .click(&items, &view, &layout, self.cursor.0, self.cursor.1);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && (self.look || self.pan)
        {
            self.mouse.0 += delta.0 as f32;
            self.mouse.1 += delta.1 as f32;
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(p) = &self.presenter {
            p.window.request_redraw();
        }
    }
}
