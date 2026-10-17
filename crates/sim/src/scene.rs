//! Gameplay collision queries over everything solid in the arena: static map
//! colliders (grid), animated map colliders, physics props (cached world
//! shapes) and player circles. Deterministic order: statics by index,
//! animated, props by index, players by id; nearest-hit ties keep the first.

use crate::content::PreparedMap;
use crate::geom::{Aabb, RayHit, Shape, layers, ray_circle};
use crate::math::V2;
use crate::physics::Prop;

/// What a query hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Static(u32),
    Animated(u16),
    Prop(u32),
    Player(u8),
}

/// A player as a collider.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Circle {
    pub c: V2,
    pub r: f32,
}

/// Borrowed view of the solid world for one query batch.
pub struct Scene<'a> {
    pub statics: Option<&'a PreparedMap>,
    pub animated: &'a [(Shape, u16)],
    pub props: &'a [Prop],
    /// Index = player id; None = not collidable (dead, inactive, ...).
    pub players: &'a [Option<Circle>],
}

impl<'a> Scene<'a> {
    /// Nearest ray hit along unit `d` up to `max`. `skip_player` excludes one
    /// player (the caster); `filter` can reject targets.
    pub fn raycast(
        &self,
        o: V2,
        d: V2,
        max: f32,
        mask: u32,
        skip_player: Option<u8>,
    ) -> Option<(RayHit, Target)> {
        self.raycast_filtered(o, d, max, mask, skip_player, |_, _| true)
    }

    pub fn raycast_filtered(
        &self,
        o: V2,
        d: V2,
        max: f32,
        mask: u32,
        skip_player: Option<u8>,
        mut accept: impl FnMut(&RayHit, Target) -> bool,
    ) -> Option<(RayHit, Target)> {
        let mut best: Option<(RayHit, Target)> = None;
        let q = Aabb::of_segment(o, o + d * max, 0.0);
        let mut consider = |h: RayHit, t: Target, best: &mut Option<(RayHit, Target)>| {
            if best.as_ref().is_none_or(|b| h.t < b.0.t) && accept(&h, t) {
                *best = Some((h, t));
            }
        };
        if let Some(m) = self.statics {
            m.grid.query(&q, |i| {
                let s = &m.statics[i as usize];
                if mask & layers::bit(s.layer) == 0 {
                    return;
                }
                if let Some(h) = s.shape.raycast(o, d, max) {
                    consider(h, Target::Static(i), &mut best);
                }
            });
        }
        for (shape, idx) in self.animated {
            if mask & layers::bit(layers::DEFAULT) == 0 || !shape.aabb().overlaps(&q) {
                continue;
            }
            if let Some(h) = shape.raycast(o, d, max) {
                consider(h, Target::Animated(*idx), &mut best);
            }
        }
        for (pi, p) in self.props.iter().enumerate() {
            if !p.alive || !p.aabb.overlaps(&q) {
                continue;
            }
            for (s, l) in p.shapes.iter().zip(p.layers.iter()) {
                if mask & layers::bit(*l) == 0 {
                    continue;
                }
                if let Some(h) = s.raycast(o, d, max) {
                    consider(h, Target::Prop(pi as u32), &mut best);
                }
            }
        }
        if mask & layers::bit(layers::PLAYER) != 0 {
            for (i, c) in self.players.iter().enumerate() {
                let Some(c) = c else { continue };
                if Some(i as u8) == skip_player {
                    continue;
                }
                if let Some(h) = ray_circle(o, d, max, c.c, c.r) {
                    consider(h, Target::Player(i as u8), &mut best);
                }
            }
        }
        best
    }

    /// Every collider overlapping the circle (`c`, `r`): target, the closest
    /// point on its surface, and whether `c` is inside it. Order: statics,
    /// animated, props, players.
    pub fn overlaps(
        &self,
        c: V2,
        r: f32,
        mask: u32,
        skip_player: Option<u8>,
        out: &mut Vec<(Target, V2, bool)>,
    ) {
        out.clear();
        let q = Aabb::around(c, r);
        let test = |s: &Shape| -> Option<(V2, bool)> {
            let (p, inside) = s.closest_surface_point(c);
            if inside || (p - c).len_sq() < r * r {
                Some((p, inside))
            } else {
                None
            }
        };
        if let Some(m) = self.statics {
            let mut tmp: Vec<u32> = Vec::new();
            m.grid.query(&q, |i| tmp.push(i));
            tmp.sort_unstable();
            for i in tmp {
                let s = &m.statics[i as usize];
                if mask & layers::bit(s.layer) == 0 {
                    continue;
                }
                if let Some((p, inside)) = test(&s.shape) {
                    out.push((Target::Static(i), p, inside));
                }
            }
        }
        for (shape, idx) in self.animated {
            if mask & layers::bit(layers::DEFAULT) == 0 || !shape.aabb().overlaps(&q) {
                continue;
            }
            if let Some((p, inside)) = test(shape) {
                out.push((Target::Animated(*idx), p, inside));
            }
        }
        for (pi, pr) in self.props.iter().enumerate() {
            if !pr.alive || !pr.aabb.overlaps(&q) {
                continue;
            }
            // One contact per prop (the closest of its shapes).
            let mut best: Option<(V2, bool, f32)> = None;
            for (s, l) in pr.shapes.iter().zip(pr.layers.iter()) {
                if mask & layers::bit(*l) == 0 {
                    continue;
                }
                if let Some((p, inside)) = test(s) {
                    let d = if inside { -1.0 } else { (p - c).len_sq() };
                    if best.is_none_or(|b| d < b.2) {
                        best = Some((p, inside, d));
                    }
                }
            }
            if let Some((p, inside, _)) = best {
                out.push((Target::Prop(pi as u32), p, inside));
            }
        }
        if mask & layers::bit(layers::PLAYER) != 0 {
            for (i, pc) in self.players.iter().enumerate() {
                let Some(pc) = pc else { continue };
                if Some(i as u8) == skip_player {
                    continue;
                }
                let s = Shape::Circle { c: pc.c, r: pc.r };
                if let Some((p, inside)) = test(&s) {
                    out.push((Target::Player(i as u8), p, inside));
                }
            }
        }
    }

    /// Map colliders (statics, props and, with `animated`, the animated
    /// colliders; never players) on `mask` whose bounds overlap `q`, in the
    /// deterministic query order.
    pub fn solid_shapes(&self, q: &Aabb, mask: u32, animated: bool, out: &mut Vec<Shape>) {
        out.clear();
        if let Some(m) = self.statics {
            let mut tmp: Vec<u32> = Vec::new();
            m.grid.query(q, |i| tmp.push(i));
            tmp.sort_unstable();
            for i in tmp {
                let s = &m.statics[i as usize];
                if mask & layers::bit(s.layer) != 0 && s.shape.aabb().overlaps(q) {
                    out.push(s.shape.clone());
                }
            }
        }
        if animated && mask & layers::bit(layers::DEFAULT) != 0 {
            for (shape, _) in self.animated {
                if shape.aabb().overlaps(q) {
                    out.push(shape.clone());
                }
            }
        }
        for pr in self.props.iter() {
            if !pr.alive || !pr.aabb.overlaps(q) {
                continue;
            }
            for (s, l) in pr.shapes.iter().zip(pr.layers.iter()) {
                if mask & layers::bit(*l) != 0 {
                    out.push(s.clone());
                }
            }
        }
    }

    /// Is `c` inside a static or prop collider on `mask` (no allocation)?
    pub fn inside_solid(&self, c: V2, mask: u32) -> bool {
        let q = Aabb::around(c, 0.0);
        let mut hit = false;
        if let Some(m) = self.statics {
            m.grid.query(&q, |i| {
                let s = &m.statics[i as usize];
                if !hit && mask & layers::bit(s.layer) != 0 && s.shape.contains(c) {
                    hit = true;
                }
            });
        }
        hit || self.props.iter().any(|pr| {
            pr.alive
                && pr.aabb.overlaps(&q)
                && pr
                    .shapes
                    .iter()
                    .zip(pr.layers.iter())
                    .any(|(s, l)| mask & layers::bit(*l) != 0 && s.contains(c))
        })
    }

    /// For a point `c` inside map geometry (statics and props on `mask`):
    /// the nearest point outside all of it, nudged slightly outward. None
    /// when `c` is not inside any such collider. Handles overlapping
    /// colliders, where the nearest face of one lies inside another
    /// (player.md 7.3, port decision). Animated colliders (smashers) and
    /// players are left to the per-contact push, so a smasher still presses
    /// a player against the floor.
    pub fn exit_point(&self, c: V2, mask: u32) -> Option<V2> {
        if !self.inside_solid(c, mask) {
            return None;
        }
        let mut near = Vec::new();
        for rad in [3.0f32, 8.0, 24.0] {
            self.solid_shapes(&Aabb::around(c, rad), mask, false, &mut near);
            if !near.iter().any(|s| s.contains(c)) {
                return None;
            }
            if let Some(q) = crate::geom::nearest_exterior(c, &near, rad, 1e-3) {
                return Some(q);
            }
        }
        None
    }

    /// Is any map collider (static/animated/prop) containing point `p` within
    /// radius `r` (teleport / spawn checks)?
    pub fn map_blocked(&self, p: V2, r: f32, mask: u32) -> bool {
        let mut v = Vec::new();
        self.overlaps(p, r, mask & !layers::bit(layers::PLAYER), None, &mut v);
        !v.is_empty()
    }

    /// `CanSeePlayer`-style line of sight: only map geometry blocks
    /// (cards.md 5.4).
    pub fn line_of_sight(&self, from: V2, to: V2) -> bool {
        self.line_of_sight_masked(from, to, layers::LINE_OF_SIGHT)
    }

    /// [`Scene::line_of_sight`] with the data's mask
    /// (`Tuning::line_of_sight_mask`); the Player bit is ignored.
    pub fn line_of_sight_masked(&self, from: V2, to: V2, mask: u32) -> bool {
        let d = to - from;
        let l = d.len();
        if l < 1e-6 {
            return true;
        }
        // Hits at the very start are ignored: effects spawned on a surface
        // (hit point) must see players standing on it.
        self.raycast_filtered(
            from,
            d / l,
            l,
            mask & !layers::bit(layers::PLAYER),
            None,
            |h, _| h.t > 0.05,
        )
        .is_none()
    }
}
