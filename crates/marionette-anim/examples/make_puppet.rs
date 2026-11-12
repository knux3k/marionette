//! Writes `examples/assets/puppet.glb`: a self-made, rigged test character (CC0, no outside
//! source) with three clips (`idle`, `walk`, `wave`), a rigidly attached hat (an unskinned
//! node under the head bone) and one morph target (`squash`, used by `wave`).
//!
//! Run: `cargo run -p marionette-anim --example make_puppet -- examples/assets/puppet.glb`.
//! The output is byte-identical every run.

use serde_json::{Value, json};
use std::f32::consts::PI;

#[derive(Clone, Copy)]
struct Bone {
    name: &'static str,
    parent: Option<usize>,
    /// Rest translation relative to the parent.
    offset: [f32; 3],
}

const BONES: [Bone; 15] = [
    Bone {
        name: "hips",
        parent: None,
        offset: [0.0, 1.0, 0.0],
    },
    Bone {
        name: "spine",
        parent: Some(0),
        offset: [0.0, 0.12, 0.0],
    },
    Bone {
        name: "chest",
        parent: Some(1),
        offset: [0.0, 0.25, 0.0],
    },
    Bone {
        name: "neck",
        parent: Some(2),
        offset: [0.0, 0.2, 0.0],
    },
    Bone {
        name: "head",
        parent: Some(3),
        offset: [0.0, 0.08, 0.0],
    },
    Bone {
        name: "upper_arm.L",
        parent: Some(2),
        offset: [0.21, 0.15, 0.0],
    },
    Bone {
        name: "lower_arm.L",
        parent: Some(5),
        offset: [0.0, -0.3, 0.0],
    },
    Bone {
        name: "upper_arm.R",
        parent: Some(2),
        offset: [-0.21, 0.15, 0.0],
    },
    Bone {
        name: "lower_arm.R",
        parent: Some(7),
        offset: [0.0, -0.3, 0.0],
    },
    Bone {
        name: "upper_leg.L",
        parent: Some(0),
        offset: [0.1, -0.04, 0.0],
    },
    Bone {
        name: "lower_leg.L",
        parent: Some(9),
        offset: [0.0, -0.46, 0.0],
    },
    Bone {
        name: "foot.L",
        parent: Some(10),
        offset: [0.0, -0.44, 0.0],
    },
    Bone {
        name: "upper_leg.R",
        parent: Some(0),
        offset: [-0.1, -0.04, 0.0],
    },
    Bone {
        name: "lower_leg.R",
        parent: Some(12),
        offset: [0.0, -0.46, 0.0],
    },
    Bone {
        name: "foot.R",
        parent: Some(13),
        offset: [0.0, -0.44, 0.0],
    },
];

fn bone(name: &str) -> usize {
    BONES.iter().position(|b| b.name == name).expect("bone exists")
}

fn rest_position(i: usize) -> [f32; 3] {
    let b = BONES[i];
    let p = b.parent.map_or([0.0; 3], rest_position);
    [p[0] + b.offset[0], p[1] + b.offset[1], p[2] + b.offset[2]]
}

#[derive(Default)]
struct Mesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    indices: Vec<u32>,
    squash: Vec<[f32; 3]>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A closed tube from `a` to `b` around bone `bone`, blending into `parent` near `a`.
/// `radius(t)` gives the radius along the tube (t from 0 at `a` to 1 at `b`).
#[allow(clippy::too_many_arguments)]
fn tube(
    m: &mut Mesh,
    a: [f32; 3],
    b: [f32; 3],
    bone: usize,
    parent: Option<usize>,
    child: Option<usize>,
    radius: &dyn Fn(f32) -> f32,
    squash: bool,
) {
    const SIDES: u32 = 12;
    const RINGS: u32 = 8;
    let axis = norm(sub(b, a));
    let helper = if axis[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = norm(cross(axis, helper));
    let v = cross(axis, u);
    let len = {
        let d = sub(b, a);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let skin = |t: f32| -> ([u16; 4], [f32; 4]) {
        // Smooth hand-over to the parent in the first 25 % and to the child in the last 15 %.
        if let Some(p) = parent
            && t < 0.25
        {
            let w = 0.5 + 2.0 * t;
            return ([bone as u16, p as u16, 0, 0], [w, 1.0 - w, 0.0, 0.0]);
        }
        if let Some(c) = child
            && t > 0.85
        {
            let w = 1.0 - (t - 0.85) / 0.15 * 0.5;
            return ([bone as u16, c as u16, 0, 0], [w, 1.0 - w, 0.0, 0.0]);
        }
        ([bone as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
    };
    let base = m.positions.len() as u32;
    for r in 0..=RINGS {
        let t = r as f32 / RINGS as f32;
        let rad = radius(t);
        let center = [
            a[0] + axis[0] * len * t,
            a[1] + axis[1] * len * t,
            a[2] + axis[2] * len * t,
        ];
        let (j, w) = skin(t);
        for s in 0..SIDES {
            let ang = s as f32 / SIDES as f32 * 2.0 * PI;
            let (c, sn) = (ang.cos(), ang.sin());
            let n = [u[0] * c + v[0] * sn, u[1] * c + v[1] * sn, u[2] * c + v[2] * sn];
            m.positions.push([
                center[0] + n[0] * rad,
                center[1] + n[1] * rad,
                center[2] + n[2] * rad,
            ]);
            m.normals.push(n);
            m.joints.push(j);
            m.weights.push(w);
            // Squash: wider and flatter (a cartoon "squish" of the head).
            m.squash.push(if squash {
                [n[0] * rad * 0.35, -(t - 0.5) * len * 0.4, n[2] * rad * 0.35]
            } else {
                [0.0; 3]
            });
        }
    }
    for r in 0..RINGS {
        for s in 0..SIDES {
            let i0 = base + r * SIDES + s;
            let i1 = base + r * SIDES + (s + 1) % SIDES;
            let i2 = i0 + SIDES;
            let i3 = i1 + SIDES;
            m.indices.extend_from_slice(&[i0, i1, i3, i0, i3, i2]);
        }
    }
    // Caps.
    for (ring, t, dir) in [(0, 0.0, -1.0f32), (RINGS, 1.0, 1.0)] {
        let center = [
            a[0] + axis[0] * len * t,
            a[1] + axis[1] * len * t,
            a[2] + axis[2] * len * t,
        ];
        let c = m.positions.len() as u32;
        let (j, w) = skin(t);
        m.positions.push(center);
        m.normals.push(axis.map(|x| x * dir));
        m.joints.push(j);
        m.weights.push(w);
        m.squash.push([0.0; 3]);
        for s in 0..SIDES {
            let i0 = base + ring * SIDES + s;
            let i1 = base + ring * SIDES + (s + 1) % SIDES;
            if dir < 0.0 {
                m.indices.extend_from_slice(&[c, i1, i0]);
            } else {
                m.indices.extend_from_slice(&[c, i0, i1]);
            }
        }
    }
}

/// One animation channel: node, glTF path, key times, values per key.
type Track<'a> = (usize, &'a str, Vec<f32>, Vec<Vec<f32>>);

fn quat(axis: [f32; 3], degrees: f32) -> [f32; 4] {
    let h = degrees.to_radians() * 0.5;
    let a = norm(axis);
    [a[0] * h.sin(), a[1] * h.sin(), a[2] * h.sin(), h.cos()]
}

/// Binary buffer builder: appends data aligned to 4 bytes and records views and accessors.
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn push(
        &mut self,
        bytes: &[u8],
        count: usize,
        ctype: u32,
        ty: &str,
        extra: Value,
        target: Option<u32>,
    ) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let mut view = json!({ "buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            view["target"] = json!(t);
        }
        self.data.extend_from_slice(bytes);
        self.views.push(view);
        let mut acc =
            json!({ "bufferView": self.views.len() - 1, "componentType": ctype, "count": count, "type": ty });
        if let (Value::Object(a), Value::Object(e)) = (&mut acc, extra) {
            a.extend(e);
        }
        self.accessors.push(acc);
        self.accessors.len() - 1
    }
    fn f32s(&mut self, v: &[f32], width: usize, ty: &str, min_max: bool, target: Option<u32>) -> usize {
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        let extra = if min_max {
            let mut min = vec![f32::MAX; width];
            let mut max = vec![f32::MIN; width];
            for c in v.chunks(width) {
                for k in 0..width {
                    min[k] = min[k].min(c[k]);
                    max[k] = max[k].max(c[k]);
                }
            }
            json!({ "min": min, "max": max })
        } else {
            json!({})
        };
        self.push(&bytes, v.len() / width, 5126, ty, extra, target)
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/assets/puppet.glb".into());

    // Body (blue-grey) and skin parts (warm), as two primitives of one skinned mesh.
    let mut body = Mesh::default();
    let mut skin = Mesh::default();
    let p = rest_position;
    let at = |name: &str| p(bone(name));
    let r = |r0: f32, r1: f32| move |t: f32| r0 + (r1 - r0) * t;
    // Torso: hips to chest, chest to neck.
    tube(
        &mut body,
        sub(at("hips"), [0.0, 0.08, 0.0]),
        at("chest"),
        bone("spine"),
        None,
        Some(bone("chest")),
        &r(0.15, 0.16),
        false,
    );
    tube(
        &mut body,
        at("chest"),
        at("neck"),
        bone("chest"),
        Some(bone("spine")),
        None,
        &|t| 0.17 - 0.07 * t * t,
        false,
    );
    tube(
        &mut skin,
        at("neck"),
        at("head"),
        bone("neck"),
        Some(bone("chest")),
        None,
        &r(0.05, 0.05),
        false,
    );
    // Head: a rounded tube.
    let head_top = sub(at("head"), [0.0, -0.26, 0.0]);
    tube(
        &mut skin,
        at("head"),
        head_top,
        bone("head"),
        None,
        None,
        &|t| 0.03 + 0.11 * (t * PI).sin().sqrt(),
        true,
    );
    for side in ["L", "R"] {
        let b = |n: &str| bone(&format!("{n}.{side}"));
        let a = |n: &str| at(&format!("{n}.{side}"));
        let hand = sub(a("lower_arm"), [0.0, 0.28, 0.0]);
        tube(
            &mut body,
            a("upper_arm"),
            a("lower_arm"),
            b("upper_arm"),
            Some(bone("chest")),
            Some(b("lower_arm")),
            &r(0.055, 0.048),
            false,
        );
        tube(
            &mut skin,
            a("lower_arm"),
            hand,
            b("lower_arm"),
            Some(b("upper_arm")),
            None,
            &r(0.046, 0.04),
            false,
        );
        tube(
            &mut body,
            a("upper_leg"),
            a("lower_leg"),
            b("upper_leg"),
            Some(bone("hips")),
            Some(b("lower_leg")),
            &r(0.08, 0.065),
            false,
        );
        tube(
            &mut body,
            a("lower_leg"),
            a("foot"),
            b("lower_leg"),
            Some(b("upper_leg")),
            None,
            &r(0.062, 0.05),
            false,
        );
        let toe = [a("foot")[0], 0.03, 0.16];
        tube(
            &mut skin,
            sub(a("foot"), [0.0, 0.0, 0.04]),
            toe,
            b("foot"),
            Some(b("lower_leg")),
            None,
            &r(0.045, 0.035),
            false,
        );
    }

    let mut bin = Bin {
        data: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut primitives = Vec::new();
    for (m, material) in [(&body, 0), (&skin, 1)] {
        let pos = bin.f32s(&m.positions.concat(), 3, "VEC3", true, Some(34962));
        let nrm = bin.f32s(&m.normals.concat(), 3, "VEC3", false, Some(34962));
        let jb: Vec<u8> = m.joints.iter().flatten().flat_map(|j| j.to_le_bytes()).collect();
        let jnt = bin.push(&jb, m.joints.len(), 5123, "VEC4", json!({}), Some(34962));
        let wgt = bin.f32s(&m.weights.concat(), 4, "VEC4", false, Some(34962));
        let sq = bin.f32s(&m.squash.concat(), 3, "VEC3", true, Some(34962));
        let ib: Vec<u8> = m.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let idx = bin.push(&ib, m.indices.len(), 5125, "SCALAR", json!({}), Some(34963));
        primitives.push(json!({
            "attributes": { "POSITION": pos, "NORMAL": nrm, "JOINTS_0": jnt, "WEIGHTS_0": wgt },
            "targets": [{ "POSITION": sq }],
            "indices": idx,
            "material": material,
        }));
    }

    // Hat: an unskinned cone under the head bone (moves rigidly with the head's node).
    let mut hat = Mesh::default();
    tube(
        &mut hat,
        [0.0, 0.2, 0.0],
        [0.0, 0.42, 0.0],
        0,
        None,
        None,
        &|t| 0.13 * (1.0 - t) + 0.005,
        false,
    );
    let hpos = bin.f32s(&hat.positions.concat(), 3, "VEC3", true, Some(34962));
    let hnrm = bin.f32s(&hat.normals.concat(), 3, "VEC3", false, Some(34962));
    let hib: Vec<u8> = hat.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let hidx = bin.push(&hib, hat.indices.len(), 5125, "SCALAR", json!({}), Some(34963));

    // Inverse bind matrices: the rest pose has no rotations, so each is a translation.
    let ibm: Vec<f32> = (0..BONES.len())
        .flat_map(|i| {
            let t = rest_position(i);
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -t[0], -t[1], -t[2], 1.0,
            ]
        })
        .collect();
    let ibm_acc = bin.f32s(&ibm, 16, "MAT4", false, None);

    // Nodes: 0..15 bones, 15 = skinned mesh node, 16 = hat (child of head).
    let mut nodes: Vec<Value> = BONES
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let children: Vec<usize> = BONES
                .iter()
                .enumerate()
                .filter(|(_, c)| c.parent == Some(i))
                .map(|(j, _)| j)
                .collect();
            let mut n = json!({ "name": b.name, "translation": b.offset });
            if !children.is_empty() {
                n["children"] = json!(children);
            }
            n
        })
        .collect();
    nodes[bone("head")]["children"] = json!([16]);
    nodes.push(json!({ "name": "body", "mesh": 0, "skin": 0 }));
    nodes.push(json!({ "name": "hat", "mesh": 1 }));

    // Animations.
    let x = [1.0, 0.0, 0.0];
    let y = [0.0, 1.0, 0.0];
    let z = [0.0, 0.0, 1.0];
    let mut animations = Vec::new();
    let mut add_clip = |bin: &mut Bin, name: &str, tracks: Vec<Track>| {
        let mut samplers = Vec::new();
        let mut channels = Vec::new();
        for (node, path, times, values) in tracks {
            let input = bin.f32s(&times, 1, "SCALAR", true, None);
            let width = values[0].len();
            let ty = match (path, width) {
                ("rotation", _) => "VEC4",
                ("weights", _) => "SCALAR",
                _ => "VEC3",
            };
            let output = bin.f32s(
                &values.concat(),
                if path == "weights" { 1 } else { width },
                ty,
                false,
                None,
            );
            samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
            channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": node, "path": path } }));
        }
        animations.push(json!({ "name": name, "samplers": samplers, "channels": channels }));
    };
    let rot = |axis: [f32; 3], degs: &[f32]| -> Vec<Vec<f32>> {
        degs.iter().map(|&d| quat(axis, d).to_vec()).collect()
    };
    let t5 = vec![0.0, 0.25, 0.5, 0.75, 1.0];
    let t3 = vec![0.0, 1.0, 2.0];

    add_clip(
        &mut bin,
        "idle",
        vec![
            (bone("chest"), "rotation", t3.clone(), rot(x, &[0.0, 4.0, 0.0])),
            (bone("head"), "rotation", t3.clone(), rot(x, &[0.0, -6.0, 0.0])),
            (
                bone("upper_arm.L"),
                "rotation",
                t3.clone(),
                rot(z, &[8.0, 12.0, 8.0]),
            ),
            (
                bone("upper_arm.R"),
                "rotation",
                t3.clone(),
                rot(z, &[-8.0, -12.0, -8.0]),
            ),
            (
                bone("lower_arm.L"),
                "rotation",
                t3.clone(),
                rot(x, &[-10.0, -16.0, -10.0]),
            ),
            (
                bone("lower_arm.R"),
                "rotation",
                t3.clone(),
                rot(x, &[-10.0, -16.0, -10.0]),
            ),
        ],
    );
    add_clip(
        &mut bin,
        "walk",
        vec![
            (
                bone("hips"),
                "translation",
                t5.clone(),
                [1.0, 0.96, 1.0, 0.96, 1.0]
                    .iter()
                    .map(|&h| vec![0.0, h, 0.0])
                    .collect(),
            ),
            (
                bone("chest"),
                "rotation",
                t5.clone(),
                rot(y, &[6.0, 0.0, -6.0, 0.0, 6.0]),
            ),
            (
                bone("upper_leg.L"),
                "rotation",
                t5.clone(),
                rot(x, &[-28.0, 0.0, 28.0, 0.0, -28.0]),
            ),
            (
                bone("upper_leg.R"),
                "rotation",
                t5.clone(),
                rot(x, &[28.0, 0.0, -28.0, 0.0, 28.0]),
            ),
            (
                bone("lower_leg.L"),
                "rotation",
                t5.clone(),
                rot(x, &[5.0, 10.0, 15.0, 50.0, 5.0]),
            ),
            (
                bone("lower_leg.R"),
                "rotation",
                t5.clone(),
                rot(x, &[15.0, 50.0, 5.0, 10.0, 15.0]),
            ),
            (
                bone("upper_arm.L"),
                "rotation",
                t5.clone(),
                rot(x, &[22.0, 0.0, -22.0, 0.0, 22.0]),
            ),
            (
                bone("upper_arm.R"),
                "rotation",
                t5.clone(),
                rot(x, &[-22.0, 0.0, 22.0, 0.0, -22.0]),
            ),
            (
                bone("lower_arm.L"),
                "rotation",
                t5.clone(),
                rot(x, &[-25.0, -15.0, -10.0, -15.0, -25.0]),
            ),
            (
                bone("lower_arm.R"),
                "rotation",
                t5.clone(),
                rot(x, &[-10.0, -15.0, -25.0, -15.0, -10.0]),
            ),
        ],
    );
    add_clip(
        &mut bin,
        "wave",
        vec![
            (
                bone("upper_arm.R"),
                "rotation",
                t5.clone(),
                rot(z, &[-150.0, -155.0, -150.0, -155.0, -150.0]),
            ),
            (
                bone("lower_arm.R"),
                "rotation",
                t5.clone(),
                rot(z, &[-25.0, 20.0, -25.0, 20.0, -25.0]),
            ),
            (
                bone("upper_arm.L"),
                "rotation",
                t5.clone(),
                rot(z, &[8.0, 10.0, 8.0, 10.0, 8.0]),
            ),
            (
                bone("head"),
                "rotation",
                t5.clone(),
                rot(z, &[0.0, 8.0, 0.0, 8.0, 0.0]),
            ),
            (
                BONES.len(),
                "weights",
                t5.clone(),
                [0.0, 0.6, 0.0, 0.6, 0.0].iter().map(|&w| vec![w]).collect(),
            ),
        ],
    );

    let gltf = json!({
        "asset": { "version": "2.0", "generator": "marionette make_puppet example", "copyright": "CC0-1.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0, 15] }],
        "nodes": nodes,
        "meshes": [
            { "name": "body", "primitives": primitives, "weights": [0.0], "extras": { "targetNames": ["squash"] } },
            { "name": "hat", "primitives": [{ "attributes": { "POSITION": hpos, "NORMAL": hnrm }, "indices": hidx, "material": 2 }] },
        ],
        "skins": [{ "name": "puppet", "joints": (0..BONES.len()).collect::<Vec<_>>(), "inverseBindMatrices": ibm_acc, "skeleton": 0 }],
        "materials": [
            { "name": "suit", "pbrMetallicRoughness": { "baseColorFactor": [0.18, 0.32, 0.62, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.6 } },
            { "name": "skin", "pbrMetallicRoughness": { "baseColorFactor": [0.93, 0.62, 0.42, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.7 } },
            { "name": "hat", "pbrMetallicRoughness": { "baseColorFactor": [0.85, 0.15, 0.12, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.5 } },
        ],
        "animations": animations,
        "accessors": bin.accessors,
        "bufferViews": bin.views,
        "buffers": [{ "byteLength": bin.data.len() }],
    });

    let mut json_bytes = serde_json::to_vec(&gltf).expect("serializes");
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut data = bin.data;
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + 8 + data.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(data.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&data);
    std::fs::write(&out, glb).expect("writes the output file");
    println!("wrote {out}");
}
