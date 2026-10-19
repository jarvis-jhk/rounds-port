//! HUD and match-flow screens (ui.md 3-5, match-flow.md 4-11): the small
//! round counter, the point/round visualiser balls, banner texts, card
//! bars, the card choice screen (cards from the imported `CardBase` and
//! card-art prefabs, the picker's big face with its hands) and the win /
//! rematch screen.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Affine2, Vec2};
use rounds_render::{Color, DrawList, FontId, Gpu, Layer, Mask, Primitive, Renderer, Text};
use rounds_sim::content::Rarity;
use rounds_sim::flow::{Phase, PickStage};
use rounds_sim::{CardId, Content, Event, EventKind as K, World};

use super::DT;
use super::assets::{PlayAssets, Skin};
use super::fx::{FeelOverTime, Key, Rng, Shaker, curve};
use super::player_view::PlayerVis;
use super::sound::Sounds;
use super::ui_prefab::{ArtAnim, UiPrefab, image_sprite};
use crate::app::DrawContext;
use crate::audio::Audio;

/// Card slots: offset from the choice root (0, -1.5) and z rotation
/// (match-flow.md 9.5).
const SLOTS: [(f32, f32, f32); 5] = [
    (-25.0, 0.0, 15.0),
    (-13.0, 6.85, 7.5),
    (0.0, 9.66, 0.0),
    (13.0, 6.85, -7.5),
    (25.0, 0.0, -15.0),
];
const CHOICE_ROOT: Vec2 = Vec2::new(0.0, -1.5);
/// Most cards a hand can have (the Pick N Cards mod, docs/MODS.md).
const MAX_HAND: usize = rounds_sim::mods::PickNCards::MAX_HAND as usize;
const FACE_POS: Vec2 = Vec2::new(0.0, -18.0);
const FACE_SCALE: f32 = 33.0;

/// Point visualiser ball curves (ui.md 3.2).
const MOVE_CURVE: [Key; 3] = [
    Key(0.0, 0.0, 0.0, 0.0),
    Key(0.4998, 0.505, 2.40, 2.40),
    Key(1.0, 1.0, -0.043, -0.043),
];
const SCALE_CURVE: [Key; 3] = [
    Key(0.0, 0.0, 0.0, 0.0),
    Key(0.5058, 0.5115, 2.94, 2.94),
    Key(1.0, 1.0, 0.0, 0.0),
];
/// Card end animation (9.4): covers half the way, accelerating.
const END_CURVE: [Key; 2] = [Key(0.0, 0.0, 0.0, 0.0), Key(1.0, 0.502, 2.914, 0.027)];

/// Card theme colours (ui.md 1.2): (target, background).
const THEMES: [([f32; 3], [f32; 3]); 9] = [
    ([0.802, 0.276, 0.276], [0.179, 0.063, 0.063]),
    ([0.792, 0.734, 0.228], [0.198, 0.182, 0.066]),
    ([0.314, 0.515, 0.820], [0.053, 0.106, 0.189]),
    ([0.679, 0.679, 0.679], [0.151, 0.151, 0.151]),
    ([0.478, 0.314, 0.820], [0.09, 0.06, 0.15]),
    ([0.0, 0.575, 0.300], [0.030, 0.094, 0.051]),
    ([0.571, 0.802, 0.125], [0.105, 0.151, 0.041]),
    ([0.314, 0.737, 0.820], [0.026, 0.091, 0.104]),
    ([0.820, 0.314, 0.534], [0.132, 0.036, 0.077]),
];

const COLOR_NAMES: [&str; 12] = [
    "ORANGE", "BLUE", "RED", "GREEN", "YELLOW", "PURPLE", "CYAN", "PINK", "LIME", "INDIGO", "SKY",
    "AMBER",
];

fn c3(a: [f32; 3]) -> Color {
    Color::rgb(a[0], a[1], a[2])
}

/// Thrown-away card of the end animation.
#[derive(Clone, Copy, Debug)]
struct Thrown {
    slot: usize,
    card: CardId,
    pos: Vec2,
    vel: Vec2,
    rot: f32,
    spin: f32,
    scale: f32,
    life: f32,
    picked: bool,
}

pub struct Hud {
    /// Numeric stat amounts instead of words (`OPTION_SHOWSTATNUMBERS`).
    pub show_stat_numbers: bool,
    /// Face items per player (empty = the default face).
    pub faces: Vec<Vec<super::assets::FaceItem>>,
    card_base: Option<UiPrefab>,
    arts: HashMap<CardId, Option<ArtAnim>>,
    /// HUD time the card art animators were last advanced to.
    art_time: f32,
    slot_scale: [f32; MAX_HAND],
    slot_vel: [f32; MAX_HAND],
    /// Simultaneous picks: card scale per player and slot.
    sim_scale: [[f32; MAX_HAND]; 12],
    hands: [(Vec2, Vec2); 2],
    pick_player: Option<u8>,
    face_in: f32,
    last_hand: Vec<CardId>,
    thrown: Vec<Thrown>,
    time: f32,
    ui_shake: Shaker,
    feel: FeelOverTime,
    seq_sounds: u8,
    seq_start: Option<u32>,
    rng: Rng,
    pick_shake: Vec2,
}

impl Hud {
    pub fn new(_players: usize) -> Hud {
        Hud {
            show_stat_numbers: false,
            faces: Vec::new(),
            card_base: None,
            arts: HashMap::new(),
            art_time: 0.0,
            slot_scale: [0.0; MAX_HAND],
            slot_vel: [0.0; MAX_HAND],
            sim_scale: [[0.0; MAX_HAND]; 12],
            hands: [
                (Vec2::new(-12.0, -8.0), Vec2::ZERO),
                (Vec2::new(12.0, -8.0), Vec2::ZERO),
            ],
            pick_player: None,
            face_in: 0.0,
            last_hand: Vec::new(),
            thrown: Vec::new(),
            time: 0.0,
            ui_shake: Shaker::ui(),
            feel: FeelOverTime::default(),
            seq_sounds: 0,
            seq_start: None,
            rng: Rng::new(0xca7d),
            pick_shake: Vec2::ZERO,
        }
    }

    pub fn load(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &PlayAssets,
        content: &Arc<Content>,
    ) {
        let _ = (gpu, renderer, assets, content);
    }

    fn card_info<'a>(
        assets: &'a PlayAssets,
        content: &Content,
        card: CardId,
    ) -> Option<&'a serde_json::Value> {
        let id = &content.card(card).id;
        assets
            .data
            .cards
            .iter()
            .find(|c| &c.id == id)
            .and_then(|c| c.components.get("CardInfo"))
    }

    /// Resolves card prefabs that are about to be shown.
    pub fn poll(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut PlayAssets,
        world: &World,
        content: &Arc<Content>,
    ) {
        let Some(pick) = world.pick() else { return };
        // Every card on show: one hand, or all hands of a simultaneous pick.
        let mut shown: Vec<CardId> = pick.hand.clone();
        for h in world.pick_hands() {
            shown.extend(h.state.hand.iter().copied());
        }
        shown.sort_unstable_by_key(|c| c.0);
        shown.dedup();
        // Card art animations (Animator clips) of the cards on show.
        let dt = (self.time - self.art_time).clamp(0.0, 0.25);
        self.art_time = self.time;
        let data = assets.data.clone();
        for card in &shown {
            if let Some(Some(art)) = self.arts.get_mut(card) {
                art.update(&data.animations, dt);
            }
        }
        for &card in &shown {
            if self.card_base.is_none()
                && let Some(base) = Self::card_info(assets, content, card)
                    .and_then(|i| i["cardBase"]["$ref"].as_str().map(str::to_owned))
                && let Ok(prefab) = assets.data.clone().prefab(&base)
            {
                self.card_base = Some(UiPrefab::new(gpu, renderer, assets, &prefab, Vec2::ZERO));
            }
            if self.arts.contains_key(&card) {
                continue;
            }
            let art = Self::card_info(assets, content, card)
                .and_then(|i| i["cardArt"]["$ref"].as_str().map(str::to_owned))
                .and_then(|id| assets.data.clone().prefab(&id).ok())
                .map(|p| UiPrefab::new(gpu, renderer, assets, &p, Vec2::new(1100.0, 864.9)))
                .map(|p| ArtAnim::new(gpu, renderer, assets, p));
            self.arts.insert(card, art);
        }
    }

    pub fn on_event(&mut self, e: &Event, w: &World, _content: &Content) {
        match &e.kind {
            K::PickStarted { .. } | K::CardPicked { .. } if w.simultaneous_pick() => {}
            K::PickStarted { player } => {
                self.pick_player = Some(*player);
                self.slot_scale = [0.0; MAX_HAND];
                self.thrown.clear();
            }
            K::CardPicked { slot, card, .. } => {
                // The picked card flies to the face, the others are thrown.
                let hand = w
                    .pick()
                    .map(|p| p.hand.clone())
                    .unwrap_or_else(|| self.last_hand.clone());
                let n = w.rules().mods.hand_size();
                for (i, c) in hand.iter().enumerate() {
                    let (pos, rot) = slot_pose(i, n);
                    let picked = i == *slot as usize;
                    let away = (pos - FACE_POS).normalize_or(Vec2::Y);
                    self.thrown.push(Thrown {
                        slot: i,
                        card: if picked { *card } else { *c },
                        pos,
                        vel: if picked {
                            Vec2::ZERO
                        } else {
                            away * self.rng.range(10.0, 50.0)
                        },
                        rot: rot.to_radians(),
                        spin: if picked {
                            0.0
                        } else {
                            self.rng.range(-4.0, 4.0)
                        },
                        scale: if picked { 1.15 } else { 0.9 } * hand_scale(n),
                        life: if picked {
                            0.0
                        } else {
                            self.rng.range(0.5, 1.0)
                        },
                        picked,
                    });
                }
                self.pick_shake =
                    (FACE_POS - slot_pose(*slot as usize, n).0).normalize_or(Vec2::NEG_Y) * 2.0;
            }
            K::PointWon { .. } | K::RoundWon { .. } | K::GameWon { .. } => {
                self.seq_start = Some(e.frame);
                self.seq_sounds = 0;
            }
            _ => {}
        }
    }

    pub fn update(&mut self, w: &World, real: f32, sounds: &mut Sounds, audio: &mut Audio) {
        self.time += real;
        let f = self.feel.step(real, &mut self.rng);
        self.ui_shake.feel(f, 1.0);
        self.ui_shake.update(real);
        // Round visualiser ball sounds (ui.md 3.2).
        if let Phase::RoundOver { t, .. } = w.phase() {
            let ts = t as f32 * DT;
            let cues = [
                (0.9, "UI_Arms_Race_A_Ball_Shrink_Go_To_Left_Corner_SE"),
                (1.39, "UI_Arms_Race_B_Ball_Go_Down_Then_Expand_SE"),
                (1.88, "UI_Arms_Race_C_Ball_Pop_Shake_SE"),
            ];
            for (i, (at, cue)) in cues.iter().enumerate() {
                if ts >= *at && self.seq_sounds & (1 << i) == 0 {
                    self.seq_sounds |= 1 << i;
                    sounds.play(audio, cue, None, 0.0);
                    if i == 2 {
                        self.feel = FeelOverTime::start(10.0, 0.2);
                    }
                }
            }
        }
        if let Phase::PointOver { t, .. } | Phase::RoundOver { t, .. } = w.phase() {
            let fill_at = if matches!(w.phase(), Phase::RoundOver { .. }) {
                0.55
            } else {
                0.65
            };
            if (t as f32 * DT - fill_at).abs() < real * 0.6 {
                self.feel = FeelOverTime::start(10.0, 0.1);
            }
        }
        // Simultaneous picks: one spring per hand and slot.
        if w.simultaneous_pick() {
            let dt = real.min(0.05);
            for h in w.pick_hands() {
                let pi = (h.state.player as usize).min(11);
                let choosing = matches!(h.state.stage, PickStage::Choosing | PickStage::Offering);
                for i in 0..MAX_HAND {
                    let target = if i >= h.state.hand.len() || h.done {
                        0.0
                    } else if h.picked == Some(i as u8) {
                        1.15
                    } else if !choosing || h.picked.is_some() {
                        0.0
                    } else if i == h.selected as usize {
                        1.15
                    } else {
                        0.9
                    };
                    let k = &mut self.sim_scale[pi][i];
                    *k += (target - *k) * (1.0 - (-18.0 * dt).exp());
                }
            }
        } else {
            self.sim_scale = [[0.0; MAX_HAND]; 12];
        }
        // Card choice springs (ScaleShake target 1.15 / 0.9).
        if let Some(pick) = w.pick().filter(|_| !w.simultaneous_pick()) {
            self.last_hand = pick.hand.clone();
            let sel = w.pick_selected() as usize;
            let choosing = !matches!(pick.stage, PickStage::Ending(_) | PickStage::Gap(_));
            for i in 0..MAX_HAND {
                let target = if i >= pick.hand.len() || !choosing {
                    0.0
                } else if i == sel {
                    1.15
                } else {
                    0.9
                };
                let dt = real.min(0.02);
                self.slot_vel[i] += (target - self.slot_scale[i]) * dt * 50.0 * 2.0;
                self.slot_vel[i] -= 0.8 * self.slot_vel[i] * 20.0 * dt;
                self.slot_scale[i] += self.slot_vel[i] * 10.0 * dt;
            }
            self.face_in = (self.face_in + real / 0.3).min(1.0);
            // Hands (9.5): spring to the selected slot.
            let n = w.rules().mods.hand_size();
            let (sp, rot) = slot_pose(sel, n);
            let target = sp + Vec2::from_angle(rot.to_radians()).rotate(Vec2::new(0.0, -6.5));
            let active = if (sel as f32) < (n as f32 - 1.0) * 0.5 {
                0
            } else {
                1
            };
            for (h, (pos, vel)) in self.hands.iter_mut().enumerate() {
                let rest = FACE_POS + Vec2::new(if h == 0 { -13.0 } else { 13.0 }, 7.0);
                let sway = Vec2::new(
                    (self.time * 1.3 + h as f32).sin(),
                    (self.time * 0.9 + h as f32 * 2.0).cos(),
                ) * 0.6;
                let (tgt, k) = if h == active {
                    (target, 1.0)
                } else {
                    (rest + sway, 0.5)
                };
                let dt = real.min(0.1);
                *vel += (tgt - *pos) * 450.0 * k * dt;
                *vel -= *vel * dt * 20.0;
                *pos += *vel * dt;
            }
        } else {
            self.face_in = (self.face_in - real / 0.3).max(0.0);
            if self.face_in <= 0.0 {
                self.pick_player = None;
            }
        }
        for t in &mut self.thrown {
            t.life -= real;
            if t.picked {
                t.life += real;
                t.vel = Vec2::ZERO;
                t.spin += real * 4.0; // curve time
                if t.spin >= 1.0 {
                    t.life = -1.0;
                }
            } else {
                t.pos += t.vel * real;
                t.rot += t.spin * real;
                t.scale *= (1.0 - real * 1.5).max(0.0);
            }
        }
        self.thrown
            .retain(|t| t.life > -0.0001 || t.picked && t.life >= 0.0);
    }

    pub fn draw(
        &self,
        w: &World,
        content: &Content,
        a: &PlayAssets,
        _vis: &[PlayerVis],
        ctx: &mut DrawContext<'_>,
    ) {
        let size = Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
        let s = size.y / 1080.0;
        let list = &mut *ctx.list;
        let skins: Vec<Skin> = (0..w.rules().team_count as usize)
            .map(|t| team_skin(w, a, t))
            .collect();
        let fonts = a.fonts;
        // Reference-space helpers: centre-anchored and top-left anchored.
        let center = |p: Vec2| size * 0.5 + Vec2::new(p.x, -p.y) * s;
        let shake = self.ui_shake.pos * s;

        // ---- small round counter (top-left) ----
        let teams = skins.len();
        let to_win = w.rules().rounds_to_win_game as usize;
        let pts = w.points();
        let rnds = w.rounds();
        let row_h = if teams <= 2 {
            45.0
        } else if teams <= 4 {
            36.0
        } else {
            26.0
        };
        let pip_r = if teams <= 4 { 13.0 } else { 9.0 };
        for (t, skin) in skins.iter().enumerate() {
            let y = (34.7 + t as f32 * row_h) * s;
            for i in 0..to_win {
                let x = (116.9 + (i as f32 - (to_win as f32 - 1.0) * 0.5) * 32.0) * s;
                let lit = (rnds[t] + pts[t]) as usize > i;
                let big = rnds[t] as usize > i;
                let (c, k) = if lit {
                    (skin.win_text, if big { 1.0 } else { 0.3 })
                } else {
                    (Color::rgb(0.17, 0.17, 0.17), 0.3)
                };
                list.circle(Layer::Screen, 10, Vec2::new(x, y), pip_r * k * s, c);
            }
        }

        // ---- card bars (top-right), one row per player ----
        for (pi, p) in w.players().iter().enumerate() {
            let skin = a.skin(p.skin as usize);
            let y = (18.0 + 25.0 + pi as f32 * 58.0) * s;
            for (k, card) in p.cards.iter().rev().take(12).enumerate() {
                let x = size.x - (18.0 + 25.0 + k as f32 * 54.0) * s;
                list.rounded_rect(
                    Layer::Screen,
                    10,
                    Vec2::new(x, y),
                    Vec2::splat(48.0 * s),
                    8.0 * s,
                    skin.background,
                );
                let name = &content.card(*card).name;
                let mut ab: String = name.chars().take(1).collect::<String>().to_uppercase();
                ab.extend(name.chars().skip(1).take(1).flat_map(|c| c.to_lowercase()));
                text(
                    list,
                    11,
                    fonts.bold,
                    &ab,
                    Vec2::new(x, y),
                    26.0 * s,
                    Vec2::splat(0.5),
                    skin.win_text,
                );
            }
        }

        // ---- point / round visualiser ----
        let phase = w.phase();
        match phase {
            Phase::PointOver { t, winner }
            | Phase::RoundOver { t, winner }
            | Phase::GameOver { t, winner } => {
                let round = matches!(phase, Phase::RoundOver { .. });
                let game = matches!(phase, Phase::GameOver { .. });
                let ts = t as f32 * DT;
                if !game {
                    self.draw_balls(
                        list,
                        w,
                        &skins,
                        winner as usize,
                        ts,
                        round,
                        s,
                        size,
                        shake,
                        fonts,
                    );
                } else {
                    let c = skins[winner as usize].win_text;
                    text(
                        list,
                        20,
                        fonts.light,
                        "VICTORY!",
                        center(Vec2::new(0.0, 60.0)) + shake,
                        200.0 * s,
                        Vec2::splat(0.5),
                        c,
                    );
                }
            }
            Phase::Rematch { yes } => {
                let winner = (0..teams).max_by_key(|t| rnds[*t]).unwrap_or(0);
                let c = skins[winner].win_text;
                text(
                    list,
                    20,
                    fonts.light,
                    "REMATCH?",
                    center(Vec2::new(0.0, 120.0)),
                    160.0 * s,
                    Vec2::splat(0.5),
                    c,
                );
                for (label, x, on) in [("YES", -200.0, yes), ("NO", 200.0, !yes)] {
                    let k = if on { 1.25 } else { 0.85 };
                    let col = if on {
                        c
                    } else {
                        Color::rgb(0.123, 0.123, 0.123).lerp(c, 0.3)
                    };
                    text(
                        list,
                        21,
                        fonts.liberation,
                        label,
                        center(Vec2::new(x, -161.0)),
                        100.0 * s * k,
                        Vec2::splat(0.5),
                        col,
                    );
                }
            }
            Phase::Start { t } if (t as f32) * DT < 0.6 => {
                text(
                    list,
                    20,
                    fonts.light,
                    "LET'S GOO!",
                    center(Vec2::ZERO),
                    120.0 * s,
                    Vec2::splat(0.5),
                    Color::rgb(0.9, 0.9, 0.9),
                );
            }
            _ => {}
        }

        // ---- card choice ----
        if w.simultaneous_pick() {
            self.draw_hands(w, content, a, list);
            return;
        }
        if let Some(picker) = self.pick_player {
            let pick = w.pick();
            let face_k = self.face_in;
            if face_k > 0.0 {
                super::dim(list, 3_900_000, 0.72 * face_k);
                // Picker's big face rising from below.
                let p = w.players().get(picker as usize);
                let skin = p.map_or(a.skin(0), |p| a.skin(p.skin as usize));
                let rise = 1.0 - (1.0 - face_k).powi(3);
                let fpos = FACE_POS + Vec2::new(0.0, -20.0 * (1.0 - rise));
                let items = self.faces.get(picker as usize).filter(|f| !f.is_empty());
                draw_face(
                    list,
                    a,
                    items.map(|f| f.as_slice()),
                    fpos,
                    FACE_SCALE * 0.5,
                    skin,
                    3_950_000,
                );
                for (h, (hp, _)) in self.hands.iter().enumerate() {
                    let hp = *hp + Vec2::new(0.0, -20.0 * (1.0 - rise));
                    list.circle(Layer::World, 4_900_000 + h as i32, hp, 1.35, skin.color);
                    list.ring(
                        Layer::World,
                        4_900_001 + h as i32,
                        hp,
                        1.35,
                        0.12,
                        0.0,
                        std::f32::consts::TAU,
                        skin.background,
                    );
                }
            }
            if let Some(pick) = pick {
                let sel = w.pick_selected() as usize;
                let choosing = matches!(pick.stage, PickStage::Choosing | PickStage::Offering);
                for (i, card) in pick.hand.iter().enumerate() {
                    if !choosing {
                        break;
                    }
                    let n = w.rules().mods.hand_size();
                    let k = self.slot_scale[i] * hand_scale(n);
                    if k <= 0.01 {
                        continue;
                    }
                    let (pos, rot) = slot_pose(i, n);
                    let tf = Affine2::from_scale_angle_translation(
                        Vec2::splat(k),
                        rot.to_radians(),
                        pos,
                    );
                    self.draw_card(
                        list,
                        a,
                        content,
                        *card,
                        tf,
                        i == sel,
                        4_000_000 + 100_000 * i as i32,
                        200 + i as u8 * 2,
                    );
                }
            }
            for th in &self.thrown {
                let (pos, scale) = if th.picked {
                    let c = curve(&END_CURVE, th.spin.min(1.0));
                    (th.pos.lerp(FACE_POS, c), th.scale)
                } else {
                    (th.pos, th.scale)
                };
                if scale < 0.02 {
                    continue;
                }
                let tf = Affine2::from_scale_angle_translation(Vec2::splat(scale), th.rot, pos);
                self.draw_card(
                    list,
                    a,
                    content,
                    th.card,
                    tf,
                    th.picked,
                    4_000_000 + 100_000 * th.slot as i32,
                    200 + th.slot as u8 * 2,
                );
            }
        }
    }

    /// Simultaneous pick phase (match-flow.md 16, pick mode): every
    /// picker's hand at once, one panel per player in a grid sized for up
    /// to 12 pickers on one screen, framed in the player's colour, with its
    /// face, the dealt cards, its own cursor (the bigger card) and the
    /// picked card; finished hands keep only the picked card.
    fn draw_hands(&self, w: &World, content: &Content, a: &PlayAssets, list: &mut DrawList) {
        let hands = w.pick_hands();
        let n = hands.len().max(1);
        let slots = w.rules().mods.hand_size().max(5);
        super::dim(list, 3_900_000, 0.72);
        // Grid: the column count that gives the biggest cards.
        let (area_w, area_h) = (68.0f32, 36.0f32);
        let (card_w, card_h) = (11.6f32, 15.6f32);
        let mut best = (1usize, 0.0f32);
        for cols in 1..=n {
            let rows = n.div_ceil(cols);
            let (pw, ph) = (area_w / cols as f32, area_h / rows as f32);
            let k = ((pw - 4.0) / (slots as f32 * card_w)).min((ph - 1.5) / (card_h * 1.1));
            if k > best.1 {
                best = (cols, k);
            }
        }
        let (cols, k) = (best.0, best.1.min(0.62));
        let rows = n.div_ceil(cols);
        let (pw, ph) = (area_w / cols as f32, area_h / rows as f32);
        for (hi, h) in hands.iter().enumerate() {
            let (c, r) = (hi % cols, hi / cols);
            // Centre the last row.
            let in_row = if r == rows - 1 { n - r * cols } else { cols };
            let x0 = -(in_row as f32) * pw * 0.5;
            let centre = Vec2::new(
                x0 + (c as f32 + 0.5) * pw,
                area_h * 0.5 - (r as f32 + 0.5) * ph - 0.5,
            );
            let pl = w.players().get(h.state.player as usize);
            let skin = pl.map_or(a.skin(0), |p| a.skin(p.skin as usize));
            let base = 3_910_000 + hi as i32 * 10;
            let panel = Vec2::new(pw - 0.6, ph - 0.6);
            list.push(
                Layer::World,
                base,
                Primitive::Rect {
                    transform: Affine2::from_translation(centre),
                    size: panel,
                    pivot: Vec2::splat(0.5),
                    corner_radius: 0.8,
                    color: skin.background.with_alpha(if h.done { 0.35 } else { 0.6 }),
                },
            );
            list.ring(
                Layer::World,
                base + 1,
                centre + Vec2::new(-panel.x * 0.5 + 1.6, panel.y * 0.5 - 1.6),
                1.25,
                0.15,
                0.0,
                std::f32::consts::TAU,
                skin.win_text,
            );
            let items = self
                .faces
                .get(h.state.player as usize)
                .filter(|f| !f.is_empty());
            draw_face(
                list,
                a,
                items.map(|f| f.as_slice()),
                centre + Vec2::new(-panel.x * 0.5 + 1.6, panel.y * 0.5 - 1.6),
                1.1,
                skin,
                base + 2,
            );
            if h.done && h.picked.is_some() {
                // A drawn tick: this player has picked.
                let o = centre + Vec2::new(panel.x * 0.5 - 1.8, panel.y * 0.5 - 1.7);
                let (p0, p1, p2) = (
                    o + Vec2::new(-0.6, 0.0),
                    o + Vec2::new(-0.15, -0.5),
                    o + Vec2::new(0.7, 0.6),
                );
                list.line(Layer::World, base + 5, p0, p1, 0.28, skin.win_text);
                list.line(Layer::World, base + 5, p1, p2, 0.28, skin.win_text);
            }
            // Cards in a row, the cursor's card raised and bigger.
            let pi = (h.state.player as usize).min(11);
            let spacing = card_w * k * 1.0;
            let shown: Vec<usize> = (0..h.state.hand.len()).collect();
            let row_w = spacing * (shown.len().max(1) as f32 - 1.0);
            for &i in &shown {
                let mut sk = self.sim_scale[pi][i];
                // Finished hands: the picked card stays, small.
                if h.done {
                    if h.picked != Some(i as u8) {
                        continue;
                    }
                    sk = 0.8;
                }
                if sk <= 0.02 {
                    continue;
                }
                let selected = h
                    .picked
                    .map_or(i == h.selected as usize, |p| p as usize == i);
                let x = if h.picked.is_some() {
                    0.0
                } else {
                    -row_w * 0.5 + spacing * i as f32
                };
                let lift = if selected && h.picked.is_none() {
                    card_h * k * 0.08
                } else {
                    0.0
                };
                let pos = centre + Vec2::new(x, -0.4 + lift);
                let tf = Affine2::from_scale_angle_translation(Vec2::splat(k * sk), 0.0, pos);
                let idx = (hi * MAX_HAND + i) as i32;
                // Selected card above its neighbours.
                let sort = 4_000_000 + idx * 2_000 + if selected { 1_000_000 } else { 0 };
                self.draw_card(
                    list,
                    a,
                    content,
                    h.state.hand[i],
                    tf,
                    selected,
                    sort,
                    20 + (idx as u8 % 100) * 2,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_balls(
        &self,
        list: &mut DrawList,
        w: &World,
        skins: &[Skin],
        winner: usize,
        ts: f32,
        round: bool,
        s: f32,
        size: Vec2,
        shake: Vec2,
        fonts: super::assets::Fonts,
    ) {
        let (show_at, fill_at, close_at) = if round {
            (0.35, 0.55, 3.35)
        } else {
            (0.45, 0.65, 2.7)
        };
        if ts < show_at || ts > close_at {
            return;
        }
        let n = skins.len().max(1);
        let spacing = (1700.0 / n as f32).min(300.0);
        let ball = (spacing * 0.66).min(200.0);
        let center = size * 0.5 + shake;
        // Dimmer.
        list.rect(
            Layer::Screen,
            30,
            size * 0.5,
            size,
            0.0,
            Color::rgba(0.0, 0.0, 0.0, 0.45),
        );
        let pts = w.points();
        let to_win = w.rules().points_to_win_round.max(1) as f32;
        let out = if !round && ts > 2.45 {
            1.0 - ((ts - 2.45) / 0.25).clamp(0.0, 1.0)
        } else {
            1.0
        };
        for t in 0..n {
            let x0 = (t as f32 - (n as f32 - 1.0) * 0.5) * spacing;
            let mut pos = center + Vec2::new(x0, 0.0) * s;
            let mut d = ball * out;
            let fill = if ts >= fill_at {
                if round && t == winner {
                    1.0
                } else {
                    pts[t] as f32 / to_win
                }
            } else if round && t == winner {
                (to_win - 1.0) / to_win
            } else {
                (pts[t] as f32 - if t == winner { 1.0 } else { 0.0 }).max(0.0) / to_win
            };
            if round {
                // Winner shrinks into its round pip; losers grow toward the card face.
                if t == winner && ts > 0.9 {
                    let c = curve(&MOVE_CURVE, ((ts - 0.9) / 0.49).clamp(0.0, 1.0));
                    let k = curve(&SCALE_CURVE, ((ts - 0.9) / 0.49).clamp(0.0, 1.0));
                    let pip = Vec2::new(
                        116.9 + ((w.rounds()[t] as f32 - 1.0) - 2.0) * 32.0,
                        34.7 + t as f32 * 45.0,
                    ) * s;
                    pos = pos.lerp(pip, c);
                    d = ball + (20.0 - ball) * k;
                    if ts > 1.39 {
                        continue;
                    }
                } else if t != winner && ts > 1.39 {
                    let c = curve(&MOVE_CURVE, ((ts - 1.39) / 0.49).clamp(0.0, 1.0));
                    let target = size * 0.5 + Vec2::new(0.0, 18.0 / 40.0 * size.y);
                    pos = pos.lerp(target, c);
                    d = ball + (900.0 / n as f32 - ball) * c;
                    if ts > 2.4 {
                        let f = 1.0 - ((ts - 2.4) / 0.5).clamp(0.0, 1.0);
                        d *= f;
                    }
                }
            }
            let r = d * 0.5 * s;
            if r < 0.5 {
                continue;
            }
            let skin = skins[t];
            list.circle(
                Layer::Screen,
                31,
                pos,
                r,
                skin.background.lerp(Color::BLACK, 0.3),
            );
            if fill > 0.001 {
                list.ring(
                    Layer::Screen,
                    32,
                    pos,
                    r,
                    r,
                    -std::f32::consts::FRAC_PI_2,
                    -std::f32::consts::TAU * fill.min(1.0),
                    skin.win_text,
                );
            }
            list.ring(
                Layer::Screen,
                33,
                pos,
                r,
                (r * 0.08).max(1.5),
                0.0,
                std::f32::consts::TAU,
                skin.win_text,
            );
        }
        if ts >= fill_at && out > 0.0 && !(round && ts > 1.39) {
            let name = COLOR_NAMES[team_color_index(w, winner) % 12];
            let label = if round {
                format!("ROUND {name}")
            } else {
                format!("HALF {name}")
            };
            text(
                list,
                34,
                fonts.light,
                &label,
                center + Vec2::new(0.0, (ball * 0.5 + 90.0) * s),
                90.0 * s,
                Vec2::splat(0.5),
                skins[winner].win_text,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_card(
        &self,
        list: &mut DrawList,
        a: &PlayAssets,
        content: &Content,
        card: CardId,
        tf: Affine2,
        selected: bool,
        sort: i32,
        mask_base: u8,
    ) {
        let def = content.card(card);
        let theme = THEMES[def.color_theme as usize % THEMES.len()];
        let theme_c = c3(theme.0);
        let grey = Color::rgb(0.179, 0.179, 0.179);
        // The prefab's Canvas node carries the 0.01 canvas scale.
        let world = tf;
        let Some(base) = &self.card_base else {
            // Fallback: a plain card.
            list.push(
                Layer::World,
                sort,
                Primitive::Rect {
                    transform: tf,
                    size: Vec2::new(11.0, 15.0),
                    pivot: Vec2::splat(0.5),
                    corner_radius: 0.6,
                    color: c3(theme.1),
                },
            );
            return;
        };
        let back = base.find("Back");
        let art_node = base.find("Art");
        let mut z = sort;
        let bg_val = mask_base;
        let art_val = mask_base + 1;
        let bg_c = if selected {
            c3(theme.1).lerp(
                Color::rgb(def.card_color.r, def.card_color.g, def.card_color.b),
                0.35,
            )
        } else {
            Color::rgb(0.1, 0.1, 0.1)
        };
        let fonts = a.fonts;
        for (i, n) in base.nodes.iter().enumerate() {
            if !n.active || back.is_some_and(|b| base.under(i, b)) || n.name == "Damagable" {
                continue;
            }
            z += 1;
            let mask_parent = base.mask_of(i);
            let inside = match mask_parent {
                Some(m) if Some(m) == art_node => Mask::InsideValue(art_val),
                Some(_) => Mask::InsideValue(bg_val),
                None => Mask::None,
            };
            match n.name.as_str() {
                "Background" if n.image.is_some() => {
                    let img = n.image.unwrap();
                    let mut sp = image_sprite(world, n, &img, bg_c);
                    sp.mask = Mask::WriteValue(bg_val);
                    list.sprite(Layer::World, z, sp);
                    continue;
                }
                "Art" if n.image.is_some() => {
                    let img = n.image.unwrap();
                    let mut sp = image_sprite(world, n, &img, Color::rgba(0.0, 0.0, 0.0, 0.0));
                    sp.mask = Mask::WriteValue(art_val);
                    list.sprite(Layer::World, z, sp);
                    // The card's art prefab inside the art rect.
                    if let Some(Some(art)) = self.arts.get(&card) {
                        let art_world =
                            world * n.tf * Affine2::from_translation(n.min + n.size * 0.5);
                        for an in &art.current.nodes {
                            if !an.active {
                                continue;
                            }
                            let Some(img) = an.image else { continue };
                            z += 1;
                            let mut tint = img.color;
                            tint.a *= an.alpha;
                            if !selected {
                                let l = (tint.r + tint.g + tint.b) / 3.0;
                                tint = Color::rgba(l * 0.45, l * 0.45, l * 0.45, tint.a);
                            }
                            let noise = an.noise.map_or(Vec2::ZERO, |(amt, spd)| {
                                let t = self.time * spd * 0.3;
                                Vec2::new((t * 1.7 + an.size.x).sin(), (t * 1.3 + an.size.y).cos())
                                    * amt
                            });
                            let mut sp = image_sprite(
                                art_world * Affine2::from_translation(noise),
                                an,
                                &img,
                                tint,
                            );
                            sp.mask = Mask::InsideValue(art_val);
                            list.sprite(Layer::World, z, sp);
                        }
                    }
                    continue;
                }
                "Particles" | "Particle" | "EffectText" | "StatObject" | "Stat" | "Value"
                | "Icon" | "Image" => continue,
                "Text_Name" => {
                    if selected {
                        let c = n.tf.transform_point2(n.min + n.size * 0.5);
                        let size = n.text.as_ref().map_or(120.0, |t| t.size);
                        let label = def.name.to_uppercase();
                        let k = (900.0 / (label.chars().count() as f32 * size * 0.62)).min(1.0);
                        world_text(
                            list,
                            z,
                            fonts.liberation,
                            &label,
                            world.transform_point2(c),
                            size * k * 0.01 * tf_scale(tf),
                            tf_angle(tf),
                            theme_c.lerp(Color::WHITE, 0.25),
                        );
                    }
                    continue;
                }
                "Grid" => {
                    self.draw_stats(list, a, def, world, n, selected, z, tf);
                    z += 40;
                    continue;
                }
                _ => {}
            }
            let Some(img) = n.image else { continue };
            let mut tint = img.color;
            if n.name.starts_with("FRAME") {
                tint = if selected { theme_c } else { grey };
            }
            if n.name == "Triangle" {
                tint = match (def.rarity, selected) {
                    (Rarity::Common, _) => continue,
                    (Rarity::Uncommon, true) => Color::rgb(0.175, 0.678, 1.0),
                    (Rarity::Uncommon, false) => Color::rgb(0.193, 0.391, 0.519),
                    (Rarity::Rare, true) => Color::rgb(1.0, 0.176, 0.757),
                    (Rarity::Rare, false) => Color::rgb(0.528, 0.197, 0.432),
                };
            }
            let mut sp = image_sprite(world, n, &img, tint);
            sp.mask = inside;
            list.sprite(Layer::World, z, sp);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_stats(
        &self,
        list: &mut DrawList,
        a: &PlayAssets,
        def: &rounds_sim::content::CardDef,
        world: Affine2,
        grid: &super::ui_prefab::UiNode,
        selected: bool,
        z: i32,
        tf: Affine2,
    ) {
        let alpha = if selected { 1.0 } else { 0.15 };
        let fonts = a.fonts;
        let mut rows: Vec<(String, String, bool, bool)> = Vec::new();
        if !def.description.trim().is_empty() {
            rows.push((def.description.trim().to_owned(), String::new(), true, true));
        }
        for st in &def.stats {
            let amount = match if self.show_stat_numbers {
                0
            } else {
                st.simple_amount
            } {
                1 => "A little bit of".to_owned(),
                2 => "Some".to_owned(),
                3 => "A lot of".to_owned(),
                4 => "A huge amount of".to_owned(),
                5 => "Slightly lower".to_owned(),
                6 => "Lower".to_owned(),
                7 => "A lot lower".to_owned(),
                8 => "Slightly smaller".to_owned(),
                9 => "Smaller".to_owned(),
                _ => st.amount.trim().to_owned(),
            };
            rows.push((st.stat.clone(), amount, st.positive, false));
        }
        let spacing = 10.0;
        let row_h = |desc: bool, text: &str| {
            if desc {
                70.0 * ((text.len() as f32 / 30.0).ceil().max(1.0)) + 20.0
            } else {
                86.1
            }
        };
        let total: f32 = rows.iter().map(|r| row_h(r.3, &r.0)).sum::<f32>()
            + spacing * rows.len().saturating_sub(1) as f32;
        let gc = grid.min + grid.size * 0.5;
        let mut y = gc.y + total * 0.5;
        let k = 0.01 * tf_scale(tf);
        let ang = tf_angle(tf);
        let w = grid.size.x;
        for (label, value, positive, desc) in rows {
            let h = row_h(desc, &label);
            let cy = y - h * 0.5;
            let centre = grid.tf.transform_point2(Vec2::new(gc.x, cy));
            if desc {
                let wrapped = wrap(&label, 30);
                world_text(
                    list,
                    z + 2,
                    fonts.liberation,
                    &wrapped,
                    world.transform_point2(centre),
                    56.0 * k,
                    ang,
                    Color::rgba(0.82, 0.82, 0.82, alpha),
                );
            } else {
                let bg = super::ui_prefab::UiNode {
                    min: Vec2::new(gc.x - w * 0.5, cy - h * 0.5) - Vec2::ZERO,
                    size: Vec2::new(w, h),
                    ..grid.clone()
                };
                let sp = rounds_render::Sprite {
                    transform: world * grid.tf * Affine2::from_translation(bg.min + bg.size * 0.5),
                    size: bg.size,
                    tint: Color::rgba(0.1415, 0.1415, 0.1415, alpha),
                    ..Default::default()
                };
                list.sprite(Layer::World, z + 1, sp);
                let lp = grid
                    .tf
                    .transform_point2(Vec2::new(gc.x - w * 0.5 + 30.0, cy));
                let rp = grid
                    .tf
                    .transform_point2(Vec2::new(gc.x + w * 0.5 - 30.0, cy));
                world_text_anchor(
                    list,
                    z + 2,
                    fonts.liberation,
                    &label,
                    world.transform_point2(lp),
                    58.0 * k,
                    ang,
                    Vec2::new(0.0, 0.5),
                    Color::rgba(0.82, 0.82, 0.82, alpha),
                );
                let vc = if positive {
                    Color::rgb(0.465, 0.604, 0.390)
                } else {
                    Color::rgb(0.698, 0.326, 0.326)
                };
                world_text_anchor(
                    list,
                    z + 2,
                    fonts.liberation,
                    &value,
                    world.transform_point2(rp),
                    58.0 * k,
                    ang,
                    Vec2::new(1.0, 0.5),
                    vc.with_alpha(alpha),
                );
            }
            y -= h + spacing;
        }
    }
}

fn wrap(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = 0;
    for word in s.split_whitespace() {
        if line > 0 && line + word.len() + 1 > width {
            out.push('\n');
            line = 0;
        } else if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(word);
        line += word.len();
    }
    out
}

fn tf_scale(tf: Affine2) -> f32 {
    tf.matrix2.x_axis.length()
}

fn tf_angle(tf: Affine2) -> f32 {
    tf.matrix2.x_axis.y.atan2(tf.matrix2.x_axis.x)
}

/// Pose of slot `i` in a hand of `n` cards: the original's five slots for
/// five cards; other hand sizes (Pick N Cards) on the same arc, spaced like
/// the original up to five cards and squeezed to its width beyond.
fn slot_pose(i: usize, n: usize) -> (Vec2, f32) {
    if n == SLOTS.len() || n == 0 {
        let (x, y, r) = SLOTS[i.min(SLOTS.len() - 1)];
        return (CHOICE_ROOT + Vec2::new(x, y), r);
    }
    let half = 25.0 * ((n as f32 - 1.0) / 4.0).min(1.0);
    let u = if n == 1 {
        0.0
    } else {
        (i.min(n - 1) as f32 / (n as f32 - 1.0)) * 2.0 - 1.0
    };
    let x = u * half;
    let y = 9.66 * (1.0 - (x / 25.0) * (x / 25.0));
    (CHOICE_ROOT + Vec2::new(x, y), -15.0 * x / 25.0)
}

/// Card scale of a hand of `n` cards (smaller beyond the original five).
fn hand_scale(n: usize) -> f32 {
    if n <= SLOTS.len() {
        1.0
    } else {
        (SLOTS.len() as f32 / n as f32).max(0.5)
    }
}

/// Team colour: the skin of the team's first member (match-flow.md 2.2).
fn team_color_index(w: &World, team: usize) -> usize {
    w.players()
        .iter()
        .find(|p| p.team as usize == team)
        .map_or(team, |p| p.skin as usize)
}

fn team_skin(w: &World, a: &PlayAssets, team: usize) -> Skin {
    a.skin(team_color_index(w, team))
}

/// Screen text (pixels).
#[allow(clippy::too_many_arguments)]
fn text(
    list: &mut DrawList,
    sort: i32,
    font: FontId,
    s: &str,
    pos: Vec2,
    size: f32,
    anchor: Vec2,
    color: Color,
) {
    list.push(
        Layer::Screen,
        sort,
        Text {
            text: s.to_owned(),
            font,
            size,
            position: pos,
            anchor,
            color,
            ..Default::default()
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn world_text(
    list: &mut DrawList,
    sort: i32,
    font: FontId,
    s: &str,
    pos: Vec2,
    size: f32,
    rot: f32,
    color: Color,
) {
    world_text_anchor(list, sort, font, s, pos, size, rot, Vec2::splat(0.5), color);
}

#[allow(clippy::too_many_arguments)]
fn world_text_anchor(
    list: &mut DrawList,
    sort: i32,
    font: FontId,
    s: &str,
    pos: Vec2,
    size: f32,
    rot: f32,
    anchor: Vec2,
    color: Color,
) {
    list.push(
        Layer::World,
        sort,
        Text {
            text: s.to_owned(),
            font,
            size,
            position: pos,
            rotation: rot,
            anchor,
            color,
        },
    );
}

/// The card-choice face: a big body disc in the picker's colour with the
/// default face (ui.md 4.1, match-flow.md 9.5).
fn draw_face(
    list: &mut DrawList,
    a: &PlayAssets,
    face: Option<&[super::assets::FaceItem]>,
    pos: Vec2,
    radius: f32,
    skin: Skin,
    sort: i32,
) {
    list.circle(Layer::World, sort, pos, radius, skin.color);
    list.ring(
        Layer::World,
        sort + 1,
        pos,
        radius,
        radius * 0.03,
        0.0,
        std::f32::consts::TAU,
        skin.background,
    );
    let k = radius * 2.0 / 1.1044 * 0.16;
    let default: Vec<super::assets::FaceItem> = [a.eyes, a.mouth].into_iter().flatten().collect();
    for item in face.unwrap_or(&default).iter().copied() {
        let off = item.offset * 1.05;
        let t = Affine2::from_translation(pos)
            * Affine2::from_scale(Vec2::splat(k))
            * Affine2::from_translation(off)
            * Affine2::from_scale(Vec2::splat(item.scale * 1.05));
        list.sprite(Layer::World, sort + 2, item.sprite.sprite(t, item.color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_poses() {
        // Five cards: exactly the original's slots.
        for (i, (x, y, r)) in SLOTS.iter().enumerate() {
            assert_eq!(slot_pose(i, 5), (CHOICE_ROOT + Vec2::new(*x, *y), *r));
        }
        assert_eq!(hand_scale(5), 1.0);
        // Other sizes: symmetric, left to right, within the original's width.
        for n in 1..=MAX_HAND {
            for i in 0..n {
                let (p, r) = slot_pose(i, n);
                let (q, s) = slot_pose(n - 1 - i, n);
                assert!(
                    (p.x + q.x).abs() < 1e-4 && (p.y - q.y).abs() < 1e-4 && (r + s).abs() < 1e-4
                );
                assert!(p.x.abs() <= 25.0 + 1e-4);
                if i > 0 {
                    assert!(slot_pose(i - 1, n).0.x < p.x);
                }
            }
        }
    }
}
