//! Render scale, upscaling and foveated rendering (decisions 0081, 0082).
//!
//! With `RenderScale` or `Foveation` in a scene, each view's world (models, sky, sprites,
//! bloom) is drawn into an internal image instead of the output; a resolve pass then writes
//! the output from it (`shaders/scale.wgsl`) and UI is drawn on top at full resolution.
//! Internal images are kept per size and shared by every view of that size, like the bloom
//! targets (views are recorded one after the other in one encoder).

use crate::renderer::{DEPTH_FORMAT, Target};
use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;
use std::num::NonZeroU64;

/// Smallest and largest render scale.
pub const MIN_SCALE: f32 = 0.5;
pub const MAX_SCALE: f32 = 2.0;

/// The frame's settings, from the `RenderScale` and `Foveation` resources.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleSettings {
    /// Internal size over output size (per axis), `MIN_SCALE..=MAX_SCALE`.
    pub scale: f32,
    /// Sharpening when upscaling, 0..1.
    pub sharpness: f32,
    /// Foveation inner radius (share of the half width and height; outside it every other
    /// 2x2 pixel quad is skipped); 0 = off.
    pub foveation: f32,
}

impl ScaleSettings {
    /// Settings that need no internal image are `None`.
    pub fn active(self) -> Option<Self> {
        let s = Self {
            scale: self.scale.clamp(MIN_SCALE, MAX_SCALE),
            sharpness: self.sharpness.clamp(0.0, 1.0),
            foveation: self.foveation.clamp(0.0, 2.0),
        };
        ((s.scale - 1.0).abs() > 1e-3 || s.foveation > 0.0).then_some(s)
    }

    /// Internal image size for an output of `size`.
    pub fn internal(&self, size: (u32, u32)) -> (u32, u32) {
        let f = |v: u32| ((v as f32 * self.scale).round() as u32).max(1);
        (f(size.0), f(size.1))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScaleUniform {
    params: [f32; 4],
    dims: [f32; 4],
}

/// One internal image and what reads it.
pub(crate) struct ScaleTargets {
    msaa: Option<wgpu::TextureView>,
    resolve: wgpu::TextureView,
    depth: wgpu::TextureView,
    size: (u32, u32),
    uniform: wgpu::Buffer,
    read: wgpu::BindGroup,
    mask: wgpu::BindGroup,
    used: bool,
}

impl ScaleTargets {
    /// The internal image as a render target.
    pub fn target(&self) -> Target<'_> {
        Target {
            color: self.msaa.as_ref().unwrap_or(&self.resolve),
            resolve: self.msaa.as_ref().map(|_| &self.resolve),
            depth: &self.depth,
            width: self.size.0,
            height: self.size.1,
        }
    }
}

pub(crate) struct Scale {
    color_format: wgpu::TextureFormat,
    sample_count: u32,
    read_layout: wgpu::BindGroupLayout,
    mask_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Output pass: internal image to the final target (format and samples of the renderer).
    resolve: wgpu::RenderPipeline,
    /// Foveation mask into the final-format and the HDR (bloom) internal pass.
    mask: [wgpu::RenderPipeline; 2],
    targets: BTreeMap<(u32, u32), ScaleTargets>,
}

impl Scale {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, sample_count: u32) -> Self {
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(size_of::<ScaleUniform>() as u64),
            },
            count: None,
        };
        let read_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scale read"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scale mask"),
            entries: &[uniform_entry],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scale"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scale.wgsl").into()),
        });
        let multisample = wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let read_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scale read"),
            bind_group_layouts: &[Some(&read_layout)],
            immediate_size: 0,
        });
        let mask_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scale mask"),
            bind_group_layouts: &[Some(&mask_layout)],
            immediate_size: 0,
        });
        let vertex = wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        };
        // The output pass writes the single-sampled final image, with no depth.
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scale resolve"),
            layout: Some(&read_pl),
            vertex: vertex.clone(),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_resolve"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // The mask writes the nearest depth where pixels are skipped and no color.
        let mask = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("foveation mask"),
                layout: Some(&mask_pl),
                vertex: vertex.clone(),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_mask"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::empty(),
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            color_format,
            sample_count,
            mask: [mask(color_format), mask(crate::post::HDR_FORMAT)],
            read_layout,
            mask_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("scale"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            }),
            resolve,
            targets: BTreeMap::new(),
        }
    }

    /// Makes internal images for the output sizes of this frame (none frees them all) and
    /// writes their settings.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: Option<ScaleSettings>,
        outputs: &[(u32, u32)],
    ) {
        for t in self.targets.values_mut() {
            t.used = false;
        }
        if let Some(s) = settings {
            for &out in outputs {
                let size = s.internal(out);
                if !self.targets.contains_key(&size) {
                    let t = self.build(device, size);
                    self.targets.insert(size, t);
                }
                let t = self.targets.get_mut(&size).expect("just inserted");
                t.used = true;
                let ratio = out.0 as f32 / size.0 as f32;
                let mode = if size == out {
                    0.0
                } else if ratio > 1.0 {
                    1.0
                } else {
                    1.0 + (1.0 / ratio).clamp(1.0, 2.0)
                };
                let u = ScaleUniform {
                    params: [s.foveation, s.sharpness, mode, 0.0],
                    dims: [size.0 as f32, size.1 as f32, 0.0, 0.0],
                };
                queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&u));
            }
        }
        self.targets.retain(|_, t| t.used);
    }

    fn build(&self, device: &wgpu::Device, size: (u32, u32)) -> ScaleTargets {
        let texture = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
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
                .create_view(&Default::default())
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let resolve = texture(
            "internal color",
            self.color_format,
            1,
            attach | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let msaa = (self.sample_count > 1).then(|| {
            texture(
                "internal color msaa",
                self.color_format,
                self.sample_count,
                attach,
            )
        });
        let depth = texture("internal depth", DEPTH_FORMAT, self.sample_count, attach);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scale"),
            size: size_of::<ScaleUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scale read"),
            layout: &self.read_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&resolve),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mask = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scale mask"),
            layout: &self.mask_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            }],
        });
        ScaleTargets {
            msaa,
            resolve,
            depth,
            size,
            uniform,
            read,
            mask,
            used: true,
        }
    }

    pub fn targets(&self, size: (u32, u32)) -> &ScaleTargets {
        &self.targets[&size]
    }

    /// Records the foveation mask into the current internal pass (before anything is
    /// drawn). `hdr` picks the bloom variant.
    pub fn mask(&self, t: &ScaleTargets, pass: &mut wgpu::RenderPass, hdr: bool) {
        pass.set_pipeline(&self.mask[hdr as usize]);
        pass.set_bind_group(0, &t.mask, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draws the internal image into the current (output) pass.
    pub fn resolve(&self, t: &ScaleTargets, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.resolve);
        pass.set_bind_group(0, &t.read, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_settings_need_no_internal_image() {
        let s = ScaleSettings {
            scale: 1.0,
            sharpness: 0.3,
            foveation: 0.0,
        };
        assert_eq!(s.active(), None);
        let up = ScaleSettings { scale: 0.75, ..s }.active().unwrap();
        assert_eq!(up.internal((2160, 2160)), (1620, 1620));
        let clamped = ScaleSettings { scale: 0.1, ..s }.active().unwrap();
        assert_eq!(clamped.scale, MIN_SCALE);
        assert!(ScaleSettings { foveation: 0.6, ..s }.active().is_some());
    }
}
