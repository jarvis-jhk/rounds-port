// Full-screen passes: applying the light map, the fog layer, the background
// grading and the final post pass (vignette, colour grading) into the output.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // One triangle covering the screen.
    let c = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(c * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
    out.uv = vec2<f32>(c.x, 1.0 - c.y);
    return out;
}

struct Params {
    // Composite: x = exposure, y = lighting enabled (0/1).
    // Post: x = vignette intensity, y = vignette inner radius,
    //       z = vignette outer radius, w = saturation.
    // Fog: premultiplied linear fog colour (rgb * a, a).
    a: vec4<f32>,
    // Post: x = contrast, y = brightness (multiplier), z/w unused.
    // Fog: linear scatter colour (rgb), soft/hard mix (w).
    b: vec4<f32>,
    // Post: tint (linear rgb). Fog: x = exposure.
    tint: vec4<f32>,
    // Unity grading (see grading.rs, GradingParams).
    g_flags: vec4<f32>,
    g_hsc: vec4<f32>,
    g_lift: vec4<f32>,
    g_inv_gamma: vec4<f32>,
    g_gain: vec4<f32>,
    g_filter: vec4<f32>,
    g_mix_r: vec4<f32>,
    g_mix_g: vec4<f32>,
    g_mix_b: vec4<f32>,
    v_settings: vec4<f32>,
    v_color: vec4<f32>,
    v_center: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var light_map: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var shadow_map: texture_2d<f32>;

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSample(src, samp, in.uv);
    let light = textureSample(light_map, samp, in.uv).rgb * params.a.x;
    let l = mix(vec3<f32>(1.0), light, params.a.y);
    return vec4<f32>(scene.rgb * l, scene.a);
}

// SF Soft Shadows fog layer (premultiplied blend over the background).
@fragment
fn fs_fog(in: VsOut) -> @location(0) vec4<f32> {
    let lm = textureSample(light_map, samp, in.uv).rgb;
    let sm = textureSample(shadow_map, samp, in.uv).rgb;
    let light = mix(lm, sm, params.b.w);
    let rgb = (params.a.rgb + params.b.rgb * light) * params.tint.x;
    return vec4<f32>(rgb, params.a.a);
}

// ---- Unity post-processing v2 HDR grading + ACES ---------------------------

fn mul3(r0: vec3<f32>, r1: vec3<f32>, r2: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(r0, v), dot(r1, v), dot(r2, v));
}

fn srgb_to_ap0(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(0.4397010, 0.3829780, 0.1773350),
                vec3<f32>(0.0897923, 0.8134230, 0.0967616),
                vec3<f32>(0.0175440, 0.1115440, 0.8707040), c);
}

fn ap0_to_ap1(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(1.4514393161, -0.2365107469, -0.2149285693),
                vec3<f32>(-0.0765537734, 1.1762296998, -0.0996759264),
                vec3<f32>(0.0083161484, -0.0060324498, 0.9977163014), c);
}

fn ap1_to_ap0(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(0.6954522414, 0.1406786965, 0.1638690622),
                vec3<f32>(0.0447945634, 0.8596711185, 0.0955343182),
                vec3<f32>(-0.0055258826, 0.0040252103, 1.0015006723), c);
}

fn ap1_to_xyz(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(0.6624541811, 0.1340042065, 0.1561876870),
                vec3<f32>(0.2722287168, 0.6740817658, 0.0536895174),
                vec3<f32>(-0.0055746495, 0.0040607335, 1.0103391003), c);
}

fn xyz_to_ap1(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(1.6410233797, -0.3248032942, -0.2364246952),
                vec3<f32>(-0.6636628587, 1.6153315917, 0.0167563477),
                vec3<f32>(0.0117218943, -0.0082844420, 0.9883948585), c);
}

fn d60_to_d65(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(0.987224, -0.00611327, 0.0159533),
                vec3<f32>(-0.00759836, 1.00186, 0.00533002),
                vec3<f32>(0.00307257, -0.00509595, 1.08168), c);
}

fn xyz_to_rec709(c: vec3<f32>) -> vec3<f32> {
    return mul3(vec3<f32>(3.2409699419, -1.5373831776, -0.4986107603),
                vec3<f32>(-0.9692436363, 1.8759675015, 0.0415550574),
                vec3<f32>(0.0556300797, -0.2039769589, 1.0569715142), c);
}

const HALF_MAX: f32 = 65504.0;
const AP1_RGB2Y: vec3<f32> = vec3<f32>(0.2722287168, 0.6740817658, 0.0536895174);

fn acescc_from(x: f32) -> f32 {
    let v = clamp(x, 0.0, HALF_MAX);
    if (v < 0.00003051757) {
        return (log2(0.00001525878 + v * 0.5) + 9.72) / 17.52;
    }
    return (log2(v) + 9.72) / 17.52;
}

fn acescc_to(x: f32) -> f32 {
    if (x < -0.3013698630) {
        return (exp2(x * 17.52 - 9.72) - 0.00001525878) * 2.0;
    }
    if (x < (log2(HALF_MAX) + 9.72) / 17.52) {
        return exp2(x * 17.52 - 9.72);
    }
    return HALF_MAX;
}

fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.bg, K.wz), vec4<f32>(c.gb, K.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(p.xyw, c.r), vec4<f32>(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-4;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126729, 0.7151522, 0.0721750));
}

fn rgb_2_saturation(rgb: vec3<f32>) -> f32 {
    let mi = min(min(rgb.r, rgb.g), rgb.b);
    let ma = max(max(rgb.r, rgb.g), rgb.b);
    return (max(ma, 1e-10) - max(mi, 1e-10)) / max(ma, 1e-2);
}

fn rgb_2_yc(rgb: vec3<f32>) -> f32 {
    let r = rgb.r;
    let g = rgb.g;
    let b = rgb.b;
    let chroma = sqrt(max(b * (b - g) + g * (g - r) + r * (r - b), 0.0));
    return (b + g + r + 1.75 * chroma) / 3.0;
}

fn sigmoid_shaper(x: f32) -> f32 {
    let t = max(1.0 - abs(x / 2.0), 0.0);
    let y = 1.0 + sign(x) * (1.0 - t * t);
    return y / 2.0;
}

fn glow_fwd(yc: f32, gain: f32, mid: f32) -> f32 {
    if (yc <= 2.0 / 3.0 * mid) {
        return gain;
    }
    if (yc >= 2.0 * mid) {
        return 0.0;
    }
    return gain * (mid / yc - 0.5);
}

fn rgb_2_hue(rgb: vec3<f32>) -> f32 {
    var hue = 0.0;
    if (!(rgb.r == rgb.g && rgb.g == rgb.b)) {
        hue = 57.29577951 * atan2(sqrt(3.0) * (rgb.g - rgb.b), 2.0 * rgb.r - rgb.g - rgb.b);
    }
    if (hue < 0.0) {
        hue = hue + 360.0;
    }
    return hue;
}

fn center_hue(hue: f32, center: f32) -> f32 {
    var h = hue - center;
    if (h < -180.0) {
        h = h + 360.0;
    } else if (h > 180.0) {
        h = h - 360.0;
    }
    return h;
}

fn dark_to_dim_surround(c: vec3<f32>) -> vec3<f32> {
    let xyz = ap1_to_xyz(c);
    let div = max(xyz.x + xyz.y + xyz.z, 1e-10);
    var xyy = vec3<f32>(xyz.x / div, xyz.y / div, xyz.y);
    xyy.z = pow(clamp(xyy.z, 0.0, HALF_MAX), 0.9811);
    let yy = max(xyy.y, 1e-10);
    let x2 = vec3<f32>(xyy.x * xyy.z / yy, xyy.z, (1.0 - xyy.x - xyy.y) * xyy.z / yy);
    return xyz_to_ap1(x2);
}

fn aces_tonemap(aces_in: vec3<f32>) -> vec3<f32> {
    var aces = aces_in;
    // Glow module.
    let saturation = rgb_2_saturation(aces);
    let yc = rgb_2_yc(aces);
    let s = sigmoid_shaper((saturation - 0.4) / 0.2);
    let added_glow = 1.0 + glow_fwd(yc, 0.05 * s, 0.08);
    aces = aces * added_glow;
    // Red modifier.
    let hue = rgb_2_hue(aces);
    let centered = center_hue(hue, 0.0);
    var w = smoothstep(0.0, 1.0, 1.0 - abs(2.0 * centered / 135.0));
    w = w * w;
    aces.r = aces.r + w * saturation * (0.03 - aces.r) * (1.0 - 0.82);
    // ACES to rendering space, global desaturation.
    var cg = max(vec3<f32>(0.0), ap0_to_ap1(aces));
    cg = mix(vec3<f32>(dot(cg, AP1_RGB2Y)), cg, 0.96);
    // Fitted RRT + ODT luminance curve.
    let x = cg;
    let post = (x * (278.5085 * x + 10.7772)) / (x * (293.6045 * x + 88.7122) + 80.6889);
    var lin = dark_to_dim_surround(post);
    lin = mix(vec3<f32>(dot(lin, AP1_RGB2Y)), lin, 0.93);
    let xyz = d60_to_d65(ap1_to_xyz(lin));
    return xyz_to_rec709(xyz);
}

// Grading of a linear HDR colour (post exposure included).
fn grade(c_in: vec3<f32>) -> vec3<f32> {
    // Post exposure, then the LUT's LogC input range.
    let c = clamp(c_in * params.g_flags.z, vec3<f32>(0.0), vec3<f32>(55.0));
    let aces = srgb_to_ap0(c);
    // Contrast in log space.
    var cc = vec3<f32>(acescc_from(aces.r), acescc_from(aces.g), acescc_from(aces.b));
    cc = (cc - vec3<f32>(0.4135884)) * params.g_hsc.z + vec3<f32>(0.4135884);
    let aces2 = vec3<f32>(acescc_to(cc.r), acescc_to(cc.g), acescc_to(cc.b));
    // Linear grading in ACEScg.
    var cg = ap0_to_ap1(aces2);
    cg = cg * params.g_filter.rgb;
    cg = vec3<f32>(dot(cg, params.g_mix_r.rgb), dot(cg, params.g_mix_g.rgb), dot(cg, params.g_mix_b.rgb));
    cg = cg * params.g_gain.rgb + params.g_lift.rgb;
    cg = sign(cg) * pow(abs(cg), params.g_inv_gamma.rgb);
    cg = max(vec3<f32>(0.0), cg);
    var hsv = rgb_to_hsv(cg);
    var h = hsv.x + params.g_hsc.x;
    if (h < 0.0) {
        h = h + 1.0;
    } else if (h > 1.0) {
        h = h - 1.0;
    }
    hsv.x = h;
    cg = hsv_to_rgb(hsv);
    let l = luminance(cg);
    cg = vec3<f32>(l) + params.g_hsc.y * (cg - vec3<f32>(l));
    let aces3 = ap1_to_ap0(cg);
    if (params.g_flags.y > 0.5) {
        return aces_tonemap(aces3);
    }
    // No tonemapper: back to linear sRGB (approximate inverse of srgb_to_ap0).
    return mul3(vec3<f32>(2.52169, -1.13413, -0.38756),
                vec3<f32>(-0.27648, 1.37272, -0.09624),
                vec3<f32>(-0.01538, -0.15298, 1.16835), aces3);
}

fn unity_vignette(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    var d = abs(uv - params.v_center.xy) * params.v_settings.x;
    d.x = d.x * mix(1.0, params.v_center.z, params.v_settings.w);
    d = pow(clamp(d, vec2<f32>(0.0), vec2<f32>(1.0)), vec2<f32>(params.v_settings.z));
    let f = pow(clamp(1.0 - dot(d, d), 0.0, 1.0), params.v_settings.y);
    return c * mix(params.v_color.rgb, vec3<f32>(1.0), f);
}

// Background stage: grade into the scene buffer (tonemapped, 0..1).
@fragment
fn fs_grade(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(src, samp, in.uv).rgb;
    if (params.g_flags.w > 0.5) {
        c = unity_vignette(c, in.uv);
    }
    if (params.g_flags.x > 0.5) {
        c = grade(c);
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// ---- bloom (post-processing v2 behaviour, fast-mode box filters) ------
// Params: a = (threshold - knee, 2 knee, 0.25 / knee, threshold),
// b = (upsample scale, clamp), tint = bloom colour.

fn box4(t: texture_2d<f32>, uv: vec2<f32>, scale: f32) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t));
    let d = texel.xyxy * vec4<f32>(-1.0, -1.0, 1.0, 1.0) * scale;
    let s = textureSample(t, samp, uv + d.xy).rgb + textureSample(t, samp, uv + d.zy).rgb
        + textureSample(t, samp, uv + d.xw).rgb + textureSample(t, samp, uv + d.zw).rgb;
    return s * 0.25;
}

@fragment
fn fs_bloom_prefilter(in: VsOut) -> @location(0) vec4<f32> {
    var c = min(box4(src, in.uv, 1.0), vec3<f32>(params.b.y));
    // Quadratic soft threshold on the brightest channel.
    let br = max(c.r, max(c.g, c.b));
    var rq = clamp(br - params.a.x, 0.0, params.a.y);
    rq = params.a.z * rq * rq;
    c = c * (max(rq, br - params.a.w) / max(br, 1e-4)) * params.tint.rgb;
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_bloom_down(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(box4(src, in.uv, 1.0), 1.0);
}

// 9-tap tent filter (1 2 1 / 2 4 2 / 1 2 1) / 16 at `scale` texels.
fn tent9(t: texture_2d<f32>, uv: vec2<f32>, scale: f32) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t));
    let d = texel.xyxy * vec4<f32>(1.0, 1.0, -1.0, 0.0) * scale;
    var s = textureSample(t, samp, uv - d.xy).rgb;
    s = s + textureSample(t, samp, uv - d.wy).rgb * 2.0;
    s = s + textureSample(t, samp, uv - d.zy).rgb;
    s = s + textureSample(t, samp, uv + d.zw).rgb * 2.0;
    s = s + textureSample(t, samp, uv).rgb * 4.0;
    s = s + textureSample(t, samp, uv + d.xw).rgb * 2.0;
    s = s + textureSample(t, samp, uv + d.zy).rgb;
    s = s + textureSample(t, samp, uv + d.wy).rgb * 2.0;
    s = s + textureSample(t, samp, uv + d.xy).rgb;
    return s * (1.0 / 16.0);
}

// src = the smaller level, light_map = this level's down texture. Tent
// upsampling (PPv2's high-quality path) keeps the halos round.
@fragment
fn fs_bloom_up(in: VsOut) -> @location(0) vec4<f32> {
    let low = tent9(src, in.uv, params.b.x);
    let high = textureSample(light_map, samp, in.uv).rgb;
    return vec4<f32>(low + high, 1.0);
}

@fragment
fn fs_post(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(src, samp, in.uv).rgb;
    // Bloom (binding 4 = the chain's result), strength in b.z.
    if (params.b.z > 0.0) {
        c = c + textureSample(shadow_map, samp, in.uv).rgb * params.b.z;
    }
    if (params.g_flags.w > 0.5) {
        c = unity_vignette(c, in.uv);
    }
    if (params.g_flags.x > 0.5) {
        c = grade(c);
    }
    // Simple grading hooks (identity by default).
    c = c * params.b.y * params.tint.rgb;
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = mix(vec3<f32>(luma), c, params.a.w);
    c = (c - vec3<f32>(0.18)) * params.b.x + vec3<f32>(0.18);
    // Simple vignette.
    let d = length((in.uv - vec2<f32>(0.5)) * vec2<f32>(1.0, 1.0)) * 1.41421356;
    let v = smoothstep(params.a.z, params.a.y, d);
    c = c * mix(1.0, v, params.a.x);
    return vec4<f32>(max(c, vec3<f32>(0.0)), 1.0);
}
