//! Drawing a player the way the original looks (player.md 18): the
//! team-coloured body disc filled with two-tone brush strokes (clipped to
//! the disc by a per-player stencil value), the default face mirrored to
//! the movement direction, IK legs that step and arms that hold the gun,
//! the two-part gun, the block orb, the health bar with its lagging bar,
//! status icons, damage flash and the spawn-in scale.
//!
//! Everything here is presentation state, updated once per tick from the
//! sim's player state and events.

use glam::{Affine2, Vec2};
use rounds_render::{Color, DrawList, Layer, Mask, Primitive};
use rounds_sim::player::Player;

use super::assets::{PlayAssets, Skin};
use super::fx::{Key, Rng, curve};

pub fn v(p: rounds_sim::V2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// One brush stroke of the body fill.
#[derive(Clone, Copy, Debug)]
struct Stroke {
    local: Vec2,
    rot: f32,
    size: f32,
    color: Color,
    age: f32,
    life: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Foot {
    pos: Vec2,
    from: Vec2,
    to: Vec2,
    stepping: bool,
    t: f32,
}

/// Presentation state of one player.
pub struct PlayerVis {
    feet: [Foot; 2],
    idle_hand: Vec2,
    idle_vel: Vec2,
    raise: f32,
    bar_pos: Vec2,
    bar_vel: Vec2,
    hp: f32,
    hp_vel: f32,
    lag: f32,
    lag_vel: f32,
    since_damage: f32,
    /// Face side: +1 right, -1 left.
    facing: f32,
    /// Gun mirror spring (0 = right side layout, 1 = left).
    mirror: f32,
    mirror_vel: f32,
    strokes: Vec<Stroke>,
    emit: f32,
    blink: Option<Color>,
    blink_cd: f32,
    /// Spawn-in scale animation time (0.4 s), None = done.
    spawn_t: Option<f32>,
    was_alive: bool,
    init: bool,
    stun_t: f32,
    stun_scale: f32,
    pub block_flash: f32,
    rng: Rng,
}

/// Ease-out curve of the spawn-in scale (reaches 1 at 0.18 s of 0.4 s).
const SPAWN_KEYS: [Key; 3] = [
    Key(0.0, 0.0, 0.0, 8.0),
    Key(0.18, 1.0, 0.0, 0.0),
    Key(0.4, 1.0, 0.0, 0.0),
];
/// IkLeg step curves (player.md 18.7).
const STEP_FORWARD: [Key; 3] = [
    Key(0.0, 0.0, -0.076, -0.076),
    Key(0.498, 0.498, 1.534, 1.534),
    Key(1.0, 1.0, -0.03, -0.03),
];
const STEP_UP: [Key; 3] = [
    Key(0.0, 0.0, 2.0, 2.0),
    Key(0.5, 0.5, 0.0, 0.0),
    Key(1.0, 0.0, -2.0, -2.0),
];

impl PlayerVis {
    pub fn new(id: usize) -> PlayerVis {
        PlayerVis {
            feet: [Foot::default(); 2],
            idle_hand: Vec2::ZERO,
            idle_vel: Vec2::ZERO,
            raise: 0.0,
            bar_pos: Vec2::ZERO,
            bar_vel: Vec2::ZERO,
            hp: 1.0,
            hp_vel: 0.0,
            lag: 1.0,
            lag_vel: 0.0,
            since_damage: 10.0,
            facing: if id.is_multiple_of(2) { 1.0 } else { -1.0 },
            mirror: 0.0,
            mirror_vel: 0.0,
            strokes: Vec::new(),
            emit: 0.0,
            blink: None,
            blink_cd: 0.0,
            spawn_t: None,
            was_alive: false,
            init: false,
            stun_t: 0.0,
            stun_scale: 0.0,
            block_flash: 0.0,
            rng: Rng::new(0x51ed + id as u64),
        }
    }

    /// Damage flash (`BlinkColor`, white * 0.85), throttled to 0.1 s.
    pub fn blink(&mut self, c: Color) {
        if self.blink_cd > 0.0 {
            return;
        }
        self.blink_cd = 0.1;
        for s in &mut self.strokes {
            s.color = c;
        }
        self.blink = Some(c);
    }

    pub fn damaged(&mut self) {
        self.since_damage = 0.0;
        self.blink(Color::rgb(0.85, 0.85, 0.85));
    }

    pub fn spawned(&mut self) {
        self.spawn_t = Some(0.0);
    }

    pub fn block(&mut self) {
        self.raise = 0.3;
        self.idle_vel.y += 20.0;
        self.block_flash = 0.4;
    }

    /// One tick: `real` = 0.014, `game` = 0.014 * time scale.
    /// `plants`: the sim's ground hits under each hip (`World::foot_plants`).
    pub fn update(
        &mut self,
        p: &Player,
        plants: [Option<rounds_sim::view::FootPlant>; 2],
        skin: &Skin,
        real: f32,
        game: f32,
    ) {
        let s = p.scale;
        let pos = v(p.pos);
        let vel = v(p.vel);
        if !self.init {
            self.init = true;
            self.bar_pos = pos;
            self.idle_hand = pos + Vec2::new(-self.facing, 0.0) * s;
            for (k, f) in self.feet.iter_mut().enumerate() {
                let side = if k == 0 { -1.0 } else { 1.0 };
                f.pos = pos + Vec2::new(0.33 * side, -1.272) * 0.7 * s;
            }
        }
        let alive = p.alive() && !p.respawning;
        if alive && !self.was_alive {
            // Revived / first appearance: re-seat the limbs.
            self.init = false;
            self.was_alive = true;
            return self.update(p, plants, skin, real, game);
        }
        self.was_alive = alive;
        if let Some(t) = &mut self.spawn_t {
            *t += real;
            if *t > 0.4 {
                self.spawn_t = None;
            }
        }
        self.blink_cd -= game;
        self.since_damage += game;
        self.raise -= game;
        self.block_flash -= real;

        // Face side (player.md 18.3).
        if vel.x > 3.0 {
            self.facing = 1.0;
        } else if vel.x < -3.0 {
            self.facing = -1.0;
        }

        // Health bar anchor spring and fills (18.5).
        let dt = game.clamp(0.0, 0.03);
        let target = pos + vel * 0.02;
        self.bar_vel += (target - self.bar_pos) * dt * 800.0;
        self.bar_vel -= self.bar_vel * 20.0 * dt;
        self.bar_pos += self.bar_vel * dt;
        if (self.bar_pos - pos).length() > 5.0 {
            self.bar_pos = pos;
            self.bar_vel = Vec2::ZERO;
        }
        let frac = (p.health / p.stats.max_health.max(1.0)).clamp(0.0, 1.0);
        let chase = |cur: &mut f32, vel: &mut f32, target: f32| {
            *vel += ((target - *cur) * 25.0 - *vel) * (1.0 - (-25.0 * dt).exp());
            *cur += *vel * dt;
        };
        chase(&mut self.hp, &mut self.hp_vel, frac);
        if self.since_damage > 0.5 || self.lag < self.hp {
            chase(
                &mut self.lag,
                &mut self.lag_vel,
                frac.max(self.hp.min(frac)),
            );
        }

        // Gun mirror (RightLeftMirrorSpring: drag 15, spring 35).
        let left = p.gun_pos.x - 0.1 < p.pos.x;
        let mt = if left { 1.0 } else { 0.0 };
        self.mirror_vel +=
            ((mt - self.mirror) * 35.0 - self.mirror_vel) * (1.0 - (-15.0 * game).exp());
        self.mirror += self.mirror_vel * game;

        // Body fill strokes (18.2): 250/s, life 0.3 s, size 1.5..2.
        for st in &mut self.strokes {
            st.age += game;
        }
        self.strokes.retain(|st| st.age < st.life);
        self.emit += game * 250.0;
        while self.emit >= 1.0 {
            self.emit -= 1.0;
            let r = self.rng.f();
            let color = self
                .blink
                .filter(|_| self.blink_cd > 0.0)
                .unwrap_or(skin.fill_min.lerp(skin.fill_max, r));
            self.strokes.push(Stroke {
                local: Vec2::new(self.rng.range(-0.6, 0.6), self.rng.range(-0.6, 0.6)),
                rot: self.rng.range(0.0, std::f32::consts::TAU),
                size: self.rng.range(1.5, 2.0) * 0.55,
                color,
                age: 0.0,
                life: 0.3,
            });
        }
        if self.blink_cd <= 0.0 {
            self.blink = None;
        }

        // Idle hand (opposite the gun) with its lag (18.6, 12.7).
        let gun_side = if p.gun_pos.x > p.pos.x { 1.0 } else { -1.0 };
        let mut rest = pos + Vec2::new(-gun_side * 0.75, -0.35) * s;
        rest -= 0.3 * Vec2::new(0.3 * vel.x, vel.y) * 0.02;
        if self.raise > 0.0 {
            rest.y += 1.0;
        }
        let k = 1.0 - (-15.0 * game).exp();
        self.idle_vel = self.idle_vel.lerp((rest - self.idle_hand) * 15.0, k);
        self.idle_hand += self.idle_vel * game;
        if (self.idle_hand - rest).length() > 3.0 * s {
            self.idle_hand = rest;
        }

        // Stun icon scale.
        let st = if p.stunned { 1.0 } else { 0.0 };
        self.stun_scale += (st - self.stun_scale) * (1.0 - (-8.0 * game).exp());
        self.stun_t += game;

        // Feet (18.7), driven by the sim's leg-step state, planted on the
        // ground the sim's leg rays hit (`World::foot_plants`; slopes and
        // props included).
        let reach = 0.8 * s;
        for (k, plant) in plants.into_iter().enumerate() {
            let side = if k == 0 { -1.0 } else { 1.0 };
            let hip = pos + Vec2::new(0.2 * side, -0.3) * s;
            let f = &mut self.feet[k];
            if !p.grounded {
                let rest = pos + Vec2::new(0.33 * side, -1.272) * 0.7 * s + vel * 0.08;
                f.pos = f.pos.lerp(rest, 1.0 - (-15.0 * game).exp());
                f.stepping = false;
                f.to = f.pos;
                continue;
            }
            let leg = p.legs[k];
            // Ground height at x along the planted surface (its slope).
            let ground_at = |x: f32| match plant {
                Some(pl) => {
                    let (pt, n) = (v(pl.point), v(pl.normal));
                    let slope = if n.y.abs() > 0.2 { -n.x / n.y } else { 0.0 };
                    pt.y + (x - pt.x) * slope.clamp(-3.0, 3.0)
                }
                None => p.ground_pos.y,
            };
            let ground_y = ground_at(f.pos.x);
            if leg.stepping && !f.stepping {
                f.stepping = true;
                f.from = f.pos;
                let lead = (vel.x * 0.12).clamp(-0.9, 0.9) * s;
                let x = pos.x + 0.25 * side * s + lead;
                f.to = Vec2::new(x, ground_at(x));
            }
            if f.stepping {
                let t = if leg.stepping {
                    leg.step_time.clamp(0.0, 1.0)
                } else {
                    1.0
                };
                f.t = t;
                f.pos = f.from.lerp(f.to, curve(&STEP_FORWARD, t))
                    + Vec2::new(0.0, curve(&STEP_UP, t) * 0.5 * s / 1.2);
                if !leg.stepping {
                    f.stepping = false;
                    f.pos = f.to;
                }
            } else {
                // Planted: keep the foot unless it got lost (presentation).
                f.pos.y = f.pos.y.max(ground_y - 0.05).min(hip.y);
                if (f.pos - hip).length() > reach * 1.15 {
                    let x = pos.x + 0.25 * side * s;
                    f.pos = Vec2::new(x, ground_at(x));
                }
            }
        }
    }
}

/// Two-bone IK: the middle joint for a chain `root -> target` with bone
/// lengths `a`, `b`, bending to the `bend` side (+1 = counter-clockwise).
pub fn ik(root: Vec2, target: Vec2, a: f32, b: f32, bend: f32) -> (Vec2, Vec2) {
    let d = target - root;
    let len = d.length().clamp(1e-4, (a + b) * 0.999);
    let dir = if d.length() > 1e-5 {
        d / d.length()
    } else {
        Vec2::NEG_Y
    };
    let end = root + dir * len;
    let cos = ((a * a + len * len - b * b) / (2.0 * a * len)).clamp(-1.0, 1.0);
    let ang = cos.acos() * bend.signum();
    let mid = root + Vec2::from_angle(ang).rotate(dir) * a;
    (mid, end)
}

/// Two-bone IK choosing the bend whose middle joint lies further along
/// `prefer` (knees toward the facing side, elbows downward).
pub fn ik_toward(root: Vec2, target: Vec2, a: f32, b: f32, prefer: Vec2) -> (Vec2, Vec2) {
    let (m1, e) = ik(root, target, a, b, 1.0);
    let (m2, _) = ik(root, target, a, b, -1.0);
    if (m1 - m2).dot(prefer) >= 0.0 {
        (m1, e)
    } else {
        (m2, e)
    }
}

/// A limb drawn like `LegRenderer`: round-capped segments along the
/// quadratic Bezier `a -> mid -> b`.
pub fn limb(list: &mut DrawList, sort: i32, a: Vec2, mid: Vec2, b: Vec2, width: f32, c: Color) {
    // Control point so the curve passes through `mid` at t = 0.5.
    let ctrl = mid * 2.0 - (a + b) * 0.5;
    let n = 8;
    let mut prev = a;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let p = a * (1.0 - t) * (1.0 - t) + ctrl * 2.0 * t * (1.0 - t) + b * t * t;
        list.push(
            Layer::World,
            sort,
            Primitive::Line {
                a: prev,
                b: p,
                width,
                round: true,
                color: c,
            },
        );
        prev = p;
    }
}

/// Everything needed to draw one player this frame.
pub struct PlayerDraw<'a> {
    pub p: &'a Player,
    pub pos: Vec2,
    pub gun_pos: Vec2,
    pub vis: &'a PlayerVis,
    pub skin: Skin,
    pub index: usize,
    /// Sort band base (World layer).
    pub sort: i32,
    /// The player's face items (None = the default face).
    pub face: Option<&'a [super::assets::FaceItem]>,
    /// Imported particle effects (the skin body fill), if loaded.
    pub effects: Option<&'a super::effects::Effects>,
}

impl PlayerDraw<'_> {
    pub fn draw(&self, a: &PlayAssets, list: &mut DrawList) {
        let p = self.p;
        if !p.alive() || p.respawning || p.pos.y > 150.0 {
            return;
        }
        let vis = self.vis;
        let spawn_k = vis.spawn_t.map_or(1.0, |t| curve(&SPAWN_KEYS, t));
        let s = p.scale;
        let pos = self.pos;
        let col = self.skin.color;
        let base = self.sort;
        let limb_w = 0.1 * s;

        // Legs.
        for k in 0..2 {
            let side = if k == 0 { -1.0 } else { 1.0 };
            let hip = pos + Vec2::new(0.2 * side, -0.3) * s * spawn_k;
            let foot = pos + (vis.feet[k].pos - v(p.pos)) * spawn_k;
            let (mid, end) = ik_toward(hip, foot, 0.4 * s, 0.4 * s, Vec2::new(vis.facing, 0.0));
            limb(list, base, hip, mid, end, limb_w, col);
        }

        // Gun and arms.
        let aim = {
            let a = v(p.aim);
            if a.length() > 1e-4 {
                a.normalize()
            } else {
                Vec2::new(vis.facing, 0.0)
            }
        };
        let gun = self.gun_pos;
        let gun_side = if gun.x > pos.x { 1.0 } else { -1.0 };
        for side in [-1.0f32, 1.0] {
            let shoulder = pos + Vec2::new(0.45 * side, 0.0) * s * spawn_k;
            let target = if side == gun_side {
                gun
            } else {
                pos + (vis.idle_hand - v(p.pos))
            };
            let (mid, end) = ik_toward(
                shoulder,
                target,
                0.5 * s,
                0.5 * s,
                Vec2::new(-side * 0.3, -1.0),
            );
            limb(list, base, shoulder, mid, end, limb_w, col);
        }
        draw_gun(list, base, gun, aim, vis.mirror, col);

        // Ammo pips beside the gun and the reload ring (ui.md 3.5).
        let g = &p.gun;
        let max = g.ammo_stats.max_ammo.clamp(0, 90);
        if max > 0 {
            let side = Vec2::new(aim.y, -aim.x) * if vis.mirror > 0.5 { -1.0 } else { 1.0 };
            let pip_d = 0.16;
            let step = 0.23;
            let shown = max.min(12);
            for k in 0..shown {
                let off = (k as f32 - (shown as f32 - 1.0) * 0.5) * step;
                let c = gun - side * 0.45 + aim * off;
                let lit = k < g.ammo;
                let colr = if lit {
                    Color::rgb(0.566, 0.463, 0.264)
                } else {
                    Color::rgba(0.1, 0.1, 0.1, 0.6)
                };
                list.circle(Layer::World, base + 5, c, pip_d * 0.5, colr);
            }
            if g.reloading {
                let dur = g.ammo_stats.reload_duration().max(1e-3);
                let f = ((dur - g.reload_counter) / dur).clamp(0.0, 1.0);
                list.ring(
                    Layer::World,
                    base + 5,
                    gun,
                    0.32,
                    0.07,
                    std::f32::consts::FRAC_PI_2,
                    -std::f32::consts::TAU * f,
                    Color::rgb(0.566, 0.463, 0.264),
                );
            }
        }

        // Body disc (stencil value per player) and the stroke fill.
        let mask_v = 2 + (self.index as u8 % 250);
        let body_d = 1.1044 * s * spawn_k;
        if let Some(b) = a.body {
            let k = body_d / b.rect_size.x.max(1e-4);
            list.sprite(
                Layer::World,
                base + 1,
                b.masked(
                    Affine2::from_scale_angle_translation(Vec2::splat(k), 0.0, pos),
                    col,
                    Mask::WriteValue(mask_v),
                ),
            );
        } else {
            list.circle(Layer::World, base + 1, pos, body_d * 0.5, col);
        }
        let skin_drawn = self
            .effects
            .is_some_and(|e| e.draw_skin(self.index, list, base + 2, mask_v, spawn_k, pos));
        if let Some(g) = a.grass.filter(|_| !skin_drawn) {
            for st in &vis.strokes {
                let t = st.age / st.life;
                let alpha = if t < 0.25 {
                    t / 0.25
                } else if t > 0.75 {
                    (1.0 - t) / 0.25
                } else {
                    1.0
                };
                let k = st.size * s * spawn_k / g.rect_size.x.max(1e-4);
                list.sprite(
                    Layer::World,
                    base + 2,
                    g.masked(
                        Affine2::from_scale_angle_translation(
                            Vec2::splat(k),
                            st.rot,
                            pos + st.local * s * spawn_k,
                        ),
                        st.color.with_alpha(alpha),
                        Mask::InsideValue(mask_v),
                    ),
                );
            }
        }

        // Face: root scale 0.16, items at (offset) * 1.05, mirrored.
        let face_k = 0.16 * s * spawn_k;
        let default_face: Vec<super::assets::FaceItem> =
            [a.eyes, a.mouth].into_iter().flatten().collect();
        for item in self.face.unwrap_or(&default_face).iter().copied() {
            let off = item.offset * 1.05;
            let fx = if off.x == 0.0 { 1.0 } else { vis.facing };
            let t = Affine2::from_translation(pos)
                * Affine2::from_scale(Vec2::splat(face_k))
                * Affine2::from_translation(Vec2::new(off.x * fx, off.y))
                * Affine2::from_scale(Vec2::new(item.scale * 1.05 * vis.facing, item.scale * 1.05));
            list.sprite(Layer::World, base + 3, item.sprite.sprite(t, item.color));
        }

        // Block orb at the idle hand (12.7).
        let hand = pos + (vis.idle_hand - v(p.pos));
        let orb = hand + Vec2::new(0.0, 0.1) * s;
        let cd = p.block.stats.cooldown * p.block.stats.cd_multiplier + p.block.stats.cd_add;
        let ready = p.block.counter >= cd.max(0.0);
        let orb_r = 0.16 * s;
        let orb_c = if ready {
            Color::rgb(0.8, 0.8, 0.8)
        } else {
            Color::rgb(0.175, 0.191, 0.208)
        };
        list.circle(Layer::World, base + 4, orb, orb_r, orb_c);
        if !ready && cd > 0.0 {
            let f = (p.block.counter / cd).clamp(0.0, 1.0);
            list.ring(
                Layer::World,
                base + 4,
                orb,
                orb_r * 1.5,
                orb_r * 0.35,
                std::f32::consts::FRAC_PI_2,
                std::f32::consts::TAU * f,
                Color::rgb(0.8, 0.8, 0.8),
            );
        }

        // Block shield flash.
        if vis.block_flash > 0.0 {
            let t = 1.0 - vis.block_flash / 0.4;
            let r = (1.1 + 1.0 * t) * s;
            list.ring(
                Layer::WorldUnlit,
                base + 5,
                pos,
                r,
                0.12 * s * (1.0 - t) + 0.02,
                0.0,
                std::f32::consts::TAU,
                Color::rgba(1.0, 1.0, 1.0, (1.0 - t) * 0.9),
            );
        }

        // Stun: three hollow triangles orbiting above the head (18.8).
        if vis.stun_scale > 0.02
            && let Some(tri) = a.hollow_triangle
        {
            {
                let centre = pos + Vec2::new(0.0, 0.65 * s);
                for (i, speed) in [150.0f32, 170.0, 210.0].iter().enumerate() {
                    let ang = (vis.stun_t * speed + i as f32 * 120.0).to_radians();
                    let off = Vec2::from_angle(ang).rotate(Vec2::new(0.0, 0.14)) * s;
                    let k = 0.218 * 0.25 * vis.stun_scale * s / tri.rect_size.x.max(1e-4) * 10.24;
                    list.sprite(
                        Layer::World,
                        base + 6,
                        tri.sprite(
                            Affine2::from_scale_angle_translation(
                                Vec2::splat(k),
                                ang,
                                centre + off,
                            ),
                            Color::rgb(0.298, 0.428, 0.613),
                        ),
                    );
                }
            }
        }
        // Silence: two crossed red bars.
        if p.silenced {
            let c = pos + Vec2::new(0.0, 0.0);
            let wob = (vis.stun_t * 6.0).sin() * 15f32.to_radians();
            for sgn in [-1.0f32, 1.0] {
                list.rect(
                    Layer::World,
                    base + 6,
                    c,
                    Vec2::new(1.26, 0.27) * s,
                    sgn * 45f32.to_radians() + wob,
                    Color::rgb(0.6226, 0.2614, 0.2842),
                );
            }
        }
    }

    /// The leader's crown above the health bar (match-flow.md 12).
    pub fn draw_crown(&self, list: &mut DrawList, sort: i32) {
        let p = self.p;
        if !p.alive() || p.respawning || p.pos.y > 150.0 {
            return;
        }
        let s = p.scale;
        let c = self.pos + (self.vis.bar_pos - v(p.pos)) + Vec2::new(0.0, 0.851 * s + 0.45);
        let w = 0.55;
        let pts = [
            c + Vec2::new(-w * 0.5, -0.15),
            c + Vec2::new(w * 0.5, -0.15),
            c + Vec2::new(w * 0.5, 0.18),
            c + Vec2::new(w * 0.25, 0.02),
            c + Vec2::new(0.0, 0.22),
            c + Vec2::new(-w * 0.25, 0.02),
            c + Vec2::new(-w * 0.5, 0.18),
        ];
        list.polygon(Layer::World, sort, &pts, Color::rgb(1.0, 0.78, 0.2));
    }

    /// Health bar above the player (drawn in a band above all players).
    pub fn draw_bar(&self, list: &mut DrawList, sort: i32) {
        let p = self.p;
        if !p.alive() || p.respawning || p.pos.y > 150.0 {
            return;
        }
        let vis = self.vis;
        let s = p.scale;
        let anchor = self.pos + (vis.bar_pos - v(p.pos)) + Vec2::new(0.0, 0.851 * s);
        let size = Vec2::new(1.71, 0.21) * (s / 1.2);
        list.rect(
            Layer::World,
            sort,
            anchor,
            size,
            0.0,
            Color::rgb(0.0755, 0.0755, 0.0755),
        );
        let inner = size - Vec2::splat(0.0);
        let fill = |list: &mut DrawList, f: f32, c: Color, z: i32| {
            let f = f.clamp(0.0, 1.0);
            if f <= 0.001 {
                return;
            }
            let w = inner.x * f;
            list.rect(
                Layer::World,
                sort + z,
                anchor + Vec2::new(-inner.x * 0.5 + w * 0.5, 0.0),
                Vec2::new(w, inner.y),
                0.0,
                c,
            );
        };
        fill(list, vis.lag, Color::rgb(0.783, 0.159, 0.159), 1);
        fill(list, vis.hp, Color::rgb(0.330, 0.509, 0.257), 2);
    }
}

/// The gun: `Handle` and `Barrel` boxes in the team colour, mirrored when
/// the gun is left of the player (player.md 4.5).
fn draw_gun(list: &mut DrawList, sort: i32, gun: Vec2, aim: Vec2, mirror: f32, c: Color) {
    // Gun frame: local up = aim, local right = aim rotated -90 degrees.
    let up = aim;
    let right = Vec2::new(up.y, -up.x);
    let m = mirror.clamp(-0.2, 1.2);
    let lerp = |a: f32, b: f32| a + (b - a) * m;
    let parts = [
        // (local pos, size, local rotation degrees)
        (
            Vec2::new(lerp(0.01, -0.01), 0.12),
            Vec2::new(0.352, 0.15),
            lerp(-15.0, 15.0),
        ),
        (
            Vec2::new(lerp(-0.15, 0.15), 0.4),
            Vec2::new(0.64, 0.15),
            90.0,
        ),
    ];
    let base_ang = right.y.atan2(right.x);
    for (lp, size, rot) in parts {
        let wp = gun + right * lp.x + up * lp.y;
        list.rect(Layer::World, sort, wp, size, base_ang + rot.to_radians(), c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ik_reaches_and_bends() {
        let (mid, end) = ik(Vec2::ZERO, Vec2::new(0.0, -1.0), 0.6, 0.6, 1.0);
        assert!((end - Vec2::new(0.0, -1.0)).length() < 1e-4);
        assert!((mid.length() - 0.6).abs() < 1e-4);
        assert!(mid.x > 0.0, "bends counter-clockwise from straight down");
        let (_, end) = ik(Vec2::ZERO, Vec2::new(5.0, 0.0), 0.6, 0.6, 1.0);
        assert!(end.x < 1.2 && end.x > 1.19);
    }
}
