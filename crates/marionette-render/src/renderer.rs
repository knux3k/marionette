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

mod game_mesh;
mod game_shader;
pub(crate) mod variants;

/// Anisotropic filtering of model textures (decision 0280).
pub const MATERIAL_ANISOTROPY: u16 = 16;
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
/// Ray-traced shadows and reflections (decision 0093).
const FLAG_RT_SHADOWS: u32 = 64;
const FLAG_RT_REFLECT: u32 = 128;
/// Reflection probes (decision 0092).
const FLAG_PROBES: u32 = 256;
/// Shadow maps for point and spot lights per frame (a point light takes six).
pub const MAX_LOCAL_SHADOW_LAYERS: usize = 24;
/// Dynamic uniform offsets must be multiples of this (WebGPU minimum alignment).
const VIEW_STRIDE: u64 = 256;
/// Most sun shadow cascades (decision 0090).
pub const MAX_CASCADES: usize = 4;
/// View buffer slots: world and UI per view, the sun's cascades, local shadow layers.
const VIEW_SLOTS: u64 = 2 * MAX_VIEWS as u64 + MAX_CASCADES as u64 + MAX_LOCAL_SHADOW_LAYERS as u64;
/// First view buffer slot of the sun's shadow cascades.
const SUN_SHADOW_SLOT: u64 = 2 * MAX_VIEWS as u64;
/// First view buffer slot of the local light shadow layers.
const LOCAL_SHADOW_SLOT: u64 = SUN_SHADOW_SLOT + MAX_CASCADES as u64;
/// Most views one [`Renderer::render_views`] call can draw (two for stereo; more for
/// split screen or mirrors).
pub const MAX_VIEWS: usize = 8;

pub mod codes {
    pub const ASSET: &str = "render.asset";
    pub const NO_CAMERA: &str = "render.no_camera";
    pub const NO_ADAPTER: &str = "render.no_adapter";
    pub const DEVICE: &str = "render.device";
    pub const SURFACE: &str = "render.surface";
    /// A game shader failed to load or compile, or its material is wrong (decision 0181).
    pub const SHADER: &str = "render.shader";
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
    /// Directional lights, local lights, flags, sun shadow cascades.
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
    /// Cascades 1 to 3 of the sun's shadow (cascade 0 is `shadow_view_proj`).
    cascade_mats: [[[f32; 4]; 4]; MAX_CASCADES - 1],
    /// Normal offset (m) per cascade.
    cascade_offsets: [f32; 4],
    /// Ray tracing: reflection roughness limit, tangent of the sun disc's radius.
    rt: [f32; 4],
    /// Reflection probes: center (w = 1 when captured) and box half size.
    probe_pos: [[f32; 4]; crate::extract::MAX_PROBES],
    probe_ext: [[f32; 4]; crate::extract::MAX_PROBES],
}

/// Per-material uniform (group 2, binding 0).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    /// rgb = emissive factor, w = alpha cutoff of a cut-out material (0: none).
    emissive: [f32; 4],
}

/// Shadow maps (point and spot lights, or the sun's cascades): one depth texture array, one
/// layer per map.
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
#[derive(Clone)]
pub(crate) struct DrawCall<'a> {
    /// The model shares its entity with a shadow-casting point or spot light: it does not
    /// cast into local light shadow maps (a lantern would otherwise block its own light).
    pub lamp: bool,
    pub part: &'a GpuPart,
    pub material: &'a wgpu::BindGroup,
    /// Index buffer and count of the level drawn (full detail or a reduced level).
    pub indices: (&'a wgpu::Buffer, u32),
    /// All instances (shadow maps draw them all) ...
    pub instances: std::ops::Range<u32>,
    /// ... of which the first `seen` are inside a view (views draw only those).
    pub seen: u32,
    /// A game shader and its material group (decision 0181).
    pub game: Option<game_shader::GameDraw<'a>>,
    /// Sides, blending and cut-out of the material (decision 0280).
    pub look: variants::Look,
}

/// Which of a frame's model draws a pass makes (decision 0280): opaque and cut-out ones, then
/// (after the sky) see-through ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Opaque,
    Blended,
}

/// Pipelines of one pass over the models.
#[derive(Clone, Copy)]
enum Pipes<'a> {
    /// Depth only: static parts, cut-out static parts (`None`: leave them out), skinned parts.
    Depth(
        &'a wgpu::RenderPipeline,
        Option<&'a wgpu::RenderPipeline>,
        &'a wgpu::RenderPipeline,
    ),
    /// Lit, per material variant: static, skinned; and whether the target is the HDR one.
    Lit(
        &'a variants::VariantPipelines,
        &'a variants::VariantPipelines,
        bool,
    ),
}

/// Everything a frame draws for models, shared by the shadow, HDR and final passes.
struct MeshCalls<'a> {
    calls: &'a [DrawCall<'a>],
    /// `calls[..first_blended]` are opaque or cut out; the rest are see-through.
    first_blended: usize,
    instances: Option<&'a wgpu::Buffer>,
    skinned: &'a [crate::skinning::SkinnedCall<'a>],
    deform_groups: &'a [wgpu::BindGroup],
    skinned_instances: Option<&'a wgpu::Buffer>,
    /// Empty bind group for groups 1 and 2 of game shader depth pipelines.
    empty_group: Option<&'a wgpu::BindGroup>,
    /// The standard lights group, which game shader pipelines use (decision 0181).
    game_lights: Option<&'a wgpu::BindGroup>,
    /// Empty group 1 of the cut-out shadow pipeline (decision 0280).
    masked_empty: &'a wgpu::BindGroup,
}

impl MeshCalls<'_> {
    /// Depth only (shadow maps): group 0 at `offset` holds the light's view-projection.
    #[allow(clippy::too_many_arguments)]
    fn draw_depth(
        &self,
        pass: &mut wgpu::RenderPass,
        pipelines: (
            &wgpu::RenderPipeline,
            &wgpu::RenderPipeline,
            &wgpu::RenderPipeline,
        ),
        view_group: &wgpu::BindGroup,
        offset: u32,
        local: bool,
        stats: &mut RenderStats,
    ) {
        let (plain, masked, skinned) = pipelines;
        self.draw(
            pass,
            Pipes::Depth(plain, Some(masked), skinned),
            (view_group, offset),
            (None, local, false, Phase::Opaque),
            stats,
        );
    }

    /// Depth of what the view draws (decision 0093 prepass): only the opaque instances in
    /// view; cut-outs draw their own depth in the lit pass.
    fn draw_prepass(
        &self,
        pass: &mut wgpu::RenderPass,
        (plain, skinned): (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        view_group: &wgpu::BindGroup,
        offset: u32,
        stats: &mut RenderStats,
    ) {
        self.draw(
            pass,
            Pipes::Depth(plain, None, skinned),
            (view_group, offset),
            (None, false, true, Phase::Opaque),
            stats,
        );
    }

    /// Lit: groups 0 (view at `offset`), 1 (lights) and 2 (material); `hdr` picks pipelines
    /// for the bloom target.
    #[allow(clippy::too_many_arguments)]
    fn draw_lit(
        &self,
        pass: &mut wgpu::RenderPass,
        pipelines: (&variants::VariantPipelines, &variants::VariantPipelines),
        view_group: &wgpu::BindGroup,
        offset: u32,
        lights: &wgpu::BindGroup,
        (hdr, phase): (bool, Phase),
        stats: &mut RenderStats,
    ) {
        self.draw(
            pass,
            Pipes::Lit(pipelines.0, pipelines.1, hdr),
            (view_group, offset),
            (Some(lights), false, true, phase),
            stats,
        );
    }

    fn draw(
        &self,
        pass: &mut wgpu::RenderPass,
        pipes: Pipes,
        (view_group, offset): (&wgpu::BindGroup, u32),
        // Lights group (lit draws), or none (depth); skip models holding a lamp; draw only
        // the instances inside a view (views) or every caster (shadow maps); which draws.
        (lights, skip_lamps, in_view, phase): (Option<&wgpu::BindGroup>, bool, bool, Phase),
        stats: &mut RenderStats,
    ) {
        let calls = match phase {
            Phase::Opaque => &self.calls[..self.first_blended.min(self.calls.len())],
            Phase::Blended => &self.calls[self.first_blended.min(self.calls.len())..],
        };
        let hdr = matches!(pipes, Pipes::Lit(_, _, true));
        if let (Some(ib), false) = (self.instances, calls.is_empty()) {
            let mut current: *const wgpu::RenderPipeline = std::ptr::null();
            pass.set_bind_group(0, view_group, &[offset]);
            let mut bound_lights: *const wgpu::BindGroup = std::ptr::null();
            if let Some(l) = lights {
                pass.set_bind_group(1, l, &[]);
                bound_lights = l;
            }
            pass.set_vertex_buffer(1, ib.slice(..));
            for call in calls.iter().filter(|c| !(skip_lamps && c.lamp)) {
                // Views draw the instances inside them; shadow maps draw every caster.
                let range = if in_view {
                    call.instances.start..call.instances.start + call.seen
                } else {
                    call.instances.clone()
                };
                if range.is_empty() {
                    continue;
                }
                // The standard pipeline for this call's material, or none (left out).
                let standard = match pipes {
                    Pipes::Lit(pbr, _, hdr) => Some(pbr.get(call.look.variant as usize, hdr)),
                    Pipes::Depth(_, masked, _) if call.look.masked && call.game.is_none() => masked,
                    Pipes::Depth(plain, _, _) => Some(plain),
                };
                // Game shaders (decision 0181): their own pipelines; blended or
                // `no_shadows` ones do not cast.
                // A depth prepass (decision 0093) keeps opaque game shaders whose vertices do not
                // move; the others draw their own depth in the lit pass.
                let pipeline = match (lights.is_some(), call.game, standard) {
                    (true, Some((g, _)), _) => &g.lit[hdr as usize],
                    (false, Some((g, _)), Some(plain)) if in_view => {
                        if g.options.blend != marionette_core::shader::ShaderBlend::Opaque
                            || g.shadow.is_some()
                        {
                            continue;
                        }
                        plain
                    }
                    (false, Some((g, _)), _) if !g.options.casts_shadows() => continue,
                    (false, Some((g, _)), Some(plain)) => g.shadow.as_ref().unwrap_or(plain),
                    (_, _, Some(p)) => p,
                    (_, _, None) => continue,
                };
                if !std::ptr::eq(current, pipeline) {
                    pass.set_pipeline(pipeline);
                    current = pipeline;
                }
                if let Some(l) = lights {
                    pass.set_bind_group(2, call.material, &[]);
                    // Game shaders are built against the standard lights group; ray-traced
                    // frames bind their own, so swap it around game draws.
                    let want = match (call.game, self.game_lights) {
                        (Some(_), Some(std)) => std,
                        _ => l,
                    };
                    if !std::ptr::eq(bound_lights, want) {
                        pass.set_bind_group(1, want, &[]);
                        bound_lights = want;
                    }
                } else if call.look.masked && call.game.is_none() {
                    // Cut-out shadows read the material's base color (decision 0280).
                    pass.set_bind_group(1, self.masked_empty, &[]);
                    pass.set_bind_group(2, call.material, &[]);
                }
                if let Some((g, group)) = call.game {
                    if lights.is_none()
                        && g.shadow.is_some()
                        && let Some(empty) = self.empty_group
                    {
                        pass.set_bind_group(1, empty, &[]);
                        pass.set_bind_group(2, empty, &[]);
                    }
                    pass.set_bind_group(3, group, &[]);
                }
                pass.set_vertex_buffer(0, call.part.vertices.slice(..));
                pass.set_index_buffer(call.indices.0.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..call.indices.1, 0, range);
                stats.draw_calls += 1;
            }
        }
        let skinned_calls = self
            .skinned
            .iter()
            .zip(self.deform_groups)
            .filter(|(c, _)| c.look.blended() == (phase == Phase::Blended))
            // The prepass leaves cut-outs to the lit pass.
            .filter(|(c, _)| !(c.look.masked && matches!(pipes, Pipes::Depth(_, None, _))));
        if let (Some(ib), false) = (self.skinned_instances, self.skinned.is_empty()) {
            let mut current: *const wgpu::RenderPipeline = std::ptr::null();
            let mut started = false;
            for (call, group) in skinned_calls {
                let Some(deform) = &call.part.deform else {
                    continue;
                };
                let pipeline = match pipes {
                    Pipes::Lit(_, skinned, hdr) => skinned.get(call.look.variant as usize, hdr),
                    Pipes::Depth(_, _, skinned) => skinned,
                };
                if !started {
                    pass.set_bind_group(0, view_group, &[offset]);
                    if let Some(l) = lights {
                        pass.set_bind_group(1, l, &[]);
                    }
                    pass.set_vertex_buffer(1, ib.slice(..));
                    started = true;
                }
                if !std::ptr::eq(current, pipeline) {
                    pass.set_pipeline(pipeline);
                    current = pipeline;
                }
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
    /// Mesh instances not drawn in views (decision 0080): outside every view, or hidden by
    /// `LevelOfDetail`. Those in a shadow volume are still drawn into shadow maps.
    /// `instances + culled` is every static mesh instance of the frame.
    pub culled: u32,
}

/// Bytes per mesh vertex (position first, as ray tracing structures read it).
pub(crate) const MESH_VERTEX_STRIDE: u64 = size_of::<MeshVertex>() as u64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Vertex color (sRGB-free linear 0..1 in 8 bits); white when the mesh has none. Read
    /// only when the instance's `params.w` has `PARAM_VERTEX_COLORS` (decision 0180).
    pub color: [u8; 4],
}

/// Bit in `InstanceData::params[3]` (as an integer): the part has a normal map.
pub(crate) const PARAM_NORMAL_MAP: f32 = 1.0;
/// Bit in `InstanceData::params[3]`: multiply the base color by the vertex colors.
pub(crate) const PARAM_VERTEX_COLORS: f32 = 2.0;

/// Vertex buffer layout of [`MeshVertex`] (shared by every mesh pipeline).
pub(crate) const MESH_ATTRS: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 13 => Unorm8x4];

/// One vertex color as stored on the GPU.
pub(crate) fn color8(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
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
    /// The mesh has vertex colors (decision 0180).
    pub colors: bool,
    /// Bounding sphere of the vertices (part space), for culling (decision 0080).
    pub bounds: crate::cull::Sphere,
    /// Reduced levels of detail (index buffer, index count); empty unless a scene asked
    /// for `LevelOfDetail` when the model was uploaded.
    pub lods: Vec<(wgpu::Buffer, u32)>,
    /// Ray tracing data (decision 0093), on devices that can trace rays.
    pub rt: Option<crate::raytrace::RtPart>,
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
    /// Model-space bounding box of the rest pose (min, max); `None` for an empty model.
    pub bounds: Option<(glam::Vec3, glam::Vec3)>,
    /// Bytes of its textures on the GPU, all mip levels.
    pub texture_bytes: u64,
}

/// Pipeline variant and cut-out of `part` drawn with its material or an override
/// (decision 0280).
pub(crate) fn part_look(
    model: &GpuModel,
    part: &GpuPart,
    over: Option<&marionette_core::spatial::Material>,
) -> variants::Look {
    let (_, md) = &model.materials[part.material.min(model.materials.len() - 1)];
    variants::Look::of(md, over)
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
            (
                [c.r, c.g, c.b, c.a],
                [m.metallic, m.roughness, 1.0, color_bit(part)],
            )
        }
        None => {
            let (_, md) = &model.materials[part.material.min(model.materials.len() - 1)];
            let has_normal = if md.normal_texture.is_some() {
                PARAM_NORMAL_MAP
            } else {
                0.0
            };
            (
                md.base_color,
                [
                    md.metallic,
                    md.roughness,
                    md.normal_scale,
                    has_normal + color_bit(part),
                ],
            )
        }
    }
}

fn color_bit(part: &GpuPart) -> f32 {
    if part.colors { PARAM_VERTEX_COLORS } else { 0.0 }
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
    /// Game shaders and their materials (decision 0181); built when first used.
    game: Option<game_shader::GameShading>,
    lights_group: wgpu::BindGroup,
    /// Sun shadow map, one layer per cascade, while a scene asks for shadows; 1x1 otherwise.
    shadow_map: LocalShadowMaps,
    shadow_sampler: wgpu::Sampler,
    shadow_pipeline: wgpu::RenderPipeline,
    /// Shadows of cut-out parts (alpha tested against the material) and the empty group 1
    /// their layout skips (decision 0280).
    masked_shadow: (wgpu::RenderPipeline, wgpu::BindGroup),
    /// Point and spot light shadow maps (decision 0065); one 1x1 layer when none cast.
    local_shadows: LocalShadowMaps,
    /// Environment (decision 0066), built in `prepare`; `None` while no scene asks.
    environment: Option<GpuEnvironment>,
    /// Stand-in environment texture (1x1 black).
    no_environment: wgpu::TextureView,
    env_sampler: wgpu::Sampler,
    /// Lit model and sky pipelines into the final and the HDR target.
    lit: LitPipelines,
    view_layout: wgpu::BindGroupLayout,
    /// Ray-traced lighting (decision 0093), made when a scene first asks for it on a GPU with
    /// ray queries.
    rt: Option<crate::raytrace::RayTracer>,
    /// Reflection probe pictures (decision 0092); a 1x1 stand-in while no scene has probes.
    probes: crate::probes::Probes,
    /// Drawing a probe's faces: linear light into the HDR target, no bloom, sprites or UI.
    capturing: bool,
    post: crate::post::Post,
    /// Render scale and foveation (decisions 0081, 0082).
    scale: crate::scale::Scale,
    instance_buffer: Option<wgpu::Buffer>,
    /// Reused per-frame arrays: instances in frame order, their group, instances by group.
    scratch: (Vec<InstanceData>, Vec<u32>, Vec<InstanceData>),
    /// Reused per-frame array: the culling class of each instance in `scratch.0`.
    scratch_class: Vec<crate::cull::Class>,
    material_layout: wgpu::BindGroupLayout,
    sprite_tex_layout: wgpu::BindGroupLayout,
    sprite_pipeline: wgpu::RenderPipeline,
    material_sampler: wgpu::Sampler,
    sprite_sampler: wgpu::Sampler,
    defaults: [wgpu::TextureView; 3], // white sRGB, white linear, flat normal
    models: BTreeMap<String, Result<GpuModel, String>>,
    /// Version of each uploaded game mesh (`mesh:<name>` assets, decision 0180).
    game_mesh_versions: BTreeMap<String, u64>,
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
    ui_output_pipeline: wgpu::RenderPipeline,
    world_quads: crate::worldquads::WorldQuadPass,
    /// Particle emitters (decision 0282).
    particles: crate::particles::Particles,
    nearest_sampler: wgpu::Sampler,
    ui_vertices: Option<wgpu::Buffer>,
    ui_indices: Option<wgpu::Buffer>,
    skinning: crate::skinning::Skinning,
    /// Frustum culling on (the default; decision 0080). Off only for comparisons.
    culling: bool,
    /// The last prepared frame asked for `LevelOfDetail`: uploads build reduced levels.
    lod_wanted: bool,
    /// The last prepared frame asked for `TextureCompression` and the device has BC formats:
    /// `Some(normal maps too)` (decision 0283).
    compress_wanted: Option<bool>,
    /// The device samples BC7 textures.
    bc_supported: bool,
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
    /// (asset, base folder, what to build beside the model)
    ask: mpsc::Sender<(String, PathBuf, Want)>,
    done: mpsc::Receiver<(String, Result<Decoded, String>)>,
    pending: BTreeSet<String>,
}

impl ModelLoader {
    fn start() -> Self {
        let (ask, requests) = mpsc::channel::<(String, PathBuf, Want)>();
        let (reply, done) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("marionette-model-loader".into())
            .spawn(move || {
                for (asset, base, want) in requests {
                    let loaded = decode_model(&asset, &base, want);
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

/// A decoded model and, when asked for, the reduced levels of each part (decision 0080),
/// both made off the render thread while streaming.
struct Decoded {
    model: ModelData,
    /// Per part, the index lists of its reduced levels (empty for parts that keep one).
    lods: Vec<Vec<Vec<u32>>>,
    /// Per image, its mipmap levels below the full size (decision 0280).
    mips: Vec<Vec<Vec<u8>>>,
    /// Per image, its BC7 blocks, level 0 first, when compressed (decision 0283).
    packed: Vec<Option<Vec<Vec<u8>>>>,
}

/// What a model decode builds beside the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Want {
    /// Reduced levels of detail (decision 0080).
    lod: bool,
    /// BC7 textures, `Some(normal maps too)` (decision 0283).
    compress: Option<bool>,
}

/// BC7 blocks of the images worth compressing (decision 0283): those with a size in whole
/// blocks; normal maps only when `normal_maps`. Encoded once, then read from the disk cache.
#[cfg(feature = "compress")]
fn pack_images(model: &ModelData, mips: &[Vec<Vec<u8>>], normal_maps: bool) -> Vec<Option<Vec<Vec<u8>>>> {
    use assets::compress::{CACHE_LIMIT, Codec, cache_dir, compress_cached};
    let normals: BTreeSet<usize> = model.materials.iter().filter_map(|m| m.normal_texture).collect();
    let dir = cache_dir();
    model
        .images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            if !normal_maps && normals.contains(&i) {
                return None;
            }
            let m = mips.get(i).map_or(&[][..], Vec::as_slice);
            compress_cached(img, m, Codec::Bc7, dir.as_deref(), CACHE_LIMIT).map(|b| b.levels)
        })
        .collect()
}

#[cfg(not(feature = "compress"))]
fn pack_images(_: &ModelData, _: &[Vec<Vec<u8>>], _: bool) -> Vec<Option<Vec<Vec<u8>>>> {
    Vec::new()
}

/// Mipmap chains of a model's images, each averaged the way its materials use it: normal
/// maps as vectors, data as numbers, colors in linear light keeping cut-out coverage.
fn texture_mips(model: &ModelData) -> Vec<Vec<Vec<u8>>> {
    use assets::mips::{MipKind, chain};
    let mut kinds: Vec<Option<MipKind>> = vec![None; model.images.len()];
    let mut set = |i: Option<usize>, kind: MipKind| {
        if let Some(k) = i.and_then(|i| kinds.get_mut(i)) {
            *k = Some(match (*k, kind) {
                // A texture shared by a cut-out and another material keeps the coverage.
                (Some(MipKind::Color { coverage: Some(c) }), MipKind::Color { .. }) => {
                    MipKind::Color { coverage: Some(c) }
                }
                _ => kind,
            });
        }
    };
    for m in &model.materials {
        let coverage = match m.alpha_mode {
            assets::AlphaMode::Mask { cutoff } => Some(cutoff),
            _ => None,
        };
        set(m.base_color_texture, MipKind::Color { coverage });
        set(m.emissive_texture, MipKind::Color { coverage: None });
        set(m.metallic_roughness_texture, MipKind::Linear);
        set(m.normal_texture, MipKind::Normal);
    }
    model
        .images
        .iter()
        .zip(kinds)
        .map(|(img, kind)| {
            let kind = kind.unwrap_or(if img.srgb {
                MipKind::Color { coverage: None }
            } else {
                MipKind::Linear
            });
            chain(img, kind)
        })
        .collect()
}

fn decode_model(asset: &str, base: &std::path::Path, want: Want) -> Result<Decoded, String> {
    let model = assets::load_model(asset, base)?;
    let lods = if want.lod {
        model
            .parts
            .iter()
            .map(|p| {
                if p.skin.is_none() && p.mesh.morph_targets.is_empty() {
                    crate::cull::reduced_levels(&p.mesh.indices, &p.mesh.positions)
                } else {
                    Vec::new()
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let mips = texture_mips(&model);
    let packed = want
        .compress
        .map_or_else(Vec::new, |normals| pack_images(&model, &mips, normals));
    Ok(Decoded {
        model,
        lods,
        mips,
        packed,
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

        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let view_layout = layout("view", &[uniform_entry(0, vf, true, view_size)]);
        let lights_layout = layout("lights", &lights_layout_entries());
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
        let shadow_map = local_shadow_maps(device, 1, 1);
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
        let probes = crate::probes::Probes::new(device, 1);
        let lights_group = lights_bind_group(
            device,
            &lights_layout,
            &lights_buffer,
            [
                &shadow_map.array,
                &local_shadows.array,
                &no_environment,
                &probes.view,
            ],
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
        let lit = lit_pipelines(
            device,
            &pbr_shader,
            [&view_layout, &lights_layout, &material_layout],
            color_format,
            multisample,
        );
        let post = crate::post::Post::new(device, color_format, sample_count, depth_stencil(false));
        let scale = crate::scale::Scale::new(device, color_format, sample_count);

        // Depth-only pass from the shadow-casting light: only the view group is used.
        let shadow_pipeline = depth_pipeline(
            device,
            &pbr_shader,
            &view_layout,
            "vs_shadow",
            shadow_depth_state(),
            wgpu::MultisampleState::default(),
            (None, None), // both faces cast shadows; no color target
        );
        let masked_shadow = masked_shadow_pipeline(device, &pbr_shader, &view_layout, &material_layout);

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
        let sprite_pipeline_for = |label, depth: Option<wgpu::DepthStencilState>, multisample| {
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
                depth_stencil: depth,
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
        let sprite_pipeline = sprite_pipeline_for("sprite", Some(depth_stencil(false)), multisample);
        // UI: same shader, drawn over everything (no depth test).
        let ui_pipeline = sprite_pipeline_for(
            "ui",
            Some(wgpu::DepthStencilState {
                depth_compare: Some(wgpu::CompareFunction::Always),
                ..depth_stencil(false)
            }),
            multisample,
        );
        // UI in the output pass of render scale and foveation: single-sampled, no depth
        // (decision 0081). UI quads are pixel-aligned, so multisampling adds nothing there.
        let ui_output_pipeline = sprite_pipeline_for("ui output", None, wgpu::MultisampleState::default());

        let world_quads = crate::worldquads::WorldQuadPass::new(
            device,
            &view_layout,
            &sprite_tex_layout,
            color_format,
            DEPTH_FORMAT,
            multisample,
        );

        let particles = crate::particles::Particles::new(
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
        // Model textures: trilinear through the mipmaps, 16x anisotropic where the GPU has it
        // (wgpu clamps to 1 elsewhere), so surfaces seen at a grazing angle stay sharp
        // without shimmering (decision 0280).
        let material_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: MATERIAL_ANISOTROPY,
            ..Default::default()
        });
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
            game: None,
            lights_group,
            view_layout,
            rt: None,
            probes,
            capturing: false,
            shadow_map,
            shadow_sampler,
            shadow_pipeline,
            masked_shadow,
            local_shadows,
            environment: None,
            no_environment,
            env_sampler,
            lit,
            post,
            scale,
            instance_buffer: None,
            scratch: Default::default(),
            material_layout,
            sprite_tex_layout,
            sprite_pipeline,
            material_sampler,
            sprite_sampler,
            defaults,
            models: BTreeMap::new(),
            game_mesh_versions: BTreeMap::new(),
            loader: None,
            streaming: false,
            prepared_once: false,
            plain_material,
            sprite_textures: BTreeMap::new(),
            sprite_vertices: None,
            sprite_indices: None,
            ui_pipeline,
            ui_output_pipeline,
            world_quads,
            particles,
            nearest_sampler,
            ui_vertices: None,
            ui_indices: None,
            skinning,
            culling: true,
            lod_wanted: false,
            compress_wanted: None,
            bc_supported: device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC),
            scratch_class: Vec::new(),
        }
    }

    /// Frustum culling (decision 0080) is on by default and never changes a pixel; turning
    /// it off draws every instance in every view, for before/after measurements.
    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
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
            .chain(crate::particles::Particles::images(&frame.particles))
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
        if frame.lod.is_some() && !self.lod_wanted {
            // Reduced levels are built at upload: upload loaded models again (once).
            self.finish_loading();
            self.models.clear();
        }
        self.lod_wanted = frame.lod.is_some();
        let compress = frame.texture_compression.filter(|_| self.bc_supported);
        if compress != self.compress_wanted {
            // Textures are encoded at decode: load models again (once per change).
            self.finish_loading();
            self.models.clear();
            self.compress_wanted = compress;
        }
        self.sync_game_meshes(frame);
        for (shader, msg) in self.sync_game_shaders(frame) {
            errors.push(
                Diagnostic::error(codes::SHADER, format!("{shader}: {msg}"))
                    .component("ShaderMaterial")
                    .fix("see docs/ai/SHADERS.md for the surface/vertex contract; the model draws with the standard shader until this is fixed"),
            );
        }
        let stream = self.streaming && self.prepared_once;
        for draw in &frame.meshes {
            self.ensure_model(&draw.asset, stream);
            if let Some(Err(msg)) = self.models.get(&*draw.asset) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone())
                        .entity(draw.entity)
                        .component("Model")
                        .fix(
                            "check the path (relative to the scene file), use primitive:cube|sphere|plane, or store the mesh:<name> in GameMeshes",
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
        for p in &frame.particles {
            if let Some(msg) = self.ensure_texture(&p.emitter.image) {
                errors.push(
                    Diagnostic::error(codes::ASSET, msg.clone())
                        .entity(p.entity)
                        .component("ParticleEmitter")
                        .fix("check the PNG path (relative to the scene file) or use builtin:circle"),
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
                    .fix("give an equirectangular PNG, .hdr or .exr image (2:1) relative to the scene file, or \"\" for a sky from SkyLight colors"),
            );
        }
        if let Err(msg) = self.update_probes(frame) {
            errors.push(Diagnostic::error(codes::DEVICE, msg).component("ReflectionProbe"));
        }
        self.prepared_once = true;
        errors
    }

    /// Captures probes that are new, moved or changed (decision 0092); frees the pictures when
    /// no probe is left. Blocks for the GPU once per capture.
    fn update_probes(&mut self, frame: &FrameData) -> Result<(), String> {
        if frame.probes.is_empty() {
            if !self.probes.is_stand_in() {
                self.probes = crate::probes::Probes::new(&self.device, 1);
                self.rebuild_lights_group();
            }
            return Ok(());
        }
        if self.models_loading() > 0 {
            // Models still decoding would be missing from the picture: capture once they are in.
            return Ok(());
        }
        if self.probes.is_stand_in() {
            self.probes = crate::probes::Probes::new(&self.device, crate::extract::MAX_PROBES as u32);
            self.rebuild_lights_group();
        }
        let far = frame
            .cameras
            .first()
            .map_or(200.0, |c| match c.camera.projection {
                marionette_core::spatial::Projection::Perspective { far, .. } => far,
                _ => 200.0,
            });
        for (i, probe) in frame.probes.iter().enumerate() {
            if self.probes.held[i] == Some(*probe) {
                continue;
            }
            let size = crate::probes::face_size(probe.resolution);
            let views = crate::probes::face_views(probe.position, far);
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("probe capture"),
                });
            // Draw the six faces with this frame's lights, minus probe reflections.
            let t = self.probes.targets(&self.device, size, self.sample_count);
            let (msaa, depth, faces) = (t.msaa.clone(), t.depth.clone(), t.face_views.clone());
            let targets: Vec<Target> = faces
                .iter()
                .map(|f| Target {
                    color: msaa.as_ref().unwrap_or(f),
                    resolve: msaa.as_ref().map(|_| f),
                    depth: &depth,
                    width: size,
                    height: size,
                })
                .collect();
            let pairs: Vec<(View, &Target)> = views.iter().copied().zip(targets.iter()).collect();
            self.capturing = true;
            self.render_views(frame, &pairs, &mut encoder);
            self.capturing = false;
            drop(pairs);
            drop(targets);
            self.probes
                .finish_capture(&self.device, &self.queue, encoder, i, &views)?;
            self.probes.held[i] = Some(*probe);
        }
        for held in self.probes.held.iter_mut().skip(frame.probes.len()) {
            *held = None;
        }
        Ok(())
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

    fn want(&self) -> Want {
        Want {
            lod: self.lod_wanted,
            compress: self.compress_wanted,
        }
    }

    /// Bytes of model textures on the GPU (all mip levels), for memory checks.
    pub fn texture_bytes(&self) -> u64 {
        self.models
            .values()
            .filter_map(|m| m.as_ref().ok())
            .map(|m| m.texture_bytes)
            .sum()
    }

    /// Loads a model now, or (streaming) asks the background thread for it.
    fn ensure_model(&mut self, asset: &str, stream: bool) {
        // Game meshes are uploaded by `sync_game_meshes`, never loaded from files.
        if self.models.contains_key(asset) || marionette_core::mesh::mesh_name(asset).is_some() {
            return;
        }
        // Built-in shapes are cheap; only files go to the background.
        if stream && !asset.starts_with("primitive:") {
            let base = self.base_dir.clone();
            let want = self.want();
            let loader = self.loader.get_or_insert_with(ModelLoader::start);
            if loader.pending.contains(asset) {
                return;
            }
            if loader.ask.send((asset.to_string(), base, want)).is_ok() {
                loader.pending.insert(asset.to_string());
                return;
            }
            self.loader = None; // thread gone: load in place below
        }
        let loaded = decode_model(asset, &self.base_dir, self.want()).map(|m| self.upload_model(&m));
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
            Source::Image(path) if assets::is_hdr_name(path) => {
                let img = assets::load_hdr_image(path, &self.base_dir)?;
                e::from_linear(img.width, img.height, &img.rgb)
            }
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
            [
                &self.shadow_map.array,
                &self.local_shadows.array,
                env,
                &self.probes.view,
            ],
            [&self.shadow_sampler, &self.env_sampler],
        );
    }

    fn upload_model(&self, decoded: &Decoded) -> GpuModel {
        let (model, lod_indices) = (&decoded.model, &decoded.lods);
        let mut texture_bytes = 0u64;
        let views: Vec<wgpu::TextureView> = model
            .images
            .iter()
            .enumerate()
            .map(|(i, img)| {
                if let Some(Some(levels)) = decoded.packed.get(i) {
                    texture_bytes += levels.iter().map(|l| l.len() as u64).sum::<u64>();
                    return create_bc7_texture_view(&self.device, &self.queue, img, levels);
                }
                let mips = decoded.mips.get(i).map_or(&[][..], Vec::as_slice);
                texture_bytes += img.rgba.len() as u64 + mips.iter().map(|m| m.len() as u64).sum::<u64>();
                create_texture_view_mips(&self.device, &self.queue, img, mips)
            })
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
        let rt = self
            .device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        // Ray tracing structures read vertex and index buffers directly.
        let blas_input = if rt {
            wgpu::BufferUsages::BLAS_INPUT
        } else {
            wgpu::BufferUsages::empty()
        };
        let parts = model
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let vertices: Vec<MeshVertex> = (0..p.mesh.positions.len())
                    .map(|i| MeshVertex {
                        position: p.mesh.positions[i],
                        normal: p.mesh.normals[i],
                        uv: p.mesh.uvs[i],
                        color: p.mesh.colors.get(i).map_or([255; 4], |&c| color8(c)),
                    })
                    .collect();
                GpuPart {
                    vertices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX | blas_input,
                    }),
                    indices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh indices"),
                        contents: bytemuck::cast_slice(&p.mesh.indices),
                        usage: wgpu::BufferUsages::INDEX | blas_input,
                    }),
                    index_count: p.mesh.indices.len() as u32,
                    material: p.material,
                    transform: p.transform,
                    node: p.node,
                    deform: crate::skinning::Deform::upload(&self.device, p, model.rig.skins.len()),
                    colors: !p.mesh.colors.is_empty(),
                    bounds: crate::cull::Sphere::around(&p.mesh.positions),
                    lods: lod_indices
                        .get(i)
                        .into_iter()
                        .flatten()
                        .map(|idx| {
                            let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("mesh lod indices"),
                                contents: bytemuck::cast_slice(idx),
                                usage: wgpu::BufferUsages::INDEX,
                            });
                            (buffer, idx.len() as u32)
                        })
                        .collect(),
                    rt: (rt && p.mesh.joints.is_empty()).then(|| {
                        crate::raytrace::RtPart::new(&p.mesh, model.materials.get(p.material), &model.images)
                    }),
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
            bounds: model_bounds(model),
            texture_bytes,
        }
    }

    /// Model-space bounding box (min, max) of a model this renderer has loaded, for picking
    /// and framing. `None` while the model is not loaded (or still streaming) or failed.
    pub fn model_bounds(&self, asset: &str) -> Option<(glam::Vec3, glam::Vec3)> {
        self.models.get(asset)?.as_ref().ok()?.bounds
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
                // w: game time for game shaders (decision 0181); unused by the standard shader.
                eye: view.eye.extend(frame.time).to_array(),
                inv_view_proj: mat(vp.inverse()),
            };
            let (sw, sh) = (target.width as f32, target.height as f32);
            let ui_proj = glam::camera::rh::proj::directx::orthographic(0.0, sw, sh, 0.0, -1.0, 1.0);
            let uvu = ViewUniform {
                view_proj: mat(ui_proj),
                eye: [0.0, 0.0, 0.0, frame.time],
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

        // Ray tracing (decision 0093): only on devices with ray queries; elsewhere the scene's
        // other looks stay as they are.
        let rt_cfg = frame.ray_tracing.filter(|r| {
            (r.shadows || r.reflections)
                && self
                    .device
                    .features()
                    .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        });
        if rt_cfg.is_some() && self.rt.is_none() {
            self.rt = Some(crate::raytrace::RayTracer::new(
                &self.device,
                crate::raytrace::Setup {
                    view_layout: &self.view_layout,
                    material_layout: &self.material_layout,
                    deform_layout: &self.skinning.layout,
                    color_format: self.color_format,
                    multisample: wgpu::MultisampleState {
                        count: self.sample_count,
                        mask: !0,
                        alpha_to_coverage_enabled: false,
                    },
                    depth: wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    },
                    mesh_stride: MESH_VERTEX_STRIDE,
                },
            ));
        }
        if let Some(rt) = self.rt.as_mut() {
            rt.group = None;
            if rt_cfg.is_some() {
                rt.prepare_models(&self.device, &self.queue, &mut self.models);
            }
        }
        let rt_shadows = rt_cfg.is_some_and(|r| r.shadows);

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
        // Sun shadow volumes, so culling keeps casters outside the views (decision 0080).
        let mut shadow_vps: Vec<Mat4> = Vec::new();
        // Shadows: the first directional light, seen from the first view; one map, or
        // cascades split along the view (decision 0090).
        let cascades: Vec<Mat4> = match (frame.shadows, frame.lights.first(), views.first()) {
            (Some(s), Some(light), Some((view, _))) if s.distance > 0.0 && !rt_shadows => {
                let resolution = s.resolution.clamp(256, 4096);
                let split = frame.cascades.and_then(|(count, far)| {
                    cascade_view_projs(light.direction, view, count, far, resolution)
                });
                let maps: Vec<(Mat4, f32)> = split.unwrap_or_else(|| {
                    let vp = shadow_view_proj(light.direction, view, s.distance, resolution);
                    vec![(vp, 2.0 * s.distance / resolution as f32)]
                });
                lu.shadow_view_proj = mat(maps[0].0);
                for (k, (m, texel)) in maps.iter().enumerate() {
                    if k > 0 {
                        lu.cascade_mats[k - 1] = mat(*m);
                    }
                    lu.cascade_offsets[k] = texel * 1.5;
                }
                lu.count[3] = maps.len() as u32;
                shadow_vps = maps.iter().map(|(m, _)| *m).collect();
                lu.post[1] = 1.0 / resolution as f32;
                lu.post[2] = s.softness.clamp(0.0, 4.0);
                lu.post[3] = maps[0].1 * 1.5;
                flags |= FLAG_SHADOWS;
                let layers = maps.len() as u32;
                if self.shadow_map.resolution != resolution || self.shadow_map.layers != layers {
                    self.shadow_map = local_shadow_maps(&self.device, resolution, layers);
                    self.rebuild_lights_group();
                }
                let mut bytes = vec![0u8; maps.len() * VIEW_STRIDE as usize];
                for (k, (m, _)) in maps.iter().enumerate() {
                    let su = ViewUniform {
                        view_proj: mat(*m),
                        eye: [0.0, 0.0, 0.0, frame.time],
                        inv_view_proj: mat(m.inverse()),
                    };
                    let at = k * VIEW_STRIDE as usize;
                    bytes[at..at + size_of::<ViewUniform>()].copy_from_slice(bytemuck::bytes_of(&su));
                }
                self.queue
                    .write_buffer(&self.view_buffer, SUN_SHADOW_SLOT * VIEW_STRIDE, &bytes);
                maps.into_iter().map(|(m, _)| m).collect()
            }
            _ => Vec::new(),
        };
        if cascades.is_empty() && self.shadow_map.resolution > 1 {
            // No sun shadows any more: give the map back.
            self.shadow_map = local_shadow_maps(&self.device, 1, 1);
            self.rebuild_lights_group();
        }
        // Point and spot light shadows (decision 0065): layers in light order until the
        // budget runs out; a point light needs six (one per cube face), a spot light one.
        let mut layer_mats: Vec<Mat4> = Vec::new();
        let mut layer_res = 0u32;
        for (j, l) in frame.local_lights.iter().take(MAX_LOCAL_LIGHTS).enumerate() {
            lu.local_shadow[j][0] = -1.0;
            let Some((resolution, softness)) = l.shadow.filter(|_| !rt_shadows) else {
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
                    eye: [0.0, 0.0, 0.0, frame.time],
                    inv_view_proj: mat(m.inverse()),
                };
                let at = k * VIEW_STRIDE as usize;
                bytes[at..at + size_of::<ViewUniform>()].copy_from_slice(bytemuck::bytes_of(&u));
            }
            self.queue
                .write_buffer(&self.view_buffer, LOCAL_SHADOW_SLOT * VIEW_STRIDE, &bytes);
        }
        if let Some(r) = rt_cfg {
            if r.shadows {
                flags |= FLAG_RT_SHADOWS;
            }
            if r.reflections {
                flags |= FLAG_RT_REFLECT;
            }
            let sun_radius = (r.sun_size_degrees.clamp(0.0, 10.0) * 0.5).to_radians().tan();
            lu.rt = [r.max_roughness.clamp(0.0, 1.0), sun_radius, 0.0, 0.0];
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
        // Reflection probes (decision 0092): not while capturing them.
        if !frame.probes.is_empty() && !self.capturing && !self.probes.is_stand_in() {
            flags |= FLAG_PROBES;
            (lu.probe_pos, lu.probe_ext) = crate::probes::uniform(&frame.probes, &self.probes.held);
        }
        // Bloom: models go to the HDR target untone-mapped; the clear and fog colors are
        // mapped back through the tone curve so they come out unchanged (decision 0064).
        // A probe capture writes linear light the same way, without the glow, at full size.
        let capture = self.capturing;
        let bloom = if capture { None } else { frame.bloom };
        // Render scale and foveation: the world goes to an internal image per view size.
        let scaled = frame
            .scale
            .and_then(crate::scale::ScaleSettings::active)
            .filter(|_| !capture);
        let world_size = |t: &Target| match scaled {
            Some(s) => s.internal((t.width, t.height)),
            None => (t.width, t.height),
        };
        let outputs: Vec<(u32, u32)> = views.iter().map(|(_, t)| (t.width, t.height)).collect();
        if !capture {
            // A capture leaves the views' internal images alone.
            self.scale.prepare(&self.device, &self.queue, scaled, &outputs);
        }
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
        if bloom.is_some() || capture {
            flags |= FLAG_HDR;
            if flags & FLAG_FOG != 0 {
                let f = hdr_color(Vec3::from_slice(&lu.fog[..3]));
                lu.fog = f.extend(lu.fog[3]).to_array();
            }
        }
        if bloom.is_some() {
            let sizes: Vec<(u32, u32)> = views.iter().map(|(_, t)| world_size(t)).collect();
            self.post.prepare(&self.device, &sizes);
        } else if !capture {
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
        // (asset, part, material overridden, holds a shadow-casting lamp, level of detail)
        // ..., game shader material slot (decision 0181), material look (decision 0280)
        type Key<'k> = (&'k str, usize, bool, bool, usize, Option<u32>, variants::Look);
        let game = self.game.as_ref();
        let game_slot = |slot: Option<u32>| slot.filter(|&s| game.is_some_and(|g| g.draw(s).is_some()));
        let mut group_of: BTreeMap<Key, u32> = BTreeMap::new();
        let culler = {
            let vps: Vec<Mat4> = views.iter().map(|(v, _)| v.proj * v.view).collect();
            let lamps = frame
                .local_lights
                .iter()
                .take(MAX_LOCAL_LIGHTS)
                .filter(|l| l.shadow.is_some())
                .map(|l| (l.position, l.range))
                .collect();
            let lod = frame.lod.filter(|_| self.lod_wanted);
            crate::cull::Culler::new(self.culling, &vps, &shadow_vps, lamps, lod)
                .keeping_all(rt_cfg.is_some())
                .with_projection_scale(views.first().map_or(1.0, |(v, _)| v.proj.y_axis.y))
        };
        // Which passes draw each instance (`cull::Class`), parallel to `gid`.
        let mut class_of: Vec<crate::cull::Class> = std::mem::take(&mut self.scratch_class);
        class_of.clear();
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
                let (class, level) = culler.classify(&part.bounds.transformed(&model_m), 1 + part.lods.len());
                if class == crate::cull::Class::Hidden {
                    stats.culled += 1;
                    continue;
                }
                let (base_color, params) = instance_material(model, part, draw.material);
                let key = (
                    &*draw.asset,
                    p,
                    draw.material.is_some(),
                    draw.lamp,
                    level,
                    game_slot(draw.shader),
                    part_look(model, part, draw.material.as_ref()),
                );
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
                class_of.push(class);
            }
        }
        for (bi, bulk) in frame.bulk.iter().enumerate() {
            let Some(Ok(model)) = self.models.get(&bulk.asset) else {
                continue;
            };
            let shader = game_slot(bulk.shader.as_ref().map(|_| game_shader::bulk_slot(frame, bi)));
            for (p, part) in model.parts.iter().enumerate() {
                let (base_color, params) = instance_material(model, part, bulk.material);
                let look = part_look(model, part, bulk.material.as_ref());
                let identity = part.transform == Mat4::IDENTITY;
                let levels = 1 + part.lods.len();
                raw.reserve(bulk.transforms.len());
                gid.reserve(bulk.transforms.len());
                class_of.reserve(bulk.transforms.len());
                let mut last: Option<(usize, u32)> = None;
                for t in &bulk.transforms {
                    let world = Mat4::from_cols_array(&[
                        t[0], t[1], t[2], 0.0, t[3], t[4], t[5], 0.0, t[6], t[7], t[8], 0.0, t[9], t[10],
                        t[11], 1.0,
                    ]);
                    let model_m = if identity { world } else { world * part.transform };
                    let (class, level) = culler.classify(&part.bounds.transformed(&model_m), levels);
                    if class == crate::cull::Class::Hidden {
                        stats.culled += 1;
                        continue;
                    }
                    let g = match last {
                        Some((l, g)) if l == level => g,
                        _ => {
                            let key = (
                                bulk.asset.as_str(),
                                p,
                                bulk.material.is_some(),
                                false,
                                level,
                                shader,
                                look,
                            );
                            let n = group_of.len() as u32;
                            let g = *group_of.entry(key).or_insert(n);
                            last = Some((level, g));
                            g
                        }
                    };
                    raw.push(InstanceData {
                        model: mat(model_m),
                        base_color,
                        params,
                        emissive: [0.0; 4],
                    });
                    gid.push(g);
                    class_of.push(class);
                }
            }
        }
        // Group order = key order; counts give each group's start in the final buffer.
        // Within a group, instances inside a view come first (views draw that prefix) and
        // shadow-only casters after them.
        let mut counts = vec![0u32; group_of.len()];
        let mut seen_counts = vec![0u32; group_of.len()];
        for (&g, &c) in gid.iter().zip(&class_of) {
            counts[g as usize] += 1;
            if c == crate::cull::Class::Seen {
                seen_counts[g as usize] += 1;
            }
        }
        let mut start_of = vec![0u32; group_of.len()];
        let mut shadow_start_of = vec![0u32; group_of.len()];
        let mut calls = Vec::with_capacity(group_of.len());
        let mut at = 0u32;
        for (&(asset, p, overridden, lamp, level, slot, look), &g) in &group_of {
            let Some(Ok(model)) = self.models.get(asset) else {
                continue;
            };
            let part = &model.parts[p];
            let material = if overridden {
                &self.plain_material
            } else {
                &model.materials[part.material.min(model.materials.len() - 1)].0
            };
            let (n, seen) = (counts[g as usize], seen_counts[g as usize]);
            start_of[g as usize] = at;
            shadow_start_of[g as usize] = at + seen;
            let indices = match level.checked_sub(1).and_then(|l| part.lods.get(l)) {
                Some((buffer, count)) => (buffer, *count),
                None => (&part.indices, part.index_count),
            };
            stats.instances += seen;
            stats.triangles += (indices.1 / 3) as u64 * seen as u64;
            stats.culled += n - seen;
            calls.push(DrawCall {
                lamp,
                part,
                material,
                indices,
                instances: at..at + n,
                seen,
                game: slot.and_then(|s| game.and_then(|gs| gs.draw(s))),
                look,
            });
            at += n;
        }
        instances.clear();
        instances.resize(raw.len(), InstanceData::zeroed());
        let (mut next, mut next_shadow) = (start_of, shadow_start_of);
        for ((inst, &g), &c) in raw.iter().zip(&gid).zip(&class_of) {
            let slot = if c == crate::cull::Class::Seen {
                &mut next[g as usize]
            } else {
                &mut next_shadow[g as usize]
            };
            instances[*slot as usize] = *inst;
            *slot += 1;
        }
        self.scratch_class = class_of;
        // See-through materials (decision 0280) draw after everything opaque, one instance per
        // draw, farthest from the first view first, so what is behind shows through what is in
        // front. They cast no shadows, so their shadow-only instances are dropped. Blended game
        // shaders follow them, in group order (instance ranges stay put).
        let eye = views.first().map_or(Vec3::ZERO, |(v, _)| v.eye);
        let (mut opaque, mut blended): (Vec<DrawCall>, Vec<DrawCall>) = calls
            .into_iter()
            .partition(|c| !c.look.blended() || c.game.is_some());
        let mut sorted: Vec<(f32, DrawCall)> = Vec::new();
        for c in blended.drain(..) {
            for k in c.instances.start..c.instances.start + c.seen {
                let m = &instances[k as usize].model;
                let d = Vec3::new(m[3][0], m[3][1], m[3][2]).distance_squared(eye);
                sorted.push((
                    d,
                    DrawCall {
                        instances: k..k + 1,
                        seen: 1,
                        ..c.clone()
                    },
                ));
            }
        }
        // Stable: equal distances keep instance order, so frames are reproducible.
        sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
        let game_blended = |c: &DrawCall| {
            c.game
                .is_some_and(|(g, _)| g.options.blend != marionette_core::shader::ShaderBlend::Opaque)
        };
        opaque.sort_by_key(game_blended);
        let first_game_blended = opaque.iter().position(game_blended).unwrap_or(opaque.len());
        let game_tail = opaque.split_off(first_game_blended);
        let first_blended = opaque.len();
        let mut calls = opaque;
        calls.extend(sorted.into_iter().map(|(_, c)| c));
        calls.extend(game_tail);
        if !instances.is_empty() {
            write_growable(
                &self.device,
                &self.queue,
                &mut self.instance_buffer,
                &instances,
                // Storage too: ray-traced hits read the hit instance's material (decision 0093).
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
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
        // Particles (decision 0282): not in probe captures. Into the HDR target they write
        // linear light; into the final target they are tone mapped like models.
        let particle_list: &[crate::extract::ParticleDraw] = if capture { &[] } else { &frame.particles };
        self.particles.upload(
            &self.device,
            &self.queue,
            particle_list,
            frame.time_f64,
            frame.exposure.filter(|_| bloom.is_none()),
        );

        if rt_cfg.is_some()
            && let (Some(rt), Some(ib)) = (self.rt.as_mut(), self.instance_buffer.as_ref())
        {
            let env = self
                .environment
                .as_ref()
                .map_or(&self.no_environment, |g| &g.view);
            rt.frame(
                &self.device,
                &self.queue,
                encoder,
                &calls,
                &instances,
                &self.plain_material,
                crate::raytrace::LightsResources {
                    uniform: &self.lights_buffer,
                    uniform_size: size_of::<LightsUniform>() as u64,
                    views: [
                        &self.shadow_map.array,
                        &self.local_shadows.array,
                        env,
                        &self.probes.view,
                    ],
                    samplers: [&self.shadow_sampler, &self.env_sampler],
                    instances: ib,
                },
            );
        }
        // Lit pipelines and lights group: the ray tracing variant when it ran this frame.
        let (lit, skinned_lit, lights_group, prepass) =
            match self.rt.as_ref().and_then(|r| Some((r, r.group.as_ref()?))) {
                Some((rt, group)) => (&rt.lit, &rt.skinned, group, Some(&rt.prepass)),
                None => (&self.lit, &self.skinning.lit, &self.lights_group, None),
            };
        let meshes = MeshCalls {
            calls: &calls,
            instances: self.instance_buffer.as_ref(),
            skinned: &skinned_calls,
            deform_groups: &deform_groups,
            skinned_instances: self.skinning.instances.as_ref(),
            empty_group: self.game.as_ref().map(|g| &g.empty_group),
            game_lights: Some(&self.lights_group),
            masked_empty: &self.masked_shadow.1,
            first_blended,
        };
        let depth_pipelines = (
            &self.shadow_pipeline,
            &self.masked_shadow.0,
            &self.skinning.shadow_pipeline,
        );
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
        for (k, layer) in self.shadow_map.per_layer.iter().take(cascades.len()).enumerate() {
            let offset = ((SUN_SHADOW_SLOT + k as u64) * VIEW_STRIDE) as u32;
            depth_pass(encoder, layer, offset, false, &mut stats);
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

        // UI quads in screen pixels, on top of everything in the output target.
        let draw_ui = |pass: &mut wgpu::RenderPass,
                       pipeline: &wgpu::RenderPipeline,
                       i: usize,
                       ui_offset: u32,
                       stats: &mut RenderStats| {
            if let (Some(vb), Some(ib), false) = (
                &self.ui_vertices,
                &self.ui_indices,
                ui_batches[i].is_empty() || capture,
            ) {
                pass.set_pipeline(pipeline);
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
        };
        for (i, (view, output)) in views.iter().enumerate() {
            let world_offset = (2 * i as u64 * VIEW_STRIDE) as u32;
            let ui_offset = world_offset + VIEW_STRIDE as u32;
            // The world is drawn into `target`: the output, or its internal image.
            let internal = scaled.map(|s| self.scale.targets(s.internal((output.width, output.height))));
            let internal_target = internal.map(crate::scale::ScaleTargets::target);
            let target: &Target = internal_target.as_ref().unwrap_or(output);
            let foveate = internal.filter(|_| scaled.is_some_and(|s| s.foveation > 0.0));
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
                if let Some(t) = foveate {
                    self.scale.mask(t, &mut pass, true);
                    stats.draw_calls += 1;
                }
                if let Some([_, [p, s]]) = prepass {
                    meshes.draw_prepass(&mut pass, (p, s), &self.view_group, world_offset, &mut stats);
                }
                for phase in [Phase::Opaque, Phase::Blended] {
                    if phase == Phase::Blended && env {
                        pass.set_pipeline(&lit.sky[1]);
                        pass.set_bind_group(0, &self.view_group, &[world_offset]);
                        pass.set_bind_group(1, lights_group, &[]);
                        pass.draw(0..3, 0..1);
                        stats.draw_calls += 1;
                    }
                    meshes.draw_lit(
                        &mut pass,
                        (&lit.pbr, skinned_lit),
                        &self.view_group,
                        world_offset,
                        lights_group,
                        (true, phase),
                        &mut stats,
                    );
                }
                stats.draw_calls +=
                    self.particles
                        .draw(&mut pass, &self.view_group, world_offset, true, |img| {
                            self.sprite_textures
                                .get(img)
                                .and_then(|t| t.as_ref().ok())
                                .map(|t| &t.bind_group)
                        });
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
                        load: wgpu::LoadOp::Clear(clear(if capture { hdr_color(rgb) } else { rgb }, c[3])),
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
                    // A probe capture draws into the floating-point target.
                    let fi = usize::from(capture);
                    if let Some(t) = foveate {
                        self.scale.mask(t, &mut pass, false);
                        stats.draw_calls += 1;
                    }
                    if let Some(depth) = prepass {
                        let [p, s] = &depth[fi];
                        meshes.draw_prepass(&mut pass, (p, s), &self.view_group, world_offset, &mut stats);
                    }
                    // Opaque models, the sky behind them, then see-through models over both.
                    for phase in [Phase::Opaque, Phase::Blended] {
                        if phase == Phase::Blended && env {
                            pass.set_pipeline(&lit.sky[fi]);
                            pass.set_bind_group(0, &self.view_group, &[world_offset]);
                            pass.set_bind_group(1, lights_group, &[]);
                            pass.draw(0..3, 0..1);
                            stats.draw_calls += 1;
                        }
                        meshes.draw_lit(
                            &mut pass,
                            (&lit.pbr, skinned_lit),
                            &self.view_group,
                            world_offset,
                            lights_group,
                            (fi == 1, phase),
                            &mut stats,
                        );
                    }
                    if !capture {
                        stats.draw_calls +=
                            self.particles
                                .draw(&mut pass, &self.view_group, world_offset, false, |img| {
                                    self.sprite_textures
                                        .get(img)
                                        .and_then(|t| t.as_ref().ok())
                                        .map(|t| &t.bind_group)
                                });
                    }
                }
            }
            if let (Some(vb), Some(ib), false) = (
                &self.sprite_vertices,
                &self.sprite_indices,
                batches.is_empty() || capture,
            ) {
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
            // World-space panels draw in the final format only (not into probe captures).
            if !capture {
                stats.draw_calls +=
                    self.world_quads
                        .draw(&mut pass, &self.view_group, world_offset, |image| {
                            self.sprite_textures
                                .get(image)
                                .and_then(|t| t.as_ref().ok())
                                .map(|t| &t.bind_group)
                        });
            }
            let Some(internal) = internal else {
                draw_ui(&mut pass, &self.ui_pipeline, i, ui_offset, &mut stats);
                continue;
            };
            drop(pass);
            // Output pass: the internal image resolved to the output size, then UI. It
            // writes the single-sampled final image directly: the resolve covers every pixel,
            // so a multisampled pass here would only add a full-size clear and resolve.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("output"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output.resolve.unwrap_or(output.color),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Every pixel is written; a clear is cheaper than loading.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.scale.resolve(internal, &mut pass);
            stats.draw_calls += 1;
            draw_ui(&mut pass, &self.ui_output_pipeline, i, ui_offset, &mut stats);
        }
        drop(calls);
        drop(skinned_calls);
        drop(deform_groups);
        if rt_cfg.is_some() {
            crate::raytrace::RayTracer::built(&mut self.models);
        }
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

/// Lights group: uniform, sun shadow map, comparison sampler, local shadow maps,
/// environment texture and its sampler.
fn lights_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    [shadow, local, env, probes]: [&wgpu::TextureView; 4],
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
            view(10, probes),
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

/// Sun shadow cascades (decision 0090): the view from the camera's near plane to `far` is cut
/// into `count` slices (a blend of even and logarithmic splits), and each slice gets an
/// orthographic map around the sphere holding it. The sphere's size depends only on the slice,
/// not on where the camera looks, and its center snaps to whole texels, so edges do not
/// shimmer as the camera turns or moves. Returns each cascade's view-projection and the world
/// size of one texel, or `None` for a camera that is not a perspective one.
pub(crate) fn cascade_view_projs(
    light_dir: Vec3,
    view: &View,
    count: u32,
    far: f32,
    resolution: u32,
) -> Option<Vec<(Mat4, f32)>> {
    if view.proj.z_axis.w.abs() < 0.5 {
        return None; // orthographic: one map
    }
    let count = (count as usize).clamp(2, MAX_CASCADES);
    let inv = (view.proj * view.view).inverse();
    let corners = |z: f32| {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
            .map(|(x, y)| inv.project_point3(Vec3::new(x, y, z)))
    };
    let (near_c, far_c) = (corners(0.0), corners(1.0));
    let depth = |p: Vec3| -view.view.transform_point3(p).z;
    let (near, cam_far) = (depth(near_c[0]), depth(far_c[0]));
    if !(near > 0.0 && cam_far > near) {
        return None;
    }
    let far = far.clamp(near * 2.0, cam_far);
    let split = |i: usize| {
        let f = i as f32 / count as f32;
        let log = near * (far / near).powf(f);
        let even = near + (far - near) * f;
        0.75 * log + 0.25 * even
    };
    let at = |d: f32| {
        let t = (d - near) / (cam_far - near);
        [0, 1, 2, 3].map(|k| near_c[k].lerp(far_c[k], t))
    };
    let d = light_dir.normalize_or(Vec3::NEG_Y);
    let up = if d.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let rot = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, d, up);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let (a, b) = (at(split(i)), at(split(i + 1)));
        let pts = [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]];
        let center = pts.iter().copied().sum::<Vec3>() / 8.0;
        let radius = pts.iter().map(|p| p.distance(center)).fold(0.0f32, f32::max);
        // Round up so float noise in the corners cannot change the size frame to frame.
        let radius = (radius * 16.0).ceil() / 16.0;
        let texel = 2.0 * radius / resolution as f32;
        let c = rot.transform_point3(center);
        let (sx, sy) = ((c.x / texel).floor() * texel, (c.y / texel).floor() * texel);
        // Room behind the slice for casters between it and the sun.
        let back = radius + far;
        let light_view = Mat4::from_translation(-Vec3::new(sx, sy, c.z + back)) * rot;
        let proj = glam::camera::rh::proj::directx::orthographic(
            -radius,
            radius,
            -radius,
            radius,
            0.0,
            back + radius,
        );
        out.push((proj * light_view, texel));
    }
    Some(out)
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
    // A cutoff of exactly zero would keep every fragment anyway.
    let cutoff = match m.alpha_mode {
        assets::AlphaMode::Mask { cutoff } => cutoff.clamp(1e-4, 1.0),
        _ => 0.0,
    };
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("material"),
        contents: bytemuck::bytes_of(&MaterialUniform {
            emissive: [e[0].max(0.0), e[1].max(0.0), e[2].max(0.0), cutoff],
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

/// Rest-pose bounding box of a model: every part's positions through its node transform.
fn model_bounds(model: &ModelData) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut b: Option<(glam::Vec3, glam::Vec3)> = None;
    for part in &model.parts {
        for p in &part.mesh.positions {
            let v = part.transform.transform_point3(glam::Vec3::from_array(*p));
            b = Some(b.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
        }
    }
    b
}

/// Uploads BC7 blocks of `img` (`levels`, level 0 first, as
/// [`assets::compress`] makes them; decision 0283) and returns a view of every level.
fn create_bc7_texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    img: &Image,
    levels: &[Vec<u8>],
) -> wgpu::TextureView {
    let format = if img.srgb {
        wgpu::TextureFormat::Bc7RgbaUnormSrgb
    } else {
        wgpu::TextureFormat::Bc7RgbaUnorm
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image bc7"),
        size: wgpu::Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, blocks) in levels.iter().enumerate() {
        let (w, h) = ((img.width >> level).max(1), (img.height >> level).max(1));
        // Copies cover whole blocks: the level's size rounded up to 4.
        let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            blocks,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16 * bw),
                rows_per_image: Some(bh),
            },
            wgpu::Extent3d {
                width: 4 * bw,
                height: 4 * bh,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Uploads an RGBA8 image (no mipmaps: sprites, UI, fonts) and returns a view of it.
pub fn create_texture_view(device: &wgpu::Device, queue: &wgpu::Queue, img: &Image) -> wgpu::TextureView {
    create_texture_view_mips(device, queue, img, &[])
}

/// Uploads an RGBA8 image with the mipmap levels below it (`mips`, largest first, as
/// [`assets::mips::chain`] makes them) and returns a view of every level.
pub fn create_texture_view_mips(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    img: &Image,
    mips: &[Vec<u8>],
) -> wgpu::TextureView {
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
        mip_level_count: 1 + mips.len() as u32,
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
    let (mut w, mut h) = (img.width, img.height);
    for (level, texels) in mips.iter().enumerate() {
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
        if texels.len() != (4 * w * h) as usize {
            break;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 1 + level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Per-instance vertex attributes of [`InstanceData`] (matches `InstIn` in `pbr.wgsl`).
pub(crate) const INSTANCE_ATTRS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
    7 => Float32x4, 8 => Float32x4, 12 => Float32x4
];

/// Lit model pipelines (final format, HDR) and sky pipelines (final format, HDR).
pub(crate) struct LitPipelines {
    /// Per material variant (decision 0280) and target.
    pub pbr: variants::VariantPipelines,
    pub sky: [wgpu::RenderPipeline; 2],
}

/// A depth-only pipeline for static meshes (view group only) from vertex entry `entry`:
/// shadow maps, or the depth prepass of ray-traced frames (decision 0093).
/// A prepass runs inside a pass with a color attachment, so it names that format and gets
/// an empty fragment shader that writes no color.
pub(crate) fn depth_targets(color: Option<wgpu::TextureFormat>) -> [Option<wgpu::ColorTargetState>; 1] {
    [color.map(|format| wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::empty(),
    })]
}

pub(crate) fn depth_fragment<'a>(
    shader: &'a wgpu::ShaderModule,
    targets: &'a [Option<wgpu::ColorTargetState>],
) -> wgpu::FragmentState<'a> {
    wgpu::FragmentState {
        module: shader,
        entry_point: Some("fs_depth"),
        compilation_options: Default::default(),
        targets,
    }
}

pub(crate) fn depth_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    view_layout: &wgpu::BindGroupLayout,
    entry: &str,
    depth: wgpu::DepthStencilState,
    multisample: wgpu::MultisampleState,
    (cull_mode, color): (Option<wgpu::Face>, Option<wgpu::TextureFormat>),
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("depth"),
        bind_group_layouts: &[Some(view_layout)],
        immediate_size: 0,
    });
    let mesh_attrs = crate::renderer::MESH_ATTRS;
    let targets = depth_targets(color);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("depth"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry),
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
                    attributes: &INSTANCE_ATTRS,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode,
            ..Default::default()
        },
        depth_stencil: Some(depth),
        multisample,
        fragment: color.map(|_| depth_fragment(shader, &targets)),
        multiview_mask: None,
        cache: None,
    })
}

/// Bindings of the lights group (group 1); the ray tracing variant adds its own after these.
pub(crate) fn lights_layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let depth_array = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    };
    vec![
        uniform_entry(
            0,
            wgpu::ShaderStages::FRAGMENT,
            false,
            size_of::<LightsUniform>() as u64,
        ),
        depth_array(1),
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
        depth_array(3),
        texture_entry(4),
        sampler_entry(5),
        wgpu::BindGroupLayoutEntry {
            binding: 10,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
    ]
}

/// The lit model and sky pipelines from `shader` (the plain PBR shader or its ray tracing
/// Shadow pipeline of cut-out parts (decision 0280): like the plain one, plus a fragment
/// stage that drops texels under the material's cutoff. Groups: view, empty, material.
fn masked_shadow_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    view_layout: &wgpu::BindGroupLayout,
    material_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
    let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("empty"),
        entries: &[],
    });
    let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("empty"),
        layout: &empty_layout,
        entries: &[],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cut-out shadow"),
        bind_group_layouts: &[Some(view_layout), Some(&empty_layout), Some(material_layout)],
        immediate_size: 0,
    });
    let mesh_attrs = MESH_ATTRS;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cut-out shadow"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_shadow_masked"),
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
                    attributes: &INSTANCE_ATTRS,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(shadow_depth_state()),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_shadow_masked"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, empty)
}

/// variant) and groups view, lights, material.
pub(crate) fn lit_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    [view_layout, lights_layout, material_layout]: [&wgpu::BindGroupLayout; 3],
    color_format: wgpu::TextureFormat,
    multisample: wgpu::MultisampleState,
) -> LitPipelines {
    let depth_stencil = |write: bool| wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    };
    let pbr_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pbr"),
        bind_group_layouts: &[Some(view_layout), Some(lights_layout), Some(material_layout)],
        immediate_size: 0,
    });
    let (dev, module, opaque_depth) = (device.clone(), shader.clone(), depth_stencil(true));
    let formats = [color_format, crate::post::HDR_FORMAT];
    let pbr = variants::VariantPipelines::new(Box::new(move |variant, target| {
        let (cull_mode, blend, depth) = variants::state(variant, &opaque_depth);
        let mesh_attrs = crate::renderer::MESH_ATTRS;
        dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr"),
            layout: Some(&pbr_layout),
            vertex: wgpu::VertexState {
                module: &module,
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
                        attributes: &INSTANCE_ATTRS,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode,
                ..Default::default()
            },
            depth_stencil: Some(depth),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: formats[target],
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }));
    // Sky background: full-screen triangle at the far plane, depth-tested, not written.
    let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky"),
        bind_group_layouts: &[Some(view_layout), Some(lights_layout)],
        immediate_size: 0,
    });
    let sky_for = |format: wgpu::TextureFormat| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth_stencil(false)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: shader,
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
    let hdr = crate::post::HDR_FORMAT;
    LitPipelines {
        pbr,
        sky: [sky_for(color_format), sky_for(hdr)],
    }
}
