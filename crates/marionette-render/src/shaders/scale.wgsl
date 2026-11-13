// Render scale, upscaling and foveated rendering (decisions 0081, 0082).
//
// The world is drawn into an internal image (`src`) whose size is the output size times the
// render scale. With foveation, a mask pass first writes the nearest depth into every other
// 2x2 pixel quad outside an inner ellipse, so nothing is shaded there; `fetch` fills those
// pixels back in from the four neighbouring quads. The resolve pass then maps the internal
// image to the output: a straight copy at scale 1, Catmull-Rom with ringing clamped to the
// nearest texels (and optional sharpening) below 1, a box filter above 1.

struct ScaleU {
    // inner foveation radius (<= 0: off), sharpening 0..1, mode (0 copy, 1 up, 1 + ratio
    // for down with ratio 1..2), unused
    params: vec4<f32>,
    // internal image width and height (the mask pass cannot read the image it draws into)
    dims: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: ScaleU;
@group(0) @binding(2) var samp: sampler;

struct FullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FullOut {
    var out: FullOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    // Depth 0 is the near plane: the mask pass writes it so later fragments fail the test.
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

// Pixel `p` of an image of `size` is skipped by foveation.
fn masked(p: vec2<i32>, size: vec2<i32>) -> bool {
    let inner = u.params.x;
    if (inner <= 0.0) {
        return false;
    }
    let half = vec2<f32>(size) * 0.5;
    let d = (vec2<f32>(p) + 0.5 - half) / half;
    if (dot(d, d) <= inner * inner) {
        return false;
    }
    let q = p >> vec2<u32>(1u);
    return ((q.x + q.y) & 1) == 1;
}

@fragment
fn fs_mask(in: FullOut) {
    if (!masked(vec2<i32>(in.clip.xy), vec2<i32>(u.dims.xy))) {
        discard;
    }
}

// One internal texel; a foveation hole is filled from the nearest pixels of the four
// neighbouring quads, weighted by inverse distance.
fn fetch(p_in: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(u.dims.xy);
    let p = clamp(p_in, vec2<i32>(0), size - 1);
    if (!masked(p, size)) {
        return textureLoad(src, p, 0).rgb;
    }
    let q = (p >> vec2<u32>(1u)) * 2;
    let l = p - q;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    let taps = array<vec3<i32>, 4>(
        vec3<i32>(q.x - 1, p.y, l.x + 1),
        vec3<i32>(q.x + 2, p.y, 2 - l.x),
        vec3<i32>(p.x, q.y - 1, l.y + 1),
        vec3<i32>(p.x, q.y + 2, 2 - l.y),
    );
    for (var k = 0; k < 4; k = k + 1) {
        let t = taps[k];
        if (t.x >= 0 && t.y >= 0 && t.x < size.x && t.y < size.y) {
            let w = 1.0 / f32(t.z);
            sum += textureLoad(src, t.xy, 0).rgb * w;
            weight += w;
        }
    }
    return sum / max(weight, 1e-4);
}

fn cubic(x: f32) -> vec4<f32> {
    // Catmull-Rom weights for the four texels around a sample at fraction x.
    let x2 = x * x;
    let x3 = x2 * x;
    return vec4<f32>(
        -0.5 * x3 + x2 - 0.5 * x,
        1.5 * x3 - 2.5 * x2 + 1.0,
        -1.5 * x3 + 2.0 * x2 + 0.5 * x,
        0.5 * x3 - 0.5 * x2,
    );
}

fn upscale(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(src));
    let pos = uv * size - 0.5;
    let base = vec2<i32>(floor(pos));
    let f = pos - floor(pos);
    let wx = cubic(f.x);
    let wy = cubic(f.y);
    var sum = vec3<f32>(0.0);
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    var bilinear = vec3<f32>(0.0);
    for (var y = 0; y < 4; y = y + 1) {
        var row = vec3<f32>(0.0);
        for (var x = 0; x < 4; x = x + 1) {
            let c = fetch(base + vec2<i32>(x - 1, y - 1));
            row += c * wx[x];
            if (x >= 1 && x <= 2 && y >= 1 && y <= 2) {
                // The 2x2 texels around the sample bound the result (no ringing halos) and
                // give the plain bilinear value the sharpening pushes away from.
                lo = min(lo, c);
                hi = max(hi, c);
                let bw = select(1.0 - f.x, f.x, x == 2) * select(1.0 - f.y, f.y, y == 2);
                bilinear += c * bw;
            }
        }
        sum += row * wy[y];
    }
    let sharp = sum + (sum - bilinear) * u.params.y;
    return clamp(sharp, lo, hi);
}

fn downscale(uv: vec2<f32>) -> vec3<f32> {
    // Output pixel footprint in internal texels; a grid of bilinear taps covers it.
    let src_size = vec2<f32>(textureDimensions(src));
    let step = 1.0 / src_size;
    let ratio = clamp(u.params.z - 1.0, 1.0, 2.0);
    var sum = vec3<f32>(0.0);
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            let o = (vec2<f32>(f32(x), f32(y)) - 0.5) * ratio * 0.5 * step;
            sum += textureSampleLevel(src, samp, uv + o, 0.0).rgb;
        }
    }
    return sum * 0.25;
}

@fragment
fn fs_resolve(in: FullOut) -> @location(0) vec4<f32> {
    let mode = u.params.z;
    var c: vec3<f32>;
    if (mode < 0.5) {
        c = fetch(vec2<i32>(in.clip.xy));
    } else if (mode < 1.5) {
        c = upscale(in.uv);
    } else {
        c = downscale(in.uv);
    }
    return vec4<f32>(c, 1.0);
}
