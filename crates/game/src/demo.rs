//! Placeholder demo scene (NOT the game): exercises the renderer (shapes,
//! polygons, text, rings, lights with soft shadows), the fixed-step loop
//! with interpolation, input devices (every connected device gets a ball)
//! and the mixer (bounce blips with pitch/pan, an optional Ogg on the music
//! bus).

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use glam::Affine2;
use glam::Vec2;
use rounds_render::{
    Color, Filter, Image, Layer, Light, LightingSettings, PostSettings, Sprite, Text, TextureId,
    TextureOptions,
};

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::audio::{self, Bus, PlayParams, SoundData};
use crate::input::{Buttons, DeviceId};

const GRAVITY: f32 = -30.0;
const RADIUS: f32 = 0.6;
const PALETTE: [u32; 12] = [
    0xff7a2a, 0x2a9dff, 0x5ad35a, 0xe84a8a, 0xf2d33a, 0x9a6bff, 0x3ad6c8, 0xff4a4a, 0xb0b0b0,
    0x8bd35a, 0xff9ad0, 0x6a8cff,
];

/// Axis-aligned boxes of the demo arena: (centre, half size).
const BOXES: [(Vec2, Vec2); 6] = [
    (Vec2::new(0.0, -9.0), Vec2::new(18.0, 1.5)),
    (Vec2::new(-10.0, -2.5), Vec2::new(3.0, 0.5)),
    (Vec2::new(10.0, -2.5), Vec2::new(3.0, 0.5)),
    (Vec2::new(0.0, 2.0), Vec2::new(2.5, 0.5)),
    (Vec2::new(-5.0, -6.0), Vec2::new(0.75, 1.5)),
    (Vec2::new(5.5, -6.5), Vec2::new(1.0, 1.0)),
];

struct Ball {
    device: DeviceId,
    pos: Vec2,
    prev: Vec2,
    vel: Vec2,
    aim: Vec2,
    color: Color,
}

pub struct Demo {
    balls: Vec<Ball>,
    tick: u64,
    blip: Option<Arc<SoundData>>,
    music_path: Option<PathBuf>,
    devices_seen: usize,
    crate_tex: TextureId,
}

impl Demo {
    pub fn new(music_path: Option<PathBuf>) -> Self {
        Self {
            balls: Vec::new(),
            tick: 0,
            blip: None,
            music_path,
            devices_seen: 0,
            crate_tex: TextureId::WHITE,
        }
    }

    fn spawn(&mut self, device: DeviceId) {
        let i = self.balls.len();
        let x = -12.0 + (i as f32 * 4.7) % 24.0;
        let pos = Vec2::new(x, 4.0 + (i % 3) as f32 * 2.0);
        self.balls.push(Ball {
            device,
            pos,
            prev: pos,
            vel: Vec2::new(if i.is_multiple_of(2) { 6.0 } else { -5.0 }, 0.0),
            aim: Vec2::X,
            color: Color::hex(PALETTE[i % PALETTE.len()]),
        });
    }
}

fn collide(pos: &mut Vec2, vel: &mut Vec2) -> bool {
    let mut hit = false;
    for (c, h) in BOXES {
        let closest = pos.clamp(c - h, c + h);
        let d = *pos - closest;
        let dist = d.length();
        if dist < RADIUS && dist > 1e-6 {
            let n = d / dist;
            *pos = closest + n * RADIUS;
            let vn = vel.dot(n);
            if vn < 0.0 {
                *vel -= n * vn * 1.8;
                hit = vn < -2.0;
            }
        }
    }
    // Arena walls.
    if pos.x.abs() > 17.0 {
        pos.x = pos.x.clamp(-17.0, 17.0);
        vel.x = -vel.x * 0.8;
        hit = true;
    }
    hit
}

impl Scene for Demo {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        // A procedural 8x8 "crate" texture for the sprite path.
        let mut img = Image::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let edge = x == 0 || y == 0 || x == 7 || y == 7 || x == y || x + y == 7;
                img.set_pixel(
                    x,
                    y,
                    if edge {
                        [90, 60, 30, 255]
                    } else {
                        [170, 120, 60, 255]
                    },
                );
            }
        }
        let opts = TextureOptions {
            filter: Filter::Nearest,
            ..Default::default()
        };
        self.crate_tex = ctx.renderer.create_texture(ctx.gpu, &img, opts);
        let rate = ctx.audio.sample_rate();
        // Test tone at start: 440 Hz, 0.4 s.
        let tone = Arc::new(SoundData::sine(440.0, 0.4, rate, 0.3));
        ctx.audio.play(
            &tone,
            PlayParams {
                bus: Bus::Ui,
                ..Default::default()
            },
        );
        self.blip = Some(Arc::new(SoundData::sine(660.0, 0.08, rate, 0.25)));
        if let Some(path) = &self.music_path {
            let music = Arc::new(audio::decode_ogg_file(path)?);
            log::info!(
                "music: {} ({:.1} s, {} Hz)",
                path.display(),
                music.duration(),
                music.sample_rate
            );
            ctx.audio.play(
                &music,
                PlayParams {
                    bus: Bus::Music,
                    looping: true,
                    volume: 0.6,
                    ..Default::default()
                },
            );
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.tick = ctx.tick;
        // Every device gets a ball the first time it is seen (hot-plug).
        for (dev, _) in ctx.inputs {
            if !self.balls.iter().any(|b| b.device == *dev) {
                self.spawn(*dev);
            }
        }
        // Headless runs have one device; add a second, AI-less ball so the
        // scene is not lonely.
        if self.balls.len() < 2 {
            self.spawn(DeviceId::Controller(u32::MAX));
        }
        self.devices_seen = ctx.inputs.len();
        let dt = ctx.dt;
        for b in &mut self.balls {
            b.prev = b.pos;
            if let Some((_, input)) = ctx.inputs.iter().find(|(d, _)| *d == b.device) {
                b.vel.x += input.move_axis.x * 40.0 * dt;
                if input.pressed.contains(Buttons::JUMP) {
                    b.vel.y = 14.0;
                }
                if input.aim_axis != Vec2::ZERO {
                    b.aim = input.aim_axis.normalize();
                }
            }
            b.vel.y += GRAVITY * dt;
            b.vel.x *= 0.995;
            b.pos += b.vel * dt;
            if collide(&mut b.pos, &mut b.vel)
                && let Some(blip) = &self.blip
            {
                let pitch = 0.8 + ((ctx.tick * 7919) % 100) as f32 / 200.0;
                ctx.audio.play(
                    blip,
                    PlayParams {
                        pitch,
                        pan: (b.pos.x / 17.0).clamp(-1.0, 1.0),
                        volume: (b.vel.length() / 15.0).min(1.0),
                        ..Default::default()
                    },
                );
            }
            if b.pos.y < -20.0 {
                b.pos = Vec2::new(0.0, 8.0);
                b.prev = b.pos;
                b.vel = Vec2::ZERO;
            }
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        let t = (self.tick as f32 + ctx.alpha) * crate::timing::FIXED_DT as f32;
        let f = &mut *ctx.frame;
        f.camera.ortho_size = 10.5;
        f.camera.position = Vec2::new(0.0, -1.0);
        f.clear = Color::hex(0xc9a27a);
        f.lighting = LightingSettings {
            enabled: true,
            ambient: Color::hex(0x5a5070),
            exposure: 1.0,
            light_map_scale: 4,
            ..Default::default()
        };
        f.post = PostSettings {
            vignette: 0.6,
            ..Default::default()
        };
        let dl = &mut *ctx.list;

        // Arena: dark boxes that also cast shadows.
        for (c, h) in BOXES {
            dl.rect(Layer::World, 0, c, h * 2.0, 0.0, Color::hex(0x1a1a22));
            dl.shadow_caster(&[
                c - h,
                Vec2::new(c.x + h.x, c.y - h.y),
                c + h,
                Vec2::new(c.x - h.x, c.y + h.y),
            ]);
        }
        // A concave polygon (mesh path) as a ramp decoration.
        let ramp = [
            Vec2::new(-17.0, -7.5),
            Vec2::new(-12.0, -7.5),
            Vec2::new(-14.0, -6.5),
            Vec2::new(-17.0, -4.0),
        ];
        dl.polygon(Layer::World, 0, &ramp, Color::hex(0x2b2b38));
        dl.shadow_caster(&ramp);

        // Sprites: a stack of crates (one batch).
        for i in 0..3 {
            dl.sprite(
                Layer::World,
                1,
                Sprite {
                    texture: self.crate_tex,
                    transform: Affine2::from_angle_translation(
                        if i == 2 { 0.15 } else { 0.0 },
                        Vec2::new(12.0, -7.5 + i as f32 * 1.0),
                    ),
                    size: Vec2::splat(1.0),
                    pivot: Vec2::new(0.5, 0.0),
                    ..Default::default()
                },
            );
        }

        // Balls, interpolated between ticks, with aim lines.
        for b in &self.balls {
            let p = b.prev.lerp(b.pos, ctx.alpha);
            dl.circle(Layer::World, 10, p, RADIUS, b.color);
            dl.line(
                Layer::World,
                11,
                p,
                p + b.aim * 1.4,
                0.15,
                Color::WHITE.with_alpha(0.8),
            );
        }

        // Lights: one orbiting warm light, one static cool light.
        let lp = Vec2::new(t.cos() * 9.0, 5.0 + (t * 0.7).sin() * 2.0);
        dl.light(Light {
            position: lp,
            color: Color::hex(0xffd9a0),
            intensity: 1.6,
            range: 26.0,
            source_radius: 0.6,
            casts_shadows: true,
            ..Default::default()
        });
        dl.light(Light {
            position: Vec2::new(12.0, 6.0),
            color: Color::hex(0x80b0ff),
            intensity: 0.8,
            range: 16.0,
            source_radius: 0.3,
            casts_shadows: true,
            ..Default::default()
        });
        dl.circle(Layer::WorldUnlit, 0, lp, 0.35, Color::hex(0xfff0c0));
        dl.ring(
            Layer::WorldUnlit,
            0,
            lp,
            0.7,
            0.08,
            0.0,
            TAU,
            Color::hex(0xfff0c0).with_alpha(0.5),
        );

        // HUD (screen pixels).
        let (w, h) = (ctx.size.0 as f32, ctx.size.1 as f32);
        let s = h / 1080.0;
        let title = "rounds-port: placeholder demo (not the game)";
        let tsize = ctx
            .renderer
            .measure_text(rounds_render::FontId::DEFAULT, title, 44.0 * s);
        dl.rounded_rect(
            Layer::Screen,
            -1,
            Vec2::new(w * 0.5, 30.0 * s + tsize.y * 0.5),
            tsize + Vec2::new(40.0, 12.0) * s,
            12.0 * s,
            Color::BLACK.with_alpha(0.35),
        );
        dl.push(
            Layer::Screen,
            0,
            Text {
                text: title.into(),
                size: 44.0 * s,
                position: Vec2::new(w * 0.5, 30.0 * s),
                anchor: Vec2::new(0.5, 0.0),
                color: Color::WHITE,
                ..Default::default()
            },
        );
        let info = format!(
            "tick {}  alpha {:.2}  devices {}  balls {}\nF11: fullscreen   WASD/stick: move   Space/A: jump",
            self.tick,
            ctx.alpha,
            self.devices_seen,
            self.balls.len()
        );
        dl.text(
            Layer::Screen,
            0,
            info,
            Vec2::new(24.0 * s, h - 24.0 * s),
            24.0 * s,
            Vec2::new(0.0, 1.0),
            Color::WHITE.with_alpha(0.85),
        );
        // Round timer ring, filling over 5 s.
        let c = Vec2::new(w - 90.0 * s, 90.0 * s);
        let frac = (t / 5.0).fract();
        dl.ring(
            Layer::Screen,
            0,
            c,
            52.0 * s,
            12.0 * s,
            0.0,
            TAU,
            Color::BLACK.with_alpha(0.35),
        );
        dl.ring(
            Layer::Screen,
            1,
            c,
            52.0 * s,
            12.0 * s,
            -TAU / 4.0,
            frac * TAU,
            Color::hex(0xff7a2a),
        );
        dl.text(
            Layer::Screen,
            2,
            format!("{:.0}", 5.0 - frac * 5.0),
            c,
            40.0 * s,
            Vec2::splat(0.5),
            Color::WHITE,
        );
        // Player swatches.
        for (i, b) in self.balls.iter().enumerate() {
            let p = Vec2::new(40.0 * s + i as f32 * 56.0 * s, 110.0 * s);
            dl.rounded_rect(Layer::Screen, 0, p, Vec2::splat(40.0 * s), 8.0 * s, b.color);
            dl.text(
                Layer::Screen,
                1,
                format!("{}", i + 1),
                p,
                24.0 * s,
                Vec2::splat(0.5),
                Color::BLACK,
            );
        }
    }
}
