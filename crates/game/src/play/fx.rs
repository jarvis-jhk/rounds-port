//! Presentation-only effects: a small particle pool, the screen shakers
//! (maps.md 9.1) and cosmetic randomness (never the sim's PRNG).

use glam::{Affine2, Vec2};
use rounds_render::{Blend, Color, DrawList, Layer, Primitive, Sprite, TextureId};

use super::assets::SpriteRef;

/// Cosmetic xorshift RNG.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    pub fn unit(&mut self) -> Vec2 {
        let a = self.f() * std::f32::consts::TAU;
        Vec2::new(a.cos(), a.sin())
    }
    /// A direction within `spread` radians of `dir`.
    pub fn cone(&mut self, dir: Vec2, spread: f32) -> Vec2 {
        let a = dir.y.atan2(dir.x) + self.range(-spread, spread);
        Vec2::new(a.cos(), a.sin())
    }
}

/// How a particle looks.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// Filled circle.
    Dot,
    /// Velocity-aligned streak (sparks): length = `stretch * speed`.
    Streak { stretch: f32 },
    /// Hollow ring of the given band (fraction of the radius).
    Ring { band: f32 },
    /// Rotated square.
    Square,
    /// A sprite (texture particle).
    #[allow(dead_code)]
    Sprite(SpriteRef),
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: Vec2,
    pub vel: Vec2,
    pub age: f32,
    pub life: f32,
    /// Size (diameter) at birth and death.
    pub size: (f32, f32),
    pub color: (Color, Color),
    /// Alpha envelope: fade in over this fraction of the life.
    pub fade_in: f32,
    pub drag: f32,
    pub gravity: f32,
    pub rot: f32,
    pub spin: f32,
    pub shape: Shape,
    pub additive: bool,
    /// Drawn after the world's lighting (glows, UI-ish effects).
    pub unlit: bool,
    /// Uses game time (slows down in slow motion) instead of real time.
    pub game_time: bool,
}

impl Default for Particle {
    fn default() -> Self {
        Particle {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            age: 0.0,
            life: 1.0,
            size: (0.3, 0.0),
            color: (Color::WHITE, Color::WHITE.with_alpha(0.0)),
            fade_in: 0.0,
            drag: 0.0,
            gravity: 0.0,
            rot: 0.0,
            spin: 0.0,
            shape: Shape::Dot,
            additive: false,
            unlit: false,
            game_time: true,
        }
    }
}

/// A pool of particles, updated with presentation time.
#[derive(Default)]
pub struct Particles {
    pub list: Vec<Particle>,
}

/// Particle cap (oldest first out).
const MAX_PARTICLES: usize = 6000;

impl Particles {
    pub fn push(&mut self, p: Particle) {
        if self.list.len() >= MAX_PARTICLES {
            self.list.remove(0);
        }
        self.list.push(p);
    }

    /// `real` seconds of real time; `game_k` = game-time factor relative to
    /// normal play (time scale / 0.85).
    pub fn update(&mut self, real: f32, game_k: f32) {
        for p in &mut self.list {
            let dt = if p.game_time { real * game_k } else { real };
            p.age += dt;
            p.vel.y -= p.gravity * dt;
            if p.drag > 0.0 {
                p.vel *= (-p.drag * dt).exp();
            }
            p.pos += p.vel * dt;
            p.rot += p.spin * dt;
        }
        self.list.retain(|p| p.age < p.life);
    }

    pub fn draw(&self, list: &mut DrawList, sort: i32) {
        for p in &self.list {
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let size = p.size.0 + (p.size.1 - p.size.0) * t;
            let mut c = p.color.0.lerp(p.color.1, t);
            if p.fade_in > 0.0 && t < p.fade_in {
                c.a *= t / p.fade_in;
            }
            if c.a <= 0.003 || size <= 0.001 {
                continue;
            }
            let layer = if p.unlit {
                Layer::WorldUnlit
            } else {
                Layer::World
            };
            let blend = if p.additive {
                Blend::Additive
            } else {
                Blend::Alpha
            };
            match p.shape {
                Shape::Dot if !p.additive => list.circle(layer, sort, p.pos, size * 0.5, c),
                Shape::Ring { band } => list.ring(
                    layer,
                    sort,
                    p.pos,
                    size * 0.5,
                    (size * 0.5 * band).max(0.02),
                    0.0,
                    std::f32::consts::TAU,
                    c,
                ),
                Shape::Streak { stretch } => {
                    let speed = p.vel.length();
                    let dir = if speed > 1e-4 { p.vel / speed } else { Vec2::X };
                    let len = (speed * stretch).max(size);
                    list.push(
                        layer,
                        sort,
                        Primitive::Line {
                            a: p.pos - dir * len,
                            b: p.pos,
                            width: size,
                            round: true,
                            color: c,
                        },
                    );
                }
                Shape::Square => list.rect(layer, sort, p.pos, Vec2::splat(size), p.rot, c),
                Shape::Sprite(s) => {
                    let k = size / s.rect_size.x.max(1e-4);
                    let mut sp = s.sprite(
                        Affine2::from_scale_angle_translation(Vec2::splat(k), p.rot, p.pos),
                        c,
                    );
                    sp.blend = blend;
                    list.sprite(layer, sort, sp);
                }
                Shape::Dot => list.sprite(
                    layer,
                    sort,
                    Sprite {
                        texture: TextureId::WHITE,
                        transform: Affine2::from_translation(p.pos),
                        size: Vec2::splat(size * 0.7),
                        tint: c,
                        blend,
                        ..Default::default()
                    },
                ),
            }
        }
    }
}

/// `Screenshaker`: a damped spring on an offset (maps.md 9.1).
#[derive(Clone, Copy, Debug)]
pub struct Shaker {
    pub pos: Vec2,
    pub vel: Vec2,
    pub spring: f32,
    pub damper: f32,
    pub force: f32,
    pub threshold: f32,
}

impl Shaker {
    /// The world camera rig: spring 3000, damper 30, force -10.
    pub fn world() -> Shaker {
        Shaker {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            spring: 3000.0,
            damper: 30.0,
            force: -10.0,
            threshold: 0.0,
        }
    }
    /// The UI rig / point visualiser: spring 100, damper 60, force -500.
    pub fn ui() -> Shaker {
        Shaker {
            spring: 100.0,
            damper: 60.0,
            force: -500.0,
            ..Shaker::world()
        }
    }

    pub fn feel(&mut self, dir: Vec2, mult: f32) {
        if dir.length() >= self.threshold {
            self.vel += dir.clamp_length_max(100.0) * self.force * mult;
        }
    }

    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.015);
        let speed = self.vel.length();
        if speed > 1e-6 {
            self.vel -= self.vel / speed * speed.powf(0.8) * self.damper * dt;
        }
        self.vel -= self.pos * self.spring * dt;
        self.pos += self.vel * dt;
    }
}

/// `AddUIGameFeelOverTime(amount, time)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FeelOverTime {
    pub amount: f32,
    pub total: f32,
    pub left: f32,
}

impl FeelOverTime {
    pub fn start(amount: f32, time: f32) -> FeelOverTime {
        FeelOverTime {
            amount,
            total: time,
            left: time,
        }
    }
    pub fn step(&mut self, dt: f32, rng: &mut Rng) -> Vec2 {
        if self.left <= 0.0 || self.total <= 0.0 {
            return Vec2::ZERO;
        }
        let k = self.left / self.total;
        self.left -= dt;
        rng.unit() * self.amount * dt * k
    }
}

/// Unity `AnimationCurve` key (time, value, in/out slope).
#[derive(Clone, Copy, Debug)]
pub struct Key(pub f32, pub f32, pub f32, pub f32);

/// Cubic Hermite evaluation, clamped outside the keys.
pub fn curve(keys: &[Key], t: f32) -> f32 {
    if keys.is_empty() {
        return 0.0;
    }
    if t <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let dt = b.0 - a.0;
            if dt <= 1e-6 {
                return b.1;
            }
            let s = (t - a.0) / dt;
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            return h00 * a.1 + h10 * dt * a.3 + h01 * b.1 + h11 * dt * b.2;
        }
    }
    keys[keys.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaker_settles() {
        let mut s = Shaker::world();
        s.feel(Vec2::new(1.0, 0.0), 1.0);
        assert!(s.vel.x < 0.0, "kicks opposite");
        for _ in 0..400 {
            s.update(0.014);
        }
        assert!(s.pos.length() < 1e-3 && s.vel.length() < 1e-2);
    }

    #[test]
    fn curve_hits_keys() {
        let k = [Key(0.0, 0.0, 0.0, 0.0), Key(1.0, 1.0, 0.0, 0.0)];
        assert!((curve(&k, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(curve(&k, 2.0), 1.0);
    }
}
