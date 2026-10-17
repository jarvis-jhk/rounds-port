// Adds one light's contribution to a light map, attenuated by its shadow
// mask (same resolution as the light map, so it is read with textureLoad).
// Radial lights fall off quadratically to `range`; cookie lights cover a
// rectangle and take their shape from a texture (like SFLight).

struct Globals {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var mask: texture_2d<f32>;
@group(2) @binding(0) var cookie: texture_2d<f32>;
@group(2) @binding(1) var cookie_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) flags: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) pos_range: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) cookie_info: vec4<f32>,
) -> VsOut {
    let c = vec2<f32>(f32(vi & 1u), f32(vi >> 1u)) * 2.0 - vec2<f32>(1.0);
    var half = vec2<f32>(pos_range.z);
    if (cookie_info.z > 0.5) {
        half = cookie_info.xy;
    }
    let world = pos_range.xy + c * half;
    var out: VsOut;
    out.pos = globals.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.local = c;
    out.color = color;
    out.flags = vec2<f32>(pos_range.w, cookie_info.z);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Cookie uv: local (-1..1, y up) -> texture (0..1, v down).
    let uv = vec2<f32>(in.local.x * 0.5 + 0.5, 0.5 - in.local.y * 0.5);
    let ck = textureSample(cookie, cookie_samp, uv).rgb;
    var atten = vec3<f32>(1.0);
    if (in.flags.y > 0.5) {
        atten = ck;
    } else {
        let falloff = clamp(1.0 - length(in.local), 0.0, 1.0);
        atten = vec3<f32>(falloff * falloff);
    }
    var lit = 1.0;
    if (in.flags.x > 0.5) {
        let occ = textureLoad(mask, vec2<i32>(in.pos.xy), 0).r;
        lit = 1.0 - clamp(occ, 0.0, 1.0);
    }
    return vec4<f32>(in.color.rgb * atten * lit, 0.0);
}
