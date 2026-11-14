//! Ray-traced lighting (decision 0093): exact shadows from every light and reflections of
//! other objects, through wgpu's ray queries (Vulkan; experimental in wgpu 30).
//!
//! Made the first time a scene has `RayTracing` on a device with ray queries; a device without
//! them never builds any of this and the scene keeps its other looks. Every static mesh part
//! gets a bottom-level structure (BLAS) the first frame it is drawn; each frame one top-level
//! structure (TLAS) is rebuilt from that frame's instances. The lit shaders are the PBR shader
//! with its ray tracing stubs replaced by `shaders/raytrace.wgsl`, bound with a lights group
//! that adds the TLAS and three storage buffers: per-triangle data (face normal, base color and
//! emissive at the triangle's center, filled on the CPU at upload), the instance buffer
//! itself (transform and material per instance), and per instance its first triangle.
//!
//! Not in the ray-traced scene: skinned and morphed parts, sprites. Reflections shade what
//! they hit with flat normals and the triangle's center color, diffuse light and the
//! environment; they are mirror-sharp, fading to the environment on rough surfaces.

use crate::assets::{Image, MaterialData, MeshData};
use crate::renderer::{DrawCall, GpuModel, InstanceData, LitPipelines};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::BTreeMap;

/// Instance mask bit of ordinary instances and of models holding a shadow-casting lamp
/// (local light shadow rays skip those, as local shadow maps do; decision 0065).
const MASK_PLAIN: u8 = 1;
const MASK_LAMP: u8 = 2;
/// Set in an instance's first-triangle word when its material is overridden (`Material`):
/// the hit uses the instance color alone, not the model's texture colors.
const OVERRIDDEN: u32 = 1 << 31;

/// Per-triangle data the shader reads at a hit (matches `RtTri` in `raytrace.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(crate) struct RtTri {
    /// Face normal in the part's mesh space.
    pub n: [f32; 3],
    /// Base color texture at the triangle's center, sRGB RGBA8 (white without a texture).
    pub color: u32,
    /// Emissive factor times emissive texture at the center (linear).
    pub emissive: [f32; 3],
    pub pad: u32,
}

/// A static mesh part's ray tracing data.
pub(crate) struct RtPart {
    pub tris: Vec<RtTri>,
    pub vertex_count: u32,
    blas: Option<wgpu::Blas>,
    size: Option<wgpu::BlasTriangleGeometrySizeDescriptor>,
    /// The BLAS was made this frame and still needs building.
    fresh: bool,
    /// Index of the first triangle in the shared triangle buffer.
    tri_offset: u32,
}

/// Nearest texel of `img` at `uv` (wrapping), as RGBA8.
fn texel(img: &Image, uv: [f32; 2]) -> [u8; 4] {
    let (w, h) = (img.width.max(1) as i64, img.height.max(1) as i64);
    let x = ((uv[0] * w as f32).floor() as i64).rem_euclid(w) as usize;
    let y = ((uv[1] * h as f32).floor() as i64).rem_euclid(h) as usize;
    let at = (y * w as usize + x) * 4;
    match img.rgba.get(at..at + 4) {
        Some(p) => [p[0], p[1], p[2], p[3]],
        None => [255; 4],
    }
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl RtPart {
    /// Per-triangle data of a mesh part with `material` (textures from `images`).
    pub fn new(mesh: &MeshData, material: Option<&MaterialData>, images: &[Image]) -> Self {
        let base_tex = material
            .and_then(|m| m.base_color_texture)
            .and_then(|i| images.get(i));
        let emissive_tex = material
            .and_then(|m| m.emissive_texture)
            .and_then(|i| images.get(i));
        let emissive = material.map_or([0.0; 3], |m| m.emissive);
        let tris = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| i as usize);
                let p = |i: usize| Vec3::from(mesh.positions.get(i).copied().unwrap_or_default());
                let mut n = (p(b) - p(a)).cross(p(c) - p(a));
                if n.length_squared() < 1e-24 {
                    n = [a, b, c]
                        .iter()
                        .map(|&i| Vec3::from(mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0])))
                        .sum();
                }
                let n = n.normalize_or(Vec3::Y);
                let uv = |i: usize| mesh.uvs.get(i).copied().unwrap_or_default();
                let center = [
                    (uv(a)[0] + uv(b)[0] + uv(c)[0]) / 3.0,
                    (uv(a)[1] + uv(b)[1] + uv(c)[1]) / 3.0,
                ];
                let color = base_tex.map_or([255; 4], |img| texel(img, center));
                let e = emissive_tex.map_or([255; 4], |img| texel(img, center));
                RtTri {
                    n: n.to_array(),
                    color: u32::from_le_bytes(color),
                    emissive: [0, 1, 2].map(|k| emissive[k].max(0.0) * srgb_to_linear(e[k])),
                    pad: 0,
                }
            })
            .collect();
        Self {
            tris,
            vertex_count: mesh.positions.len() as u32,
            blas: None,
            size: None,
            fresh: false,
            tri_offset: 0,
        }
    }
}

/// Ray tracing state of a renderer.
pub(crate) struct RayTracer {
    pub layout: wgpu::BindGroupLayout,
    pub lit: LitPipelines,
    /// Skinned pipelines (final, HDR) with the ray tracing fragment shader.
    pub skinned: crate::renderer::variants::VariantPipelines,
    /// Depth prepass ([static, skinned] for the final and HDR formats): every pixel's nearest surface is known before the lit
    /// pass, so rays are traced once per pixel instead of once per overlapping surface.
    pub prepass: [[wgpu::RenderPipeline; 2]; 2],
    tlas: wgpu::Tlas,
    tlas_capacity: u32,
    /// Instances set in the TLAS last frame (slots past this are already empty).
    tlas_used: usize,
    tris: wgpu::Buffer,
    tri_base: Option<wgpu::Buffer>,
    /// The lights group for this frame (rebuilt every frame: cheap, and it follows every
    /// buffer and texture that changed).
    pub group: Option<wgpu::BindGroup>,
}

/// The ray tracing variant of a PBR shader source: the stubs between the `RT:BEGIN` and
/// `RT:END` markers replaced by `raytrace.wgsl`, and the ray query extension enabled.
pub(crate) fn shader_source(pbr: &str) -> String {
    let begin = pbr.find("// RT:BEGIN").expect("pbr.wgsl has RT:BEGIN");
    let end = pbr.find("// RT:END").expect("pbr.wgsl has RT:END");
    format!(
        "enable wgpu_ray_query;\n{}{}{}",
        &pbr[..begin],
        include_str!("shaders/raytrace.wgsl"),
        &pbr[end..]
    )
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn make_tlas(device: &wgpu::Device, capacity: u32) -> wgpu::Tlas {
    device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("scene tlas"),
        max_instances: capacity,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    })
}

fn storage_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(32),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// What the renderer passes to build the ray tracing variant's pipelines.
pub(crate) struct Setup<'a> {
    pub view_layout: &'a wgpu::BindGroupLayout,
    pub material_layout: &'a wgpu::BindGroupLayout,
    pub deform_layout: &'a wgpu::BindGroupLayout,
    pub color_format: wgpu::TextureFormat,
    pub multisample: wgpu::MultisampleState,
    pub depth: wgpu::DepthStencilState,
    pub mesh_stride: u64,
}

impl RayTracer {
    pub fn new(device: &wgpu::Device, setup: Setup) -> Self {
        let mut entries = crate::renderer::lights_layout_entries();
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
            count: None,
        });
        entries.extend([storage_entry(7), storage_entry(8), storage_entry(9)]);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lights (ray tracing)"),
            entries: &entries,
        });
        let pbr = shader_source(include_str!("shaders/pbr.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr (ray tracing)"),
            source: wgpu::ShaderSource::Wgsl(pbr.as_str().into()),
        });
        let lit = crate::renderer::lit_pipelines(
            device,
            &shader,
            [setup.view_layout, &layout, setup.material_layout],
            setup.color_format,
            setup.multisample,
        );
        let skinned_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr skinned (ray tracing)"),
            source: wgpu::ShaderSource::Wgsl(crate::skinning::skinned_source(&pbr).into()),
        });
        let skinned = crate::skinning::lit_pipelines(
            device,
            &skinned_shader,
            [
                setup.view_layout,
                &layout,
                setup.material_layout,
                setup.deform_layout,
            ],
            setup.color_format,
            setup.depth.clone(),
            setup.multisample,
            setup.mesh_stride,
        );
        // The same vertex entry points as the lit pipelines, so depths match exactly.
        let prepass = [setup.color_format, crate::post::HDR_FORMAT].map(|format| {
            [
                crate::renderer::depth_pipeline(
                    device,
                    &shader,
                    setup.view_layout,
                    "vs_main",
                    setup.depth.clone(),
                    setup.multisample,
                    (Some(wgpu::Face::Back), Some(format)),
                ),
                crate::skinning::depth_pipeline(
                    device,
                    &skinned_shader,
                    [setup.view_layout, setup.deform_layout],
                    "vs_skinned",
                    setup.depth.clone(),
                    setup.multisample,
                    (setup.mesh_stride, Some(wgpu::Face::Back), Some(format)),
                ),
            ]
        });
        Self {
            layout,
            lit,
            skinned,
            prepass,
            tlas: make_tlas(device, 64),
            tlas_capacity: 64,
            tlas_used: 0,
            tris: storage_buffer(device, "rt triangles", 32),
            tri_base: None,
            group: None,
        }
    }

    /// Gives every static part drawn by `models` a BLAS (made now, built in [`Self::frame`]),
    /// and refills the triangle buffer when parts were added. Call before the frame borrows
    /// the models.
    pub fn prepare_models(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        models: &mut BTreeMap<String, Result<GpuModel, String>>,
    ) {
        let mut added = false;
        for model in models.values_mut().filter_map(|m| m.as_mut().ok()) {
            for part in model.parts.iter_mut().filter(|p| p.deform.is_none()) {
                let Some(rt) = part.rt.as_mut() else {
                    continue;
                };
                if rt.blas.is_some() || rt.tris.is_empty() {
                    continue;
                }
                let size = wgpu::BlasTriangleGeometrySizeDescriptor {
                    vertex_format: wgpu::VertexFormat::Float32x3,
                    vertex_count: rt.vertex_count,
                    index_format: Some(wgpu::IndexFormat::Uint32),
                    index_count: Some(part.index_count),
                    flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
                };
                rt.blas = Some(device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some("mesh blas"),
                        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles {
                        descriptors: vec![size.clone()],
                    },
                ));
                rt.size = Some(size);
                rt.fresh = true;
                added = true;
            }
        }
        if added {
            self.refill_triangles(device, queue, models);
        }
    }

    /// Rewrites the shared triangle buffer from every part that has a BLAS.
    fn refill_triangles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        models: &mut BTreeMap<String, Result<GpuModel, String>>,
    ) {
        let mut all: Vec<RtTri> = Vec::new();
        for model in models.values_mut().filter_map(|m| m.as_mut().ok()) {
            for part in &mut model.parts {
                if let Some(rt) = part.rt.as_mut().filter(|rt| rt.blas.is_some()) {
                    rt.tri_offset = all.len() as u32;
                    all.extend_from_slice(&rt.tris);
                }
            }
        }
        let bytes: &[u8] = bytemuck::cast_slice(&all);
        if self.tris.size() < bytes.len() as u64 {
            self.tris = storage_buffer(device, "rt triangles", (bytes.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.tris, 0, bytes);
        }
    }

    /// Fills and builds this frame's TLAS from the static draw calls (instances indexed as in
    /// the instance buffer), builds new BLASes, and makes the lights group.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        calls: &[DrawCall],
        instances: &[InstanceData],
        plain_material: &wgpu::BindGroup,
        lights: LightsResources,
    ) {
        let count = instances.len();
        let limit = device.limits().max_tlas_instance_count.max(1) as usize;
        if count > self.tlas_capacity as usize {
            let capacity = count.next_power_of_two().min(limit) as u32;
            self.tlas = make_tlas(device, capacity);
            self.tlas_capacity = capacity;
            self.tlas_used = 0;
        }
        let mut bases = vec![0u32; count.max(1)];
        let mut used = 0usize;
        for call in calls {
            let Some(rt) = call.part.rt.as_ref() else {
                continue;
            };
            let Some(blas) = rt.blas.as_ref() else {
                continue;
            };
            let overridden = std::ptr::eq(call.material, plain_material);
            let mask = if call.lamp { MASK_LAMP } else { MASK_PLAIN };
            for k in call.instances.clone() {
                let k = k as usize;
                if k >= self.tlas_capacity as usize {
                    break;
                }
                let m = instances[k].model;
                // Column-major 4x4 to row-major 3x4.
                let transform = [
                    m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1], m[0][2], m[1][2],
                    m[2][2], m[3][2],
                ];
                self.tlas[k] = Some(wgpu::TlasInstance::new(blas, transform, k as u32, mask));
                bases[k] = rt.tri_offset | if overridden { OVERRIDDEN } else { 0 };
                used = used.max(k + 1);
            }
        }
        for k in used..self.tlas_used.min(self.tlas_capacity as usize) {
            self.tlas[k] = None;
        }
        // Instances that are not drawn statically (none today) stay empty.
        self.tlas_used = used;
        crate::renderer::write_growable(
            device,
            queue,
            &mut self.tri_base,
            &bases,
            wgpu::BufferUsages::STORAGE,
        );

        let mut fresh: Vec<(
            &wgpu::Blas,
            &wgpu::BlasTriangleGeometrySizeDescriptor,
            &crate::renderer::GpuPart,
        )> = Vec::new();
        for call in calls {
            if let Some(rt) = call.part.rt.as_ref()
                && rt.fresh
                && let (Some(blas), Some(size)) = (rt.blas.as_ref(), rt.size.as_ref())
                && !fresh.iter().any(|(b, _, _)| std::ptr::eq(*b, blas))
            {
                fresh.push((blas, size, call.part));
            }
        }
        let entries: Vec<wgpu::BlasBuildEntry> = fresh
            .iter()
            .map(|(blas, size, part)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size,
                    vertex_buffer: &part.vertices,
                    first_vertex: 0,
                    vertex_stride: crate::renderer::MESH_VERTEX_STRIDE,
                    index_buffer: Some(&part.indices),
                    first_index: Some(0),
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            })
            .collect();
        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&self.tlas));

        let tri_base = self.tri_base.as_ref().expect("written above");
        let mut entries = lights.entries();
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: self.tlas.as_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: self.tris.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: lights.instances.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 9,
            resource: tri_base.as_entire_binding(),
        });
        self.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lights (ray tracing)"),
            layout: &self.layout,
            entries: &entries,
        }));
    }

    /// Marks BLASes built in this frame as built. Call after the frame's commands are
    /// recorded.
    pub fn built(models: &mut BTreeMap<String, Result<GpuModel, String>>) {
        for model in models.values_mut().filter_map(|m| m.as_mut().ok()) {
            for part in &mut model.parts {
                if let Some(rt) = part.rt.as_mut() {
                    rt.fresh = false;
                }
            }
        }
    }
}

/// The plain lights group's resources, which the ray tracing group repeats.
pub(crate) struct LightsResources<'a> {
    pub uniform: &'a wgpu::Buffer,
    pub uniform_size: u64,
    pub views: [&'a wgpu::TextureView; 4],
    pub samplers: [&'a wgpu::Sampler; 2],
    pub instances: &'a wgpu::Buffer,
}

impl<'a> LightsResources<'a> {
    fn entries(&self) -> Vec<wgpu::BindGroupEntry<'a>> {
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let sampler = |binding, sampler| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(sampler),
        };
        vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: self.uniform,
                    offset: 0,
                    size: std::num::NonZeroU64::new(self.uniform_size),
                }),
            },
            view(1, self.views[0]),
            sampler(2, self.samplers[0]),
            view(3, self.views[1]),
            view(4, self.views[2]),
            sampler(5, self.samplers[1]),
            view(10, self.views[3]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_replaces_the_stubs_and_enables_ray_queries() {
        let src = shader_source(include_str!("shaders/pbr.wgsl"));
        assert!(src.starts_with("enable wgpu_ray_query;"));
        assert!(src.contains("rayQueryInitialize"));
        assert_eq!(src.matches("fn rt_visible(").count(), 1);
    }

    #[test]
    fn triangle_data_has_face_normals_and_center_colors() {
        let model = crate::assets::primitive("cube").unwrap();
        let part = &model.parts[0];
        let mut img = Image::solid([10, 20, 30, 255], true);
        img.width = 1;
        let mat = MaterialData {
            base_color_texture: Some(0),
            ..Default::default()
        };
        let rt = RtPart::new(&part.mesh, Some(&mat), &[img]);
        assert_eq!(rt.tris.len(), part.mesh.indices.len() / 3);
        for t in &rt.tris {
            let n = Vec3::from(t.n);
            assert!((n.length() - 1.0).abs() < 1e-4);
            assert_eq!(t.color.to_le_bytes(), [10, 20, 30, 255]);
        }
    }
}
