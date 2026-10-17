//! A local match: the [`MatchScene`] hosts the simulation through a
//! [`MatchDriver`], maps local devices (or bots) to player slots, and
//! presents the sim state with the original's look: maps sliding in and
//! out, players, bullets, effects, particles, screen shake, HUD, card
//! choice, win screen, sounds and music.
//!
//! Structure:
//! - [`driver`]: `MatchDriver` (local now, GGRS later), event filter;
//! - [`input_map`]: devices -> `PlayerInput` (mouse aim, dead zones);
//! - [`map_play`]: imported maps positioned by the sim;
//! - [`player_view`], [`world_view`]: players, bullets, effect objects;
//! - [`hud`]: round counter, point/round visualiser, card choice, banners;
//! - [`sound`]: sound events and music; [`fx`]: particles and shakers.

pub mod assets;
pub mod driver;
pub mod effects;
pub mod fx;
pub mod hud;
pub mod input_map;
pub mod map_play;
pub mod player_view;
pub mod sound;
pub mod ui_prefab;
pub mod world_view;

use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use glam::Vec2;
use rounds_content::bots::Bots;
use rounds_import::GameData;
use rounds_render::{Color, DrawList, FrameSettings, Layer};
use rounds_sim::flow::Phase;
use rounds_sim::{Content, Event, MapId, MatchSetup, PlayerInput};

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::art::MainLook;
use crate::map_scene::letterbox;
use assets::{FaceItem, PlayAssets};
use driver::{EventFilter, LocalDriver, MatchDriver, MatchOutcome};
use fx::{Particles, Rng, Shaker};
use input_map::{SlotControl, ViewInfo, bind_controllers};
use map_play::{Motion, ShownMap};
use player_view::{PlayerVis, v};
use sound::Sounds;
use world_view::WorldFx;

/// Fixed step (s).
pub const DT: f32 = rounds_sim::DT;

/// Everything needed to start a match.
pub struct MatchConfig {
    pub setup: MatchSetup,
    pub controls: Vec<SlotControl>,
    /// Seed of the bot inputs.
    pub bot_seed: u64,
    /// Play sounds (off for silent headless runs).
    pub sound: bool,
    /// Debug: cards every player gets at the start.
    pub cards: Vec<String>,
    /// Face items per slot (empty = the default face).
    pub faces: Vec<Vec<FaceItem>>,
    /// Player options (aim snapping, shake, stat numbers).
    pub options: PlayOptions,
    /// Bots answer the rematch prompt with this (None = like any input).
    pub bot_rematch: Option<bool>,
    /// Card pick mode (local driver).
    pub pick_mode: rounds_sim::PickMode,
}

/// Settings that affect play.
#[derive(Clone, Copy, Debug)]
pub struct PlayOptions {
    pub mouse_aim_snap: bool,
    pub controller_aim_snap: bool,
    pub left_stick_aim: bool,
    /// Screen shake factor 0..1.
    pub shake: f32,
    pub show_stat_numbers: bool,
}

impl Default for PlayOptions {
    fn default() -> Self {
        PlayOptions {
            mouse_aim_snap: false,
            controller_aim_snap: false,
            left_stick_aim: true,
            shake: 1.0,
            show_stat_numbers: false,
        }
    }
}

impl MatchConfig {
    /// A config with defaults for everything but the setup and controls.
    pub fn basic(
        setup: MatchSetup,
        controls: Vec<SlotControl>,
        seed: u64,
        sound: bool,
    ) -> MatchConfig {
        MatchConfig {
            setup,
            controls,
            bot_seed: seed,
            sound,
            cards: Vec::new(),
            faces: Vec::new(),
            options: PlayOptions::default(),
            bot_rematch: None,
            pick_mode: rounds_sim::PickMode::Auto,
        }
    }
}

/// Builds the sim content from the import (once per process).
pub fn build_content(data: &GameData) -> Result<Arc<Content>> {
    let mut built = rounds_content::build(data)
        .map_err(|r| anyhow::anyhow!("content conversion failed: {}", r.errors.join("; ")))?;
    let content = built.content();
    log::info!(
        "content: {} maps, {} cards, {} objects ({} warnings)",
        content.maps.len(),
        content.cards.len(),
        content.objects.len(),
        built.report.warnings.len()
    );
    Ok(Arc::new(content))
}

/// Counters for headless runs.
#[derive(Clone, Debug, Default)]
pub struct MatchStats {
    pub frames: u32,
    pub shots: u32,
    pub hits: u32,
    pub deaths: u32,
    pub explosions: u32,
    pub picks: u32,
    pub points: u32,
    pub rounds: u32,
    pub jumps: u32,
    pub blocks: u32,
}

pub struct MatchScene {
    data: Rc<GameData>,
    look: Rc<MainLook>,
    driver: Box<dyn MatchDriver>,
    controls: Vec<SlotControl>,
    bots: Bots,
    assets: Option<PlayAssets>,
    maps: Vec<(MapId, ShownMap)>,
    vis: Vec<PlayerVis>,
    fx: WorldFx,
    particles: Particles,
    /// Imported particle effects (psys).
    effects: effects::Effects,
    shake: Shaker,
    hud: hud::Hud,
    sounds: Sounds,
    filter: EventFilter,
    events: Vec<Event>,
    view: Option<ViewInfo>,
    rng: Rng,
    real_time: f32,
    /// Shake force factor (halved with more than 2 players, maps.md 9.1).
    shake_mult: f32,
    pub stats: MatchStats,
    /// Print flow events (headless runs).
    pub verbose: bool,
    faces: Vec<Vec<FaceItem>>,
    options: PlayOptions,
    bot_rematch: Option<bool>,
    /// Pause menu open: local play stops; online, local devices are muted.
    pub paused: bool,
    /// Draw this world instead of the driver's (confirmed snapshots for
    /// screenshots); cleared after one draw.
    pub shot_world: Option<rounds_sim::World>,
}

impl MatchScene {
    pub fn new(data: Rc<GameData>, content: Arc<Content>, cfg: MatchConfig) -> MatchScene {
        let mut local = LocalDriver::new(content, &cfg.setup).with_pick_mode(cfg.pick_mode);
        let unknown = local.give_cards(&cfg.cards);
        if !unknown.is_empty() {
            log::warn!("unknown cards: {}", unknown.join(", "));
        }
        Self::with_driver(data, Box::new(local), cfg, None, None)
    }

    /// A match on any driver; `assets`/`look` reuse the app's.
    pub fn with_driver(
        data: Rc<GameData>,
        driver: Box<dyn MatchDriver>,
        cfg: MatchConfig,
        assets: Option<PlayAssets>,
        look: Option<Rc<MainLook>>,
    ) -> MatchScene {
        let look = look.unwrap_or_else(|| Rc::new(MainLook::from_data(&data)));
        let n = cfg.setup.slots.len();
        let mut hud = hud::Hud::new(n);
        hud.show_stat_numbers = cfg.options.show_stat_numbers;
        hud.faces = cfg.faces.clone();
        let mut controls = cfg.controls;
        controls.resize(n, SlotControl::Bot);
        MatchScene {
            sounds: Sounds::new(data.clone(), cfg.sound, cfg.setup.seed),
            data,
            look,
            driver,
            controls,
            bots: Bots::new(cfg.bot_seed, n),
            assets,
            maps: Vec::new(),
            vis: (0..n).map(PlayerVis::new).collect(),
            fx: WorldFx::default(),
            particles: Particles::default(),
            effects: effects::Effects::default(),
            shake: Shaker::world(),
            hud,
            filter: EventFilter::default(),
            events: Vec::new(),
            view: None,
            rng: Rng::new(cfg.setup.seed ^ 0xfeed),
            real_time: 0.0,
            shake_mult: if n > 2 { 0.5 } else { 1.0 } * cfg.options.shake,
            stats: MatchStats::default(),
            verbose: false,
            faces: cfg.faces,
            options: cfg.options,
            bot_rematch: cfg.bot_rematch,
            paused: false,
            shot_world: None,
        }
    }

    /// Gives the shared assets back (when leaving the match).
    pub fn take_assets(&mut self) -> Option<PlayAssets> {
        self.assets.take()
    }

    pub fn assets(&self) -> Option<&PlayAssets> {
        self.assets.as_ref()
    }

    pub fn driver(&self) -> &dyn MatchDriver {
        self.driver.as_ref()
    }

    pub fn driver_mut(&mut self) -> &mut dyn MatchDriver {
        self.driver.as_mut()
    }

    pub fn outcome(&self) -> Option<MatchOutcome> {
        self.driver.outcome()
    }

    /// Stops sounds and music (leaving the match).
    pub fn stop_sounds(&mut self, audio: &mut crate::audio::Audio) {
        audio.stop_all();
    }

    /// Builds/drops the shown maps to match the sim's map state.
    fn reconcile_maps(&mut self, ctx: &mut LoadContext<'_>) {
        let Some(assets) = self.assets.as_mut() else {
            return;
        };
        let w = self.driver.current();
        let mv = w.map_view();
        let leaving_time = mv.since_call_in as f32 * DT;
        let wanted: Vec<MapId> = [
            mv.current,
            mv.loaded,
            mv.previous.filter(|_| leaving_time < 2.0),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.maps.retain(|(id, _)| wanted.contains(id));
        for id in wanted {
            if self.maps.iter().any(|(m, _)| *m == id) {
                continue;
            }
            let art = (self.rng.next() % self.look.arts.len().max(1) as u64) as usize;
            let content = self.driver.content().clone();
            let shown = ShownMap::new(
                ctx.gpu,
                ctx.renderer,
                &mut assets.textures,
                &self.data,
                &self.look,
                &content,
                id,
                art,
                mv.seed ^ id.0 as u32,
            );
            self.maps.push((id, shown));
        }
    }

    fn handle_event(&mut self, e: &Event, ctx: &mut TickContext<'_>) {
        use rounds_sim::EventKind as K;
        let w = self.driver.current();
        let content = self.driver.content().clone();
        let Some(assets) = self.assets.as_ref() else {
            return;
        };
        match &e.kind {
            K::Shake { vec } => self.shake.feel(v(*vec), self.shake_mult),
            K::ShotFired { .. } => self.stats.shots += 1,
            K::ProjectileHit { .. } => self.stats.hits += 1,
            K::Died { .. } => self.stats.deaths += 1,
            K::Explosion { .. } => self.stats.explosions += 1,
            K::CardPicked { .. } => self.stats.picks += 1,
            K::PointWon { .. } => self.stats.points += 1,
            K::RoundWon { .. } => self.stats.rounds += 1,
            K::Jumped { .. } => self.stats.jumps += 1,
            K::BlockStarted { .. } => self.stats.blocks += 1,
            _ => {}
        }
        if self.verbose {
            match &e.kind {
                K::MapLoaded { .. }
                | K::MapCallIn { .. }
                | K::BattleStarted
                | K::PointWon { .. }
                | K::RoundWon { .. }
                | K::GameWon { .. }
                | K::PickStarted { .. }
                | K::CardPicked { .. }
                | K::RematchPrompt
                | K::PlayerDisconnected { .. } => println!("frame {:5}: {:?}", e.frame, e.kind),
                _ => {}
            }
        }
        match &e.kind {
            K::Damaged { player, .. } => {
                if let Some(vis) = self.vis.get_mut(*player as usize) {
                    vis.damaged();
                }
                self.effects
                    .blink(*player as usize, Color::rgb(0.85, 0.85, 0.85));
            }
            K::Revived { player, .. } | K::PlayerSpawned { player, .. } => {
                if let Some(vis) = self.vis.get_mut(*player as usize) {
                    vis.spawned();
                }
            }
            K::BlockStarted { player, .. } => {
                if let Some(vis) = self.vis.get_mut(*player as usize) {
                    vis.block();
                }
            }
            _ => {}
        }
        // Imported particle effects first; the procedural ones are the
        // fallback for events without an imported effect.
        if !self.effects.on_event(e, w, &content, assets) {
            self.fx
                .on_event(e, w, &content, assets, &mut self.particles, &mut self.rng);
        }
        self.hud.on_event(e, w, &content);
        world_view::sound_for_event(e, w, &content, &mut self.sounds, ctx.audio);
    }
}

impl MatchScene {
    /// Prints sound events that resolved to no clips (headless report).
    pub fn report_sounds(&self) {
        println!(
            "sounds: {} played, {} events without clips{}",
            self.sounds.played,
            self.sounds.missing.len(),
            if self.sounds.missing.is_empty() {
                String::new()
            } else {
                format!(
                    ": {}",
                    self.sounds
                        .missing
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        );
    }
}

impl Scene for MatchScene {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        let assets = match self.assets.take() {
            Some(a) => a,
            None => PlayAssets::new(ctx.gpu, ctx.renderer, self.data.clone()),
        };
        self.hud
            .load(ctx.gpu, ctx.renderer, &assets, self.driver.content());
        let mut assets = assets;
        // ROUNDS_NO_PSYS=1: procedural effects only (comparisons).
        if !self.effects.loaded() && std::env::var_os("ROUNDS_NO_PSYS").is_none() {
            self.effects = effects::Effects::new(effects::EffectBank::load(
                ctx.gpu,
                ctx.renderer,
                &mut assets,
            ));
        }
        self.assets = Some(assets);
        self.reconcile_maps(ctx);
        Ok(())
    }

    fn poll(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        self.reconcile_maps(ctx);
        if let Some(a) = self.assets.as_mut() {
            let content = self.driver.content().clone();
            self.fx
                .poll(ctx.gpu, ctx.renderer, a, self.driver.current(), &content);
            self.hud.poll(
                ctx.gpu,
                ctx.renderer,
                a,
                self.driver.current(),
                self.driver.content(),
            );
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        if self.paused && self.driver.uses_local_pause() {
            return;
        }
        // Inputs per slot.
        let devices: Vec<_> = ctx.inputs.iter().map(|(d, _)| *d).collect();
        bind_controllers(&mut self.controls, &devices);
        let world = self.driver.current();
        let bot_inputs = if self
            .controls
            .iter()
            .any(|c| matches!(c, SlotControl::Bot | SlotControl::AnyController))
        {
            self.bots.inputs(world)
        } else {
            Vec::new()
        };
        let mut local: Vec<(usize, PlayerInput)> = Vec::new();
        let rematch = match world.phase() {
            Phase::Rematch { yes } => Some(yes),
            _ => None,
        };
        for (slot, c) in self.controls.iter().enumerate() {
            match c {
                SlotControl::Bot | SlotControl::AnyController => {
                    if let (Some(want), Some(yes)) = (self.bot_rematch, rematch) {
                        local.push((slot, rematch_input(world.frame(), want, yes)));
                    } else if let Some(i) = bot_inputs.get(slot) {
                        local.push((slot, *i));
                    }
                }
                SlotControl::Device(d) => {
                    let pos = world.players().get(slot).map_or(Vec2::ZERO, |p| v(p.pos));
                    let input = match ctx.inputs.iter().find(|(id, _)| id == d) {
                        Some((_, s)) if !self.paused => input_map::device_input_with(
                            *d,
                            s,
                            pos,
                            self.view.as_ref(),
                            &self.options,
                        ),
                        _ => PlayerInput::default(),
                    };
                    local.push((slot, input));
                }
                SlotControl::Orphan => local.push((slot, PlayerInput::default())),
                SlotControl::Remote => {}
            }
        }
        self.driver.advance(&local);
        self.stats.frames = self.driver.current().frame();

        // Events (each (frame, seq) once).
        let mut events = std::mem::take(&mut self.events);
        self.driver.drain_events(&mut events);
        for e in &events {
            if self.filter.first(e) {
                self.handle_event(e, ctx);
            }
        }
        events.clear();
        self.events = events;

        // Presentation state.
        let real = DT;
        let world = self.driver.current();
        let ts = world.time_scale();
        let game = DT * ts;
        if let Some(a) = &self.assets {
            let content = self.driver.content().clone();
            for (i, p) in world.players().iter().enumerate() {
                let skin = a.skin(p.skin as usize);
                let plants = if p.alive() && p.grounded {
                    world.foot_plants(&content, i)
                } else {
                    [None, None]
                };
                self.vis[i].update(p, plants, &skin, real, game);
            }
        }
        self.real_time += real;
        if std::env::var_os("ROUNDS_DEBUG_PLAYERS").is_some() && world.frame().is_multiple_of(20) {
            for p in world.players() {
                println!(
                    "f{} p{} pos ({:.2},{:.2}) gun ({:.2},{:.2}) aim ({:.2},{:.2}) scale {:.2} ground {} phase {:?}",
                    world.frame(),
                    p.id,
                    p.pos.x,
                    p.pos.y,
                    p.gun_pos.x,
                    p.gun_pos.y,
                    p.aim.x,
                    p.aim.y,
                    p.scale,
                    p.grounded,
                    world.phase()
                );
            }
        }
        self.particles.update(real, ts / 0.85);
        if let Some(a) = &self.assets {
            self.effects.update(world, a, real, game);
        }
        self.shake.update(game);
        self.fx.update(world, real, game);
        for (_, leg, pos, big) in self.fx.take_steps() {
            let cue = match (big, leg) {
                (false, 0) => "SFX_Character_Step_Left_SE",
                (false, _) => "SFX_Character_Step_Right_SE",
                (true, 0) => "SFX_Character_Step_Big_Left_SE",
                (true, _) => "SFX_Character_Step_Big_Right_SE",
            };
            self.sounds.play(ctx.audio, cue, Some(pos.x), 0.0);
        }
        self.hud.update(world, real, &mut self.sounds, ctx.audio);
        let battle =
            matches!(world.phase(), Phase::Battle | Phase::Resume { .. }) && world.pick().is_none();
        self.sounds.music(ctx.audio, true, battle);
        self.sounds.update(ctx.audio, real);
        let alive_loops = world_view::loop_keys(world);
        self.sounds
            .retain_loops(ctx.audio, |k| alive_loops.contains(&k));
        // Art particles of the shown map.
        let mv = world.map_view();
        let art_map = mv.current.or(mv.loaded);
        for (id, m) in &mut self.maps {
            m.spin_time += game;
            if Some(*id) == art_map {
                m.pres.update(real);
                m.update_fx(world, real, game);
            }
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        let Some(assets) = self.assets.as_ref() else {
            return;
        };
        let shot = self.shot_world.take();
        let (prev, cur, alpha) = match &shot {
            Some(w) => (w, w, 1.0),
            None => (
                self.driver.previous(),
                self.driver.current(),
                ctx.alpha.clamp(0.0, 1.0),
            ),
        };
        let mv = cur.map_view();
        let since = mv.since_call_in as f32 * DT + alpha * DT;
        let entering = matches!(mv.phase, rounds_sim::map::MapPhase::Entering(_));
        let art_map = mv.current.or(mv.loaded);

        // Frame settings from the map whose art is shown.
        *ctx.frame = FrameSettings::default();
        if let Some((_, m)) = self.maps.iter().find(|(id, _)| Some(*id) == art_map) {
            m.pres.frame_settings(&self.look, ctx.frame, ctx.size);
        }
        ctx.frame.camera.shake = self.shake.pos;
        self.view = Some(ViewInfo {
            camera: ctx.frame.camera,
            size: ctx.size,
        });

        let mut lights = true;
        for (id, m) in &self.maps {
            let motion = if Some(*id) == mv.current {
                if entering {
                    Motion::Entering(since)
                } else {
                    Motion::Settled
                }
            } else if Some(*id) == mv.previous && mv.current.is_some() {
                Motion::Leaving(since)
            } else {
                Motion::Waiting
            };
            if motion == Motion::Waiting && Some(*id) != art_map {
                continue;
            }
            let art = Some(*id) == art_map;
            m.draw(&self.look, cur, motion, ctx.list, art, lights);
            lights = false;
        }

        // Effects behind players, players, bullets, particles.
        let content = self.driver.content().clone();
        self.fx
            .draw_objects(cur, prev, alpha, &content, assets, ctx.list);
        let n = cur.players().len();
        let draws: Vec<_> = (0..n)
            .map(|i| {
                let p = &cur.players()[i];
                let q = prev.players().get(i).unwrap_or(p);
                let lerp = |a: rounds_sim::V2, b: rounds_sim::V2| v(a).lerp(v(b), alpha);
                player_view::PlayerDraw {
                    p,
                    pos: lerp(q.pos, p.pos),
                    gun_pos: lerp(q.gun_pos, p.gun_pos),
                    vis: &self.vis[i],
                    skin: assets.skin(p.skin as usize),
                    index: i,
                    sort: 2_000_000 + 100 * i as i32,
                    face: self
                        .faces
                        .get(i)
                        .filter(|f| !f.is_empty())
                        .map(|f| f.as_slice()),
                    effects: Some(&self.effects),
                }
            })
            .collect();
        for d in &draws {
            d.draw(assets, ctx.list);
        }
        for d in &draws {
            d.draw_bar(ctx.list, 2_900_000);
        }
        // Crown over the leading team's first living member.
        if let Some(team) = cur.leader()
            && let Some(d) = draws.iter().find(|d| d.p.team == team && d.p.alive())
        {
            d.draw_crown(ctx.list, 2_950_000);
        }
        self.fx.draw_projectiles(cur, prev, alpha, assets, ctx.list);
        self.particles.draw(ctx.list, 3_500_000);
        self.effects.draw(ctx.list);

        // HUD and card choice.
        self.hud.draw(cur, &content, assets, &self.vis, ctx);
        letterbox(ctx.list, ctx.size);
    }
}

/// Rematch prompt input of a bot that wants `want` (yes/no): move to it,
/// then confirm with jump presses.
fn rematch_input(frame: u32, want: bool, yes: bool) -> PlayerInput {
    if want != yes {
        let x = if want { -1.0 } else { 1.0 };
        return PlayerInput::from_axes(rounds_sim::v2(x, 0.0), rounds_sim::V2::ZERO, 0);
    }
    let buttons = if (frame / 10).is_multiple_of(2) {
        PlayerInput::BTN_JUMP
    } else {
        0
    };
    PlayerInput {
        buttons,
        ..Default::default()
    }
}

/// Darkens everything below `sort` on the world layer (card choice).
pub fn dim(list: &mut DrawList, sort: i32, amount: f32) {
    list.rect(
        Layer::World,
        sort,
        Vec2::ZERO,
        Vec2::new(400.0, 200.0),
        0.0,
        Color::rgba(0.0, 0.0, 0.0, amount),
    );
}
