// World-space quads whose texture's red channel is a signed distance field (0.5 on the edge).
// The edge is smoothed over about one screen pixel (fwidth), so text and shapes stay crisp at
// any size, distance and angle without a texture per size.

struct ViewU {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: ViewU;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = view.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = textureSample(tex, samp, in.uv).r;
    let w = max(fwidth(d) * 0.75, 0.0005);
    let a = smoothstep(0.5 - w, 0.5 + w, d);
    return vec4<f32>(in.color.rgb, in.color.a * a);
}
