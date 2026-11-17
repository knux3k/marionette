// The deforming maths shared by the skinned shader (skinned.wgsl) and skinned game shaders
// (skinned_game.wgsl, decision 0185). Expects `joint_mats`, `morph_weights` and
// `morph_deltas` declared before it.

struct SkinIn {
    // x = first palette entry, y = first morph weight, z = morph target count, w = vertex count
    @location(9) joint_base: vec4<u32>,
    @location(10) joints: vec4<u32>,
    @location(11) weights: vec4<f32>,
};

// The vertex in model space after morph targets and the skin (normal not normalized).
fn skin_local(in: VsIn, skin: SkinIn, vi: u32) -> VsIn {
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
    var out = in;
    out.position = (s * vec4<f32>(position, 1.0)).xyz;
    // Joint matrices are rigid or uniformly scaled in practice, so they carry normals
    // directly (the fragment shader normalizes).
    out.normal = (s * vec4<f32>(normal, 0.0)).xyz;
    return out;
}
