//! Job data: named typed arrays that live on an entity and are saved with the scene
//! (decision 0070).
//!
//! Hot loops over many items (particles, crowds, belt slots, machine timers) keep their
//! numbers in a [`JobData`] component instead of one entity per item. Job functions
//! (`marionette-jobs-ts`, or a game's own Rust jobs) read and write the arrays in place;
//! scripts reach them through `engine.jobs`; [`JobDraw`] draws one model copy per element.
//! The arrays are ordinary scene data: saved, restored by snapshots and part of the state
//! hash.
//!
//! Running job files is not part of the core. A module that can run them (job TypeScript)
//! inserts a [`JobRunner`] resource, the same way physics offers ray casts to scripts
//! ([`crate::module::RayCaster`]), so scripts never depend on it.

use crate::Diagnostic;
use crate::spatial::{BulkDraw, Material, Transform};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;

/// The values of one job array.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum JobValues {
    /// `Float32Array` in a job file. The usual choice for bulk data.
    F32(Vec<f32>),
    /// `Float64Array`.
    F64(Vec<f64>),
    /// `Int32Array`: counts, indices, item ids, states.
    I32(Vec<i32>),
}

impl JobValues {
    pub fn len(&self) -> usize {
        match self {
            JobValues::F32(v) => v.len(),
            JobValues::F64(v) => v.len(),
            JobValues::I32(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// `Float32Array`, `Float64Array` or `Int32Array`.
    pub fn ts_name(&self) -> &'static str {
        match self {
            JobValues::F32(_) => "Float32Array",
            JobValues::F64(_) => "Float64Array",
            JobValues::I32(_) => "Int32Array",
        }
    }
    /// Element `i` widened to `f64` (0 when out of range).
    pub fn get_f64(&self, i: usize) -> f64 {
        match self {
            JobValues::F32(v) => v.get(i).map_or(0.0, |x| *x as f64),
            JobValues::F64(v) => v.get(i).copied().unwrap_or(0.0),
            JobValues::I32(v) => v.get(i).map_or(0.0, |x| *x as f64),
        }
    }
}

/// One named array of a [`JobData`].
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct JobArray {
    pub name: String,
    /// In a scene file: `{"F32": [0.0, 1.5]}`, `{"F64": [...]}` or `{"I32": [...]}`.
    pub values: JobValues,
}

/// Named typed arrays that job functions read and write in place (decision 0070). Names
/// are unique per entity. Saved with the scene and part of the state hash; keep arrays to
/// what the game needs (every element is written in snapshots).
#[derive(Component, Reflect, Debug, Clone, PartialEq, Default)]
pub struct JobData {
    pub arrays: Vec<JobArray>,
}

impl JobData {
    pub fn get(&self, name: &str) -> Option<&JobValues> {
        self.arrays.iter().find(|a| a.name == name).map(|a| &a.values)
    }
    pub fn get_mut(&mut self, name: &str) -> Option<&mut JobValues> {
        self.arrays
            .iter_mut()
            .find(|a| a.name == name)
            .map(|a| &mut a.values)
    }
    /// Replaces the array `name`, or adds it at the end.
    pub fn set(&mut self, name: &str, values: JobValues) {
        match self.get_mut(name) {
            Some(v) => *v = values,
            None => self.arrays.push(JobArray {
                name: name.to_string(),
                values,
            }),
        }
    }
    /// Array names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.arrays.iter().map(|a| a.name.as_str()).collect()
    }
}

/// Draws one copy of a model per element of the entity's [`JobData`] arrays `x`, `y` and
/// `z` (element `i` is copy `i`), placed by the entity's `Transform` when it has one.
/// `z` may be empty for flat scenes (then 0). Built at render time: the simulation pays
/// nothing for it, and it is drawn as one instanced batch (decision 0025).
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct JobDraw {
    /// Model asset, as in `Model.asset` (`primitive:cube`, `primitive:sphere`, a file).
    pub asset: String,
    /// One material for every copy (null uses the model's own).
    pub material: Option<Material>,
    pub x: String,
    pub y: String,
    pub z: String,
    /// Uniform scale of each copy.
    pub scale: f32,
}

/// How a job argument that is an array is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobArgMode {
    /// Chunked; each call sees its piece; changes are kept.
    Write,
    /// Chunked; read only.
    Read,
    /// The whole array to every chunk; read only (lookup tables, neighbours).
    Whole,
}

/// One argument of a [`JobRunner`] call, in parameter order.
#[derive(Debug)]
pub enum JobCallArg<'a> {
    /// An array of an entity's [`JobData`], used in place.
    Data {
        entity: Entity,
        name: String,
        mode: JobArgMode,
    },
    /// A caller's own array (a script's typed array), copied in and, for `Write`, back.
    F32(&'a mut [f32], JobArgMode),
    F64(&'a mut [f64], JobArgMode),
    I32(&'a mut [i32], JobArgMode),
    /// A number for a `number` or (when whole) an `i32` parameter.
    Number(f64),
    Bool(bool),
    /// The index of the chunk's first element (an `i32` parameter).
    ChunkStart,
}

/// What to run: an exported function of a job file (path relative to the scene).
#[derive(Debug, Clone, Copy)]
pub struct JobCall<'a> {
    pub path: &'a str,
    pub function: &'a str,
    /// Elements per chunk; 0 picks the default.
    pub chunk: usize,
}

/// Runs a job function over the world's job data. Returns one value per chunk, in chunk
/// order, when the function returns a value.
pub type JobRunFn = fn(&mut World, JobCall<'_>, &mut [JobCallArg<'_>]) -> Result<Vec<f64>, Diagnostic>;

/// Offered by a module that runs job files (job TypeScript) to code that must not depend on
/// it (scripts). Inserted in the module's `init`; not simulation state.
#[derive(Resource, Clone, Copy)]
pub struct JobRunner(pub JobRunFn);

/// Rotates `v` by the unit quaternion `q`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    // t = 2 * cross(q.xyz, v); v' = v + w * t + cross(q.xyz, t)
    let t = [
        2.0 * (y * v[2] - z * v[1]),
        2.0 * (z * v[0] - x * v[2]),
        2.0 * (x * v[1] - y * v[0]),
    ];
    [
        v[0] + w * t[0] + (y * t[2] - z * t[1]),
        v[1] + w * t[1] + (z * t[0] - x * t[2]),
        v[2] + w * t[2] + (x * t[1] - y * t[0]),
    ]
}

/// The bulk draw hook for [`JobDraw`] (added to every world by [`crate::Sim`]).
pub fn draw_job_data(world: &World, _alpha: f32, out: &mut Vec<BulkDraw>) {
    let Some(mut q) = world.try_query::<(&JobDraw, &JobData, Option<&Transform>)>() else {
        return;
    };
    for (draw, data, t) in q.iter(world) {
        let (Some(x), Some(y)) = (data.get(&draw.x), data.get(&draw.y)) else {
            continue;
        };
        let z = data.get(&draw.z);
        let n = x.len().min(y.len()).min(z.map_or(usize::MAX, JobValues::len));
        let t = t.copied().unwrap_or_default();
        let q = [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w];
        let s = [t.scale.x, t.scale.y, t.scale.z];
        let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .map(|a: [f32; 3]| rotate(q, a).map(|c| c * draw.scale));
        let axes = [0, 1, 2].map(|i| axes[i].map(|c| c * s[i]));
        let mut transforms = Vec::with_capacity(n);
        for i in 0..n {
            let local = [
                x.get_f64(i) as f32 * s[0],
                y.get_f64(i) as f32 * s[1],
                z.map_or(0.0, |z| z.get_f64(i) as f32) * s[2],
            ];
            let p = rotate(q, local);
            transforms.push([
                axes[0][0],
                axes[0][1],
                axes[0][2],
                axes[1][0],
                axes[1][1],
                axes[1][2],
                axes[2][0],
                axes[2][1],
                axes[2][2],
                p[0] + t.translation.x,
                p[1] + t.translation.y,
                p[2] + t.translation.z,
            ]);
        }
        if !transforms.is_empty() {
            out.push(BulkDraw {
                asset: draw.asset.clone(),
                material: draw.material,
                transforms,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_by_identity_and_quarter_turn() {
        assert_eq!(rotate([0.0, 0.0, 0.0, 1.0], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // A quarter turn about +Z takes +X to +Y.
        let r = rotate([0.0, 0.0, h, h], [1.0, 0.0, 0.0]);
        assert!((r[0]).abs() < 1e-6 && (r[1] - 1.0).abs() < 1e-6 && r[2].abs() < 1e-6);
    }

    #[test]
    fn set_replaces_or_appends() {
        let mut d = JobData::default();
        d.set("a", JobValues::F32(vec![1.0]));
        d.set("b", JobValues::I32(vec![2]));
        d.set("a", JobValues::F64(vec![3.0]));
        assert_eq!(d.names(), ["a", "b"]);
        assert_eq!(d.get("a"), Some(&JobValues::F64(vec![3.0])));
    }
}
