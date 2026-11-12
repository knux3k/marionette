// GPU skinning and morph targets. Appended to pbr.wgsl (same view, lights, material and
// fragment shader); only the vertex stage differs. Every instance has its own joint palette
// (joint matrix = pose in model space * inverse bind matrix) and its own morph weights.

@group(3) @binding(0) var<storage, read> joint_mats: array<mat4x4<f32>>;
@group(3) @binding(1) var<storage, read> morph_weights: array<f32>;
// Per part: for target t and vertex v, position delta at 2 * (t * vertex_count + v), normal
// delta right after it.
@group(3) @binding(2) var<storage, read> morph_deltas: array<vec4<f32>>;

struct SkinIn {
    // x = first palette entry, y = first morph weight, z = morph target count, w = vertex count
    @location(9) joint_base: vec4<u32>,
    @location(10) joints: vec4<u32>,
    @location(11) weights: vec4<f32>,
};

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
    var position = in.position;
    var normal = in.normal;
    for (var t = 0u; t < skin.joint_base.z; t = t + 1u) {
        let w = morph_weights[skin.joint_base.y + t];
        if (w != 0.0) {
            let d = 2u * (t * skin.joint_base.w + vi);
            position = position + w * morph_deltas[d].xyz;
            normal = normal + w * morph_deltas[d + 1u].xyz;
        }
    }
    let b = skin.joint_base.x;
    let s = joint_mats[b + skin.joints.x] * skin.weights.x
          + joint_mats[b + skin.joints.y] * skin.weights.y
          + joint_mats[b + skin.joints.z] * skin.weights.z
          + joint_mats[b + skin.joints.w] * skin.weights.w;
    let local = s * vec4<f32>(position, 1.0);
    // Joint matrices are rigid or uniformly scaled in practice, so they carry normals
    // directly (the fragment shader normalizes).
    let local_n = (s * vec4<f32>(normal, 0.0)).xyz;

    let model = mat4x4<f32>(inst.model0, inst.model1, inst.model2, inst.model3);
    let c0 = inst.model0.xyz;
    let c1 = inst.model1.xyz;
    let c2 = inst.model2.xyz;
    let k0 = cross(c1, c2);
    let flip = select(1.0, -1.0, dot(c0, k0) < 0.0);
    let cof = mat3x3<f32>(k0, cross(c2, c0), cross(c0, c1)) * flip;
    let world = model * local;
    out.clip = view.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = cof * local_n;
    out.uv = in.uv;
    out.base_color = inst.base_color;
    out.params = inst.params;
    out.emissive = inst.emissive;
    return out;
}
