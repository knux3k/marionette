//! The wgpu renderer: a forward PBR pass for models, then a batched sprite pass.
//!
//! Frame structure (stereo-ready): bind group 0 holds per-view data (view-projection, eye),
//! group 1 per-frame lights, group 2 material textures. Per-object data (transform and
//! material factors) is a per-instance vertex buffer: every (model, part, material source)
//! is one instanced draw, however many entities use it. Views are values passed to
//! [`Renderer::render_views`], never renderer state: one call draws every view of a frame
//! (two eyes for stereo) from one set of per-frame buffers, each view picking its slot of
//! the view uniform buffer by dynamic offset.

use crate::assets::{self, Image, MaterialData, ModelData};
use crate::extract::{FrameData, View};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use marionette_core::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::mpsc;
use wgpu::util::DeviceExt;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_LIGHTS: usize = 4;
/// Point and spot lights per frame (lowest entity ids win).
pub const MAX_LOCAL_LIGHTS: usize = 16;
/// Depth format of the shadow map.
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Flags in `LightsUniform::count[2]` (match `pbr.wgsl`).
const FLAG_SHADOWS: u32 = 1;
const FLAG_TONEMAP: u32 = 2;
const FLAG_FOG: u32 = 4;
const FLAG_SKY: u32 = 8;
const FLAG_HDR: u32 = 16;
const FLAG_ENV: u32 = 32;
/// Shadow maps for point and spot lights per frame (a point light takes six).
pub const MAX_LOCAL_SHADOW_LAYERS: usize = 24;
/// Dynamic uniform offsets must be multiples of this (WebGPU minimum alignment).
const VIEW_STRIDE: u64 = 256;
/// View buffer slots: world and UI per view, the sun's shadow map, local shadow layers.
const VIEW_SLOTS: u64 = 2 * MAX_VIEWS as u64 + 1 + MAX_LOCAL_SHADOW_LAYERS as u64;
/// First view buffer slot of the local light shadow layers.
const LOCAL_SHADOW_SLOT: u64 = 2 * MAX_VIEWS as u64 + 1;
/// Most views one [`Renderer::render_views`] call can draw (two for stereo; more for
/// split screen or mirrors).
pub const MAX_VIEWS: usize = 8;

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
    inv_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightsUniform {
    direction: [[f32; 4]; MAX_LIGHTS],
    radiance: [[f32; 4]; MAX_LIGHTS],
    ambient: [f32; 4],
    /// Directional lights, local lights, flags.
    count: [u32; 4],
    ground: [f32; 4],
    fog: [f32; 4],
    /// Exposure, shadow texel size (uv), softness (texels), normal offset (m).
    post: [f32; 4],
    shadow_view_proj: [[f32; 4]; 4],
    local_pos: [[f32; 4]; MAX_LOCAL_LIGHTS],
    local_rad: [[f32; 4]; MAX_LOCAL_LIGHTS],
    local_dir: [[f32; 4]; MAX_LOCAL_LIGHTS],
    /// First shadow layer (< 0: none), normal offset per meter, texel size (uv), softness.
    local_shadow: [[f32; 4]; MAX_LOCAL_LIGHTS],
    shadow_mats: [[[f32; 4]; 4]; MAX_LOCAL_SHADOW_LAYERS],
    /// Environment intensity, last mip level.
    env: [f32; 4],
    sh: [[f32; 4]; 9],
}

/// Per-material uniform (group 2, binding 0).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    emissive: [f32; 4],
}

/// Shadow maps of point and spot lights: one depth texture array, one layer per map.
struct LocalShadowMaps {
    resolution: u32,
    layers: u32,
    array: wgpu::TextureView,
    per_layer: Vec<wgpu::TextureView>,
}

/// The environment texture (prefiltered mips) and its irradiance.
struct GpuEnvironment {
    source: crate::environment::Source,
    view: wgpu::TextureView,
    sh: [[f32; 4]; 9],
}

/// One instanced draw of a static mesh part.
struct DrawCall<'a> {
    /// The model shares its entity with a shadow-casting point or spot light: it does not
    /// cast into local light shadow maps (a lantern would otherwise block its own light).
    lamp: bool,
    part: &'a GpuPart,
    material: &'a wgpu::BindGroup,
    instances: std::ops::Range<u32>,
}

/// Everything a frame draws for models, shared by the shadow, HDR and final passes.
struct MeshCalls<'a> {
    calls: &'a [DrawCall<'a>],
    instances: Option<&'a wgpu::Buffer>,
    skinned: &'a [crate::skinning::SkinnedCall<'a>],
    deform_groups: &'a [wgpu::BindGroup],
    skinned_instances: Option<&'a wgpu::Buffer>,
}

impl MeshCalls<'_> {
    /// Depth only (shadow maps): group 0 at `offset` holds the light's view-projection.
    fn draw_depth(
        &self,
        pass: &mut wgpu::RenderPass,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        view_group: &wgpu::BindGroup,
        offset: u32,
        local: bool,
        stats: &mut RenderStats,
    ) {
        self.draw(pass, pipelines, (view_group, offset), (None, local), stats);
    }

    /// Lit: groups 0 (view at `offset`), 1 (lights) and 2 (material).
    fn draw_lit(
        &self,
        pass: &mut wgpu::RenderPass,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        view_group: &wgpu::BindGroup,
        offset: u32,
        lights: &wgpu::BindGroup,
        stats: &mut RenderStats,
    ) {
        self.draw(
            pass,
            pipelines,
            (view_group, offset),
            (Some(lights), false),
            stats,
        );
    }

    fn draw(
        &self,
        pass: &mut wgpu::RenderPass,
        (plain, skinned): (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        (view_group, offset): (&wgpu::BindGroup, u32),
        // Lights group (lit draws), or none (depth); skip models holding a lamp.
        (lights, skip_lamps): (Option<&wgpu::BindGroup>, bool),
        stats: &mut RenderStats,
    ) {
        if let (Some(ib), false) = (self.instances, self.calls.is_empty()) {
            pass.set_pipeline(plain);
            pass.set_bind_group(0, view_group, &[offset]);
            if let Some(l) = lights {
                pass.set_bind_group(1, l, &[]);
            }
            pass.set_vertex_buffer(1, ib.slice(..));
            for call in self.calls.iter().filter(|c| !(skip_lamps && c.lamp)) {
                if lights.is_some() {
                    pass.set_bind_group(2, call.material, &[]);
                }
                pass.set_vertex_buffer(0, call.part.vertices.slice(..));
                pass.set_index_buffer(call.part.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..call.part.index_count, 0, call.instances.clone());
                stats.draw_calls += 1;
            }
        }
        if let (Some(ib), false) = (self.skinned_instances, self.skinned.is_empty()) {
            pass.set_pipeline(skinned);
            pass.set_bind_group(0, view_group, &[offset]);
            if let Some(l) = lights {
                pass.set_bind_group(1, l, &[]);
            }
            pass.set_vertex_buffer(1, ib.slice(..));
            for (call, group) in self.skinned.iter().zip(self.deform_groups) {
                let Some(deform) = &call.part.deform else {
                    continue;
                };
                if lights.is_some() {
                    pass.set_bind_group(2, call.material, &[]);
                }
                pass.set_bind_group(3, group, &[]);
                pass.set_vertex_buffer(0, call.part.vertices.slice(..));
                pass.set_vertex_buffer(2, deform.skin_vertices.slice(..));
                pass.set_index_buffer(call.part.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..call.part.index_count, 0, call.instances.clone());
                stats.draw_calls += 1;
            }
        }
    }
}

/// Per-instance vertex data: one per drawn copy of a mesh part.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct InstanceData {
    /// World matrix; the shader derives the normal matrix from it.
    pub model: [[f32; 4]; 4],
    pub base_color: [f32; 4],
    /// metallic, roughness, normal_scale, has_normal_map (0/1)
    pub params: [f32; 4],
    /// Light given off (rgb); zero for most instances.
    pub emissive: [f32; 4],
}

/// What one frame cost in GPU work, counted on the CPU. Deterministic for a given frame,
/// so tests can hold it to a budget (decision 0023).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RenderStats {
    /// Views drawn (render passes).
    pub views: u32,
    /// Draw calls over all views.
    pub draw_calls: u32,
    /// Mesh instances per view.
    pub instances: u32,
    /// Mesh triangles per view (sprites and UI not counted).
    pub triangles: u64,
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

pub(crate) struct GpuPart {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
    pub material: usize,
    pub transform: Mat4,
    /// The glTF node the part hangs from; an animated pose moves it.
    pub node: Option<usize>,
    /// Skin and morph target data for parts the GPU deforms (drawn by `skinning.rs`, not
    /// the static path).
    pub deform: Option<crate::skinning::Deform>,
}

pub(crate) struct GpuModel {
    pub parts: Vec<GpuPart>,
    /// Texture bind group and factors per material.
    pub materials: Vec<(wgpu::BindGroup, MaterialData)>,
    pub skins: Vec<crate::skinning::GpuSkin>,
    /// Model-space rest matrix of every node (used when an entity has no pose).
    pub rest: Vec<Mat4>,
    /// Default morph target weights per node (used when an entity has no pose).
    pub rest_morph: Vec<Vec<f32>>,
}

/// Base color and (metallic, roughness, normal scale, has normal map) of one instance.
pub(crate) fn instance_material(
    model: &GpuModel,
    part: &GpuPart,
    over: Option<marionette_core::spatial::Material>,
) -> ([f32; 4], [f32; 4]) {
    match over {
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
    }
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
    lights_layout: wgpu::BindGroupLayout,
    lights_group: wgpu::BindGroup,
    /// Shadow map (resolution, view) while a scene asks for shadows; a 1x1 map otherwise.
    shadow_map: (u32, wgpu::TextureView),
    shadow_sampler: wgpu::Sampler,
    shadow_pipeline: wgpu::RenderPipeline,
    /// Point and spot light shadow maps (decision 0065); one 1x1 layer when none cast.
    local_shadows: LocalShadowMaps,
    /// Environment (decision 0066), built in `prepare`; `None` while no scene asks.
    environment: Option<GpuEnvironment>,
    /// Stand-in environment texture (1x1 black).
    no_environment: wgpu::TextureView,
    env_sampler: wgpu::Sampler,
    /// Models into the HDR target, for bloom (decision 0064).
    pbr_hdr_pipeline: wgpu::RenderPipeline,
    /// Environment background in the final and HDR target.
    sky_pipelines: [wgpu::RenderPipeline; 2],
    post: crate::post::Post,
    instance_buffer: Option<wgpu::Buffer>,
    /// Reused per-frame arrays: instances in frame order, their group, instances by group.
    scratch: (Vec<InstanceData>, Vec<u32>, Vec<InstanceData>),
    material_layout: wgpu::BindGroupLayout,
    sprite_tex_layout: wgpu::BindGroupLayout,
    pbr_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    material_sampler: wgpu::Sampler,
    sprite_sampler: wgpu::Sampler,
    defaults: [wgpu::TextureView; 3], // white sRGB, white linear, flat normal
    models: BTreeMap<String, Result<GpuModel, String>>,
    /// Background model decoding while streaming (decision 0068); `None` until first used.
    loader: Option<ModelLoader>,
    /// Models first needed after the first prepared frame decode in the background.
    streaming: bool,
    /// At least one frame was prepared (the first frame always loads everything at once).
    prepared_once: bool,
    /// Textures for `Material` overrides (all defaults).
    plain_material: wgpu::BindGroup,
    sprite_textures: BTreeMap<String, Result<GpuTexture, String>>,
    sprite_vertices: Option<wgpu::Buffer>,
    sprite_indices: Option<wgpu::Buffer>,
    ui_pipeline: wgpu::RenderPipeline,
    world_quads: crate::worldquads::WorldQuadPass,
    nearest_sampler: wgpu::Sampler,
    ui_vertices: Option<wgpu::Buffer>,
    ui_indices: Option<wgpu::Buffer>,
    skinning: crate::skinning::Skinning,
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

/// One background thread decoding model files (decision 0068). It ends when the renderer
/// drops its sender; at most one request per distinct asset is in flight, so the queues are
/// bounded by the scene's asset count.
struct ModelLoader {
    ask: mpsc::Sender<(String, PathBuf)>,
    done: mpsc::Receiver<(String, Result<ModelData, String>)>,
    pending: BTreeSet<String>,
}

impl ModelLoader {
    fn start() -> Self {
        let (ask, requests) = mpsc::channel::<(String, PathBuf)>();
        let (reply, done) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("marionette-model-loader".into())
            .spawn(move || {
                for (asset, base) in requests {
                    let loaded = assets::load_model(&asset, &base);
                    if reply.send((asset, loaded)).is_err() {
                        break;
                    }
                }
            });
        // Without a thread the request channel is closed, so `ensure_model` loads in place.
        drop(spawned);
        Self {
            ask,
            done,
            pending: BTreeSet::new(),
        }
    }
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

        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let view_layout = layout("view", &[uniform_entry(0, vf, true, view_size)]);
        let lights_layout = layout(
            "lights",
            &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT, false, lights_size),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                texture_entry(4),
                sampler_entry(5),
            ],
        );
        let material_layout = layout(
            "material",
            &[
                uniform_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    false,
                    size_of::<MaterialUniform>() as u64,
                ),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                sampler_entry(4),
                texture_entry(5),
            ],
        );
        let sprite_tex_layout = layout("sprite texture", &[texture_entry(0), sampler_entry(1)]);

        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        // Two slots per view (world, UI), one for the sun's shadow map, one per local light
        // shadow layer.
        let view_buffer = buffer("views", VIEW_STRIDE * VIEW_SLOTS);
        let lights_buffer = buffer("lights", lights_size);
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
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_map = (0, shadow_texture(device, 1));
        let local_shadows = local_shadow_maps(device, 1, 1);
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("environment"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let no_environment = env_texture(
            device,
            queue,
            &[crate::environment::EnvImage {
                w: 1,
                h: 1,
                px: vec![Vec3::ZERO],
            }],
        );
        let lights_group = lights_bind_group(
            device,
            &lights_layout,
            &lights_buffer,
            [&shadow_map.1, &local_shadows.array, &no_environment],
            [&shadow_sampler, &env_sampler],
        );

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
            bind_group_layouts: &[Some(&view_layout), Some(&lights_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let instance_attrs = wgpu::vertex_attr_array![
            3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
            7 => Float32x4, 8 => Float32x4, 12 => Float32x4
        ];
        let pbr_for = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pbr"),
                layout: Some(&pbr_layout),
                vertex: wgpu::VertexState {
                    module: &pbr_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: size_of::<MeshVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &mesh_attrs,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: size_of::<InstanceData>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &instance_attrs,
                        }),
                    ],
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
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pbr_pipeline = pbr_for(color_format);
        let pbr_hdr_pipeline = pbr_for(crate::post::HDR_FORMAT);
        // Sky background: full-screen triangle at the far plane, depth-tested, not written.
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&view_layout), Some(&lights_layout)],
            immediate_size: 0,
        });
        let sky_for = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky"),
                layout: Some(&sky_layout),
                vertex: wgpu::VertexState {
                    module: &pbr_shader,
                    entry_point: Some("vs_sky"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(depth_stencil(false)),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &pbr_shader,
                    entry_point: Some("fs_sky"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky_pipelines = [sky_for(color_format), sky_for(crate::post::HDR_FORMAT)];
        let post = crate::post::Post::new(device, color_format, sample_count, depth_stencil(false));

        // Depth-only pass from the shadow-casting light: only the view group is used.
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&view_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &pbr_shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<MeshVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &mesh_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<InstanceData>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState::default(), // both faces cast shadows
            depth_stencil: Some(shadow_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let skinning = crate::skinning::Skinning::new(
            device,
            [&view_layout, &lights_layout, &material_layout],
            color_format,
            depth_stencil(true),
            multisample,
            size_of::<MeshVertex>() as u64,
        );

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

        let world_quads = crate::worldquads::WorldQuadPass::new(
            device,
            &view_layout,
            &sprite_tex_layout,
            color_format,
            DEPTH_FORMAT,
            multisample,
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
            lights_layout,
            lights_group,
            shadow_map,
            shadow_sampler,
            shadow_pipeline,
            local_shadows,
            environment: None,
            no_environment,
            env_sampler,
            pbr_hdr_pipeline,
            sky_pipelines,
            post,
            instance_buffer: None,
            scratch: Default::default(),
            material_layout,
            sprite_tex_layout,
            pbr_pipeline,
            sprite_pipeline,
            material_sampler,
            sprite_sampler,
            defaults,
            models: BTreeMap::new(),
            loader: None,
            streaming: false,
            prepared_once: false,
            plain_material,
            sprite_textures: BTreeMap::new(),
            sprite_vertices: None,
            sprite_indices: None,
            ui_pipeline,
            world_quads,
            nearest_sampler,
            ui_vertices: None,
            ui_indices: None,
            skinning,
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

    /// Frees cached GPU models and textures that `frame` does not draw; returns how many
    /// were freed. The cache only grows while the same assets are drawn, so call this after
    /// switching scenes (decision 0032). Freed assets load again if a later frame needs them.
    pub fn trim(&mut self, frame: &FrameData) -> usize {
        let models: std::collections::BTreeSet<&str> = frame
            .meshes
            .iter()
            .map(|d| &*d.asset)
            .chain(frame.bulk.iter().map(|b| b.asset.as_str()))
            .collect();
        let images: std::collections::BTreeSet<&str> = frame
            .sprites
            .iter()
            .map(|s| s.image.as_str())
            .chain(frame.ui.iter().map(|u| u.look.image()))
            .chain(crate::worldquads::WorldQuadPass::images(&frame.quads))
            .collect();
        let before = self.cached_assets();
        self.models.retain(|k, _| models.contains(k.as_str()));
        self.sprite_textures.retain(|k, _| images.contains(k.as_str()));
        before - self.cached_assets()
    }

    /// Models and textures held in the GPU cache (including failed loads).
    pub fn cached_assets(&self) -> usize {
        self.models.len() + self.sprite_textures.len()
    }

    /// Loads (once) every asset the frame needs. Returns a diagnostic per asset that failed.
    pub fn prepare(&mut self, frame: &FrameData) -> Vec<Diagnostic> {
        let mut errors = Vec::new();
        self.receive_models();
        let stream = self.streaming && self.prepared_once;
        for draw in &frame.meshes {
            self.ensure_model(&draw.asset, stream);
            if let Some(Err(msg)) = self.models.get(&*draw.asset) {
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
        for bulk in &frame.bulk {
            self.ensure_model(&bulk.asset, stream);
            if let Some(Err(msg)) = self.models.get(&bulk.asset) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone()).fix(
                        "check the model path a module draws in bulk, or use primitive:cube|sphere|plane",
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
        for q in &frame.quads {
            if let Some(msg) = self.ensure_texture(&q.image) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone()).fix(
                        "a module draws world-space quads with an image that is missing or was replaced",
                    ),
                );
            }
        }
        if frame.environment.is_none() && self.environment.is_some() {
            // Not used any more: give the texture back.
            self.environment = None;
            self.rebuild_lights_group();
        }
        if let Some(env) = &frame.environment
            && let Err(msg) = self.ensure_environment(env)
        {
            errors.push(
                Diagnostic::error(codes::ASSET, msg)
                    .component("Environment")
                    .fix("give an equirectangular PNG (2:1) relative to the scene file, or \"\" for a sky from SkyLight colors"),
            );
        }
        self.prepared_once = true;
        errors
    }

    /// Streaming (decision 0068): after the first prepared frame, a model no earlier frame
    /// needed is decoded on a background thread and drawn from the first frame after it
    /// arrives, instead of stalling the frame that first needs it (a generated model takes
    /// about 30 ms to decode). Off by default so offscreen pictures are exact; the window
    /// loop and render stability modes turn it on.
    pub fn set_streaming(&mut self, on: bool) {
        self.streaming = on;
    }

    /// Models still decoding in the background.
    pub fn models_loading(&self) -> usize {
        self.loader.as_ref().map_or(0, |l| l.pending.len())
    }

    /// Blocks until every model being decoded in the background is uploaded.
    pub fn finish_loading(&mut self) {
        while self.models_loading() > 0 {
            let Some(loader) = self.loader.as_mut() else {
                return;
            };
            match loader.done.recv() {
                Ok((asset, loaded)) => {
                    loader.pending.remove(&asset);
                    let gpu = loaded.map(|m| self.upload_model(&m));
                    self.models.insert(asset, gpu);
                }
                Err(_) => {
                    self.loader = None; // the thread is gone; later loads start a new one
                    return;
                }
            }
        }
    }

    /// Loads a model now, or (streaming) asks the background thread for it.
    fn ensure_model(&mut self, asset: &str, stream: bool) {
        if self.models.contains_key(asset) {
            return;
        }
        // Built-in shapes are cheap; only files go to the background.
        if stream && !asset.starts_with("primitive:") {
            let base = self.base_dir.clone();
            let loader = self.loader.get_or_insert_with(ModelLoader::start);
            if loader.pending.contains(asset) {
                return;
            }
            if loader.ask.send((asset.to_string(), base)).is_ok() {
                loader.pending.insert(asset.to_string());
                return;
            }
            self.loader = None; // thread gone: load in place below
        }
        let loaded = assets::load_model(asset, &self.base_dir).map(|m| self.upload_model(&m));
        self.models.insert(asset.to_string(), loaded);
    }

    /// Uploads models the background thread finished, without waiting.
    fn receive_models(&mut self) {
        let Some(loader) = self.loader.as_mut() else {
            return;
        };
        let done: Vec<_> = loader.done.try_iter().collect();
        for (asset, loaded) in done {
            if let Some(l) = self.loader.as_mut() {
                l.pending.remove(&asset);
            }
            let gpu = loaded.map(|m| self.upload_model(&m));
            self.models.insert(asset, gpu);
        }
    }

    /// Builds the environment texture when the scene's source changed (decision 0066).
    fn ensure_environment(&mut self, env: &crate::extract::EnvironmentData) -> Result<(), String> {
        use crate::environment::{self as e, Source};
        let source = if env.image.is_empty() {
            Source::Sky {
                sky: env.sky.to_array(),
                ground: env.ground.to_array(),
            }
        } else {
            Source::Image(env.image.clone())
        };
        if self.environment.as_ref().is_some_and(|g| g.source == source) {
            return Ok(());
        }
        let base = match &source {
            Source::Sky { sky, ground } => e::sky_image(Vec3::from(*sky), Vec3::from(*ground)),
            Source::Image(path) => {
                let img = assets::load_image(path, &self.base_dir)?;
                e::from_rgba8(img.width, img.height, &img.rgba, true)
            }
        };
        let filtered = e::prefilter(base);
        let view = env_texture(&self.device, &self.queue, &filtered.levels);
        self.environment = Some(GpuEnvironment {
            source,
            view,
            sh: filtered.sh,
        });
        self.rebuild_lights_group();
        Ok(())
    }

    fn rebuild_lights_group(&mut self) {
        let env = self
            .environment
            .as_ref()
            .map_or(&self.no_environment, |g| &g.view);
        self.lights_group = lights_bind_group(
            &self.device,
            &self.lights_layout,
            &self.lights_buffer,
            [&self.shadow_map.1, &self.local_shadows.array, env],
            [&self.shadow_sampler, &self.env_sampler],
        );
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
                    node: p.node,
                    deform: crate::skinning::Deform::upload(&self.device, p, model.rig.skins.len()),
                }
            })
            .collect();
        let skins = model
            .rig
            .skins
            .iter()
            .map(|s| crate::skinning::GpuSkin {
                // Joints outside the node list (a broken file) fall back to the rest pose.
                joints: s.joints.clone(),
                inverse_bind: s.inverse_bind.iter().map(Mat4::from_cols_array).collect(),
            })
            .collect();
        let rest = model.rig.rest_pose().iter().map(Mat4::from_cols_array).collect();
        GpuModel {
            parts,
            materials,
            skins,
            rest,
            rest_morph: model.rig.morph_weights.clone(),
        }
    }

    /// Records one view of the frame into `encoder`. Call [`Renderer::prepare`] first.
    /// Uploads with `queue.write_buffer`, so submit before rendering the next frame.
    pub fn render(
        &mut self,
        frame: &FrameData,
        view: &View,
        target: &Target,
        encoder: &mut wgpu::CommandEncoder,
    ) -> RenderStats {
        self.render_views(frame, &[(*view, target)], encoder)
    }

    /// Records every view of the frame (two for stereo) into `encoder`, one render pass per
    /// view. Per-frame work (instance data, sprite and UI geometry, lights) is done once and
    /// shared by all views; only the view uniforms differ. Call [`Renderer::prepare`] first,
    /// and submit before rendering the next frame. At most [`MAX_VIEWS`] views.
    pub fn render_views(
        &mut self,
        frame: &FrameData,
        views: &[(View, &Target)],
        encoder: &mut wgpu::CommandEncoder,
    ) -> RenderStats {
        assert!(views.len() <= MAX_VIEWS, "at most {MAX_VIEWS} views per frame");
        let mut stats = RenderStats {
            views: views.len() as u32,
            ..Default::default()
        };

        // Per-view data: world view in slot 2i, UI (screen pixels) in slot 2i+1.
        let mut view_bytes = vec![0u8; (views.len() * 2) * VIEW_STRIDE as usize];
        for (i, (view, target)) in views.iter().enumerate() {
            let vp = view.proj * view.view;
            let vu = ViewUniform {
                view_proj: mat(vp),
                eye: view.eye.extend(1.0).to_array(),
                inv_view_proj: mat(vp.inverse()),
            };
            let (sw, sh) = (target.width as f32, target.height as f32);
            let ui_proj = glam::camera::rh::proj::directx::orthographic(0.0, sw, sh, 0.0, -1.0, 1.0);
            let uvu = ViewUniform {
                view_proj: mat(ui_proj),
                eye: [0.0, 0.0, 0.0, 1.0],
                inv_view_proj: mat(ui_proj.inverse()),
            };
            let at = 2 * i * VIEW_STRIDE as usize;
            view_bytes[at..at + size_of::<ViewUniform>()].copy_from_slice(bytemuck::bytes_of(&vu));
            let at = at + VIEW_STRIDE as usize;
            view_bytes[at..at + size_of::<ViewUniform>()].copy_from_slice(bytemuck::bytes_of(&uvu));
        }
        if !view_bytes.is_empty() {
            self.queue.write_buffer(&self.view_buffer, 0, &view_bytes);
        }

        // Per-frame lights.
        let mut lu = LightsUniform::zeroed();
        for (i, l) in frame.lights.iter().take(MAX_LIGHTS).enumerate() {
            lu.direction[i] = l.direction.extend(0.0).to_array();
            lu.radiance[i] = l.radiance.extend(0.0).to_array();
        }
        lu.ambient = frame.ambient.extend(0.0).to_array();
        lu.count[0] = frame.lights.len().min(MAX_LIGHTS) as u32;
        for (i, l) in frame.local_lights.iter().take(MAX_LOCAL_LIGHTS).enumerate() {
            lu.local_pos[i] = l.position.extend(l.range).to_array();
            lu.local_rad[i] = l.radiance.extend(l.cos_outer).to_array();
            lu.local_dir[i] = l.direction.extend(l.cos_inner).to_array();
        }
        lu.count[1] = frame.local_lights.len().min(MAX_LOCAL_LIGHTS) as u32;
        let mut flags = 0;
        if let Some((sky, ground)) = frame.sky {
            flags |= FLAG_SKY;
            lu.ambient = sky.extend(0.0).to_array();
            lu.ground = ground.extend(0.0).to_array();
        }
        if let Some(exposure) = frame.exposure {
            flags |= FLAG_TONEMAP;
            lu.post[0] = exposure;
        }
        if let Some((color, density)) = frame.fog {
            flags |= FLAG_FOG;
            lu.fog = color.extend(density).to_array();
        }
        // Shadows: the first directional light, seen from the first view.
        let shadow = match (frame.shadows, frame.lights.first(), views.first()) {
            (Some(s), Some(light), Some((view, _))) if s.distance > 0.0 => {
                let resolution = s.resolution.clamp(256, 4096);
                let vp = shadow_view_proj(light.direction, view, s.distance, resolution);
                let texel = 2.0 * s.distance / resolution as f32;
                lu.shadow_view_proj = mat(vp);
                lu.post[1] = 1.0 / resolution as f32;
                lu.post[2] = s.softness.clamp(0.0, 4.0);
                lu.post[3] = texel * 1.5;
                flags |= FLAG_SHADOWS;
                if self.shadow_map.0 != resolution {
                    self.shadow_map = (resolution, shadow_texture(&self.device, resolution));
                    self.rebuild_lights_group();
                }
                let at = 2 * MAX_VIEWS * VIEW_STRIDE as usize;
                let su = ViewUniform {
                    view_proj: mat(vp),
                    eye: [0.0, 0.0, 0.0, 1.0],
                    inv_view_proj: mat(vp.inverse()),
                };
                self.queue
                    .write_buffer(&self.view_buffer, at as u64, bytemuck::bytes_of(&su));
                true
            }
            _ => false,
        };
        // Point and spot light shadows (decision 0065): layers in light order until the
        // budget runs out; a point light needs six (one per cube face), a spot light one.
        let mut layer_mats: Vec<Mat4> = Vec::new();
        let mut layer_res = 0u32;
        for (j, l) in frame.local_lights.iter().take(MAX_LOCAL_LIGHTS).enumerate() {
            lu.local_shadow[j][0] = -1.0;
            let Some((resolution, softness)) = l.shadow else {
                continue;
            };
            let point = l.cos_outer < -1.0;
            let faces = local_shadow_view_projs(l, resolution);
            if layer_mats.len() + faces.len() > MAX_LOCAL_SHADOW_LAYERS {
                continue;
            }
            // Normal offset per meter of distance: about one texel of the map there.
            let half_tan = if point {
                1.0
            } else {
                l.cos_outer.clamp(0.01, 1.0).acos().tan()
            };
            lu.local_shadow[j] = [
                layer_mats.len() as f32,
                2.0 * half_tan / resolution as f32,
                0.0,
                softness,
            ];
            layer_mats.extend(faces);
            layer_res = layer_res.max(resolution);
        }
        for ls in lu.local_shadow.iter_mut().filter(|ls| ls[0] >= 0.0) {
            ls[2] = 1.0 / layer_res as f32;
        }
        for (k, m) in layer_mats.iter().enumerate() {
            lu.shadow_mats[k] = mat(*m);
        }
        if layer_mats.is_empty() && self.local_shadows.resolution > 1 {
            // No light casts any more: give the maps back.
            self.local_shadows = local_shadow_maps(&self.device, 1, 1);
            self.rebuild_lights_group();
        }
        if !layer_mats.is_empty() {
            let layers = layer_mats.len() as u32;
            if self.local_shadows.resolution != layer_res || self.local_shadows.layers < layers {
                self.local_shadows = local_shadow_maps(&self.device, layer_res, layers);
                self.rebuild_lights_group();
            }
            let mut bytes = vec![0u8; layer_mats.len() * VIEW_STRIDE as usize];
            for (k, m) in layer_mats.iter().enumerate() {
                let u = ViewUniform {
                    view_proj: mat(*m),
                    eye: [0.0, 0.0, 0.0, 1.0],
                    inv_view_proj: mat(m.inverse()),
                };
                let at = k * VIEW_STRIDE as usize;
                bytes[at..at + size_of::<ViewUniform>()].copy_from_slice(bytemuck::bytes_of(&u));
            }
            self.queue
                .write_buffer(&self.view_buffer, LOCAL_SHADOW_SLOT * VIEW_STRIDE, &bytes);
        }
        // Environment (built in `prepare`).
        let env = match (&frame.environment, &self.environment) {
            (Some(e), Some(g)) => {
                flags |= FLAG_ENV;
                lu.env = [e.intensity, (crate::environment::LEVELS - 1) as f32, 0.0, 0.0];
                lu.sh = g.sh;
                e.background
            }
            _ => false,
        };
        // Bloom: models go to the HDR target untone-mapped; the clear and fog colors are
        // mapped back through the tone curve so they come out unchanged (decision 0064).
        let bloom = frame.bloom;
        let hdr_color = |c: Vec3| match frame.exposure {
            Some(e) => {
                Vec3::new(
                    crate::post::untonemap(c.x),
                    crate::post::untonemap(c.y),
                    crate::post::untonemap(c.z),
                ) / e.max(1e-4)
            }
            None => c,
        };
        if bloom.is_some() {
            flags |= FLAG_HDR;
            if flags & FLAG_FOG != 0 {
                let f = hdr_color(Vec3::from_slice(&lu.fog[..3]));
                lu.fog = f.extend(lu.fog[3]).to_array();
            }
            let sizes: Vec<(u32, u32)> = views.iter().map(|(_, t)| (t.width, t.height)).collect();
            self.post.prepare(&self.device, &sizes);
        } else {
            // Frees the HDR targets of earlier frames with bloom.
            self.post.prepare(&self.device, &[]);
        }
        lu.count[2] = flags;
        self.queue
            .write_buffer(&self.lights_buffer, 0, bytemuck::bytes_of(&lu));

        // Instances, grouped by (model, part, material source) so every group is one
        // instanced draw. Groups are kept in a stable order (BTreeMap key) and instances
        // within a group in entity order, so frames are reproducible.
        // Pass 1 writes every instance once, in frame order, with its group number; pass 2
        // scatters them into one buffer ordered by group key. The model lookup and the group
        // lookup are skipped while the key repeats (entities come sorted by id, so runs of
        // the same model are the common case).
        // (asset, part, material overridden, holds a shadow-casting lamp)
        type Key<'k> = (&'k str, usize, bool, bool);
        let mut group_of: BTreeMap<Key, u32> = BTreeMap::new();
        // Scratch arrays live across frames: fresh multi-megabyte allocations every frame
        // cost more in page faults than filling them does.
        let (mut raw, mut gid, mut instances) = std::mem::take(&mut self.scratch);
        raw.clear();
        gid.clear();
        let mut last_model: Option<(&str, &GpuModel)> = None;
        let mut last_key: Option<(Key, u32)> = None;
        for draw in &frame.meshes {
            let model = match last_model {
                Some((name, m))
                    if std::ptr::eq(name.as_ptr(), draw.asset.as_ptr()) || name == &*draw.asset =>
                {
                    m
                }
                _ => match self.models.get(&*draw.asset) {
                    Some(Ok(m)) => {
                        last_model = Some((&*draw.asset, m));
                        m
                    }
                    _ => continue,
                },
            };
            for (p, part) in model.parts.iter().enumerate() {
                if part.deform.is_some() {
                    continue; // skinned or morphed: drawn by the deforming pass
                }
                // An animated pose moves parts hanging from animated nodes (props on bones).
                let local = match (&draw.pose, part.node) {
                    (Some(pose), Some(n)) if n < pose.nodes.len() => Mat4::from_cols_array(&pose.nodes[n]),
                    _ => part.transform,
                };
                let model_m = draw.world * local;
                let (base_color, params) = instance_material(model, part, draw.material);
                let key = (&*draw.asset, p, draw.material.is_some(), draw.lamp);
                let g = match last_key {
                    Some((k, g)) if k == key => g,
                    _ => {
                        let n = group_of.len() as u32;
                        let g = *group_of.entry(key).or_insert(n);
                        last_key = Some((key, g));
                        g
                    }
                };
                raw.push(InstanceData {
                    model: mat(model_m),
                    base_color,
                    params,
                    emissive: draw.emissive.extend(0.0).to_array(),
                });
                gid.push(g);
            }
        }
        for bulk in &frame.bulk {
            let Some(Ok(model)) = self.models.get(&bulk.asset) else {
                continue;
            };
            for (p, part) in model.parts.iter().enumerate() {
                let (base_color, params) = match bulk.material {
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
                let key = (bulk.asset.as_str(), p, bulk.material.is_some(), false);
                let n = group_of.len() as u32;
                let g = *group_of.entry(key).or_insert(n);
                let identity = part.transform == Mat4::IDENTITY;
                raw.reserve(bulk.transforms.len());
                gid.reserve(bulk.transforms.len());
                for t in &bulk.transforms {
                    let world = Mat4::from_cols_array(&[
                        t[0], t[1], t[2], 0.0, t[3], t[4], t[5], 0.0, t[6], t[7], t[8], 0.0, t[9], t[10],
                        t[11], 1.0,
                    ]);
                    raw.push(InstanceData {
                        model: mat(if identity { world } else { world * part.transform }),
                        base_color,
                        params,
                        emissive: [0.0; 4],
                    });
                    gid.push(g);
                }
            }
        }
        // Group order = key order; counts give each group's start in the final buffer.
        let mut counts = vec![0u32; group_of.len()];
        for &g in &gid {
            counts[g as usize] += 1;
        }
        let mut start_of = vec![0u32; group_of.len()];
        let mut calls = Vec::with_capacity(group_of.len());
        let mut at = 0u32;
        for (&(asset, p, overridden, lamp), &g) in &group_of {
            let Some(Ok(model)) = self.models.get(asset) else {
                continue;
            };
            let part = &model.parts[p];
            let material = if overridden {
                &self.plain_material
            } else {
                &model.materials[part.material.min(model.materials.len() - 1)].0
            };
            let n = counts[g as usize];
            start_of[g as usize] = at;
            stats.instances += n;
            stats.triangles += (part.index_count / 3) as u64 * n as u64;
            calls.push(DrawCall {
                lamp,
                part,
                material,
                instances: at..at + n,
            });
            at += n;
        }
        instances.clear();
        instances.resize(raw.len(), InstanceData::zeroed());
        let mut next = start_of;
        for (inst, &g) in raw.iter().zip(&gid) {
            instances[next[g as usize] as usize] = *inst;
            next[g as usize] += 1;
        }
        if !instances.is_empty() {
            write_growable(
                &self.device,
                &self.queue,
                &mut self.instance_buffer,
                &instances,
                wgpu::BufferUsages::VERTEX,
            );
        }

        // Skinned parts: their own instanced draws, one joint palette per character.
        let built = crate::skinning::build(frame, &self.models, &self.plain_material);
        let skinned_calls = built.calls;
        let deform_groups = self.skinning.upload(
            &self.device,
            &self.queue,
            &built.palettes,
            &built.morph_weights,
            &built.instances,
            &skinned_calls,
        );
        for call in &skinned_calls {
            let n = call.instances.end - call.instances.start;
            stats.instances += n;
            stats.triangles += (call.part.index_count / 3) as u64 * n as u64;
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

        // UI: quads in screen pixels (y down), batched by texture like sprites. Layout depends
        // on the target size, so each view gets its own batches in one shared buffer.
        let mut uv: Vec<SpriteVertex> = Vec::new();
        let mut ui_idx: Vec<u32> = Vec::new();
        let mut ui_batches: Vec<Vec<(&str, std::ops::Range<u32>)>> = Vec::with_capacity(views.len());
        for (_, target) in views {
            let (sw, sh) = (target.width as f32, target.height as f32);
            let mut view_batches: Vec<(&str, std::ops::Range<u32>)> = Vec::new();
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
                match view_batches.last_mut() {
                    Some((img, range)) if *img == image => range.end = end,
                    _ => view_batches.push((image, start..end)),
                }
            }
            ui_batches.push(view_batches);
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

        self.world_quads.upload(&self.device, &self.queue, &frame.quads);

        let meshes = MeshCalls {
            calls: &calls,
            instances: self.instance_buffer.as_ref(),
            skinned: &skinned_calls,
            deform_groups: &deform_groups,
            skinned_instances: self.skinning.instances.as_ref(),
        };
        let depth_pipelines = (&self.shadow_pipeline, &self.skinning.shadow_pipeline);
        let depth_pass = |encoder: &mut wgpu::CommandEncoder,
                          view: &wgpu::TextureView,
                          offset: u32,
                          local: bool,
                          stats: &mut RenderStats| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            meshes.draw_depth(&mut pass, depth_pipelines, &self.view_group, offset, local, stats);
        };
        if shadow {
            let offset = (2 * MAX_VIEWS as u64 * VIEW_STRIDE) as u32;
            depth_pass(encoder, &self.shadow_map.1, offset, false, &mut stats);
        }
        for (k, layer) in self
            .local_shadows
            .per_layer
            .iter()
            .take(layer_mats.len())
            .enumerate()
        {
            let offset = ((LOCAL_SHADOW_SLOT + k as u64) * VIEW_STRIDE) as u32;
            depth_pass(encoder, layer, offset, true, &mut stats);
        }

        for (i, (view, target)) in views.iter().enumerate() {
            let world_offset = (2 * i as u64 * VIEW_STRIDE) as u32;
            let ui_offset = world_offset + VIEW_STRIDE as u32;
            let c = view.clear;
            let clear = |c: Vec3, a: f32| wgpu::Color {
                r: c.x as f64,
                g: c.y as f64,
                b: c.z as f64,
                a: a as f64,
            };
            let rgb = Vec3::new(c[0], c[1], c[2]);
            // With bloom, models (and the sky) go to the HDR target first; the final pass
            // then starts with the composite and keeps the depth for sprites.
            let post = bloom.map(|(threshold, intensity)| {
                let t = self.post.targets((target.width, target.height));
                self.post.write(
                    &self.queue,
                    threshold,
                    intensity,
                    frame.exposure,
                    (target.width, target.height),
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("hdr"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: t.msaa.as_ref().unwrap_or(&t.hdr),
                        depth_slice: None,
                        resolve_target: t.msaa.as_ref().map(|_| &t.hdr),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear(hdr_color(rgb), c[3])),
                            store: if t.msaa.is_some() {
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
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                meshes.draw_lit(
                    &mut pass,
                    (&self.pbr_hdr_pipeline, &self.skinning.hdr_pipeline),
                    &self.view_group,
                    world_offset,
                    &self.lights_group,
                    &mut stats,
                );
                if env {
                    pass.set_pipeline(&self.sky_pipelines[1]);
                    pass.set_bind_group(0, &self.view_group, &[world_offset]);
                    pass.set_bind_group(1, &self.lights_group, &[]);
                    pass.draw(0..3, 0..1);
                    stats.draw_calls += 1;
                }
                drop(pass);
                stats.draw_calls += self.post.glow(t, encoder);
                t
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.color,
                    depth_slice: None,
                    resolve_target: target.resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear(rgb, c[3])),
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
                        load: if post.is_some() {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        },
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            match post {
                Some(t) => {
                    self.post.composite(t, &mut pass);
                    stats.draw_calls += 1;
                }
                None => {
                    meshes.draw_lit(
                        &mut pass,
                        (&self.pbr_pipeline, &self.skinning.pipeline),
                        &self.view_group,
                        world_offset,
                        &self.lights_group,
                        &mut stats,
                    );
                    if env {
                        pass.set_pipeline(&self.sky_pipelines[0]);
                        pass.set_bind_group(0, &self.view_group, &[world_offset]);
                        pass.set_bind_group(1, &self.lights_group, &[]);
                        pass.draw(0..3, 0..1);
                        stats.draw_calls += 1;
                    }
                }
            }
            if let (Some(vb), Some(ib), false) =
                (&self.sprite_vertices, &self.sprite_indices, batches.is_empty())
            {
                pass.set_pipeline(&self.sprite_pipeline);
                pass.set_bind_group(0, &self.view_group, &[world_offset]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                for (image, range) in &batches {
                    if let Some(Ok(tex)) = self.sprite_textures.get(*image) {
                        pass.set_bind_group(1, &tex.bind_group, &[]);
                        pass.draw_indexed(range.clone(), 0, 0..1);
                        stats.draw_calls += 1;
                    }
                }
            }
            stats.draw_calls += self
                .world_quads
                .draw(&mut pass, &self.view_group, world_offset, |image| {
                    self.sprite_textures
                        .get(image)
                        .and_then(|t| t.as_ref().ok())
                        .map(|t| &t.bind_group)
                });
            if let (Some(vb), Some(ib), false) =
                (&self.ui_vertices, &self.ui_indices, ui_batches[i].is_empty())
            {
                pass.set_pipeline(&self.ui_pipeline);
                pass.set_bind_group(0, &self.view_group, &[ui_offset]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                for (image, range) in &ui_batches[i] {
                    if let Some(Ok(tex)) = self.sprite_textures.get(*image) {
                        pass.set_bind_group(1, &tex.bind_group, &[]);
                        pass.draw_indexed(range.clone(), 0, 0..1);
                        stats.draw_calls += 1;
                    }
                }
            }
        }
        drop(calls);
        drop(skinned_calls);
        drop(deform_groups);
        self.scratch = (raw, gid, instances);
        stats
    }
}

/// Depth state of the shadow pass: biased against self-shadowing ("acne").
pub(crate) fn shadow_depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: SHADOW_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 2.0,
            clamp: 0.0,
        },
    }
}

fn shadow_texture(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// Lights group: uniform, sun shadow map, comparison sampler, local shadow maps,
/// environment texture and its sampler.
fn lights_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    [shadow, local, env]: [&wgpu::TextureView; 3],
    [compare, env_sampler]: [&wgpu::Sampler; 2],
) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    let sampler = |binding, sampler| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Sampler(sampler),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lights"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer_resource(buffer, size_of::<LightsUniform>() as u64),
            },
            view(1, shadow),
            sampler(2, compare),
            view(3, local),
            view(4, env),
            sampler(5, env_sampler),
        ],
    })
}

/// A depth texture array of `layers` square maps of `resolution` texels.
fn local_shadow_maps(device: &wgpu::Device, resolution: u32, layers: u32) -> LocalShadowMaps {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("local shadow maps"),
        size: wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let array = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let per_layer = (0..layers)
        .map(|i| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: i,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    LocalShadowMaps {
        resolution,
        layers,
        array,
        per_layer,
    }
}

/// Uploads prefiltered environment levels (finest first) as one half-float texture.
fn env_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    levels: &[crate::environment::EnvImage],
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("environment"),
        size: wgpu::Extent3d {
            width: levels[0].w as u32,
            height: levels[0].h as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, level) in levels.iter().enumerate() {
        let data: Vec<u16> = level
            .px
            .iter()
            .flat_map(|c| {
                use crate::environment::f16_bits;
                [f16_bits(c.x), f16_bits(c.y), f16_bits(c.z), f16_bits(1.0)]
            })
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
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
    texture.create_view(&Default::default())
}

/// View-projection of the shadow-casting light: an orthographic box `distance` meters each
/// side of a point ahead of the camera, snapped to whole shadow texels so edges do not
/// shimmer as the camera moves.
pub(crate) fn shadow_view_proj(light_dir: Vec3, view: &View, distance: f32, resolution: u32) -> Mat4 {
    let d = light_dir.normalize_or(Vec3::NEG_Y);
    let forward = -view.view.inverse().z_axis.truncate().normalize_or(Vec3::NEG_Z);
    let center = view.eye + forward * distance * 0.5;
    let up = if d.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let rot = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, d, up);
    let c = rot.transform_point3(center);
    let texel = 2.0 * distance / resolution as f32;
    let (sx, sy) = ((c.x / texel).floor() * texel, (c.y / texel).floor() * texel);
    // Room behind the area for tall casters, in front for the area's depth.
    let depth = distance * 4.0;
    let light_view = Mat4::from_translation(-Vec3::new(sx, sy, c.z + depth)) * rot;
    let proj = glam::camera::rh::proj::directx::orthographic(
        -distance,
        distance,
        -distance,
        distance,
        0.0,
        2.0 * depth,
    );
    proj * light_view
}

/// View-projections of a local light's shadow maps (decision 0065): six 90-degree faces
/// for a point light (+x, -x, +y, -y, +z, -z, matching `local_shadow_factor` in
/// `pbr.wgsl`), one cone-wide perspective for a spot light. Faces are widened by two texels
/// so the 3x3 filter does not run off their edges.
pub(crate) fn local_shadow_view_projs(light: &crate::extract::LocalLightData, resolution: u32) -> Vec<Mat4> {
    let near = (light.range * 0.002).clamp(0.02, 0.1);
    let pad = 1.0 + 4.0 / resolution as f32;
    let eye = light.position;
    let look = |dir: Vec3, up: Vec3, half_tan: f32| {
        let proj = glam::camera::rh::proj::directx::perspective(
            2.0 * (half_tan * pad).atan(),
            1.0,
            near,
            light.range,
        );
        proj * glam::camera::rh::view::look_at_mat4(eye, eye + dir, up)
    };
    if light.cos_outer < -1.0 {
        [
            (Vec3::X, Vec3::Y),
            (Vec3::NEG_X, Vec3::Y),
            (Vec3::Y, Vec3::Z),
            (Vec3::NEG_Y, Vec3::Z),
            (Vec3::Z, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y),
        ]
        .into_iter()
        .map(|(d, up)| look(d, up, 1.0))
        .collect()
    } else {
        let d = light.direction.normalize_or(Vec3::NEG_Y);
        let up = if d.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let half_tan = light.cos_outer.clamp(0.01, 1.0).acos().tan();
        vec![look(d, up, half_tan)]
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
    let e = m.emissive;
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("material"),
        contents: bytemuck::bytes_of(&MaterialUniform {
            emissive: [e[0].max(0.0), e[1].max(0.0), e[2].max(0.0), 0.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            tex(1, pick(m.base_color_texture, 0)),
            tex(2, pick(m.metallic_roughness_texture, 1)),
            tex(3, pick(m.normal_texture, 2)),
            tex(5, pick(m.emissive_texture, 0)),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Writes `data` into `slot`, recreating the buffer when it is too small.
pub(crate) fn write_growable<T: Pod>(
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
