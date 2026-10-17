//! Shape queries used by players and projectiles: ray casts, closest points,
//! containment, and a uniform grid over the immutable static map geometry.
//!
//! Map colliders are boxes (oriented), circles or convex polygons in world
//! space. Physics props are simulated by rapier (physics.rs) but queried here
//! through a per-tick cache of their world shapes, so all gameplay queries use
//! one deterministic code path.

use crate::math::{V2, v2};
use serde::{Deserialize, Serialize};

/// Unity layer indices (player.md 1.3, maps.md 2.1).
pub mod layers {
    pub const DEFAULT: u8 = 0;
    pub const TRANSPARENT_FX: u8 = 1;
    pub const IGNORE_RAYCAST: u8 = 2;
    pub const WATER: u8 = 4;
    pub const UI: u8 = 5;
    pub const DEFAULT_POST: u8 = 8;
    pub const LIGHTING: u8 = 9;
    pub const PLAYER: u8 = 10;
    pub const CORPSE: u8 = 11;
    pub const VFX: u8 = 12;
    pub const LOCK_SELECTION: u8 = 13;
    pub const FOREGROUND: u8 = 14;
    pub const PROJECTILE: u8 = 15;
    pub const UI_TOUCH: u8 = 16;
    pub const IGNORE_PLAYER: u8 = 17;
    pub const BACKGROUND_OBJECT: u8 = 18;
    pub const PLAYER_OBJECT_COLLIDER: u8 = 19;
    pub const SPAWN_POINT: u8 = 20;
    pub const IGNORE_MAP: u8 = 21;

    pub const fn bit(l: u8) -> u32 {
        1u32 << l
    }

    /// PlayerCollision push-out mask (player.md 1.3).
    pub const PLAYER_COLLISION: u32 =
        bit(DEFAULT) | bit(PLAYER) | bit(IGNORE_PLAYER) | bit(IGNORE_MAP);
    /// Leg ground ray mask (player.md 1.3): includes other players.
    pub const LEG_RAY: u32 = bit(DEFAULT)
        | bit(TRANSPARENT_FX)
        | bit(IGNORE_RAYCAST)
        | bit(WATER)
        | bit(UI)
        | bit(DEFAULT_POST)
        | bit(LIGHTING)
        | bit(PLAYER)
        | bit(VFX)
        | bit(LOCK_SELECTION)
        | bit(IGNORE_PLAYER);
    /// Standard bullet wall mask (weapons.md 2).
    pub const BULLET_WALLS: u32 = bit(DEFAULT)
        | bit(TRANSPARENT_FX)
        | bit(WATER)
        | bit(UI)
        | bit(DEFAULT_POST)
        | bit(LIGHTING)
        | bit(PROJECTILE)
        | bit(IGNORE_PLAYER)
        | bit(IGNORE_MAP);
    /// Bullet "ignore walls" mask (only players).
    pub const BULLET_IGNORE_WALLS: u32 = bit(PLAYER);
    /// Line of sight (cards.md 5.4).
    pub const LINE_OF_SIGHT: u32 = bit(DEFAULT)
        | bit(TRANSPARENT_FX)
        | bit(IGNORE_RAYCAST)
        | bit(WATER)
        | bit(UI)
        | bit(DEFAULT_POST)
        | bit(LIGHTING)
        | bit(VFX)
        | bit(LOCK_SELECTION)
        | bit(FOREGROUND)
        | bit(IGNORE_PLAYER)
        | bit(IGNORE_MAP);
    /// Explosion overlap: everything except PlayerObjectCollider (weapons.md 19.1).
    pub const EXPLOSION: u32 = !bit(PLAYER_OBJECT_COLLIDER);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: V2,
    pub max: V2,
}

impl Aabb {
    pub fn new(min: V2, max: V2) -> Aabb {
        Aabb { min, max }
    }
    pub fn around(c: V2, r: f32) -> Aabb {
        Aabb {
            min: c - v2(r, r),
            max: c + v2(r, r),
        }
    }
    pub fn of_segment(a: V2, b: V2, pad: f32) -> Aabb {
        Aabb {
            min: a.min(b) - v2(pad, pad),
            max: a.max(b) + v2(pad, pad),
        }
    }
    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.min.x <= o.max.x
            && self.max.x >= o.min.x
            && self.min.y <= o.max.y
            && self.max.y >= o.min.y
    }
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }
    pub fn closest_point(&self, p: V2) -> V2 {
        v2(
            p.x.clamp(self.min.x, self.max.x),
            p.y.clamp(self.min.y, self.max.y),
        )
    }
    pub fn center(&self) -> V2 {
        (self.min + self.max) * 0.5
    }
}

/// A collider shape in world space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Circle {
        c: V2,
        r: f32,
    },
    /// Oriented box: centre, unit local x axis, half extents.
    Obb {
        c: V2,
        axis: V2,
        half: V2,
    },
    /// Convex polygon, counter-clockwise.
    Poly {
        pts: Vec<V2>,
    },
}

/// Result of a ray cast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub t: f32,
    pub point: V2,
    pub normal: V2,
}

impl Shape {
    /// Build a convex polygon, fixing the winding to counter-clockwise.
    pub fn poly(mut pts: Vec<V2>) -> Shape {
        let mut area = 0.0;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            area += a.cross(b);
        }
        if area < 0.0 {
            pts.reverse();
        }
        Shape::Poly { pts }
    }

    pub fn aabb(&self) -> Aabb {
        match self {
            Shape::Circle { c, r } => Aabb::around(*c, *r),
            Shape::Obb { c, axis, half } => {
                let ex = axis.x.abs() * half.x + axis.y.abs() * half.y;
                let ey = axis.y.abs() * half.x + axis.x.abs() * half.y;
                Aabb {
                    min: *c - v2(ex, ey),
                    max: *c + v2(ex, ey),
                }
            }
            Shape::Poly { pts } => {
                let mut a = Aabb {
                    min: pts[0],
                    max: pts[0],
                };
                for p in pts.iter() {
                    a.min = a.min.min(*p);
                    a.max = a.max.max(*p);
                }
                a
            }
        }
    }

    pub fn center(&self) -> V2 {
        match self {
            Shape::Circle { c, .. } | Shape::Obb { c, .. } => *c,
            Shape::Poly { pts } => {
                let mut s = V2::ZERO;
                for p in pts {
                    s += *p;
                }
                s / pts.len() as f32
            }
        }
    }

    pub fn contains(&self, p: V2) -> bool {
        match self {
            Shape::Circle { c, r } => (p - *c).len_sq() <= r * r,
            Shape::Obb { c, axis, half } => {
                let d = p - *c;
                let lx = d.dot(*axis);
                let ly = d.dot(axis.perp_left());
                lx.abs() <= half.x && ly.abs() <= half.y
            }
            Shape::Poly { pts } => {
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    if (b - a).cross(p - a) < 0.0 {
                        return false;
                    }
                }
                true
            }
        }
    }

    /// Closest point on the shape's surface to `p` and whether `p` is inside.
    pub fn closest_surface_point(&self, p: V2) -> (V2, bool) {
        match self {
            Shape::Circle { c, r } => {
                let d = p - *c;
                let l = d.len();
                let dir = if l > 1e-6 { d / l } else { V2::UP };
                (*c + dir * *r, l < *r)
            }
            Shape::Obb { c, axis, half } => {
                let up = axis.perp_left();
                let d = p - *c;
                let lx = d.dot(*axis);
                let ly = d.dot(up);
                let inside = lx.abs() <= half.x && ly.abs() <= half.y;
                let (qx, qy) = if inside {
                    // Project to the nearest face.
                    let dx = half.x - lx.abs();
                    let dy = half.y - ly.abs();
                    if dx < dy {
                        (half.x * sign(lx), ly)
                    } else {
                        (lx, half.y * sign(ly))
                    }
                } else {
                    (lx.clamp(-half.x, half.x), ly.clamp(-half.y, half.y))
                };
                (*c + *axis * qx + up * qy, inside)
            }
            Shape::Poly { pts } => {
                let inside = self.contains(p);
                let mut best = pts[0];
                let mut best_d = f32::INFINITY;
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    let q = closest_on_segment(a, b, p);
                    let dd = (q - p).len_sq();
                    if dd < best_d {
                        best_d = dd;
                        best = q;
                    }
                }
                (best, inside)
            }
        }
    }

    /// Ray cast from `o` along unit `d` up to `max`. Starting inside counts as
    /// a hit at t = 0 with the normal opposite to the travel direction
    /// (weapons.md 11).
    pub fn raycast(&self, o: V2, d: V2, max: f32) -> Option<RayHit> {
        if self.contains(o) {
            return Some(RayHit {
                t: 0.0,
                point: o,
                normal: -d,
            });
        }
        match self {
            Shape::Circle { c, r } => ray_circle(o, d, max, *c, *r),
            Shape::Obb { c, axis, half } => {
                let up = axis.perp_left();
                let lo = v2((o - *c).dot(*axis), (o - *c).dot(up));
                let ld = v2(d.dot(*axis), d.dot(up));
                let mut tmin = 0.0f32;
                let mut tmax = max;
                let mut n_local = V2::ZERO;
                for ax in 0..2 {
                    let (po, pd, h) = if ax == 0 {
                        (lo.x, ld.x, half.x)
                    } else {
                        (lo.y, ld.y, half.y)
                    };
                    if pd.abs() < 1e-12 {
                        if po < -h || po > h {
                            return None;
                        }
                    } else {
                        let mut t1 = (-h - po) / pd;
                        let mut t2 = (h - po) / pd;
                        let mut s = -1.0;
                        if t1 > t2 {
                            core::mem::swap(&mut t1, &mut t2);
                            s = 1.0;
                        }
                        if t1 > tmin {
                            tmin = t1;
                            n_local = if ax == 0 { v2(s, 0.0) } else { v2(0.0, s) };
                        }
                        if t2 < tmax {
                            tmax = t2;
                        }
                        if tmin > tmax {
                            return None;
                        }
                    }
                }
                if n_local.is_zero() {
                    return None;
                }
                let n = *axis * n_local.x + up * n_local.y;
                Some(RayHit {
                    t: tmin,
                    point: o + d * tmin,
                    normal: n,
                })
            }
            Shape::Poly { pts } => {
                // Cyrus-Beck against a CCW convex polygon.
                let mut tmin = 0.0f32;
                let mut tmax = max;
                let mut n_hit = V2::ZERO;
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    let e = b - a;
                    let n = e.perp_right().normalized(); // outward for CCW
                    let denom = n.dot(d);
                    let dist = n.dot(o - a);
                    if denom.abs() < 1e-12 {
                        if dist > 0.0 {
                            return None;
                        }
                        continue;
                    }
                    let t = -dist / denom;
                    if denom < 0.0 {
                        if t > tmin {
                            tmin = t;
                            n_hit = n;
                        }
                    } else if t < tmax {
                        tmax = t;
                    }
                    if tmin > tmax {
                        return None;
                    }
                }
                if n_hit.is_zero() {
                    return None;
                }
                Some(RayHit {
                    t: tmin,
                    point: o + d * tmin,
                    normal: n_hit,
                })
            }
        }
    }

    /// Translate the shape.
    pub fn translated(&self, by: V2) -> Shape {
        match self {
            Shape::Circle { c, r } => Shape::Circle { c: *c + by, r: *r },
            Shape::Obb { c, axis, half } => Shape::Obb {
                c: *c + by,
                axis: *axis,
                half: *half,
            },
            Shape::Poly { pts } => Shape::Poly {
                pts: pts.iter().map(|p| *p + by).collect(),
            },
        }
    }

    /// Transform a body-local shape by a pose (position, rotation cos/sin).
    pub fn transformed(&self, pos: V2, c: f32, s: f32) -> Shape {
        match self {
            Shape::Circle { c: cc, r } => Shape::Circle {
                c: pos + cc.rotate(c, s),
                r: *r,
            },
            Shape::Obb { c: cc, axis, half } => Shape::Obb {
                c: pos + cc.rotate(c, s),
                axis: axis.rotate(c, s),
                half: *half,
            },
            Shape::Poly { pts } => Shape::Poly {
                pts: pts.iter().map(|p| pos + p.rotate(c, s)).collect(),
            },
        }
    }

    /// Boundary segments `(a, b, outward unit normal)`. Circles are
    /// approximated by an inscribed 16-gon (only used to generate exit
    /// candidates; containment stays exact).
    pub fn boundary(&self, out: &mut Vec<(V2, V2, V2)>) {
        match self {
            Shape::Circle { c, r } => {
                const N: usize = 16;
                let at = |k: usize| {
                    let a = k as f32 * (2.0 * core::f32::consts::PI / N as f32);
                    *c + v2(crate::math::cos(a), crate::math::sin(a)) * *r
                };
                for k in 0..N {
                    let (a, b) = (at(k), at(k + 1));
                    out.push((a, b, (b - a).perp_right().normalized()));
                }
            }
            Shape::Obb { c, axis, half } => {
                let up = axis.perp_left();
                let x = *axis * half.x;
                let y = up * half.y;
                let p = [*c - x - y, *c + x - y, *c + x + y, *c - x + y];
                let n = [-up, *axis, up, -*axis];
                for k in 0..4 {
                    out.push((p[k], p[(k + 1) % 4], n[k]));
                }
            }
            Shape::Poly { pts } => {
                // Counter-clockwise: the outside is on the right.
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    let n = (b - a).perp_right().normalized();
                    if !n.is_zero() {
                        out.push((a, b, n));
                    }
                }
            }
        }
    }

    /// Area (for mass distribution and tests).
    pub fn area(&self) -> f32 {
        match self {
            Shape::Circle { r, .. } => core::f32::consts::PI * r * r,
            Shape::Obb { half, .. } => 4.0 * half.x * half.y,
            Shape::Poly { pts } => {
                let mut a = 0.0;
                for i in 0..pts.len() {
                    a += pts[i].cross(pts[(i + 1) % pts.len()]);
                }
                a.abs() * 0.5
            }
        }
    }
}

#[inline]
fn sign(x: f32) -> f32 {
    if x < 0.0 { -1.0 } else { 1.0 }
}

/// Intersection point of segments `a-b` and `c-d`, if they cross.
pub fn segment_intersection(a: V2, b: V2, c: V2, d: V2) -> Option<V2> {
    let r = b - a;
    let s = d - c;
    let den = r.cross(s);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = (c - a).cross(s) / den;
    let u = (c - a).cross(r) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

/// Nearest point to `c` outside every shape of `shapes` (a union of convex
/// shapes), searched among the projections of `c` onto every boundary
/// segment and the crossings of segments of different shapes, each nudged
/// `eps` outward. Candidates farther than `max` are ignored (the caller
/// only knows the shapes within `max`). Deterministic: ties keep the first.
pub fn nearest_exterior(c: V2, shapes: &[Shape], max: f32, eps: f32) -> Option<V2> {
    let mut segs: Vec<(V2, V2, V2, usize)> = Vec::new();
    let mut tmp = Vec::new();
    for (k, s) in shapes.iter().enumerate() {
        tmp.clear();
        s.boundary(&mut tmp);
        segs.extend(tmp.iter().map(|&(a, b, n)| (a, b, n, k)));
    }
    let mut best: Option<(f32, V2)> = None;
    let consider = |q: V2, best: &mut Option<(f32, V2)>| {
        let d = (q - c).len_sq();
        if d > max * max || best.is_some_and(|b| d >= b.0) {
            return;
        }
        if shapes.iter().all(|s| !s.contains(q)) {
            *best = Some((d, q));
        }
    };
    for &(a, b, n, _) in &segs {
        consider(closest_on_segment(a, b, c) + n * eps, &mut best);
    }
    for (i, &(a, b, n1, k1)) in segs.iter().enumerate() {
        for &(cc, d, n2, k2) in &segs[i + 1..] {
            if k1 == k2 {
                continue;
            }
            if let Some(x) = segment_intersection(a, b, cc, d) {
                let m = (n1 + n2).normalized();
                let dir = if m.is_zero() { n1 } else { m };
                consider(x + dir * (2.0 * eps), &mut best);
            }
        }
    }
    best.map(|b| b.1)
}

pub fn closest_on_segment(a: V2, b: V2, p: V2) -> V2 {
    let ab = b - a;
    let l2 = ab.len_sq();
    if l2 < 1e-12 {
        return a;
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    a + ab * t
}

/// Ray vs circle; `o` assumed outside.
pub fn ray_circle(o: V2, d: V2, max: f32, c: V2, r: f32) -> Option<RayHit> {
    let m = o - c;
    let b = m.dot(d);
    let cc = m.len_sq() - r * r;
    if cc <= 0.0 {
        return Some(RayHit {
            t: 0.0,
            point: o,
            normal: -d,
        });
    }
    if b > 0.0 {
        return None;
    }
    let disc = b * b - cc;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if t < 0.0 || t > max {
        return None;
    }
    let p = o + d * t;
    Some(RayHit {
        t,
        point: p,
        normal: (p - c).normalized(),
    })
}

/// A circle of radius `r` swept from `o` along unit `d` by `max` against a
/// circle collider (`c`, `R`). The hit point is on the collider surface, the
/// normal points from the collider to the sweeping centre (weapons.md 11).
pub fn sweep_circle_circle(o: V2, d: V2, max: f32, r: f32, c: V2, rr: f32) -> Option<RayHit> {
    let h = ray_circle(o, d, max, c, r + rr)?;
    let centre = o + d * h.t;
    let n = (centre - c).normalized();
    let n = if n.is_zero() { -d } else { n };
    Some(RayHit {
        t: h.t,
        point: c + n * rr,
        normal: n,
    })
}

/// Uniform grid over static shapes (built once per map in `Content`).
#[derive(Clone, Debug, Default)]
pub struct Grid {
    pub origin: V2,
    pub cell: f32,
    pub w: i32,
    pub h: i32,
    /// For each cell, the sorted indices of shapes whose AABB touches it.
    cells: Vec<Vec<u32>>,
    pub aabbs: Vec<Aabb>,
}

impl Grid {
    pub fn build(aabbs: Vec<Aabb>, cell: f32) -> Grid {
        if aabbs.is_empty() {
            return Grid {
                origin: V2::ZERO,
                cell,
                w: 0,
                h: 0,
                cells: vec![],
                aabbs,
            };
        }
        let mut b = aabbs[0];
        for a in &aabbs {
            b = b.union(a);
        }
        let w = (((b.max.x - b.min.x) / cell).floor() as i32 + 1).max(1);
        let h = (((b.max.y - b.min.y) / cell).floor() as i32 + 1).max(1);
        let mut g = Grid {
            origin: b.min,
            cell,
            w,
            h,
            cells: vec![Vec::new(); (w * h) as usize],
            aabbs,
        };
        for i in 0..g.aabbs.len() {
            let a = g.aabbs[i];
            let (x0, y0, x1, y1) = g.range(&a);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    g.cells[(y * w + x) as usize].push(i as u32);
                }
            }
        }
        g
    }

    fn range(&self, a: &Aabb) -> (i32, i32, i32, i32) {
        let f = |v: f32, o: f32, n: i32| (((v - o) / self.cell).floor() as i32).clamp(0, n - 1);
        (
            f(a.min.x, self.origin.x, self.w),
            f(a.min.y, self.origin.y, self.h),
            f(a.max.x, self.origin.x, self.w),
            f(a.max.y, self.origin.y, self.h),
        )
    }

    /// Call `f` once for every shape whose AABB overlaps `q`, in ascending
    /// index order per cell (deterministic). Duplicate visits are avoided by
    /// only reporting a shape in the first cell of the overlap region.
    pub fn query(&self, q: &Aabb, mut f: impl FnMut(u32)) {
        if self.w == 0 {
            return;
        }
        let gb = Aabb {
            min: self.origin,
            max: self.origin + v2(self.w as f32 * self.cell, self.h as f32 * self.cell),
        };
        if !gb.overlaps(q) {
            return;
        }
        let (x0, y0, x1, y1) = self.range(q);
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &i in &self.cells[(y * self.w + x) as usize] {
                    let a = &self.aabbs[i as usize];
                    if !a.overlaps(q) {
                        continue;
                    }
                    // Region overlap's min corner must be in this cell.
                    let inter_min = v2(a.min.x.max(q.min.x), a.min.y.max(q.min.y));
                    let (cx, cy, _, _) = self.range(&Aabb {
                        min: inter_min,
                        max: inter_min,
                    });
                    if cx == x && cy == y {
                        f(i);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Shape {
        Shape::Obb {
            c: V2::ZERO,
            axis: V2::RIGHT,
            half: v2(1.0, 1.0),
        }
    }

    #[test]
    fn obb_ray() {
        let b = unit_box();
        let h = b.raycast(v2(-5.0, 0.0), V2::RIGHT, 10.0).unwrap();
        assert!((h.t - 4.0).abs() < 1e-5);
        assert_eq!(h.normal, v2(-1.0, 0.0));
        assert!(b.raycast(v2(-5.0, 3.0), V2::RIGHT, 10.0).is_none());
        assert!(b.raycast(v2(-5.0, 0.0), V2::RIGHT, 3.0).is_none());
        let h = b.raycast(v2(0.0, 5.0), V2::DOWN, 10.0).unwrap();
        assert_eq!(h.normal, v2(0.0, 1.0));
    }

    #[test]
    fn poly_ray_and_closest() {
        let p = Shape::poly(vec![
            v2(-1.0, -1.0),
            v2(-1.0, 1.0),
            v2(1.0, 1.0),
            v2(1.0, -1.0),
        ]);
        let h = p.raycast(v2(0.0, 5.0), V2::DOWN, 10.0).unwrap();
        assert!((h.t - 4.0).abs() < 1e-5);
        assert!((h.normal - V2::UP).len() < 1e-5);
        let (q, inside) = p.closest_surface_point(v2(0.0, 3.0));
        assert!(!inside);
        assert!((q - v2(0.0, 1.0)).len() < 1e-6);
        let (q, inside) = unit_box().closest_surface_point(v2(0.2, 0.9));
        assert!(inside);
        assert!((q - v2(0.2, 1.0)).len() < 1e-6);
    }

    #[test]
    fn grid_query_unique() {
        let aabbs = vec![
            Aabb::new(v2(0.0, 0.0), v2(20.0, 1.0)),
            Aabb::new(v2(5.0, 5.0), v2(6.0, 6.0)),
        ];
        let g = Grid::build(aabbs, 4.0);
        let mut hits = vec![];
        g.query(&Aabb::new(v2(-1.0, -1.0), v2(30.0, 30.0)), |i| hits.push(i));
        hits.sort();
        assert_eq!(hits, vec![0, 1]);
    }

    #[test]
    fn sweep() {
        let h = sweep_circle_circle(v2(-5.0, 0.0), V2::RIGHT, 10.0, 0.5, V2::ZERO, 1.0).unwrap();
        assert!((h.t - 3.5).abs() < 1e-5);
        assert!((h.point - v2(-1.0, 0.0)).len() < 1e-5);
    }
}
