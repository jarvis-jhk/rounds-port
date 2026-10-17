//! Projectiles (weapons.md 9-17): movement integration, the per-tick sweep,
//! hit selection and resolution, blocking, bounces, bullet-vs-bullet.
//!
//! Card-specific projectile behaviours (homing, drill, poison, bash, remote,
//! grow, ...) are add-ons (`crate::script::addons`).

use crate::content::{Content, SpawnEntry};
use crate::effects::Hook;
use crate::events::{EventKind, HitTarget, cue};
use crate::geom::{RayHit, layers, sweep_circle_circle};
use crate::math::{V2, clamp, cos, v2};
use crate::scene::{Circle, Target};
use crate::world::{DT, Hasher, World, ticks};
use std::sync::Arc;

/// Players a projectile currently cannot hit: (player, real ticks left).
/// Inline (no allocation) so cloning thousands of bullets stays cheap.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HoldList {
    items: [(u8, u16); 6],
    len: u8,
}

impl HoldList {
    pub fn contains(&self, p: u8) -> bool {
        self.items[..self.len as usize].iter().any(|h| h.0 == p)
    }
    pub fn hold(&mut self, p: u8, ticks: u32) {
        let t = ticks.min(u16::MAX as u32) as u16;
        let n = self.len as usize;
        if let Some(h) = self.items[..n].iter_mut().find(|h| h.0 == p) {
            h.1 = t;
        } else if n < self.items.len() {
            self.items[n] = (p, t);
            self.len += 1;
        } else {
            // Full: replace the entry that expires first.
            let k = (0..n).min_by_key(|k| (self.items[*k].1, *k)).unwrap();
            self.items[k] = (p, t);
        }
    }
    pub fn remove(&mut self, p: u8) {
        let n = self.len as usize;
        if let Some(k) = self.items[..n].iter().position(|h| h.0 == p) {
            self.items.copy_within(k + 1..n, k);
            self.len -= 1;
        }
    }
    pub fn tick(&mut self) {
        let mut w = 0;
        for r in 0..self.len as usize {
            let (p, t) = self.items[r];
            if t > 1 {
                self.items[w] = (p, t - 1);
                w += 1;
            }
        }
        self.len = w as u8;
    }
}

/// Per-tick broadphase over bullet collision circles: a counting-sort grid
/// of a tick-start snapshot (position, velocity, radius). A sweeping bullet
/// tests the other circles at their tick-start positions, which makes the
/// result independent of update order. Positions beyond the grid clamp into
/// its border cells.
pub(crate) struct BulletBroad {
    start: Vec<u32>,
    items: Vec<BroadItem>,
    rmax: f32,
}

const BB_CELL: f32 = 1.5;
const BB_MIN: V2 = V2 { x: -60.0, y: -60.0 };
const BB_W: i32 = 80;
const BB_H: i32 = 80;

fn bb_cell(p: V2) -> (i32, i32) {
    let cx = (((p.x - BB_MIN.x) / BB_CELL).floor() as i32).clamp(0, BB_W - 1);
    let cy = (((p.y - BB_MIN.y) / BB_CELL).floor() as i32).clamp(0, BB_H - 1);
    (cx, cy)
}

/// Tick-start snapshot of one collision circle.
#[derive(Clone, Copy, Debug, Default)]
struct BroadItem {
    pos: V2,
    vel: V2,
    radius: f32,
    idx: u32,
}

impl BulletBroad {
    /// Candidates (ascending index per cell) overlapping the box.
    fn query(&self, lo: V2, hi: V2, mut f: impl FnMut(&BroadItem)) {
        let (x0, y0) = bb_cell(lo);
        let (x1, y1) = bb_cell(hi);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let c = (cy * BB_W + cx) as usize;
                for it in &self.items[self.start[c] as usize..self.start[c + 1] as usize] {
                    f(it);
                }
            }
        }
    }
}

/// Bullet-vs-bullet collision circle (weapons.md 17).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletCircle {
    pub hp: f32,
    pub threshold: f32,
    pub radius: f32,
    pub collided: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub id: u32,
    /// Shooter (spawner) player id.
    pub owner: Option<u8>,
    pub team: u8,
    pub pos: V2,
    pub last_pos: V2,
    pub vel: V2,
    pub start_speed: f32,
    pub gravity: f32,
    pub drag: f32,
    pub drag_min_speed: f32,
    pub sim_speed: f32,
    pub multiplier: f32,
    /// Gravity applies only while 0 (homing/remote increment it).
    pub simulate_gravity: i32,
    pub distance: f32,
    pub damage: f32,
    pub force: f32,
    pub deal_mult: f32,
    pub percentage_damage: f32,
    pub slow: f32,
    pub unblockable: bool,
    pub wall_mask: u32,
    /// Sweep radius against players.
    pub size: f32,
    pub hold_player_for: f32,
    /// Real seconds since spawn (0.2 s self-ignore); +inf after a block.
    pub age_real: f32,
    /// Remaining life, game seconds.
    pub lifetime: f32,
    /// Players this projectile cannot hit (id, real ticks left).
    pub held: HoldList,
    pub reflects: i32,
    pub speed_m_on_bounce: f32,
    pub dmg_m_on_bounce: f32,
    pub since_bounce_real: f32,
    pub since_reflect: f32,
    pub spawns: Arc<[SpawnEntry]>,
    pub shake: f32,
    pub can_push_box: bool,
    pub destroy_on_block: bool,
    pub collision: Option<BulletCircle>,
    /// Created this tick (first update next tick, like Unity's Start/Update).
    pub fresh: bool,
    pub damage_after_distance: Option<f32>,
    pub cos: f32,
    /// Has the screen-edge-bounce add-on (weapons.md 20.5).
    pub screen_edge_bounce: bool,
    pub last_edge: i8,
    pub immunity_key: Option<u32>,
    /// False for poison-type bullets (direct damage becomes 1).
    pub can_deal_damage: bool,
    pub allowed_to_spawn: bool,
    /// Destroyed this tick; removed at the end of the projectile pass.
    pub dead: bool,
    /// Add-on behaviours (homing, drill, poison, ...), in child order.
    pub addons: Vec<crate::script::addons::Addon>,
    /// Movement disabled (target-bounce pause, weapons.md 20.4).
    pub paused: bool,
}

/// Angle between two velocities below 45 degrees (weapons.md 17 step 1),
/// as `cos(angle) > cos(45)` without trigonometry.
#[inline]
fn within_45(a: V2, b: V2) -> bool {
    let d = a.dot(b);
    d > 0.0 && d * d > 0.5 * a.len_sq() * b.len_sq()
}

/// Stable 32-bit key for names (immunity entries).
pub fn name_key(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl Projectile {
    pub fn forward(&self) -> V2 {
        let f = self.vel.normalized();
        if f.is_zero() { V2::RIGHT } else { f }
    }

    pub(crate) fn hash_into(&self, h: &mut Hasher) {
        h.u32(self.id);
        h.v2(self.pos);
        h.v2(self.vel);
        h.f32(self.damage);
        h.f32(self.lifetime);
        h.u32(self.reflects as u32);
        h.f32(self.sim_speed);
        h.u32(self.simulate_gravity as u32);
        h.u32(self.addons.len() as u32);
    }

    /// Change the damage before the projectile's first update (Empower):
    /// force, sweep radius and collision circle follow (weapons.md 8).
    pub fn set_damage(&mut self, damage: f32, extra_size: f32) {
        let old = self.damage.max(1e-6);
        let k = damage / old;
        self.force *= k * k;
        self.damage = damage;
        self.size = clamp(crate::math::pow(damage, 0.85) / 400.0, 0.0, 100.0) + 0.3 + extra_size;
        if let Some(c) = self.collision.as_mut() {
            let r0 = c.radius / ((old / 55.0 + 1.0) * 0.5);
            c.hp = damage;
            c.threshold = 0.1 * damage;
            c.radius = r0 * (damage / 55.0 + 1.0) * 0.5;
        }
    }

    fn is_held(&self, p: u8) -> bool {
        self.held.contains(p)
    }
    fn hold(&mut self, p: u8, t: u32) {
        self.held.hold(p, t);
    }
}

/// Outcome of a hit effect (weapons.md 16).
use crate::script::addons::HitResult;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cand {
    Map(Target),
    Player(u8),
}

impl World {
    /// Add-on derived flags of a fresh projectile (weapons.md 16.3, 20.5).
    pub(crate) fn apply_projectile_addons(&mut self, _content: &Content, idx: usize) {
        use crate::script::addons::AddonDef;
        let p = &mut self.projectiles[idx];
        for a in &p.addons {
            match a.def {
                AddonDef::ScreenEdgeBounce => p.screen_edge_bounce = true,
                AddonDef::Poison { .. } => p.can_deal_damage = false,
                _ => {}
            }
        }
    }

    pub(crate) fn projectiles_tick(&mut self, content: &Content, game_dt: f32) {
        let circles = self.player_circles();
        let broad = self.bullet_broad(content);
        for i in 0..self.projectiles.len() {
            if self.projectiles[i].dead {
                continue;
            }
            if self.projectiles[i].fresh {
                self.projectiles[i].fresh = false;
                self.apply_projectile_addons(content, i);
                continue;
            }
            if !self.projectile_update(content, i, game_dt, &circles, broad.as_ref()) {
                self.projectiles[i].dead = true;
            }
        }
        if self.projectiles.iter().any(|p| p.dead) {
            for p in self.projectiles.iter().filter(|p| p.dead) {
                let seq = self.events.list.len() as u32;
                self.events.list.push(crate::events::Event {
                    frame: self.frame,
                    seq,
                    kind: EventKind::ProjectileRemoved { id: p.id },
                });
            }
            self.projectiles.retain(|p| !p.dead);
        }
    }

    fn bullet_broad(&self, content: &Content) -> Option<BulletBroad> {
        if !content.tuning.bullet_collision || self.projectiles.len() < 2 {
            return None;
        }
        let ncell = (BB_W * BB_H) as usize;
        let mut count = vec![0u32; ncell + 1];
        let mut rmax = 0.0f32;
        let mut cells = Vec::with_capacity(self.projectiles.len());
        for p in self.projectiles.iter() {
            let c = match p.collision {
                Some(bc) if !bc.collided && !p.fresh && p.since_bounce_real >= 0.5 => {
                    rmax = rmax.max(bc.radius);
                    let (cx, cy) = bb_cell(p.pos);
                    let c = (cy * BB_W + cx) as usize;
                    count[c + 1] += 1;
                    c as u32
                }
                _ => u32::MAX,
            };
            cells.push(c);
        }
        for k in 0..ncell {
            count[k + 1] += count[k];
        }
        let mut fill = count.clone();
        let mut items = vec![BroadItem::default(); count[ncell] as usize];
        for (j, c) in cells.iter().enumerate() {
            if *c != u32::MAX {
                let at = &mut fill[*c as usize];
                let p = &self.projectiles[j];
                items[*at as usize] = BroadItem {
                    pos: p.pos,
                    vel: p.vel,
                    radius: p.collision.map(|b| b.radius).unwrap_or(0.0),
                    idx: j as u32,
                };
                *at += 1;
            }
        }
        Some(BulletBroad {
            start: count,
            items,
            rmax,
        })
    }

    /// Returns false if the projectile must be destroyed.
    fn projectile_update(
        &mut self,
        content: &Content,
        i: usize,
        dt: f32,
        circles: &[Option<Circle>],
        broad: Option<&BulletBroad>,
    ) -> bool {
        // 1. Behaviours: add-ons in child order, then wave motion.
        if !self.projectiles[i].addons.is_empty() {
            self.addons_update(content, i, dt);
        }
        if !self.projectiles[i].paused {
            let p = &mut self.projectiles[i];
            if p.cos > 0.0 {
                let m = p.cos;
                let right = p.forward().perp_right();
                p.pos += right * (cos(p.age_real * 20.0 * m) * 10.0 * m * DT);
            }
        }
        // 2. Movement integration (10); disabled while paused.
        if !self.projectiles[i].paused {
            let p = &mut self.projectiles[i];
            let step_drag = clamp(dt, 0.0, 0.02) * p.sim_speed;
            let step = dt * p.sim_speed;
            if p.simulate_gravity == 0 {
                p.vel.y -= p.gravity * step * p.multiplier;
            }
            let sp = p.vel.len();
            if sp > 2.0 && sp > p.drag_min_speed {
                p.vel -=
                    p.vel * clamp(p.drag * step_drag * clamp(p.multiplier, 0.0, 1.0), 0.0, 1.0);
            }
            p.pos += p.vel * (step * p.multiplier);
            p.distance += p.vel.len() * step * p.multiplier;
            if let Some(m) = p.damage_after_distance
                && p.distance > 10.0
            {
                p.damage *= m;
                p.shake *= m;
                p.damage_after_distance = None;
            }
        }
        // 3. Sweep (11).
        if !self.projectile_sweep(content, i, circles, broad) {
            return false;
        }
        if self.projectiles[i].dead {
            return false;
        }
        // Screen edge bounce (20.5).
        self.addons_after_sweep(content, i);
        // 4. Timers.
        let p = &mut self.projectiles[i];
        p.lifetime -= dt;
        p.age_real += DT;
        p.since_bounce_real += DT;
        // Game clock (`TimeHandler.deltaTime`, weapons.md 20.2).
        p.since_reflect += dt;
        p.held.tick();
        p.lifetime > 0.0
    }

    /// Sweep from `last_pos` to `pos`; resolves at most one hit. Returns false
    /// if the projectile is destroyed.
    fn projectile_sweep(
        &mut self,
        content: &Content,
        i: usize,
        circles: &[Option<Circle>],
        broad: Option<&BulletBroad>,
    ) -> bool {
        let (from, to, size, mask, owner, young) = {
            let p = &self.projectiles[i];
            (
                p.last_pos,
                p.pos,
                p.size,
                p.wall_mask,
                p.owner,
                p.age_real < 0.2,
            )
        };
        let d = to - from;
        let l = d.len();
        if l < 1e-7 {
            self.projectiles[i].last_pos = to;
            return true;
        }
        let dir = d / l;
        let mut best: Option<(RayHit, Cand)> = None;
        {
            let scene = self.scene(content, circles);
            // Wall ray (players are not on the wall mask).
            if let Some((h, t)) =
                scene.raycast(from, dir, l, mask & !layers::bit(layers::PLAYER), None)
            {
                best = Some((h, Cand::Map(t)));
            }
            // Player circle cast.
            for (pi, c) in circles.iter().enumerate() {
                let Some(Circle { c, r }) = c else { continue };
                if young && owner == Some(pi as u8) {
                    continue;
                }
                if layers::bit(layers::PLAYER) & (mask | layers::BULLET_IGNORE_WALLS) == 0 {
                    continue;
                }
                if let Some(h) = sweep_circle_circle(from, dir, l, size, *c, *r)
                    && best.as_ref().is_none_or(|b| h.t < b.0.t)
                {
                    best = Some((h, Cand::Player(pi as u8)));
                }
            }
        }
        // Bullet circles (surface hook, always HasToStop: side effects only).
        if let (Some(bb), true) = (broad, mask & layers::bit(layers::PROJECTILE) != 0) {
            let limit = best.as_ref().map(|b| b.0.t).unwrap_or(l);
            let end = from + dir * limit;
            // Other circles are tested at their tick-start snapshot, so the
            // box only needs the largest radius around the swept segment.
            let pad = bb.rmax;
            let lo = from.min(end) - V2 { x: pad, y: pad };
            let hi = from.max(end) + V2 { x: pad, y: pad };
            let (vel_i, young_i) = (
                self.projectiles[i].vel,
                self.projectiles[i].since_bounce_real < 0.5,
            );
            let mut hits: Vec<(f32, usize)> = Vec::new();
            if !young_i {
                bb.query(lo, hi, |it| {
                    // The hook ignores pairs flying within 45 degrees.
                    if it.idx as usize == i || within_45(vel_i, it.vel) {
                        return;
                    }
                    if let Some(h) = crate::geom::ray_circle(from, dir, limit, it.pos, it.radius) {
                        hits.push((h.t, it.idx as usize));
                    }
                });
            }
            hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, j) in hits {
                self.bullet_surface_hook(content, i, j);
                if self.projectiles[i].dead {
                    return false;
                }
            }
        }
        let Some((h, cand)) = best else {
            self.projectiles[i].last_pos = self.projectiles[i].pos;
            return true;
        };
        let target = match cand {
            Cand::Player(p) => HitTarget::Player(p),
            Cand::Map(Target::Prop(pi)) => HitTarget::Prop(pi),
            Cand::Map(_) => HitTarget::Map,
        };
        let alive = self.projectile_hit(content, i, h.point, h.normal, target, false);
        if alive {
            let p = &mut self.projectiles[i];
            p.last_pos = p.pos;
        }
        alive
    }

    /// `ProjectileHit.Hit` + `DoHit` (12). Returns false if destroyed.
    pub(crate) fn projectile_hit(
        &mut self,
        content: &Content,
        i: usize,
        point: V2,
        normal: V2,
        target: HitTarget,
        forced: bool,
    ) -> bool {
        let mut blocked = false;
        if let HitTarget::Player(pl) = target {
            let pr = &self.projectiles[i];
            if pr.is_held(pl) && !forced {
                return true;
            }
            blocked = self.players[pl as usize].is_blocking() && !pr.unblockable;
            let t = ticks(pr.hold_player_for);
            self.projectiles[i].hold(pl, t);
        }
        // 2. Move to the hit point.
        if self.projectiles[i].allowed_to_spawn {
            self.projectiles[i].pos = point;
        }
        let fwd = self.projectiles[i].forward();
        let owner = self.projectiles[i].owner;
        let id = self.projectiles[i].id;
        // 4. Player: block / slow.
        if let HitTarget::Player(pl) = target {
            if blocked {
                self.events.push(EventKind::ProjectileHit {
                    id,
                    point,
                    normal,
                    damage: 0.0,
                    target,
                    blocked: true,
                });
                return self.projectile_blocked(content, i, pl as usize, point);
            }
            let slow = self.projectiles[i].slow;
            if slow != 0.0 {
                self.add_slow(pl as usize, slow, false);
            }
        }
        // 5. Damage.
        let mut dealt = 0.0;
        match target {
            HitTarget::Player(pl) => {
                let pi = pl as usize;
                if self.projectiles[i].percentage_damage != 0.0 {
                    let add =
                        self.players[pi].stats.max_health * self.projectiles[i].percentage_damage;
                    self.projectiles[i].damage += add;
                }
                let pr = &self.projectiles[i];
                let amount = if pr.can_deal_damage {
                    pr.damage * pr.deal_mult
                } else {
                    pr.deal_mult
                };
                // The original records the immunity entry but ignores the
                // answer: bullets always deal their damage (weapons.md 12.2
                // step 5, Clarified).
                if let Some(k) = pr.immunity_key {
                    let _ = self.is_immune(pi, 0.1, amount, k);
                }
                dealt = amount;
                self.take_damage(content, pi, fwd * amount, point, owner, true, true);
            }
            HitTarget::Prop(pp) => {
                let pr = &self.projectiles[i];
                let amount = pr.damage * pr.deal_mult;
                dealt = amount;
                self.damage_prop(content, pp as usize, fwd * amount);
            }
            _ => {}
        }
        // 6. Knockback.
        if let HitTarget::Player(pl) = target {
            let pi = pl as usize;
            let (mass, v) = (self.players[pi].mass, self.players[pi].vel);
            self.take_force(
                pi,
                -v * 0.1 * mass,
                crate::player::ForceMode::Impulse,
                false,
                false,
                0.0,
            );
            let force = self.projectiles[i].force;
            self.take_force(
                pi,
                fwd * (clamp(mass / 100.0, 0.0, 1.0) * force),
                crate::player::ForceMode::Impulse,
                false,
                false,
                0.0,
            );
        }
        self.events.push(EventKind::ProjectileHit {
            id,
            point,
            normal,
            damage: dealt,
            target,
            blocked: false,
        });
        // 7. Hit-spawned objects.
        if self.projectiles[i].allowed_to_spawn {
            let shake = self.projectiles[i].shake;
            self.events.shake(
                fwd * if matches!(target, HitTarget::Player(_)) {
                    shake * 3.0
                } else {
                    shake
                },
            );
            let spawns = self.projectiles[i].spawns.clone();
            let dmg = self.projectiles[i].damage;
            for e in spawns.iter() {
                self.spawn_hit_objects(content, e, point, normal, fwd, dmg, target, owner, blocked);
            }
            self.projectiles[i].pos = point + normal * 0.01;
        }
        // 8. Push props.
        if let HitTarget::Prop(pp) = target {
            let pr = &self.projectiles[i];
            if pr.can_push_box
                && let Some(mid) = self.map.current
            {
                let def = self.physics.props[pp as usize].def;
                if let Some(push) = content.map(mid).props[def as usize].pushable {
                    let j =
                        fwd * ((pr.force * 0.5 + pr.damage * 100.0) * push.bullet_push_multiplier);
                    self.physics.impulse_at(pp as usize, j, point);
                }
            }
        }
        // 9. Hit effects sorted by priority: drill (100) first, then the
        // reflect effect (root), then the add-ons in child order.
        let mut survive = false;
        if !self.projectiles[i].addons.is_empty() {
            let dt = DT * self.time.scale();
            match self.addons_drill_hit(i, point, target, dt) {
                Some(HitResult::HasToReturnNow) => return true,
                Some(HitResult::HasToReturn) => survive = true,
                _ => {}
            }
        }
        match self.reflect_effect(content, i, point, normal, target) {
            HitResult::HasToReturnNow => return true,
            HitResult::HasToReturn => survive = true,
            HitResult::CanContinue => {}
        }
        if !self.projectiles[i].addons.is_empty() {
            self.addons_hit(content, i, point, target);
        }
        if !survive {
            // 10. Destroy.
            return false;
        }
        true
    }

    /// The reflect (bounce) hit effect (16.1).
    fn reflect_effect(
        &mut self,
        content: &Content,
        i: usize,
        point: V2,
        normal: V2,
        target: HitTarget,
    ) -> HitResult {
        let p = &mut self.projectiles[i];
        if matches!(target, HitTarget::Player(_)) {
            p.reflects -= 10;
        }
        if p.reflects <= 0 {
            return HitResult::CanContinue;
        }
        // Reflect hooks (target bounce, 20.4).
        if !p.addons.is_empty() {
            self.addons_on_reflect(content, i, normal);
        }
        let p = &mut self.projectiles[i];
        p.vel = p.vel.reflect(normal) * p.speed_m_on_bounce;
        p.damage *= p.dmg_m_on_bounce;
        p.shake *= p.dmg_m_on_bounce;
        if p.dmg_m_on_bounce > 1.0 {
            p.dmg_m_on_bounce = (p.dmg_m_on_bounce - 1.0) * 0.6 + 1.0;
        }
        p.since_bounce_real = 0.0;
        p.pos = point + p.vel.normalized() * 0.1;
        p.reflects -= 1;
        let id = p.id;
        self.events
            .push(EventKind::ProjectileBounced { id, point, normal });
        self.events.sound(cue::IMPACT_BOUNCE, point, 1.0);
        HitResult::HasToReturn
    }

    /// Block handling (13). Returns false if destroyed.
    fn projectile_blocked(
        &mut self,
        content: &Content,
        i: usize,
        blocker: usize,
        point: V2,
    ) -> bool {
        let t = ticks(self.projectiles[i].hold_player_for);
        let p = &mut self.projectiles[i];
        if let Some(o) = p.owner {
            p.held.remove(o);
        }
        p.hold(blocker as u8, t);
        p.vel = -p.vel;
        p.age_real = f32::INFINITY;
        p.since_reflect = 0.0;
        let (id, fwd, owner, destroy) = (p.id, p.forward(), p.owner, p.destroy_on_block);
        self.players[blocker].block.since_block = 0.0;
        if owner != Some(blocker as u8) {
            self.fire_hook(
                content,
                blocker,
                Hook::BlockedProjectile {
                    projectile: id,
                    forward: fwd,
                    point,
                },
            );
        }
        self.events.push(EventKind::ProjectileBlocked {
            id,
            player: blocker as u8,
            point,
        });
        self.events.sound(cue::BLOCK_BLOCKED, point, 1.0);
        // `GameFeel(forward)`: magnitude 1 along the shot (weapons.md 13).
        self.events.shake(-fwd);
        !destroy
    }

    /// Bullet-vs-bullet surface hook (17): sweeping bullet `s` hits the
    /// collision circle of bullet `c`.
    fn bullet_surface_hook(&mut self, content: &Content, s: usize, c: usize) {
        if self.projectiles[c].dead || within_45(self.projectiles[s].vel, self.projectiles[c].vel) {
            return;
        }
        if self.projectiles[c].since_bounce_real < 0.5
            || self.projectiles[s].since_bounce_real < 0.5
        {
            return;
        }
        let (Some(cs), Some(cc)) = (self.projectiles[s].collision, self.projectiles[c].collision)
        else {
            return;
        };
        if cs.collided || cc.collided {
            return;
        }
        let (hp_s, hp_c) = (cs.hp, cc.hp);
        self.bullet_take_damage(content, c, hp_s);
        self.bullet_take_damage(content, s, hp_c);
    }

    fn bullet_take_damage(&mut self, content: &Content, j: usize, x: f32) {
        let Some(mut bc) = self.projectiles[j].collision else {
            return;
        };
        if bc.collided {
            return;
        }
        bc.hp -= x;
        let dead = bc.hp < bc.threshold;
        if dead {
            bc.collided = true;
        }
        self.projectiles[j].collision = Some(bc);
        if !dead {
            return;
        }
        let (pos, fwd, id) = (
            self.projectiles[j].pos,
            self.projectiles[j].forward(),
            self.projectiles[j].id,
        );
        self.events.push(EventKind::ProjectileKilled { id, pos });
        self.events.sound(cue::IMPACT_BULLET_ON_BULLET, pos, 1.0);
        let alive = self.projectile_hit(content, j, pos, -fwd, HitTarget::Nothing, true);
        if !alive {
            self.projectiles[j].dead = true;
        } else if let Some(bc) = self.projectiles[j].collision.as_mut() {
            // Survived by bouncing: the circle stays spent.
            bc.collided = true;
        }
    }

    /// Screen-edge bounce (20.5).
    pub(crate) fn screen_edge(&mut self, content: &Content, i: usize) {
        let b = content.tuning.bounds_half;
        let p = &self.projectiles[i];
        let (x, y) = (p.pos.x, p.pos.y);
        let (edge, n) = if x <= -b.x {
            (1i8, V2::RIGHT)
        } else if x >= b.x {
            (2, V2::LEFT)
        } else if y <= -b.y {
            (3, V2::UP)
        } else if y >= b.y {
            (4, V2::DOWN)
        } else {
            self.projectiles[i].last_edge = 0;
            return;
        };
        if p.vel.dot(n) >= 0.0 || (p.last_edge == edge && p.vel.dot(n) >= 0.0) {
            return;
        }
        if p.reflects <= 0 {
            self.projectiles[i].screen_edge_bounce = false;
            return;
        }
        let point = v2(x.clamp(-b.x, b.x), y.clamp(-b.y, b.y));
        self.projectiles[i].last_edge = edge;
        let _ = self.reflect_effect(content, i, point, n, HitTarget::Nothing);
        self.projectiles[i].last_pos = self.projectiles[i].pos;
    }
}
