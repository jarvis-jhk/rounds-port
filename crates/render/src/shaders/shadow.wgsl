// Soft shadow mask: one instance per (light, light-facing caster edge).
// Writes the fraction of the light's disk hidden by the edge; the mask is
// cleared to 0 and blended additively, so occlusion of adjacent edges adds up.

struct Globals {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) @interpolate(flat) edge: vec4<f32>,
    @location(2) @interpolate(flat) light: vec4<f32>,
};

fn perp(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-v.y, v.x);
}

// Endpoint pushed away from the light along the outer penumbra ray: the
// ray from the light-disk point on the `toward` side of the edge.
fn extrude(p: vec2<f32>, toward: vec2<f32>, l: vec2<f32>, r: f32, len: f32) -> vec2<f32> {
    let d = p - l;
    let dl = max(length(d), 1e-4);
    var n = perp(d / dl);
    if (dot(n, toward) < 0.0) {
        n = -n;
    }
    let src = l + n * min(r, dl * 0.99);
    return p + normalize(p - src) * len;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) edge: vec4<f32>, @location(1) light: vec4<f32>) -> VsOut {
    let a = edge.xy;
    let b = edge.zw;
    let l = light.xy;
    let r = light.z;
    let len = light.w;
    let a2 = extrude(a, b - a, l, r, len);
    let b2 = extrude(b, a - b, l, r, len);
    // Two triangles: (a, b, b2), (a, b2, a2).
    var p: vec2<f32>;
    switch vi {
        case 0u: { p = a; }
        case 1u: { p = b; }
        case 2u: { p = b2; }
        case 3u: { p = a; }
        case 4u: { p = b2; }
        default: { p = a2; }
    }
    var out: VsOut;
    out.pos = globals.view_proj * vec4<f32>(p, 0.0, 1.0);
    out.world = p;
    out.edge = edge;
    out.light = light;
    return out;
}

// Offset (across the view direction, at the light's distance) at which the
// ray from `p` through `e` meets the light plane. `other` is the far
// endpoint, used when `e` lies behind `p`.
fn project(p: vec2<f32>, e: vec2<f32>, other: vec2<f32>, d: vec2<f32>, n: vec2<f32>, dist: f32) -> f32 {
    let v = e - p;
    let along = dot(v, d);
    if (along > 1e-4) {
        return dot(v, n) * dist / along;
    }
    // Behind the pixel: the segment runs off to infinity on the side where
    // it crosses the pixel's plane.
    let w = other - p;
    let wa = dot(w, d);
    let t = (wa - 1e-4) / max(wa - along, 1e-6);
    let c = other + (e - other) * t;
    return sign(dot(c - p, n)) * 1e6;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let a = in.edge.xy;
    let b = in.edge.zw;
    let l = in.light.xy;
    let r = in.light.z;
    let p = in.world;
    // The pixel must be on the inner side of the edge (the light is on the
    // outer side by construction).
    let e = b - a;
    let side = e.x * (p.y - a.y) - e.y * (p.x - a.x);
    if (side <= 0.0) {
        discard;
    }
    let to_l = l - p;
    let dist = length(to_l);
    if (dist < 1e-4) {
        discard;
    }
    let d = to_l / dist;
    let n = perp(d);
    let ta = project(p, a, b, d, n, dist);
    let tb = project(p, b, a, d, n, dist);
    let lo = clamp(min(ta, tb), -r, r);
    let hi = clamp(max(ta, tb), -r, r);
    let occ = (hi - lo) / (2.0 * r);
    return vec4<f32>(occ, 0.0, 0.0, 0.0);
}
