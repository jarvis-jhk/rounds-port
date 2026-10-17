//! Bullets (trails), effect objects, and the particles/sounds triggered by
//! sim events (weapons.md 23, player.md 19, cards.md 9).
//!
//! The import has no particle-system data for prefabs yet, so hit sparks,
//! explosions, death bursts, jump/land dust and field effects are
//! procedural approximations of the original's prefabs (colours and sizes
//! from the specs and the prefab trees); effect objects that carry
//! sprites (saws, shields, rings) draw their imported sprites, with node
//! activity from the sim.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::{Affine2, Vec2};
use rounds_render::{Color, DrawList, Gpu, Layer, Primitive, Renderer};
use rounds_sim::events::HitTarget;
use rounds_sim::objects::{Attach, ObjKind};
use rounds_sim::{Content, Event, EventKind as K, ObjectId, World};

use super::assets::{PlayAssets, SpriteRef};
use super::fx::{Particle, Particles, Rng, Shape};
use super::player_view::v;
use super::sound::Sounds;
use crate::audio::Audio;

/// Trail of one bullet (`Trail_Bullet_Base`: time 0.03 s, width 0.3,
/// width curve 1 -> 0.35 -> 0 over the first 70 % of its length).
struct Trail {
    pts: VecDeque<(f32, Vec2)>,
    color: Color,
    width: f32,
    /// Real time the bullet died (trail fades out), None = alive.
    dead: Option<f32>,
}

const TRAIL_TIME: f32 = 0.03;

/// Imported sprites of an effect prefab, per sim node.
#[derive(Clone, Default)]
struct PrefabLook {
    /// (node index, transform relative to the root, sprite, tint, order).
    parts: Vec<(u16, Affine2, SpriteRef, Color, i32)>,
}

#[derive(Default)]
pub struct WorldFx {
    trails: HashMap<u32, Trail>,
    time: f32,
    looks: HashMap<ObjectId, Option<PrefabLook>>,
    /// Footstep edge detection: per player, per leg, was stepping.
    stepping: Vec<[bool; 2]>,
    pending_steps: Vec<(u8, usize, Vec2, bool)>,
}

/// Effect colours by prefab name (approximations of the prefabs'
/// particle colours).
fn effect_color(name: &str) -> Color {
    let n = name.to_ascii_lowercase();
    if n.contains("frost") || n.contains("cold") || n.contains("chilling") {
        Color::rgb(0.45, 0.75, 1.0)
    } else if n.contains("heal") {
        Color::rgb(0.3, 0.85, 0.35)
    } else if n.contains("toxic") || n.contains("poison") {
        Color::rgb(0.35, 0.8, 0.2)
    } else if n.contains("static") || n.contains("emp") || n.contains("radar") {
        Color::rgb(0.55, 0.85, 1.0)
    } else if n.contains("demon") || n.contains("silence") || n.contains("abyssal") {
        Color::rgb(0.85, 0.15, 0.25)
    } else if n.contains("black") || n.contains("implode") {
        Color::rgb(0.45, 0.2, 0.8)
    } else if n.contains("supernova") || n.contains("shockwave") || n.contains("overpower") {
        Color::rgb(1.0, 0.95, 0.8)
    } else {
        Color::rgb(1.0, 0.55, 0.2)
    }
}

impl WorldFx {
    /// Loads the imported sprites of effect prefabs present in the world.
    pub fn poll(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut PlayAssets,
        world: &World,
        content: &Content,
    ) {
        for o in world.objects() {
            if self.looks.contains_key(&o.def) {
                continue;
            }
            let def = content.object(o.def);
            let look = (|| {
                if def.id.is_empty() {
                    return None;
                }
                let prefab = assets.data.prefab(&def.id).ok()?;
                let root = prefab.objects.first()?;
                let m = |o: &rounds_import::format::Object| {
                    let m = o.world.matrix;
                    Affine2::from_cols_array(&m)
                };
                let inv = m(root).inverse();
                let mut parts = Vec::new();
                for (i, ob) in prefab.objects.iter().enumerate() {
                    let Some(sr) = &ob.sprite else { continue };
                    if !sr.enabled || sr.color[3] <= 0.01 {
                        continue;
                    }
                    let Some(sid) = &sr.sprite else { continue };
                    let Some(sp) = assets.sprite_id(gpu, renderer, sid) else {
                        continue;
                    };
                    let [r, g, b, a] = sr.color;
                    parts.push((
                        i as u16,
                        inv * m(ob),
                        sp,
                        Color::rgba(r, g, b, a),
                        sr.sorting_order,
                    ));
                }
                parts.sort_by_key(|p| p.4);
                (!parts.is_empty()).then_some(PrefabLook { parts })
            })();
            self.looks.insert(o.def, look);
        }
    }

    pub fn on_event(
        &mut self,
        e: &Event,
        w: &World,
        content: &Content,
        assets: &PlayAssets,
        ps: &mut Particles,
        rng: &mut Rng,
    ) {
        let skin_of = |p: u8| {
            w.players()
                .get(p as usize)
                .map(|pl| assets.skin(pl.skin as usize))
        };
        match &e.kind {
            K::ProjectileHit {
                point,
                normal,
                target,
                damage,
                ..
            } => {
                let p = v(*point);
                let n = v(*normal);
                let n = if n.length() > 1e-4 {
                    n.normalize()
                } else {
                    Vec2::Y
                };
                let (count, col) = match target {
                    HitTarget::Player(pl) => {
                        (12, skin_of(*pl).map_or(Color::WHITE, |s| s.particle))
                    }
                    _ => (7, Color::rgb(1.0, 0.85, 0.6)),
                };
                let k = (damage / 55.0).sqrt().clamp(0.6, 3.0);
                for _ in 0..count {
                    ps.push(Particle {
                        pos: p,
                        vel: rng.cone(n, 1.1) * rng.range(8.0, 30.0) * k,
                        life: rng.range(0.15, 0.4),
                        size: (rng.range(0.08, 0.16) * k, 0.0),
                        color: (Color::rgb(1.0, 0.95, 0.85), col.with_alpha(0.0)),
                        drag: 6.0,
                        gravity: 20.0,
                        shape: Shape::Streak { stretch: 0.025 },
                        ..Default::default()
                    });
                }
                ps.push(Particle {
                    pos: p,
                    life: 0.12,
                    size: (0.5 * k, 1.2 * k),
                    color: (
                        Color::rgba(1.0, 0.95, 0.85, 0.9),
                        Color::rgba(1.0, 0.8, 0.5, 0.0),
                    ),
                    additive: true,
                    unlit: true,
                    ..Default::default()
                });
            }
            K::ProjectileKilled { pos, .. } => {
                for _ in 0..6 {
                    ps.push(Particle {
                        pos: v(*pos),
                        vel: rng.unit() * rng.range(5.0, 18.0),
                        life: rng.range(0.1, 0.3),
                        size: (0.12, 0.0),
                        color: (Color::WHITE, Color::WHITE.with_alpha(0.0)),
                        drag: 5.0,
                        shape: Shape::Streak { stretch: 0.02 },
                        ..Default::default()
                    });
                }
            }
            K::Died { player, dir, .. } | K::PhoenixDied { player, dir } => {
                let Some(pl) = w.players().get(*player as usize) else {
                    return;
                };
                let skin = assets.skin(pl.skin as usize);
                let p = v(pl.pos);
                let d = v(*dir);
                let d = if d.length() > 1e-4 {
                    d.normalize()
                } else {
                    Vec2::Y
                };
                // PlayerDeath: close burst, pieces, directed smoke.
                for i in 0..40 {
                    let piece = i < 16;
                    ps.push(Particle {
                        pos: p + rng.unit() * 0.3,
                        vel: rng.cone(d, if piece { 0.9 } else { 0.5 })
                            * rng.range(10.0, if piece { 45.0 } else { 25.0 }),
                        life: rng.range(0.4, 1.4),
                        size: (rng.range(0.25, 0.7) * pl.scale, 0.0),
                        color: (
                            skin.fill_min.lerp(skin.fill_max, rng.f()),
                            skin.color.with_alpha(0.0),
                        ),
                        drag: rng.range(2.0, 4.0),
                        gravity: if piece { 15.0 } else { 0.0 },
                        rot: rng.range(0.0, std::f32::consts::TAU),
                        spin: rng.range(-8.0, 8.0),
                        shape: if piece { Shape::Square } else { Shape::Dot },
                        ..Default::default()
                    });
                }
                ps.push(Particle {
                    pos: p,
                    life: 0.25,
                    size: (1.0, 4.0),
                    color: (skin.win_text.with_alpha(0.8), skin.win_text.with_alpha(0.0)),
                    shape: Shape::Ring { band: 0.25 },
                    unlit: true,
                    ..Default::default()
                });
            }
            K::Jumped {
                player, pos, dir, ..
            } => {
                let col = skin_of(*player).map_or(Color::WHITE, |s| s.particle);
                let d = v(*dir).normalize_or_zero();
                // JumpSmoke: 15 puffs (0.77, 0.48, 0.40), cone 30 deg.
                for _ in 0..15 {
                    ps.push(Particle {
                        pos: v(*pos),
                        vel: rng.cone(-d, 0.6) * rng.range(0.0, 12.0)
                            + Vec2::new(rng.range(-4.0, 4.0), 0.0),
                        life: rng.range(0.2, 0.8),
                        size: (rng.range(0.3, 0.8), 0.0),
                        color: (
                            Color::rgba(0.77, 0.48, 0.40, 0.6).lerp(col, 0.4),
                            Color::rgba(0.77, 0.48, 0.40, 0.0),
                        ),
                        drag: 5.0,
                        ..Default::default()
                    });
                }
            }
            K::Landed {
                pos,
                normal,
                impact,
                ..
            } => {
                if *impact < 20.0 {
                    return;
                }
                let k = (impact / 40.0).clamp(0.5, 1.0);
                let n = v(*normal).normalize_or(Vec2::Y);
                for _ in 0..(25.0 * k) as i32 {
                    let side = if rng.f() < 0.5 { -1.0 } else { 1.0 };
                    let tangent = Vec2::new(n.y, -n.x) * side;
                    ps.push(Particle {
                        pos: v(*pos),
                        vel: (tangent + n * rng.range(0.0, 0.6)) * rng.range(2.0, 18.0) * k,
                        life: rng.range(0.2, 1.0),
                        size: (rng.range(0.3, 0.8) * k, 0.0),
                        color: (
                            Color::rgba(0.77, 0.48, 0.40, 0.5),
                            Color::rgba(0.77, 0.48, 0.40, 0.0),
                        ),
                        drag: 4.0,
                        ..Default::default()
                    });
                }
            }
            K::Explosion { pos, radius, def } => {
                let name = def.map_or("", |d| content.object(d).name.as_str());
                explosion(ps, rng, v(*pos), *radius, effect_color(name));
            }
            K::Healed { player, amount } if *amount > 1.0 => {
                if let Some(pl) = w.players().get(*player as usize) {
                    for _ in 0..(*amount as i32 / 5).clamp(1, 10) {
                        ps.push(Particle {
                            pos: v(pl.pos) + rng.unit() * 0.5,
                            vel: rng.cone(Vec2::Y, 1.0) * rng.range(5.0, 10.0),
                            life: rng.range(0.2, 0.7),
                            size: (0.3, 0.1),
                            color: (
                                Color::rgb(0.26, 0.59, 0.29),
                                Color::rgba(0.26, 0.59, 0.29, 0.0),
                            ),
                            drag: 4.0,
                            shape: Shape::Square,
                            ..Default::default()
                        });
                    }
                }
            }
            K::Slowed {
                player, particles, ..
            } => {
                if let Some(pl) = w.players().get(*player as usize) {
                    for _ in 0..(*particles).clamp(1, 50) {
                        ps.push(Particle {
                            pos: v(pl.pos) + rng.unit() * 0.6,
                            vel: rng.unit() * rng.range(0.5, 2.0),
                            life: rng.range(2.0, 3.0) * 0.5,
                            size: (rng.range(0.2, 0.7) * 0.5, 0.0),
                            color: (
                                Color::rgb(0.19, 0.31, 0.46),
                                Color::rgba(0.19, 0.31, 0.46, 0.0),
                            ),
                            gravity: 2.0,
                            rot: rng.range(0.0, std::f32::consts::TAU),
                            shape: Shape::Square,
                            ..Default::default()
                        });
                    }
                }
            }
            K::BlockStarted { player, .. } => {
                if let Some(pl) = w.players().get(*player as usize) {
                    ps.push(Particle {
                        pos: v(pl.pos),
                        life: 0.4,
                        size: (2.2 * pl.scale * 0.6, 2.2 * pl.scale),
                        color: (
                            Color::rgba(1.0, 1.0, 1.0, 0.9),
                            Color::rgba(1.0, 1.0, 1.0, 0.0),
                        ),
                        shape: Shape::Ring { band: 0.12 },
                        unlit: true,
                        ..Default::default()
                    });
                }
            }
            K::OutOfBounds {
                pos,
                inward,
                shielded,
                ..
            } => {
                let c = if *shielded {
                    (Color::rgb(0.79, 0.79, 0.79), Color::rgb(0.63, 0.63, 0.63))
                } else {
                    (Color::rgb(0.79, 0.27, 0.27), Color::rgb(0.6, 0.21, 0.21))
                };
                let n = v(*inward).normalize_or(Vec2::Y);
                for _ in 0..30 {
                    ps.push(Particle {
                        pos: v(*pos),
                        vel: rng.cone(n, 1.2) * rng.range(5.0, 30.0),
                        life: rng.range(0.3, 0.9),
                        size: (rng.range(0.2, 0.5), 0.0),
                        color: (c.0, c.1.with_alpha(0.0)),
                        drag: 3.0,
                        shape: Shape::Square,
                        rot: rng.range(0.0, std::f32::consts::TAU),
                        ..Default::default()
                    });
                }
            }
            K::Teleported { from, to, player } => {
                let col = skin_of(*player).map_or(Color::WHITE, |s| s.win_text);
                for p in [v(*from), v(*to)] {
                    ps.push(Particle {
                        pos: p,
                        life: 0.35,
                        size: (2.5, 0.2),
                        color: (col, col.with_alpha(0.0)),
                        shape: Shape::Ring { band: 0.2 },
                        unlit: true,
                        ..Default::default()
                    });
                }
            }
            K::PropDestroyed { pos, .. } => {
                for _ in 0..14 {
                    ps.push(Particle {
                        pos: v(*pos) + rng.unit(),
                        vel: rng.unit() * rng.range(3.0, 12.0),
                        life: rng.range(0.3, 0.8),
                        size: (rng.range(0.3, 0.8), 0.0),
                        color: (
                            Color::rgba(0.6, 0.6, 0.6, 0.6),
                            Color::rgba(0.5, 0.5, 0.5, 0.0),
                        ),
                        drag: 3.0,
                        ..Default::default()
                    });
                }
            }
            K::SawHit { pos, dir } => {
                for _ in 0..10 {
                    ps.push(Particle {
                        pos: v(*pos),
                        vel: rng.cone(v(*dir).normalize_or(Vec2::Y), 0.8) * rng.range(10.0, 30.0),
                        life: rng.range(0.1, 0.3),
                        size: (0.1, 0.0),
                        color: (Color::rgb(1.0, 0.9, 0.5), Color::rgba(1.0, 0.5, 0.2, 0.0)),
                        shape: Shape::Streak { stretch: 0.02 },
                        ..Default::default()
                    });
                }
            }
            K::ObjectSpawned {
                def, pos, scale, ..
            } => {
                let name = &content.object(*def).name;
                let n = name.to_ascii_lowercase();
                // Visual-only spawns that have no explosion: rings / puffs.
                if n.contains("poof") || n.contains("spark") {
                    for _ in 0..8 {
                        ps.push(Particle {
                            pos: v(*pos),
                            vel: rng.unit() * rng.range(3.0, 10.0) * scale.max(0.3),
                            life: rng.range(0.2, 0.5),
                            size: (0.25, 0.0),
                            color: (effect_color(name), effect_color(name).with_alpha(0.0)),
                            drag: 4.0,
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {}
        }
    }

    pub fn update(&mut self, world: &World, real: f32, _game: f32) {
        self.time += real;
        let t = self.time;
        for tr in self.trails.values_mut() {
            if tr.dead.is_none() {
                tr.dead = Some(t);
            }
        }
        for p in world.projectiles() {
            if p.dead {
                continue;
            }
            let tr = self.trails.entry(p.id).or_insert_with(|| Trail {
                pts: VecDeque::new(),
                color: Color::WHITE,
                width: 0.3,
                dead: None,
            });
            tr.dead = None;
            let owner = p.owner.and_then(|o| world.players().get(o as usize));
            tr.color = owner.map_or(Color::rgb(1.0, 0.379, 0.118), |o| {
                let c = o.gun.stats.projectile_color;
                if c.is_black() {
                    Color::rgb(-1.0, o.skin as f32, 0.0) // resolved at draw
                } else {
                    Color::rgba(c.r, c.g, c.b, 1.0)
                }
            });
            tr.width = 0.3 * (p.damage / 55.0).max(0.05).sqrt().clamp(0.4, 4.0);
            tr.pts.push_back((t, v(p.pos)));
            while tr.pts.len() > 2 && t - tr.pts[1].0 > TRAIL_TIME + 0.02 {
                tr.pts.pop_front();
            }
        }
        self.trails
            .retain(|_, tr| tr.dead.is_none_or(|d| t - d < TRAIL_TIME + 0.02));
        // Footsteps (player.md 18.7): a step starts while grounded.
        let n = world.players().len();
        self.stepping.resize(n, [false; 2]);
        for (i, p) in world.players().iter().enumerate() {
            for k in 0..2 {
                let s = p.legs[k].stepping;
                if s && !self.stepping[i][k] && p.grounded && p.alive() && !p.wall_grab {
                    self.pending_steps
                        .push((i as u8, k, v(p.pos), p.scale > 1.5));
                }
                self.stepping[i][k] = s;
            }
        }
    }

    /// Footstep sounds collected by `update`.
    pub fn take_steps(&mut self) -> Vec<(u8, usize, Vec2, bool)> {
        std::mem::take(&mut self.pending_steps)
    }

    pub fn draw_projectiles(
        &self,
        cur: &World,
        prev: &World,
        alpha: f32,
        assets: &PlayAssets,
        list: &mut DrawList,
    ) {
        let now = self.time;
        for (id, tr) in &self.trails {
            let head = cur
                .projectiles()
                .iter()
                .find(|p| p.id == *id && !p.dead)
                .map(|p| {
                    let q = prev.projectiles().iter().find(|q| q.id == p.id);
                    q.map_or(v(p.pos), |q| v(q.pos).lerp(v(p.pos), alpha))
                });
            let color = if tr.color.r < 0.0 {
                let s = assets.skin(tr.color.g as usize);
                // Bright team colour (the trail's default is a bright orange).
                s.win_text.lerp(Color::WHITE, 0.15)
            } else {
                tr.color
            };
            let fade = tr
                .dead
                .map_or(1.0, |d| (1.0 - (now - d) / (TRAIL_TIME + 0.02)).max(0.0));
            let mut pts: Vec<Vec2> = tr.pts.iter().rev().map(|p| p.1).collect();
            if let Some(h) = head {
                pts.insert(0, h);
            }
            if pts.len() < 2 {
                if let Some(h) = head {
                    list.circle(Layer::World, 3_000_000, h, tr.width * 0.5, color);
                }
                continue;
            }
            // Cut the polyline to the trail length (TRAIL_TIME of travel).
            let speed = (pts[0] - pts[1]).length() / (crate::play::DT.max(1e-4));
            let max_len = (speed * TRAIL_TIME * 2.2).clamp(0.6, 6.0);
            let mut acc = 0.0;
            let mut poly = vec![pts[0]];
            for w2 in pts.windows(2) {
                let seg = (w2[1] - w2[0]).length();
                if acc + seg >= max_len {
                    let f = (max_len - acc) / seg.max(1e-5);
                    poly.push(w2[0].lerp(w2[1], f));
                    break;
                }
                acc += seg;
                poly.push(w2[1]);
            }
            let total: f32 = poly.windows(2).map(|w2| (w2[1] - w2[0]).length()).sum();
            if total < 1e-3 {
                continue;
            }
            // Tapered ribbon: width 1 at the head to 0 at the tail.
            let mut left = Vec::new();
            let mut right = Vec::new();
            let mut s = 0.0;
            for i in 0..poly.len() {
                let d = if i + 1 < poly.len() {
                    poly[i + 1] - poly[i]
                } else {
                    poly[i] - poly[i - 1]
                }
                .normalize_or(Vec2::X);
                if i > 0 {
                    s += (poly[i] - poly[i - 1]).length();
                }
                let t = s / total;
                let w = tr.width * 0.5 * (1.0 - t).max(0.0).powf(0.8);
                let nrm = Vec2::new(-d.y, d.x) * w;
                left.push(poly[i] + nrm);
                right.push(poly[i] - nrm);
            }
            right.reverse();
            left.extend(right);
            list.push(
                Layer::World,
                3_000_000,
                Primitive::Polygon {
                    points: left,
                    color: color.with_alpha(fade),
                },
            );
            if head.is_some() {
                list.circle(
                    Layer::World,
                    3_000_001,
                    poly[0],
                    tr.width * 0.5,
                    color.lerp(Color::WHITE, 0.4),
                );
            }
        }
    }

    /// Effect objects: imported sprites (by node activity) and procedural
    /// fields/rings for the prefabs that are only particles.
    pub fn draw_objects(
        &self,
        cur: &World,
        prev: &World,
        alpha: f32,
        content: &Content,
        assets: &PlayAssets,
        list: &mut DrawList,
    ) {
        for o in cur.objects() {
            if o.dead {
                continue;
            }
            let def = content.object(o.def);
            let q = prev.objects().iter().find(|q| q.id == o.id);
            let pos = q.map_or(v(o.pos), |q| v(q.pos).lerp(v(o.pos), alpha));
            let owner = match o.kind {
                ObjKind::PlayerEffect { owner } => Some(owner),
                _ => o.spawner,
            };
            let owner_alive = owner
                .and_then(|p| cur.players().get(p as usize))
                .is_some_and(|p| p.alive() && p.pos.y < 150.0);
            if matches!(o.kind, ObjKind::PlayerEffect { .. }) && !owner_alive {
                continue;
            }
            let pos = match (o.attach, owner) {
                (Attach::Player(p, off), _) => cur
                    .players()
                    .get(p as usize)
                    .map_or(pos, |pl| v(pl.pos) + v(off)),
                _ => pos,
            };
            let ang = o.dir.y.atan2(o.dir.x) - std::f32::consts::FRAC_PI_2;
            let tf = Affine2::from_scale_angle_translation(Vec2::splat(o.scale), ang, pos);
            let skin_col = owner
                .and_then(|p| cur.players().get(p as usize))
                .map(|p| assets.skin(p.skin as usize));
            let mut drew = false;
            if let Some(Some(look)) = self.looks.get(&o.def) {
                for (node, rel, sp, tint, order) in &look.parts {
                    if !cur.object_node_active(content, o.id, *node) {
                        continue;
                    }
                    list.sprite(Layer::World, 1_500_000 + order, sp.sprite(tf * *rel, *tint));
                    drew = true;
                }
            }
            let name = def.name.to_ascii_lowercase();
            let col = effect_color(&def.name);
            let t = o.age;
            // Countdown/charge rings (Abyssal Countdown, Healing Field).
            if let Some(f) = cur.effect_progress(content, o.id) {
                let r = 2.0 * o.scale.max(0.5);
                list.ring(
                    Layer::WorldUnlit,
                    1_600_000,
                    pos,
                    r,
                    0.12,
                    std::f32::consts::FRAC_PI_2,
                    std::f32::consts::TAU * f.clamp(0.0, 1.0),
                    col.with_alpha(0.8),
                );
            }
            if drew || matches!(o.kind, ObjKind::PlayerEffect { .. }) {
                continue;
            }
            let r = o.scale.max(0.2);
            if name.contains("blackhole") || name.contains("black_hole") {
                list.circle(
                    Layer::World,
                    1_500_000,
                    pos,
                    1.2 * r,
                    Color::rgb(0.05, 0.02, 0.1),
                );
                for i in 0..3 {
                    let a = t * 4.0 + i as f32 * 2.1;
                    list.ring(
                        Layer::WorldUnlit,
                        1_500_001,
                        pos,
                        (1.6 + 0.6 * i as f32) * r,
                        0.08,
                        a,
                        2.0,
                        col.with_alpha(0.7),
                    );
                }
            } else if name.contains("heal")
                || name.contains("static")
                || name.contains("field")
                || name.contains("toxic")
                || name.contains("chilling")
                || name.contains("radiance")
            {
                let rr = 3.5 * r;
                list.circle(Layer::WorldUnlit, 1_500_000, pos, rr, col.with_alpha(0.12));
                list.ring(
                    Layer::WorldUnlit,
                    1_500_001,
                    pos,
                    rr,
                    0.1,
                    0.0,
                    std::f32::consts::TAU,
                    col.with_alpha(0.6),
                );
            } else if name.contains("saw") {
                let rr = 1.5 * r;
                let c = skin_col.map_or(Color::rgb(0.7, 0.7, 0.7), |s| s.color);
                let teeth = 12;
                let pts: Vec<Vec2> = (0..teeth * 2)
                    .map(|i| {
                        let a = t * 12.0 + i as f32 * std::f32::consts::PI / teeth as f32;
                        let rad = if i % 2 == 0 { rr } else { rr * 0.78 };
                        pos + Vec2::from_angle(a) * rad
                    })
                    .collect();
                list.polygon(Layer::World, 1_500_000, &pts, c);
                list.circle(
                    Layer::World,
                    1_500_001,
                    pos,
                    rr * 0.3,
                    Color::rgb(0.15, 0.15, 0.15),
                );
            } else if name.contains("bomb") || name.contains("timed") {
                list.circle(
                    Layer::World,
                    1_500_000,
                    pos,
                    0.35 * r,
                    Color::rgb(0.15, 0.15, 0.15),
                );
                let blink = ((t * 8.0) as i32) % 2 == 0;
                if blink {
                    list.circle(
                        Layer::WorldUnlit,
                        1_500_001,
                        pos,
                        0.15 * r,
                        Color::rgb(1.0, 0.3, 0.2),
                    );
                }
            }
        }
    }
}

/// A procedural explosion: flash, ring, fire puffs, smoke and sparks.
fn explosion(ps: &mut Particles, rng: &mut Rng, p: Vec2, radius: f32, col: Color) {
    let r = radius.max(0.5);
    ps.push(Particle {
        pos: p,
        life: 0.12,
        size: (r * 1.2, r * 2.0),
        color: (Color::rgba(1.0, 0.97, 0.9, 0.95), col.with_alpha(0.0)),
        unlit: true,
        ..Default::default()
    });
    ps.push(Particle {
        pos: p,
        life: 0.3,
        size: (r * 0.6, r * 2.2),
        color: (
            col.lerp(Color::WHITE, 0.5).with_alpha(0.9),
            col.with_alpha(0.0),
        ),
        shape: Shape::Ring { band: 0.15 },
        unlit: true,
        ..Default::default()
    });
    let n = (10.0 + r * 4.0).min(40.0) as i32;
    for _ in 0..n {
        ps.push(Particle {
            pos: p + rng.unit() * rng.range(0.0, r * 0.3),
            vel: rng.unit() * rng.range(r * 1.0, r * 5.0),
            life: rng.range(0.3, 0.8),
            size: (rng.range(0.3, 0.6) * r, 0.0),
            color: (
                col.lerp(Color::rgb(1.0, 0.9, 0.6), 0.5),
                Color::rgba(0.2, 0.2, 0.2, 0.0),
            ),
            drag: 5.0,
            ..Default::default()
        });
    }
    for _ in 0..n / 2 {
        ps.push(Particle {
            pos: p,
            vel: rng.unit() * rng.range(r * 6.0, r * 14.0),
            life: rng.range(0.2, 0.5),
            size: (0.12, 0.0),
            color: (Color::rgb(1.0, 0.95, 0.7), col.with_alpha(0.0)),
            drag: 4.0,
            gravity: 15.0,
            shape: Shape::Streak { stretch: 0.02 },
            ..Default::default()
        });
    }
}

/// Keys of looping sounds that may keep playing: effect objects alive.
pub struct LoopAlive {
    objects: HashSet<u32>,
}

impl LoopAlive {
    pub fn contains(&self, k: &u32) -> bool {
        self.objects.contains(&(k / 64))
    }
}

pub fn loop_keys(world: &World) -> LoopAlive {
    LoopAlive {
        objects: world
            .objects()
            .iter()
            .filter(|o| !o.dead)
            .map(|o| o.id)
            .collect(),
    }
}

/// Sounds for an event: sim cues as they are, plus the presentation-side
/// gun/impact logic (audio.md 5) and UI sequences.
pub fn sound_for_event(
    e: &Event,
    w: &World,
    content: &Content,
    sounds: &mut Sounds,
    audio: &mut Audio,
) {
    let card_ids = |p: u8| -> Vec<&str> {
        w.players().get(p as usize).map_or(vec![], |pl| {
            pl.cards
                .iter()
                .map(|c| content.card(*c).id.as_str())
                .collect()
        })
    };
    match &e.kind {
        K::Sound { cue, pos, .. } => {
            let x = (!cue.starts_with("UI_")).then_some(pos.x);
            sounds.play(audio, cue, x, 0.0);
        }
        K::LoopSound {
            cue,
            key,
            pos,
            start,
        } => sounds.set_loop(audio, *key, cue, pos.x, *start),
        K::ShotFired {
            player,
            projectiles,
            pos,
            ..
        } => {
            let slot = if *projectiles > 1 {
                "shotgun"
            } else {
                "single"
            };
            let evs = sounds.modifier_events(&card_ids(*player), false, slot, 3);
            let offset = match evs.len() {
                0 | 1 => 0.0,
                2 => -3.0,
                _ => -6.0,
            };
            for ev in evs {
                sounds.play(audio, &ev, Some(pos.x), offset);
            }
        }
        K::ProjectileHit {
            id,
            point,
            target,
            damage,
            ..
        } => {
            let owner = w
                .projectiles()
                .iter()
                .find(|p| p.id == *id)
                .and_then(|p| p.owner);
            let ids = owner.map(card_ids).unwrap_or_default();
            let slot = match target {
                HitTarget::Player(_) => "impactCharacter",
                HitTarget::Map | HitTarget::Prop(_) => "impactEnvironment",
                _ => "",
            };
            if !slot.is_empty() {
                let evs = sounds.modifier_events(&ids, true, slot, 2);
                let offset = if evs.len() > 1 { -3.0 } else { 0.0 };
                for ev in evs {
                    sounds.play(audio, &ev, Some(point.x), offset);
                }
            }
            if *damage / 55.0 > 8.0 {
                sounds.play(
                    audio,
                    "SFX_Impact_Damage_To_Explosion_Huge_SE",
                    Some(point.x),
                    0.0,
                );
            } else if *damage / 55.0 > 2.6 {
                sounds.play(
                    audio,
                    "SFX_Impact_Damage_To_Explosion_Medium_SE",
                    Some(point.x),
                    0.0,
                );
            }
        }
        K::ReloadStarted { .. } => {}
        K::PropImpact { pos, impulse, .. } if *impulse >= 200_000.0 => {
            sounds.play(audio, "SFX_Environment_Box_Impact_SE", Some(pos.x), 0.0)
        }
        K::ObjectSpawned { def, pos, .. } => {
            // Block-effect spawn cues named after the prefab (audio.md 6.4).
            let name = content.object(*def).name.as_str();
            let cue = match name {
                "A_FrostSlam" | "E_FrostSlam" => Some("SFX_Block_Frost_Slam_Spawn_SE"),
                "E_Implode" | "A_Implode" => Some("SFX_Block_Implode_Spawn_SE"),
                "E_Silence" => Some("SFX_Block_Silence_Spawn_SE"),
                "E_SupernovaFirst" => Some("SFX_Block_Supernova_Charge_SE"),
                "E_SupernovaSecond" => Some("SFX_Block_Supernova_Explosion_SE"),
                "A_EMP" => Some("SFX_Block_EMP_Spawn_SE"),
                "A_Radar" => Some("SFX_Block_Radar_Spawn_SE"),
                "A_TacticalReload" => Some("SFX_Block_Tactical_Reload_Spawn_SE"),
                "A_OverPower" | "E_Overpower" => Some("SFX_Block_Overpower_Activate_SE"),
                _ => None,
            };
            if let Some(c) = cue {
                sounds.play(audio, c, Some(pos.x), 0.0);
            }
        }
        _ => {}
    }
}
