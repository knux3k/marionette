//! Reflection probes (decision 0092): pictures of the scene taken from a point, so shiny
//! surfaces inside a box reflect the objects around them, on every GPU.
//!
//! A probe is captured when it first appears and again when it moves or changes: the renderer
//! draws the frame six times (90-degree faces, linear light, no bloom, sprites or UI) into a
//! floating-point target, reads the faces back, resamples them into an equirectangular image
//! and prefilters that exactly like the environment (`environment.rs`: mip levels blurred by
//! roughness). Each probe is one layer of a texture array (350 KB each, four layers). The
//! shader corrects the reflection direction for the probe's box (parallax correction), so
//! reflections of the walls line up with the walls. Capture blocks for one frame; nothing is
//! captured while a probe stays put.

use crate::environment::{self as env, BASE_W, EnvImage, LEVELS};
use crate::extract::{MAX_PROBES, ProbeData, View};
use glam::{Mat4, Vec3, Vec4};

/// Face directions and up vectors (+x, -x, +y, -y, +z, -z).
const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::NEG_Y),
    (Vec3::NEG_X, Vec3::NEG_Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::Z, Vec3::NEG_Y),
    (Vec3::NEG_Z, Vec3::NEG_Y),
];

/// Probe face size in texels: a multiple of 32 so a row of half floats is 256-byte aligned.
pub(crate) fn face_size(resolution: u32) -> u32 {
    resolution.clamp(32, 512) / 32 * 32
}

/// The six views of a probe at `position`. Each face sees a little more than 90 degrees so the
/// resampling never reads past an edge.
pub(crate) fn face_views(position: Vec3, far: f32) -> Vec<View> {
    let proj = glam::camera::rh::proj::directx::perspective(2.0 * 1.02f32.atan(), 1.0, 0.05, far);
    FACES
        .iter()
        .map(|&(dir, up)| View {
            view: glam::camera::rh::view::look_at_mat4(position, position + dir, up),
            proj,
            eye: position,
            clear: [0.0, 0.0, 0.0, 1.0],
        })
        .collect()
}

/// Half floats to f32.
fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let frac = (h & 0x3ff) as f32;
    match exp {
        0 => sign * frac * 5.960_464_5e-8,
        31 => {
            if frac == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// Resamples six faces (row-major RGBA16F, `size` x `size`, in [`FACES`] order, seen through
/// `views`) into an equirectangular image of the base size, bilinear within each face.
pub(crate) fn faces_to_equirect(faces: &[Vec<u16>], size: u32, views: &[View]) -> EnvImage {
    let (w, h) = (BASE_W, BASE_W / 2);
    let n = size as usize;
    let mats: Vec<Mat4> = views.iter().map(|v| v.proj * v.view).collect();
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            let phi = (u - 0.5) * 2.0 * std::f32::consts::PI;
            let theta = v * std::f32::consts::PI;
            let d = Vec3::new(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos());
            let a = d.abs();
            let face = if a.x >= a.y && a.x >= a.z {
                if d.x > 0.0 { 0 } else { 1 }
            } else if a.y >= a.z {
                if d.y > 0.0 { 2 } else { 3 }
            } else if d.z > 0.0 {
                4
            } else {
                5
            };
            let clip = mats[face] * (views[face].eye + d).extend(1.0);
            let ndc = clip.truncate() / clip.w;
            let fx = ((ndc.x * 0.5 + 0.5) * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
            let fy = ((0.5 - ndc.y * 0.5) * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let at = |xx: usize, yy: usize| {
                let i = (yy * n + xx) * 4;
                let f = &faces[face];
                Vec3::new(f16_to_f32(f[i]), f16_to_f32(f[i + 1]), f16_to_f32(f[i + 2]))
            };
            let c = at(x0, y0)
                .lerp(at(x1, y0), tx)
                .lerp(at(x0, y1).lerp(at(x1, y1), tx), ty);
            px.push(if c.is_finite() {
                c.clamp(Vec3::ZERO, Vec3::splat(env::HDR_MAX))
            } else {
                Vec3::ZERO
            });
        }
    }
    EnvImage { w, h, px }
}

/// The probe texture array, the capture targets and what each layer holds.
pub(crate) struct Probes {
    pub array: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// What each layer was captured from; a probe whose data differs is captured again.
    pub held: [Option<ProbeData>; MAX_PROBES],
    /// Capture targets for one face size: multisampled color and depth (shared by the faces),
    /// six single-sample layers, a readback buffer.
    targets: Option<CaptureTargets>,
}

pub(crate) struct CaptureTargets {
    pub size: u32,
    pub msaa: Option<wgpu::TextureView>,
    pub depth: wgpu::TextureView,
    pub faces: wgpu::Texture,
    pub face_views: Vec<wgpu::TextureView>,
    readback: wgpu::Buffer,
}

impl Probes {
    /// An array of `layers` layers (one per probe; 1 as a stand-in while no scene has probes).
    pub fn new(device: &wgpu::Device, layers: u32) -> Self {
        let array = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection probes"),
            size: wgpu::Extent3d {
                width: if layers > 1 { BASE_W as u32 } else { 1 },
                height: if layers > 1 { BASE_W as u32 / 2 } else { 1 },
                depth_or_array_layers: layers,
            },
            mip_level_count: if layers > 1 { LEVELS as u32 } else { 1 },
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Self {
            array,
            view,
            held: [None; MAX_PROBES],
            targets: None,
        }
    }

    /// True while this is the 1x1 stand-in.
    pub fn is_stand_in(&self) -> bool {
        self.array.width() == 1
    }

    /// Capture targets for faces of `size` texels.
    pub fn targets(&mut self, device: &wgpu::Device, size: u32, sample_count: u32) -> &CaptureTargets {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let tex = |label, samples, format, layers| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: layers,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let hdr = crate::post::HDR_FORMAT;
            let msaa = (sample_count > 1)
                .then(|| tex("probe msaa", sample_count, hdr, 1).create_view(&Default::default()));
            let depth = tex("probe depth", sample_count, crate::renderer::DEPTH_FORMAT, 1)
                .create_view(&Default::default());
            let faces = tex("probe faces", 1, hdr, 6);
            let face_views = (0..6)
                .map(|i| {
                    faces.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: i,
                        array_layer_count: Some(1),
                        ..Default::default()
                    })
                })
                .collect();
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("probe readback"),
                size: 6 * (size * size * 8) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.targets = Some(CaptureTargets {
                size,
                msaa,
                depth,
                faces,
                face_views,
                readback,
            });
        }
        self.targets.as_ref().expect("made above")
    }

    /// Reads the six captured faces back (the frame must be recorded into `encoder`), then
    /// prefilters them into `layer`. Blocks until the GPU is done.
    pub fn finish_capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        layer: usize,
        views: &[View],
    ) -> Result<(), String> {
        let t = self.targets.as_ref().ok_or("no capture targets")?;
        let size = t.size;
        encoder.copy_texture_to_buffer(
            t.faces.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &t.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 8),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = t.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU wait failed: {e}"))?;
        let faces: Vec<Vec<u16>> = {
            let data = slice
                .get_mapped_range()
                .map_err(|e| format!("readback failed: {e}"))?;
            let halves: &[u16] = bytemuck::cast_slice(&data);
            halves
                .chunks_exact((size * size * 4) as usize)
                .map(|f| f.to_vec())
                .collect()
        };
        t.readback.unmap();
        let filtered = env::prefilter(faces_to_equirect(&faces, size, views));
        for (mip, level) in filtered.levels.iter().enumerate() {
            let data: Vec<u16> = level
                .px
                .iter()
                .flat_map(|c| {
                    [
                        env::f16_bits(c.x),
                        env::f16_bits(c.y),
                        env::f16_bits(c.z),
                        env::f16_bits(1.0),
                    ]
                })
                .collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.array,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&data),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8 * level.w as u32),
                    rows_per_image: Some(level.h as u32),
                },
                wgpu::Extent3d {
                    width: level.w as u32,
                    height: level.h as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        Ok(())
    }
}

/// Uniform data per probe: center (w = 1 when the layer holds a picture) and extent.
pub(crate) fn uniform(
    probes: &[ProbeData],
    held: &[Option<ProbeData>],
) -> ([[f32; 4]; MAX_PROBES], [[f32; 4]; MAX_PROBES]) {
    let mut pos = [[0.0; 4]; MAX_PROBES];
    let mut ext = [[0.0; 4]; MAX_PROBES];
    for (i, p) in probes.iter().take(MAX_PROBES).enumerate() {
        let ready = held.get(i).copied().flatten() == Some(*p);
        pos[i] = p.position.extend(if ready { 1.0 } else { 0.0 }).to_array();
        ext[i] = Vec4::from((p.extent, 0.0)).to_array();
    }
    (pos, ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_decode() {
        for v in [0.0f32, 1.0, -2.5, 0.000_1, 1000.0, 60000.0] {
            let back = f16_to_f32(env::f16_bits(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {back}");
        }
    }

    /// Faces colored by direction come out where the equirectangular layout puts them.
    #[test]
    fn faces_land_in_the_right_directions() {
        let size = 32u32;
        let colors = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ];
        let faces: Vec<Vec<u16>> = colors
            .iter()
            .map(|c| {
                (0..size * size)
                    .flat_map(|_| {
                        [
                            env::f16_bits(c[0]),
                            env::f16_bits(c[1]),
                            env::f16_bits(c[2]),
                            env::f16_bits(1.0),
                        ]
                    })
                    .collect()
            })
            .collect();
        let views = face_views(Vec3::new(3.0, 1.0, -2.0), 100.0);
        let img = faces_to_equirect(&faces, size, &views);
        let at = |d: Vec3| {
            // Inverse of the equirectangular mapping in pbr.wgsl (env_uv).
            let u = d.x.atan2(-d.z) / (2.0 * std::f32::consts::PI) + 0.5;
            let v = d.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            let x = ((u * img.w as f32) as usize).min(img.w - 1);
            let y = ((v * img.h as f32) as usize).min(img.h - 1);
            img.px[y * img.w + x]
        };
        for (k, (dir, _)) in FACES.iter().enumerate() {
            let c = at(*dir * 0.999 + Vec3::new(0.001, 0.0005, 0.0007));
            assert!(
                (c - Vec3::from(colors[k])).abs().max_element() < 1e-3,
                "face {k}: {c:?}"
            );
        }
    }
}
