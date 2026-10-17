// Instanced quads: sprites, glyphs and analytic shapes in one pipeline.
// Colours are linear, premultiplied alpha.

struct Globals {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

const KIND_TEXTURED: u32 = 0u;
const KIND_ELLIPSE: u32 = 1u;
const KIND_RING: u32 = 2u;
const KIND_ROUNDED_RECT: u32 = 3u;
const KIND_SDF: u32 = 4u;
const TAU: f32 = 6.28318530718;

struct Instance {
    // Columns of the 2x2 matrix mapping the unit quad to world (size included).
    @location(0) axes: vec4<f32>,
    // World position of quad corner (0,0); local size of the quad.
    @location(1) origin_size: vec4<f32>,
    // u0, v0, u1, v1: texture coords at local (0,0) and (1,1).
    @location(2) uv: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) kind: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Position inside the quad in local units, centred on the quad.
    @location(1) local: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) params: vec4<f32>,
    @location(4) @interpolate(flat) size: vec2<f32>,
    @location(5) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    let c = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    let world = inst.origin_size.xy + inst.axes.xy * c.x + inst.axes.zw * c.y;
    var out: VsOut;
    out.pos = globals.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    out.local = (c - vec2<f32>(0.5)) * inst.origin_size.zw;
    out.color = inst.color;
    out.params = inst.params;
    out.size = inst.origin_size.zw;
    out.kind = inst.kind;
    return out;
}

fn coverage(d: f32, aa: f32) -> f32 {
    return clamp(0.5 - d / aa, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Derivatives and texture samples must stay in uniform control flow.
    // Textures hold straight alpha; premultiply after filtering.
    let ts = textureSample(tex, samp, in.uv);
    let t = vec4<f32>(ts.rgb * ts.a, ts.a);
    let aa = max(length(dpdx(in.local)) + length(dpdy(in.local)), 1e-6) * 0.5;
    let half = in.size * 0.5;
    let sdf_w = max(fwidth(t.a), 1e-4);
    let duv_dx = dpdx(in.uv);
    let duv_dy = dpdy(in.uv);

    var alpha = 1.0;
    switch in.kind {
        case KIND_ELLIPSE: {
            let r = min(half.x, half.y);
            let d = (length(in.local / max(half, vec2<f32>(1e-6))) - 1.0) * r;
            alpha = coverage(d, aa);
        }
        case KIND_RING: {
            let r = length(in.local);
            let outer = half.x;
            let inner = outer - in.params.x;
            var d = max(r - outer, inner - r);
            let sweep = in.params.z;
            if (sweep < TAU - 1e-4) {
                let mid = in.params.y + sweep * 0.5;
                let a = atan2(in.local.y, in.local.x) - mid;
                let dev = abs(a - TAU * round(a / TAU));
                d = max(d, (dev - sweep * 0.5) * r);
            }
            alpha = coverage(d, aa);
        }
        case KIND_ROUNDED_RECT: {
            let rad = min(in.params.x, min(half.x, half.y));
            let q = abs(in.local) - half + vec2<f32>(rad);
            let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - rad;
            alpha = coverage(d, aa);
        }
        case KIND_SDF: {
            // Signed distance in alpha, 0.5 at the edge (TextMeshPro style);
            // params.x shifts the edge (dilate/erode), params.y is the
            // distance-value change per atlas texel (1 / (2 (padding + 1))).
            let edge = 0.5 - in.params.x;
            if (in.params.y <= 0.0) {
                return in.color * smoothstep(edge - sdf_w, edge + sdf_w, t.a);
            }
            // Value change per screen pixel from the UV derivatives (stable
            // inside thin strokes, unlike fwidth of the sampled value), and
            // four sub-pixel taps so strokes thinner than a pixel at small
            // sizes keep their coverage instead of aliasing away.
            let dims = vec2<f32>(textureDimensions(tex));
            let texels = 0.5 * (length(duv_dx * dims) + length(duv_dy * dims));
            let w = max(texels * in.params.y, 1e-4);
            var cov = 0.0;
            for (var i = 0u; i < 4u; i = i + 1u) {
                let o = vec2<f32>(f32(i & 1u), f32(i >> 1u)) * 0.5 - vec2<f32>(0.25);
                let uv = in.uv + duv_dx * o.x + duv_dy * o.y;
                let v = textureSampleLevel(tex, samp, uv, 0.0).a;
                cov = cov + clamp((v - edge) / w + 0.5, 0.0, 1.0);
            }
            return in.color * (cov * 0.25);
        }
        default: {}
    }
    if (in.kind == KIND_TEXTURED) {
        // params.z: stencil mask writer (drop transparent texels);
        // params.w: additive (premultiplied colour with zero alpha).
        var o = t * in.color;
        if (in.params.z > 0.5 && o.a < 0.01) {
            discard;
        }
        // params.y: value-mask writer, drop by texture alpha only.
        if (in.params.y > 0.5 && t.a < 0.01) {
            discard;
        }
        if (in.params.w > 0.5) {
            o.a = 0.0;
        }
        return o;
    }
    return t * in.color * alpha;
}
