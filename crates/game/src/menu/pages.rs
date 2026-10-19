//! Main menu, Local (Versus / Sandbox), Options, Controls, Map pool, Mods
//! and the pause menu (ui.md 2.3, 2.4, 2.7, 2.9).

use glam::Vec2;

use super::lobby::LobbyPage;
use super::online;
use super::{Action, Cx, Page, Shared};
use crate::settings::{DisplayMode, language_label};
use crate::ui::{Font, ListEvent, ListNav, Rect, bar, style};

/// The page stack to start with (`--screen`, automated online runs,
/// launcher arguments).
pub fn initial_pages(sh: &mut Shared) -> Vec<Box<dyn Page>> {
    if let Some(plan) = sh.opts.auto.clone() {
        return vec![Box::new(online::AutoPage::new(plan))];
    }
    let screen = sh.opts.screen.clone().unwrap_or_default();
    let main: Box<dyn Page> = Box::new(MainPage::default());
    let local = || -> Box<dyn Page> { Box::new(LocalModePage::default()) };
    match screen.as_str() {
        "local" => vec![main, local()],
        "options" => vec![main, Box::new(OptionsPage::default())],
        "controls" => vec![
            main,
            Box::new(OptionsPage::default()),
            Box::new(ControlsPage::default()),
        ],
        "maps" => vec![main, local(), Box::new(MapPoolPage::default())],
        "mods" => vec![main, local(), Box::new(ModsPage::default())],
        "lobby" => vec![main, local(), Box::new(LobbyPage::local(false))],
        "lobby12" => vec![main, local(), Box::new(LobbyPage::demo(sh, 12, false))],
        "pause" => vec![main, Box::new(StartBotsPage(4))],
        "lobby-teams" => vec![main, local(), Box::new(LobbyPage::demo(sh, 12, true))],
        "editor" => {
            let mut l = LobbyPage::demo(sh, 3, false);
            l.open_editor_demo();
            vec![main, local(), Box::new(l)]
        }
        "register" | "online" | "host" | "browse" | "code" | "onlinelobby" => {
            let mut v: Vec<Box<dyn Page>> = vec![main];
            v.extend(online::demo_pages(sh, &screen));
            v
        }
        _ => {
            let launch = online::launch_intent(&sh.opts.launch_args);
            if launch {
                vec![main, Box::new(online::OnlineHomePage::new(true))]
            } else if screen == "intro" || (screen.is_empty() && sh.opts.sound) {
                // The intro (ui.md 2.1) once per start of the game.
                sh.intro = true;
                vec![main, Box::new(IntroPage::default())]
            } else {
                vec![main]
            }
        }
    }
}

/// The intro (ui.md 2.1): after 1 s the Landfall logo pops in (its
/// CurveAnimation: scale 0 -> 1 in 0.18 s, held to 0.54 s, then out in
/// 0.2 s), then the ROUNDS logo the same way, then the main menu. Any
/// button, click or key skips it.
#[derive(Default)]
pub struct IntroPage {
    t: f32,
    sounds: u8,
}

/// Scale of a logo `t` seconds after its PlayIn (in 0.543 s, out 0.2 s).
fn logo_scale(t: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t < 0.543 {
        let x = (t / 0.177).min(1.0);
        1.0 - (1.0 - x).powi(3)
    } else {
        let x = ((t - 0.543) / 0.2).min(1.0);
        1.0 - x * x
    }
}

impl Page for IntroPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        self.t += 1.0 / 71.428_57;
        let (t1, t2, end) = (1.0, 1.743, 2.486);
        if self.sounds == 0 && self.t >= t1 {
            self.sounds = 1;
            cx.shared.sfx(cx.audio, "SFX_Shot_Grow_Single_SE");
        }
        if self.sounds == 1 && self.t >= t2 {
            self.sounds = 2;
            cx.shared.sfx(cx.audio, "SFX_Character_Block_Blocked_SE");
        }
        let skip = cx.input.nav.any || cx.input.click || cx.input.right_click;
        if self.t >= end || (skip && self.t > 0.1) {
            cx.shared.intro = false;
            return Action::Pop;
        }
        let tint = rounds_render::Color::rgb(0.95, 0.9, 0.86);
        let [lf, rl] = cx.shared.logos;
        let k = logo_scale(self.t - t1);
        if k > 0.0
            && let Some(s) = lf
        {
            let h = 420.0 * k;
            let w = h * s.rect_size.x / s.rect_size.y.max(1e-6);
            cx.frame
                .sprite(s, Vec2::new(0.0, 540.0), Vec2::new(w, h), tint);
        }
        let k = logo_scale(self.t - t2);
        if k > 0.0 {
            match rl {
                Some(s) => {
                    let w = 1300.0 * k;
                    let h = w * s.rect_size.y / s.rect_size.x.max(1e-6);
                    cx.frame
                        .sprite(s, Vec2::new(0.0, 540.0), Vec2::new(w, h), tint);
                }
                None => cx.frame.text(
                    "ROUNDS",
                    Vec2::new(0.0, 540.0),
                    300.0 * k,
                    Vec2::splat(0.5),
                    Font::UltraLight,
                    tint,
                ),
            }
        }
        Action::None
    }
}

/// A centred column of big menu buttons; plays the hover/click sounds.
#[allow(clippy::too_many_arguments)]
pub fn button_column(
    cx: &mut Cx<'_>,
    nav: &mut ListNav,
    labels: &[String],
    enabled: &[bool],
    top: f32,
    size: f32,
    spacing: f32,
) -> ListEvent {
    let rects: Vec<Rect> = (0..labels.len())
        .map(|i| {
            Rect::new(
                Vec2::new(0.0, top + i as f32 * spacing),
                Vec2::new(900.0, spacing * 0.92),
            )
        })
        .collect();
    let ev = nav.update(cx.input, &rects, enabled);
    if ev.moved {
        cx.shared.sfx(cx.audio, "UI_Button_Hover");
    }
    if ev.activate.is_some() {
        cx.shared.sfx(cx.audio, "UI_Button_Click");
    }
    for (i, l) in labels.iter().enumerate() {
        let r = rects[i];
        if i == nav.focus {
            bar(
                cx.frame,
                cx.input,
                Rect::new(r.c, Vec2::new(cx.width.min(2200.0), size * 1.25)),
            );
        }
        let (c, f) = nav.style(i, enabled.get(i).copied().unwrap_or(true));
        cx.frame.text(
            l.clone(),
            r.c + Vec2::new(nav.shake(i), 0.0),
            size,
            Vec2::splat(0.5),
            f,
            c,
        );
    }
    ev
}

/// Page title (Gravity Light, like the banner texts).
pub fn title(cx: &mut Cx<'_>, text: &str, y: f32) {
    cx.frame.text(
        text,
        Vec2::new(0.0, y),
        110.0,
        Vec2::splat(0.5),
        Font::Light,
        style::TEXT,
    );
}

/// A hint line at the bottom of the screen.
pub fn hint(cx: &mut Cx<'_>, text: &str) {
    cx.frame.text(
        text,
        Vec2::new(0.0, 1040.0),
        30.0,
        Vec2::splat(0.5),
        Font::Regular,
        style::GREY,
    );
}

/// Starts a local bot match at once (`--screen pause`).
pub struct StartBotsPage(pub usize);

impl Page for StartBotsPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        Action::StartMatch(Box::new(super::lobby::bot_launch(cx.shared, self.0)))
    }
}

#[derive(Default)]
pub struct MainPage {
    nav: ListNav,
}

impl Page for MainPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        // Logo (ui.md 2.3): the pink ROUNDS wordmark.
        cx.frame.text(
            "ROUNDS",
            Vec2::new(0.0, 250.0),
            300.0,
            Vec2::splat(0.5),
            Font::UltraLight,
            style::PINK,
        );
        let sh = &cx.shared;
        let labels = vec![
            sh.up("BUTTON_LOCAL", "Local"),
            sh.up("BUTTON_ONLINE", "Online"),
            sh.up("BUTTON_OPTIONS", "Options"),
            sh.up("BUTTON_QUIT", "Quit"),
        ];
        let ev = button_column(cx, &mut self.nav, &labels, &[true; 4], 560.0, 80.0, 115.0);
        cx.frame.text(
            format!("rounds-port {}", env!("CARGO_PKG_VERSION")),
            Vec2::new(-cx.width * 0.5 + 30.0, 1050.0),
            26.0,
            Vec2::new(0.0, 0.5),
            Font::Regular,
            style::DIM,
        );
        match ev.activate {
            Some(0) => Action::Push(Box::new(LocalModePage::default())),
            Some(1) => Action::Push(Box::new(online::OnlineHomePage::new(false))),
            Some(2) => Action::Push(Box::new(OptionsPage::default())),
            Some(3) => Action::Quit(0),
            _ => Action::None,
        }
    }
}

#[derive(Default)]
pub struct LocalModePage {
    nav: ListNav,
}

impl Page for LocalModePage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, &cx.shared.up("BUTTON_LOCAL", "Local"), 250.0);
        let sh = &cx.shared;
        let labels = vec![
            sh.up("BUTTON_VERSUS", "Versus"),
            sh.up("BUTTON_SANDBOX", "Sandbox"),
            sh.up("BUTTON_BACK", "Back"),
        ];
        let ev = button_column(cx, &mut self.nav, &labels, &[true; 3], 520.0, 80.0, 115.0);
        if ev.back {
            return Action::Pop;
        }
        match ev.activate {
            Some(0) => Action::Push(Box::new(LobbyPage::local(false))),
            Some(1) => Action::Push(Box::new(LobbyPage::local(true))),
            Some(2) => Action::Pop,
            _ => Action::None,
        }
    }
}

// ------------------------------------------------------------- options

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opt {
    Resolution,
    Fullscreen,
    Language,
    Vsync,
    Master,
    Sfx,
    Music,
    Shake,
    LeftStick,
    MouseSnap,
    PadSnap,
    StatNumbers,
    Controls,
    Back,
}

const OPTS: [Opt; 14] = [
    Opt::Resolution,
    Opt::Fullscreen,
    Opt::Language,
    Opt::Vsync,
    Opt::Master,
    Opt::Sfx,
    Opt::Music,
    Opt::Shake,
    Opt::LeftStick,
    Opt::MouseSnap,
    Opt::PadSnap,
    Opt::StatNumbers,
    Opt::Controls,
    Opt::Back,
];

#[derive(Default)]
pub struct OptionsPage {
    nav: ListNav,
    /// Slider being dragged with the mouse.
    drag: Option<Opt>,
}

fn yes_no(sh: &Shared, b: bool) -> String {
    if b {
        sh.up("OPTION_YES", "Yes")
    } else {
        sh.up("OPTION_NO", "No")
    }
}

impl OptionsPage {
    fn label(&self, sh: &Shared, o: Opt) -> String {
        match o {
            Opt::Resolution => sh.up("OPTION_RESOLUTION", "Resolution"),
            Opt::Fullscreen => sh.up("OPTION_FULLSCREEN", "Fullscreen"),
            Opt::Language => sh.up("OPTION_LANGUAGE", "Language"),
            Opt::Vsync => sh.up("OPTION_VSYNC", "VSync"),
            Opt::Master => sh.up("OPTION_VOLUME_MASTER", "Master volume"),
            Opt::Sfx => sh.up("OPTION_VOLUME_SFX", "SFX volume"),
            Opt::Music => sh.up("OPTION_VOLUME_MUSIC", "Music volume"),
            Opt::Shake => "SCREEN SHAKE".into(),
            Opt::LeftStick => sh.up("OPTION_LEFTSTICKAIMING", "Aim with left stick"),
            Opt::MouseSnap => sh.up("OPTION_MOUSE_AIM8DIR", "Mouse aim 8 dir"),
            Opt::PadSnap => sh.up("OPTION_CONTROLLER_AIM8DIR", "Controller aim 8 dir"),
            Opt::StatNumbers => sh.up("OPTION_SHOWSTATNUMBERS", "Show stat numbers"),
            Opt::Controls => "CONTROLS".into(),
            Opt::Back => sh.up("BUTTON_BACK", "Back"),
        }
    }

    fn value(&self, sh: &Shared, cx_res: &[(u32, u32)], o: Opt) -> Option<String> {
        let s = &sh.settings;
        Some(match o {
            Opt::Resolution => {
                let r = s
                    .resolution
                    .or(cx_res.first().copied())
                    .unwrap_or((1920, 1080));
                format!("{}X{}", r.0, r.1)
            }
            Opt::Fullscreen => {
                let (k, f) = s.display_mode.label();
                sh.up(k, f)
            }
            Opt::Language => {
                let (k, f) = language_label(&s.language);
                // Each language names itself (the original's labels are
                // localised; the native name is clearer in a list).
                let _ = k;
                f.to_uppercase()
            }
            Opt::Vsync => yes_no(sh, s.vsync),
            Opt::LeftStick => yes_no(sh, s.left_stick_aim),
            Opt::MouseSnap => yes_no(sh, s.mouse_aim_snap),
            Opt::PadSnap => yes_no(sh, s.controller_aim_snap),
            Opt::StatNumbers => yes_no(sh, s.show_stat_numbers),
            _ => return None,
        })
    }

    fn slider(&self, sh: &Shared, o: Opt) -> Option<f32> {
        let s = &sh.settings;
        match o {
            Opt::Master => Some(s.master_volume),
            Opt::Sfx => Some(s.sfx_volume),
            Opt::Music => Some(s.music_volume),
            Opt::Shake => Some(s.screen_shake),
            _ => None,
        }
    }

    fn set_slider(sh: &mut Shared, o: Opt, v: f32) {
        let v = (v * 20.0).round() / 20.0;
        let v = v.clamp(0.0, 1.0);
        let s = &mut sh.settings;
        match o {
            Opt::Master => s.master_volume = v,
            Opt::Sfx => s.sfx_volume = v,
            Opt::Music => s.music_volume = v,
            Opt::Shake => s.screen_shake = v,
            _ => {}
        }
    }

    /// Applies a change (dir -1/+1, 0 = activate) and saves.
    fn change(&mut self, cx: &mut Cx<'_>, o: Opt, dir: i32) {
        let res = cx.platform.resolutions.clone();
        let sh = &mut *cx.shared;
        let step = |i: usize, n: usize, d: i32| -> usize {
            if n == 0 {
                0
            } else {
                ((i as i32 + if d == 0 { 1 } else { d }).rem_euclid(n as i32)) as usize
            }
        };
        let mut display = false;
        match o {
            Opt::Resolution => {
                if !res.is_empty() {
                    let cur = sh.settings.resolution.or(res.first().copied());
                    let i = res.iter().position(|r| Some(*r) == cur).unwrap_or(0);
                    sh.settings.resolution = Some(res[step(i, res.len(), -dir)]);
                    display = true;
                }
            }
            Opt::Fullscreen => {
                let all = DisplayMode::ALL;
                let i = all
                    .iter()
                    .position(|m| *m == sh.settings.display_mode)
                    .unwrap_or(0);
                sh.settings.display_mode = all[step(i, all.len(), dir)];
                display = true;
            }
            Opt::Language => {
                let ls = sh.strings.locales();
                let i = ls
                    .iter()
                    .position(|l| *l == sh.settings.language)
                    .unwrap_or(0);
                sh.settings.language = ls[step(i, ls.len(), dir)].clone();
                sh.strings.locale = sh.settings.language.clone();
            }
            Opt::Vsync => {
                sh.settings.vsync = !sh.settings.vsync;
                display = true;
            }
            Opt::LeftStick => sh.settings.left_stick_aim = !sh.settings.left_stick_aim,
            Opt::MouseSnap => sh.settings.mouse_aim_snap = !sh.settings.mouse_aim_snap,
            Opt::PadSnap => sh.settings.controller_aim_snap = !sh.settings.controller_aim_snap,
            Opt::StatNumbers => sh.settings.show_stat_numbers = !sh.settings.show_stat_numbers,
            Opt::Master | Opt::Sfx | Opt::Music | Opt::Shake => {
                let v = self.slider(sh, o).unwrap_or(1.0);
                let d = if dir == 0 { 1 } else { dir };
                Self::set_slider(sh, o, v + 0.1 * d as f32);
            }
            Opt::Controls | Opt::Back => return,
        }
        sh.apply_audio(cx.audio);
        if display {
            cx.requests.display = Some(sh.display_request());
        }
        sh.settings.save();
    }
}

const SLIDER_W: f32 = 380.0;

impl Page for OptionsPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, &cx.shared.up("BUTTON_OPTIONS", "Options"), 95.0);
        let row_h = 62.0;
        let top = 200.0;
        let rects: Vec<Rect> = (0..OPTS.len())
            .map(|i| {
                Rect::new(
                    Vec2::new(0.0, top + i as f32 * row_h),
                    Vec2::new(1300.0, row_h * 0.95),
                )
            })
            .collect();
        let ev = self.nav.update(cx.input, &rects, &[true; OPTS.len()]);
        if ev.moved {
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        // Mouse on sliders: press and drag sets the value.
        let slider_x = 650.0 - SLIDER_W;
        if cx.input.mouse_held
            && let Some(p) = cx.input.pointer
        {
            if self.drag.is_none() && cx.input.click {
                self.drag = rects
                    .iter()
                    .position(|r| r.contains(p))
                    .map(|i| OPTS[i])
                    .filter(|o| self.slider(cx.shared, *o).is_some() && p.x > slider_x - 20.0);
            }
            if let Some(o) = self.drag {
                Self::set_slider(cx.shared, o, (p.x - slider_x) / SLIDER_W);
                cx.shared.apply_audio(cx.audio);
            }
        } else if self.drag.take().is_some() {
            cx.shared.settings.save();
        }
        let mut action = Action::None;
        if let Some((i, d)) = ev.change {
            self.change(cx, OPTS[i], d);
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        if let Some(i) = ev.activate
            && self.drag.is_none()
        {
            cx.shared.sfx(cx.audio, "UI_Button_Click");
            match OPTS[i] {
                Opt::Back => action = Action::Pop,
                Opt::Controls => action = Action::Push(Box::new(ControlsPage::default())),
                o if self.slider(cx.shared, o).is_some() => {}
                o => self.change(cx, o, 0),
            }
        }
        if ev.back {
            action = Action::Pop;
        }
        let res = cx.platform.resolutions.clone();
        for (i, o) in OPTS.iter().enumerate() {
            let r = rects[i];
            if i == self.nav.focus {
                bar(cx.frame, cx.input, r);
            }
            let (c, f) = self.nav.style(i, true);
            let label = self.label(cx.shared, *o);
            if matches!(o, Opt::Controls | Opt::Back) {
                cx.frame.text(label, r.c, 46.0, Vec2::splat(0.5), f, c);
                continue;
            }
            cx.frame.text(
                label,
                Vec2::new(-630.0, r.c.y),
                40.0,
                Vec2::new(0.0, 0.5),
                f,
                c,
            );
            if let Some(v) = self.slider(cx.shared, *o) {
                let y = r.c.y;
                cx.frame.rect(
                    Rect::new(
                        Vec2::new(slider_x + SLIDER_W * 0.5, y),
                        Vec2::new(SLIDER_W, 10.0),
                    ),
                    5.0,
                    style::DIM,
                );
                cx.frame.rect(
                    Rect::new(
                        Vec2::new(slider_x + SLIDER_W * v * 0.5, y),
                        Vec2::new(SLIDER_W * v, 10.0),
                    ),
                    5.0,
                    style::TEXT,
                );
                cx.frame
                    .circle(Vec2::new(slider_x + SLIDER_W * v, y), 14.0, style::TEXT);
                cx.frame.text(
                    format!("{}", (v * 100.0).round() as i32),
                    Vec2::new(slider_x - 30.0, y),
                    34.0,
                    Vec2::new(1.0, 0.5),
                    Font::Regular,
                    style::GREY,
                );
            } else if let Some(v) = self.value(cx.shared, &res, *o) {
                let arrows = i == self.nav.focus && !cx.input.mouse_mode;
                let txt = if arrows { format!("< {v} >") } else { v };
                cx.frame.text(
                    txt,
                    Vec2::new(650.0, r.c.y),
                    40.0,
                    Vec2::new(1.0, 0.5),
                    f,
                    c,
                );
            }
        }
        action
    }
}

/// Controls (player.md 3.2, docs/platform.md "Input"): the bindings of
/// every scheme as a table; pick a cell (keys / D-pad / mouse) and press
/// the new key or button to remap it (Esc cancels). Saved in
/// `settings.json` and applied at once.
#[derive(Default)]
pub struct ControlsPage {
    /// Focused cell: row (actions, then the buttons row) and column.
    row: usize,
    col: usize,
    /// Waiting for a key/button for (action row, scheme column); armed
    /// from the next tick so the press that started it is not taken.
    capture: Option<(usize, usize, bool)>,
    mouse_seen: bool,
}

impl ControlsPage {
    fn apply(cx: &mut Cx<'_>) {
        cx.shared.settings.save();
        cx.requests.bindings = Some(cx.shared.settings.bindings.clone());
    }
}

impl Page for ControlsPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        use crate::input::{ACTIONS, DeviceId, SCHEMES, Scheme};
        title(cx, "CONTROLS", 80.0);
        let n_rows = ACTIONS.len();
        let w = (cx.width - 160.0).min(1700.0);
        let label_w = 300.0;
        let col_w = (w - label_w) / SCHEMES.len() as f32;
        let x0 = -w * 0.5 + label_w;
        let row_h = 74.0;
        let top = 250.0;
        let cell = |r: usize, c: usize| {
            Rect::new(
                Vec2::new(x0 + col_w * (c as f32 + 0.5), top + row_h * r as f32),
                Vec2::new(col_w - 12.0, row_h - 10.0),
            )
        };
        let buttons = [
            Rect::new(
                Vec2::new(-220.0, top + row_h * n_rows as f32 + 60.0),
                Vec2::new(380.0, 70.0),
            ),
            Rect::new(
                Vec2::new(220.0, top + row_h * n_rows as f32 + 60.0),
                Vec2::new(380.0, 70.0),
            ),
        ];
        let editable =
            |r: usize, c: usize| !(SCHEMES[c] == Scheme::Controller && ACTIONS[r].is_move());

        // Capture a key / button.
        if let Some((r, c, armed)) = self.capture {
            if !armed {
                self.capture = Some((r, c, true));
            } else {
                let pad = SCHEMES[c] == Scheme::Controller;
                for p in cx.raw {
                    if p.name == "Escape" {
                        self.capture = None;
                        break;
                    }
                    let ok = match p.device {
                        DeviceId::Controller(_) => {
                            pad && !matches!(p.name.as_str(), "start" | "back" | "guide")
                        }
                        _ => !pad,
                    };
                    if ok {
                        cx.shared.settings.bindings.set(
                            SCHEMES[c],
                            ACTIONS[r],
                            vec![p.name.clone()],
                        );
                        Self::apply(cx);
                        cx.shared.sfx(cx.audio, "UI_Button_Click");
                        self.capture = None;
                        break;
                    }
                }
            }
        } else {
            let nav = &cx.input.nav;
            let before = (self.row, self.col);
            if nav.up {
                self.row = (self.row + n_rows) % (n_rows + 1);
            }
            if nav.down {
                self.row = (self.row + 1) % (n_rows + 1);
            }
            let cols = if self.row == n_rows { 2 } else { SCHEMES.len() };
            self.col = self.col.min(cols - 1);
            if nav.left {
                self.col = (self.col + cols - 1) % cols;
            }
            if nav.right {
                self.col = (self.col + 1) % cols;
            }
            let mut activate = nav.confirm;
            if cx.input.mouse_mode
                && let Some(p) = cx.input.pointer
            {
                self.mouse_seen = true;
                for r in 0..n_rows {
                    for c in 0..SCHEMES.len() {
                        if cell(r, c).contains(p) {
                            (self.row, self.col) = (r, c);
                        }
                    }
                }
                for (i, b) in buttons.iter().enumerate() {
                    if b.contains(p) {
                        (self.row, self.col) = (n_rows, i);
                    }
                }
                let over = (0..n_rows).any(|r| (0..SCHEMES.len()).any(|c| cell(r, c).contains(p)))
                    || buttons.iter().any(|b| b.contains(p));
                activate |= cx.input.click && over;
            }
            if (self.row, self.col) != before {
                cx.shared.sfx(cx.audio, "UI_Button_Hover");
            }
            if activate {
                if self.row == n_rows {
                    cx.shared.sfx(cx.audio, "UI_Button_Click");
                    if self.col == 0 {
                        cx.shared.settings.bindings = Default::default();
                        Self::apply(cx);
                    } else {
                        return Action::Pop;
                    }
                } else if editable(self.row, self.col) {
                    cx.shared.sfx(cx.audio, "UI_Button_Click");
                    self.capture = Some((self.row, self.col, false));
                }
            }
            if cx.input.nav.back {
                return Action::Pop;
            }
        }

        // Table.
        for (c, sc) in SCHEMES.iter().enumerate() {
            cx.frame.text(
                sc.label(),
                Vec2::new(x0 + col_w * (c as f32 + 0.5), top - 70.0),
                28.0,
                Vec2::splat(0.5),
                Font::Bold,
                style::TEXT,
            );
        }
        for (r, a) in ACTIONS.iter().enumerate() {
            let y = top + row_h * r as f32;
            cx.frame.text(
                a.label(),
                Vec2::new(-w * 0.5, y),
                30.0,
                Vec2::new(0.0, 0.5),
                Font::Regular,
                style::GREY,
            );
            for (c, sc) in SCHEMES.iter().enumerate() {
                let rc = cell(r, c);
                let focused = (self.row, self.col) == (r, c);
                let capturing = self.capture.is_some_and(|(cr, cc, _)| (cr, cc) == (r, c));
                cx.frame.rect(
                    rc,
                    10.0,
                    if capturing {
                        style::BAR
                    } else if focused && (!cx.input.mouse_mode || self.mouse_seen) {
                        style::PANEL_LIGHT
                    } else {
                        style::PANEL
                    },
                );
                let txt = if capturing {
                    if *sc == Scheme::Controller {
                        "PRESS A BUTTON".to_string()
                    } else {
                        "PRESS A KEY".to_string()
                    }
                } else {
                    cx.shared.settings.bindings.describe(*sc, *a)
                };
                cx.frame.text(
                    txt,
                    rc.c,
                    if capturing { 26.0 } else { 24.0 },
                    Vec2::splat(0.5),
                    if focused { Font::Bold } else { Font::Regular },
                    if editable(r, c) {
                        style::TEXT
                    } else {
                        style::GREY
                    },
                );
            }
        }
        for (i, (b, label)) in buttons
            .iter()
            .zip([
                "RESET DEFAULTS".to_string(),
                cx.shared.up("BUTTON_BACK", "Back"),
            ])
            .enumerate()
        {
            let focused = (self.row, self.col) == (n_rows, i);
            if focused {
                bar(cx.frame, cx.input, *b);
            }
            cx.frame.text(
                label,
                b.c,
                40.0,
                Vec2::splat(0.5),
                if focused { Font::Bold } else { Font::Light },
                style::TEXT,
            );
        }
        let hint = if self.capture.is_some() {
            "ESC CANCELS"
        } else {
            "MENUS: ARROWS / WASD, ENTER, ESC · PAUSE: ESC / START · KEYBOARD RIGHT LEAVES THE LOBBY: DELETE · AIM: MOUSE / RIGHT STICK / I J K L"
        };
        hint_line(cx, hint);
        Action::None
    }
}

fn hint_line(cx: &mut Cx<'_>, s: &str) {
    cx.frame.text(
        s,
        Vec2::new(0.0, 1040.0),
        22.0,
        Vec2::splat(0.5),
        Font::Regular,
        style::DIM,
    );
}

/// Maps the match picks from (lobby setting).
/// One row of the [`ModsPage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModRow {
    /// On / off of `rounds_sim::mods::ALL[i]`.
    Toggle(usize),
    PickNPicks,
    PickNHand,
    Back,
}

/// Built-in mods (docs/MODS.md): one on/off row per mod, its settings
/// below it while it is on, and the original author in the hint line.
/// Stored in the lobby rules; online only the host gets here.
#[derive(Default)]
pub struct ModsPage {
    nav: ListNav,
}

impl ModsPage {
    fn rows(mods: &rounds_sim::Mods) -> Vec<ModRow> {
        let mut v = Vec::new();
        for (i, m) in rounds_sim::mods::ALL.iter().enumerate() {
            v.push(ModRow::Toggle(i));
            if m.id == rounds_sim::mods::PICK_N_CARDS.id && mods.pick_n_cards.is_some() {
                v.push(ModRow::PickNPicks);
                v.push(ModRow::PickNHand);
            }
        }
        v.push(ModRow::Back);
        v
    }

    fn change(mods: &mut rounds_sim::Mods, row: ModRow, dir: i32) {
        use rounds_sim::mods::{ALL, PICK_N_CARDS, PickNCards};
        let cyc =
            |v: u8, hi: u8| -> u8 { (((v as i32 - 1 + dir).rem_euclid(hi as i32)) + 1) as u8 };
        match row {
            ModRow::Toggle(i) if ALL[i].id == PICK_N_CARDS.id => {
                mods.pick_n_cards = match mods.pick_n_cards {
                    Some(_) => None,
                    None => Some(PickNCards::default()),
                };
            }
            ModRow::PickNPicks => {
                if let Some(p) = &mut mods.pick_n_cards {
                    p.picks = cyc(p.picks, PickNCards::MAX_PICKS);
                }
            }
            ModRow::PickNHand => {
                if let Some(p) = &mut mods.pick_n_cards {
                    p.hand_size = cyc(p.hand_size, PickNCards::MAX_HAND);
                }
            }
            _ => {}
        }
    }

    fn text(sh: &Shared, mods: &rounds_sim::Mods, row: ModRow) -> (String, String) {
        use rounds_sim::mods::{ALL, PICK_N_CARDS};
        match row {
            ModRow::Toggle(i) => {
                let on = ALL[i].id == PICK_N_CARDS.id && mods.pick_n_cards.is_some();
                (ALL[i].name.to_uppercase(), yes_no(sh, on))
            }
            ModRow::PickNPicks => (
                "    PICKS PER PHASE".into(),
                mods.pick_n_cards.map_or(1, |p| p.picks).to_string(),
            ),
            ModRow::PickNHand => ("    CARDS PER HAND".into(), mods.hand_size().to_string()),
            ModRow::Back => (sh.up("BUTTON_BACK", "Back"), String::new()),
        }
    }
}

impl Page for ModsPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, "MODS", 95.0);
        let rows = Self::rows(&cx.shared.settings.rules.mods);
        let row_h = 62.0;
        let top = 220.0;
        let rects: Vec<Rect> = (0..rows.len())
            .map(|i| {
                Rect::new(
                    Vec2::new(0.0, top + i as f32 * row_h),
                    Vec2::new(1300.0, row_h * 0.95),
                )
            })
            .collect();
        let ev = self.nav.update(cx.input, &rects, &vec![true; rows.len()]);
        if ev.moved {
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        let mut action = Action::None;
        let mut changed = None;
        if let Some((i, d)) = ev.change {
            changed = Some((rows[i], d));
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        if let Some(i) = ev.activate {
            cx.shared.sfx(cx.audio, "UI_Button_Click");
            match rows[i] {
                ModRow::Back => action = Action::Pop,
                r => changed = Some((r, 1)),
            }
        }
        if let Some((r, d)) = changed {
            Self::change(&mut cx.shared.settings.rules.mods, r, d);
            cx.shared.settings.save();
        }
        if ev.back {
            action = Action::Pop;
        }
        let mods = cx.shared.settings.rules.mods;
        let rows = Self::rows(&mods);
        self.nav.focus = self.nav.focus.min(rows.len() - 1);
        for (i, row) in rows.iter().enumerate() {
            let r = Rect::new(
                Vec2::new(0.0, top + i as f32 * row_h),
                Vec2::new(1300.0, row_h * 0.95),
            );
            if i == self.nav.focus {
                bar(cx.frame, cx.input, r);
            }
            let (c, f) = self.nav.style(i, true);
            let (label, value) = Self::text(cx.shared, &mods, *row);
            if *row == ModRow::Back {
                cx.frame.text(label, r.c, 46.0, Vec2::splat(0.5), f, c);
                continue;
            }
            cx.frame.text(
                label,
                Vec2::new(-630.0, r.c.y),
                40.0,
                Vec2::new(0.0, 0.5),
                f,
                c,
            );
            let arrows = i == self.nav.focus && !cx.input.mouse_mode;
            let txt = if arrows {
                format!("< {value} >")
            } else {
                value
            };
            cx.frame.text(
                txt,
                Vec2::new(650.0, r.c.y),
                40.0,
                Vec2::new(1.0, 0.5),
                f,
                c,
            );
        }
        // Credit of the focused mod (or of its settings rows).
        let owner = rows[..=self.nav.focus.min(rows.len() - 1)]
            .iter()
            .rev()
            .find_map(|r| match r {
                ModRow::Toggle(i) => Some(*i),
                _ => None,
            });
        if let Some(i) = owner
            && rows[self.nav.focus] != ModRow::Back
        {
            let m = &rounds_sim::mods::ALL[i];
            hint(
                cx,
                &format!(
                    "{} AFTER THE MOD BY {}",
                    m.about.to_uppercase(),
                    m.author.to_uppercase()
                ),
            );
        } else {
            hint(
                cx,
                "BUILT-IN VERSIONS OF POPULAR COMMUNITY MODS. ALL OFF = THE ORIGINAL RULES.",
            );
        }
        action
    }
}

#[derive(Default)]
pub struct MapPoolPage {
    focus: usize,
    router_t: u32,
}

impl Page for MapPoolPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        self.router_t += 1;
        title(cx, "MAPS", 80.0);
        let names: Vec<String> = cx
            .shared
            .content
            .maps
            .iter()
            .map(|m| m.name.clone())
            .collect();
        let n = names.len();
        let cols = 7usize;
        let cw = ((cx.width - 80.0).min(1840.0)) / cols as f32;
        let rows = n.div_ceil(cols).max(1);
        let rh = (820.0 / rows as f32).min(60.0);
        let x0 = -cw * cols as f32 * 0.5;
        // Items: maps, then ALL, NONE, BACK.
        let total = n + 3;
        let mut rects: Vec<Rect> = (0..n)
            .map(|i| {
                Rect::new(
                    Vec2::new(
                        x0 + cw * ((i % cols) as f32 + 0.5),
                        160.0 + rh * ((i / cols) as f32 + 0.5),
                    ),
                    Vec2::new(cw - 8.0, rh - 6.0),
                )
            })
            .collect();
        for (k, x) in [-400.0f32, 0.0, 400.0].iter().enumerate() {
            let _ = k;
            rects.push(Rect::new(Vec2::new(*x, 1010.0), Vec2::new(360.0, 70.0)));
        }
        let input = cx.input;
        let nav = input.nav;
        let f = self.focus.min(total - 1);
        let mut f2 = f;
        if f < n {
            if nav.left && !f.is_multiple_of(cols) {
                f2 = f - 1;
            }
            if nav.right && f % cols + 1 < cols && f + 1 < n {
                f2 = f + 1;
            }
            if nav.up && f >= cols {
                f2 = f - cols;
            }
            if nav.down {
                f2 = if f + cols < n {
                    f + cols
                } else {
                    n + ((f % cols) * 3 / cols).min(2)
                };
            }
        } else {
            if nav.left && f > n {
                f2 = f - 1;
            }
            if nav.right && f + 1 < total {
                f2 = f + 1;
            }
            if nav.up {
                f2 = n.saturating_sub(1);
            }
        }
        if input.mouse_mode
            && let Some(p) = input.pointer
            && let Some(i) = rects.iter().position(|r| r.contains(p))
        {
            f2 = i;
        }
        if f2 != f {
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        self.focus = f2;
        let mut act = nav.confirm.then_some(self.focus);
        if input.click
            && let Some(p) = input.pointer
        {
            act = rects.iter().position(|r| r.contains(p)).or(act);
        }
        let ex = &mut cx.shared.settings.rules.maps.excluded;
        let mut action = Action::None;
        if let Some(i) = act {
            if i < n {
                let id = i as u16;
                if let Some(k) = ex.iter().position(|x| *x == id) {
                    ex.remove(k);
                } else if ex.len() + 1 < n {
                    ex.push(id);
                    ex.sort();
                }
            } else if i == n {
                ex.clear();
            } else if i == n + 1 {
                *ex = (1..n as u16).collect(); // keep one map
            } else {
                action = Action::Pop;
            }
            cx.shared.settings.save();
            cx.shared.sfx(cx.audio, "UI_Button_Click");
        }
        if nav.back {
            cx.shared.settings.save();
            action = Action::Pop;
        }
        let ex = cx.shared.settings.rules.maps.excluded.clone();
        for (i, name) in names.iter().enumerate() {
            let r = rects[i];
            let on = !ex.contains(&(i as u16));
            let focused = i == self.focus;
            cx.frame.rect(
                r,
                8.0,
                if focused {
                    style::BAR
                } else {
                    style::PANEL_LIGHT
                },
            );
            cx.frame.circle(
                Vec2::new(r.left() + 22.0, r.c.y),
                9.0,
                if on { style::GOOD } else { style::DIM },
            );
            cx.frame.text(
                name.trim_end_matches("_M").replace('_', " "),
                Vec2::new(r.left() + 42.0, r.c.y),
                (rh * 0.45).min(26.0),
                Vec2::new(0.0, 0.5),
                if focused { Font::Bold } else { Font::Regular },
                if on { style::TEXT } else { style::DIM },
            );
        }
        for (k, l) in ["ALL", "NONE", "BACK"].iter().enumerate() {
            let i = n + k;
            let r = rects[i];
            if i == self.focus {
                bar(cx.frame, cx.input, r);
            }
            cx.frame.text(
                *l,
                r.c,
                46.0,
                Vec2::splat(0.5),
                if i == self.focus {
                    Font::Bold
                } else {
                    Font::Light
                },
                style::TEXT,
            );
        }
        cx.frame.text(
            format!("{} OF {} MAPS", n - ex.len(), n),
            Vec2::new(0.0, 135.0),
            28.0,
            Vec2::splat(0.5),
            Font::Regular,
            style::GREY,
        );
        action
    }
}

/// Escape menu (ui.md 2.9): Resume, Options, Menu (leave match), Quit.
pub struct PausePage {
    nav: ListNav,
    online: bool,
}

impl PausePage {
    pub fn new(online: bool) -> PausePage {
        PausePage {
            nav: ListNav::default(),
            online,
        }
    }
}

impl Page for PausePage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        if self.online {
            cx.frame.text(
                "THE MATCH KEEPS RUNNING",
                Vec2::new(0.0, 330.0),
                32.0,
                Vec2::splat(0.5),
                Font::Regular,
                style::GREY,
            );
        }
        let sh = &cx.shared;
        let labels = vec![
            sh.up("BUTTON_RESUME", "Resume"),
            sh.up("BUTTON_OPTIONS", "Options"),
            sh.up("BUTTON_MAINMENU", "Menu"),
            sh.up("BUTTON_QUIT", "Quit"),
        ];
        let ev = button_column(cx, &mut self.nav, &labels, &[true; 4], 430.0, 76.0, 110.0);
        // Esc / Start again closes it.
        let close = cx.input.devs.iter().any(|(d, n)| {
            (matches!(d, crate::input::DeviceId::Controller(_)) && n.start)
                || (*d == crate::input::DeviceId::KeyboardMouse && n.back)
        });
        if close || ev.back {
            return Action::Resume;
        }
        match ev.activate {
            Some(0) => Action::Resume,
            Some(1) => Action::Push(Box::new(OptionsPage::default())),
            Some(2) => Action::LeaveMatch,
            Some(3) => Action::Quit(0),
            _ => Action::None,
        }
    }
}
