//! FBX to glTF conversion (decision 0027).
//!
//! A pure-Rust reader for FBX 2011 and newer (file version 7000+, binary or ASCII) and a writer
//! for binary glTF (`.glb`). What is converted:
//!
//! - the node hierarchy, with FBX pivots, offsets, pre/post rotation and rotation order folded
//!   into glTF translation/rotation/scale;
//! - meshes (polygons fan-triangulated, normals, first UV set, one primitive per material),
//!   with the geometric transform baked into the vertices;
//! - materials (diffuse color, opacity, emissive, shininess as roughness) and their diffuse,
//!   normal and emissive textures, which the caller resolves (embedded media first);
//! - blend shapes as morph targets, with their default and animated weights;
//! - skins (clusters to joints, up to four weights per vertex, inverse bind matrices);
//! - animation stacks (first layer; translation, rotation and scale sampled at the union of the
//!   key times with linear or constant interpolation).
//!
//! The file's axis system and unit are converted to glTF (Y up, +Z front, right-handed, meters)
//! by a change of basis applied to every node, vertex and bind matrix, so no extra root node is
//! added. Everything not converted is listed in [`Converted::notes`].

pub mod tree;

use glam::{DMat3, DMat4, DQuat, DVec3};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tree::{Document, Node, Prop, TreeLimits};

/// FBX time units per second (FBX 7.x).
const KTIME_PER_SECOND: f64 = 46_186_158_000.0;

/// A texture the FBX file references, to be resolved by the caller.
#[derive(Debug, Clone)]
pub struct TextureRequest {
    /// `RelativeFilename` as written in the file (may be empty).
    pub relative: String,
    /// `FileName` as written in the file (usually an absolute path on the author's machine).
    pub absolute: String,
    /// Bytes embedded in the FBX file (`Video` content), if any.
    pub embedded: Option<Vec<u8>>,
}

impl TextureRequest {
    /// The bare file name the request is about.
    pub fn file_name(&self) -> String {
        let pick = if self.relative.is_empty() {
            &self.absolute
        } else {
            &self.relative
        };
        pick.rsplit(['/', '\\']).next().unwrap_or(pick).to_string()
    }
}

/// An image ready for the glTF file.
#[derive(Debug, Clone)]
pub struct ResolvedImage {
    pub bytes: Vec<u8>,
    /// `image/png` or `image/jpeg`.
    pub mime: &'static str,
    /// Where it came from, for the notes.
    pub origin: String,
}

/// What the conversion produced.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub fbx_version: u32,
    pub binary: bool,
    pub nodes: usize,
    pub meshes: usize,
    pub triangles: usize,
    pub materials: usize,
    pub images: usize,
    pub skins: usize,
    pub joints: usize,
    pub animations: usize,
    /// Blend shapes converted to glTF morph targets (all meshes).
    pub morph_targets: usize,
    /// Source axis system as `right/up/front`, e.g. `+X/+Y/+Z`.
    pub axes: String,
    /// Meters per source unit.
    pub meters_per_unit: f64,
}

#[derive(Debug, Clone)]
pub struct Converted {
    pub glb: Vec<u8>,
    pub summary: Summary,
    /// What was approximated or left out, one line each.
    pub notes: Vec<String>,
}

/// Converts FBX bytes to a GLB file. `resolve` turns texture references into images; an `Err`
/// leaves the texture out and records the reason in the notes.
pub fn convert(
    bytes: &[u8],
    limits: TreeLimits,
    resolve: &mut dyn FnMut(&TextureRequest) -> Result<ResolvedImage, String>,
) -> Result<Converted, String> {
    let doc = tree::parse(bytes, limits)?;
    if doc.version < 7000 {
        return Err(format!(
            "FBX file version {} is older than FBX 2011 (7000); re-export it as FBX 2011 or newer",
            doc.version
        ));
    }
    Scene::read(&doc)?.write(&doc, resolve)
}

// ---------------------------------------------------------------------------------------------
// Properties

type Props70 = BTreeMap<String, Vec<Prop>>;

fn props70(node: &Node) -> Props70 {
    let mut out = Props70::new();
    for list in node
        .children_named("Properties70")
        .chain(node.children_named("Properties60"))
    {
        for p in list
            .children
            .iter()
            .filter(|p| p.name == "P" || p.name == "Property")
        {
            if let Some(name) = p.prop(0).and_then(Prop::as_str) {
                let skip = if p.name == "P" { 4 } else { 3 };
                out.insert(name.to_string(), p.props.iter().skip(skip).cloned().collect());
            }
        }
    }
    out
}

/// Object properties with the file's template as fallback.
struct PropView<'a> {
    own: Props70,
    template: Option<&'a Props70>,
}

impl PropView<'_> {
    fn get(&self, name: &str) -> Option<&Vec<Prop>> {
        self.own
            .get(name)
            .or_else(|| self.template.and_then(|t| t.get(name)))
    }
    fn vec3(&self, name: &str, default: DVec3) -> DVec3 {
        match self.get(name) {
            Some(v) if v.len() >= 3 => DVec3::new(
                v[0].as_f64().unwrap_or(default.x),
                v[1].as_f64().unwrap_or(default.y),
                v[2].as_f64().unwrap_or(default.z),
            ),
            _ => default,
        }
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.get(name)?.first()?.as_f64()
    }
    fn int(&self, name: &str, default: i64) -> i64 {
        self.get(name)
            .and_then(|v| v.first())
            .and_then(Prop::as_i64)
            .unwrap_or(default)
    }
}

// ---------------------------------------------------------------------------------------------
// Scene graph

struct Object<'a> {
    node: &'a Node,
    /// `Model`, `Geometry`, `Material`, ...
    class: &'a str,
    /// `Mesh`, `LimbNode`, `Skin`, `Cluster`, ...
    sub: &'a str,
    name: String,
}

#[derive(Clone)]
struct Link {
    child: i64,
    parent: i64,
    prop: Option<String>,
}

struct Scene<'a> {
    objects: BTreeMap<i64, Object<'a>>,
    /// Object ids in file order.
    order: Vec<i64>,
    links: Vec<Link>,
    templates: BTreeMap<String, Props70>,
    /// Change of basis (signed permutation) and scale to glTF.
    basis: DMat3,
    scale: f64,
    axes: String,
    notes: std::cell::RefCell<Notes>,
}

#[derive(Default)]
struct Notes(Vec<String>);

impl Notes {
    fn add(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.0.contains(&s) {
            self.0.push(s);
        }
    }
}

fn strip_class(name: &str) -> String {
    match name.split_once("::") {
        Some((_, n)) => n.to_string(),
        None => name.to_string(),
    }
}

impl<'a> Scene<'a> {
    fn read(doc: &'a Document) -> Result<Self, String> {
        let objs = doc.root("Objects").ok_or("FBX file has no Objects section")?;
        let mut objects = BTreeMap::new();
        let mut order = Vec::new();
        for n in &objs.children {
            let Some(id) = n.prop(0).and_then(Prop::as_i64) else {
                continue;
            };
            let name = n.prop(1).and_then(Prop::as_str).unwrap_or("");
            let sub = n.prop(2).and_then(Prop::as_str).unwrap_or("");
            order.push(id);
            objects.insert(
                id,
                Object {
                    node: n,
                    class: &n.name,
                    sub,
                    name: strip_class(name),
                },
            );
        }
        let mut links = Vec::new();
        if let Some(c) = doc.root("Connections") {
            for l in c.children.iter().filter(|l| l.name == "C" || l.name == "Connect") {
                let (Some(child), Some(parent)) =
                    (l.prop(1).and_then(Prop::as_i64), l.prop(2).and_then(Prop::as_i64))
                else {
                    continue;
                };
                links.push(Link {
                    child,
                    parent,
                    prop: l.prop(3).and_then(Prop::as_str).map(str::to_string),
                });
            }
        }
        let mut templates = BTreeMap::new();
        if let Some(defs) = doc.root("Definitions") {
            for t in defs.children_named("ObjectType") {
                let Some(kind) = t.prop(0).and_then(Prop::as_str) else {
                    continue;
                };
                if let Some(pt) = t.child("PropertyTemplate") {
                    templates.insert(kind.to_string(), props70(pt));
                }
            }
        }

        // Axis system and unit from GlobalSettings.
        let gs = doc.root("GlobalSettings").map(props70).unwrap_or_default();
        let gv = PropView {
            own: gs,
            template: None,
        };
        let axis = |which: &str, sign: &str, default: (i64, i64)| -> (usize, f64) {
            let a = gv.int(which, default.0).clamp(0, 2) as usize;
            let s = if gv.int(sign, default.1) < 0 { -1.0 } else { 1.0 };
            (a, s)
        };
        let (ra, rs) = axis("CoordAxis", "CoordAxisSign", (0, 1));
        let (ua, us) = axis("UpAxis", "UpAxisSign", (1, 1));
        let (fa, fs) = axis("FrontAxis", "FrontAxisSign", (2, 1));
        let unit = |i: usize, s: f64| {
            let mut v = DVec3::ZERO;
            v[i] = s;
            v
        };
        let mut notes = Notes::default();
        let (basis, axes) = if ra != ua && ua != fa && ra != fa {
            let (r, u, f) = (unit(ra, rs), unit(ua, us), unit(fa, fs));
            // Columns r, u, f map glTF axes to file axes; the inverse (transpose) maps back.
            let name =
                |i: usize, s: f64| format!("{}{}", if s < 0.0 { "-" } else { "+" }, ["X", "Y", "Z"][i]);
            (
                DMat3::from_cols(r, u, f).transpose(),
                format!("{}/{}/{}", name(ra, rs), name(ua, us), name(fa, fs)),
            )
        } else {
            notes.add("axis system in GlobalSettings is invalid; assumed Y up, +Z front");
            (DMat3::IDENTITY, "+X/+Y/+Z".to_string())
        };
        let cm_per_unit = gv.num("UnitScaleFactor").filter(|v| *v > 0.0).unwrap_or(1.0);
        Ok(Self {
            objects,
            order,
            links,
            templates,
            basis,
            scale: cm_per_unit / 100.0,
            axes,
            notes: std::cell::RefCell::new(notes),
        })
    }

    fn view(&self, id: i64) -> PropView<'_> {
        let o = &self.objects[&id];
        PropView {
            own: props70(o.node),
            template: self.templates.get(o.class),
        }
    }

    /// Objects connected below `parent`, in connection order, with the link property.
    fn children_of(&self, parent: i64, class: &str) -> Vec<(i64, Option<&str>)> {
        self.links
            .iter()
            .filter(|l| l.parent == parent)
            .filter(|l| self.objects.get(&l.child).is_some_and(|o| o.class == class))
            .map(|l| (l.child, l.prop.as_deref()))
            .collect()
    }

    fn parents_of(&self, child: i64, class: &str) -> Vec<(i64, Option<&str>)> {
        self.links
            .iter()
            .filter(|l| l.child == child)
            .filter(|l| self.objects.get(&l.parent).is_some_and(|o| o.class == class))
            .map(|l| (l.parent, l.prop.as_deref()))
            .collect()
    }

    /// Conjugates an FBX-space transform into glTF space.
    fn to_gltf_trs(&self, t: DVec3, r: DQuat, s: DVec3) -> (DVec3, DQuat, DVec3) {
        let b = self.basis;
        let t2 = b * t * self.scale;
        let r2 = DQuat::from_mat3(&(b * DMat3::from_quat(r) * b.transpose())).normalize();
        let sm = b * DMat3::from_diagonal(s) * b.transpose();
        (t2, r2, DVec3::new(sm.x_axis.x, sm.y_axis.y, sm.z_axis.z))
    }

    fn to_gltf_mat(&self, m: DMat4) -> DMat4 {
        let c = DMat4::from_mat3(self.basis) * DMat4::from_scale(DVec3::splat(self.scale));
        c * m * c.inverse()
    }

    fn write(
        self,
        doc: &Document,
        resolve: &mut dyn FnMut(&TextureRequest) -> Result<ResolvedImage, String>,
    ) -> Result<Converted, String> {
        let mut out = Gltf::default();
        let models: Vec<i64> = self
            .order
            .iter()
            .copied()
            .filter(|id| self.objects[id].class == "Model")
            .collect();
        let node_of: HashMap<i64, usize> = models.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // Nodes and hierarchy.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); models.len()];
        let mut roots = Vec::new();
        for (i, &id) in models.iter().enumerate() {
            let parent = self
                .parents_of(id, "Model")
                .first()
                .and_then(|(p, _)| node_of.get(p).copied());
            match parent {
                Some(p) if p != i => children[p].push(i),
                _ => roots.push(i),
            }
        }
        let mut nodes: Vec<Value> = Vec::new();
        for &id in &models {
            let v = self.view(id);
            let (t, r, s) = local_trs(&v, None);
            let (t, r, s) = self.to_gltf_trs(t, r, s);
            if s.min_element() < 0.0 {
                self.notes
                    .borrow_mut()
                    .add("a node has negative scale (mirroring); renderers may show it inside out");
            }
            let o = &self.objects[&id];
            let mut n = json!({
                "name": o.name,
                "translation": f32s(t.to_array()),
                "rotation": f32s(r.to_array()),
                "scale": f32s(s.to_array()),
            });
            if !matches!(o.sub, "Mesh" | "LimbNode" | "Null" | "Root" | "") {
                n["extras"] = json!({ "fbx_type": o.sub });
            }
            nodes.push(n);
        }
        for (i, c) in children.iter().enumerate() {
            if !c.is_empty() {
                nodes[i]["children"] = json!(c);
            }
        }

        // Materials and images.
        let mut materials: Vec<Value> = Vec::new();
        let mut material_of: HashMap<i64, usize> = HashMap::new();
        let mut images: Vec<(String, usize)> = Vec::new(); // (dedupe key, glTF texture index)
        for &id in &self.order {
            if self.objects[&id].class != "Material" {
                continue;
            }
            let m = self.material(id, &mut out, &mut images, resolve);
            material_of.insert(id, materials.len());
            materials.push(m);
        }

        // Meshes and skins.
        let mut meshes: Vec<Value> = Vec::new();
        let mut skins: Vec<Value> = Vec::new();
        let mut triangles = 0;
        let mut joint_nodes: BTreeSet<usize> = BTreeSet::new();
        // Blend shape channel -> (node, target index); node -> default weights.
        let mut morph_of: HashMap<i64, (usize, usize)> = HashMap::new();
        let mut morph_defaults: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
        let mut morph_targets = 0;
        for (i, &id) in models.iter().enumerate() {
            let geoms: Vec<i64> = self
                .children_of(id, "Geometry")
                .into_iter()
                .map(|(g, _)| g)
                .collect();
            let Some(&geom) = geoms.first() else { continue };
            if geoms.len() > 1 {
                self.notes
                    .borrow_mut()
                    .add("a node has more than one geometry; only the first is converted");
            }
            let gobj = &self.objects[&geom];
            if gobj.sub != "Mesh" {
                self.notes.borrow_mut().add(format!(
                    "geometry type `{}` is not converted (meshes only)",
                    gobj.sub
                ));
                continue;
            }
            let mats: Vec<Option<usize>> = self
                .children_of(id, "Material")
                .into_iter()
                .map(|(m, _)| material_of.get(&m).copied())
                .collect();
            let skin = self.skin(geom, &node_of);
            let v = self.view(id);
            let g = geometric(&v);
            let Some(mesh) = self.mesh(gobj.node, g, skin.as_ref())? else {
                continue;
            };
            triangles += mesh.prims.iter().map(|p| p.1.len() / 3).sum::<usize>();
            let mut attrs = json!({
                "POSITION": out.vec3s(&mesh.positions, true),
            });
            if !mesh.normals.is_empty() {
                attrs["NORMAL"] = json!(out.vec3s(&mesh.normals, false));
            }
            if !mesh.uvs.is_empty() {
                attrs["TEXCOORD_0"] = json!(out.floats(&mesh.uvs.concat(), "VEC2"));
            }
            if let Some(sk) = &skin {
                attrs["JOINTS_0"] = json!(out.joints(&mesh.joints));
                attrs["WEIGHTS_0"] = json!(out.floats(&mesh.weights.concat(), "VEC4"));
                let ibm: Vec<f32> = sk
                    .ibms
                    .iter()
                    .flat_map(|m| self.to_gltf_mat(*m).to_cols_array().map(|x| x as f32))
                    .collect();
                let ibm = out.floats(&ibm, "MAT4");
                joint_nodes.extend(sk.joints.iter().copied());
                nodes[i]["skin"] = json!(skins.len());
                skins.push(json!({
                    "name": format!("{} skin", self.objects[&id].name),
                    "joints": sk.joints,
                    "inverseBindMatrices": ibm,
                }));
                if sk.unweighted > 0 {
                    self.notes.borrow_mut().add(format!(
                        "{} vertices had no skin weights and were bound to the first joint",
                        sk.unweighted
                    ));
                }
                if sk.over_four > 0 {
                    self.notes.borrow_mut().add(format!(
                        "{} vertices had more than four bone influences; the four largest were kept",
                        sk.over_four
                    ));
                }
            }
            let morphs = self.morphs(geom, &mesh);
            let mut targets = Vec::new();
            for (k, m) in morphs.iter().enumerate() {
                targets.push(json!({ "POSITION": out.vec3s(&m.deltas, true) }));
                morph_of.insert(m.channel, (i, k));
            }
            let mut prims = Vec::new();
            for (slot, idx) in &mesh.prims {
                let mut p = json!({ "attributes": attrs, "indices": out.indices(idx) });
                if !targets.is_empty() {
                    p["targets"] = json!(targets);
                }
                match slot.and_then(|s| mats.get(s).copied().flatten()) {
                    Some(m) => p["material"] = json!(m),
                    None if !mats.is_empty() => {
                        self.notes
                            .borrow_mut()
                            .add("some polygons use a material slot the node does not have");
                    }
                    None => {}
                }
                prims.push(p);
            }
            nodes[i]["mesh"] = json!(meshes.len());
            let mut m = json!({ "name": gobj.name, "primitives": prims });
            if !morphs.is_empty() {
                let defaults: Vec<f32> = morphs.iter().map(|m| m.default_weight).collect();
                m["weights"] = json!(defaults);
                m["extras"] =
                    json!({ "targetNames": morphs.iter().map(|m| m.name.clone()).collect::<Vec<_>>() });
                morph_targets += morphs.len();
                morph_defaults.insert(i, defaults);
            }
            meshes.push(m);
        }

        // Animations.
        let mut animations = Vec::new();
        for &id in &self.order {
            if self.objects[&id].class != "AnimationStack" {
                continue;
            }
            if let Some(a) = self.animation(id, &node_of, &models, &morph_of, &morph_defaults, &mut out) {
                animations.push(a);
            }
        }

        let summary = Summary {
            fbx_version: doc.version,
            binary: doc.binary,
            nodes: nodes.len(),
            meshes: meshes.len(),
            triangles,
            materials: materials.len(),
            images: out.images.len(),
            skins: skins.len(),
            joints: joint_nodes.len(),
            animations: animations.len(),
            morph_targets,
            axes: self.axes.clone(),
            meters_per_unit: self.scale,
        };
        let mut root = json!({
            "asset": { "version": "2.0", "generator": "Marionette FBX converter" },
            "scene": 0,
            "scenes": [{ "nodes": roots }],
        });
        let put = |root: &mut Value, key: &str, v: Vec<Value>| {
            if !v.is_empty() {
                root[key] = Value::Array(v);
            }
        };
        put(&mut root, "nodes", nodes);
        put(&mut root, "meshes", meshes);
        put(&mut root, "materials", materials);
        put(&mut root, "skins", skins);
        put(&mut root, "animations", animations);
        if !out.images.is_empty() {
            root["images"] = Value::Array(out.images.clone());
            root["textures"] = Value::Array(out.textures.clone());
            root["samplers"] = json!([{ "wrapS": 10497, "wrapT": 10497 }]);
        }
        if self
            .objects
            .values()
            .any(|o| o.class == "NodeAttribute" && matches!(o.sub, "Camera" | "Light"))
        {
            self.notes
                .borrow_mut()
                .add("cameras and lights are kept as empty nodes");
        }
        let glb = out.finish(root);
        Ok(Converted {
            glb,
            summary,
            notes: self.notes.into_inner().0,
        })
    }

    fn material(
        &self,
        id: i64,
        out: &mut Gltf,
        images: &mut Vec<(String, usize)>,
        resolve: &mut dyn FnMut(&TextureRequest) -> Result<ResolvedImage, String>,
    ) -> Value {
        let v = self.view(id);
        let name = self.objects[&id].name.clone();
        let diffuse = v.vec3("DiffuseColor", v.vec3("Diffuse", DVec3::splat(0.8)));
        let alpha = v
            .num("Opacity")
            .or_else(|| v.num("TransparencyFactor").map(|t| 1.0 - t))
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let emissive = v.vec3("EmissiveColor", DVec3::ZERO) * v.num("EmissiveFactor").unwrap_or(1.0);
        let shininess = v.num("ShininessExponent").or_else(|| v.num("Shininess"));
        let roughness = shininess.map_or(0.8, |n| (2.0 / (n.max(0.0) + 2.0)).sqrt().clamp(0.0, 1.0));

        let mut textures: BTreeMap<&str, usize> = BTreeMap::new();
        for (tex, prop) in self.children_of(id, "Texture") {
            let slot = match prop.unwrap_or("") {
                "DiffuseColor"
                | "Maya|baseColor"
                | "Maya|TEX_color_map"
                | "3dsMax|Parameters|base_color_map" => "base",
                "NormalMap" | "Maya|normalCamera" | "Maya|TEX_normal_map" | "3dsMax|Parameters|bump_map" => {
                    "normal"
                }
                "EmissiveColor" | "Maya|emissionColor" | "Maya|TEX_emissive_map" => "emissive",
                "Bump" => {
                    self.notes
                        .borrow_mut()
                        .add("bump (height) maps are not converted; normal maps are");
                    continue;
                }
                other => {
                    self.notes.borrow_mut().add(format!(
                        "material texture slot `{other}` has no glTF equivalent and was left out"
                    ));
                    continue;
                }
            };
            if textures.contains_key(slot) {
                continue;
            }
            if let Some(t) = self.texture(tex, out, images, resolve) {
                textures.insert(slot, t);
            }
        }
        let base_factor = if textures.contains_key("base") {
            [1.0, 1.0, 1.0, alpha]
        } else {
            [diffuse.x, diffuse.y, diffuse.z, alpha]
        };
        let mut m = json!({
            "name": name,
            "pbrMetallicRoughness": {
                "baseColorFactor": f32s(base_factor.map(|x| x.clamp(0.0, 1.0))),
                "metallicFactor": 0.0,
                "roughnessFactor": roughness as f32,
            },
        });
        if let Some(t) = textures.get("base") {
            m["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": t });
        }
        if let Some(t) = textures.get("normal") {
            m["normalTexture"] = json!({ "index": t });
        }
        let e = emissive.clamp(DVec3::ZERO, DVec3::ONE);
        if let Some(t) = textures.get("emissive") {
            m["emissiveTexture"] = json!({ "index": t });
            m["emissiveFactor"] = json!(if e == DVec3::ZERO {
                [1.0, 1.0, 1.0]
            } else {
                f32s(e.to_array())
            });
        } else if e != DVec3::ZERO {
            m["emissiveFactor"] = json!(f32s(e.to_array()));
        }
        if alpha < 1.0 {
            m["alphaMode"] = json!("BLEND");
        }
        m
    }

    fn texture(
        &self,
        tex: i64,
        out: &mut Gltf,
        images: &mut Vec<(String, usize)>,
        resolve: &mut dyn FnMut(&TextureRequest) -> Result<ResolvedImage, String>,
    ) -> Option<usize> {
        let tnode = self.objects[&tex].node;
        let str_of = |n: &Node, k: &str| n.value(k).and_then(Prop::as_str).unwrap_or("").to_string();
        let mut req = TextureRequest {
            relative: str_of(tnode, "RelativeFilename"),
            absolute: str_of(tnode, "FileName"),
            embedded: None,
        };
        for (video, _) in self.children_of(tex, "Video") {
            let vnode = self.objects[&video].node;
            if req.relative.is_empty() {
                req.relative = str_of(vnode, "RelativeFilename");
            }
            if req.absolute.is_empty() {
                req.absolute = str_of(vnode, "Filename");
            }
            match vnode.value("Content") {
                Some(Prop::Raw(b)) if !b.is_empty() => req.embedded = Some(b.clone()),
                Some(Prop::Str(s)) if !s.is_empty() => req.embedded = base64_decode(s),
                _ => {}
            }
        }
        let key = if req.embedded.is_some() {
            format!("embedded:{tex}")
        } else {
            req.relative.replace('\\', "/").to_ascii_lowercase()
                + "|"
                + &req.absolute.replace('\\', "/").to_ascii_lowercase()
        };
        if let Some((_, t)) = images.iter().find(|(k, _)| *k == key) {
            return Some(*t);
        }
        let v = self.view(tex);
        let uv_t = v.vec3("Translation", DVec3::ZERO);
        let uv_s = v.vec3("Scaling", DVec3::ONE);
        if uv_t != DVec3::ZERO || uv_s != DVec3::ONE {
            self.notes
                .borrow_mut()
                .add("texture offset/tiling is not converted");
        }
        match resolve(&req) {
            Ok(img) => {
                let t = out.image(img.bytes, img.mime, &req.file_name());
                images.push((key, t));
                self.notes
                    .borrow_mut()
                    .add(format!("texture `{}`: {}", req.file_name(), img.origin));
                Some(t)
            }
            Err(why) => {
                self.notes
                    .borrow_mut()
                    .add(format!("texture `{}` left out: {why}", req.file_name()));
                None
            }
        }
    }

    fn skin(&self, geom: i64, node_of: &HashMap<i64, usize>) -> Option<SkinData> {
        let skin = self
            .children_of(geom, "Deformer")
            .into_iter()
            .map(|(d, _)| d)
            .find(|d| self.objects[d].sub == "Skin")?;
        let cps = self.objects[&geom]
            .node
            .array("Vertices")
            .and_then(Prop::f64s)
            .map_or(0, |v| v.len() / 3);
        let mut joints = Vec::new();
        let mut ibms = Vec::new();
        let mut influences: Vec<Vec<(u16, f32)>> = vec![Vec::new(); cps];
        for (cluster, _) in self.children_of(skin, "Deformer") {
            let c = &self.objects[&cluster];
            if c.sub != "Cluster" {
                continue;
            }
            let Some(bone) = self
                .children_of(cluster, "Model")
                .first()
                .and_then(|(b, _)| node_of.get(b).copied())
            else {
                continue;
            };
            let mat = |k: &str| {
                c.node
                    .array(k)
                    .and_then(Prop::f64s)
                    .filter(|v| v.len() == 16)
                    .map(|v| DMat4::from_cols_array(&v.try_into().unwrap()))
            };
            // `Transform` maps the mesh node's space into the bone's space at bind time: it is
            // the inverse bind matrix already (`TransformLink` is the bone's bind pose in world).
            let Some(ibm) = mat("Transform").or_else(|| mat("TransformLink").map(|l| l.inverse())) else {
                self.notes
                    .borrow_mut()
                    .add("a skin cluster has no bind matrix and was left out");
                continue;
            };
            if c.node
                .value("Mode")
                .and_then(Prop::as_str)
                .is_some_and(|m| m != "Normalize" && m != "Total1")
            {
                self.notes
                    .borrow_mut()
                    .add("cluster link modes other than normalize are treated as normalize");
            }
            let j = joints.len() as u16;
            joints.push(bone);
            ibms.push(ibm);
            let idx = c.node.array("Indexes").and_then(Prop::i64s).unwrap_or_default();
            let w = c.node.array("Weights").and_then(Prop::f64s).unwrap_or_default();
            for (&i, &w) in idx.iter().zip(&w) {
                if let Some(slot) = usize::try_from(i).ok().and_then(|i| influences.get_mut(i))
                    && w > 0.0
                {
                    slot.push((j, w as f32));
                }
            }
        }
        if joints.is_empty() {
            return None;
        }
        let mut unweighted = 0;
        let mut over_four = 0;
        let mut per_cp = Vec::with_capacity(cps);
        for mut inf in influences {
            if inf.len() > 4 {
                over_four += 1;
            }
            inf.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            inf.truncate(4);
            let sum: f32 = inf.iter().map(|x| x.1).sum();
            let mut js = [0u16; 4];
            let mut ws = [0f32; 4];
            if sum <= 0.0 {
                unweighted += 1;
                ws[0] = 1.0;
            } else {
                for (k, (j, w)) in inf.iter().enumerate() {
                    js[k] = *j;
                    ws[k] = w / sum;
                }
            }
            per_cp.push((js, ws));
        }
        Some(SkinData {
            joints,
            ibms,
            per_cp,
            unweighted,
            over_four,
        })
    }

    fn mesh(&self, g: &Node, geo: DMat4, skin: Option<&SkinData>) -> Result<Option<MeshOut>, String> {
        let verts = g.array("Vertices").and_then(Prop::f64s).unwrap_or_default();
        let pvi = g
            .array("PolygonVertexIndex")
            .and_then(Prop::i64s)
            .unwrap_or_default();
        if verts.is_empty() || pvi.is_empty() {
            return Ok(None);
        }
        let cps = verts.len() / 3;
        let normal_layer = first_layer(g, "LayerElementNormal");
        let uv_layer = first_layer(g, "LayerElementUV");
        let mat_layer = first_layer(g, "LayerElementMaterial");
        if g.children_named("LayerElementUV").count() > 1 {
            self.notes.borrow_mut().add("only the first UV set is converted");
        }
        if g.child("LayerElementColor").is_some() {
            self.notes.borrow_mut().add("vertex colors are not converted");
        }
        let normals = normal_layer.map(|l| Layer::new(l, "Normals", "NormalsIndex", 3));
        let uvs = uv_layer.map(|l| Layer::new(l, "UV", "UVIndex", 2));
        let mats = mat_layer.map(|l| Layer::new(l, "Materials", "", 1));

        let full = DMat4::from_mat3(self.basis) * DMat4::from_scale(DVec3::splat(self.scale)) * geo;
        let nmat = (DMat3::from_mat4(full)).inverse().transpose();
        let flip = DMat3::from_mat4(full).determinant() < 0.0;

        let mut out = MeshOut {
            linear: DMat3::from_mat4(full),
            ..Default::default()
        };
        let mut dedupe: HashMap<(u32, [u32; 3], [u32; 2]), u32> = HashMap::new();
        let mut prims: BTreeMap<Option<usize>, Vec<u32>> = BTreeMap::new();
        let mut poly: Vec<u32> = Vec::new();
        let mut poly_index = 0usize;
        let mut skipped_polys = 0usize;
        for (pv, &raw) in pvi.iter().enumerate() {
            let last = raw < 0;
            let cp = if last { -raw - 1 } else { raw };
            let cp = usize::try_from(cp)
                .ok()
                .filter(|&c| c < cps)
                .ok_or_else(|| format!("polygon vertex index {cp} is outside the {cps} control points"))?;
            let p = DVec3::new(verts[cp * 3], verts[cp * 3 + 1], verts[cp * 3 + 2]);
            let n = normals
                .as_ref()
                .and_then(|l| l.get(pv, cp, poly_index))
                .map(|n| nmat * DVec3::new(n[0], n[1], n[2]))
                .map(|n| n.normalize_or(DVec3::Y));
            let uv = uvs
                .as_ref()
                .and_then(|l| l.get(pv, cp, poly_index))
                .map(|t| [t[0] as f32, 1.0 - t[1] as f32]);
            let nf = n.map(|n| [n.x as f32, n.y as f32, n.z as f32]);
            let key = (
                cp as u32,
                nf.map_or([0; 3], |n| n.map(f32::to_bits)),
                uv.map_or([0; 2], |t| t.map(f32::to_bits)),
            );
            let idx = *dedupe.entry(key).or_insert_with(|| {
                let w = full.transform_point3(p);
                out.positions.push([w.x as f32, w.y as f32, w.z as f32]);
                out.cps.push(cp as u32);
                if let Some(n) = nf {
                    out.normals.push(n);
                }
                if normals.is_some() && nf.is_none() {
                    out.normals.push([0.0, 1.0, 0.0]);
                }
                if uvs.is_some() {
                    out.uvs.push(uv.unwrap_or([0.0, 0.0]));
                }
                if let Some(s) = skin {
                    let (j, w) = s.per_cp[cp];
                    out.joints.push(j);
                    out.weights.push(w);
                }
                (out.positions.len() - 1) as u32
            });
            poly.push(idx);
            if last {
                if poly.len() >= 3 {
                    let slot = mats
                        .as_ref()
                        .and_then(|l| l.get(pv, cp, poly_index))
                        .map(|m| m[0] as usize);
                    let list = prims.entry(slot).or_default();
                    for k in 1..poly.len() - 1 {
                        let (a, b, c) = (poly[0], poly[k], poly[k + 1]);
                        if flip {
                            list.extend_from_slice(&[a, c, b]);
                        } else {
                            list.extend_from_slice(&[a, b, c]);
                        }
                    }
                } else {
                    skipped_polys += 1;
                }
                poly.clear();
                poly_index += 1;
            }
        }
        if skipped_polys > 0 {
            self.notes
                .borrow_mut()
                .add("polygons with fewer than three corners (points and lines) are left out");
        }
        out.prims = prims.into_iter().filter(|(_, v)| !v.is_empty()).collect();
        if out.prims.is_empty() {
            return Ok(None);
        }
        if normals.is_none() {
            self.notes
                .borrow_mut()
                .add("a mesh has no normals; the engine computes them");
        }
        Ok(Some(out))
    }

    /// Blend shape channels of a geometry as glTF morph targets (position offsets per output
    /// vertex). A channel with in-between shapes uses its full-weight (last) shape.
    fn morphs(&self, geom: i64, mesh: &MeshOut) -> Vec<Morph> {
        let mut out = Vec::new();
        for (bs, _) in self.children_of(geom, "Deformer") {
            if self.objects[&bs].sub != "BlendShape" {
                continue;
            }
            for (ch, _) in self.children_of(bs, "Deformer") {
                if self.objects[&ch].sub != "BlendShapeChannel" {
                    continue;
                }
                let shapes = self.children_of(ch, "Geometry");
                if shapes.len() > 1 {
                    self.notes.borrow_mut().add(
                        "blend shape in-between shapes are not converted; the full-weight shape is used",
                    );
                }
                let Some(&(shape, _)) = shapes.last() else {
                    continue;
                };
                let node = self.objects[&shape].node;
                let idx = node.array("Indexes").and_then(Prop::i64s).unwrap_or_default();
                let offs = node.array("Vertices").and_then(Prop::f64s).unwrap_or_default();
                let mut per_cp: HashMap<u32, DVec3> = HashMap::new();
                for (k, &i) in idx.iter().enumerate() {
                    let (Ok(i), Some(d)) = (u32::try_from(i), offs.get(k * 3..k * 3 + 3)) else {
                        continue;
                    };
                    per_cp.insert(i, mesh.linear * DVec3::new(d[0], d[1], d[2]));
                }
                let deltas: Vec<[f32; 3]> = mesh
                    .cps
                    .iter()
                    .map(|cp| per_cp.get(cp).map_or([0.0; 3], |d| f32s(d.to_array())))
                    .collect();
                if node.child("Normals").is_some() {
                    self.notes
                        .borrow_mut()
                        .add("blend shape normal changes are not converted; base normals are kept");
                }
                let name = match self.objects[&ch].name.as_str() {
                    "" => self.objects[&shape].name.clone(),
                    n => n.to_string(),
                };
                out.push(Morph {
                    name,
                    channel: ch,
                    // Written as a plain child node by most exporters, sometimes as a property.
                    default_weight: (self.objects[&ch]
                        .node
                        .value("DeformPercent")
                        .and_then(Prop::as_f64)
                        .or_else(|| self.view(ch).num("DeformPercent"))
                        .unwrap_or(0.0)
                        / 100.0) as f32,
                    deltas,
                });
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn animation(
        &self,
        stack: i64,
        node_of: &HashMap<i64, usize>,
        models: &[i64],
        morph_of: &HashMap<i64, (usize, usize)>,
        morph_defaults: &BTreeMap<usize, Vec<f32>>,
        out: &mut Gltf,
    ) -> Option<Value> {
        let layers = self.children_of(stack, "AnimationLayer");
        if layers.len() > 1 {
            self.notes
                .borrow_mut()
                .add("animation stacks with several layers: only the first layer is converted");
        }
        let layer = layers.first()?.0;
        let start = self
            .view(stack)
            .get("LocalStart")
            .and_then(|v| v.first())
            .and_then(Prop::as_i64);

        // model -> property ("T","R","S") -> per-axis curves (or defaults).
        let mut channels: BTreeMap<usize, BTreeMap<char, [Channel; 3]>> = BTreeMap::new();
        // node -> per-target blend shape weight curve (percent).
        let mut weights: BTreeMap<usize, Vec<Option<Curve>>> = BTreeMap::new();
        for (cn, _) in self.children_of(layer, "AnimationCurveNode") {
            let defaults = self.view(cn);
            for (ch, prop) in self.parents_of(cn, "Deformer") {
                let Some(&(ni, k)) = morph_of.get(&ch).filter(|_| prop == Some("DeformPercent")) else {
                    continue;
                };
                let n = morph_defaults.get(&ni).map_or(0, Vec::len);
                let slot = weights
                    .entry(ni)
                    .or_insert_with(|| (0..n).map(|_| None).collect());
                if let Some(c) = self
                    .children_of(cn, "AnimationCurve")
                    .first()
                    .and_then(|(c, _)| Curve::read(self.objects[c].node))
                    && let Some(s) = slot.get_mut(k)
                {
                    *s = Some(c);
                }
            }
            for (model, prop) in self.parents_of(cn, "Model") {
                let Some(&ni) = node_of.get(&model) else { continue };
                let kind = match prop.unwrap_or("") {
                    "Lcl Translation" => 'T',
                    "Lcl Rotation" => 'R',
                    "Lcl Scaling" => 'S',
                    other => {
                        self.notes
                            .borrow_mut()
                            .add(format!("animated property `{other}` is not converted"));
                        continue;
                    }
                };
                let mut axes: [Channel; 3] = Default::default();
                for (k, name) in ["d|X", "d|Y", "d|Z"].iter().enumerate() {
                    axes[k].default = defaults.num(name);
                }
                for (curve, cprop) in self.children_of(cn, "AnimationCurve") {
                    let k = match cprop.unwrap_or("") {
                        "d|X" | "X" => 0,
                        "d|Y" | "Y" => 1,
                        "d|Z" | "Z" => 2,
                        _ => continue,
                    };
                    axes[k].curve = Curve::read(self.objects[&curve].node);
                    if axes[k].curve.as_ref().is_some_and(|c| c.cubic) {
                        self.notes.borrow_mut().add(
                            "cubic animation curves are sampled at their keys and interpolated linearly",
                        );
                    }
                }
                channels.entry(ni).or_default().insert(kind, axes);
            }
        }
        if channels.is_empty() && weights.is_empty() {
            return None;
        }
        let mut all_times: BTreeSet<i64> = BTreeSet::new();
        for curves in weights.values() {
            for c in curves.iter().flatten() {
                all_times.extend(c.times.iter().copied());
            }
        }
        for ch in channels.values() {
            for axes in ch.values() {
                for a in axes {
                    if let Some(c) = &a.curve {
                        all_times.extend(c.times.iter().copied());
                    }
                }
            }
        }
        let min_time = *all_times.first()?;
        let base = start.filter(|&s| s <= min_time).unwrap_or(min_time);

        let mut gl_channels = Vec::new();
        let mut samplers = Vec::new();
        for (ni, ch) in &channels {
            let mut times: BTreeSet<i64> = BTreeSet::new();
            for axes in ch.values() {
                for a in axes {
                    if let Some(c) = &a.curve {
                        times.extend(c.times.iter().copied());
                    }
                }
            }
            if times.is_empty() {
                continue;
            }
            let v = self.view(models[*ni]);
            let (mut ts, mut rs, mut ss) = (Vec::new(), Vec::new(), Vec::new());
            let mut prev: Option<DQuat> = None;
            for &t in &times {
                let eval = |kind: char, stat: DVec3| -> DVec3 {
                    let Some(axes) = ch.get(&kind) else { return stat };
                    let mut out = stat;
                    for (k, a) in axes.iter().enumerate() {
                        out[k] = match &a.curve {
                            Some(c) => c.eval(t),
                            None => a.default.unwrap_or(stat[k]),
                        };
                    }
                    out
                };
                let over = [
                    eval('T', v.vec3("Lcl Translation", DVec3::ZERO)),
                    eval('R', v.vec3("Lcl Rotation", DVec3::ZERO)),
                    eval('S', v.vec3("Lcl Scaling", DVec3::ONE)),
                ];
                let (tt, rr, sc) = local_trs(&v, Some(over));
                let (tt, mut rr, sc) = self.to_gltf_trs(tt, rr, sc);
                if let Some(p) = prev
                    && p.dot(rr) < 0.0
                {
                    rr = -rr;
                }
                prev = Some(rr);
                ts.extend(f32s(tt.to_array()));
                rs.extend(f32s(rr.to_array()));
                ss.extend(f32s(sc.to_array()));
            }
            let secs: Vec<f32> = times
                .iter()
                .map(|&t| ((t - base) as f64 / KTIME_PER_SECOND) as f32)
                .collect();
            let input = out.times(&secs);
            for (path, data, ty) in [
                ("translation", ts, "VEC3"),
                ("rotation", rs, "VEC4"),
                ("scale", ss, "VEC3"),
            ] {
                let output = out.floats(&data, ty);
                gl_channels.push(json!({
                    "sampler": samplers.len(),
                    "target": { "node": ni, "path": path },
                }));
                samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
            }
        }
        for (ni, curves) in &weights {
            let times: BTreeSet<i64> = curves
                .iter()
                .flatten()
                .flat_map(|c| c.times.iter().copied())
                .collect();
            if times.is_empty() {
                continue;
            }
            let defaults = &morph_defaults[ni];
            let mut data = Vec::with_capacity(times.len() * defaults.len());
            for &t in &times {
                for (k, d) in defaults.iter().enumerate() {
                    data.push(match curves.get(k).and_then(Option::as_ref) {
                        Some(c) => (c.eval(t) / 100.0) as f32,
                        None => *d,
                    });
                }
            }
            let secs: Vec<f32> = times
                .iter()
                .map(|&t| ((t - base) as f64 / KTIME_PER_SECOND) as f32)
                .collect();
            let input = out.times(&secs);
            let output = out.floats(&data, "SCALAR");
            gl_channels.push(json!({
                "sampler": samplers.len(),
                "target": { "node": ni, "path": "weights" },
            }));
            samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
        }
        if gl_channels.is_empty() {
            return None;
        }
        Some(json!({
            "name": self.objects[&stack].name,
            "channels": gl_channels,
            "samplers": samplers,
        }))
    }
}

/// FBX rotation (degrees, FBX rotation order) as a quaternion.
fn euler(v: DVec3, order: i64) -> DQuat {
    let x = DQuat::from_rotation_x(v.x.to_radians());
    let y = DQuat::from_rotation_y(v.y.to_radians());
    let z = DQuat::from_rotation_z(v.z.to_radians());
    // Order "ABC" applies A first: the matrix is C * B * A.
    match order {
        1 => y * z * x, // XZY
        2 => x * z * y, // YZX
        3 => z * x * y, // YXZ
        4 => y * x * z, // ZXY
        5 => x * y * z, // ZYX
        _ => z * y * x, // XYZ (and spheric XYZ)
    }
}

/// Local transform of a model as translation/rotation/scale, in FBX space. `over` replaces the
/// static Lcl Translation/Rotation/Scaling (for animation sampling).
fn local_trs(v: &PropView, over: Option<[DVec3; 3]>) -> (DVec3, DQuat, DVec3) {
    let [lt, lr, ls] = over.unwrap_or([
        v.vec3("Lcl Translation", DVec3::ZERO),
        v.vec3("Lcl Rotation", DVec3::ZERO),
        v.vec3("Lcl Scaling", DVec3::ONE),
    ]);
    let active = v.int("RotationActive", 0) != 0;
    let (pre, post, order) = if active {
        (
            euler(v.vec3("PreRotation", DVec3::ZERO), 0),
            euler(v.vec3("PostRotation", DVec3::ZERO), 0),
            v.int("RotationOrder", 0),
        )
    } else {
        (DQuat::IDENTITY, DQuat::IDENTITY, 0)
    };
    let r = (pre * euler(lr, order) * post.inverse()).normalize();
    let t = DMat4::from_translation;
    let roff = v.vec3("RotationOffset", DVec3::ZERO);
    let rp = v.vec3("RotationPivot", DVec3::ZERO);
    let soff = v.vec3("ScalingOffset", DVec3::ZERO);
    let sp = v.vec3("ScalingPivot", DVec3::ZERO);
    let m = t(lt)
        * t(roff)
        * t(rp)
        * DMat4::from_quat(r)
        * t(-rp)
        * t(soff)
        * t(sp)
        * DMat4::from_scale(ls)
        * t(-sp);
    (m.w_axis.truncate(), r, ls)
}

/// The geometric transform (applies to the geometry only, not inherited).
fn geometric(v: &PropView) -> DMat4 {
    DMat4::from_scale_rotation_translation(
        v.vec3("GeometricScaling", DVec3::ONE),
        euler(v.vec3("GeometricRotation", DVec3::ZERO), 0),
        v.vec3("GeometricTranslation", DVec3::ZERO),
    )
}

fn f32s<const N: usize>(v: [f64; N]) -> [f32; N] {
    v.map(|x| x as f32)
}

fn first_layer<'a>(g: &'a Node, name: &'a str) -> Option<&'a Node> {
    g.children_named(name)
        .find(|l| l.prop(0).and_then(Prop::as_i64) == Some(0))
        .or_else(|| g.child(name))
}

struct Layer {
    data: Vec<f64>,
    index: Option<Vec<i64>>,
    mapping: String,
    width: usize,
}

impl Layer {
    fn new(l: &Node, data: &str, index: &str, width: usize) -> Self {
        let s = |k: &str| l.value(k).and_then(Prop::as_str).unwrap_or("").to_string();
        let reference = s("ReferenceInformationType");
        Self {
            data: l.array(data).and_then(Prop::f64s).unwrap_or_default(),
            index: (reference == "IndexToDirect" || reference == "Index")
                .then(|| l.array(index).and_then(Prop::i64s))
                .flatten(),
            mapping: s("MappingInformationType"),
            width,
        }
    }

    fn get(&self, pv: usize, cp: usize, poly: usize) -> Option<&[f64]> {
        let i = match self.mapping.as_str() {
            "ByPolygonVertex" => pv,
            "ByVertice" | "ByVertex" | "ByControlPoint" => cp,
            "ByPolygon" => poly,
            "AllSame" => 0,
            _ => return None,
        };
        let i = match &self.index {
            Some(ix) => usize::try_from(*ix.get(i)?).ok()?,
            None => i,
        };
        self.data.get(i * self.width..(i + 1) * self.width)
    }
}

struct SkinData {
    joints: Vec<usize>,
    ibms: Vec<DMat4>,
    per_cp: Vec<([u16; 4], [f32; 4])>,
    unweighted: usize,
    over_four: usize,
}

#[derive(Default)]
struct MeshOut {
    /// Control point of each output vertex.
    cps: Vec<u32>,
    /// FBX geometry space to glTF space (linear part), for blend shape offsets.
    linear: DMat3,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    prims: Vec<(Option<usize>, Vec<u32>)>,
}

struct Morph {
    name: String,
    /// The FBX blend shape channel (animated through `DeformPercent`).
    channel: i64,
    default_weight: f32,
    deltas: Vec<[f32; 3]>,
}

#[derive(Default)]
struct Channel {
    curve: Option<Curve>,
    default: Option<f64>,
}

struct Curve {
    times: Vec<i64>,
    values: Vec<f64>,
    constant: Vec<bool>,
    cubic: bool,
}

impl Curve {
    fn read(n: &Node) -> Option<Self> {
        let times = n.array("KeyTime").and_then(Prop::i64s)?;
        let values = n.array("KeyValueFloat").and_then(Prop::f64s)?;
        if times.is_empty() || times.len() != values.len() {
            return None;
        }
        let flags = n.array("KeyAttrFlags").and_then(Prop::i64s).unwrap_or_default();
        let counts = n
            .array("KeyAttrRefCount")
            .and_then(Prop::i64s)
            .unwrap_or_default();
        let mut per_key = Vec::with_capacity(times.len());
        for (f, c) in flags.iter().zip(counts.iter().chain(std::iter::repeat(&1))) {
            for _ in 0..(*c).max(0) {
                per_key.push(*f);
            }
        }
        per_key.resize(times.len(), per_key.last().copied().unwrap_or(4));
        Some(Self {
            constant: per_key.iter().map(|f| f & 0x02 != 0).collect(),
            cubic: per_key
                .iter()
                .take(times.len().saturating_sub(1))
                .any(|f| f & 0x08 != 0),
            times,
            values,
        })
    }

    fn eval(&self, t: i64) -> f64 {
        match self.times.binary_search(&t) {
            Ok(i) => self.values[i],
            Err(0) => self.values[0],
            Err(i) if i >= self.times.len() => *self.values.last().unwrap(),
            Err(i) => {
                let (a, b) = (i - 1, i);
                if self.constant[a] {
                    return self.values[a];
                }
                let f = (t - self.times[a]) as f64 / (self.times[b] - self.times[a]) as f64;
                self.values[a] + (self.values[b] - self.values[a]) * f
            }
        }
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// glTF writer

#[derive(Default)]
struct Gltf {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    images: Vec<Value>,
    textures: Vec<Value>,
}

impl Gltf {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut v = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    fn accessor(&mut self, a: Value) -> usize {
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn floats(&mut self, data: &[f32], ty: &str) -> usize {
        let comps = match ty {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 16,
        };
        let bytes: Vec<u8> = data.iter().flat_map(|x| x.to_le_bytes()).collect();
        let v = self.view(&bytes, None);
        self.accessor(json!({
            "bufferView": v, "componentType": 5126, "count": data.len() / comps, "type": ty,
        }))
    }

    fn vec3s(&mut self, data: &[[f32; 3]], bounds: bool) -> usize {
        let flat: Vec<f32> = data.concat();
        let a = self.floats(&flat, "VEC3");
        if bounds {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for p in data {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            self.accessors[a]["min"] = json!(lo);
            self.accessors[a]["max"] = json!(hi);
        }
        a
    }

    fn times(&mut self, secs: &[f32]) -> usize {
        let a = self.floats(secs, "SCALAR");
        let lo = secs.iter().copied().fold(f32::MAX, f32::min);
        let hi = secs.iter().copied().fold(f32::MIN, f32::max);
        self.accessors[a]["min"] = json!([lo]);
        self.accessors[a]["max"] = json!([hi]);
        a
    }

    fn joints(&mut self, data: &[[u16; 4]]) -> usize {
        let bytes: Vec<u8> = data.iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
        let v = self.view(&bytes, None);
        self.accessor(json!({ "bufferView": v, "componentType": 5123, "count": data.len(), "type": "VEC4" }))
    }

    fn indices(&mut self, idx: &[u32]) -> usize {
        let bytes: Vec<u8> = idx.iter().flat_map(|x| x.to_le_bytes()).collect();
        let v = self.view(&bytes, Some(34963));
        self.accessor(json!({ "bufferView": v, "componentType": 5125, "count": idx.len(), "type": "SCALAR" }))
    }

    fn image(&mut self, bytes: Vec<u8>, mime: &str, name: &str) -> usize {
        let v = self.view(&bytes, None);
        self.images
            .push(json!({ "bufferView": v, "mimeType": mime, "name": name }));
        self.textures
            .push(json!({ "source": self.images.len() - 1, "sampler": 0 }));
        self.textures.len() - 1
    }

    fn finish(mut self, mut root: Value) -> Vec<u8> {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        if !self.bin.is_empty() {
            root["buffers"] = json!([{ "byteLength": self.bin.len() }]);
            root["bufferViews"] = Value::Array(self.views);
            root["accessors"] = Value::Array(self.accessors);
        }
        let mut js = serde_json::to_vec(&root).expect("json");
        while !js.len().is_multiple_of(4) {
            js.push(b' ');
        }
        let total = 12
            + 8
            + js.len()
            + if self.bin.is_empty() {
                0
            } else {
                8 + self.bin.len()
            };
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(js.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&js);
        if !self.bin.is_empty() {
            out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&self.bin);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_orders_match_matrix_products() {
        let v = DVec3::new(10.0, 20.0, 30.0);
        let (x, y, z) = (
            DMat3::from_rotation_x(10f64.to_radians()),
            DMat3::from_rotation_y(20f64.to_radians()),
            DMat3::from_rotation_z(30f64.to_radians()),
        );
        let expect = [z * y * x, y * z * x, x * z * y, z * x * y, y * x * z, x * y * z];
        for (order, m) in expect.iter().enumerate() {
            let q = DMat3::from_quat(euler(v, order as i64));
            assert!(q.abs_diff_eq(*m, 1e-12), "order {order}");
        }
    }

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVs\nbG8h").unwrap(), b"hello!");
        assert!(base64_decode("a$b").is_none());
    }

    #[test]
    fn curves_interpolate_linear_and_constant() {
        let c = Curve {
            times: vec![0, 10, 20],
            values: vec![0.0, 10.0, 0.0],
            constant: vec![false, true, false],
            cubic: false,
        };
        assert_eq!(c.eval(-5), 0.0);
        assert_eq!(c.eval(5), 5.0);
        assert_eq!(c.eval(15), 10.0);
        assert_eq!(c.eval(25), 0.0);
    }
}
