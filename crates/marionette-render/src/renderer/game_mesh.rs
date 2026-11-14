//! Uploading meshes a game builds in code (decision 0180).
//!
//! A `mesh:<name>` asset becomes an ordinary [`GpuModel`] with one part, so every draw path
//! (instancing, culling, level of detail, shadows, bulk draws) treats it like a glTF model.
//! A mesh is uploaded the first time a frame draws it and again whenever the game stores a
//! new version; a new version of the same size is written into the existing buffers
//! instead of allocating new ones, so a game can rewrite a terrain patch every frame.

use super::{GpuModel, GpuPart, MeshVertex, Renderer, color8};
use crate::assets::MaterialData;
use marionette_core::mesh::{GameMesh, mesh_name};
use std::collections::BTreeSet;
use wgpu::util::DeviceExt;

impl Renderer {
    /// Uploads the game meshes `frame` draws that are new or changed, records an error for
    /// names that do not exist, and frees GPU copies of meshes the game removed.
    pub(super) fn sync_game_meshes(&mut self, frame: &crate::extract::FrameData) {
        // Versions of models dropped from the cache (trim, level of detail switched on)
        // no longer describe anything on the GPU.
        let models = &self.models;
        self.game_mesh_versions
            .retain(|k, _| matches!(models.get(k), Some(Ok(_))));
        let wanted: BTreeSet<&str> = frame
            .meshes
            .iter()
            .map(|d| &*d.asset)
            .chain(frame.bulk.iter().map(|b| b.asset.as_str()))
            .filter(|a| mesh_name(a).is_some())
            .collect();
        for &asset in &wanted {
            let name = mesh_name(asset).unwrap_or_default();
            match frame.game_meshes.get(name) {
                None => {
                    self.game_mesh_versions.remove(asset);
                    self.models.insert(
                        asset.to_string(),
                        Err(format!(
                            "no game mesh named `{name}` (store one with GameMeshes::set)"
                        )),
                    );
                }
                Some(entry) => {
                    if self.game_mesh_versions.get(asset) == Some(&entry.version) {
                        continue;
                    }
                    let previous = match self.models.remove(asset) {
                        Some(Ok(m)) => Some(m),
                        _ => None,
                    };
                    let gpu = self.upload_game_mesh(&entry.mesh, previous);
                    self.models.insert(asset.to_string(), Ok(gpu));
                    self.game_mesh_versions.insert(asset.to_string(), entry.version);
                }
            }
        }
        // Meshes the game removed: free them now rather than at the next trim.
        let meshes = &frame.game_meshes;
        let before = self.models.len();
        self.models.retain(|k, _| match mesh_name(k) {
            Some(n) => meshes.contains_key(n) || wanted.contains(k.as_str()),
            None => true,
        });
        if self.models.len() != before {
            let models = &self.models;
            self.game_mesh_versions.retain(|k, _| models.contains_key(k));
        }
    }

    /// One-part model for `mesh`, reusing `previous`'s buffers when the sizes match.
    fn upload_game_mesh(&self, mesh: &GameMesh, previous: Option<GpuModel>) -> GpuModel {
        let normals = if mesh.normals.len() == mesh.positions.len() {
            std::borrow::Cow::Borrowed(&mesh.normals)
        } else {
            // Stored without `GameMeshes::set` (shared): fill in here.
            std::borrow::Cow::Owned(mesh.computed_normals())
        };
        let vertices: Vec<MeshVertex> = (0..mesh.positions.len())
            .map(|i| MeshVertex {
                position: mesh.positions[i],
                normal: normals[i],
                uv: mesh.uvs.get(i).copied().unwrap_or([0.0; 2]),
                color: mesh.colors.get(i).map_or([255; 4], |&c| color8(c)),
            })
            .collect();
        let lods: Vec<Vec<u32>> = if !mesh.lods.is_empty() {
            mesh.lods.clone()
        } else if self.lod_wanted {
            crate::cull::reduced_levels(&mesh.indices, &mesh.positions)
        } else {
            Vec::new()
        };
        let bounds = crate::cull::Sphere::around(&mesh.positions);
        let colors = !mesh.colors.is_empty();
        let vbytes: &[u8] = bytemuck::cast_slice(&vertices);
        let ibytes: &[u8] = bytemuck::cast_slice(&mesh.indices);

        // Same sizes as before: write in place, no new allocation.
        if let Some(mut model) = previous
            && let [part] = model.parts.as_mut_slice()
            && part.vertices.size() == vbytes.len() as u64
            && part.indices.size() == ibytes.len() as u64
            && part.lods.len() == lods.len()
            && part
                .lods
                .iter()
                .zip(&lods)
                .all(|((b, _), l)| b.size() == (l.len() * 4) as u64)
        {
            self.queue.write_buffer(&part.vertices, 0, vbytes);
            self.queue.write_buffer(&part.indices, 0, ibytes);
            for ((buffer, _), list) in part.lods.iter().zip(&lods) {
                self.queue.write_buffer(buffer, 0, bytemuck::cast_slice(list));
            }
            part.bounds = bounds;
            part.colors = colors;
            return model;
        }

        let buffer = |label, contents: &[u8], usage: wgpu::BufferUsages| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: usage | wgpu::BufferUsages::COPY_DST,
            })
        };
        let part = GpuPart {
            vertices: buffer("game mesh vertices", vbytes, wgpu::BufferUsages::VERTEX),
            indices: buffer("game mesh indices", ibytes, wgpu::BufferUsages::INDEX),
            index_count: mesh.indices.len() as u32,
            material: 0,
            transform: glam::Mat4::IDENTITY,
            node: None,
            deform: None,
            colors,
            bounds,
            lods: lods
                .iter()
                .map(|l| {
                    (
                        buffer(
                            "game mesh lod indices",
                            bytemuck::cast_slice(l),
                            wgpu::BufferUsages::INDEX,
                        ),
                        l.len() as u32,
                    )
                })
                .collect(),
            // Game meshes are not in ray-traced scenes yet (decision 0180).
            rt: None,
        };
        GpuModel {
            parts: vec![part],
            materials: vec![(self.plain_material.clone(), MaterialData::default())],
            skins: Vec::new(),
            rest: Vec::new(),
            rest_morph: Vec::new(),
            bounds: mesh
                .bounds()
                .map(|(lo, hi)| (glam::Vec3::from(lo), glam::Vec3::from(hi))),
            texture_bytes: 0,
        }
    }
}
