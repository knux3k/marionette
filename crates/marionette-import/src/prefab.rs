//! Unity prefabs to Marionette prefabs (decision 0040).
//!
//! A Unity `.prefab` (text serialization) is read into a tree of game objects, nested prefab
//! instances and prefab variants are expanded with their overrides applied, and the tree is
//! flattened into entities whose transforms are relative to the prefab root, converted to
//! Marionette axes. The result is a `prefab.json` (format `marionette.prefab/1`) stored as an
//! owned-library asset; `prefab_instantiate` places it into a scene.
//!
//! Marionette has no parent-child transforms, so each entity keeps the name path of its
//! hierarchy and the key of its parent for reference only. What has no Marionette equivalent
//! is listed per entity (`unity.unmapped`) and never silently dropped.
//!
//! Axes: Unity is left-handed (+Z forward), Marionette right-handed (-Z forward): positions
//! (x, y, z) -> (x, y, -z), rotations (x, y, z, w) -> (-x, -y, z, w). Unity imports a model by
//! mirroring its X axis, so a converted glTF model needs an extra half turn about Y to face the
//! way it faced in Unity.

use crate::unity_yaml::{self, UnityDoc, num, object_ref, ref_file_id};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PREFAB_FORMAT: &str = "marionette.prefab/1";

/// Unity's built-in meshes and materials live in this pseudo-asset.
const BUILTIN_EXTRA: &str = "0000000000000000e000000000000000";
const BUILTIN_DEFAULT: &str = "0000000000000000f000000000000000";
/// Nested prefabs deeper than this are not expanded (Unity itself warns far earlier).
const MAX_DEPTH: usize = 16;

/// What a GUID in a prefab points to.
#[derive(Debug, Clone)]
pub enum Resolved {
    /// Another prefab in the same source, allowed under the prefab's own license.
    Prefab { rel: String, text: String },
    /// A model in the owned library (this import or an earlier one).
    Model { id: String, main: String, name: String },
    /// Known but not usable, with the reason (refused, not converted, ...).
    Refused(String),
    /// Not in the source or the library.
    Unknown,
}

/// The PBR factors of a library material, for meshes that take a `Material` component.
#[derive(Debug, Clone)]
pub struct MaterialInfo {
    pub id: String,
    pub name: String,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
}

/// What the converter may look up.
pub trait PrefabEnv {
    fn resolve(&self, guid: &str) -> Resolved;
    fn material(&self, guid: &str) -> Option<MaterialInfo>;
    /// Path inside the source of the file with this GUID (scripts, audio clips), if any.
    fn asset_path(&self, guid: &str) -> Option<String>;
}

/// A converted prefab.
#[derive(Debug, Clone)]
pub struct Converted {
    /// The `prefab.json` document.
    pub doc: Value,
    /// Library ids the prefab refers to.
    pub uses: Vec<String>,
    /// Prefab-wide notes (also in the document).
    pub notes: Vec<String>,
    pub entities: usize,
}

// ---------------------------------------------------------------------------------------
// Math (f64; runs at import time only, never per tick).

type V3 = [f64; 3];
type Q = [f64; 4];

fn qmul(a: Q, b: Q) -> Q {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn qnorm(q: Q) -> Q {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-12 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }
}

fn qrot(q: Q, v: V3) -> V3 {
    let r = qmul(qmul(q, [v[0], v[1], v[2], 0.0]), [-q[0], -q[1], -q[2], q[3]]);
    [r[0], r[1], r[2]]
}

#[derive(Debug, Clone, Copy)]
pub struct Trs {
    pub t: V3,
    pub r: Q,
    pub s: V3,
}

impl Trs {
    pub const IDENTITY: Trs = Trs {
        t: [0.0; 3],
        r: [0.0, 0.0, 0.0, 1.0],
        s: [1.0; 3],
    };

    /// `self` (parent) applied after `local` (child). Exact for uniform parent scale; with a
    /// non-uniform parent scale and a rotated child the shear is dropped (`sheared` says so).
    pub fn then(&self, local: &Trs) -> (Trs, bool) {
        let scaled = [
            local.t[0] * self.s[0],
            local.t[1] * self.s[1],
            local.t[2] * self.s[2],
        ];
        let p = qrot(self.r, scaled);
        let uniform = (self.s[0] - self.s[1]).abs() < 1e-6 && (self.s[1] - self.s[2]).abs() < 1e-6;
        let rotated = 1.0 - local.r[3].abs() > 1e-6;
        (
            Trs {
                t: [p[0] + self.t[0], p[1] + self.t[1], p[2] + self.t[2]],
                r: qnorm(qmul(self.r, local.r)),
                s: [
                    self.s[0] * local.s[0],
                    self.s[1] * local.s[1],
                    self.s[2] * local.s[2],
                ],
            },
            !uniform && rotated,
        )
    }

    /// Unity axes to Marionette axes.
    pub fn from_unity(&self) -> Trs {
        Trs {
            t: [self.t[0], self.t[1], -self.t[2]],
            r: [-self.r[0], -self.r[1], self.r[2], self.r[3]],
            s: self.s,
        }
    }

    pub fn to_json(self) -> Value {
        let f = |v: f64| -> Value {
            let x = v as f32;
            // Keep -0 out of the files.
            json!(if x == 0.0 { 0.0f32 } else { x })
        };
        json!({
            "translation": {"x": f(self.t[0]), "y": f(self.t[1]), "z": f(self.t[2])},
            "rotation": {"x": f(self.r[0]), "y": f(self.r[1]), "z": f(self.r[2]), "w": f(self.r[3])},
            "scale": {"x": f(self.s[0]), "y": f(self.s[1]), "z": f(self.s[2])},
        })
    }

    pub fn from_json(v: &Value) -> Trs {
        let g = |o: &Value, k: &str, d: f64| o.get(k).and_then(Value::as_f64).unwrap_or(d);
        let t = &v["translation"];
        let r = &v["rotation"];
        let s = &v["scale"];
        Trs {
            t: [g(t, "x", 0.0), g(t, "y", 0.0), g(t, "z", 0.0)],
            r: qnorm([g(r, "x", 0.0), g(r, "y", 0.0), g(r, "z", 0.0), g(r, "w", 1.0)]),
            s: [g(s, "x", 1.0), g(s, "y", 1.0), g(s, "z", 1.0)],
        }
    }
}

/// Half a turn about +Y: turns a converted glTF model to face as it did in Unity.
const HALF_TURN_Y: Q = [0.0, 1.0, 0.0, 0.0];

fn v3(v: &Value, d: f64) -> V3 {
    let g = |k: &str| v.get(k).and_then(num).unwrap_or(d);
    [g("x"), g("y"), g("z")]
}

fn local_trs(tr: &Value) -> Trs {
    let r = &tr["m_LocalRotation"];
    let g = |k: &str, d: f64| r.get(k).and_then(num).unwrap_or(d);
    Trs {
        t: v3(&tr["m_LocalPosition"], 0.0),
        r: qnorm([g("x", 0.0), g("y", 0.0), g("z", 0.0), g("w", 1.0)]),
        s: v3(&tr["m_LocalScale"], 1.0),
    }
}

fn srgb_color(v: &Value) -> Value {
    let g = |k: &str, d: f64| v.get(k).and_then(num).unwrap_or(d) as f32;
    let lin = crate::unity::srgb_to_linear;
    json!({"r": lin(g("r", 1.0)), "g": lin(g("g", 1.0)), "b": lin(g("b", 1.0)), "a": g("a", 1.0)})
}

// ---------------------------------------------------------------------------------------
// The tree.

#[derive(Debug, Clone)]
struct ModelRef {
    id: String,
    main: String,
    name: String,
}

#[derive(Debug, Clone)]
struct Comp {
    class_id: u32,
    type_name: String,
    body: Value,
    removed: bool,
}

#[derive(Debug, Clone)]
struct Node {
    go: Value,
    tr: Value,
    comps: Vec<Comp>,
    parent: Option<usize>,
    /// Position in the parent's `m_Children`, when the parent lists it.
    order: Option<usize>,
    model: Option<ModelRef>,
    removed: bool,
    /// The prefab file that defines this object.
    origin: String,
    notes: Vec<String>,
}

impl Node {
    fn new(origin: &str) -> Self {
        Node {
            go: json!({}),
            tr: json!({}),
            comps: Vec::new(),
            parent: None,
            order: None,
            model: None,
            removed: false,
            origin: origin.to_string(),
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Go(usize),
    Tr(usize),
    Comp(usize, usize),
}

impl Target {
    fn shift(self, by: usize) -> Target {
        match self {
            Target::Go(n) => Target::Go(n + by),
            Target::Tr(n) => Target::Tr(n + by),
            Target::Comp(n, c) => Target::Comp(n + by, c),
        }
    }
}

#[derive(Debug, Default)]
struct Tree {
    nodes: Vec<Node>,
    ids: BTreeMap<i64, Target>,
    root: Option<usize>,
    notes: Vec<String>,
    uses: BTreeSet<String>,
}

/// The id Unity gives an object of a nested prefab inside the outer file.
fn nested_id(instance: i64, inner: i64) -> i64 {
    (instance ^ inner) & 0x7fff_ffff_ffff_ffff
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// Sets `path` (`m_Size.x`, `m_Materials.Array.data[0]`, `m_Materials.Array.size`) in `body`.
fn set_path(body: &mut Value, path: &str, value: Value) {
    let mut cur = body;
    let parts: Vec<&str> = path.split('.').collect();
    let mut i = 0;
    while i < parts.len() {
        let p = parts[i];
        let last = i + 1 == parts.len();
        if p == "Array" {
            i += 1;
            continue;
        }
        if p == "size" && i > 0 && parts[i - 1] == "Array" {
            let n = num(&value).unwrap_or(0.0).max(0.0) as usize;
            if !cur.is_array() {
                *cur = Value::Array(Vec::new());
            }
            let a = cur.as_array_mut().expect("array");
            a.resize(n.min(1 << 16), Value::String(String::new()));
            return;
        }
        if let Some(idx) = p
            .strip_prefix("data[")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.parse::<usize>().ok())
        {
            if !cur.is_array() {
                *cur = Value::Array(Vec::new());
            }
            let a = cur.as_array_mut().expect("array");
            if idx >= a.len() && idx < 1 << 16 {
                a.resize(idx + 1, Value::String(String::new()));
            }
            let Some(slot) = a.get_mut(idx) else { return };
            if last {
                *slot = value;
                return;
            }
            cur = slot;
            i += 1;
            continue;
        }
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let m = cur.as_object_mut().expect("object");
        if last {
            m.insert(p.to_string(), value);
            return;
        }
        cur = m.entry(p.to_string()).or_insert_with(|| json!({}));
        i += 1;
    }
}

struct Builder<'a> {
    env: &'a dyn PrefabEnv,
}

impl Builder<'_> {
    fn model_tree(&self, m: ModelRef, origin: &str) -> Tree {
        let mut t = Tree::default();
        let mut n = Node::new(origin);
        n.go = json!({"m_Name": m.name, "m_IsActive": "1"});
        t.uses.insert(m.id.clone());
        n.model = Some(m);
        t.nodes.push(n);
        t.root = Some(0);
        t
    }

    fn placeholder(&self, name: &str, why: String, origin: &str) -> Tree {
        let mut t = Tree::default();
        let mut n = Node::new(origin);
        n.go = json!({"m_Name": name, "m_IsActive": "1"});
        n.notes.push(why);
        t.nodes.push(n);
        t.root = Some(0);
        t
    }

    fn instance_source(&self, guid: Option<&str>, rel: &str, stack: &mut Vec<String>) -> Tree {
        let Some(guid) = guid else {
            return self.placeholder("missing prefab", "prefab instance names no source".into(), rel);
        };
        match self.env.resolve(guid) {
            Resolved::Prefab { rel: r, text } => {
                if stack.iter().any(|g| g == guid) {
                    return self.placeholder(
                        "recursive prefab",
                        format!("`{r}` contains itself; not expanded"),
                        rel,
                    );
                }
                if stack.len() >= MAX_DEPTH {
                    return self.placeholder(
                        "deep prefab",
                        format!("`{r}` is nested more than {MAX_DEPTH} levels deep; not expanded"),
                        rel,
                    );
                }
                stack.push(guid.to_string());
                let t = match self.build(&r, &text, stack) {
                    Ok(t) => t,
                    Err(e) => self.placeholder(
                        "unreadable prefab",
                        format!("nested prefab `{r}` not read: {e}"),
                        rel,
                    ),
                };
                stack.pop();
                t
            }
            Resolved::Model { id, main, name } => self.model_tree(ModelRef { id, main, name }, rel),
            Resolved::Refused(why) => self.placeholder("refused source", why, rel),
            Resolved::Unknown => self.placeholder(
                "missing source",
                format!("source asset {guid} is not in this source or the owned library"),
                rel,
            ),
        }
    }

    fn build(&self, rel: &str, text: &str, stack: &mut Vec<String>) -> Result<Tree, String> {
        let docs = unity_yaml::parse(text)?;
        let mut tree = Tree::default();

        // 1. Expand prefab instances (nested prefabs, models, and the base of a variant).
        struct Inst<'d> {
            doc: &'d UnityDoc,
            shift: usize,
            sub_ids: BTreeMap<i64, Target>,
            root: Option<usize>,
            is_model: bool,
        }
        let mut insts: Vec<Inst> = Vec::new();
        for d in docs.iter().filter(|d| d.class_id == 1001 && !d.stripped) {
            let guid = object_ref(&d.body["m_SourcePrefab"]).and_then(|(_, g)| g);
            let sub = self.instance_source(guid.as_deref(), rel, stack);
            let shift = tree.nodes.len();
            let is_model = sub.nodes.first().is_some_and(|n| n.model.is_some()) && sub.nodes.len() == 1;
            for mut n in sub.nodes {
                n.parent = n.parent.map(|p| p + shift);
                tree.nodes.push(n);
            }
            tree.notes.extend(sub.notes);
            tree.uses.extend(sub.uses);
            let sub_ids: BTreeMap<i64, Target> =
                sub.ids.into_iter().map(|(k, t)| (k, t.shift(shift))).collect();
            for (k, t) in &sub_ids {
                tree.ids.insert(nested_id(d.file_id, *k), *t);
            }
            insts.push(Inst {
                doc: d,
                shift,
                sub_ids,
                root: sub.root.map(|r| r + shift),
                is_model,
            });
        }

        // 2. Own game objects.
        for d in docs.iter().filter(|d| d.class_id == 1 && !d.stripped) {
            let mut n = Node::new(rel);
            n.go = d.body.clone();
            tree.nodes.push(n);
            tree.ids.insert(d.file_id, Target::Go(tree.nodes.len() - 1));
        }

        // 3. Stripped placeholders that the id rule did not already map.
        for d in docs.iter().filter(|d| d.stripped) {
            if tree.ids.contains_key(&d.file_id) {
                continue;
            }
            let p = ref_file_id(&d.body["m_PrefabInstance"]);
            let c = ref_file_id(&d.body["m_CorrespondingSourceObject"]);
            if let (Some(p), Some(c)) = (p, c)
                && let Some(inst) = insts.iter().find(|i| i.doc.file_id == p)
            {
                if let Some(t) = inst.sub_ids.get(&c) {
                    tree.ids.insert(d.file_id, *t);
                } else if let Some(r) = inst.root.filter(|_| inst.is_model) {
                    // A part inside a model: Marionette keeps the model whole.
                    let t = if matches!(d.class_id, 4 | 224) {
                        Target::Tr(r)
                    } else {
                        Target::Go(r)
                    };
                    tree.ids.insert(d.file_id, t);
                }
            }
        }

        // 4. Own transforms and components.
        for d in docs
            .iter()
            .filter(|d| !d.stripped && !matches!(d.class_id, 1 | 1001))
        {
            let Some(go) = ref_file_id(&d.body["m_GameObject"]) else {
                continue;
            };
            let Some(Target::Go(n) | Target::Tr(n)) = tree.ids.get(&go).copied() else {
                tree.notes.push(format!(
                    "{} &{} belongs to an unknown game object; skipped",
                    d.type_name, d.file_id
                ));
                continue;
            };
            if matches!(d.class_id, 4 | 224) {
                tree.nodes[n].tr = d.body.clone();
                tree.ids.insert(d.file_id, Target::Tr(n));
            } else {
                tree.nodes[n].comps.push(Comp {
                    class_id: d.class_id,
                    type_name: d.type_name.clone(),
                    body: d.body.clone(),
                    removed: false,
                });
                let c = tree.nodes[n].comps.len() - 1;
                tree.ids.insert(d.file_id, Target::Comp(n, c));
            }
        }

        // 5. Overrides of each instance.
        for inst in &insts {
            let m = &inst.doc.body["m_Modification"];
            let mods = m["m_Modifications"].as_array().cloned().unwrap_or_default();
            // For a model, overrides of the root transform move the whole model; Unity always
            // writes position and rotation overrides for an instance root.
            let model_root_target: Option<i64> = if inst.is_model {
                mods.iter()
                    .filter(|x| str_of(&x["propertyPath"]).starts_with("m_LocalPosition"))
                    .find_map(|x| ref_file_id(&x["target"]))
            } else {
                None
            };
            let mut skipped: BTreeSet<String> = BTreeSet::new();
            for x in &mods {
                let Some(c) = ref_file_id(&x["target"]) else {
                    continue;
                };
                let path = str_of(&x["propertyPath"]);
                let value = match object_ref(&x["objectReference"]) {
                    Some((id, g)) if id != 0 || g.is_some() => x["objectReference"].clone(),
                    _ => x["value"].clone(),
                };
                let target = inst.sub_ids.get(&c).copied().or_else(|| {
                    let r = inst.root?;
                    if Some(c) == model_root_target {
                        Some(Target::Tr(r))
                    } else if inst.is_model && path == "m_Name" && model_root_target.is_none() {
                        Some(Target::Go(r))
                    } else {
                        None
                    }
                });
                let Some(target) = target else {
                    if inst.is_model {
                        skipped.insert(format!("model part {c}: {path}"));
                    } else {
                        skipped.insert(format!("object {c}: {path}"));
                    }
                    continue;
                };
                // A model root's transform overrides go on the model node's transform even if
                // the target was mapped to its game object.
                let target = match target {
                    Target::Go(n) if path.starts_with("m_Local") => Target::Tr(n),
                    t => t,
                };
                match target {
                    Target::Go(n) => set_path(&mut tree.nodes[n].go, &path, value),
                    Target::Tr(n) => {
                        if path.starts_with("m_Local") || path == "m_RootOrder" {
                            set_path(&mut tree.nodes[n].tr, &path, value);
                        } else if path == "m_Name" || path == "m_IsActive" {
                            set_path(&mut tree.nodes[n].go, &path, value);
                        } else {
                            set_path(&mut tree.nodes[n].tr, &path, value);
                        }
                    }
                    Target::Comp(n, ci) => set_path(&mut tree.nodes[n].comps[ci].body, &path, value),
                }
            }
            if !skipped.is_empty() {
                let list: Vec<String> = skipped.into_iter().take(12).collect();
                tree.notes.push(format!(
                    "overrides not applied (their object is inside a model or missing): {}",
                    list.join("; ")
                ));
            }
            for r in m["m_RemovedComponents"].as_array().into_iter().flatten() {
                if let Some(c) = ref_file_id(r)
                    && let Some(Target::Comp(n, ci)) = inst.sub_ids.get(&c).copied()
                {
                    tree.nodes[n].comps[ci].removed = true;
                }
            }
            for r in m["m_RemovedGameObjects"].as_array().into_iter().flatten() {
                if let Some(c) = ref_file_id(r)
                    && let Some(Target::Go(n) | Target::Tr(n)) = inst.sub_ids.get(&c).copied()
                {
                    tree.nodes[n].removed = true;
                }
            }
            // Parent of the instance root.
            if let (Some(root), Some(p)) = (inst.root, ref_file_id(&m["m_TransformParent"]))
                && p != 0
            {
                match tree.ids.get(&p).copied() {
                    Some(Target::Go(pn) | Target::Tr(pn)) => tree.nodes[root].parent = Some(pn),
                    _ => tree.notes.push(format!(
                        "parent {p} of a nested prefab is unknown; placed at the root"
                    )),
                }
            }
            let _ = inst.shift;
        }

        // 6. Own hierarchy.
        for d in docs
            .iter()
            .filter(|d| matches!(d.class_id, 4 | 224) && !d.stripped)
        {
            let Some(Target::Tr(n)) = tree.ids.get(&d.file_id).copied() else {
                continue;
            };
            if let Some(f) = ref_file_id(&d.body["m_Father"]).filter(|f| *f != 0)
                && let Some(Target::Go(p) | Target::Tr(p)) = tree.ids.get(&f).copied()
            {
                tree.nodes[n].parent = Some(p);
            }
            for (i, c) in d.body["m_Children"].as_array().into_iter().flatten().enumerate() {
                if let Some(c) = ref_file_id(c)
                    && let Some(Target::Go(cn) | Target::Tr(cn)) = tree.ids.get(&c).copied()
                {
                    tree.nodes[cn].order = Some(i);
                    if tree.nodes[cn].parent.is_none() && cn != n {
                        tree.nodes[cn].parent = Some(n);
                    }
                }
            }
        }

        // 7. The root: an own transform with no father, else an instance placed at the root
        // (a prefab variant).
        tree.root = docs
            .iter()
            .filter(|d| matches!(d.class_id, 4 | 224) && !d.stripped)
            .find(|d| ref_file_id(&d.body["m_Father"]).unwrap_or(0) == 0)
            .and_then(|d| match tree.ids.get(&d.file_id) {
                Some(Target::Tr(n)) => Some(*n),
                _ => None,
            })
            .or_else(|| {
                insts
                    .iter()
                    .filter(|i| {
                        ref_file_id(&i.doc.body["m_Modification"]["m_TransformParent"]).unwrap_or(0) == 0
                    })
                    .find_map(|i| i.root)
            });
        if tree.root.is_none() {
            return Err("no root object found".into());
        }
        Ok(tree)
    }
}

// ---------------------------------------------------------------------------------------
// Mapping one node to Marionette components.

fn builtin_mesh(file_id: i64) -> Option<(&'static str, &'static str)> {
    Some(match file_id {
        10202 => ("Cube", "primitive:cube"),
        10207 => ("Sphere", "primitive:sphere"),
        10209 => ("Plane", "primitive:plane"),
        10206 => ("Cylinder", ""),
        10208 => ("Capsule", ""),
        10210 => ("Quad", ""),
        _ => return None,
    })
}

const STANDARD_KEYS: &[&str] = &[
    "m_ObjectHideFlags",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "m_GameObject",
    "m_Enabled",
    "m_EditorHideFlags",
    "m_Script",
    "m_Name",
    "m_EditorClassIdentifier",
    "serializedVersion",
];

fn enabled(c: &Comp) -> bool {
    c.body.get("m_Enabled").and_then(num).is_none_or(|v| v != 0.0)
}

fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}

struct Out {
    entities: Vec<Value>,
    notes: Vec<String>,
    uses: BTreeSet<String>,
}

struct Mapped {
    components: Map<String, Value>,
    /// Extra entities (offset colliders, models split off from cameras or lights).
    extra: Vec<(String, Trs, Map<String, Value>)>,
    scripts: Vec<Value>,
    unmapped: Vec<String>,
    notes: Vec<String>,
}

fn library_ref(id: &str, main: &str) -> String {
    format!("library:{id}/{main}")
}

/// Volume of a collider at world scale, for turning a Unity mass into a density.
fn shape_volume(shape: &Value, s: V3) -> f64 {
    let sc = s[0].abs() * s[1].abs() * s[2].abs();
    let pi = std::f64::consts::PI;
    if let Some(c) = shape.get("Cuboid") {
        let h = &c["half_extents"];
        let g = |k: &str| h[k].as_f64().unwrap_or(0.5);
        return 8.0 * g("x") * g("y") * g("z") * sc;
    }
    if let Some(b) = shape.get("Ball") {
        let r = b["radius"].as_f64().unwrap_or(0.5);
        return 4.0 / 3.0 * pi * r * r * r * sc;
    }
    if let Some(c) = shape.get("Capsule") {
        let r = c["radius"].as_f64().unwrap_or(0.5);
        let h = c["half_height"].as_f64().unwrap_or(0.5);
        return (pi * r * r * 2.0 * h + 4.0 / 3.0 * pi * r * r * r) * sc;
    }
    sc
}

fn map_node(
    node: &Node,
    world: &Trs,
    under_body: bool,
    env: &dyn PrefabEnv,
    uses: &mut BTreeSet<String>,
) -> Mapped {
    let mut out = Mapped {
        components: Map::new(),
        extra: Vec::new(),
        scripts: Vec::new(),
        unmapped: Vec::new(),
        notes: node.notes.clone(),
    };
    let comps: Vec<&Comp> = node.comps.iter().filter(|c| !c.removed).collect();
    let has = |id: u32| comps.iter().any(|c| c.class_id == id && enabled(c));
    let renderer = comps
        .iter()
        .find(|c| matches!(c.class_id, 23 | 137) && enabled(c))
        .copied();

    // Mesh: the node's own model, a MeshFilter, or a skinned mesh.
    let mut model: Option<(String, bool)> = None; // (asset, needs the half turn)
    let mut mesh_scale = 1.0;
    if let Some(m) = &node.model {
        uses.insert(m.id.clone());
        model = Some((library_ref(&m.id, &m.main), true));
    } else if let Some(r) = renderer {
        let mesh_ref = if r.class_id == 137 {
            object_ref(&r.body["m_Mesh"])
        } else {
            comps
                .iter()
                .find(|c| c.class_id == 33)
                .and_then(|c| object_ref(&c.body["m_Mesh"]))
        };
        match mesh_ref {
            Some((fid, Some(g))) if g == BUILTIN_EXTRA => match builtin_mesh(fid) {
                Some((_, asset)) if !asset.is_empty() => {
                    if asset == "primitive:plane" {
                        mesh_scale = 10.0;
                    }
                    model = Some((asset.to_string(), false));
                }
                Some((name, _)) => out.unmapped.push(format!(
                    "built-in {name} mesh (Marionette has cube, sphere and plane primitives)"
                )),
                None => out.unmapped.push(format!("built-in mesh {fid}")),
            },
            Some((fid, Some(g))) => match env.resolve(&g) {
                Resolved::Model { id, main, name } => {
                    uses.insert(id.clone());
                    out.notes.push(format!(
                        "mesh {fid} of model `{name}`: the whole model is drawn (sub-mesh selection is not supported)"
                    ));
                    model = Some((library_ref(&id, &main), true));
                }
                Resolved::Refused(why) => out.notes.push(format!("mesh not used: {why}")),
                _ => out.notes.push(format!(
                    "mesh source {g} is not in this source or the owned library"
                )),
            },
            _ => {
                if r.class_id == 137 || comps.iter().any(|c| c.class_id == 33) {
                    out.notes.push("renderer without a mesh".into());
                }
            }
        }
    }
    let world_m = world.from_unity();
    let needs_split = model.as_ref().is_some_and(|(_, turn)| *turn) && (has(20) || has(108));
    let model_transform = |base: &Trs| -> Trs {
        let mut t = *base;
        t.r = qnorm(qmul(t.r, HALF_TURN_Y));
        t
    };

    // Material: a primitive takes the first material as a `Material`; models keep theirs.
    if let Some(r) = renderer
        && let Some(mats) = r.body["m_Materials"].as_array()
    {
        let refs: Vec<(i64, String)> = mats
            .iter()
            .filter_map(object_ref)
            .filter_map(|(f, g)| g.map(|g| (f, g)))
            .collect();
        if refs.len() > 1 {
            out.notes.push(format!(
                "{} material slots; Marionette's `Material` overrides a whole model",
                refs.len()
            ));
        }
        if let Some((_, g)) = refs
            .first()
            .filter(|(_, g)| g != BUILTIN_DEFAULT && g != BUILTIN_EXTRA)
        {
            match env.material(g) {
                Some(m) => {
                    uses.insert(m.id.clone());
                    if model.as_ref().is_some_and(|(a, _)| a.starts_with("primitive:")) {
                        out.components.insert(
                            "Material".into(),
                            json!({
                                "base_color": {"r": m.base_color[0], "g": m.base_color[1], "b": m.base_color[2], "a": m.base_color[3]},
                                "metallic": m.metallic,
                                "roughness": m.roughness,
                            }),
                        );
                    } else if model.is_some() {
                        out.notes.push(format!(
                            "Unity material `{}` ({}); the model's own materials are drawn",
                            m.name, m.id
                        ));
                    }
                }
                None => out
                    .notes
                    .push(format!("material {g} is not in the owned library")),
            }
        }
    }

    if let Some((asset, turn)) = &model {
        let m = json!({ "asset": asset });
        if needs_split {
            let mut c = Map::new();
            c.insert("Model".into(), m);
            out.extra.push(("model".into(), model_transform(&world_m), c));
        } else {
            out.components.insert("Model".into(), m);
            if *turn {
                out.notes
                    .push("half turn about Y added: Unity mirrors imported models on X".into());
            }
        }
    }

    // Physics.
    let body = comps.iter().find(|c| c.class_id == 54).copied();
    let colliders: Vec<&Comp> = comps
        .iter()
        .filter(|c| matches!(c.class_id, 64 | 65 | 135 | 136) && enabled(c))
        .copied()
        .collect();
    let mut first_collider = true;
    for c in &colliders {
        if under_body && body.is_none() {
            out.unmapped.push(format!(
                "{} on a child of a Rigidbody (compound bodies are not supported)",
                c.type_name
            ));
            continue;
        }
        let b = &c.body;
        let center = v3(&b["m_Center"], 0.0);
        let (shape, note): (Option<Value>, Option<String>) = match c.class_id {
            65 => {
                let s = v3(&b["m_Size"], 1.0);
                (
                    Some(
                        json!({"Cuboid": {"half_extents": {"x": s[0] / 2.0, "y": s[1] / 2.0, "z": s[2] / 2.0}}}),
                    ),
                    None,
                )
            }
            135 => {
                let r = b.get("m_Radius").and_then(num).unwrap_or(0.5);
                let uniform =
                    (world.s[0] - world.s[1]).abs() < 1e-6 && (world.s[1] - world.s[2]).abs() < 1e-6;
                (
                    Some(json!({"Ball": {"radius": r}})),
                    (!uniform).then(|| {
                        "sphere collider under non-uniform scale: Unity uses the largest axis".into()
                    }),
                )
            }
            136 => {
                let r = b.get("m_Radius").and_then(num).unwrap_or(0.5);
                let h = b.get("m_Height").and_then(num).unwrap_or(2.0);
                let dir = b.get("m_Direction").and_then(num).unwrap_or(1.0) as i64;
                if dir == 1 {
                    (
                        Some(json!({"Capsule": {"half_height": (h / 2.0 - r).max(0.0), "radius": r}})),
                        None,
                    )
                } else {
                    (
                        None,
                        Some(format!(
                            "capsule along axis {dir} (Marionette capsules run along Y)"
                        )),
                    )
                }
            }
            64 => {
                let convex = b.get("m_Convex").and_then(num).unwrap_or(0.0) != 0.0;
                let asset = match object_ref(&b["m_Mesh"]) {
                    Some((_, Some(g))) => match env.resolve(&g) {
                        Resolved::Model { id, main, .. } => {
                            uses.insert(id.clone());
                            Some(library_ref(&id, &main))
                        }
                        _ => None,
                    },
                    _ => model
                        .as_ref()
                        .map(|(a, _)| a.clone())
                        .filter(|a| a.starts_with("library:")),
                };
                match asset {
                    Some(a) if convex => (Some(json!({"ConvexHull": {"model": a}})), None),
                    Some(a) => (Some(json!({"TriMesh": {"model": a}})), None),
                    None => (
                        None,
                        Some("mesh collider whose mesh is not a library model".into()),
                    ),
                }
            }
            _ => (None, None),
        };
        if let Some(n) = note {
            match &shape {
                Some(_) => out.notes.push(n),
                None => {
                    out.unmapped.push(format!("{}: {n}", c.type_name));
                    continue;
                }
            }
        }
        let Some(shape) = shape else { continue };
        if c.class_id == 64 {
            out.notes.push(
                "mesh collider: the converted model's half turn is not applied to the collider shape".into(),
            );
        }
        let collider = json!({
            "shape": shape,
            "friction": 0.6,
            "restitution": 0.0,
            "density": 1.0,
            "sensor": b.get("m_IsTrigger").and_then(num).unwrap_or(0.0) != 0.0,
        });
        if b.get("m_Material")
            .and_then(object_ref)
            .is_some_and(|(f, _)| f != 0)
        {
            out.notes
                .push("physic material not mapped; Unity default friction 0.6 and bounciness 0 used".into());
        }
        let offset = center.iter().any(|v| v.abs() > 1e-9);
        if first_collider && !offset && !out.components.contains_key("Collider") {
            out.components.insert("Collider".into(), collider);
            first_collider = false;
        } else if body.is_none() {
            // A fixed collider can live on its own entity at the offset.
            let local = Trs {
                t: center,
                r: [0.0, 0.0, 0.0, 1.0],
                s: [1.0; 3],
            };
            let (w, _) = world.then(&local);
            let mut t = w.from_unity();
            t.s = world.s;
            let mut comps = Map::new();
            comps.insert("Collider".into(), collider);
            out.extra.push((c.type_name.clone(), t, comps));
        } else {
            out.unmapped.push(format!(
                "{} offset from its Rigidbody or a second collider on it (one collider per body, no offsets)",
                c.type_name
            ));
        }
    }
    if let Some(b) = body {
        if let Some(col) = out.components.get_mut("Collider") {
            let bb = &b.body;
            let kinematic = bb.get("m_IsKinematic").and_then(num).unwrap_or(0.0) != 0.0;
            let gravity = bb.get("m_UseGravity").and_then(num).unwrap_or(1.0) != 0.0;
            let constraints = bb.get("m_Constraints").and_then(num).unwrap_or(0.0) as i64;
            let lock_rot = constraints & 112 == 112;
            if constraints & 112 != 0 && !lock_rot {
                out.notes
                    .push("only some rotation axes frozen; Marionette locks all or none".into());
            }
            if constraints & 14 != 0 {
                out.unmapped.push("Rigidbody position constraints".into());
            }
            let mass = bb.get("m_Mass").and_then(num).unwrap_or(1.0);
            let volume = shape_volume(&col["shape"], world.s);
            if volume > 1e-9
                && !col["shape"].get("TriMesh").is_some()
                && !col["shape"].get("ConvexHull").is_some()
            {
                col["density"] = json!(mass / volume);
            } else {
                out.notes
                    .push(format!("Unity mass {mass} kg not converted to a density"));
            }
            out.components.insert(
                "RigidBody".into(),
                json!({
                    "kind": if kinematic { "Kinematic" } else { "Dynamic" },
                    "gravity_scale": if gravity { 1.0 } else { 0.0 },
                    "linear_damping": bb.get("m_Drag").and_then(num).unwrap_or(0.0),
                    "angular_damping": bb.get("m_AngularDrag").and_then(num).unwrap_or(0.05),
                    "lock_rotations": lock_rot,
                }),
            );
        } else {
            out.unmapped
                .push("Rigidbody without a usable collider on the same object".into());
        }
    }

    // Everything else.
    for c in &comps {
        let b = &c.body;
        match c.class_id {
            23 | 33 | 137 | 54 | 64 | 65 | 135 | 136 | 4 | 224 => {}
            20 => {
                let ortho = b.get("orthographic").and_then(num).unwrap_or(0.0) != 0.0;
                let near = b.get("near clip plane").and_then(num).unwrap_or(0.3);
                let far = b.get("far clip plane").and_then(num).unwrap_or(1000.0);
                let projection = if ortho {
                    json!({"Orthographic": {"height": 2.0 * b.get("orthographic size").and_then(num).unwrap_or(5.0), "near": near, "far": far}})
                } else {
                    json!({"Perspective": {"fov_y_degrees": b.get("field of view").and_then(num).unwrap_or(60.0), "near": near, "far": far}})
                };
                out.components.insert(
                    "Camera".into(),
                    json!({"projection": projection, "clear_color": srgb_color(&b["m_BackGroundColor"])}),
                );
            }
            108 => {
                let kind = b.get("m_Type").and_then(num).unwrap_or(2.0) as i64;
                if kind == 1 {
                    let d = qrot(world.r, [0.0, 0.0, 1.0]);
                    out.components.insert(
                        "DirectionalLight".into(),
                        json!({
                            "color": srgb_color(&b["m_Color"]),
                            "direction": {"x": d[0], "y": d[1], "z": -d[2]},
                            "illuminance": b.get("m_Intensity").and_then(num).unwrap_or(1.0),
                        }),
                    );
                } else {
                    let name = match kind {
                        0 => "Spot",
                        2 => "Point",
                        _ => "Area",
                    };
                    out.unmapped.push(format!(
                        "{name} light (Marionette has directional and ambient light)"
                    ));
                }
            }
            81 => {
                out.components
                    .insert("AudioListener".into(), json!({"volume": 1.0}));
            }
            82 => {
                let clip = object_ref(&b["m_audioClip"])
                    .and_then(|(_, g)| g)
                    .and_then(|g| env.asset_path(&g))
                    .unwrap_or_else(|| "no clip".into());
                out.unmapped
                    .push(format!("AudioSource `{clip}` (audio import is not built yet)"));
            }
            114 => {
                let script = object_ref(&b["m_Script"])
                    .and_then(|(_, g)| g)
                    .map(|g| env.asset_path(&g).unwrap_or(g));
                let mut fields = Map::new();
                if let Some(o) = b.as_object() {
                    for (k, v) in o {
                        if !STANDARD_KEYS.contains(&k.as_str()) {
                            fields.insert(k.clone(), v.clone());
                        }
                    }
                }
                out.scripts.push(json!({
                    "script": script.as_deref().map(stem),
                    "path": script,
                    "enabled": enabled(c),
                    "fields": fields,
                }));
            }
            _ => out.unmapped.push(c.type_name.clone()),
        }
    }
    let _ = mesh_scale;
    out
}

fn name_of(n: &Node) -> String {
    let s = str_of(&n.go["m_Name"]);
    if s.is_empty() { "GameObject".into() } else { s }
}

fn flatten(tree: &Tree, env: &dyn PrefabEnv) -> Out {
    let mut out = Out {
        entities: Vec::new(),
        notes: Vec::new(),
        uses: tree.uses.clone(),
    };
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); tree.nodes.len()];
    for (i, n) in tree.nodes.iter().enumerate() {
        if let Some(p) = n.parent
            && p != i
        {
            children[p].push(i);
        }
    }
    for c in &mut children {
        c.sort_by_key(|&i| (tree.nodes[i].order.unwrap_or(usize::MAX), i));
    }
    let root = tree.root.expect("checked in build");
    let mut sheared = false;
    // (node, parent key, parent world, path, active, under a body)
    let mut stack: Vec<(usize, Option<u64>, Trs, String, bool, bool)> =
        vec![(root, None, Trs::IDENTITY, String::new(), true, false)];
    let mut visited = vec![false; tree.nodes.len()];
    while let Some((i, parent_key, parent_world, parent_path, parent_active, under_body)) = stack.pop() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        let n = &tree.nodes[i];
        if n.removed {
            continue;
        }
        // The root's own placement is the instantiation point: only its scale is kept.
        let mut local = local_trs(&n.tr);
        if parent_key.is_none() {
            local.t = [0.0; 3];
            local.r = [0.0, 0.0, 0.0, 1.0];
        }
        let (world, shear) = parent_world.then(&local);
        sheared |= shear;
        let name = name_of(n);
        let path = if parent_path.is_empty() {
            name
        } else {
            format!("{parent_path}/{name}")
        };
        let active = parent_active && n.go.get("m_IsActive").and_then(num).is_none_or(|v| v != 0.0);
        let mapped = map_node(n, &world, under_body, env, &mut out.uses);
        let key = out.entities.len() as u64 + 1;
        let mut components = mapped.components;
        let mut t = world.from_unity();
        if components
            .get("Model")
            .is_some_and(|m| m["asset"].as_str().is_some_and(|a| a.starts_with("library:")))
        {
            t.r = qnorm(qmul(t.r, HALF_TURN_Y));
        }
        if components
            .get("Model")
            .is_some_and(|m| m["asset"] == "primitive:plane")
        {
            t.s = [t.s[0] * 10.0, t.s[1], t.s[2] * 10.0];
            if components.contains_key("Collider") {
                out.notes.push(format!(
                    "`{path}`: plane scaled 10x for Unity's 10 m plane; its collider is scaled too"
                ));
            }
        }
        components.insert("Transform".into(), t.to_json());
        let has_body = components.contains_key("RigidBody");
        let mut unity = Map::new();
        unity.insert("origin".into(), json!(n.origin));
        if !mapped.scripts.is_empty() {
            unity.insert("scripts".into(), Value::Array(mapped.scripts));
        }
        if !mapped.unmapped.is_empty() {
            unity.insert("unmapped".into(), json!(mapped.unmapped));
        }
        if !mapped.notes.is_empty() {
            unity.insert("notes".into(), json!(mapped.notes));
        }
        out.entities.push(json!({
            "key": key,
            "name": path,
            "parent": parent_key,
            "active": active,
            "components": components,
            "unity": unity,
        }));
        for (suffix, t, comps) in mapped.extra {
            let mut comps = comps;
            comps.insert("Transform".into(), t.to_json());
            let k = out.entities.len() as u64 + 1;
            out.entities.push(json!({
                "key": k,
                "name": format!("{path}/{suffix}"),
                "parent": key,
                "active": active,
                "components": comps,
                "unity": {"origin": n.origin, "notes": ["split off: Marionette allows one of these per entity"]},
            }));
        }
        for &c in children[i].iter().rev() {
            stack.push((c, Some(key), world, path.clone(), active, under_body || has_body));
        }
    }
    let orphans = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| !visited[*i] && !n.removed)
        .count();
    if orphans > 0 {
        out.notes.push(format!(
            "{orphans} objects are not under the prefab root and were left out"
        ));
    }
    if sheared {
        out.notes.push(
            "a non-uniformly scaled parent has rotated children: Unity shears them, Marionette cannot (scale applied per axis)"
                .into(),
        );
    }
    out
}

/// Converts the text of a Unity prefab at `rel` (path inside its source) and its nested
/// prefabs into a Marionette prefab document.
pub fn convert(rel: &str, text: &str, env: &dyn PrefabEnv) -> Result<Converted, String> {
    let b = Builder { env };
    let mut stack = Vec::new();
    let tree = b.build(rel, text, &mut stack)?;
    let flat = flatten(&tree, env);
    let mut notes = tree.notes.clone();
    notes.extend(flat.notes);
    notes.push(
        "Marionette has no parent-child transforms: entities are placed relative to the prefab root and do not follow their parent when it moves"
            .into(),
    );
    let root_name = flat
        .entities
        .first()
        .and_then(|e| e["name"].as_str())
        .unwrap_or("Prefab")
        .to_string();
    let entities = flat.entities.len();
    let uses: Vec<String> = flat.uses.into_iter().collect();
    let doc = json!({
        "format": PREFAB_FORMAT,
        "name": root_name,
        "source": rel,
        "entities": flat.entities,
        "uses": uses,
        "notes": notes,
    });
    Ok(Converted {
        doc,
        uses,
        notes,
        entities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composing_and_converting_transforms() {
        let parent = Trs {
            t: [1.0, 0.0, 0.0],
            r: [0.0, (0.5f64).sqrt(), 0.0, (0.5f64).sqrt()], // 90 degrees about +Y (Unity)
            s: [2.0; 3],
        };
        let child = Trs {
            t: [0.0, 0.0, 1.0],
            ..Trs::IDENTITY
        };
        let (w, sheared) = parent.then(&child);
        assert!(!sheared);
        // Unity: +Z turned 90 degrees about +Y is +X; scaled by 2.
        assert!((w.t[0] - 3.0).abs() < 1e-9 && w.t[2].abs() < 1e-9);
        let m = w.from_unity();
        assert!((m.t[0] - 3.0).abs() < 1e-9);
        // Unity forward (+Z) maps to Marionette forward (-Z).
        let f = Trs {
            t: [0.0, 0.0, 5.0],
            ..Trs::IDENTITY
        }
        .from_unity();
        assert_eq!(f.t, [0.0, 0.0, -5.0]);
    }

    #[test]
    fn set_path_handles_arrays() {
        let mut v = json!({"m_Materials": [{"fileID": 1}]});
        set_path(&mut v, "m_Materials.Array.size", json!("2"));
        set_path(&mut v, "m_Materials.Array.data[1]", json!({"fileID": 2}));
        set_path(&mut v, "m_Size.x", json!("3"));
        assert_eq!(v["m_Materials"][1]["fileID"], 2);
        assert_eq!(v["m_Size"]["x"], "3");
    }

    #[test]
    fn nested_ids_follow_unitys_rule() {
        assert_eq!(nested_id(5, 3), 6);
        assert!(nested_id(-1, 0) >= 0);
    }
}
