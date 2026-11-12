//! The wgpu renderer: a forward PBR pass for models, then a batched sprite pass.
//!
//! Frame structure (stereo-ready): bind group 0 holds per-view data (view-projection, eye),
//! group 1 per-frame lights, group 2 material textures, group 3 per-draw data (transform and
//! material factors, one dynamic-offset uniform buffer for the whole frame). A view is a
//! value passed to [`Renderer::render`], never renderer state, so a later OpenXR module can
//! render two eyes by calling it twice (or with multiview) with different views.

use crate::assets::{self, Image, MaterialData, ModelData};
use crate::extract::{FrameData, View};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use marionette_core::Diagnostic;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::path::PathBuf;
use wgpu::util::DeviceExt;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_LIGHTS: usize = 4;
/// Dynamic uniform offsets must be multiples of this (WebGPU minimum alignment).
const DRAW_STRIDE: u64 = 256;

pub mod codes {
    pub const ASSET: &str = "render.asset";
    pub const NO_CAMERA: &str = "render.no_camera";
    pub const NO_ADAPTER: &str = "render.no_adapter";
    pub const DEVICE: &str = "render.device";
    pub const SURFACE: &str = "render.surface";
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightsUniform {
    direction: [[f32; 4]; MAX_LIGHTS],
    radiance: [[f32; 4]; MAX_LIGHTS],
    ambient: [f32; 4],
    count: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawUniform {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    base_color: [f32; 4],
    /// metallic, roughness, normal_scale, has_normal_map (0/1)
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteVertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

struct GpuPart {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    material: usize,
    transform: Mat4,
}

struct GpuModel {
    parts: Vec<GpuPart>,
    /// Texture bind group and factors per material.
    materials: Vec<(wgpu::BindGroup, MaterialData)>,
}

struct GpuTexture {
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// Color, resolve and depth views for one render.
pub struct Target<'a> {
    /// Multisampled color view when `sample_count > 1`, else the final view.
    pub color: &'a wgpu::TextureView,
    /// Final single-sample view when multisampling, else `None`.
    pub resolve: Option<&'a wgpu::TextureView>,
    pub depth: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub color_format: wgpu::TextureFormat,
    pub sample_count: u32,
    base_dir: PathBuf,
    view_buffer: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    lights_buffer: wgpu::Buffer,
    lights_group: wgpu::BindGroup,
    draw_buffer: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    draw_capacity: u64,
    draw_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    sprite_tex_layout: wgpu::BindGroupLayout,
    pbr_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    material_sampler: wgpu::Sampler,
    sprite_sampler: wgpu::Sampler,
    defaults: [wgpu::TextureView; 3], // white sRGB, white linear, flat normal
    models: BTreeMap<String, Result<GpuModel, String>>,
    /// Textures for `Material` overrides (all defaults).
    plain_material: wgpu::BindGroup,
    sprite_textures: BTreeMap<String, Result<GpuTexture, String>>,
    sprite_vertices: Option<wgpu::Buffer>,
    sprite_indices: Option<wgpu::Buffer>,
    ui_pipeline: wgpu::RenderPipeline,
    ui_view_buffer: wgpu::Buffer,
    ui_view_group: wgpu::BindGroup,
    nearest_sampler: wgpu::Sampler,
    ui_vertices: Option<wgpu::Buffer>,
    ui_indices: Option<wgpu::Buffer>,
}

fn uniform_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    dynamic: bool,
    size: u64,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn buffer_resource(buffer: &wgpu::Buffer, size: u64) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: 0,
        size: NonZeroU64::new(size),
    })
}

fn mat(m: Mat4) -> [[f32; 4]; 4] {
    m.to_cols_array_2d()
}

impl Renderer {
    /// `base_dir` is where relative asset paths (models, sprite images) are resolved; normally
    /// the scene file's directory.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        base_dir: PathBuf,
    ) -> Self {
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let view_size = size_of::<ViewUniform>() as u64;
        let lights_size = size_of::<LightsUniform>() as u64;
        let draw_size = size_of::<DrawUniform>() as u64;

        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let view_layout = layout("view", &[uniform_entry(0, vf, false, view_size)]);
        let lights_layout = layout(
            "lights",
            &[uniform_entry(0, wgpu::ShaderStages::FRAGMENT, false, lights_size)],
        );
        let material_layout = layout(
            "material",
            &[
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                sampler_entry(4),
            ],
        );
        let draw_layout = layout("draw", &[uniform_entry(0, vf, true, draw_size)]);
        let sprite_tex_layout = layout("sprite texture", &[texture_entry(0), sampler_entry(1)]);

        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let view_buffer = buffer("view", view_size);
        let lights_buffer = buffer("lights", lights_size);
        let draw_capacity = 64;
        let draw_buffer = buffer("draws", draw_capacity * DRAW_STRIDE);
        let group = |label, layout: &wgpu::BindGroupLayout, res| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: res,
                }],
            })
        };
        let view_group = group("view", &view_layout, buffer_resource(&view_buffer, view_size));
        let ui_view_buffer = buffer("ui view", view_size);
        let ui_view_group = group(
            "ui view",
            &view_layout,
            buffer_resource(&ui_view_buffer, view_size),
        );
        let lights_group = group(
            "lights",
            &lights_layout,
            buffer_resource(&lights_buffer, lights_size),
        );
        let draw_group = group("draws", &draw_layout, buffer_resource(&draw_buffer, draw_size));

        let depth_stencil = |write: bool| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let multisample = wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };

        let pbr_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pbr.wgsl").into()),
        });
        let pbr_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr"),
            bind_group_layouts: &[
                Some(&view_layout),
                Some(&lights_layout),
                Some(&material_layout),
                Some(&draw_layout),
            ],
            immediate_size: 0,
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let pbr_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr"),
            layout: Some(&pbr_layout),
            vertex: wgpu::VertexState {
                module: &pbr_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &mesh_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth_stencil(true)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &pbr_shader,
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

        let sprite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sprite.wgsl").into()),
        });
        let sprite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&view_layout), Some(&sprite_tex_layout)],
            immediate_size: 0,
        });
        let sprite_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let sprite_pipeline_for = |label, depth: wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&sprite_layout),
                vertex: wgpu::VertexState {
                    module: &sprite_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<SpriteVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &sprite_attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(), // sprites are visible from both sides
                depth_stencil: Some(depth),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_shader,
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
            })
        };
        let sprite_pipeline = sprite_pipeline_for("sprite", depth_stencil(false));
        // UI: same shader, drawn over everything (no depth test).
        let ui_pipeline = sprite_pipeline_for(
            "ui",
            wgpu::DepthStencilState {
                depth_compare: Some(wgpu::CompareFunction::Always),
                ..depth_stencil(false)
            },
        );

        let sampler = |label, filter, address| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let material_sampler = sampler("material", wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat);
        // Linear + clamp; atlas regions are inset by half a texel so neighbours never bleed in.
        let sprite_sampler = sampler("sprite", wgpu::FilterMode::Linear, wgpu::AddressMode::ClampToEdge);
        // Nearest for the bitmap font, so text stays crisp at whole-number scales.
        let nearest_sampler = sampler(
            "nearest",
            wgpu::FilterMode::Nearest,
            wgpu::AddressMode::ClampToEdge,
        );

        let defaults = [
            create_texture_view(device, queue, &Image::solid([255; 4], true)),
            create_texture_view(device, queue, &Image::solid([255; 4], false)),
            create_texture_view(device, queue, &Image::solid([128, 128, 255, 255], false)),
        ];
        let plain_material = material_textures(
            device,
            &material_layout,
            &material_sampler,
            &defaults,
            &MaterialData::default(),
            &[],
        );
        Self {
            device: device.clone(),
            queue: queue.clone(),
            color_format,
            sample_count,
            base_dir,
            view_buffer,
            view_group,
            lights_buffer,
            lights_group,
            draw_buffer,
            draw_group,
            draw_capacity,
            draw_layout,
            material_layout,
            sprite_tex_layout,
            pbr_pipeline,
            sprite_pipeline,
            material_sampler,
            sprite_sampler,
            defaults,
            models: BTreeMap::new(),
            plain_material,
            sprite_textures: BTreeMap::new(),
            sprite_vertices: None,
            sprite_indices: None,
            ui_pipeline,
            ui_view_buffer,
            ui_view_group,
            nearest_sampler,
            ui_vertices: None,
            ui_indices: None,
        }
    }

    /// Loads an image (once) as a sprite/UI texture.
    fn ensure_texture(&mut self, image: &str) -> Option<&String> {
        if !self.sprite_textures.contains_key(image) {
            let sampler = if image == "builtin:font" {
                &self.nearest_sampler
            } else {
                &self.sprite_sampler
            };
            let loaded = assets::load_image(image, &self.base_dir).map(|img| {
                let view = create_texture_view(&self.device, &self.queue, &img);
                GpuTexture {
                    bind_group: self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("sprite texture"),
                        layout: &self.sprite_tex_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    }),
                    width: img.width,
                    height: img.height,
                }
            });
            self.sprite_textures.insert(image.to_string(), loaded);
        }
        self.sprite_textures[image].as_ref().err()
    }

    /// Loads (once) every asset the frame needs. Returns a diagnostic per asset that failed.
    pub fn prepare(&mut self, frame: &FrameData) -> Vec<Diagnostic> {
        let mut errors = Vec::new();
        for draw in &frame.meshes {
            if !self.models.contains_key(&draw.asset) {
                let loaded = assets::load_model(&draw.asset, &self.base_dir).map(|m| self.upload_model(&m));
                self.models.insert(draw.asset.clone(), loaded);
            }
            if let Some(Err(msg)) = self.models.get(&draw.asset) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone())
                        .entity(draw.entity)
                        .component("Model")
                        .fix(
                            "check the path (relative to the scene file) or use primitive:cube|sphere|plane",
                        ),
                );
            }
        }
        for sprite in &frame.sprites {
            if let Some(msg) = self.ensure_texture(&sprite.image) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone())
                        .entity(sprite.entity)
                        .component("Sprite")
                        .fix("check the PNG path (relative to the scene file) or use a builtin: image"),
                );
            }
        }
        for ui in &frame.ui {
            if let Some(msg) = self.ensure_texture(ui.look.image()) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone())
                        .entity(ui.entity)
                        .component("UiImage")
                        .fix("check the PNG path (relative to the scene file) or use a builtin: image"),
                );
            }
        }
        errors
    }

    fn upload_model(&self, model: &ModelData) -> GpuModel {
        let views: Vec<wgpu::TextureView> = model
            .images
            .iter()
            .map(|img| create_texture_view(&self.device, &self.queue, img))
            .collect();
        let materials = model
            .materials
            .iter()
            .map(|m| {
                let group = material_textures(
                    &self.device,
                    &self.material_layout,
                    &self.material_sampler,
                    &self.defaults,
                    m,
                    &views,
                );
                let mut m = m.clone();
                // A normal texture index that points nowhere means "no normal map".
                m.normal_texture = m.normal_texture.filter(|&i| i < views.len());
                (group, m)
            })
            .collect();
        let parts = model
            .parts
            .iter()
            .map(|p| {
                let vertices: Vec<MeshVertex> = (0..p.mesh.positions.len())
                    .map(|i| MeshVertex {
                        position: p.mesh.positions[i],
                        normal: p.mesh.normals[i],
                        uv: p.mesh.uvs[i],
                    })
                    .collect();
                GpuPart {
                    vertices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh indices"),
                        contents: bytemuck::cast_slice(&p.mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: p.mesh.indices.len() as u32,
                    material: p.material,
                    transform: p.transform,
                }
            })
            .collect();
        GpuModel { parts, materials }
    }

    fn ensure_draw_capacity(&mut self, draws: u64) {
        if draws <= self.draw_capacity {
            return;
        }
        self.draw_capacity = draws.next_power_of_two();
        self.draw_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draws"),
            size: self.draw_capacity * DRAW_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let size = size_of::<DrawUniform>() as u64;
        self.draw_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draws"),
            layout: &self.draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer_resource(&self.draw_buffer, size),
            }],
        });
    }

    /// Records one view of the frame into `encoder`. Call [`Renderer::prepare`] first.
    /// Uploads per-view data with `queue.write_buffer`, so submit before rendering the next
    /// view.
    pub fn render(
        &mut self,
        frame: &FrameData,
        view: &View,
        target: &Target,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        // Per-view data.
        let vu = ViewUniform {
            view_proj: mat(view.proj * view.view),
            eye: view.eye.extend(1.0).to_array(),
        };
        self.queue
            .write_buffer(&self.view_buffer, 0, bytemuck::bytes_of(&vu));

        // Per-frame lights.
        let mut lu = LightsUniform::zeroed();
        for (i, l) in frame.lights.iter().take(MAX_LIGHTS).enumerate() {
            lu.direction[i] = l.direction.extend(0.0).to_array();
            lu.radiance[i] = l.radiance.extend(0.0).to_array();
        }
        lu.ambient = frame.ambient.extend(0.0).to_array();
        lu.count[0] = frame.lights.len().min(MAX_LIGHTS) as u32;
        self.queue
            .write_buffer(&self.lights_buffer, 0, bytemuck::bytes_of(&lu));

        // Per-draw transforms, in a fixed order.
        struct DrawCall<'a> {
            part: &'a GpuPart,
            material: &'a wgpu::BindGroup,
        }
        let mut uniforms: Vec<u8> = Vec::new();
        let mut count = 0u64;
        for draw in &frame.meshes {
            if let Some(Ok(model)) = self.models.get(&draw.asset) {
                for part in &model.parts {
                    let model_m = draw.world * part.transform;
                    let (base_color, params) = match draw.material {
                        Some(m) => {
                            let c = m.base_color;
                            ([c.r, c.g, c.b, c.a], [m.metallic, m.roughness, 1.0, 0.0])
                        }
                        None => {
                            let (_, md) = &model.materials[part.material.min(model.materials.len() - 1)];
                            let has_normal = if md.normal_texture.is_some() { 1.0 } else { 0.0 };
                            (
                                md.base_color,
                                [md.metallic, md.roughness, md.normal_scale, has_normal],
                            )
                        }
                    };
                    let du = DrawUniform {
                        model: mat(model_m),
                        normal: mat(model_m.inverse().transpose()),
                        base_color,
                        params,
                    };
                    uniforms.extend_from_slice(bytemuck::bytes_of(&du));
                    uniforms.resize(((count + 1) * DRAW_STRIDE) as usize, 0);
                    count += 1;
                }
            }
        }
        self.ensure_draw_capacity(count.max(1));
        if !uniforms.is_empty() {
            self.queue.write_buffer(&self.draw_buffer, 0, &uniforms);
        }
        let mut calls = Vec::new();
        for draw in &frame.meshes {
            if let Some(Ok(model)) = self.models.get(&draw.asset) {
                for part in &model.parts {
                    let material = match draw.material {
                        Some(_) => &self.plain_material,
                        None => &model.materials[part.material.min(model.materials.len() - 1)].0,
                    };
                    calls.push(DrawCall { part, material });
                }
            }
        }

        // Sprites: build one vertex/index buffer, batch consecutive sprites by image.
        let mut sv: Vec<SpriteVertex> = Vec::new();
        let mut si: Vec<u32> = Vec::new();
        let mut batches: Vec<(&str, std::ops::Range<u32>)> = Vec::new();
        for s in &frame.sprites {
            let Some(Ok(tex)) = self.sprite_textures.get(&s.image) else {
                continue;
            };
            let (u0, v0, u1, v1) = match s.region {
                Some(r) => {
                    let (w, h) = (tex.width as f32, tex.height as f32);
                    (
                        (r.x as f32 + 0.5) / w,
                        (r.y as f32 + 0.5) / h,
                        ((r.x + r.width) as f32 - 0.5) / w,
                        ((r.y + r.height) as f32 - 0.5) / h,
                    )
                }
                None => (0.0, 0.0, 1.0, 1.0),
            };
            let base = sv.len() as u32;
            let (hx, hy) = (s.size.x * 0.5, s.size.y * 0.5);
            for (x, y, u, v) in [
                (-hx, -hy, u0, v1),
                (hx, -hy, u1, v1),
                (hx, hy, u1, v0),
                (-hx, hy, u0, v0),
            ] {
                let p = s.world.transform_point3(Vec3::new(x, y, 0.0));
                sv.push(SpriteVertex {
                    position: p.to_array(),
                    uv: [u, v],
                    color: s.color,
                });
            }
            let start = si.len() as u32;
            si.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            match batches.last_mut() {
                Some((img, range)) if *img == s.image => range.end = si.len() as u32,
                _ => batches.push((&s.image, start..si.len() as u32)),
            }
        }
        if !sv.is_empty() {
            write_growable(
                &self.device,
                &self.queue,
                &mut self.sprite_vertices,
                &sv,
                wgpu::BufferUsages::VERTEX,
            );
            write_growable(
                &self.device,
                &self.queue,
                &mut self.sprite_indices,
                &si,
                wgpu::BufferUsages::INDEX,
            );
        }

        // UI: quads in screen pixels (y down), batched by texture like sprites.
        let (sw, sh) = (target.width as f32, target.height as f32);
        let ui_proj = glam::camera::rh::proj::directx::orthographic(0.0, sw, sh, 0.0, -1.0, 1.0);
        let uvu = ViewUniform {
            view_proj: mat(ui_proj),
            eye: [0.0, 0.0, 0.0, 1.0],
        };
        self.queue
            .write_buffer(&self.ui_view_buffer, 0, bytemuck::bytes_of(&uvu));
        let mut uv: Vec<SpriteVertex> = Vec::new();
        let mut ui_idx: Vec<u32> = Vec::new();
        let mut ui_batches: Vec<(&str, std::ops::Range<u32>)> = Vec::new();
        for u in &frame.ui {
            let image = u.look.image();
            let Some(Ok(tex)) = self.sprite_textures.get(image) else {
                continue;
            };
            let (x, y, w, h) = marionette_core::ui::layout(&u.node, sw, sh);
            let start = ui_idx.len() as u32;
            let mut quad =
                |x0: f32, y0: f32, x1: f32, y1: f32, uv0: [f32; 2], uv1: [f32; 2], color: [f32; 4]| {
                    let base = uv.len() as u32;
                    for (px, py, tu, tv) in [
                        (x0, y0, uv0[0], uv0[1]),
                        (x1, y0, uv1[0], uv0[1]),
                        (x1, y1, uv1[0], uv1[1]),
                        (x0, y1, uv0[0], uv1[1]),
                    ] {
                        uv.push(SpriteVertex {
                            position: [px, py, 0.0],
                            uv: [tu, tv],
                            color,
                        });
                    }
                    ui_idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                };
            match &u.look {
                crate::extract::UiLook::Rect { color } | crate::extract::UiLook::Image { color, .. } => {
                    quad(x, y, x + w, y + h, [0.0, 0.0], [1.0, 1.0], *color)
                }
                crate::extract::UiLook::Text { text, size, color } => {
                    use assets::font::{CELL_H, CELL_W, cell};
                    let k = size / CELL_H as f32;
                    let (tw, th) = (tex.width as f32, tex.height as f32);
                    let (mut cx, mut cy) = (x, y);
                    for ch in text.chars() {
                        if ch == '\n' {
                            cx = x;
                            cy += CELL_H as f32 * k;
                            continue;
                        }
                        if ch != ' ' {
                            let (gx, gy) = cell(ch);
                            let uv0 = [gx as f32 / tw, gy as f32 / th];
                            let uv1 = [(gx + CELL_W) as f32 / tw, (gy + CELL_H) as f32 / th];
                            quad(
                                cx,
                                cy,
                                cx + CELL_W as f32 * k,
                                cy + CELL_H as f32 * k,
                                uv0,
                                uv1,
                                *color,
                            );
                        }
                        cx += CELL_W as f32 * k;
                    }
                }
            }
            let end = ui_idx.len() as u32;
            match ui_batches.last_mut() {
                Some((img, range)) if *img == image => range.end = end,
                _ => ui_batches.push((image, start..end)),
            }
        }
        if !uv.is_empty() {
            write_growable(
                &self.device,
                &self.queue,
                &mut self.ui_vertices,
                &uv,
                wgpu::BufferUsages::VERTEX,
            );
            write_growable(
                &self.device,
                &self.queue,
                &mut self.ui_indices,
                &ui_idx,
                wgpu::BufferUsages::INDEX,
            );
        }

        let c = view.clear;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.color,
                depth_slice: None,
                resolve_target: target.resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: c[0] as f64,
                        g: c[1] as f64,
                        b: c[2] as f64,
                        a: c[3] as f64,
                    }),
                    store: if target.resolve.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        if !calls.is_empty() {
            pass.set_pipeline(&self.pbr_pipeline);
            pass.set_bind_group(0, &self.view_group, &[]);
            pass.set_bind_group(1, &self.lights_group, &[]);
            for (i, call) in calls.iter().enumerate() {
                pass.set_bind_group(2, call.material, &[]);
                pass.set_bind_group(3, &self.draw_group, &[(i as u64 * DRAW_STRIDE) as u32]);
                pass.set_vertex_buffer(0, call.part.vertices.slice(..));
                pass.set_index_buffer(call.part.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..call.part.index_count, 0, 0..1);
            }
        }
        if let (Some(vb), Some(ib), false) = (&self.sprite_vertices, &self.sprite_indices, batches.is_empty())
        {
            pass.set_pipeline(&self.sprite_pipeline);
            pass.set_bind_group(0, &self.view_group, &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            for (image, range) in batches {
                if let Some(Ok(tex)) = self.sprite_textures.get(image) {
                    pass.set_bind_group(1, &tex.bind_group, &[]);
                    pass.draw_indexed(range, 0, 0..1);
                }
            }
        }
        if let (Some(vb), Some(ib), false) = (&self.ui_vertices, &self.ui_indices, ui_batches.is_empty()) {
            pass.set_pipeline(&self.ui_pipeline);
            pass.set_bind_group(0, &self.ui_view_group, &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            for (image, range) in ui_batches {
                if let Some(Ok(tex)) = self.sprite_textures.get(image) {
                    pass.set_bind_group(1, &tex.bind_group, &[]);
                    pass.draw_indexed(range, 0, 0..1);
                }
            }
        }
    }
}

/// Bind group with a material's textures (defaults where it has none).
fn material_textures(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    defaults: &[wgpu::TextureView; 3],
    m: &MaterialData,
    views: &[wgpu::TextureView],
) -> wgpu::BindGroup {
    let pick =
        |idx: Option<usize>, default: usize| idx.and_then(|i| views.get(i)).unwrap_or(&defaults[default]);
    let tex = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            tex(1, pick(m.base_color_texture, 0)),
            tex(2, pick(m.metallic_roughness_texture, 1)),
            tex(3, pick(m.normal_texture, 2)),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Writes `data` into `slot`, recreating the buffer when it is too small.
fn write_growable<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut Option<wgpu::Buffer>,
    data: &[T],
    usage: wgpu::BufferUsages,
) {
    let bytes: &[u8] = bytemuck::cast_slice(data);
    let needed = (bytes.len() as u64).next_power_of_two().max(1024);
    if slot.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite batch"),
            size: needed,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    queue.write_buffer(slot.as_ref().expect("just created"), 0, bytes);
}

/// Uploads an RGBA8 image (no mipmaps) and returns a view of it.
pub fn create_texture_view(device: &wgpu::Device, queue: &wgpu::Queue, img: &Image) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: img.width,
        height: img.height,
        depth_or_array_layers: 1,
    };
    let format = if img.srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &img.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * img.width),
            rows_per_image: Some(img.height),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
