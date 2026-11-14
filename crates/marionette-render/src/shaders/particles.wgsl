// Particles (decision 0282): camera-facing quads computed entirely from game time. Nothing
// is simulated or stored: particle `first + i` of an emitter started `i / rate` seconds before
// the newest one, and its random start (point, direction) comes from hashing its number, so
// any tick draws the same picture on any GPU and the CPU never touches a particle.

struct ViewU {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
};

struct EmitterU {
    model: mat4x4<f32>,
    start_color: vec4<f32>,
    end_color: vec4<f32>,
    // rate (per second), lifetime (s), speed (m/s), cos(spread)
    a: vec4<f32>,
    // gravity (m/s^2), start size, end size, start radius (m)
    b: vec4<f32>,
    // age of particle `first` (s), exposure (0: no tone mapping)
    c: vec4<f32>,
    // seed, first particle number
    d: vec4<u32>,
};

@group(0) @binding(0) var<uniform> view: ViewU;
@group(1) @binding(0) var<uniform> e: EmitterU;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn rand(h: ptr<function, u32>) -> f32 {
    *h = pcg(*h);
    return f32(*h >> 8u) / 16777216.0;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    var out: VsOut;
    let life = e.a.y;
    let age = e.c.x - f32(ii) / e.a.x;
    if (age < 0.0 || age >= life) {
        // Not born yet or already gone: outside the clip volume.
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return out;
    }
    var h = pcg((e.d.y + ii) ^ pcg(e.d.x));
    // Direction within the cone around +Y, uniform over the cap.
    let cos_t = mix(1.0, e.a.w, rand(&h));
    let sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
    let phi = 6.2831853 * rand(&h);
    let dir = vec3<f32>(sin_t * cos(phi), cos_t, sin_t * sin(phi));
    // Start point uniform in the ball.
    let z = 2.0 * rand(&h) - 1.0;
    let phi2 = 6.2831853 * rand(&h);
    let rz = sqrt(max(0.0, 1.0 - z * z));
    let start = vec3<f32>(rz * cos(phi2), z, rz * sin(phi2)) * e.b.w * pow(rand(&h), 1.0 / 3.0);
    let local = start + dir * (e.a.z * age);
    var world = (e.model * vec4<f32>(local, 1.0)).xyz;
    world.y = world.y - 0.5 * e.b.x * age * age;

    let k = age / life;
    let size = mix(e.b.y, e.b.z, k);
    let to_eye = normalize(view.eye.xyz - world);
    var right = cross(vec3<f32>(0.0, 1.0, 0.0), to_eye);
    if (dot(right, right) < 1e-8) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    }
    right = normalize(right);
    let up = cross(to_eye, right);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[vi];
    let p = world + (right * c.x + up * c.y) * (0.5 * size);
    out.clip = view.view_proj * vec4<f32>(p, 1.0);
    out.uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    out.color = mix(e.start_color, e.end_color, k);
    return out;
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Premultiplied: additive pipelines add rgb, alpha pipelines also cover by alpha.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv) * in.color;
    var rgb = t.rgb * t.a;
    if (e.c.y > 0.0) {
        rgb = tonemap(rgb * e.c.y);
    }
    return vec4<f32>(rgb, t.a);
}

// Into the floating-point target of bloom: linear light, tone mapped later.
@fragment
fn fs_hdr(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv) * in.color;
    return vec4<f32>(max(t.rgb, vec3<f32>(0.0)) * t.a, t.a);
}
