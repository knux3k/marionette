//! World-space quads (text and panels placed in the 3D world), drawn from
//! `FrameData::quads` with a distance-field shader (`shaders/worldquads.wgsl`). Kept out of
//! `renderer.rs` on purpose: the renderer only creates this, calls [`WorldQuadPass::upload`]
//! once per frame and [`WorldQuadPass::draw`] once per view.

use crate::renderer::write_growable;
use bytemuck::{Pod, Zeroable};
use marionette_core::spatial::WorldQuads;
use std::ops::Range;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

pub(crate) struct WorldQuadPass {
    pipeline: wgpu::RenderPipeline,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    scratch_v: Vec<Vertex>,
    scratch_i: Vec<u32>,
    /// One range of indices per `FrameData::quads` entry (image name, indices), merged when
    /// neighbours share an image.
    batches: Vec<(String, Range<u32>)>,
}

impl WorldQuadPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        multisample: wgpu::MultisampleState,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world quads"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/worldquads.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world quads"),
            bind_group_layouts: &[Some(view_layout), Some(tex_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world quads"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
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
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            vertices: None,
            indices: None,
            scratch_v: Vec::new(),
            scratch_i: Vec::new(),
            batches: Vec::new(),
        }
    }
}

impl WorldQuadPass {
    /// Builds this frame's buffers. Indices are made here (two triangles per four vertices),
    /// so hooks only produce corners.
    pub(crate) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, quads: &[WorldQuads]) {
        self.scratch_v.clear();
        self.scratch_i.clear();
        self.batches.clear();
        for q in quads {
            let count = q.vertices.len() / 4 * 4;
            if count == 0 {
                continue;
            }
            let start = self.scratch_i.len() as u32;
            let base = self.scratch_v.len() as u32;
            self.scratch_v.extend(q.vertices[..count].iter().map(|v| Vertex {
                position: v.position,
                uv: v.uv,
                color: v.color,
            }));
            for k in 0..(count / 4) as u32 {
                let b = base + k * 4;
                self.scratch_i.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
            }
            let end = self.scratch_i.len() as u32;
            match self.batches.last_mut() {
                Some((image, range)) if *image == q.image => range.end = end,
                _ => self.batches.push((q.image.clone(), start..end)),
            }
        }
        if self.batches.is_empty() {
            return;
        }
        write_growable(device, queue, &mut self.vertices, &self.scratch_v, wgpu::BufferUsages::VERTEX);
        write_growable(device, queue, &mut self.indices, &self.scratch_i, wgpu::BufferUsages::INDEX);
    }

    /// Draws the uploaded quads into `pass`; returns the draw calls made. `texture` finds the
    /// bind group of an image (nothing is drawn for an image that failed to load).
    pub(crate) fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        view_group: &'a wgpu::BindGroup,
        view_offset: u32,
        texture: impl Fn(&str) -> Option<&'a wgpu::BindGroup>,
    ) -> u32 {
        let (Some(vb), Some(ib)) = (&self.vertices, &self.indices) else {
            return 0;
        };
        if self.batches.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_group, &[view_offset]);
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
        let mut calls = 0;
        for (image, range) in &self.batches {
            if let Some(group) = texture(image) {
                pass.set_bind_group(1, group, &[]);
                pass.draw_indexed(range.clone(), 0, 0..1);
                calls += 1;
            }
        }
        calls
    }

    /// Image names of this frame's batches (for loading and trimming).
    pub(crate) fn images(quads: &[WorldQuads]) -> impl Iterator<Item = &str> {
        quads.iter().map(|q| q.image.as_str())
    }
}
