// Forward PBR: metallic-roughness (glTF), GGX specular, Lambert diffuse, directional
// lights plus an ambient term. Output is linear; the sRGB target encodes it.
// Opt-in looks (decision 0063), each off unless its flag is set: shadows from the first
// directional light, point and spot lights, sky (hemisphere) ambient, emissive, filmic tone
// mapping and fog. Later looks (decisions 0064 to 0067): output to a floating-point target
// for bloom, shadows from point and spot lights, environment lighting and reflections, a sky
// background, and glTF emissive. With every flag off the result is the plain look, bit for bit.

struct ViewU {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
};

const FLAG_SHADOWS: u32 = 1u;
const FLAG_TONEMAP: u32 = 2u;
const FLAG_FOG: u32 = 4u;
const FLAG_SKY: u32 = 8u;
// Write unclamped linear light for the bloom pass; tone mapping happens there.
const FLAG_HDR: u32 = 16u;
const FLAG_ENV: u32 = 32u;
// Ray-traced shadows and reflections (decision 0093); only the ray tracing variant of this
// shader (raytrace.wgsl in place of the stubs below) ever sees them set.
const FLAG_RT_SHADOWS: u32 = 64u;
const FLAG_RT_REFLECT: u32 = 128u;
const FLAG_PROBES: u32 = 256u; // reflection probes (decision 0092)
const MAX_LOCAL: u32 = 16u;

struct LightsU {
    direction: array<vec4<f32>, 4>, // xyz = direction the light travels
    radiance: array<vec4<f32>, 4>,  // rgb = color * illuminance
    ambient: vec4<f32>,             // rgb (sky color with FLAG_SKY)
    count: vec4<u32>,               // directional lights, local lights, flags, sun cascades
    ground: vec4<f32>,              // rgb = ground ambient (FLAG_SKY)
    fog: vec4<f32>,                 // rgb = color, w = density
    post: vec4<f32>,                // exposure, shadow texel size, softness (texels), normal offset
    shadow_view_proj: mat4x4<f32>,
    local_pos: array<vec4<f32>, 16>, // xyz = position, w = range
    local_rad: array<vec4<f32>, 16>, // rgb = color * intensity, w = cos outer (< -1: point)
    local_dir: array<vec4<f32>, 16>, // xyz = direction, w = cos inner
    // x = first shadow layer (< 0: no shadow), y = normal offset per meter of distance,
    // z = shadow texel size (uv), w = softness (texels)
    local_shadow: array<vec4<f32>, 16>,
    shadow_mats: array<mat4x4<f32>, 24>, // view-projection per shadow layer
    env: vec4<f32>,                      // intensity, last mip level
    sh: array<vec4<f32>, 9>,             // environment irradiance / PI (SH9, rgb)
    cascade_mats: array<mat4x4<f32>, 3>, // sun shadow cascades 1 to 3 (decision 0090)
    cascade_offsets: vec4<f32>,          // normal offset (m) per cascade
    rt: vec4<f32>,                       // reflection roughness limit, tan(sun disc radius)
    probe_pos: array<vec4<f32>, 4>,      // probe center, w = 1 when captured
    probe_ext: array<vec4<f32>, 4>,      // probe box half size
};

struct MaterialU {
    emissive: vec4<f32>, // rgb = glTF emissive factor times strength
};


@group(0) @binding(0) var<uniform> view: ViewU;
@group(1) @binding(0) var<uniform> lights: LightsU;
@group(1) @binding(1) var shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var shadow_samp: sampler_comparison;
@group(1) @binding(3) var local_shadow_map: texture_depth_2d_array;
@group(1) @binding(4) var env_tex: texture_2d<f32>;
@group(1) @binding(5) var env_samp: sampler;
@group(1) @binding(10) var probe_tex: texture_2d_array<f32>;
@group(2) @binding(0) var<uniform> material: MaterialU;
@group(2) @binding(1) var base_tex: texture_2d<f32>;
@group(2) @binding(2) var mr_tex: texture_2d<f32>;
@group(2) @binding(3) var normal_tex: texture_2d<f32>;
@group(2) @binding(4) var samp: sampler;
@group(2) @binding(5) var emissive_tex: texture_2d<f32>;

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

// RT:BEGIN Ray tracing stubs. The ray tracing variant (raytrace.rs) replaces them, up to the
// end marker, with shaders/raytrace.wgsl; they keep the plain shader free of ray queries.
// Fraction of light reaching `p` from the sun along `l` (pixel `px` picks soft-shadow rays).
fn rt_sun(p: vec3<f32>, ng: vec3<f32>, l: vec3<f32>, px: vec2<f32>) -> f32 {
    return 1.0;
}
// 1 when nothing blocks the segment from `p` toward `l` up to `dist` (local lights).
fn rt_visible(p: vec3<f32>, ng: vec3<f32>, l: vec3<f32>, dist: f32, mask: u32) -> f32 {
    return 1.0;
}
// Light arriving at `p` along the mirror direction `r` (rgb); a = 0 when nothing was traced.
fn rt_reflect(p: vec3<f32>, ng: vec3<f32>, r: vec3<f32>, roughness: f32) -> vec4<f32> {
    return vec4<f32>(0.0);
}
// RT:END

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
// The normal offset grows with the angle between surface and light (none when the light is
// straight on, where it would only push the shadow away from its caster's base). With
// cascades (decision 0090) the first cascade whose map holds the point is used.
fn shadow_factor(p: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    let n_dot_l = clamp(dot(n, l), 0.0, 1.0);
    let slope = sqrt(1.0 - n_dot_l * n_dot_l);
    let count = max(lights.count.w, 1u);
    let step = lights.post.y * lights.post.z;
    // Keep the 3x3 filter inside the map before trusting a cascade.
    let margin = select(0.0, step + lights.post.y, count > 1u);
    for (var c = 0u; c < count; c = c + 1u) {
        var m = lights.shadow_view_proj;
        if (c > 0u) {
            m = lights.cascade_mats[c - 1u];
        }
        let offset = n * (lights.cascade_offsets[c] * slope);
        let clip = m * vec4<f32>(p + offset, 1.0);
        let ndc = clip.xyz / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (uv.x < margin || uv.x > 1.0 - margin || uv.y < margin || uv.y > 1.0 - margin || ndc.z > 1.0 || ndc.z < 0.0) {
            continue;
        }
        var lit = 0.0;
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let o = vec2<f32>(f32(x), f32(y)) * step;
                lit += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, i32(c), ndc.z);
            }
        }
        return lit / 9.0;
    }
    return 1.0;
}

// Fraction of local light `j` reaching `p` (1 = lit). A spot light has one shadow layer; a
// point light six, one per cube face (+x, -x, +y, -y, +z, -z), picked by the major axis.
fn local_shadow_factor(j: u32, p: vec3<f32>, n: vec3<f32>, l: vec3<f32>, dist: f32) -> f32 {
    let info = lights.local_shadow[j];
    var layer = u32(info.x);
    if (lights.local_rad[j].w < -1.0) {
        let d = p - lights.local_pos[j].xyz;
        let a = abs(d);
        if (a.x >= a.y && a.x >= a.z) {
            layer = layer + select(1u, 0u, d.x > 0.0);
        } else if (a.y >= a.z) {
            layer = layer + select(3u, 2u, d.y > 0.0);
        } else {
            layer = layer + select(5u, 4u, d.z > 0.0);
        }
    }
    let n_dot_l = clamp(dot(n, l), 0.0, 1.0);
    let offset = n * (info.y * dist * (0.5 + sqrt(1.0 - n_dot_l * n_dot_l)));
    let clip = lights.shadow_mats[layer] * vec4<f32>(p + offset, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (clip.w <= 0.0 || uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    let step = info.z * info.w;
    var lit = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * step;
            lit += textureSampleCompareLevel(local_shadow_map, shadow_samp, clamp(uv + o, vec2<f32>(0.0), vec2<f32>(1.0)), i32(layer), ndc.z);
        }
    }
    return lit / 9.0;
}

// Equirectangular lookup: u turns around +y starting at -z, v runs from up (0) to down (1).
fn env_uv(d: vec3<f32>) -> vec2<f32> {
    let u = atan2(d.x, -d.z) / (2.0 * PI) + 0.5;
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}

// Reflection from the first captured probe whose box holds `p` (decision 0092): the mirror
// direction `r` is corrected for the box (it hits the box's wall, and the probe is looked up
// toward that point), and blurred to the roughness. a = weight, fading out just outside the box.
fn probe_reflection(p: vec3<f32>, r: vec3<f32>, roughness: f32) -> vec4<f32> {
    for (var i = 0u; i < 4u; i = i + 1u) {
        let c = lights.probe_pos[i];
        let e = lights.probe_ext[i].xyz;
        let d = p - c.xyz;
        // Full weight inside the box, fading to none a margin outside it (so a floor or wall
        // on the box's face is fully covered).
        let margin = 0.1 * min(min(e.x, e.y), e.z) + 0.05;
        if (c.w < 0.5 || any(abs(d) > e + vec3<f32>(margin))) {
            continue;
        }
        let inv = 1.0 / select(r, vec3<f32>(1e-6), abs(r) < vec3<f32>(1e-6));
        let far = max((e - d) * inv, (-e - d) * inv);
        let t = min(min(far.x, far.y), far.z);
        let dir = normalize(d + r * max(t, 0.0));
        let edge = min(min(e.x - abs(d.x), e.y - abs(d.y)), e.z - abs(d.z));
        let w = clamp(1.0 + edge / margin, 0.0, 1.0);
        let level = roughness * lights.env.y;
        let rgb = textureSampleLevel(probe_tex, env_samp, env_uv(dir), i32(i), level).rgb;
        return vec4<f32>(rgb, w);
    }
    return vec4<f32>(0.0);
}

// Irradiance / PI from the environment's nine spherical harmonics coefficients.
fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    let c = lights.sh;
    let r = c[0].rgb * 0.282095
        + c[1].rgb * 0.488603 * n.y + c[2].rgb * 0.488603 * n.z + c[3].rgb * 0.488603 * n.x
        + c[4].rgb * 1.092548 * n.x * n.y + c[5].rgb * 1.092548 * n.y * n.z
        + c[6].rgb * 0.315392 * (3.0 * n.z * n.z - 1.0)
        + c[7].rgb * 1.092548 * n.x * n.z + c[8].rgb * 0.546274 * (n.x * n.x - n.y * n.y);
    return max(r, vec3<f32>(0.0));
}

// Split-sum environment BRDF, analytic fit (Karis, "Physically based shading on mobile").
fn env_brdf(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
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

// Depth prepass (decision 0093): no color, only depth.
@fragment
fn fs_depth() {}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = in.base_color * textureSample(base_tex, samp, in.uv);
    let mr = textureSample(mr_tex, samp, in.uv);
    let emissive = textureSample(emissive_tex, samp, in.uv);
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
    // Geometric normal facing the viewer's side, for ray origins.
    var ng = normalize(in.normal);
    if (!front) {
        ng = -ng;
    }
    // Reflections of other objects, premultiplied (rgb) with their total weight (a); the
    // environment or ambient reflection fills the rest. A probe (decision 0092), then on top
    // a ray-traced reflection (decision 0093): mirror-sharp, fading out toward the roughness
    // limit.
    var refl = vec4<f32>(0.0);
    if ((flags & FLAG_PROBES) != 0u) {
        let pr = probe_reflection(in.world_pos, reflect(-v, n), roughness);
        refl = vec4<f32>(pr.rgb * pr.a, pr.a);
    }
    if ((flags & FLAG_RT_REFLECT) != 0u && roughness < lights.rt.x) {
        let traced = rt_reflect(in.world_pos, ng, reflect(-v, n), roughness);
        let a = traced.a * (1.0 - smoothstep(0.5 * lights.rt.x, lights.rt.x, roughness));
        refl = vec4<f32>(refl.rgb * (1.0 - a) + traced.rgb * a, refl.a * (1.0 - a) + a);
    }
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
            lit = lit * shadow_factor(in.world_pos, normalize(in.normal), l);
        }
        if ((flags & FLAG_RT_SHADOWS) != 0u) {
            lit = lit * rt_sun(in.world_pos, ng, l, in.clip.xy);
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
        if (atten > 0.0 && lights.local_shadow[j].x >= 0.0) {
            atten = atten * local_shadow_factor(j, in.world_pos, normalize(in.normal), l, sqrt(dist2));
        }
        if (atten > 0.0 && (flags & FLAG_RT_SHADOWS) != 0u) {
            // Mask 1: models holding a shadow-casting lamp do not block lamps (decision 0065).
            atten = atten * rt_visible(in.world_pos, ng, l, sqrt(dist2), 1u);
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
    if ((flags & FLAG_ENV) != 0u) {
        // Environment: diffuse from its irradiance, specular from the mip blurred to match
        // the roughness, weighted by the split-sum BRDF.
        let r = reflect(-v, n);
        let spec = textureSampleLevel(env_tex, env_samp, env_uv(r), roughness * lights.env.y).rgb;
        if (refl.a > 0.0) {
            color += lights.env.x * sh_irradiance(n) * base.rgb * (1.0 - metallic)
                + (lights.env.x * spec * (1.0 - refl.a) + refl.rgb) * env_brdf(f0, roughness, n_dot_v);
        } else {
            color += lights.env.x * (sh_irradiance(n) * base.rgb * (1.0 - metallic)
                + spec * env_brdf(f0, roughness, n_dot_v));
        }
    } else if (refl.a > 0.0) {
        color += ambient_light * base.rgb * (1.0 - metallic)
            + ambient_light * f0 * (1.0 - 0.5 * roughness) * (1.0 - refl.a)
            + refl.rgb * env_brdf(f0, roughness, n_dot_v);
    } else {
        color += ambient_light * (base.rgb * (1.0 - metallic) + f0 * (1.0 - 0.5 * roughness));
    }
    color += in.emissive.rgb + material.emissive.rgb * emissive.rgb;
    return finish(color, length(view.eye.xyz - in.world_pos), base.a);
}

// Tone mapping and fog, or (for bloom) fog only, in linear light: `fog.rgb` then holds the
// fog color mapped back through the tone curve, so fully fogged pixels end up the same.
fn finish(lit: vec3<f32>, distance: f32, alpha: f32) -> vec4<f32> {
    let flags = lights.count.z;
    var color = lit;
    if ((flags & FLAG_HDR) != 0u) {
        if ((flags & FLAG_FOG) != 0u) {
            let dist = distance * lights.fog.w;
            color = mix(lights.fog.rgb, color, exp(-dist * dist));
        }
        return vec4<f32>(max(color, vec3<f32>(0.0)), alpha);
    }
    if ((flags & FLAG_TONEMAP) != 0u) {
        color = tonemap(color * lights.post.x);
    }
    if ((flags & FLAG_FOG) != 0u) {
        let dist = distance * lights.fog.w;
        color = mix(lights.fog.rgb, color, exp(-dist * dist));
    }
    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), alpha);
}

// Sky background (decision 0066): a full-screen triangle at the far plane, drawn after
// models so only uncovered pixels run it. Fog thickens toward and below the horizon.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    var out: SkyOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let near = view.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = view.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let d = normalize(far.xyz / far.w - near.xyz / near.w);
    let color = textureSampleLevel(env_tex, env_samp, env_uv(d), 0.0).rgb;
    // Fog: as if the sky were 3 / density meters away at the horizon, farther upward.
    let up = max(d.y, 0.0);
    return finish(color, 3.0 / max(lights.fog.w, 1e-4) / (1.0 + 8.0 * up), 1.0);
}
