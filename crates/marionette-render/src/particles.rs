//! Particles (decision 0272): one uniform per emitter, one instanced draw of six vertices per
//! particle slot; the vertex shader (`shaders/particles.wgsl`) computes every particle from
//! game time. The CPU cost is one small uniform per emitter, whatever the particle count.

use crate::extract::ParticleDraw;
use bytemuck::{Pod, Zeroable};
use marionette_core::spatial::{ParticleBlend, ParticleEmitter};
use std::num::NonZeroU64;
use std::sync::OnceLock;

/// Bytes per emitter in the uniform buffer (the dynamic offset alignment wgpu guarantees).
const STRIDE: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EmitterUniform {
    model: [[f32; 4]; 4],
    start_color: [f32; 4],
    end_color: [f32; 4],
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [u32; 4],
}

/// Which particle slots one emitter draws this frame, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Number of the oldest particle that may still be alive.
    pub first: u64,
    /// Slots drawn (particles `first .. first + count`); ones not alive are skipped on the GPU.
    pub count: u32,
    /// Age in seconds of particle `first`.
    pub age_first: f64,
}

/// The particles of `e` alive at game time `time` (seconds). Computed in double precision,
/// so hours of game time keep particle ages exact to well under a frame.
pub fn span(e: &ParticleEmitter, time: f64) -> Option<Span> {
    let rate = e.rate as f64;
    let life = e.lifetime as f64;
    if !(rate > 0.0 && life > 0.0 && time >= 0.0) || !rate.is_finite() || !life.is_finite() {
        return None;
    }
    // Particle n starts at n / rate; the newest has n = floor(time * rate).
    let newest = (time * rate).floor();
    let oldest = ((time - life) * rate).floor() + 1.0;
    let first = oldest.max(0.0);
    let count = (newest - first + 1.0).clamp(0.0, ParticleEmitter::MAX as f64) as u32;
    // With more than MAX alive, the newest MAX are drawn.
    let first = newest + 1.0 - count as f64;
    (count > 0).then(|| Span {
        first: first as u64,
        count,
        age_first: time - first / rate,
    })
}

type Make = Box<dyn Fn(usize, usize) -> wgpu::RenderPipeline + Send + Sync>;

/// GPU side of particles: emitter uniforms and pipelines.
pub struct Particles {
    layout: wgpu::BindGroupLayout,
    buffer: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    /// Per blend (additive, alpha) and target (final, floating-point): built on first use.
    pipelines: [[OnceLock<wgpu::RenderPipeline>; 2]; 2],
    make: Make,
    /// This frame's draws: uniform offset, particle slots, image, blend.
    draws: Vec<(u32, u32, String, ParticleBlend)>,
}

impl Particles {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        multisample: wgpu::MultisampleState,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle emitter"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(size_of::<EmitterUniform>() as u64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles"),
            bind_group_layouts: &[Some(view_layout), Some(&layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/particles.wgsl").into()),
        });
        let dev = device.clone();
        let formats = [color_format, crate::post::HDR_FORMAT];
        let make: Make = Box::new(move |blend, target| {
            let add = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            };
            let over = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            };
            let keep = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            };
            let state = if blend == 0 {
                wgpu::BlendState {
                    color: add,
                    alpha: keep,
                }
            } else {
                wgpu::BlendState {
                    color: over,
                    alpha: over,
                }
            };
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("particles"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if target == 0 { "fs_main" } else { "fs_hdr" }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: formats[target],
                        blend: Some(state),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        Self {
            layout,
            buffer: None,
            pipelines: Default::default(),
            make,
            draws: Vec::new(),
        }
    }

    /// Writes this frame's emitter uniforms. `exposure` tone maps the final target's output.
    pub(crate) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        particles: &[ParticleDraw],
        time: f64,
        exposure: Option<f32>,
    ) {
        self.draws.clear();
        let mut bytes: Vec<u8> = Vec::new();
        for p in particles {
            let e = &p.emitter;
            let Some(s) = span(e, time) else {
                continue;
            };
            let color = |c: marionette_core::spatial::Color| [c.r, c.g, c.b, c.a];
            let u = EmitterUniform {
                model: p.world.to_cols_array_2d(),
                start_color: color(e.start_color),
                end_color: color(e.end_color),
                a: [
                    e.rate,
                    e.lifetime,
                    e.speed,
                    e.spread_degrees.clamp(0.0, 180.0).to_radians().cos(),
                ],
                b: [
                    e.gravity,
                    e.start_size.max(0.0),
                    e.end_size.max(0.0),
                    e.radius.max(0.0),
                ],
                c: [s.age_first as f32, exposure.unwrap_or(0.0).max(0.0), 0.0, 0.0],
                d: [e.seed, s.first as u32, 0, 0],
            };
            let offset = bytes.len() as u32;
            bytes.extend_from_slice(bytemuck::bytes_of(&u));
            bytes.resize(bytes.len().next_multiple_of(STRIDE as usize), 0);
            self.draws.push((offset, s.count, e.image.clone(), e.blend));
        }
        if bytes.is_empty() {
            return;
        }
        let needed = (bytes.len() as u64).next_power_of_two().max(STRIDE * 4);
        if self
            .buffer
            .as_ref()
            .is_none_or(|(b, _)| b.size() < bytes.len() as u64)
        {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle emitters"),
                size: needed,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle emitters"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: NonZeroU64::new(size_of::<EmitterUniform>() as u64),
                    }),
                }],
            });
            self.buffer = Some((buffer, group));
        }
        if let Some((buffer, _)) = &self.buffer {
            queue.write_buffer(buffer, 0, &bytes);
        }
    }

    /// Images this frame's emitters use.
    pub(crate) fn images(particles: &[ParticleDraw]) -> impl Iterator<Item = &str> {
        particles.iter().map(|p| p.emitter.image.as_str())
    }

    /// Draws every emitter (after see-through models); returns the draw calls made.
    pub(crate) fn draw<'t>(
        &self,
        pass: &mut wgpu::RenderPass,
        view_group: &wgpu::BindGroup,
        view_offset: u32,
        hdr: bool,
        texture: impl Fn(&str) -> Option<&'t wgpu::BindGroup>,
    ) -> u32 {
        let Some((_, group)) = &self.buffer else {
            return 0;
        };
        let mut calls = 0;
        let mut current: *const wgpu::RenderPipeline = std::ptr::null();
        for (offset, count, image, blend) in &self.draws {
            let Some(tex) = texture(image) else {
                continue;
            };
            let b = usize::from(*blend == ParticleBlend::Alpha);
            let t = usize::from(hdr);
            let pipeline = self.pipelines[b][t].get_or_init(|| (self.make)(b, t));
            if !std::ptr::eq(current, pipeline) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, view_group, &[view_offset]);
                current = pipeline;
            }
            pass.set_bind_group(1, group, &[*offset]);
            pass.set_bind_group(2, tex, &[]);
            pass.draw(0..6, 0..*count);
            calls += 1;
        }
        calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use marionette_core::spatial::Color;

    fn emitter(rate: f32, lifetime: f32) -> ParticleEmitter {
        ParticleEmitter {
            rate,
            lifetime,
            speed: 1.0,
            spread_degrees: 20.0,
            radius: 0.0,
            gravity: 0.0,
            start_size: 0.1,
            end_size: 0.1,
            start_color: Color::default(),
            end_color: Color::default(),
            image: "builtin:circle".into(),
            blend: ParticleBlend::Additive,
            seed: 1,
        }
    }

    #[test]
    fn span_covers_exactly_the_living_particles() {
        // 10 per second for 2 s: at t = 5.05 particles 31..=50 live (30 died at 5.0).
        let s = span(&emitter(10.0, 2.0), 5.05).unwrap();
        assert_eq!((s.first, s.count), (31, 20));
        assert!((s.age_first - (5.05 - 3.1)).abs() < 1e-9);
        // Before the first second fills up, only the ones born so far.
        let s = span(&emitter(10.0, 2.0), 0.35).unwrap();
        assert_eq!((s.first, s.count), (0, 4));
        assert!(span(&emitter(0.0, 2.0), 1.0).is_none());
        assert!(span(&emitter(10.0, 0.0), 1.0).is_none());
    }

    #[test]
    fn span_stays_exact_after_hours_and_is_capped() {
        let s = span(&emitter(1000.0, 1.0), 36_000.000_5).unwrap();
        assert_eq!(s.count, 1000);
        assert_eq!(s.first, 35_999_001);
        assert!(s.age_first > 0.999 && s.age_first < 1.0);
        let s = span(&emitter(1.0e6, 1.0), 10.0).unwrap();
        assert_eq!(s.count, ParticleEmitter::MAX);
    }
}
