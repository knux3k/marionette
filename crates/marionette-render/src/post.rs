//! Bloom (decision 0064): with the `Bloom` resource, models are drawn into a floating-point
//! (HDR) target instead of the final one. The light above a threshold is blurred through a
//! chain of half-size images and added back, then a composite pass tone maps the sum into
//! the final target, where sprites and UI are drawn as usual (no glow, no tone mapping).
//!
//! The targets are shared by every view of a frame (the views are drawn one after the
//! other in one encoder) and kept per size, so stereo eyes of one size share one set.

use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;
use std::num::NonZeroU64;

/// Format of the HDR target and the glow chain.
pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Most glow levels (half, quarter, ... size).
const MAX_LEVELS: usize = 6;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniform {
    /// threshold, intensity / levels, exposure, tone map (1) or clamp (0)
    params: [f32; 4],
}

/// Textures for one target size.
pub(crate) struct PostTargets {
    /// Multisampled HDR color when the renderer multisamples.
    pub msaa: Option<wgpu::TextureView>,
    /// Single-sample HDR color (the resolve target when multisampling).
    pub hdr: wgpu::TextureView,
    /// Glow levels, largest first.
    levels: Vec<wgpu::TextureView>,
    /// Bind group reading the HDR image (prefilter), then each level (down and up).
    read_hdr: wgpu::BindGroup,
    read_level: Vec<wgpu::BindGroup>,
    composite: wgpu::BindGroup,
    used: bool,
}

pub(crate) struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// Stand-in for the unused glow binding of the chain passes.
    dummy: wgpu::TextureView,
    sample_count: u32,
    targets: BTreeMap<(u32, u32), PostTargets>,
}

fn texture(device: &wgpu::Device, label: &str, size: (u32, u32), samples: u32, mips: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Post {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        depth: wgpu::DepthStencilState,
    ) -> Self {
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(size_of::<PostUniform>() as u64),
                    },
                    count: None,
                },
                tex(3),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, entry, format, blend, depth: Option<wgpu::DepthStencilState>, samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        // The composite runs in the final pass, which also holds the scene depth (kept for
        // sprites); it neither tests nor writes it.
        let composite_depth = wgpu::DepthStencilState {
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            ..depth
        };
        Self {
            prefilter: pipeline("bloom prefilter", "fs_prefilter", HDR_FORMAT, None, None, 1),
            down: pipeline("bloom down", "fs_down", HDR_FORMAT, None, None, 1),
            up: pipeline("bloom up", "fs_up", HDR_FORMAT, Some(add), None, 1),
            composite: pipeline(
                "bloom composite",
                "fs_composite",
                color_format,
                None,
                Some(composite_depth),
                sample_count,
            ),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post"),
                size: size_of::<PostUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            dummy: texture(device, "post dummy", (1, 1), 1, 1).create_view(&Default::default()),
            layout,
            sample_count,
            targets: BTreeMap::new(),
        }
    }

    /// Glow levels for an image of `size`: halving until the smaller side drops below 8.
    fn level_sizes(size: (u32, u32)) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let (mut w, mut h) = ((size.0 / 2).max(1), (size.1 / 2).max(1));
        while out.len() < MAX_LEVELS {
            out.push((w, h));
            if w.min(h) < 16 {
                break;
            }
            (w, h) = (w / 2, h / 2);
        }
        out
    }

    /// Makes sure targets exist for every size in `sizes`; frees sizes not asked for.
    pub fn prepare(&mut self, device: &wgpu::Device, sizes: &[(u32, u32)]) {
        for t in self.targets.values_mut() {
            t.used = false;
        }
        for &size in sizes {
            if let Some(t) = self.targets.get_mut(&size) {
                t.used = true;
                continue;
            }
            let t = self.build(device, size);
            self.targets.insert(size, t);
        }
        self.targets.retain(|_, t| t.used);
    }

    fn build(&self, device: &wgpu::Device, size: (u32, u32)) -> PostTargets {
        let hdr = texture(device, "hdr", size, 1, 1).create_view(&Default::default());
        let msaa = (self.sample_count > 1).then(|| {
            texture(device, "hdr msaa", size, self.sample_count, 1).create_view(&Default::default())
        });
        let levels: Vec<wgpu::TextureView> = Self::level_sizes(size)
            .into_iter()
            .map(|s| texture(device, "glow", s, 1, 1).create_view(&Default::default()))
            .collect();
        let group = |src: &wgpu::TextureView, glow: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(glow),
                    },
                ],
            })
        };
        PostTargets {
            read_hdr: group(&hdr, &self.dummy),
            read_level: levels.iter().map(|l| group(l, &self.dummy)).collect(),
            composite: group(&hdr, &levels[0]),
            msaa,
            hdr,
            levels,
            used: true,
        }
    }

    pub fn targets(&self, size: (u32, u32)) -> &PostTargets {
        &self.targets[&size]
    }

    /// Writes this frame's settings. `exposure` is `Some` with tone mapping on.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        threshold: f32,
        intensity: f32,
        exposure: Option<f32>,
        size: (u32, u32),
    ) {
        let levels = Self::level_sizes(size).len() as f32;
        let u = PostUniform {
            params: [
                threshold.max(0.0),
                intensity.max(0.0) / levels,
                exposure.unwrap_or(1.0),
                if exposure.is_some() { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    /// Records the glow chain for `t` (after the HDR pass, before the composite). Returns
    /// the passes recorded.
    pub fn glow(&self, t: &PostTargets, encoder: &mut wgpu::CommandEncoder) -> u32 {
        let pass = |encoder: &mut wgpu::CommandEncoder,
                    pipeline: &wgpu::RenderPipeline,
                    group: &wgpu::BindGroup,
                    target: &wgpu::TextureView,
                    load: wgpu::LoadOp<wgpu::Color>| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        pass(encoder, &self.prefilter, &t.read_hdr, &t.levels[0], clear);
        for i in 1..t.levels.len() {
            pass(encoder, &self.down, &t.read_level[i - 1], &t.levels[i], clear);
        }
        for i in (1..t.levels.len()).rev() {
            pass(
                encoder,
                &self.up,
                &t.read_level[i],
                &t.levels[i - 1],
                wgpu::LoadOp::Load,
            );
        }
        (2 * t.levels.len() - 1) as u32
    }

    /// Draws the glow plus the HDR image, tone mapped, into the current (final) pass.
    pub fn composite(&self, t: &PostTargets, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &t.composite, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Inverse of the tone curve in `pbr.wgsl` (Narkowicz's ACES fit), per channel: the linear
/// value that maps to `y`. Used so the clear color and fog color look the same with bloom
/// (tone mapped after blending) as without it (tone mapped before).
pub(crate) fn untonemap(y: f32) -> f32 {
    let y = y.clamp(0.0, 0.99);
    let (a, b, c) = (2.43 * y - 2.51, 0.59 * y - 0.03, 0.14 * y);
    let disc = (b * b - 4.0 * a * c).max(0.0);
    ((-b - disc.sqrt()) / (2.0 * a)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tonemap(x: f32) -> f32 {
        ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
    }

    #[test]
    fn untonemap_inverts_the_tone_curve() {
        for i in 0..=98 {
            let y = i as f32 / 100.0;
            assert!((tonemap(untonemap(y)) - y).abs() < 1e-4, "{y}");
        }
    }

    #[test]
    fn glow_levels_halve_down_to_a_small_image() {
        let l = Post::level_sizes((1920, 1080));
        assert_eq!(l.len(), MAX_LEVELS);
        assert_eq!(l[0], (960, 540));
        assert_eq!(Post::level_sizes((64, 32)), vec![(32, 16), (16, 8)]);
    }
}
