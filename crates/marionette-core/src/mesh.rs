//! Meshes a game builds in its own code at run time (terrain, generated props, anything
//! not loaded from a file), decision 0180.
//!
//! A module puts a [`GameMesh`] into the [`GameMeshes`] resource under a name; any `Model`
//! (or bulk draw) whose asset is `mesh:<name>` then draws it, through the same lighting,
//! shadows, culling and level of detail as a glTF model. Replacing a mesh under the same
//! name updates every entity that draws it from the next frame.
//!
//! Like [`crate::spatial::BulkDrawHooks`], this is render data, not simulation state: it is
//! not in scenes, snapshots or the state hash. A game rebuilds its meshes from its own
//! saved state (a seed, a heightmap) when a scene loads.

use std::collections::BTreeMap;
use std::sync::Arc;

/// Asset prefix that names a game mesh: `Model { asset: "mesh:<name>" }`.
pub const MESH_PREFIX: &str = "mesh:";

/// Most reduced levels of detail a mesh can carry (matches the renderer's levels).
pub const MAX_MESH_LODS: usize = 2;

/// One mesh made in code: an indexed triangle list.
///
/// Only `positions` and `indices` are required. Empty `normals` are computed (smooth,
/// area-weighted); empty `uvs` are zero; empty `colors` are white. Non-empty attribute
/// arrays must have one entry per position.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Linear RGBA per vertex, multiplied into the material's base color. Custom shaders
    /// may read it as four free numbers instead (for example terrain layer weights); it is
    /// stored as 8 bits per channel, so keep values in 0..1.
    pub colors: Vec<[f32; 4]>,
    /// Three indices per triangle, counter-clockwise when seen from the front.
    pub indices: Vec<u32>,
    /// Reduced levels of detail made by the game (at most [`MAX_MESH_LODS`]): index lists
    /// over the same vertices, coarsest last. Drawn by distance when the scene has a
    /// `LevelOfDetail` resource. Empty: the renderer simplifies the mesh itself under
    /// `LevelOfDetail`, as it does for glTF models.
    pub lods: Vec<Vec<u32>>,
}

/// What is wrong with a mesh, as text for a diagnostic.
pub type MeshError = String;

impl GameMesh {
    /// A mesh from positions and indices only.
    pub fn new(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Self {
        Self {
            positions,
            indices,
            ..Default::default()
        }
    }

    /// Triangles at full detail.
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Checks sizes and index ranges.
    pub fn validate(&self) -> Result<(), MeshError> {
        let n = self.positions.len();
        if n == 0 || self.indices.is_empty() {
            return Err("a mesh needs at least one triangle".into());
        }
        if n > u32::MAX as usize {
            return Err(format!("{n} vertices; at most {} fit 32-bit indices", u32::MAX));
        }
        for (name, len) in [
            ("normals", self.normals.len()),
            ("uvs", self.uvs.len()),
            ("colors", self.colors.len()),
        ] {
            if len != 0 && len != n {
                return Err(format!(
                    "{name} has {len} entries for {n} positions (give one per position or none)"
                ));
            }
        }
        if self.lods.len() > MAX_MESH_LODS {
            return Err(format!(
                "{} reduced levels; at most {MAX_MESH_LODS}",
                self.lods.len()
            ));
        }
        for (level, list) in std::iter::once(&self.indices).chain(&self.lods).enumerate() {
            if list.len() % 3 != 0 {
                return Err(format!(
                    "level {level}: {} indices is not a whole number of triangles",
                    list.len()
                ));
            }
            if let Some(bad) = list.iter().find(|&&i| i as usize >= n) {
                return Err(format!(
                    "level {level}: index {bad} is past the last vertex ({})",
                    n - 1
                ));
            }
        }
        Ok(())
    }

    /// Smooth normals, area-weighted, from the full-detail triangles. Vertices on no
    /// triangle (or only degenerate ones) get +Y.
    pub fn computed_normals(&self) -> Vec<[f32; 3]> {
        let mut acc = vec![[0.0f32; 3]; self.positions.len()];
        for t in self.indices.chunks_exact(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (Some(pa), Some(pb), Some(pc)) = (
                self.positions.get(a),
                self.positions.get(b),
                self.positions.get(c),
            ) else {
                continue;
            };
            let u = sub(*pb, *pa);
            let v = sub(*pc, *pa);
            // Cross product length is twice the area: weights by area for free.
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            for i in [a, b, c] {
                for k in 0..3 {
                    acc[i][k] += n[k];
                }
            }
        }
        acc.into_iter()
            .map(|n| {
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len > 1e-20 {
                    [n[0] / len, n[1] / len, n[2] / len]
                } else {
                    [0.0, 1.0, 0.0]
                }
            })
            .collect()
    }

    /// Box around the positions: (min, max). `None` when empty.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.positions.first()?;
        let (mut lo, mut hi) = (first, first);
        for p in &self.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        Some((lo, hi))
    }

    /// A flat grid of `cells_x` by `cells_z` squares, `size_x` by `size_z` meters, centered
    /// on the origin, facing +Y, with UVs 0..1. `height(x, z)` lifts each vertex (meters);
    /// normals are left to be computed. A starting point for terrain patches.
    pub fn grid(
        cells_x: u32,
        cells_z: u32,
        size_x: f32,
        size_z: f32,
        mut height: impl FnMut(f32, f32) -> f32,
    ) -> Self {
        let (cx, cz) = (cells_x.max(1), cells_z.max(1));
        let mut m = GameMesh::default();
        for j in 0..=cz {
            for i in 0..=cx {
                let (u, v) = (i as f32 / cx as f32, j as f32 / cz as f32);
                let (x, z) = ((u - 0.5) * size_x, (v - 0.5) * size_z);
                m.positions.push([x, height(x, z), z]);
                m.uvs.push([u, v]);
            }
        }
        let row = cx + 1;
        for j in 0..cz {
            for i in 0..cx {
                let a = j * row + i;
                let (b, c, d) = (a + 1, a + row, a + row + 1);
                // Counter-clockwise seen from +Y.
                m.indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        m
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// A stored mesh and the version it was stored at. A renderer re-uploads a mesh when the
/// version it holds differs.
#[derive(Debug, Clone)]
pub struct MeshEntry {
    pub version: u64,
    pub mesh: Arc<GameMesh>,
}

/// The current game meshes. Cloning is one reference count: the renderer takes a copy per
/// frame, and the map is copied on write only while a frame still holds the old one.
pub type MeshMap = Arc<BTreeMap<String, MeshEntry>>;

/// Game meshes by name (decision 0180). Render data: not in scenes or the state hash.
#[derive(bevy_ecs::prelude::Resource, Debug, Clone, Default)]
pub struct GameMeshes {
    map: MeshMap,
    next_version: u64,
}

impl GameMeshes {
    /// Stores `mesh` under `name` (replacing any earlier one), after checking it. Missing
    /// normals are computed here, once, not per frame.
    pub fn set(&mut self, name: &str, mut mesh: GameMesh) -> Result<(), MeshError> {
        if name.is_empty() {
            return Err("a game mesh needs a name".into());
        }
        mesh.validate()?;
        if mesh.normals.is_empty() {
            mesh.normals = mesh.computed_normals();
        }
        self.set_shared(name, Arc::new(mesh));
        Ok(())
    }

    /// Stores an already shared mesh without checking or completing it again. Use when the
    /// same mesh goes under several names, or was checked on a worker thread.
    pub fn set_shared(&mut self, name: &str, mesh: Arc<GameMesh>) {
        self.next_version += 1;
        let entry = MeshEntry {
            version: self.next_version,
            mesh,
        };
        Arc::make_mut(&mut self.map).insert(name.to_string(), entry);
    }

    /// Removes a mesh; its GPU copy is freed with the next frame. Returns whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        if !self.map.contains_key(name) {
            return false;
        }
        Arc::make_mut(&mut self.map).remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<GameMesh>> {
        self.map.get(name).map(|e| &e.mesh)
    }

    /// Names in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// A cheap copy of every mesh for one frame.
    pub fn snapshot(&self) -> MeshMap {
        self.map.clone()
    }

    /// Convenience for modules: stores into the world's resource, creating it.
    pub fn set_in(world: &mut bevy_ecs::world::World, name: &str, mesh: GameMesh) -> Result<(), MeshError> {
        world
            .get_resource_or_insert_with(GameMeshes::default)
            .set(name, mesh)
    }
}

/// The game mesh name inside a model asset string, if it is one.
pub fn mesh_name(asset: &str) -> Option<&str> {
    asset.strip_prefix(MESH_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri() -> GameMesh {
        GameMesh::new(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
            vec![0, 1, 2],
        )
    }

    #[test]
    fn validates_sizes_and_indices() {
        assert!(tri().validate().is_ok());
        let mut m = tri();
        m.indices.push(7);
        assert!(m.validate().unwrap_err().contains("whole number"));
        m.indices.extend([0, 9]);
        assert!(m.validate().unwrap_err().contains("past the last vertex"));
        let mut m = tri();
        m.uvs = vec![[0.0; 2]];
        assert!(m.validate().unwrap_err().contains("uvs"));
        let mut m = tri();
        m.lods = vec![vec![0, 1, 2]; 3];
        assert!(m.validate().is_err());
        assert!(GameMesh::default().validate().is_err());
    }

    #[test]
    fn computes_upward_normals_for_a_ccw_floor_triangle() {
        let n = tri().computed_normals();
        for v in n {
            assert!((v[1] - 1.0).abs() < 1e-6, "{v:?}");
        }
    }

    #[test]
    fn grid_faces_up_and_has_the_right_size() {
        let g = GameMesh::grid(4, 2, 8.0, 4.0, |x, _| x * 0.0);
        assert_eq!(g.positions.len(), 5 * 3);
        assert_eq!(g.triangles(), 4 * 2 * 2);
        assert!(g.validate().is_ok());
        assert!(g.computed_normals().iter().all(|n| (n[1] - 1.0).abs() < 1e-6));
        assert_eq!(g.bounds(), Some(([-4.0, 0.0, -2.0], [4.0, 0.0, 2.0])));
    }

    #[test]
    fn versions_grow_and_snapshots_stay_unchanged() {
        let mut meshes = GameMeshes::default();
        meshes.set("a", tri()).unwrap();
        let before = meshes.snapshot();
        let v1 = before["a"].version;
        meshes.set("a", tri()).unwrap();
        assert!(meshes.snapshot()["a"].version > v1);
        assert_eq!(before["a"].version, v1, "a frame's copy does not change");
        assert!(meshes.get("a").unwrap().normals.len() == 3, "normals filled in");
        assert!(meshes.remove("a"));
        assert!(!meshes.remove("a"));
        assert!(meshes.is_empty());
        assert_eq!(mesh_name("mesh:x"), Some("x"));
        assert_eq!(mesh_name("primitive:cube"), None);
    }
}
