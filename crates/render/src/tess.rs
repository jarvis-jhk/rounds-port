//! Polygon triangulation (ear clipping) for filled polygons.

use glam::Vec2;

/// Twice the signed area; positive for counter-clockwise polygons.
pub fn signed_area2(points: &[Vec2]) -> f32 {
    let n = points.len();
    (0..n)
        .map(|i| points[i].perp_dot(points[(i + 1) % n]))
        .sum()
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = (b - a).perp_dot(p - a);
    let d2 = (c - b).perp_dot(p - b);
    let d3 = (a - c).perp_dot(p - c);
    d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
}

/// Triangulates a simple polygon (convex or concave, either winding, no
/// self-intersections, no holes). Appends indices into `points` to `out`
/// (counter-clockwise triangles). Degenerate input yields what it can.
pub fn triangulate(points: &[Vec2], out: &mut Vec<u32>) {
    let n = points.len();
    if n < 3 {
        return;
    }
    // Work on a CCW index ring.
    let mut ring: Vec<u32> = (0..n as u32).collect();
    if signed_area2(points) < 0.0 {
        ring.reverse();
    }
    let p = |i: u32| points[i as usize];
    let mut guard = 0;
    let mut i = 0;
    while ring.len() > 3 {
        let m = ring.len();
        let (ia, ib, ic) = (ring[(i + m - 1) % m], ring[i % m], ring[(i + 1) % m]);
        let (a, b, c) = (p(ia), p(ib), p(ic));
        let convex = (b - a).perp_dot(c - b) > 0.0;
        let is_ear = convex
            && !ring
                .iter()
                .any(|&j| j != ia && j != ib && j != ic && point_in_triangle(p(j), a, b, c));
        if is_ear {
            out.extend_from_slice(&[ia, ib, ic]);
            ring.remove(i % m);
            guard = 0;
        } else {
            i += 1;
            guard += 1;
            if guard > m {
                // No ear found (degenerate or self-intersecting): fan the rest.
                break;
            }
        }
    }
    for k in 1..ring.len().saturating_sub(1) {
        out.extend_from_slice(&[ring[0], ring[k], ring[k + 1]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(points: &[Vec2], idx: &[u32]) -> f32 {
        idx.chunks(3)
            .map(|t| {
                let (a, b, c) = (
                    points[t[0] as usize],
                    points[t[1] as usize],
                    points[t[2] as usize],
                );
                (b - a).perp_dot(c - a) * 0.5
            })
            .sum()
    }

    #[test]
    fn square_both_windings() {
        let mut sq = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let mut idx = Vec::new();
        triangulate(&sq, &mut idx);
        assert_eq!(idx.len(), 6);
        assert!((area_of(&sq, &idx) - 1.0).abs() < 1e-6);
        sq.reverse();
        idx.clear();
        triangulate(&sq, &mut idx);
        assert!((area_of(&sq, &idx) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn concave_l_shape() {
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let mut idx = Vec::new();
        triangulate(&l, &mut idx);
        assert_eq!(idx.len(), 12);
        assert!((area_of(&l, &idx) - 3.0).abs() < 1e-6);
        // Every triangle is CCW.
        for t in idx.chunks(3) {
            let (a, b, c) = (l[t[0] as usize], l[t[1] as usize], l[t[2] as usize]);
            assert!((b - a).perp_dot(c - a) > 0.0);
        }
    }
}
