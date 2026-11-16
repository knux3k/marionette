//! Draws camera shots (decision 0286) for whichever loop renders the frame: the window, the
//! headset and the offscreen renderer use the same steps.
//!
//! 1. [`ShotDraws::plan`] takes the requests out of the world's `Shots` resource and makes a
//!    target for each (owned here, so the caller only borrows them).
//! 2. The caller draws its own views and then [`ShotDraws::views`] in one
//!    `Renderer::render_views_ui` call: the shots share the frame's per-frame work and get no
//!    screen UI.
//! 3. [`ShotDraws::finish`] records the copies of read-backs into mappable buffers and points
//!    each camera screen's image at the picture just drawn.
//! 4. After the submit, [`ShotDraws::submitted`] starts mapping the buffers; [`ShotDraws::collect`]
//!    (any later frame) polls the device without waiting and hands finished pictures to the
//!    world. A frame never waits on the GPU for a shot.
//!
//! Ownership: this struct owns every texture and buffer it makes. Camera screens keep two
//! textures each and alternate: a shot writes one while the frame's sprites and UI read the
//! other, so no pass samples the image it draws into.

use crate::extract::View;
use crate::renderer::{DEPTH_FORMAT, Renderer, Target};
use bevy_ecs::world::World;
use marionette_core::shots::{ShotDone, ShotRequest, Shots};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// A buffer's mapping state, set from wgpu's callback (which may run on another thread).
const WAITING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// Color, resolve and depth of one shot this frame.
struct Planned {
    id: u64,
    /// Empty for a read-back; else the camera screen's image name.
    image: String,
    view: View,
    width: u32,
    height: u32,
    /// The single-sample picture (a screen's texture, or a read-back's own).
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
}

/// A read-back on its way from the GPU.
struct Reading {
    id: u64,
    width: u32,
    height: u32,
    padded_row: u32,
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    mapping: bool,
}

/// A camera screen's two pictures; `front` is the one shown.
struct Screen {
    textures: [(wgpu::Texture, wgpu::TextureView); 2],
    /// Kept with the screen, so a screen drawn every few frames makes no new targets.
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    width: u32,
    height: u32,
    front: usize,
    /// Frame counter of its last shot, to drop the least recently used past the limit.
    used: u64,
}

pub struct ShotDraws {
    format: wgpu::TextureFormat,
    samples: u32,
    planned: Vec<Planned>,
    reading: Vec<Reading>,
    screens: BTreeMap<String, Screen>,
    frame: u64,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::Texture {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if samples == 1 && format != DEPTH_FORMAT {
        usage |= wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
    }
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// The view of a shot's pose, as `CameraData::view` makes one for a camera.
pub fn shot_view(r: &ShotRequest, width: u32, height: u32, clear: [f32; 4]) -> View {
    let world = glam::Mat4::from_rotation_translation(
        glam::Quat::from_array(r.rotation).normalize(),
        glam::Vec3::from_array(r.position),
    );
    let near = r.near.max(1e-3);
    let far = r.far.max(near * 2.0);
    let fov = r.fov_y_degrees.clamp(1.0, 170.0).to_radians();
    View {
        view: world.inverse(),
        proj: glam::camera::rh::proj::directx::perspective(fov, width as f32 / height as f32, near, far),
        eye: world.w_axis.truncate(),
        clear,
    }
}

impl ShotDraws {
    /// For a renderer drawing in `format` with `samples` per pixel.
    pub fn new(format: wgpu::TextureFormat, samples: u32) -> Self {
        Self {
            format,
            samples,
            planned: Vec::new(),
            reading: Vec::new(),
            screens: BTreeMap::new(),
            frame: 0,
        }
    }

    /// For the renderer `r`.
    pub fn for_renderer(r: &Renderer) -> Self {
        Self::new(r.color_format, r.sample_count)
    }

    /// Takes this frame's requests from the world (none without a `Shots` resource) and
    /// makes their targets. `clear` is the main camera's background color and `screen` the
    /// size of the frame's own view (for requests of size 0 by 0). Read-backs wait while
    /// `done` plus those in flight would pass `Shots::LIMIT`.
    pub fn plan(&mut self, device: &wgpu::Device, world: &mut World, clear: [f32; 4], screen: (u32, u32)) {
        self.planned.clear();
        self.frame += 1;
        let Some(mut shots) = world.get_resource_mut::<Shots>() else {
            return;
        };
        if shots.requests.is_empty() {
            return;
        }
        let room = Shots::LIMIT.saturating_sub(shots.done.len() + self.reading.len());
        let mut reads = 0;
        let mut kept = Vec::new();
        // At most MAX_VIEWS - 2 shots a frame: the frame's own one or two views come first.
        let max = crate::MAX_VIEWS - 2;
        for r in shots.requests.drain(..) {
            let is_read = r.image.is_empty();
            if self.planned.len() >= max || (is_read && reads >= room) {
                kept.push(r);
                continue;
            }
            reads += usize::from(is_read);
            let planned = self.target(device, r, clear, screen);
            self.planned.push(planned);
        }
        shots.requests = kept;
    }

    fn target(
        &mut self,
        device: &wgpu::Device,
        r: ShotRequest,
        clear: [f32; 4],
        screen: (u32, u32),
    ) -> Planned {
        let (width, height) = Shots::clamp_size(r.width, r.height, screen);
        let view = shot_view(&r, width, height, clear);
        let (samples, format) = (self.samples, self.format);
        let msaa = || {
            (samples > 1).then(|| {
                texture(device, "shot msaa", (width, height), format, samples)
                    .create_view(&Default::default())
            })
        };
        let depth = || {
            texture(device, "shot depth", (width, height), DEPTH_FORMAT, samples)
                .create_view(&Default::default())
        };
        let (color, color_view, msaa, depth) = if r.image.is_empty() {
            let t = texture(device, "shot", (width, height), format, 1);
            let v = t.create_view(&Default::default());
            (t, v, msaa(), depth())
        } else {
            let frame = self.frame;
            let fresh = || {
                let make = || {
                    let t = texture(device, "camera screen", (width, height), self.format, 1);
                    let v = t.create_view(&Default::default());
                    (t, v)
                };
                Screen {
                    textures: [make(), make()],
                    msaa: msaa(),
                    depth: depth(),
                    width,
                    height,
                    front: 0,
                    used: frame,
                }
            };
            let screen = self.screens.entry(r.image.clone()).or_insert_with(fresh);
            if (screen.width, screen.height) != (width, height) {
                *screen = fresh();
            }
            screen.used = frame;
            // The back picture is drawn; the front one stays on show this frame.
            // wgpu handles are reference counted: these clones share the screen's textures.
            let (t, v) = &screen.textures[1 - screen.front];
            (t.clone(), v.clone(), screen.msaa.clone(), screen.depth.clone())
        };
        Planned {
            id: r.id,
            image: r.image,
            view,
            width,
            height,
            color,
            color_view,
            msaa,
            depth,
        }
    }

    /// True when this frame has shots to draw.
    pub fn any(&self) -> bool {
        !self.planned.is_empty()
    }

    /// This frame's shots as views and targets, to follow the frame's own views.
    pub fn views(&self) -> Vec<(View, Target<'_>)> {
        self.planned
            .iter()
            .map(|p| {
                (
                    p.view,
                    Target {
                        color: p.msaa.as_ref().unwrap_or(&p.color_view),
                        resolve: p.msaa.as_ref().map(|_| &p.color_view),
                        depth: &p.depth,
                        width: p.width,
                        height: p.height,
                    },
                )
            })
            .collect()
    }

    /// After the shots were recorded: copies read-backs to buffers in `encoder`, and shows
    /// each camera screen's new picture. Call before submitting `encoder`.
    pub fn finish(
        &mut self,
        device: &wgpu::Device,
        renderer: &mut Renderer,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        for p in self.planned.drain(..) {
            if !p.image.is_empty() {
                if let Some(s) = self.screens.get_mut(&p.image) {
                    s.front = 1 - s.front;
                    let (_, v) = &s.textures[s.front];
                    renderer.set_image_view(&Shots::image_name(&p.image), v, s.width, s.height);
                }
                continue;
            }
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_row = (p.width * 4).div_ceil(align) * align;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shot read-back"),
                size: u64::from(padded_row) * u64::from(p.height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                p.color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row),
                        rows_per_image: Some(p.height),
                    },
                },
                wgpu::Extent3d {
                    width: p.width,
                    height: p.height,
                    depth_or_array_layers: 1,
                },
            );
            self.reading.push(Reading {
                id: p.id,
                width: p.width,
                height: p.height,
                padded_row,
                buffer,
                state: Arc::new(AtomicU8::new(WAITING)),
                mapping: false,
            });
        }
        // Screens nobody asked for lately go past the limit, oldest first.
        while self.screens.len() > Shots::LIMIT {
            let Some(oldest) = self
                .screens
                .iter()
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.screens.remove(&oldest);
        }
    }

    /// After the submit that holds the copies: starts mapping their buffers.
    pub fn submitted(&mut self) {
        for r in self.reading.iter_mut().filter(|r| !r.mapping) {
            r.mapping = true;
            let state = Arc::clone(&r.state);
            r.buffer.slice(..).map_async(wgpu::MapMode::Read, move |res| {
                state.store(if res.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
    }

    /// Read-backs still on their way.
    pub fn in_flight(&self) -> usize {
        self.reading.len()
    }

    /// Polls the device without waiting and moves finished pictures into the world's
    /// `Shots::done` (dropped when the world has no `Shots` any more).
    pub fn collect(&mut self, device: &wgpu::Device, world: &mut World) {
        if self.reading.is_empty() {
            return;
        }
        // A failed poll leaves the shots waiting; a lost device fails them below on its own.
        let _ = device.poll(wgpu::PollType::Poll);
        self.take_ready(world);
    }

    /// Waits for every read-back in flight (tests and one-off renders), then collects.
    pub fn wait(&mut self, device: &wgpu::Device, world: &mut World) {
        if self.reading.is_empty() {
            return;
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        self.take_ready(world);
    }

    fn take_ready(&mut self, world: &mut World) {
        let format = self.format;
        let mut finished = Vec::new();
        self.reading.retain(|r| match r.state.load(Ordering::Acquire) {
            MAPPED => {
                finished.push(read(r, format));
                false
            }
            FAILED => {
                finished.push(ShotDone {
                    id: r.id,
                    width: r.width,
                    height: r.height,
                    rgba: Vec::new(),
                    error: "the picture could not be read back".into(),
                });
                false
            }
            _ => true,
        });
        if let Some(mut shots) = world.get_resource_mut::<Shots>() {
            shots.done.extend(finished);
        }
    }
}

/// The mapped buffer as tight RGBA rows (BGRA formats swapped), then unmapped.
fn read(r: &Reading, format: wgpu::TextureFormat) -> ShotDone {
    let bgra = matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let row = (r.width * 4) as usize;
    let mut rgba = Vec::with_capacity(row * r.height as usize);
    let error = match r.buffer.slice(..).get_mapped_range() {
        Ok(data) => {
            for line in data.chunks_exact(r.padded_row as usize) {
                rgba.extend_from_slice(&line[..row]);
            }
            String::new()
        }
        Err(e) => format!("the picture could not be read back: {e}"),
    };
    r.buffer.unmap();
    if bgra {
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    ShotDone {
        id: r.id,
        width: r.width,
        height: r.height,
        rgba,
        error,
    }
}

/// Draws the shots waiting in `world` on their own and waits for them (tests, one-off and
/// headless renders); the pictures land in the world's `Shots::done`. `frame` is the world's
/// extracted frame; `clear` the background color; `screen` the size that 0 by 0 requests get.
/// Returns how many shots were drawn.
pub fn take_now(
    gpu: &crate::Gpu,
    renderer: &mut Renderer,
    draws: &mut ShotDraws,
    frame: &crate::FrameData,
    world: &mut World,
    (clear, screen): ([f32; 4], (u32, u32)),
) -> Result<usize, marionette_core::Errors> {
    let errors = renderer.prepare(frame);
    if !errors.is_empty() {
        return Err(marionette_core::Errors(errors));
    }
    draws.plan(&gpu.device, world, clear, screen);
    let n = draws.planned.len();
    if n > 0 {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let views = draws.views();
        let pairs: Vec<(View, &Target)> = views.iter().map(|(v, t)| (*v, t)).collect();
        renderer.render_views_ui(frame, &pairs, 0, &mut encoder);
        drop(pairs);
        drop(views);
        draws.finish(&gpu.device, renderer, &mut encoder);
        gpu.queue.submit([encoder.finish()]);
        draws.submitted();
    }
    draws.wait(&gpu.device, world);
    Ok(n)
}
