// GPU skinning. Appended to pbr.wgsl (same view, lights, material and fragment shader);
// only the vertex stage differs. Every skinned instance has its own joint palette in one
// storage buffer: joint matrix = pose (model space) * inverse bind matrix.

@group(3) @binding(0) var<storage, read> joint_mats: array<mat4x4<f32>>;

struct SkinIn {
    @location(9) joint_base: vec4<u32>, // x = first palette entry of this instance
    @location(10) joints: vec4<u32>,
    @location(11) weights: vec4<f32>,
};

@vertex
fn vs_skinned(in: VsIn, inst: InstIn, skin: SkinIn) -> VsOut {
    var out: VsOut;
    let b = skin.joint_base.x;
    let s = joint_mats[b + skin.joints.x] * skin.weights.x
          + joint_mats[b + skin.joints.y] * skin.weights.y
          + joint_mats[b + skin.joints.z] * skin.weights.z
          + joint_mats[b + skin.joints.w] * skin.weights.w;
    let local = s * vec4<f32>(in.position, 1.0);
    // Joint matrices are rigid or uniformly scaled in practice, so they carry normals
    // directly (the fragment shader normalizes).
    let local_n = (s * vec4<f32>(in.normal, 0.0)).xyz;

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
    return out;
}
