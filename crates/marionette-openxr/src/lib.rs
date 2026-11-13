//! Marionette on a headset through OpenXR (decision 0190).
//!
//! [`run`] opens an OpenXR session on the installed runtime (SteamVR, Monado, a standalone
//! headset's runtime), renders the scene to both eyes with the engine's renderer and feeds
//! the headset's poses, buttons, sticks and hands into the simulation as input events
//! (`marionette-xr` turns them into gameplay). The simulation still runs in fixed ticks;
//! the display rate only decides how many ticks run before each frame.
//!
//! [`probe`] says whether this machine has a runtime and a headset, without rendering.
//!
//! Graphics: Vulkan (the runtime creates the Vulkan instance and device, wgpu wraps them;
//! `gpu.rs`). One swapchain per eye, each eye a render pass of `Renderer::render_views`.
//! The OpenXR loader and the Vulkan loader are opened at run time, so nothing here is needed
//! to build or to run without a headset.

use bevy_ecs::prelude::*;
use marionette_core::builtin::{DEFAULT_DT, FixedStep};
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use marionette_core::{Diagnostic, Errors, Sim};
use marionette_render::extract::{FrameData, View};
use marionette_render::gpu::pick_camera;
use marionette_render::renderer::{DEPTH_FORMAT, Renderer, Target};
use marionette_xr::{XrComfort, XrHaptics, XrRig, XrTracked};
use openxr as xr;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub mod controls;
pub mod gpu;
mod vignette;

pub use controls::HandJoints;

pub mod codes {
    /// No OpenXR loader or no active runtime on this machine.
    pub const NO_RUNTIME: &str = "xr.no_runtime";
    /// A runtime exists but no headset is connected (or it is asleep).
    pub const NO_HEADSET: &str = "xr.no_headset";
    /// The runtime cannot give us a usable Vulkan GPU.
    pub const VULKAN: &str = "xr.vulkan";
    /// Session, swapchain or frame loop failure.
    pub const SESSION: &str = "xr.session";
}

/// An error with a stable code (converted to engine [`Errors`]).
#[derive(Debug)]
pub struct XrError {
    pub code: &'static str,
    pub message: String,
    pub fix: Option<String>,
}

impl XrError {
    pub fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            fix: None,
        }
    }
    pub fn fix(mut self, fix: &str) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

impl From<XrError> for Errors {
    fn from(e: XrError) -> Self {
        let mut d = Diagnostic::error(e.code, e.message);
        if let Some(f) = e.fix {
            d = d.fix(f);
        }
        d.into()
    }
}

/// Options for [`run`].
#[derive(Debug, Clone)]
pub struct XrOptions {
    /// Camera entity (default: the lowest-id camera). Its near/far planes and clear colour
    /// are used; its [`XrTracked`] rig places the eyes in the world.
    pub camera: Option<u64>,
    /// MSAA samples per eye (1 or 4).
    pub sample_count: u32,
    /// Stop after this many frames (tests); `None` runs until the runtime ends the session.
    pub max_frames: Option<u64>,
    /// Stop after this many ticks.
    pub max_ticks: Option<u64>,
    /// Multiplies the runtime's recommended eye size (1.0 = recommended).
    pub resolution_scale: f32,
    pub base_dir: PathBuf,
    /// Shown by the runtime (for example in SteamVR's binding settings).
    pub app_name: String,
}

impl Default for XrOptions {
    fn default() -> Self {
        Self {
            camera: None,
            sample_count: 4,
            max_frames: None,
            max_ticks: None,
            resolution_scale: 1.0,
            base_dir: PathBuf::from("."),
            app_name: "Marionette".into(),
        }
    }
}

/// What the headset loop hands back to its caller each frame. Defaults do nothing.
pub trait XrHooks {
    /// After the simulation caught up, before rendering (play sounds here).
    fn after_ticks(&mut self, _sim: &mut Sim) {}
    /// Tracked hand joints this frame (play space), for drawing hands. `None` per hand
    /// when hand tracking is off or the hand is not seen.
    fn hands(&mut self, _sim: &mut Sim, _joints: &[Option<HandJoints>; 2]) {}
    /// The head's world pose as rendered this frame (position, rotation x y z w): fresher
    /// than the tick's head entity, for 3D sound (`marionette_audio::Player::set_head`).
    fn head(&mut self, _position: [f32; 3], _rotation: [f32; 4]) {}
}

pub struct NoHooks;
impl XrHooks for NoHooks {}

fn entry() -> Result<xr::Entry, XrError> {
    // SAFETY: loads the system OpenXR loader library.
    unsafe { xr::Entry::load(&()) }.map_err(|e| {
        XrError::new(codes::NO_RUNTIME, format!("no OpenXR loader: {e}")).fix(
            "install an OpenXR runtime (SteamVR, or Monado on Linux) and ship the OpenXR loader \
             (openxr_loader.dll / libopenxr_loader.so.1) next to the game",
        )
    })
}

struct Base {
    instance: xr::Instance,
    system: xr::SystemId,
    hand_tracking: bool,
}

fn instance(app_name: &str) -> Result<Base, XrError> {
    let entry = entry()?;
    let available = entry.enumerate_extensions().map_err(|e| {
        XrError::new(codes::NO_RUNTIME, format!("no active OpenXR runtime: {e}"))
            .fix("start SteamVR (or monado-service) and set it as the OpenXR runtime")
    })?;
    if !available.khr_vulkan_enable2 {
        return Err(XrError::new(
            codes::VULKAN,
            "the OpenXR runtime does not offer Vulkan (XR_KHR_vulkan_enable2)".into(),
        ));
    }
    let mut ext = xr::ExtensionSet::default();
    ext.khr_vulkan_enable2 = true;
    ext.ext_hand_tracking = available.ext_hand_tracking;
    let instance = entry
        .create_instance(
            &xr::ApplicationInfo {
                application_name: app_name,
                application_version: 0,
                engine_name: "Marionette",
                engine_version: 0,
                api_version: xr::Version::new(1, 0, 0),
            },
            &ext,
            &[],
            &(),
        )
        .map_err(|e| {
            XrError::new(
                codes::NO_RUNTIME,
                format!("the OpenXR runtime refused to start: {e}"),
            )
            .fix("start SteamVR (or monado-service) and set it as the OpenXR runtime")
        })?;
    let system = instance
        .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .map_err(|e| {
            XrError::new(codes::NO_HEADSET, format!("no headset found: {e}"))
                .fix("connect and wake the headset, then try again")
        })?;
    let hand_tracking =
        available.ext_hand_tracking && instance.supports_hand_tracking(system).unwrap_or(false);
    Ok(Base {
        instance,
        system,
        hand_tracking,
    })
}

/// Checks for an OpenXR runtime and headset without opening a session. The result names
/// the runtime, the headset, the recommended eye size and whether hand tracking exists.
pub fn probe() -> Result<Value, Errors> {
    let base = instance("Marionette probe")?;
    let props = base
        .instance
        .properties()
        .map_err(|e| XrError::new(codes::NO_RUNTIME, e.to_string()))?;
    let sys = base
        .instance
        .system_properties(base.system)
        .map_err(|e| XrError::new(codes::NO_HEADSET, e.to_string()))?;
    let views = base
        .instance
        .enumerate_view_configuration_views(base.system, xr::ViewConfigurationType::PRIMARY_STEREO)
        .map_err(|e| XrError::new(codes::NO_HEADSET, e.to_string()))?;
    Ok(json!({
        "runtime": format!("{} {}", props.runtime_name, props.runtime_version),
        "headset": sys.system_name,
        "eyes": views.iter().map(|v| json!({
            "recommended": [v.recommended_image_rect_width, v.recommended_image_rect_height],
            "max": [v.max_image_rect_width, v.max_image_rect_height],
        })).collect::<Vec<_>>(),
        "position_tracking": sys.tracking_properties.position_tracking,
        "hand_tracking": base.hand_tracking,
    }))
}

/// Frame time statistics (milliseconds).
fn stats(v: &mut [f64]) -> Value {
    if v.is_empty() {
        return Value::Null;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    let avg = v.iter().sum::<f64>() / v.len() as f64;
    json!({ "avg": (avg * 1000.0).round() / 1000.0, "p50": (at(0.5) * 1000.0).round() / 1000.0,
            "p99": (at(0.99) * 1000.0).round() / 1000.0, "max": (at(1.0) * 1000.0).round() / 1000.0 })
}

struct Eye {
    swapchain: xr::Swapchain<xr::Vulkan>,
    textures: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    width: u32,
    height: u32,
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
}

fn attachment(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("xr eye attachment"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
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
}

/// Asymmetric projection for one eye (right-handed, looking down -Z, depth 0..1 as the
/// renderer's other projections).
pub fn projection(fov: &xr::Fovf, near: f32, far: f32) -> glam::Mat4 {
    let (l, r) = (fov.angle_left.tan(), fov.angle_right.tan());
    let (d, u) = (fov.angle_down.tan(), fov.angle_up.tan());
    glam::Mat4::from_cols(
        glam::Vec4::new(2.0 / (r - l), 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 2.0 / (u - d), 0.0, 0.0),
        glam::Vec4::new((r + l) / (r - l), (u + d) / (u - d), far / (near - far), -1.0),
        glam::Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    )
}

fn mat(t: marionette_core::spatial::Vec3, q: marionette_core::spatial::Quat) -> glam::Mat4 {
    glam::Mat4::from_rotation_translation(
        glam::Quat::from_xyzw(q.x, q.y, q.z, q.w),
        glam::Vec3::new(t.x, t.y, t.z),
    )
}

/// Where the play space is in the world: the camera's rig (with its height offset), or the
/// camera itself when it is not tracked.
fn play_space(sim: &mut Sim, camera: u64) -> glam::Mat4 {
    let world = &mut sim.world;
    let mut q = world.query::<(Entity, &SceneId)>();
    let ids: BTreeMap<u64, Entity> = q.iter(world).map(|(e, id)| (id.0, e)).collect();
    let Some(&cam) = ids.get(&camera) else {
        return glam::Mat4::IDENTITY;
    };
    if let Some(t) = world.get::<XrTracked>(cam).copied()
        && let Some(&rig) = ids.get(&t.rig.0)
    {
        let rt = world.get::<Transform>(rig).copied().unwrap_or_default();
        let h = world.get::<XrRig>(rig).map_or(0.0, |r| r.height_offset);
        return mat(rt.translation, rt.rotation) * glam::Mat4::from_translation(glam::Vec3::new(0.0, h, 0.0));
    }
    let ct = world.get::<Transform>(cam).copied().unwrap_or_default();
    mat(ct.translation, ct.rotation)
}

fn feed(sim: &mut Sim, events: &[marionette_input::InputEvent]) -> Result<(), Errors> {
    if events.is_empty() {
        return Ok(());
    }
    marionette_input::feed(&mut sim.world, events)
}

fn session_err(what: &str) -> impl Fn(xr::sys::Result) -> XrError + '_ {
    move |e| XrError::new(codes::SESSION, format!("{what}: {e}"))
}

/// Runs the scene on the headset until the runtime ends the session (the player quits from
/// the runtime's menu), `max_frames` or `max_ticks`. Returns the simulation and a report:
/// runtime and headset names, eye size, frames, ticks, frame times.
pub fn run(mut sim: Sim, opts: XrOptions, hooks: &mut dyn XrHooks) -> Result<(Sim, Value), Errors> {
    let base = instance(&opts.app_name)?;
    let (xi, system) = (&base.instance, base.system);
    let props = xi.properties().map_err(session_err("runtime properties"))?;
    let sys = xi
        .system_properties(system)
        .map_err(session_err("headset properties"))?;
    let mut controls = controls::Controls::new(xi)?;
    controls.suggest(xi)?;
    let (gpu, vk) = gpu::create(xi, system)?;
    // SAFETY: the handles are the Vulkan objects the runtime made for this system.
    let (session, mut frame_wait, mut frame_stream) = unsafe {
        xi.create_session::<xr::Vulkan>(
            system,
            &xr::vulkan::SessionCreateInfo {
                instance: vk.instance as _,
                physical_device: vk.physical_device as _,
                device: vk.device as _,
                queue_family_index: vk.queue_family,
                queue_index: 0,
            },
        )
    }
    .map_err(session_err("cannot start a headset session"))?;
    let spaces = controls.attach(&session, base.hand_tracking)?;
    let space_types = session.enumerate_reference_spaces().unwrap_or_default();
    let (space_type, space_name) = if space_types.contains(&xr::ReferenceSpaceType::STAGE) {
        (xr::ReferenceSpaceType::STAGE, "stage")
    } else {
        (xr::ReferenceSpaceType::LOCAL, "local")
    };
    let space = session
        .create_reference_space(space_type, xr::Posef::IDENTITY)
        .map_err(session_err("play space"))?;
    let blend = xi
        .enumerate_environment_blend_modes(system, xr::ViewConfigurationType::PRIMARY_STEREO)
        .ok()
        .and_then(|m| m.first().copied())
        .unwrap_or(xr::EnvironmentBlendMode::OPAQUE);
    let formats = session
        .enumerate_swapchain_formats()
        .map_err(session_err("swapchain formats"))?;
    let format = gpu::pick_format(&formats).ok_or_else(|| {
        XrError::new(
            codes::SESSION,
            "the runtime offers no 8-bit sRGB swapchain format".into(),
        )
    })?;
    let config = xi
        .enumerate_view_configuration_views(system, xr::ViewConfigurationType::PRIMARY_STEREO)
        .map_err(session_err("eye sizes"))?;
    let samples = if opts.sample_count > 1 { 4 } else { 1 };
    let mut eyes = Vec::new();
    for v in &config {
        let scale = opts.resolution_scale.clamp(0.25, 2.0);
        let w = ((v.recommended_image_rect_width as f32 * scale) as u32).clamp(16, v.max_image_rect_width);
        let h = ((v.recommended_image_rect_height as f32 * scale) as u32).clamp(16, v.max_image_rect_height);
        let swapchain = session
            .create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT,
                format: gpu::vk_format(format),
                sample_count: 1,
                width: w,
                height: h,
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })
            .map_err(session_err("swapchain"))?;
        let images = swapchain
            .enumerate_images()
            .map_err(session_err("swapchain images"))?;
        let textures = images
            .iter()
            .map(|&img| gpu::wrap_image(&gpu, img, format, w, h))
            .collect::<Result<Vec<_>, _>>()?;
        let views = textures
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        eyes.push(Eye {
            swapchain,
            textures,
            views,
            width: w,
            height: h,
            msaa: (samples > 1).then(|| attachment(&gpu.device, w, h, format, samples)),
            depth: attachment(&gpu.device, w, h, DEPTH_FORMAT, samples),
        });
    }
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, format, samples, opts.base_dir.clone());
    renderer.set_streaming(true);
    let vignette = vignette::Vignette::new(&gpu.device, format);

    let dt = sim.world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt) as f64;
    let mut accumulator = 0.0f64;
    let mut last_display: Option<i64> = None;
    let mut last_inputs = BTreeMap::new();
    let mut event_buf = xr::EventDataBuffer::new();
    let mut running = false;
    let mut states: Vec<String> = Vec::new();
    let mut frames: u64 = 0;
    let mut rendered: u64 = 0;
    let mut exit_requested = false;
    let mut cpu_ms = Vec::new();
    let mut loop_ms = Vec::new();
    let mut last_wait: Option<Instant> = None;
    let mut max_tracked = 0usize;
    let mut problems: Vec<String> = Vec::new();
    let started = Instant::now();
    let mut result: Result<(), Errors> = Ok(());

    'main: loop {
        while let Some(event) = xi.poll_event(&mut event_buf).map_err(session_err("events"))? {
            match event {
                xr::Event::SessionStateChanged(e) => {
                    states.push(format!("{:?}", e.state()).to_lowercase());
                    match e.state() {
                        xr::SessionState::READY => {
                            session
                                .begin(xr::ViewConfigurationType::PRIMARY_STEREO)
                                .map_err(session_err("begin session"))?;
                            running = true;
                        }
                        xr::SessionState::STOPPING => {
                            session.end().map_err(session_err("end session"))?;
                            running = false;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => break 'main,
                        _ => {}
                    }
                }
                xr::Event::InstanceLossPending(_) => break 'main,
                _ => {}
            }
        }
        if !running {
            if !exit_requested && frames == 0 && started.elapsed() > Duration::from_secs(30) {
                result = Err(
                    XrError::new(codes::NO_HEADSET, "the headset never became ready (30 s)".into())
                        .fix("put the headset on (its proximity sensor wakes it) and keep the runtime open")
                        .into(),
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let done =
            opts.max_frames.is_some_and(|m| frames >= m) || opts.max_ticks.is_some_and(|m| sim.tick() >= m);
        if done && !exit_requested {
            exit_requested = true;
            let _ = session.request_exit();
        }
        // After asking to exit, frames keep going (empty) until the runtime stops the
        // session, as the OpenXR spec requires.

        let fs = frame_wait.wait().map_err(session_err("wait frame"))?;
        let now = Instant::now();
        if let Some(l) = last_wait {
            loop_ms.push((now - l).as_secs_f64() * 1000.0);
        }
        last_wait = Some(now);
        frame_stream.begin().map_err(session_err("begin frame"))?;
        let t = fs.predicted_display_time;

        // Input for the next tick, sampled for when this frame will be seen.
        let reading = controls.read(&session, &spaces, &space, t, sim.tick(), &mut last_inputs);
        max_tracked = max_tracked.max(reading.tracked_sources);
        if let Err(e) = feed(&mut sim, &reading.events) {
            result = Err(e);
            break;
        }
        hooks.hands(&mut sim, &reading.joints);

        // Catch the simulation up with display time, in fixed ticks.
        if let Some(prev) = last_display {
            accumulator = (accumulator + (t.as_nanos() - prev) as f64 * 1e-9).min(dt * 8.0);
        }
        last_display = Some(t.as_nanos());
        while accumulator >= dt && !opts.max_ticks.is_some_and(|m| sim.tick() >= m) {
            sim.step();
            accumulator -= dt;
        }
        hooks.after_ticks(&mut sim);
        if let Some(mut h) = sim.world.get_resource_mut::<XrHaptics>() {
            let pulses = std::mem::take(&mut h.pulses);
            controls.vibrate(&session, &pulses);
        }

        let mut layers_views = Vec::new();
        if fs.should_render && !exit_requested {
            let cpu = Instant::now();
            match render_eyes(
                &mut sim,
                &opts,
                &gpu,
                &mut renderer,
                &vignette,
                &session,
                &space,
                t,
                &mut eyes,
            ) {
                Ok((v, head)) => {
                    layers_views = v;
                    rendered += 1;
                    let (_, r, t) = head.to_scale_rotation_translation();
                    hooks.head(t.to_array(), r.to_array());
                }
                Err(e) => {
                    problems.push(e.0.first().map(|d| d.message.clone()).unwrap_or_default());
                    if problems.len() > 8 {
                        result = Err(e);
                        break;
                    }
                }
            }
            cpu_ms.push(cpu.elapsed().as_secs_f64() * 1000.0);
        }
        if layers_views.len() == eyes.len() && !layers_views.is_empty() {
            let views: Vec<xr::CompositionLayerProjectionView<xr::Vulkan>> = layers_views
                .iter()
                .zip(&eyes)
                .map(|(v, eye)| {
                    xr::CompositionLayerProjectionView::new()
                        .pose(v.pose)
                        .fov(v.fov)
                        .sub_image(
                            xr::SwapchainSubImage::new()
                                .swapchain(&eye.swapchain)
                                .image_array_index(0)
                                .image_rect(xr::Rect2Di {
                                    offset: xr::Offset2Di { x: 0, y: 0 },
                                    extent: xr::Extent2Di {
                                        width: eye.width as i32,
                                        height: eye.height as i32,
                                    },
                                }),
                        )
                })
                .collect();
            let layer = xr::CompositionLayerProjection::new().space(&space).views(&views);
            frame_stream
                .end(t, blend, &[&layer])
                .map_err(session_err("end frame"))?;
        } else {
            frame_stream
                .end(t, blend, &[])
                .map_err(session_err("end frame"))?;
        }
        frames += 1;
    }

    // Tear down in order: GPU work done, wgpu views of runtime images gone, then the
    // swapchains and session, then the GPU (which owns the Vulkan device and instance).
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    for e in &mut eyes {
        e.views.clear();
        e.textures.clear();
    }
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let eye_sizes: Vec<Value> = eyes.iter().map(|e| json!([e.width, e.height])).collect();
    drop(eyes);
    drop(spaces);
    drop(space);
    drop(frame_stream);
    drop(frame_wait);
    drop(session);
    drop(renderer);
    drop(vignette);
    drop(gpu);
    result?;
    let report = json!({
        "runtime": format!("{} {}", props.runtime_name, props.runtime_version),
        "headset": sys.system_name,
        "play_space": space_name,
        "eye_size": eye_sizes,
        "msaa": samples,
        "hand_tracking": base.hand_tracking,
        "refused_controller_profiles": controls.refused_profiles,
        "session_states": states,
        "frames": frames,
        "frames_rendered": rendered,
        "ticks": sim.tick(),
        "tracked_sources_max": max_tracked,
        "render_cpu_ms": stats(&mut cpu_ms),
        "frame_interval_ms": stats(&mut loop_ms),
        "problems": problems,
    });
    Ok((sim, report))
}

#[allow(clippy::too_many_arguments)]
fn render_eyes(
    sim: &mut Sim,
    opts: &XrOptions,
    gpu: &marionette_render::Gpu,
    renderer: &mut Renderer,
    vignette: &vignette::Vignette,
    session: &xr::Session<xr::Vulkan>,
    space: &xr::Space,
    t: xr::Time,
    eyes: &mut [Eye],
) -> Result<(Vec<xr::View>, glam::Mat4), Errors> {
    let (_, views) = session
        .locate_views(xr::ViewConfigurationType::PRIMARY_STEREO, t, space)
        .map_err(session_err("eye poses"))?;
    let frame: FrameData = marionette_render::extract(&sim.world);
    let cam = pick_camera(&frame, opts.camera)?;
    let errors = renderer.prepare(&frame);
    if !errors.is_empty() {
        return Err(Errors(errors));
    }
    let (near, far) = match cam.camera.projection {
        marionette_core::spatial::Projection::Perspective { near, far, .. } => (near, far),
        marionette_core::spatial::Projection::Orthographic { near, far, .. } => (near.max(0.01), far),
    };
    let space_m = play_space(sim, cam.entity);
    let clear = cam.camera.clear_color;
    let mut indices = Vec::new();
    for eye in eyes.iter_mut() {
        let i = eye
            .swapchain
            .acquire_image()
            .map_err(session_err("acquire eye image"))?;
        eye.swapchain
            .wait_image(xr::Duration::INFINITE)
            .map_err(session_err("wait eye image"))?;
        indices.push(i as usize);
    }
    let mut eye_views = Vec::new();
    let mut head = glam::Mat4::IDENTITY;
    for v in views.iter().take(eyes.len()) {
        let p = &v.pose;
        let local = glam::Mat4::from_rotation_translation(
            glam::Quat::from_xyzw(p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w),
            glam::Vec3::new(p.position.x, p.position.y, p.position.z),
        );
        let world = space_m * local;
        head = world;
        eye_views.push(View {
            view: world.inverse(),
            proj: projection(&v.fov, near, far),
            eye: world.w_axis.truncate(),
            clear: [clear.r, clear.g, clear.b, clear.a],
        });
    }
    let targets: Vec<Target> = eyes
        .iter()
        .zip(&indices)
        .map(|(eye, &i)| Target {
            color: eye.msaa.as_ref().unwrap_or(&eye.views[i]),
            resolve: eye.msaa.as_ref().map(|_| &eye.views[i]),
            depth: &eye.depth,
            width: eye.width,
            height: eye.height,
        })
        .collect();
    let pairs: Vec<(View, &Target)> = eye_views.iter().copied().zip(targets.iter()).collect();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render_views(&frame, &pairs, &mut encoder);
    let strength = sim.world.get_resource::<XrComfort>().map_or(0.0, |c| c.vignette);
    for (eye, &i) in eyes.iter().zip(&indices) {
        vignette.draw(
            &gpu.queue,
            &mut encoder,
            &eye.views[i],
            strength,
            eye.width as f32 / eye.height as f32,
        );
    }
    gpu.queue.submit([encoder.finish()]);
    for eye in eyes.iter_mut() {
        eye.swapchain
            .release_image()
            .map_err(session_err("release eye image"))?;
    }
    // The head sits between the eyes.
    if let (Some(a), Some(b)) = (eye_views.first(), eye_views.last()) {
        let mid = (a.eye + b.eye) * 0.5;
        let (_, r, _) = head.to_scale_rotation_translation();
        head = glam::Mat4::from_rotation_translation(r, mid);
    }
    Ok((views, head))
}
