// Ray tracing (decision 0093). Replaces the stubs between RT:BEGIN and RT:END in pbr.wgsl in
// the ray tracing variant only (raytrace.rs adds `enable wgpu_ray_query;`). Group 1 gains the
// scene's top-level structure, per-triangle data, the instance buffer and each instance's
// first triangle.

@group(1) @binding(6) var tlas: acceleration_structure;

struct RtTri {
    n: vec3<f32>,        // face normal, mesh space
    color: u32,          // base color texture at the center, sRGB RGBA8
    emissive: vec3<f32>, // emissive factor times texture (linear)
    pad: u32,
};
@group(1) @binding(7) var<storage, read> rt_tris: array<RtTri>;

struct RtInst {
    model0: vec4<f32>,
    model1: vec4<f32>,
    model2: vec4<f32>,
    model3: vec4<f32>,
    base_color: vec4<f32>,
    params: vec4<f32>, // metallic, roughness, normal scale, has normal map
    emissive: vec4<f32>,
};
@group(1) @binding(8) var<storage, read> rt_insts: array<RtInst>;
// First triangle of each instance's part; bit 31 set when its material is overridden.
@group(1) @binding(9) var<storage, read> rt_tri_base: array<u32>;

const RT_FAR: f32 = 10000.0;

// Ray origin pushed off the surface along its geometric normal, by more far from the eye
// (where depth and position precision are coarser).
fn rt_origin(p: vec3<f32>, ng: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let side = select(-1.0, 1.0, dot(ng, l) >= 0.0);
    let d = length(view.eye.xyz - p);
    return p + ng * side * (0.002 + 0.0005 * d);
}

// True when anything (instances matching `mask`) lies on the ray before `tmax`.
fn rt_blocked(origin: vec3<f32>, dir: vec3<f32>, tmax: f32, mask: u32) -> bool {
    var rq: ray_query;
    rayQueryInitialize(&rq, tlas, RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT | RAY_FLAG_FORCE_OPAQUE, mask, 0.0, tmax, origin, dir));
    rayQueryProceed(&rq);
    let hit = rayQueryGetCommittedIntersection(&rq);
    return hit.kind != RAY_QUERY_INTERSECTION_NONE;
}

// Interleaved gradient noise (Jimenez): a per-pixel rotation for soft-shadow rays.
fn rt_noise(px: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(px, vec2<f32>(0.06711056, 0.00583715))));
}

fn rt_sun(p: vec3<f32>, ng: vec3<f32>, l: vec3<f32>, px: vec2<f32>) -> f32 {
    let origin = rt_origin(p, ng, l);
    let radius = lights.rt.y;
    if (radius <= 0.0) {
        return select(1.0, 0.0, rt_blocked(origin, l, RT_FAR, 0xFFu));
    }
    // Soft: four rays spread over the sun's disc, rotated per pixel.
    let t = normalize(cross(l, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(l.y) > 0.9)));
    let b = cross(l, t);
    let spin = rt_noise(px) * 2.0 * PI;
    var lit = 0.0;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let a = spin + f32(k) * 0.5 * PI;
        let r = radius * sqrt((f32(k) + 0.5) / 4.0);
        let dir = normalize(l + (t * cos(a) + b * sin(a)) * r);
        lit += select(0.25, 0.0, rt_blocked(origin, dir, RT_FAR, 0xFFu));
    }
    return lit;
}

fn rt_visible(p: vec3<f32>, ng: vec3<f32>, l: vec3<f32>, dist: f32, mask: u32) -> f32 {
    let origin = rt_origin(p, ng, l);
    let tmax = max(dist - distance(origin, p) - 0.01, 0.0);
    return select(1.0, 0.0, rt_blocked(origin, l, tmax, mask));
}

fn rt_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// What a ray that hits nothing sees: the environment (blurred to the roughness) or ambient.
fn rt_sky(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    let flags = lights.count.z;
    if ((flags & FLAG_ENV) != 0u) {
        return lights.env.x * textureSampleLevel(env_tex, env_samp, env_uv(dir), roughness * lights.env.y).rgb;
    }
    if ((flags & FLAG_SKY) != 0u) {
        return mix(lights.ground.rgb, lights.ambient.rgb, dir.y * 0.5 + 0.5);
    }
    return lights.ambient.rgb;
}

fn rt_reflect(p: vec3<f32>, ng: vec3<f32>, r: vec3<f32>, roughness: f32) -> vec4<f32> {
    let origin = rt_origin(p, ng, r);
    var rq: ray_query;
    rayQueryInitialize(&rq, tlas, RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xFFu, 0.0, RT_FAR, origin, r));
    rayQueryProceed(&rq);
    let hit = rayQueryGetCommittedIntersection(&rq);
    if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
        return vec4<f32>(rt_sky(r, roughness), 1.0);
    }
    let inst = rt_insts[hit.instance_custom_data];
    let base_word = rt_tri_base[hit.instance_custom_data];
    let tri = rt_tris[(base_word & 0x7FFFFFFFu) + hit.primitive_index];
    // World normal through the cofactor of the instance matrix (as in vs_main), facing the ray.
    let c0 = inst.model0.xyz;
    let c1 = inst.model1.xyz;
    let c2 = inst.model2.xyz;
    let k0 = cross(c1, c2);
    let flip = select(1.0, -1.0, dot(c0, k0) < 0.0);
    var n = normalize(mat3x3<f32>(k0, cross(c2, c0), cross(c0, c1)) * tri.n * flip);
    if (dot(n, r) > 0.0) {
        n = -n;
    }
    var albedo = inst.base_color.rgb;
    var emissive = inst.emissive.rgb;
    if ((base_word & 0x80000000u) == 0u) {
        albedo = albedo * rt_srgb(unpack4x8unorm(tri.color).rgb);
        emissive = emissive + tri.emissive;
    }
    let metallic = clamp(inst.params.x, 0.0, 1.0);
    let diffuse = albedo * (1.0 - metallic);
    let hp = origin + r * hit.t;
    let flags = lights.count.z;
    var color = emissive;
    for (var i = 0u; i < lights.count.x; i = i + 1u) {
        let l = -normalize(lights.direction[i].xyz);
        let n_dot_l = dot(n, l);
        if (n_dot_l <= 0.0) {
            continue;
        }
        var vis = 1.0;
        if ((flags & FLAG_RT_SHADOWS) != 0u && rt_blocked(rt_origin(hp, n, l), l, RT_FAR, 0xFFu)) {
            vis = 0.0;
        }
        color += diffuse * lights.radiance[i].rgb * n_dot_l * vis;
    }
    for (var j = 0u; j < min(lights.count.y, MAX_LOCAL); j = j + 1u) {
        let to_light = lights.local_pos[j].xyz - hp;
        let dist2 = max(dot(to_light, to_light), 1e-4);
        let l = to_light * inverseSqrt(dist2);
        let range = lights.local_pos[j].w;
        let ratio = dist2 / (range * range);
        let window = clamp(1.0 - ratio * ratio, 0.0, 1.0);
        var atten = window * window / dist2;
        let cos_outer = lights.local_rad[j].w;
        if (cos_outer >= -1.0) {
            atten = atten * smoothstep(cos_outer, lights.local_dir[j].w, dot(-l, lights.local_dir[j].xyz));
        }
        color += diffuse * lights.local_rad[j].rgb * atten * max(dot(n, l), 0.0);
    }
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    if ((flags & FLAG_ENV) != 0u) {
        color += lights.env.x * (sh_irradiance(n) * diffuse
            + textureSampleLevel(env_tex, env_samp, env_uv(reflect(r, n)), lights.env.y * 0.6).rgb * f0);
    } else {
        var ambient = lights.ambient.rgb;
        if ((flags & FLAG_SKY) != 0u) {
            ambient = mix(lights.ground.rgb, lights.ambient.rgb, n.y * 0.5 + 0.5);
        }
        color += ambient * (diffuse + f0 * 0.5);
    }
    return vec4<f32>(color, 1.0);
}
