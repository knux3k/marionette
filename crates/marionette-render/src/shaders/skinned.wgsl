// GPU skinning and morph targets. Appended to pbr.wgsl (same view, lights, material and
// fragment shader); only the vertex stage differs. Every instance has its own joint palette
// (joint matrix = pose in model space * inverse bind matrix) and its own morph weights. The
// deforming maths is in skin_core.wgsl, appended after this file.

@group(3) @binding(0) var<storage, read> joint_mats: array<mat4x4<f32>>;
@group(3) @binding(1) var<storage, read> morph_weights: array<f32>;
// Per part: for target t and vertex v, position delta at 2 * (t * vertex_count + v), normal
// delta right after it.
@group(3) @binding(2) var<storage, read> morph_deltas: array<vec4<f32>>;

@vertex
fn vs_skinned(in: VsIn, inst: InstIn, skin: SkinIn, @builtin(vertex_index) vi: u32) -> VsOut {
    return skin_vertex(in, inst, skin, vi);
}

// Depth-only pass from the shadow-casting light (group 0 holds the light's view-projection).
@vertex
fn vs_skinned_shadow(in: VsIn, inst: InstIn, skin: SkinIn, @builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    return skin_vertex(in, inst, skin, vi).clip;
}

fn skin_vertex(in: VsIn, inst: InstIn, skin: SkinIn, vi: u32) -> VsOut {
    var out: VsOut;
    let local = skin_local(in, skin, vi);

    let model = mat4x4<f32>(inst.model0, inst.model1, inst.model2, inst.model3);
    let c0 = inst.model0.xyz;
    let c1 = inst.model1.xyz;
    let c2 = inst.model2.xyz;
    let k0 = cross(c1, c2);
    let flip = select(1.0, -1.0, dot(c0, k0) < 0.0);
    let cof = mat3x3<f32>(k0, cross(c2, c0), cross(c0, c1)) * flip;
    let world = model * vec4<f32>(local.position, 1.0);
    out.clip = view.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = cof * local.normal;
    out.uv = in.uv;
    out.base_color = inst.base_color;
    out.params = inst.params;
    out.emissive = inst.emissive;
    out.color = in.color;
    return out;
}
