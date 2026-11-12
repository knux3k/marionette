// Forward PBR: metallic-roughness (glTF), GGX specular, Lambert diffuse, directional
// lights plus an ambient term. Output is linear; the sRGB target encodes it.
// Opt-in looks (decision 0063), each off unless its flag is set: shadows from the first
// directional light, point and spot lights, sky (hemisphere) ambient, emissive, filmic tone
// mapping and fog. With every flag off the result is the plain look, bit for bit.

struct ViewU {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

const FLAG_SHADOWS: u32 = 1u;
const FLAG_TONEMAP: u32 = 2u;
const FLAG_FOG: u32 = 4u;
const FLAG_SKY: u32 = 8u;
const MAX_LOCAL: u32 = 16u;

struct LightsU {
    direction: array<vec4<f32>, 4>, // xyz = direction the light travels
    radiance: array<vec4<f32>, 4>,  // rgb = color * illuminance
    ambient: vec4<f32>,             // rgb (sky color with FLAG_SKY)
    count: vec4<u32>,               // directional lights, local lights, flags
    ground: vec4<f32>,              // rgb = ground ambient (FLAG_SKY)
    fog: vec4<f32>,                 // rgb = color, w = density
    post: vec4<f32>,                // exposure, shadow texel size, softness (texels), normal offset
    shadow_view_proj: mat4x4<f32>,
    local_pos: array<vec4<f32>, 16>, // xyz = position, w = range
    local_rad: array<vec4<f32>, 16>, // rgb = color * intensity, w = cos outer (< -1: point)
    local_dir: array<vec4<f32>, 16>, // xyz = direction, w = cos inner
};


@group(0) @binding(0) var<uniform> view: ViewU;
@group(1) @binding(0) var<uniform> lights: LightsU;
@group(1) @binding(1) var shadow_map: texture_depth_2d;
@group(1) @binding(2) var shadow_samp: sampler_comparison;
@group(2) @binding(1) var base_tex: texture_2d<f32>;
@group(2) @binding(2) var mr_tex: texture_2d<f32>;
@group(2) @binding(3) var normal_tex: texture_2d<f32>;
@group(2) @binding(4) var samp: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

// Per-instance data: one entry per drawn copy of a mesh part (GPU instancing).
struct InstIn {
    @location(3) model0: vec4<f32>,
    @location(4) model1: vec4<f32>,
    @location(5) model2: vec4<f32>,
    @location(6) model3: vec4<f32>,
    @location(7) base_color: vec4<f32>,
    @location(8) params: vec4<f32>, // metallic, roughness, normal_scale, has_normal_map (0/1)
    @location(12) emissive: vec4<f32>, // rgb = light given off
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) base_color: vec4<f32>,
    @location(4) @interpolate(flat) params: vec4<f32>,
    @location(5) @interpolate(flat) emissive: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn, inst: InstIn) -> VsOut {
    var out: VsOut;
    let model = mat4x4<f32>(inst.model0, inst.model1, inst.model2, inst.model3);
    // Normal matrix as the cofactor matrix of the model's 3x3 part: the inverse-transpose
    // times the determinant. The fragment shader normalizes, so the scale factor cancels;
    // sign(det) keeps normals pointing out on mirrored instances. No CPU inverse needed.
    let c0 = inst.model0.xyz;
    let c1 = inst.model1.xyz;
    let c2 = inst.model2.xyz;
    let k0 = cross(c1, c2);
    let flip = select(1.0, -1.0, dot(c0, k0) < 0.0);
    let cof = mat3x3<f32>(k0, cross(c2, c0), cross(c0, c1)) * flip;
    let world = model * vec4<f32>(in.position, 1.0);
    out.clip = view.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = cof * in.normal;
    out.uv = in.uv;
    out.base_color = inst.base_color;
    out.params = inst.params;
    out.emissive = inst.emissive;
    return out;
}

// Depth-only pass from the shadow-casting light: group 0 holds the light's view-projection.
@vertex
fn vs_shadow(in: VsIn, inst: InstIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(inst.model0, inst.model1, inst.model2, inst.model3);
    return view.view_proj * model * vec4<f32>(in.position, 1.0);
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

// Fraction of the first directional light reaching `p` (1 = lit), filtered over 3x3 taps.
fn shadow_factor(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let offset = n * lights.post.w;
    let clip = lights.shadow_view_proj * vec4<f32>(p + offset, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0 || ndc.z < 0.0) {
        return 1.0;
    }
    let step = lights.post.y * lights.post.z;
    var lit = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * step;
            lit += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, ndc.z);
        }
    }
    return lit / 9.0;
}

// GGX specular plus Lambert diffuse for one light arriving from direction `l`.
fn shade(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, base: vec3<f32>, f0: vec3<f32>, metallic: f32, a2: f32) -> vec3<f32> {
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = normalize(v + l);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);
    let d_den = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * d_den * d_den);
    let vis = 0.5 / (n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2)
                   + n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2));
    let f = f0 + (1.0 - f0) * pow(1.0 - v_dot_h, 5.0);
    let diffuse = (1.0 - f) * (1.0 - metallic) * base / PI;
    return (diffuse + d * vis * f) * n_dot_l * PI;
}

// Filmic curve (Narkowicz's fit of ACES): keeps mid-tones, rolls highlights off to white.
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = in.base_color * textureSample(base_tex, samp, in.uv);
    let mr = textureSample(mr_tex, samp, in.uv);
    let metallic = clamp(in.params.x * mr.b, 0.0, 1.0);
    let roughness = clamp(in.params.y * mr.g, 0.045, 1.0);

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let nm = textureSample(normal_tex, samp, in.uv).xyz;
    if (in.params.w > 0.5) {
        n = perturb_normal(n, in.world_pos, in.uv, nm, in.params.z);
    }

    let v = normalize(view.eye.xyz - in.world_pos);
    let n_dot_v = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), base.rgb, metallic);
    let a = roughness * roughness;
    let a2 = a * a;

    let flags = lights.count.z;
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
        var lit = (diffuse + d * vis * f) * lights.radiance[i].rgb * n_dot_l * PI;
        if (i == 0u && (flags & FLAG_SHADOWS) != 0u) {
            lit = lit * shadow_factor(in.world_pos, normalize(in.normal));
        }
        color += lit;
    }
    // Point and spot lights: inverse-square falloff, windowed to zero at the range.
    for (var j = 0u; j < min(lights.count.y, MAX_LOCAL); j = j + 1u) {
        let to_light = lights.local_pos[j].xyz - in.world_pos;
        let dist2 = max(dot(to_light, to_light), 1e-4);
        let l = to_light * inverseSqrt(dist2);
        let range = lights.local_pos[j].w;
        let ratio = dist2 / (range * range);
        let window = clamp(1.0 - ratio * ratio, 0.0, 1.0);
        var atten = window * window / dist2;
        let cos_outer = lights.local_rad[j].w;
        if (cos_outer >= -1.0) {
            let cos_inner = lights.local_dir[j].w;
            let cd = dot(-l, lights.local_dir[j].xyz);
            atten = atten * smoothstep(cos_outer, cos_inner, cd);
        }
        if (atten > 0.0) {
            color += shade(n, v, l, base.rgb, f0, metallic, a2) * lights.local_rad[j].rgb * atten;
        }
    }
    // Ambient: diffuse part plus a rough stand-in for reflected sky (no environment map yet).
    var ambient_light = lights.ambient.rgb;
    if ((flags & FLAG_SKY) != 0u) {
        ambient_light = mix(lights.ground.rgb, lights.ambient.rgb, n.y * 0.5 + 0.5);
    }
    let ambient = ambient_light * (base.rgb * (1.0 - metallic) + f0 * (1.0 - 0.5 * roughness));
    color += ambient;
    color += in.emissive.rgb;
    if ((flags & FLAG_TONEMAP) != 0u) {
        color = tonemap(color * lights.post.x);
    }
    if ((flags & FLAG_FOG) != 0u) {
        let dist = length(view.eye.xyz - in.world_pos) * lights.fog.w;
        color = mix(lights.fog.rgb, color, exp(-dist * dist));
    }
    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), base.a);
}
