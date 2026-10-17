//! The application flow (ui.md 2): main menu, local and online lobbies,
//! options, the pause menu and the match, as one [`Scene`].
//!
//! Pages form a stack (`Back` pops). Starting a match keeps the lobby page
//! on the stack; while the match runs, pages above it are overlays (pause,
//! options). When the match ends the lobby gets the outcome and is shown
//! again.

pub mod editor;
pub mod lobby;
pub mod online;
pub mod overlay;
pub mod pages;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use glam::Vec2;
use rounds_import::GameData;
use rounds_render::{Color, Layer};
use rounds_sim::{Content, MatchSetup};

use crate::app::{
    AppRequests, DisplayRequest, DrawContext, LoadContext, PlatformInfo, Scene, TickContext,
};
use crate::art::MainLook;
use crate::audio::{Audio, Bus};
use crate::faces::{FaceCatalog, FaceSprites};
use crate::input::DeviceId;
use crate::online::NetRef;
use crate::play::assets::{FaceItem, PlayAssets};
use crate::play::driver::{MatchDriver, MatchOutcome};
use crate::play::input_map::SlotControl;
use crate::play::sound::Sounds;
use crate::play::{MatchConfig, MatchScene, PlayOptions};
use crate::settings::{Face, Settings};
use crate::ui::background::MenuBackground;
use crate::ui::strings::Strings;
use crate::ui::{Canvas, Frame, Router, UiInput, style};

/// Command-line options that shape the app flow (tests, launcher).
#[derive(Clone, Debug, Default)]
pub struct AppOptions {
    /// First screen (headless screenshots): `main`, `local`, `options`,
    /// `controls`, `lobby`, `lobby12`, `editor`, `register`, `online`,
    /// `host`, `browse`, `code`, `onlinelobby`, `pause`.
    pub screen: Option<String>,
    /// Automated online run (tests): host or join a room with bot players.
    pub auto: Option<online::AutoPlan>,
    /// `--warpzone-join` / `--warpzone-host` / invite link arguments.
    pub launch_args: Vec<String>,
    /// Username to register on first use without asking.
    pub username: Option<String>,
    /// Sound on (off for silent headless runs).
    pub sound: bool,
}

/// What every page can use.
pub struct Shared {
    pub data: Rc<GameData>,
    pub content: Arc<Content>,
    pub look: Rc<MainLook>,
    pub settings: Settings,
    pub strings: Strings,
    pub catalog: FaceCatalog,
    pub assets: Option<PlayAssets>,
    pub faces: Option<FaceSprites>,
    pub bg: Option<MenuBackground>,
    pub sounds: Sounds,
    pub net: Option<NetRef>,
    pub opts: AppOptions,
    /// Intro logos (Landfall, ROUNDS), loaded with the assets.
    pub logos: [Option<crate::play::assets::SpriteRef>; 2],
    /// Controller button glyphs for prompts.
    pub glyphs: crate::ui::Glyphs,
    /// The intro is on screen (menu music waits for it).
    pub intro: bool,
    /// Ticks since start.
    pub t: u64,
}

impl Shared {
    /// UI sound cue (ui.md 6).
    pub fn sfx(&mut self, audio: &mut Audio, cue: &str) {
        self.sounds.play(audio, cue, None, 0.0);
    }

    pub fn tr(&self, key: &str, fallback: &str) -> String {
        self.strings.tr(key, fallback)
    }

    pub fn up(&self, key: &str, fallback: &str) -> String {
        self.strings.up(key, fallback)
    }

    /// Face preset `i`.
    pub fn preset(&self, i: usize) -> Face {
        self.settings.faces.get(i).copied().unwrap_or_default()
    }

    pub fn face_items(&self, face: &Face) -> Vec<FaceItem> {
        self.faces
            .as_ref()
            .map(|f| f.items(face))
            .unwrap_or_default()
    }

    /// Mixer volumes from the settings.
    pub fn apply_audio(&self, audio: &mut Audio) {
        let s = &self.settings;
        audio.set_master_volume(Settings::gain(s.master_volume));
        audio.set_bus_volume(Bus::Sfx, Settings::gain(s.sfx_volume));
        audio.set_bus_volume(Bus::Ui, Settings::gain(s.sfx_volume));
        audio.set_bus_volume(Bus::Music, Settings::gain(s.music_volume));
    }

    pub fn display_request(&self) -> DisplayRequest {
        DisplayRequest {
            mode: self.settings.display_mode,
            resolution: self.settings.resolution,
            vsync: self.settings.vsync,
        }
    }

    pub fn play_options(&self) -> PlayOptions {
        let s = &self.settings;
        PlayOptions {
            mouse_aim_snap: s.mouse_aim_snap,
            controller_aim_snap: s.controller_aim_snap,
            left_stick_aim: s.left_stick_aim,
            shake: s.screen_shake,
            show_stat_numbers: s.show_stat_numbers,
        }
    }

    /// Skin colour of colour index `c` (for UI swatches).
    pub fn colour(&self, skin: usize) -> Color {
        self.assets
            .as_ref()
            .map(|a| a.skin(skin).color)
            .unwrap_or(style::GREY)
    }

    pub fn win_colour(&self, skin: usize) -> Color {
        self.assets
            .as_ref()
            .map(|a| a.skin(skin).win_text)
            .unwrap_or(style::TEXT)
    }
}

/// Everything a page's tick gets.
pub struct Cx<'a> {
    pub input: &'a UiInput,
    /// Raw key / button presses this tick (binding capture).
    pub raw: &'a [crate::input::RawPress],
    pub frame: &'a mut Frame,
    pub shared: &'a mut Shared,
    pub audio: &'a mut Audio,
    /// Canvas width in reference pixels.
    pub width: f32,
    pub platform: &'a PlatformInfo,
    pub requests: &'a mut AppRequests,
}

/// A match to start.
pub struct Launch {
    pub setup: MatchSetup,
    /// Pick mode of a local match (online drivers set theirs).
    pub pick_mode: rounds_sim::PickMode,
    pub controls: Vec<SlotControl>,
    /// Face per slot.
    pub faces: Vec<Face>,
    /// `None` = local driver.
    pub driver: Option<Box<dyn MatchDriver>>,
    pub bot_seed: u64,
    pub bot_rematch: Option<bool>,
    /// Headless: screenshots of these (confirmed) sim frames.
    pub shots: Vec<u32>,
    pub shot_prefix: Option<PathBuf>,
    /// Online: (name, team, skin) per slot for the "VS" intro.
    pub vs_names: Vec<(String, u8, usize)>,
}

pub enum Action {
    None,
    Push(Box<dyn Page>),
    Replace(Box<dyn Page>),
    Pop,
    StartMatch(Box<Launch>),
    /// Pause menu: close it.
    Resume,
    /// Pause menu: leave the match (back to the lobby / menu).
    LeaveMatch,
    Quit(i32),
}

pub trait Page {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action;
    /// A text field has focus (SDL text input on, letters do not navigate).
    fn text_input(&self) -> bool {
        false
    }
    /// The match ended; the page under it gets the outcome.
    fn match_ended(&mut self, _outcome: &MatchOutcome, _shared: &mut Shared) {}
    /// Called when the page becomes the top page again.
    fn resumed(&mut self, _shared: &mut Shared) {}
    /// Leave-match from the pause menu landed here (online: leave room).
    fn left_match(&mut self, _shared: &mut Shared) {}
}

struct Running {
    scene: MatchScene,
    /// Stack depth of the page that started it.
    base: usize,
    shot_prefix: Option<PathBuf>,
    online: bool,
    ticks: u64,
    vs_names: Vec<(String, u8, usize)>,
}

pub struct GameApp {
    shared: Shared,
    pages: Vec<Box<dyn Page>>,
    game: Option<Running>,
    router: Router,
    canvas: Canvas,
    frame: Frame,
    text_on: bool,
    pending: Option<Box<Launch>>,
    exit: Option<i32>,
    /// Locale whose system CJK fallback font was looked up (once each).
    cjk_for: Option<String>,
    /// The online "Match found / VS" overlay of the running match.
    vs_frame: Frame,
}

impl GameApp {
    pub fn new(data: Rc<GameData>, content: Arc<Content>, opts: AppOptions) -> GameApp {
        let look = Rc::new(MainLook::from_data(&data));
        let settings = Settings::load(&data.localization.locales);
        let strings = Strings::new(data.clone(), &settings.language);
        let catalog = FaceCatalog::load(&data);
        let sounds = Sounds::new(data.clone(), opts.sound, 7);
        let mut shared = Shared {
            data,
            content,
            look,
            settings,
            strings,
            catalog,
            assets: None,
            faces: None,
            bg: None,
            sounds,
            net: None,
            opts,
            logos: [None, None],
            glyphs: Default::default(),
            intro: false,
            t: 0,
        };
        // Fresh presets: different expressions.
        if shared.settings.faces.iter().all(|f| *f == Face::default()) {
            let p = shared.catalog.default_presets();
            for (i, f) in p.into_iter().enumerate() {
                if i < shared.settings.faces.len() {
                    shared.settings.faces[i] = f;
                }
            }
        }
        let pages = pages::initial_pages(&mut shared);
        GameApp {
            shared,
            pages,
            game: None,
            router: Router::new(),
            canvas: Canvas::new((1920, 1080)),
            frame: Frame::default(),
            text_on: false,
            pending: None,
            exit: None,
            cjk_for: None,
            vs_frame: Frame::default(),
        }
    }

    fn start_match(&mut self, l: Launch, ctx: &mut LoadContext<'_>) -> Result<()> {
        let sh = &mut self.shared;
        let faces: Vec<Vec<FaceItem>> = l.faces.iter().map(|f| sh.face_items(f)).collect();
        let mut cfg = MatchConfig::basic(l.setup.clone(), l.controls, l.bot_seed, sh.opts.sound);
        cfg.faces = faces;
        cfg.options = sh.play_options();
        cfg.bot_rematch = l.bot_rematch;
        let driver: Box<dyn MatchDriver> = match l.driver {
            Some(d) => d,
            None => Box::new(
                crate::play::driver::LocalDriver::new(sh.content.clone(), &l.setup)
                    .with_pick_mode(l.pick_mode),
            ),
        };
        let online = driver.net_status().is_some();
        let mut scene = MatchScene::with_driver(
            sh.data.clone(),
            driver,
            cfg,
            sh.assets.take(),
            Some(sh.look.clone()),
        );
        scene.load(ctx)?;
        for &f in &l.shots {
            scene.driver_mut().request_snapshot(f);
        }
        self.game = Some(Running {
            scene,
            base: self.pages.len(),
            shot_prefix: l.shot_prefix,
            online,
            ticks: 0,
            vs_names: l.vs_names,
        });
        Ok(())
    }

    fn end_match(&mut self, outcome: Option<MatchOutcome>, audio: &mut Audio) {
        let Some(mut g) = self.game.take() else {
            return;
        };
        g.scene.stop_sounds(audio);
        let st = &g.scene.stats;
        log::info!(
            "match over: {} sim frames, {} jumps, {} shots, {} blocks, {} hits, {} deaths, {} picks, {} points, {} rounds",
            st.frames,
            st.jumps,
            st.shots,
            st.blocks,
            st.hits,
            st.deaths,
            st.picks,
            st.points,
            st.rounds
        );
        if let Some(a) = g.scene.take_assets() {
            self.shared.assets = Some(a);
        }
        self.pages.truncate(g.base);
        if let Some(p) = self.pages.last_mut() {
            match &outcome {
                Some(o) => p.match_ended(o, &mut self.shared),
                None => p.left_match(&mut self.shared),
            }
            p.resumed(&mut self.shared);
        }
        self.shared.apply_audio(audio);
    }

    fn apply(&mut self, action: Action, audio: &mut Audio) {
        match action {
            Action::None => {}
            Action::Push(p) => self.pages.push(p),
            Action::Replace(p) => {
                self.pages.pop();
                self.pages.push(p);
            }
            Action::Pop => {
                if self.pages.len() > 1 {
                    self.pages.pop();
                }
                if let Some(p) = self.pages.last_mut() {
                    p.resumed(&mut self.shared);
                }
                if let Some(g) = &mut self.game
                    && self.pages.len() <= g.base
                {
                    g.scene.paused = false;
                }
            }
            Action::StartMatch(l) => self.pending = Some(l),
            Action::Resume => {
                if let Some(g) = &mut self.game {
                    self.pages.truncate(g.base);
                    g.scene.paused = false;
                }
            }
            Action::LeaveMatch => self.end_match(None, audio),
            Action::Quit(code) => self.exit = Some(code),
        }
    }
}

impl Scene for GameApp {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        let mut assets = PlayAssets::new(ctx.gpu, ctx.renderer, self.shared.data.clone());
        let faces = FaceSprites::new(ctx.gpu, ctx.renderer, &mut assets, &self.shared.catalog);
        self.shared.glyphs = crate::ui::Glyphs::load(ctx.gpu, ctx.renderer, &mut assets);
        self.shared.logos = [
            assets.sprite_named(ctx.gpu, ctx.renderer, "LandfallLogo"),
            assets.sprite_named(ctx.gpu, ctx.renderer, "Rounds_Logo2_White"),
        ];
        let bg = MenuBackground::new(
            ctx.gpu,
            ctx.renderer,
            &mut assets.textures,
            &self.shared.data,
            &self.shared.look,
        );
        self.shared.assets = Some(assets);
        self.shared.faces = Some(faces);
        self.shared.bg = Some(bg);
        self.shared.apply_audio(ctx.audio);
        Ok(())
    }

    fn poll(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        // Japanese / Chinese / Korean: a system font behind the UI fonts.
        let lang = self.shared.settings.language.clone();
        if self.cjk_for.as_deref() != Some(lang.as_str()) {
            self.cjk_for = Some(lang.clone());
            if let Some(script) = crate::sysfonts::cjk_lang(&lang) {
                load_cjk_font(ctx.renderer, script);
            }
        }
        if let Some(l) = self.pending.take() {
            self.start_match(*l, ctx)?;
        }
        if let Some(g) = &mut self.game {
            g.scene.poll(ctx)?;
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.shared.t += 1;
        if self.shared.t == 1 {
            ctx.requests.bindings = Some(self.shared.settings.bindings.clone());
        }
        // Apply the saved window mode once (the window opens windowed).
        if self.shared.t == 1 && !ctx.platform.headless {
            let s = &self.shared.settings;
            if s.display_mode != crate::settings::DisplayMode::Windowed
                || s.resolution.is_some()
                || !s.vsync
            {
                ctx.requests.display = Some(self.shared.display_request());
            }
        }
        if let Some(code) = self.exit {
            ctx.requests.quit = true;
            ctx.requests.exit_code = code;
            return;
        }
        let text_mode = self.pages.last().is_some_and(|p| p.text_input());
        if text_mode != self.text_on {
            self.text_on = text_mode;
            ctx.requests.text_input = Some(text_mode);
        }
        let input = self
            .router
            .update(ctx.inputs, ctx.text, &self.canvas, text_mode);
        if let Some(bg) = &mut self.shared.bg
            && self.game.is_none()
        {
            bg.update(ctx.dt);
        }
        self.shared.sounds.update(ctx.audio, ctx.dt);
        let in_menu = self.game.is_none() && self.pending.is_none() && !self.shared.intro;
        self.shared.sounds.menu_music(ctx.audio, in_menu);

        // The match (with the pause toggle).
        if let Some(g) = &mut self.game {
            let paused = self.pages.len() > g.base;
            let toggle = input.devs.iter().any(|(d, n)| {
                (matches!(d, DeviceId::Controller(_)) && n.start)
                    || (*d == DeviceId::KeyboardMouse && n.back)
            });
            let mut opened = false;
            if toggle && !paused {
                self.pages.push(Box::new(pages::PausePage::new(g.online)));
                opened = true;
            }
            g.scene.paused = self.pages.len() > g.base;
            g.scene.tick(ctx);
            g.ticks += 1;
            // Online: "Match found" (0.5 s), then the names with "VS" (2 s)
            // (ui.md 2.8) over the start of the match.
            self.vs_frame.clear();
            if g.online && !g.vs_names.is_empty() && g.ticks < VS_TICKS {
                if g.ticks == 1 {
                    self.shared.sfx(ctx.audio, "UI_Match_Found_SE");
                }
                let colours: Vec<Color> = g
                    .vs_names
                    .iter()
                    .map(|n| {
                        g.scene
                            .assets()
                            .map_or(style::TEXT, |a| a.skin(n.2).win_text)
                    })
                    .collect();
                record_vs(
                    &mut self.vs_frame,
                    &self.shared,
                    &g.vs_names,
                    &colours,
                    g.ticks as f32 * crate::timing::FIXED_DT as f32,
                );
            }
            // Automated runs: leave / crash at a frame (drop tests).
            if let Some(plan) = &self.shared.opts.auto {
                let f = g.scene.driver().current().frame();
                if plan.crash_at.is_some_and(|c| f >= c) {
                    println!("crashing at frame {f}");
                    std::process::exit(3);
                }
                if plan.leave_at.is_some_and(|c| f >= c) {
                    println!("leaving at frame {f}");
                    if let Some(n) = &self.shared.net {
                        n.borrow_mut().leave();
                        let t = std::time::Instant::now();
                        while t.elapsed() < std::time::Duration::from_millis(500) {
                            n.borrow_mut().poll();
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    }
                    ctx.requests.quit = true;
                    return;
                }
            }
            // `--screen pause`: open the pause menu over a running match.
            if self.shared.opts.screen.as_deref() == Some("pause")
                && g.ticks == 400
                && self.pages.len() <= g.base
            {
                self.pages.push(Box::new(pages::PausePage::new(false)));
                g.scene.paused = true;
            }
            // Screenshots of confirmed frames (headless test runs).
            if let Some(w) = g.scene.driver_mut().take_snapshot()
                && let Some(prefix) = &g.shot_prefix
            {
                let f = w.frame();
                let stem = prefix
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "shot".into());
                ctx.requests.screenshot = Some(prefix.with_file_name(format!("{stem}-f{f}.png")));
                g.scene.shot_world = Some(w);
                println!("SNAPSHOT frame {f}");
            }
            if let Some(o) = g.scene.outcome() {
                // Online: the driver reports; local: the flow's `Ended`.
                self.end_match(Some(o), ctx.audio);
                self.frame.clear();
                return;
            }
            if opened || self.pages.len() <= g.base {
                // No overlay to tick this time (a fresh pause page starts
                // next tick so the opening press does not act on it).
                if self.pages.len() <= g.base {
                    self.frame.clear();
                }
                return;
            }
        }
        self.frame.clear();
        let Some(page) = self.pages.last_mut() else {
            ctx.requests.quit = true;
            return;
        };
        let mut cx = Cx {
            input: &input,
            raw: ctx.raw,
            frame: &mut self.frame,
            shared: &mut self.shared,
            audio: ctx.audio,
            width: self.canvas.width(),
            platform: ctx.platform,
            requests: ctx.requests,
        };
        let action = page.tick(&mut cx);
        self.apply(action, ctx.audio);
        if let Some(code) = self.exit {
            ctx.requests.quit = true;
            ctx.requests.exit_code = code;
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        self.canvas = Canvas::new(ctx.size);
        let sh = &self.shared;
        if let Some(g) = &mut self.game {
            let snapshot = g.scene.shot_world.is_some();
            g.scene.draw(ctx);
            if g.online
                && !snapshot
                && let Some(st) = g.scene.driver().net_status()
                && let Some(a) = g.scene.assets()
            {
                overlay::draw_net_status(ctx.list, &self.canvas, a, &st);
            }
            if !self.vs_frame.cmds.is_empty()
                && !snapshot
                && let Some(a) = g.scene.assets()
            {
                let size = Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
                ctx.list.rect(
                    Layer::Screen,
                    900_000,
                    size * 0.5,
                    size,
                    0.0,
                    Color::rgba(0.0, 0.0, 0.0, 0.85),
                );
                self.vs_frame
                    .draw(ctx.list, &self.canvas, a, false, 900_001);
            }
            if self.pages.len() > g.base
                && let Some(a) = g.scene.assets()
            {
                let size = Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
                ctx.list.rect(
                    Layer::Screen,
                    1_000_000,
                    size * 0.5,
                    size,
                    0.0,
                    Color::rgba(0.0, 0.0, 0.0, 0.6),
                );
                self.frame.draw(ctx.list, &self.canvas, a, false, 1_000_001);
            }
            return;
        }
        *ctx.frame = Default::default();
        if let Some(bg) = &sh.bg {
            bg.draw(&sh.look, ctx.frame, ctx.list, ctx.size);
        }
        if let Some(a) = &sh.assets {
            self.frame.draw(ctx.list, &self.canvas, a, false, 1_000);
        }
        crate::map_scene::letterbox(ctx.list, ctx.size);
    }
}

/// Ticks of the online match intro (0.5 s "Match found" + 2 s "VS").
const VS_TICKS: u64 = (2.5 / crate::timing::FIXED_DT) as u64;

/// "MATCH FOUND", then every side's names (in their colours) with "VS"
/// between the sides; up to 6 sides per row.
fn record_vs(f: &mut Frame, sh: &Shared, names: &[(String, u8, usize)], colours: &[Color], t: f32) {
    use crate::ui::Font;
    if t < 0.5 {
        f.text(
            sh.up("PROMPT_MATCHFOUND", "Match found"),
            Vec2::new(0.0, 540.0),
            120.0,
            Vec2::splat(0.5),
            Font::Light,
            style::TEXT,
        );
        return;
    }
    let mut teams: Vec<u8> = names.iter().map(|n| n.1).collect();
    teams.sort_unstable();
    teams.dedup();
    let per_row = teams.len().min(6);
    let rows = teams.len().div_ceil(per_row.max(1));
    let col_w = 1700.0 / per_row.max(1) as f32;
    let vs = sh.up("PROMPT_VS", "VS");
    for (ti, team) in teams.iter().enumerate() {
        let (r, c) = (ti / per_row, ti % per_row);
        let in_row = if r == rows - 1 {
            teams.len() - r * per_row
        } else {
            per_row
        };
        let x = (c as f32 - (in_row as f32 - 1.0) * 0.5) * col_w;
        let y0 = 540.0 + (r as f32 - (rows as f32 - 1.0) * 0.5) * 380.0;
        let members: Vec<(&(String, u8, usize), Color)> = names
            .iter()
            .zip(colours.iter().copied())
            .filter(|(n, _)| n.1 == *team)
            .collect();
        let size = if members.len() > 3 || per_row > 3 {
            44.0
        } else {
            70.0
        };
        let h = size * 1.2;
        for (k, ((name, _, _), colour)) in members.iter().enumerate() {
            let y = y0 + (k as f32 - (members.len() as f32 - 1.0) * 0.5) * h;
            f.text(
                name.to_uppercase(),
                Vec2::new(x, y),
                size,
                Vec2::splat(0.5),
                Font::Regular,
                *colour,
            );
        }
        if c + 1 < in_row {
            f.text(
                vs.clone(),
                Vec2::new(x + col_w * 0.5, y0),
                90.0,
                Vec2::splat(0.5),
                Font::Bold,
                style::TEXT,
            );
        }
    }
}

/// Loads the first system font that has `script`'s glyphs and makes it the
/// renderer's fallback for missing characters (sysfonts.rs).
fn load_cjk_font(renderer: &mut rounds_render::Renderer, script: &str) {
    let probe = crate::sysfonts::probe_char(script);
    for (path, index) in crate::sysfonts::cjk_candidates(script).into_iter().take(4) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        match renderer.load_font_indexed(&bytes, index) {
            Ok(id) if renderer.font_has_char(id, probe) => {
                log::info!(
                    "{script} text: system font {} (face {index})",
                    path.display()
                );
                renderer.set_fallback_fonts(&[id]);
                return;
            }
            Ok(_) => log::info!("{}: no {script} glyphs", path.display()),
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
    }
    log::warn!("no system font with {script} glyphs found; such text shows as boxes");
}
