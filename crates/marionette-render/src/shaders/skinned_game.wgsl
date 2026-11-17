// Skinned game shaders (decision 0185): appended to a composed game shader (pbr.wgsl with the
// game's hooks, its group-3 bindings and source) and skin_core.wgsl. The skin buffers share
// group 3 with the game material, at bindings 6 to 8. The vertex is skinned in model space,
// then goes through the game's `vertex` function like a rigid one.

@group(3) @binding(6) var<storage, read> joint_mats: array<mat4x4<f32>>;
@group(3) @binding(7) var<storage, read> morph_weights: array<f32>;
@group(3) @binding(8) var<storage, read> morph_deltas: array<vec4<f32>>;

@vertex
fn vs_skinned_hooked(in: VsIn, inst: InstIn, skin: SkinIn, @builtin(vertex_index) vi: u32) -> VsOut {
    let v = hooked_vertex(skin_local(in, skin, vi), inst);
    var out: VsOut;
    out.clip = view.view_proj * vec4<f32>(v.position, 1.0);
    out.world_pos = v.position;
    out.normal = v.normal;
    out.uv = v.uv;
    out.base_color = v.base_color;
    out.params = v.params;
    out.emissive = inst.emissive;
    out.color = v.color;
    return out;
}
