//! Skeletons, skins and animation clips from glTF files (CPU data only).
//!
//! A [`Rig`] holds every node of a glTF file (its rest transform and parent), the skins that
//! bind meshes to nodes, and the animation clips that move nodes. The animation module samples
//! clips into a pose (one model-space matrix per node); the renderer turns a pose into joint
//! matrices with [`Skin::inverse_bind`].

use std::path::Path;

/// One node of the glTF node tree, in its rest pose.
#[derive(Debug, Clone, PartialEq)]
pub struct RigNode {
    pub name: String,
    /// Index of the parent node, `None` for scene roots.
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    /// Unit quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// Binds a mesh to nodes: vertex joint index `j` follows node `joints[j]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub name: String,
    /// Node index of each joint.
    pub joints: Vec<usize>,
    /// Column-major inverse bind matrix of each joint (identity when the file has none).
    pub inverse_bind: Vec<[f32; 16]>,
}

/// What a channel animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
    /// Morph target (blend shape) weights of the node's mesh.
    Weights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    /// Hermite spline; values are stored as (in-tangent, value, out-tangent) per key.
    CubicSpline,
}

/// Keyframes for one property of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    /// Key times in seconds, ascending.
    pub times: Vec<f32>,
    /// Flat values: `width` floats per key (three times that for cubic spline).
    pub values: Vec<f32>,
    /// Floats per value: 3 (translation, scale), 4 (rotation) or the morph target count.
    pub width: usize,
}

/// A named animation clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds: the last key time over all channels.
    pub duration: f32,
    pub channels: Vec<Channel>,
}

/// Nodes, skins and clips of one model file. Empty for built-in shapes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rig {
    pub nodes: Vec<RigNode>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    /// Default morph target weights per node (empty when the node's mesh has none).
    pub morph_weights: Vec<Vec<f32>>,
}

impl Rig {
    /// The clip called `name`, or `#<index>` for a clip by position (unnamed clips are listed
    /// that way).
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        if let Some(i) = name.strip_prefix('#').and_then(|i| i.parse::<usize>().ok()) {
            return self.clips.get(i);
        }
        self.clips.iter().find(|c| c.name == name)
    }

    /// The node called `name`.
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Nodes listed so that every parent comes before its children.
    pub fn order(&self) -> Vec<usize> {
        let mut depth = vec![0usize; self.nodes.len()];
        for (i, d) in depth.iter_mut().enumerate() {
            let mut p = self.nodes[i].parent;
            let mut guard = 0;
            while let Some(q) = p {
                *d += 1;
                p = self.nodes[q].parent;
                guard += 1;
                if guard > self.nodes.len() {
                    break;
                }
            }
        }
        let mut order: Vec<usize> = (0..self.nodes.len()).collect();
        order.sort_by_key(|&i| (depth[i], i));
        order
    }

    /// Model-space matrix of every node in its rest pose.
    pub fn rest_pose(&self) -> Vec<[f32; 16]> {
        let local: Vec<[f32; 16]> = self
            .nodes
            .iter()
            .map(|n| trs(n.translation, n.rotation, n.scale))
            .collect();
        self.to_model_space(&local)
    }

    /// Turns local node matrices into model-space ones (parent times child).
    pub fn to_model_space(&self, local: &[[f32; 16]]) -> Vec<[f32; 16]> {
        let mut out = local.to_vec();
        for i in self.order() {
            if let Some(p) = self.nodes[i].parent {
                out[i] = mul(&out[p], &local[i]);
            }
        }
        out
    }
}

/// Column-major matrix from translation, rotation (unit quaternion) and scale. Plain
/// arithmetic only, so the result is the same on every platform.
pub fn trs(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> [f32; 16] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    [
        (1.0 - (yy + zz)) * s[0],
        (xy + wz) * s[0],
        (xz - wy) * s[0],
        0.0,
        (xy - wz) * s[1],
        (1.0 - (xx + zz)) * s[1],
        (yz + wx) * s[1],
        0.0,
        (xz + wy) * s[2],
        (yz - wx) * s[2],
        (1.0 - (xx + yy)) * s[2],
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

/// Column-major 4x4 product `a * b`, summed in a fixed order.
pub fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = a[row] * b[c * 4]
                + a[4 + row] * b[c * 4 + 1]
                + a[8 + row] * b[c * 4 + 2]
                + a[12 + row] * b[c * 4 + 3];
        }
    }
    r
}

pub const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Loads only the rig of a model (no images, no meshes). Built-in shapes have an empty rig.
pub fn load_rig(asset: &str, base_dir: &Path) -> Result<Rig, String> {
    if asset.starts_with("primitive:") {
        return Ok(Rig::default());
    }
    let path = base_dir.join(asset);
    let gltf = gltf::Gltf::open(&path).map_err(|e| format!("cannot load glTF {}: {e}", path.display()))?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone())
        .map_err(|e| format!("cannot load glTF buffers of {}: {e}", path.display()))?;
    rig_from(&gltf.document, &buffers)
}

/// Builds the rig of a parsed glTF document.
pub(crate) fn rig_from(doc: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Result<Rig, String> {
    let get = |b: gltf::Buffer| buffers.get(b.index()).map(|d| &d.0[..]);
    let mut rig = Rig {
        nodes: doc
            .nodes()
            .map(|n| {
                let (translation, rotation, scale) = n.transform().decomposed();
                RigNode {
                    name: n.name().map_or_else(|| format!("#{}", n.index()), str::to_string),
                    parent: None,
                    translation,
                    rotation,
                    scale,
                }
            })
            .collect(),
        ..Default::default()
    };
    rig.morph_weights = doc
        .nodes()
        .map(|n| {
            n.weights()
                .map(<[f32]>::to_vec)
                .or_else(|| n.mesh().and_then(|m| m.weights().map(<[f32]>::to_vec)))
                .or_else(|| {
                    n.mesh().map(|m| {
                        let targets = m.primitives().map(|p| p.morph_targets().len()).max().unwrap_or(0);
                        vec![0.0; targets]
                    })
                })
                .unwrap_or_default()
        })
        .collect();
    for n in doc.nodes() {
        for c in n.children() {
            rig.nodes[c.index()].parent = Some(n.index());
        }
    }
    for skin in doc.skins() {
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let mut inverse_bind: Vec<[f32; 16]> = match skin.reader(get).read_inverse_bind_matrices() {
            Some(m) => m.map(|m| flatten(&m)).collect(),
            None => Vec::new(),
        };
        inverse_bind.resize(joints.len(), IDENTITY);
        rig.skins.push(Skin {
            name: skin.name().unwrap_or_default().to_string(),
            joints,
            inverse_bind,
        });
    }
    for (index, anim) in doc.animations().enumerate() {
        let mut clip = Clip {
            name: anim.name().map_or_else(|| format!("#{index}"), str::to_string),
            duration: 0.0,
            channels: Vec::new(),
        };
        for ch in anim.channels() {
            let reader = ch.reader(get);
            let times: Vec<f32> = reader
                .read_inputs()
                .ok_or("animation channel has no key times")?
                .collect();
            use gltf::animation::util::ReadOutputs;
            let (property, values, width): (Property, Vec<f32>, usize) =
                match reader.read_outputs().ok_or("animation channel has no values")? {
                    ReadOutputs::Translations(v) => (Property::Translation, v.flatten().collect(), 3),
                    ReadOutputs::Scales(v) => (Property::Scale, v.flatten().collect(), 3),
                    ReadOutputs::Rotations(v) => (Property::Rotation, v.into_f32().flatten().collect(), 4),
                    ReadOutputs::MorphTargetWeights(v) => {
                        let vals: Vec<f32> = v.into_f32().collect();
                        let keys = times.len().max(1);
                        let per =
                            if ch.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline {
                                vals.len() / (keys * 3)
                            } else {
                                vals.len() / keys
                            };
                        (Property::Weights, vals, per)
                    }
                };
            let interpolation = match ch.sampler().interpolation() {
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let per_key = if interpolation == Interpolation::CubicSpline {
                width * 3
            } else {
                width
            };
            if times.is_empty() || width == 0 || values.len() != times.len() * per_key {
                continue; // malformed channel: skip it rather than fail the whole model
            }
            if let Some(&last) = times.last()
                && last > clip.duration
            {
                clip.duration = last;
            }
            clip.channels.push(Channel {
                node: ch.target().node().index(),
                property,
                interpolation,
                times,
                values,
                width,
            });
        }
        // Fixed channel order (node, property) so sampling order never depends on the file.
        clip.channels.sort_by_key(|c| (c.node, c.property as u8));
        rig.clips.push(clip);
    }
    Ok(rig)
}

fn flatten(m: &[[f32; 4]; 4]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for (c, col) in m.iter().enumerate() {
        out[c * 4..c * 4 + 4].copy_from_slice(col);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trs_matches_glam() {
        let q = glam::Quat::from_rotation_y(0.7) * glam::Quat::from_rotation_x(-0.3);
        let m = trs([1.0, 2.0, 3.0], q.to_array(), [2.0, 0.5, 1.5]);
        let g = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(2.0, 0.5, 1.5),
            q,
            glam::Vec3::new(1.0, 2.0, 3.0),
        );
        for (a, b) in m.iter().zip(g.to_cols_array()) {
            assert!((a - b).abs() < 1e-6, "{m:?} vs {g:?}");
        }
        let p = mul(&m, &m);
        let gp = (g * g).to_cols_array();
        for (a, b) in p.iter().zip(gp) {
            assert!((a - b).abs() < 1e-4);
        }
    }
}
