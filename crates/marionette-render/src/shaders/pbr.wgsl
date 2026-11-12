// Forward PBR: metallic-roughness (glTF), GGX specular, Lambert diffuse, directional
// lights plus an ambient term. Output is linear; the sRGB target encodes it.

struct ViewU {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

struct LightsU {
    direction: array<vec4<f32>, 4>, // xyz = direction the light travels
    radiance: array<vec4<f32>, 4>,  // rgb = color * illuminance
    ambient: vec4<f32>,
    count: vec4<u32>,
};

struct DrawU {
    model: mat4x4<f32>,
    normal: mat4x4<f32>, // inverse-transpose of model (upper 3x3 used)
    base_color: vec4<f32>,
    params: vec4<f32>, // metallic, roughness, normal_scale, has_normal_map (0/1)
};

@group(0) @binding(0) var<uniform> view: ViewU;
@group(1) @binding(0) var<uniform> lights: LightsU;
@group(2) @binding(1) var base_tex: texture_2d<f32>;
@group(2) @binding(2) var mr_tex: texture_2d<f32>;
@group(2) @binding(3) var normal_tex: texture_2d<f32>;
@group(2) @binding(4) var samp: sampler;
@group(3) @binding(0) var<uniform> draw: DrawU;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let world = draw.model * vec4<f32>(in.position, 1.0);
    out.clip = view.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = (draw.normal * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

const PI: f32 = 3.14159265;

// Tangent frame from screen-space derivatives (no tangents needed in the mesh).
// glTF normal maps use +Y toward the top of the image (decreasing v), hence -b.
fn perturb_normal(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, sample: vec3<f32>, scale: f32) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let len2 = max(dot(t, t), dot(b, b));
    if (len2 <= 1e-20) {
        return n;
    }
    let inv = inverseSqrt(len2);
    var m = sample * 2.0 - 1.0;
    m = vec3<f32>(m.xy * scale, m.z);
    return normalize(t * inv * m.x - b * inv * m.y + n * m.z);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = draw.base_color * textureSample(base_tex, samp, in.uv);
    let mr = textureSample(mr_tex, samp, in.uv);
    let metallic = clamp(draw.params.x * mr.b, 0.0, 1.0);
    let roughness = clamp(draw.params.y * mr.g, 0.045, 1.0);

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let nm = textureSample(normal_tex, samp, in.uv).xyz;
    if (draw.params.w > 0.5) {
        n = perturb_normal(n, in.world_pos, in.uv, nm, draw.params.z);
    }

    let v = normalize(view.eye.xyz - in.world_pos);
    let n_dot_v = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), base.rgb, metallic);
    let a = roughness * roughness;
    let a2 = a * a;

    var color = vec3<f32>(0.0);
    for (var i = 0u; i < lights.count.x; i = i + 1u) {
        let l = -normalize(lights.direction[i].xyz);
        let h = normalize(v + l);
        let n_dot_l = max(dot(n, l), 0.0);
        if (n_dot_l <= 0.0) {
            continue;
        }
        let n_dot_h = max(dot(n, h), 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        let d_den = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
        let d = a2 / (PI * d_den * d_den);
        let vis = 0.5 / (n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2)
                       + n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2));
        let f = f0 + (1.0 - f0) * pow(1.0 - v_dot_h, 5.0);
        let diffuse = (1.0 - f) * (1.0 - metallic) * base.rgb / PI;
        // Scaled by PI so illuminance 1.0 lights a white diffuse surface to 1.0.
        color += (diffuse + d * vis * f) * lights.radiance[i].rgb * n_dot_l * PI;
    }
    // Ambient: diffuse part plus a rough stand-in for reflected sky (no environment map yet).
    let ambient = lights.ambient.rgb * (base.rgb * (1.0 - metallic) + f0 * (1.0 - 0.5 * roughness));
    color += ambient;
    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), base.a);
}
