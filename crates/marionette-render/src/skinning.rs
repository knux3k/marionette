//! GPU skinning and morph targets (blend shapes) for animated glTF models.
//!
//! Deformed mesh parts (skinned, or with morph targets) are drawn instanced like static
//! ones: one draw per (model, part, material source), whatever the number of characters.
//! Each instance carries where its joint palette and its morph weights start; all palettes
//! and weights of a frame live in two storage buffers (group 3), built once per frame and
//! shared by every view. A palette entry is the pose's model-space node matrix times the
//! joint's inverse bind matrix, so the vertex shader only blends four matrices per vertex.
//! Morph target deltas live in one storage buffer per part. A part with morph targets but no
//! skin gets a one-entry palette (its node's matrix). The fragment stage is the PBR
//! shader's, unchanged.

use crate::assets::Part;
use crate::extract::FrameData;
use crate::renderer::{GpuModel, GpuPart, InstanceData};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

/// Per-vertex skin attributes (vertex buffer slot 2).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkinVertex {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// Per-instance data of a deformed draw (vertex buffer slot 1): the static instance data
/// plus (first palette entry, first morph weight, morph target count, vertex count).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkinnedInstance {
    pub inst: InstanceData,
    pub deform: [u32; 4],
}

/// A skin as the renderer needs it: joint node indices and inverse bind matrices.
pub(crate) struct GpuSkin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

/// GPU data of a part the vertex shader deforms.
pub(crate) struct Deform {
    /// Skin index in the model, or `None` for a morph-only part.
    pub skin: Option<usize>,
    pub skin_vertices: wgpu::Buffer,
    /// Morph deltas (position then normal, per vertex, per target) and the target count.
    pub morph: Option<(wgpu::Buffer, u32)>,
    pub vertex_count: u32,
}

impl Deform {
    /// Uploads skin and morph data of `part`; `None` for a part drawn by the static path.
    pub fn upload(device: &wgpu::Device, part: &Part, skins: usize) -> Option<Self> {
        let n = part.mesh.positions.len();
        let skin = part.skin.filter(|&s| s < skins && part.mesh.joints.len() == n);
        let targets: Vec<&crate::assets::MorphTarget> = part
            .mesh
            .morph_targets
            .iter()
            .filter(|t| t.positions.len() == n)
            .collect();
        if skin.is_none() && targets.is_empty() {
            return None;
        }
        let skin_vertices: Vec<SkinVertex> = (0..n)
            .map(|i| match skin {
                Some(_) => SkinVertex {
                    joints: part.mesh.joints[i],
                    weights: part.mesh.weights[i],
                },
                None => SkinVertex {
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                },
            })
            .collect();
        let skin_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skin vertices"),
            contents: bytemuck::cast_slice(&skin_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let morph = (!targets.is_empty()).then(|| {
            let mut deltas: Vec<[f32; 4]> = Vec::with_capacity(targets.len() * n * 2);
            for t in &targets {
                for v in 0..n {
                    let p = t.positions[v];
                    let q = t.normals.get(v).copied().unwrap_or([0.0; 3]);
                    deltas.push([p[0], p[1], p[2], 0.0]);
                    deltas.push([q[0], q[1], q[2], 0.0]);
                }
            }
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("morph deltas"),
                contents: bytemuck::cast_slice(&deltas),
                usage: wgpu::BufferUsages::STORAGE,
            });
            (buffer, targets.len() as u32)
        });
        Some(Self {
            skin,
            skin_vertices,
            morph,
            vertex_count: n as u32,
        })
    }
}

pub(crate) struct SkinnedCall<'a> {
    pub part: &'a GpuPart,
    pub material: &'a wgpu::BindGroup,
    pub instances: std::ops::Range<u32>,
}

pub(crate) struct Skinning {
    pub pipeline: wgpu::RenderPipeline,
    /// Depth-only pass from the shadow-casting light (decision 0063).
    pub shadow_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    palettes: wgpu::Buffer,
    weights: wgpu::Buffer,
    /// Stand-in delta buffer for parts without morph targets.
    no_deltas: wgpu::Buffer,
    /// Bind group for parts without morph targets (rebuilt when the frame buffers grow).
    plain: wgpu::BindGroup,
    pub instances: Option<wgpu::Buffer>,
}

const MAT_SIZE: u64 = 64;

fn storage(binding: u32, min: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(min),
        },
        count: None,
    }
}

fn storage_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Skinning {
    /// The deforming pipeline: the PBR shader with `vs_skinned` as vertex entry point.
    pub fn new(
        device: &wgpu::Device,
        groups: [&wgpu::BindGroupLayout; 3],
        color_format: wgpu::TextureFormat,
        depth: wgpu::DepthStencilState,
        multisample: wgpu::MultisampleState,
        mesh_stride: u64,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("deform"),
            entries: &[storage(0, MAT_SIZE), storage(1, 4), storage(2, 16)],
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
            7 => Float32x4, 8 => Float32x4, 12 => Float32x4, 9 => Uint32x4
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
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr skinned shadow"),
            bind_group_layouts: &[Some(groups[0]), None, None, Some(&layout)],
            immediate_size: 0,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr skinned shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_skinned_shadow"),
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
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(crate::renderer::shadow_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let palettes = storage_buffer(device, "joint palettes", 64 * MAT_SIZE);
        let weights = storage_buffer(device, "morph weights", 256);
        let no_deltas = storage_buffer(device, "no morph deltas", 16);
        let plain = Self::bind(device, &layout, &palettes, &weights, &no_deltas);
        Self {
            pipeline,
            shadow_pipeline,
            layout,
            palettes,
            weights,
            no_deltas,
            plain,
            instances: None,
        }
    }

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        palettes: &wgpu::Buffer,
        weights: &wgpu::Buffer,
        deltas: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("deform"),
            layout,
            entries: &[entry(0, palettes), entry(1, weights), entry(2, deltas)],
        })
    }

    /// Uploads this frame's palettes, morph weights and instances (growing buffers when
    /// needed) and returns the group-3 bind group of each call.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        palettes: &[[[f32; 4]; 4]],
        weights: &[f32],
        instances: &[SkinnedInstance],
        calls: &[SkinnedCall],
    ) -> Vec<wgpu::BindGroup> {
        if instances.is_empty() {
            return Vec::new();
        }
        let mut grown = false;
        let p_bytes = palettes.len() as u64 * MAT_SIZE;
        if self.palettes.size() < p_bytes {
            self.palettes = storage_buffer(device, "joint palettes", p_bytes.next_power_of_two());
            grown = true;
        }
        let w_bytes = weights.len() as u64 * 4;
        if self.weights.size() < w_bytes {
            self.weights = storage_buffer(device, "morph weights", w_bytes.next_power_of_two());
            grown = true;
        }
        if grown {
            self.plain = Self::bind(
                device,
                &self.layout,
                &self.palettes,
                &self.weights,
                &self.no_deltas,
            );
        }
        if !palettes.is_empty() {
            queue.write_buffer(&self.palettes, 0, bytemuck::cast_slice(palettes));
        }
        if !weights.is_empty() {
            queue.write_buffer(&self.weights, 0, bytemuck::cast_slice(weights));
        }
        crate::renderer::write_growable(
            device,
            queue,
            &mut self.instances,
            instances,
            wgpu::BufferUsages::VERTEX,
        );
        calls
            .iter()
            .map(|c| match c.part.deform.as_ref().and_then(|d| d.morph.as_ref()) {
                Some((deltas, _)) => Self::bind(device, &self.layout, &self.palettes, &self.weights, deltas),
                None => self.plain.clone(),
            })
            .collect()
    }
}

/// One frame of deformed draws.
pub(crate) struct Built<'a> {
    pub palettes: Vec<[[f32; 4]; 4]>,
    pub morph_weights: Vec<f32>,
    pub instances: Vec<SkinnedInstance>,
    pub calls: Vec<SkinnedCall<'a>>,
}

/// Builds this frame's joint palettes, morph weights, deformed instances (grouped by model,
/// part and material source, in key order, entities in id order within a group) and draw
/// calls.
pub(crate) fn build<'a>(
    frame: &FrameData,
    models: &'a BTreeMap<String, Result<GpuModel, String>>,
    plain_material: &'a wgpu::BindGroup,
) -> Built<'a> {
    let mut palettes: Vec<[[f32; 4]; 4]> = Vec::new();
    let mut morph_weights: Vec<f32> = Vec::new();
    let mut groups: BTreeMap<(&str, usize, bool), Vec<SkinnedInstance>> = BTreeMap::new();
    for draw in &frame.meshes {
        let Some(Ok(model)) = models.get(&*draw.asset) else {
            continue;
        };
        let node_matrix = |node: usize| {
            draw.pose
                .as_ref()
                .and_then(|pose| pose.nodes.get(node).map(Mat4::from_cols_array))
                .or_else(|| model.rest.get(node).copied())
                .unwrap_or(Mat4::IDENTITY)
        };
        // One palette per skin this draw uses, in skin order.
        let mut base_of: Vec<Option<u32>> = vec![None; model.skins.len()];
        for (p, part) in model.parts.iter().enumerate() {
            let Some(deform) = &part.deform else {
                continue;
            };
            let base = match deform.skin {
                Some(s) => match base_of[s] {
                    Some(b) => b,
                    None => {
                        let b = palettes.len() as u32;
                        let skin = &model.skins[s];
                        for (j, &node) in skin.joints.iter().enumerate() {
                            palettes.push((node_matrix(node) * skin.inverse_bind[j]).to_cols_array_2d());
                        }
                        base_of[s] = Some(b);
                        b
                    }
                },
                None => {
                    // Morph-only part: one palette entry, its node's matrix.
                    let b = palettes.len() as u32;
                    let m = part.node.map_or(part.transform, node_matrix);
                    palettes.push(m.to_cols_array_2d());
                    b
                }
            };
            let (weight_base, count) = match (&deform.morph, part.node) {
                (Some((_, targets)), Some(node)) => {
                    let w = draw
                        .pose
                        .as_ref()
                        .and_then(|pose| pose.morph_weights.get(node))
                        .or_else(|| model.rest_morph.get(node));
                    let b = morph_weights.len() as u32;
                    for t in 0..*targets as usize {
                        morph_weights.push(w.and_then(|w| w.get(t)).copied().unwrap_or(0.0));
                    }
                    (b, *targets)
                }
                _ => (0, 0),
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
                        emissive: draw.emissive.extend(0.0).to_array(),
                    },
                    deform: [base, weight_base, count, deform.vertex_count],
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
    Built {
        palettes,
        morph_weights,
        instances,
        calls,
    }
}
