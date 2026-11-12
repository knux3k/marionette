//! GPU skinning for animated glTF models.
//!
//! Skinned mesh parts are drawn instanced like static ones: one draw per (model, part,
//! material source), whatever the number of characters. Each instance carries the start of
//! its joint palette; all palettes of a frame live in one storage buffer (group 3), built
//! once per frame and shared by every view. A palette entry is the pose's model-space node
//! matrix times the joint's inverse bind matrix, so the vertex shader only blends four
//! matrices per vertex. The fragment stage is the PBR shader's, unchanged.

use crate::extract::FrameData;
use crate::renderer::{GpuModel, GpuPart, InstanceData};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use std::collections::BTreeMap;
use std::num::NonZeroU64;

/// Per-vertex skin attributes (vertex buffer slot 2).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkinVertex {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// Per-instance data of a skinned draw (vertex buffer slot 1): the static instance data plus
/// the first palette entry of this instance.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkinnedInstance {
    pub inst: InstanceData,
    pub joint_base: [u32; 4],
}

/// A skin as the renderer needs it: joint node indices and inverse bind matrices.
pub(crate) struct GpuSkin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

pub(crate) struct SkinnedCall<'a> {
    pub part: &'a GpuPart,
    pub material: &'a wgpu::BindGroup,
    pub instances: std::ops::Range<u32>,
}

pub(crate) struct Skinning {
    pub pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    joints: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    pub instances: Option<wgpu::Buffer>,
}

const MAT_SIZE: u64 = 64;

impl Skinning {
    /// The skinned pipeline: the PBR shader with `vs_skinned` as vertex entry point.
    pub fn new(
        device: &wgpu::Device,
        groups: [&wgpu::BindGroupLayout; 3],
        color_format: wgpu::TextureFormat,
        depth: wgpu::DepthStencilState,
        multisample: wgpu::MultisampleState,
        mesh_stride: u64,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("joint palettes"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(MAT_SIZE),
                },
                count: None,
            }],
        });
        let source = [
            include_str!("shaders/pbr.wgsl"),
            include_str!("shaders/skinned.wgsl"),
        ]
        .join("\n");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr skinned"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr skinned"),
            bind_group_layouts: &[Some(groups[0]), Some(groups[1]), Some(groups[2]), Some(&layout)],
            immediate_size: 0,
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let instance_attrs = wgpu::vertex_attr_array![
            3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
            7 => Float32x4, 8 => Float32x4, 9 => Uint32x4
        ];
        let skin_attrs = wgpu::vertex_attr_array![10 => Uint16x4, 11 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr skinned"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_skinned"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: mesh_stride,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &mesh_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<SkinnedInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<SkinVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &skin_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
        let joints = Self::joint_buffer(device, 64);
        let group = Self::bind(device, &layout, &joints);
        Self {
            pipeline,
            layout,
            joints,
            group,
            instances: None,
        }
    }

    fn joint_buffer(device: &wgpu::Device, matrices: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("joint palettes"),
            size: matrices * MAT_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("joint palettes"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// Uploads this frame's palettes and instances (growing the buffers when needed).
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        palettes: &[[[f32; 4]; 4]],
        instances: &[SkinnedInstance],
    ) {
        if palettes.is_empty() || instances.is_empty() {
            return;
        }
        let needed = palettes.len() as u64 * MAT_SIZE;
        if self.joints.size() < needed {
            self.joints = Self::joint_buffer(device, (palettes.len() as u64).next_power_of_two());
            self.group = Self::bind(device, &self.layout, &self.joints);
        }
        queue.write_buffer(&self.joints, 0, bytemuck::cast_slice(palettes));
        crate::renderer::write_growable(
            device,
            queue,
            &mut self.instances,
            instances,
            wgpu::BufferUsages::VERTEX,
        );
    }
}

/// Builds this frame's joint palettes, skinned instances (grouped by model, part and
/// material source, in key order, entities in id order within a group) and draw calls.
pub(crate) fn build<'a>(
    frame: &FrameData,
    models: &'a BTreeMap<String, Result<GpuModel, String>>,
    plain_material: &'a wgpu::BindGroup,
) -> (Vec<[[f32; 4]; 4]>, Vec<SkinnedInstance>, Vec<SkinnedCall<'a>>) {
    let mut palettes: Vec<[[f32; 4]; 4]> = Vec::new();
    let mut groups: BTreeMap<(&str, usize, bool), Vec<SkinnedInstance>> = BTreeMap::new();
    for draw in &frame.meshes {
        let Some(Ok(model)) = models.get(&*draw.asset) else {
            continue;
        };
        if model.skins.is_empty() {
            continue;
        }
        // One palette per skin this draw uses, in skin order.
        let mut base_of: Vec<Option<u32>> = vec![None; model.skins.len()];
        for (p, part) in model.parts.iter().enumerate() {
            let Some((s, _)) = &part.skin else {
                continue;
            };
            let base = match base_of[*s] {
                Some(b) => b,
                None => {
                    let b = palettes.len() as u32;
                    let skin = &model.skins[*s];
                    for (j, &node) in skin.joints.iter().enumerate() {
                        let m = match &draw.pose {
                            Some(pose) => pose.nodes.get(node).map(Mat4::from_cols_array),
                            None => None,
                        }
                        .or_else(|| model.rest.get(node).copied())
                        .unwrap_or(Mat4::IDENTITY);
                        palettes.push((m * skin.inverse_bind[j]).to_cols_array_2d());
                    }
                    base_of[*s] = Some(b);
                    b
                }
            };
            let (base_color, params) = crate::renderer::instance_material(model, part, draw.material);
            groups
                .entry((&*draw.asset, p, draw.material.is_some()))
                .or_default()
                .push(SkinnedInstance {
                    inst: InstanceData {
                        model: draw.world.to_cols_array_2d(),
                        base_color,
                        params,
                    },
                    joint_base: [base, 0, 0, 0],
                });
        }
    }
    let mut instances = Vec::new();
    let mut calls = Vec::with_capacity(groups.len());
    for ((asset, p, overridden), list) in groups {
        let Some(Ok(model)) = models.get(asset) else {
            continue;
        };
        let part = &model.parts[p];
        let material = if overridden {
            plain_material
        } else {
            &model.materials[part.material.min(model.materials.len() - 1)].0
        };
        let start = instances.len() as u32;
        instances.extend(list);
        calls.push(SkinnedCall {
            part,
            material,
            instances: start..instances.len() as u32,
        });
    }
    (palettes, instances, calls)
}
