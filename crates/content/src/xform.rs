//! Unity transforms and 2D colliders -> sim geometry.
//!
//! The import already stores each object's composed 2D world matrix
//! (`world.matrix`, exact for skewed hierarchies). [`compose_world`] rebuilds
//! the same matrices from the local TRS chain (full 3D composition projected
//! to the XY plane); `content-check` uses it to cross-check the import, and
//! it is what the unit tests exercise. Colliders become the sim's convex
//! [`Shape`]s: boxes stay oriented boxes unless the transform shears them
//! (then a polygon), concave polygons are decomposed into convex pieces,
//! capsules become a box plus two circles, edge chains become thin boxes.
//!
//! Everything here is plain f32 arithmetic plus `rounds_sim::math` (libm),
//! so the result is identical on every platform (content must agree between
//! peers).

use rounds_import::format::{Collider, Object, Shape as UShape, Trs};
use rounds_sim::content::Affine;
use rounds_sim::geom::Shape;
use rounds_sim::math::{V2, v2};

/// 2D affine from the import's `[a, b, c, d, tx, ty]`
/// (`x' = a x + c y + tx`, `y' = b x + d y + ty`).
pub fn affine(m: [f32; 6]) -> Affine {
    Affine {
        ax: v2(m[0], m[1]),
        ay: v2(m[2], m[3]),
        t: v2(m[4], m[5]),
    }
}

/// A 3x4 affine matrix (3x3 linear part, row-major, plus translation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat34 {
    pub m: [[f32; 4]; 3],
}

impl Mat34 {
    pub const IDENTITY: Mat34 = Mat34 {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };

    /// `T(position) * R(rotation) * S(scale)` of a Unity local transform.
    pub fn from_trs(t: &Trs) -> Mat34 {
        let [x, y, z, w] = t.rotation;
        let r = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ];
        let s = t.scale;
        let p = t.position;
        let mut m = [[0.0; 4]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for j in 0..3 {
                row[j] = r[i][j] * s[j];
            }
            row[3] = p[i];
        }
        Mat34 { m }
    }

    /// `self * o`.
    pub fn mul(&self, o: &Mat34) -> Mat34 {
        let a = &self.m;
        let b = &o.m;
        let mut m = [[0.0; 4]; 3];
        for i in 0..3 {
            for j in 0..4 {
                let mut s = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
                if j == 3 {
                    s += a[i][3];
                }
                m[i][j] = s;
            }
        }
        Mat34 { m }
    }

    /// Projection to the XY plane: how local (x, y, 0) maps to world (x, y).
    pub fn to_affine(&self) -> Affine {
        let m = &self.m;
        Affine {
            ax: v2(m[0][0], m[1][0]),
            ay: v2(m[0][1], m[1][1]),
            t: v2(m[0][3], m[1][3]),
        }
    }
}

/// World matrices of a flat object list (parents before children), from the
/// local transforms. For prefabs this is relative to the root's parent.
pub fn compose_world(objects: &[Object]) -> Vec<Mat34> {
    let mut out: Vec<Mat34> = Vec::with_capacity(objects.len());
    for o in objects {
        let local = Mat34::from_trs(&o.local);
        let w = match o.parent {
            Some(p) if p < out.len() => out[p].mul(&local),
            _ => local,
        };
        out.push(w);
    }
    out
}

/// Largest absolute difference between two affines (for cross-checks).
pub fn affine_diff(a: &Affine, b: &Affine) -> f32 {
    [
        a.ax.x - b.ax.x,
        a.ax.y - b.ax.y,
        a.ay.x - b.ay.x,
        a.ay.y - b.ay.y,
        a.t.x - b.t.x,
        a.t.y - b.t.y,
    ]
    .iter()
    .fold(0.0f32, |m, d| m.max(d.abs()))
}

/// Pose of a rigid body (position, rotation) without scale.
pub fn pose(pos: V2, angle: f32) -> Affine {
    Affine::trs(pos, angle, v2(1.0, 1.0))
}

/// Rotation angle (radians, CCW) of a Unity world transform. Uses the
/// quaternion (`rotation_z` is derived from it); for pure Z rotations this
/// is `2 atan2(z, w)`.
pub fn angle_of(o: &Object) -> f32 {
    o.world.rotation_z * rounds_sim::math::DEG2RAD
}

/// How a collider was converted (for the report).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approx {
    Exact,
    /// Box/edge radius treated as a square inflation.
    RoundedCorners,
    /// A concave polygon split into convex pieces.
    Decomposed,
    /// Degenerate geometry dropped (zero area, < 3 points).
    Dropped,
}

/// Convert one collider into convex shapes in the space `m` maps to
/// (`m` = collider object -> target space; world for statics, body-local
/// for props).
pub fn collider_shapes(c: &Collider, m: &Affine) -> (Vec<Shape>, Approx) {
    let off = v2(c.offset[0], c.offset[1]);
    let sx = m.ax.len();
    let sy = m.ay.len();
    match &c.shape {
        UShape::Box { size, edge_radius } => {
            let r = edge_radius.max(0.0);
            let size = v2(size[0].abs() + 2.0 * r, size[1].abs() + 2.0 * r);
            if size.x * sx <= 1e-6 || size.y * sy <= 1e-6 {
                return (vec![], Approx::Dropped);
            }
            let approx = if r > 0.0 {
                Approx::RoundedCorners
            } else {
                Approx::Exact
            };
            (vec![m.box_shape(size, off)], approx)
        }
        UShape::Circle { radius } => {
            if *radius <= 0.0 || sx.max(sy) <= 1e-6 {
                return (vec![], Approx::Dropped);
            }
            (vec![m.circle_shape(*radius, off)], Approx::Exact)
        }
        UShape::Capsule { size, direction } => capsule(m, off, v2(size[0], size[1]), *direction),
        UShape::Polygon { paths } => {
            let mut out = Vec::new();
            let mut approx = Approx::Exact;
            for path in paths {
                let pts: Vec<V2> = path.iter().map(|p| m.apply(v2(p[0], p[1]) + off)).collect();
                let (pieces, a) = convex_pieces(&pts);
                if a != Approx::Exact {
                    approx = a;
                }
                out.extend(pieces.into_iter().map(Shape::poly));
            }
            if out.is_empty() {
                approx = Approx::Dropped;
            }
            (out, approx)
        }
        UShape::Edge {
            points,
            edge_radius,
        } => {
            let r = (edge_radius * sx.max(sy)).max(0.025);
            let pts: Vec<V2> = points
                .iter()
                .map(|p| m.apply(v2(p[0], p[1]) + off))
                .collect();
            let mut out = Vec::new();
            for w in pts.windows(2) {
                let d = w[1] - w[0];
                let l = d.len();
                if l <= 1e-6 {
                    continue;
                }
                out.push(Shape::Obb {
                    c: (w[0] + w[1]) * 0.5,
                    axis: d / l,
                    half: v2(l * 0.5 + r, r),
                });
            }
            let approx = if out.is_empty() {
                Approx::Dropped
            } else {
                Approx::RoundedCorners
            };
            (out, approx)
        }
    }
}

/// `CapsuleCollider2D`: size scaled per local axis; direction 0 = vertical.
fn capsule(m: &Affine, off: V2, size: V2, direction: i32) -> (Vec<Shape>, Approx) {
    let c = m.apply(off);
    let wx = m.ax.len() * size.x.abs();
    let wy = m.ay.len() * size.y.abs();
    let (ux, uy) = (m.ax.normalized(), m.ay.normalized());
    // (radius, half length of the core segment, segment direction, width axis)
    let (r, h, dir, side) = if direction == 0 {
        (wx * 0.5, (wy * 0.5 - wx * 0.5).max(0.0), uy, ux)
    } else {
        (wy * 0.5, (wx * 0.5 - wy * 0.5).max(0.0), ux, uy)
    };
    if r <= 1e-6 || ux.is_zero() || uy.is_zero() {
        return (vec![], Approx::Dropped);
    }
    let mut out = Vec::new();
    if h > 1e-6 {
        // Box axis = the width axis so `half = (r, h)` lines up.
        out.push(Shape::Obb {
            c,
            axis: side,
            half: v2(r, h),
        });
        out.push(Shape::Circle { c: c + dir * h, r });
        out.push(Shape::Circle { c: c - dir * h, r });
    } else {
        out.push(Shape::Circle { c, r });
    }
    (out, Approx::Exact)
}

fn signed_area(p: &[V2]) -> f32 {
    let mut a = 0.0;
    for i in 0..p.len() {
        a += p[i].cross(p[(i + 1) % p.len()]);
    }
    a * 0.5
}

fn is_convex(p: &[V2]) -> bool {
    let n = p.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let a = p[i];
        let b = p[(i + 1) % n];
        let c = p[(i + 2) % n];
        if (b - a).cross(c - b) < -1e-7 {
            return false;
        }
    }
    true
}

/// Remove duplicate and collinear consecutive points.
fn clean(pts: &[V2]) -> Vec<V2> {
    let mut p: Vec<V2> = Vec::with_capacity(pts.len());
    for &q in pts {
        if p.last().is_none_or(|l| (q - *l).len_sq() > 1e-12) {
            p.push(q);
        }
    }
    while p.len() > 1 && (p[0] - p[p.len() - 1]).len_sq() <= 1e-12 {
        p.pop();
    }
    let mut changed = true;
    while changed && p.len() >= 3 {
        changed = false;
        for i in 0..p.len() {
            let n = p.len();
            let a = p[(i + n - 1) % n];
            let b = p[i];
            let c = p[(i + 1) % n];
            if (b - a).cross(c - b).abs() <= 1e-9 * (1.0 + (c - a).len_sq()) {
                p.remove(i);
                changed = true;
                break;
            }
        }
    }
    p
}

/// Split a simple polygon (any winding) into convex CCW pieces: ear clipping
/// followed by a Hertel-Mehlhorn merge of triangles across diagonals.
pub fn convex_pieces(pts: &[V2]) -> (Vec<Vec<V2>>, Approx) {
    let mut p = clean(pts);
    if p.len() < 3 || signed_area(&p).abs() <= 1e-9 {
        return (vec![], Approx::Dropped);
    }
    if signed_area(&p) < 0.0 {
        p.reverse();
    }
    if is_convex(&p) {
        return (vec![p], Approx::Exact);
    }
    // Ear clipping on indices.
    let mut idx: Vec<usize> = (0..p.len()).collect();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = idx.len();
        let mut clipped = false;
        for i in 0..n {
            let (ia, ib, ic) = (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
            let (a, b, c) = (p[ia], p[ib], p[ic]);
            if (b - a).cross(c - b) <= 1e-12 {
                continue; // reflex or degenerate
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let q = p[j];
                (b - a).cross(q - a) >= 0.0
                    && (c - b).cross(q - b) >= 0.0
                    && (a - c).cross(q - c) >= 0.0
            });
            if inside {
                continue;
            }
            tris.push([ia, ib, ic]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break; // self-intersecting input; keep what we have
        }
    }
    if idx.len() == 3 {
        tris.push([idx[0], idx[1], idx[2]]);
    }
    // Merge across shared diagonals while the result stays convex.
    let mut polys: Vec<Vec<usize>> = tris.into_iter().map(|t| t.to_vec()).collect();
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..polys.len() {
            for j in i + 1..polys.len() {
                if let Some(m) = merge(&polys[i], &polys[j])
                    && is_convex(&m.iter().map(|&k| p[k]).collect::<Vec<_>>())
                {
                    polys[i] = m;
                    polys.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    let out = polys
        .into_iter()
        .map(|q| q.into_iter().map(|k| p[k]).collect::<Vec<_>>())
        .filter(|q| signed_area(q).abs() > 1e-9)
        .collect();
    (out, Approx::Decomposed)
}

/// Merge two CCW index polygons sharing one edge (a->b in `x`, b->a in `y`).
fn merge(x: &[usize], y: &[usize]) -> Option<Vec<usize>> {
    let (nx, ny) = (x.len(), y.len());
    for i in 0..nx {
        let (a, b) = (x[i], x[(i + 1) % nx]);
        for j in 0..ny {
            if y[j] == b && y[(j + 1) % ny] == a {
                // x from b around to a, then y from a around to b (exclusive).
                let mut out = Vec::with_capacity(nx + ny - 2);
                for k in 0..nx {
                    out.push(x[(i + 1 + k) % nx]);
                }
                for k in 0..ny - 2 {
                    out.push(y[(j + 2 + k) % ny]);
                }
                return Some(out);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rounds_import::format::{Trs, WorldTransform};

    fn trs(pos: [f32; 2], deg: f32, scale: [f32; 2]) -> Trs {
        let h = deg.to_radians() * 0.5;
        Trs {
            position: [pos[0], pos[1], 0.0],
            rotation: [0.0, 0.0, h.sin(), h.cos()],
            scale: [scale[0], scale[1], 1.0],
        }
    }

    fn obj(parent: Option<usize>, local: Trs) -> Object {
        Object {
            id: "t:1".into(),
            name: "o".into(),
            parent,
            children: vec![],
            active: true,
            active_in_hierarchy: true,
            layer: 0,
            transform: None,
            tag: "Untagged".into(),
            local,
            world: WorldTransform::default(),
            rect: None,
            sprite: None,
            colliders: vec![],
            rigidbody: None,
            joints: vec![],
            particles: None,
            components: vec![],
        }
    }

    fn close(a: V2, b: V2) -> bool {
        (a - b).len() < 1e-4
    }

    fn collider(shape: UShape, offset: [f32; 2]) -> Collider {
        Collider {
            id: "t:2".into(),
            enabled: true,
            is_trigger: false,
            offset,
            density: 1.0,
            material: None,
            shape,
        }
    }

    #[test]
    fn compose_translation_rotation_scale() {
        // Parent at (10, 0) rotated 90 deg, scale 2; child at local (1, 0).
        let objs = vec![
            obj(None, trs([10.0, 0.0], 90.0, [2.0, 2.0])),
            obj(Some(0), trs([1.0, 0.0], 0.0, [1.0, 1.0])),
        ];
        let w = compose_world(&objs);
        let a = w[1].to_affine();
        assert!(close(a.t, v2(10.0, 2.0)), "{:?}", a.t);
        assert!(close(a.apply(v2(1.0, 0.0)), v2(10.0, 4.0)));
        assert!(close(a.ax, v2(0.0, 2.0)) && close(a.ay, v2(-2.0, 0.0)));
    }

    #[test]
    fn non_uniform_parent_scale_shears_rotated_child() {
        // Parent scale (4, 1), child rotated 45 deg: the child's axes are no
        // longer orthogonal, so a box becomes a polygon.
        let objs = vec![
            obj(None, trs([0.0, 0.0], 0.0, [4.0, 1.0])),
            obj(Some(0), trs([0.0, 0.0], 45.0, [1.0, 1.0])),
        ];
        let a = compose_world(&objs)[1].to_affine();
        assert!(a.ax.dot(a.ay).abs() > 1.0);
        let (s, approx) = collider_shapes(
            &collider(
                UShape::Box {
                    size: [1.0, 1.0],
                    edge_radius: 0.0,
                },
                [0.0, 0.0],
            ),
            &a,
        );
        assert_eq!(approx, Approx::Exact);
        match &s[0] {
            Shape::Poly { pts } => {
                assert_eq!(pts.len(), 4);
                // Corner (0.5, 0.5) of the child: rotated to (0, 0.707), then x*4.
                assert!(pts.iter().any(|p| close(*p, v2(0.0, 0.70710677))));
                assert!(pts.iter().any(|p| close(*p, v2(2.828427, 0.0))));
            }
            s => panic!("expected a polygon, got {s:?}"),
        }
    }

    #[test]
    fn y_rotation_flips_x() {
        // 180 deg about Y mirrors local x (as the player's HoldingObject does with 90).
        let mut t = trs([0.0, 0.0], 0.0, [1.0, 1.0]);
        t.rotation = [0.0, 1.0, 0.0, 0.0];
        let a = Mat34::from_trs(&t).to_affine();
        assert!(close(a.ax, v2(-1.0, 0.0)) && close(a.ay, v2(0.0, 1.0)));
    }

    #[test]
    fn box_with_offset_and_rotation() {
        let a = compose_world(&[obj(None, trs([3.0, 4.0], 90.0, [2.0, 1.0]))])[0].to_affine();
        let (s, _) = collider_shapes(
            &collider(
                UShape::Box {
                    size: [1.0, 1.0],
                    edge_radius: 0.0,
                },
                [0.5, 0.0],
            ),
            &a,
        );
        match &s[0] {
            Shape::Obb { c, axis, half } => {
                // Offset (0.5, 0) scaled x2 then rotated 90: +1 in y.
                assert!(close(*c, v2(3.0, 5.0)), "{c:?}");
                assert!(close(*axis, v2(0.0, 1.0)));
                assert!(close(*half, v2(1.0, 0.5)));
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn circle_uses_largest_axis_scale() {
        let a = compose_world(&[obj(None, trs([1.0, 1.0], 30.0, [1.5, 3.0]))])[0].to_affine();
        let (s, _) = collider_shapes(&collider(UShape::Circle { radius: 0.5 }, [0.0, 0.0]), &a);
        assert_eq!(
            s,
            vec![Shape::Circle {
                c: v2(1.0, 1.0),
                r: 1.5
            }]
        );
    }

    #[test]
    fn capsule_is_box_and_two_circles() {
        let a = Affine::IDENTITY;
        let (s, _) = collider_shapes(
            &collider(
                UShape::Capsule {
                    size: [1.0, 3.0],
                    direction: 0,
                },
                [0.0, 0.0],
            ),
            &a,
        );
        assert_eq!(s.len(), 3);
        assert!(s.contains(&Shape::Circle {
            c: v2(0.0, 1.0),
            r: 0.5
        }));
        assert!(s.contains(&Shape::Circle {
            c: v2(0.0, -1.0),
            r: 0.5
        }));
        // A capsule wider than tall in vertical mode is just a circle.
        let (s, _) = collider_shapes(
            &collider(
                UShape::Capsule {
                    size: [2.0, 1.0],
                    direction: 0,
                },
                [0.0, 0.0],
            ),
            &a,
        );
        assert_eq!(
            s,
            vec![Shape::Circle {
                c: V2::ZERO,
                r: 1.0
            }]
        );
    }

    #[test]
    fn concave_polygon_is_decomposed() {
        // An L shape (clockwise on purpose).
        let l = vec![
            [0.0, 0.0],
            [0.0, 2.0],
            [1.0, 2.0],
            [1.0, 1.0],
            [2.0, 1.0],
            [2.0, 0.0],
        ];
        let (s, approx) = collider_shapes(
            &collider(UShape::Polygon { paths: vec![l] }, [0.0, 0.0]),
            &Affine::IDENTITY,
        );
        assert_eq!(approx, Approx::Decomposed);
        assert_eq!(s.len(), 2);
        let area: f32 = s.iter().map(|s| s.area()).sum();
        assert!((area - 3.0).abs() < 1e-4, "{area}");
        for sh in &s {
            if let Shape::Poly { pts } = sh {
                assert!(is_convex(pts) && signed_area(pts) > 0.0);
            }
        }
        // Points inside both arms are covered.
        assert!(s.iter().any(|sh| sh.contains(v2(0.5, 1.5))));
        assert!(s.iter().any(|sh| sh.contains(v2(1.5, 0.5))));
        assert!(!s.iter().any(|sh| sh.contains(v2(1.5, 1.5))));
    }

    #[test]
    fn convex_polygon_with_offset_and_transform() {
        let a = Affine::trs(v2(1.0, 0.0), 0.0, v2(2.0, 2.0));
        let tri = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let (s, approx) = collider_shapes(
            &collider(UShape::Polygon { paths: vec![tri] }, [1.0, 1.0]),
            &a,
        );
        assert_eq!(approx, Approx::Exact);
        let Shape::Poly { pts } = &s[0] else { panic!() };
        assert!(pts.iter().any(|p| close(*p, v2(3.0, 2.0))));
        assert!(pts.iter().any(|p| close(*p, v2(5.0, 2.0))));
        assert!(pts.iter().any(|p| close(*p, v2(3.0, 4.0))));
    }

    #[test]
    fn edge_chain_becomes_thin_boxes() {
        let (s, _) = collider_shapes(
            &collider(
                UShape::Edge {
                    points: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
                    edge_radius: 0.1,
                },
                [0.0, 0.0],
            ),
            &Affine::IDENTITY,
        );
        assert_eq!(s.len(), 2);
        assert!(s[0].contains(v2(1.0, 0.05)) && !s[0].contains(v2(1.0, 0.2)));
        assert!(s[1].contains(v2(2.0, 1.0)));
    }

    #[test]
    fn degenerate_shapes_are_dropped() {
        let (s, a) = collider_shapes(
            &collider(
                UShape::Box {
                    size: [1.0, 0.0],
                    edge_radius: 0.0,
                },
                [0.0, 0.0],
            ),
            &Affine::IDENTITY,
        );
        assert!(s.is_empty() && a == Approx::Dropped);
    }

    #[test]
    fn body_local_colliders() {
        // A prop at (5, 5) rotated 90 with scale (2, 1): the body frame has no
        // scale, so its local collider is a 2x1 box aligned with x.
        let w = compose_world(&[obj(None, trs([5.0, 5.0], 90.0, [2.0, 1.0]))])[0].to_affine();
        let local = pose(v2(5.0, 5.0), 90f32.to_radians()).inverse().mul(&w);
        let (s, _) = collider_shapes(
            &collider(
                UShape::Box {
                    size: [1.0, 1.0],
                    edge_radius: 0.0,
                },
                [0.0, 0.0],
            ),
            &local,
        );
        let Shape::Obb { c, axis, half } = &s[0] else {
            panic!()
        };
        assert!(close(*c, V2::ZERO) && close(*axis, v2(1.0, 0.0)) && close(*half, v2(1.0, 0.5)));
    }
}
