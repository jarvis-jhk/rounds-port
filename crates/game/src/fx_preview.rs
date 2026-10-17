//! `--fx-preview NAME[,NAME...]` (headless dev scene): plays imported
//! prefab particle effects (by root name, e.g. `A_Explosion`,
//! `PlayerDeath`, `E_HealingCircle`; `all` = every one) side by side on a
//! neutral background and restarts them every 2 s, to compare them with the
//! original. Use with `--shots` to catch the first frames.

use std::rc::Rc;

use anyhow::Result;
use glam::Vec2;
use rounds_import::GameData;
use rounds_render::{Color, Layer};

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::play::assets::PlayAssets;
use crate::play::effects::{EffectBank, Effects};

pub struct FxPreview {
    data: Rc<GameData>,
    names: Vec<String>,
    assets: Option<PlayAssets>,
    effects: Effects,
    t: f32,
    next: f32,
    look: crate::art::MainLook,
}

impl FxPreview {
    pub fn new(data: Rc<GameData>, names: &str) -> FxPreview {
        FxPreview {
            data: data.clone(),
            names: names
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
            assets: None,
            effects: Effects::default(),
            t: 0.0,
            next: 0.0,
            look: crate::art::MainLook::from_data(&data),
        }
    }

    fn spot(&self, i: usize) -> Vec2 {
        let n = self.names.len().max(1);
        let cols = (n as f32).sqrt().ceil() as usize;
        let rows = n.div_ceil(cols);
        let (c, r) = (i % cols, i / cols);
        Vec2::new(
            (c as f32 + 0.5 - cols as f32 * 0.5) * 12.0,
            -(r as f32 + 0.5 - rows as f32 * 0.5) * 12.0,
        )
    }
}

impl Scene for FxPreview {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        let mut assets = PlayAssets::new(ctx.gpu, ctx.renderer, self.data.clone());
        self.effects = Effects::new(EffectBank::load(ctx.gpu, ctx.renderer, &mut assets));
        if self.names.iter().any(|n| n == "all") {
            self.names = self.effects.names();
        }
        println!(
            "fx-preview: {} (bloom {:?})",
            self.names.join(", "),
            self.look.main_profile.bloom
        );
        self.assets = Some(assets);
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        if self.t >= self.next {
            self.next = self.t + 2.0;
            let skin = self.assets.as_ref().map(|a| a.skin(0));
            let team = skin.map_or([Color::WHITE; 4], |s| {
                [s.color, s.background, s.particle, s.win_text]
            });
            for i in 0..self.names.len() {
                let p = self.spot(i);
                if !self
                    .effects
                    .spawn_named(&self.names[i].clone(), p, 1.0, team)
                {
                    log::warn!("fx-preview: no effect named {}", self.names[i]);
                }
            }
        }
        self.effects.update_free(ctx.dt, ctx.dt);
        self.t += ctx.dt;
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        let n = self.names.len().max(1);
        let cols = (n as f32).sqrt().ceil();
        ctx.frame.camera = rounds_render::Camera2D::new(Vec2::ZERO, (cols * 6.0).max(6.0));
        ctx.frame.clear = Color::rgb(0.13, 0.15, 0.2);
        // The main camera's post-processing (bloom, grading) as in a match.
        ctx.frame.post = rounds_render::PostSettings {
            grading: self.look.main_profile.grading,
            bloom: self.look.main_profile.bloom,
            ..Default::default()
        };
        for i in 0..self.names.len() {
            let p = self.spot(i);
            ctx.list.text(
                Layer::WorldUnlit,
                0,
                self.names[i].clone(),
                p + Vec2::new(0.0, -5.0),
                0.8,
                Vec2::splat(0.5),
                Color::rgba(1.0, 1.0, 1.0, 0.6),
            );
        }
        self.effects.draw(ctx.list);
    }
}
