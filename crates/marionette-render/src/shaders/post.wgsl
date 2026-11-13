// Bloom (decision 0064). Models are drawn into a floating-point target; this file blurs the
// light above a threshold through a chain of half-size images (down), adds the chain back up
// (up), then the composite adds the glow to the image and tone maps it into the final target.

struct PostU {
    // threshold, intensity / levels, exposure, tone map (1) or clamp (0)
    params: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> post: PostU;
@group(0) @binding(3) var glow: texture_2d<f32>;

struct FullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FullOut {
    var out: FullOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Four bilinear taps around the destination texel: a 4x4 box of the source.
fn box4(uv: vec2<f32>, karis: bool) -> vec3<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let o = vec2<f32>(f32(k & 1) * 2.0 - 1.0, f32(k >> 1u) * 2.0 - 1.0) * t;
        let c = textureSampleLevel(src, samp, uv + o, 0.0).rgb;
        // Weighting by 1 / (1 + brightness) on the first step keeps single very bright
        // pixels from flickering as they move.
        let w = select(1.0, 1.0 / (1.0 + luma(c)), karis);
        sum += c * w;
        weight += w;
    }
    return sum / weight;
}

@fragment
fn fs_prefilter(in: FullOut) -> @location(0) vec4<f32> {
    let c = box4(in.uv, true);
    // Soft threshold: a quadratic knee of half the threshold eases the glow in.
    let threshold = post.params.x;
    let knee = max(threshold * 0.5, 1e-4);
    let br = max(c.r, max(c.g, c.b));
    var soft = clamp(br - threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    let contribution = max(soft, br - threshold) / max(br, 1e-4);
    return vec4<f32>(c * contribution, 1.0);
}

@fragment
fn fs_down(in: FullOut) -> @location(0) vec4<f32> {
    return vec4<f32>(box4(in.uv, false), 1.0);
}

// 3x3 tent of the smaller level, added (blend one + one) onto the larger one.
@fragment
fn fs_up(in: FullOut) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    var sum = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let w = f32((2 - abs(x)) * (2 - abs(y)));
            sum += textureSampleLevel(src, samp, in.uv + vec2<f32>(f32(x), f32(y)) * t, 0.0).rgb * w;
        }
    }
    return vec4<f32>(sum / 16.0, 1.0);
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_composite(in: FullOut) -> @location(0) vec4<f32> {
    let hdr = textureLoad(src, vec2<i32>(in.clip.xy), 0).rgb;
    let g = textureSampleLevel(glow, samp, in.uv, 0.0).rgb;
    var c = hdr + g * post.params.y;
    if (post.params.w > 0.5) {
        c = tonemap(c * post.params.z);
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
