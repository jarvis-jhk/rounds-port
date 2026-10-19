//! The lobby (ui.md 2.5 generalised to 12 players, match-flow.md 3): any
//! device presses jump to join (keyboard + mouse with Space, the right
//! keyboard half with Right Shift, every controller with A; Tab switches a
//! keyboard player between mouse aim and the keys-only left half), picks a
//! face preset (and edits it), a colour (12, unique) or a team (2..6 teams,
//! any split, team colours), and readies up. When everybody is ready the
//! match starts. Bots fill slots for testing. Up to 12 cards on one screen.
//!
//! Online the same page shows every machine's players: this machine's are
//! its slot requests; the host edits the settings and starts.

use glam::Vec2;
use rounds_net::warpzone_core::client::Event as WzEvent;
use rounds_net::{MatchSettings, MatchStart, NetEvent, SlotRequest, TeamMode as NetTeams};
use rounds_sim::{GameMode, MatchSetup, SlotSetup, TeamMode, World};

use super::editor::FaceEditor;
use super::{Action, Cx, Launch, Page, Shared};
use crate::input::DeviceId;
use crate::online::{self, NetRef, NetRules};
use crate::play::driver::MatchOutcome;
use crate::play::input_map::SlotControl;
use crate::settings::{FACE_PRESETS, Face, MatchRules, Profile};
use crate::ui::{Font, Rect, style};

pub const MAX_PLAYERS: usize = 12;
const COLOURS: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Device(DeviceId),
    Bot,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub who: Who,
    pub profile: String,
    pub face: usize,
    pub colour: u8,
    pub team: u8,
    pub ready: bool,
    /// Focused row on the card: 0 face, 1 colour/team.
    pub row: usize,
    pub t: u32,
}

/// One card to draw.
struct CardView {
    title: String,
    sub: String,
    face: Face,
    skin: usize,
    row_values: Option<(String, String)>,
    ready: bool,
    local: Option<usize>,
    /// The local player's device (ready prompt with its button).
    device: Option<DeviceId>,
    focus_row: Option<usize>,
    bot: bool,
    /// Another machine's slot: (machine, local index, team).
    remote: Option<(rounds_net::MachineId, usize, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Mode,
    Points,
    Rounds,
    Picks,
    Maps,
    Mods,
    AddBot,
    RemoveBot,
}

const SETTINGS: [Setting; 8] = [
    Setting::Mode,
    Setting::Points,
    Setting::Rounds,
    Setting::Picks,
    Setting::Maps,
    Setting::Mods,
    Setting::AddBot,
    Setting::RemoveBot,
];

pub struct LobbyPage {
    net: Option<NetRef>,
    sandbox: bool,
    pub players: Vec<Player>,
    /// A device navigating the settings bar, and the item.
    sfocus: Option<(DeviceId, usize)>,
    editor: Option<FaceEditor>,
    countdown: Option<u32>,
    t: u32,
    msg: Option<(String, u32)>,
    esc_armed: u32,
    last_sent: Option<Vec<SlotRequest>>,
    last_ready: Option<bool>,
    last_rules: Option<(NetTeams, Vec<u8>)>,
    knocks: Vec<(String, String)>,
    room: Option<(String, Option<String>, String)>,
    /// Demo pages (screenshots) never start.
    demo: bool,
    hover: Option<usize>,
    /// Demo: other machines' players (name, machine, face preset, colour, ready).
    fake_remote: Vec<(String, String, usize, u8, bool)>,
}

impl LobbyPage {
    fn base(net: Option<NetRef>, sandbox: bool) -> LobbyPage {
        LobbyPage {
            net,
            sandbox,
            players: Vec::new(),
            sfocus: None,
            editor: None,
            countdown: None,
            t: 0,
            msg: None,
            esc_armed: 0,
            last_sent: None,
            last_ready: None,
            last_rules: None,
            knocks: Vec::new(),
            room: None,
            demo: false,
            hover: None,
            fake_remote: Vec::new(),
        }
    }

    pub fn local(sandbox: bool) -> LobbyPage {
        Self::base(None, sandbox)
    }

    pub fn online(net: NetRef, room: Option<(String, Option<String>, String)>) -> LobbyPage {
        let mut l = Self::base(Some(net), false);
        l.room = room;
        l
    }

    /// A lobby full of joined players (screenshots).
    pub fn demo(sh: &mut Shared, n: usize, teams: bool) -> LobbyPage {
        let mut l = Self::base(None, false);
        l.demo = true;
        if teams {
            sh.settings.rules.teams = 3;
        }
        let devices = [
            Who::Device(DeviceId::KeyboardMouse),
            Who::Device(DeviceId::KeyboardRight),
        ];
        for i in 0..n {
            let who = devices
                .get(i)
                .copied()
                .unwrap_or(Who::Device(DeviceId::Controller(100 + i as u32)));
            l.add(sh, who);
            let p = l.players.last_mut().unwrap();
            p.face = i % FACE_PRESETS;
            p.ready = i % 3 != 2;
            p.t = 100;
            if teams {
                p.team = (i % 3) as u8;
            }
        }
        l
    }

    /// An online lobby with other machines (screenshots, no network).
    pub fn demo_online(sh: &mut Shared) -> LobbyPage {
        let mut l = Self::demo(sh, 2, false);
        l.room = Some((
            "demo".into(),
            Some("K7QM3X".into()),
            "warpzone://join/demo-room".into(),
        ));
        let others = [
            ("ANN", "ann", 2, 2, true),
            ("BOB 1", "bob", 3, 3, true),
            ("BOB 2", "bob", 4, 4, false),
            ("BOB 3", "bob", 5, 5, true),
            ("EVE", "eve", 6, 6, true),
            ("MALLORY", "mallory", 7, 7, false),
        ];
        l.fake_remote = others
            .iter()
            .map(|(n, m, f, c, r)| (n.to_string(), m.to_string(), *f, *c, *r))
            .collect();
        l
    }

    pub fn open_editor_demo(&mut self) {
        self.editor = Some(FaceEditor::new(None, 1, 0));
    }

    fn rules(&self, sh: &Shared) -> MatchRules {
        match &self.net {
            Some(n) if !n.borrow().is_host() => {
                NetRules::decode(&n.borrow().lobby().settings.game).rules
            }
            _ => sh.settings.rules.clone(),
        }
    }

    fn can_edit_settings(&self) -> bool {
        self.net.as_ref().is_none_or(|n| n.borrow().is_host())
    }

    fn teams(&self, sh: &Shared) -> u8 {
        if self.sandbox {
            0
        } else {
            self.rules(sh).teams
        }
    }

    fn colour_taken(&self, c: u8, except: usize) -> bool {
        self.players
            .iter()
            .enumerate()
            .any(|(i, p)| i != except && p.colour == c)
    }

    fn smallest_team(&self, teams: u8) -> u8 {
        (0..teams.max(1))
            .min_by_key(|t| (self.players.iter().filter(|p| p.team == *t).count(), *t))
            .unwrap_or(0)
    }

    fn add(&mut self, sh: &Shared, who: Who) {
        if self.players.len() >= MAX_PLAYERS {
            return;
        }
        let profile = match who {
            Who::Device(d) => profile_key(d, &[]),
            Who::Bot => String::new(),
        };
        let prof = if profile.is_empty() {
            Profile {
                face: (self.players.len() * 3) % FACE_PRESETS,
                colour: self.players.len() as u8,
            }
        } else {
            sh.settings.profile(&profile)
        };
        let colour = if self.colour_taken(prof.colour % COLOURS, usize::MAX) {
            (0..COLOURS)
                .find(|c| !self.colour_taken(*c, usize::MAX))
                .unwrap_or(0)
        } else {
            prof.colour % COLOURS
        };
        let teams = self.teams(sh);
        let team = self.smallest_team(teams);
        self.players.push(Player {
            who,
            profile,
            face: prof.face % FACE_PRESETS,
            colour,
            team,
            ready: who == Who::Bot,
            row: 0,
            t: 0,
        });
        self.countdown = None;
    }

    fn save_profile(&self, sh: &mut Shared, i: usize) {
        let p = &self.players[i];
        if p.profile.is_empty() {
            return;
        }
        sh.settings.profiles.insert(
            p.profile.clone(),
            Profile {
                face: p.face,
                colour: p.colour,
            },
        );
        sh.settings.save();
    }

    /// Value change of a card row.
    fn change_row(&mut self, sh: &mut Shared, i: usize, row: usize, dir: i32) {
        let teams = self.teams(sh);
        if row == 0 {
            let p = &mut self.players[i];
            p.face = (p.face as i32 + dir).rem_euclid(FACE_PRESETS as i32) as usize;
        } else if teams > 0 {
            let p = &mut self.players[i];
            p.team = (p.team as i32 + dir).rem_euclid(teams as i32) as u8;
        } else {
            let mut c = self.players[i].colour as i32;
            for _ in 0..COLOURS {
                c = (c + dir).rem_euclid(COLOURS as i32);
                if !self.colour_taken(c as u8, i) {
                    break;
                }
            }
            self.players[i].colour = c as u8;
        }
        self.countdown = None;
        self.save_profile(sh, i);
    }

    fn change_setting(&mut self, cx: &mut Cx<'_>, s: Setting, dir: i32) -> Action {
        if !self.can_edit_settings() && !matches!(s, Setting::AddBot | Setting::RemoveBot) {
            self.msg = Some(("THE HOST CHOOSES THE SETTINGS".into(), 120));
            return Action::None;
        }
        let r = &mut cx.shared.settings.rules;
        let cyc = |v: u8, lo: u8, hi: u8| -> u8 {
            let span = (hi - lo + 1) as i32;
            (lo as i32 + (v as i32 - lo as i32 + dir).rem_euclid(span)) as u8
        };
        match s {
            Setting::Mode => {
                // 0 (FFA), 2..6 teams.
                let idx = if r.teams == 0 { 0 } else { r.teams - 1 };
                let next = cyc(idx, 0, 5);
                r.teams = if next == 0 { 0 } else { next + 1 };
                let t = r.teams;
                for k in 0..self.players.len() {
                    self.players[k].team = (k as u8) % t.max(1);
                }
            }
            Setting::Points => r.points_to_win_round = cyc(r.points_to_win_round, 1, 5),
            Setting::Rounds => r.rounds_to_win = cyc(r.rounds_to_win, 1, 10),
            Setting::Picks => {
                // Off -> auto -> in turn -> together -> off.
                use crate::settings::PickModeSetting as M;
                let order = [
                    None,
                    Some(M::Auto),
                    Some(M::Sequential),
                    Some(M::Simultaneous),
                ];
                let cur = if r.pick_phase {
                    Some(r.pick_mode)
                } else {
                    None
                };
                let i = order.iter().position(|o| *o == cur).unwrap_or(1) as i32;
                let next = order[(i + dir).rem_euclid(order.len() as i32) as usize];
                r.pick_phase = next.is_some();
                if let Some(m) = next {
                    r.pick_mode = m;
                }
            }
            Setting::Maps => {
                cx.shared.settings.save();
                return Action::Push(Box::new(super::pages::MapPoolPage::default()));
            }
            Setting::Mods => {
                cx.shared.settings.save();
                return Action::Push(Box::new(super::pages::ModsPage::default()));
            }
            Setting::AddBot => {
                let sh = &*cx.shared;
                self.add(sh, Who::Bot);
            }
            Setting::RemoveBot => {
                if let Some(i) = self.players.iter().rposition(|p| p.who == Who::Bot) {
                    self.players.remove(i);
                }
            }
        }
        self.countdown = None;
        cx.shared.settings.save();
        Action::None
    }

    fn setting_text(&self, sh: &Shared, s: Setting) -> (String, String) {
        let r = self.rules(sh);
        match s {
            Setting::Mode => (
                "MODE".into(),
                if self.sandbox {
                    sh.up("BUTTON_SANDBOX", "Sandbox")
                } else if r.teams == 0 {
                    "FREE-FOR-ALL".into()
                } else {
                    format!("{} TEAMS", r.teams)
                },
            ),
            Setting::Points => ("POINTS PER ROUND".into(), r.points_to_win_round.to_string()),
            Setting::Rounds => ("ROUNDS TO WIN".into(), r.rounds_to_win.to_string()),
            Setting::Picks => (
                "CARD PICKS".into(),
                if r.pick_phase {
                    match r.pick_mode {
                        crate::settings::PickModeSetting::Auto => "AUTO".into(),
                        crate::settings::PickModeSetting::Sequential => "IN TURN".into(),
                        crate::settings::PickModeSetting::Simultaneous => "TOGETHER".into(),
                    }
                } else {
                    sh.up("OPTION_NO", "No")
                },
            ),
            Setting::Maps => {
                let n = sh.content.maps.len();
                (
                    "MAPS".into(),
                    format!("{} / {}", n - r.maps.excluded.len().min(n), n),
                )
            }
            Setting::Mods => {
                let on = r.mods.enabled_count();
                (
                    "MODS".into(),
                    if on == 0 {
                        sh.up("OPTION_NO", "No")
                    } else {
                        format!("{on} / {}", rounds_sim::mods::ALL.len())
                    },
                )
            }
            Setting::AddBot => ("".into(), "+ BOT".into()),
            Setting::RemoveBot => ("".into(), "- BOT".into()),
        }
    }

    // --------------------------------------------------------- online

    fn my_slots(&self, sh: &Shared) -> Vec<SlotRequest> {
        let teams = self.teams(sh);
        let name = self
            .net
            .as_ref()
            .and_then(|n| {
                n.borrow_mut()
                    .with_warpzone(|c| c.account().map(|a| a.username))
            })
            .flatten()
            .unwrap_or_else(|| "player".into());
        self.players
            .iter()
            .enumerate()
            .map(|(i, p)| SlotRequest {
                name: if self.players.len() == 1 {
                    name.clone()
                } else {
                    format!("{name} {}", i + 1)
                },
                colour: p.colour,
                team: (teams > 0).then_some(p.team),
                extra: online::encode_face(&sh.preset(p.face)),
            })
            .collect()
    }

    /// Lobby traffic: events, our slots/ready, the host's settings.
    fn net_tick(&mut self, cx: &mut Cx<'_>) -> Action {
        let Some(net) = self.net.clone() else {
            return Action::None;
        };
        net.borrow_mut().poll();
        loop {
            let e = net.borrow_mut().next_event();
            let Some(e) = e else { break };
            match e {
                NetEvent::MatchStarted(ms) => {
                    return Action::StartMatch(Box::new(self.online_launch(cx.shared, &net, &ms)));
                }
                NetEvent::Warpzone(w) => match *w {
                    WzEvent::Knock {
                        knock, username, ..
                    } => self.knocks.push((knock, username)),
                    WzEvent::RoomHosted { room, code, invite } => {
                        self.room = Some((room.id, code, invite));
                    }
                    WzEvent::Error { op, message, .. } => {
                        self.msg = Some((format!("{op}: {message}").to_uppercase(), 240));
                    }
                    _ => {}
                },
                NetEvent::MachineJoined { name, .. } => {
                    self.msg = Some((format!("{name} JOINED").to_uppercase(), 150));
                    cx.shared.sfx(cx.audio, "UI_Player_Added");
                }
                NetEvent::MachineLeft { machine } => {
                    self.msg = Some((format!("{machine} LEFT").to_uppercase(), 150));
                }
                NetEvent::Rejected { reason } | NetEvent::RoomLeft { reason } => {
                    cx.shared.net = None;
                    return Action::Replace(Box::new(super::online::OnlineHomePage::with_message(
                        format!("LEFT THE ROOM: {reason}").to_uppercase(),
                    )));
                }
                NetEvent::WaitingForNextMatch { .. } => {
                    self.msg = Some(("A MATCH IS RUNNING - YOU PLAY IN THE NEXT ONE".into(), 300));
                }
                _ => {}
            }
        }
        let slots = self.my_slots(cx.shared);
        if self.last_sent.as_ref() != Some(&slots) {
            net.borrow_mut().set_local_slots(slots.clone());
            self.last_sent = Some(slots);
        }
        let ready = !self.players.is_empty() && self.players.iter().all(|p| p.ready);
        if self.last_ready != Some(ready) {
            net.borrow_mut().set_ready(ready);
            self.last_ready = Some(ready);
        }
        if net.borrow().is_host() {
            let r = &cx.shared.settings.rules;
            let mode = if r.teams == 0 {
                NetTeams::Ffa
            } else {
                NetTeams::Teams(r.teams)
            };
            let game = NetRules {
                rules: r.clone(),
                sandbox: false,
            }
            .encode();
            if self.last_rules.as_ref() != Some(&(mode, game.clone())) {
                let n_maps = cx.shared.content.maps.len();
                let pool = n_maps - r.maps.excluded.len().min(n_maps);
                let map = if pool == n_maps {
                    "all maps".to_string()
                } else {
                    format!("{pool} maps")
                };
                let mut nb = net.borrow_mut();
                let _ = nb.set_settings(MatchSettings {
                    team_mode: mode,
                    game: game.clone(),
                });
                nb.set_room_metadata("map", &map);
                drop(nb);
                self.last_rules = Some((mode, game));
            }
        }
        Action::None
    }

    fn online_launch(&self, sh: &Shared, net: &NetRef, ms: &MatchStart) -> Launch {
        let whos: Vec<Who> = self.players.iter().map(|p| p.who).collect();
        online_launch(sh, net, ms, &whos)
    }

    fn local_launch(&self, sh: &Shared) -> Launch {
        let rules = &sh.settings.rules;
        let teams = self.teams(sh);
        let mut members = [0usize; 13];
        let slots = self
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if teams > 0 {
                    let k = members[p.team as usize % 13];
                    members[p.team as usize % 13] += 1;
                    SlotSetup {
                        team: p.team,
                        skin: online::team_skin(p.team, k),
                    }
                } else {
                    SlotSetup {
                        team: i as u8,
                        skin: p.colour,
                    }
                }
            })
            .collect();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let setup = MatchSetup {
            slots,
            mode: if self.sandbox {
                GameMode::Sandbox
            } else {
                GameMode::ArmsRace
            },
            teams: if teams > 0 {
                TeamMode::Teams
            } else {
                TeamMode::FreeForAll
            },
            seed,
            points_to_win_round: rules.points_to_win_round,
            rounds_to_win_game: rules.rounds_to_win,
            pick_phase: rules.pick_phase,
            map_rotation: online::map_rotation(&sh.content, rules),
            mods: rules.mods,
        };
        Launch {
            setup,
            pick_mode: rules.pick_mode.to_sim(),
            controls: self
                .players
                .iter()
                .map(|p| match p.who {
                    Who::Device(d) => SlotControl::Device(d),
                    Who::Bot => SlotControl::Bot,
                })
                .collect(),
            faces: self.players.iter().map(|p| sh.preset(p.face)).collect(),
            driver: None,
            bot_seed: seed,
            bot_rematch: None,
            shots: Vec::new(),
            shot_prefix: None,
            vs_names: Vec::new(),
        }
    }

    /// Why the match cannot start yet (None = it can).
    fn blocker(&self, sh: &Shared) -> Option<String> {
        let (total, all_ready) = match &self.net {
            Some(n) => {
                let n = n.borrow();
                let lob = n.lobby();
                let total: usize = lob.machines.iter().map(|m| m.slots.len()).sum();
                let ready = lob
                    .machines
                    .iter()
                    .filter(|m| !m.slots.is_empty())
                    .all(|m| m.ready);
                (total.min(MAX_PLAYERS), ready && !self.players.is_empty())
            }
            None => (self.players.len(), self.players.iter().all(|p| p.ready)),
        };
        let need = if self.sandbox { 1 } else { 2 };
        if total < need {
            return Some(format!("{need} PLAYERS NEEDED"));
        }
        let teams = self.teams(sh);
        if teams > 0 && self.net.is_none() {
            let used = (0..teams)
                .filter(|t| self.players.iter().any(|p| p.team == *t))
                .count();
            if used < 2 {
                return Some("PLAYERS ON AT LEAST 2 TEAMS NEEDED".into());
            }
        }
        if !all_ready {
            return Some(String::new());
        }
        if let Some(n) = &self.net
            && !n.borrow().is_host()
        {
            return Some("WAITING FOR THE HOST".into());
        }
        None
    }

    fn cards(&self, sh: &Shared) -> Vec<CardView> {
        let teams = self.teams(sh);
        let mut out = Vec::new();
        let label = |who: Who| match who {
            Who::Device(d) => d.label().to_uppercase(),
            Who::Bot => "BOT".into(),
        };
        let mut members = [0usize; 13];
        for (i, p) in self.players.iter().enumerate() {
            let skin = if teams > 0 {
                let k = members[p.team as usize % 13];
                members[p.team as usize % 13] += 1;
                online::team_skin(p.team, k) as usize
            } else {
                p.colour as usize
            };
            let row1 = if teams > 0 {
                format!("TEAM {}", p.team + 1)
            } else {
                colour_name(p.colour).to_string()
            };
            out.push(CardView {
                title: format!("P{}", i + 1),
                sub: label(p.who),
                face: sh.preset(p.face),
                skin,
                row_values: Some((format!("FACE {}", p.face + 1), row1)),
                ready: p.ready,
                local: Some(i),
                device: match p.who {
                    Who::Device(d) => Some(d),
                    Who::Bot => None,
                },
                focus_row: (!p.ready && self.sfocus.is_none_or(|(d, _)| Who::Device(d) != p.who))
                    .then_some(p.row)
                    .filter(|_| matches!(p.who, Who::Device(_))),
                bot: p.who == Who::Bot,
                remote: None,
            });
        }
        for (name, machine, face, colour, ready) in &self.fake_remote {
            out.push(CardView {
                title: name.clone(),
                sub: format!("ONLINE · {}", machine.to_uppercase()),
                face: sh.preset(*face),
                skin: *colour as usize,
                row_values: None,
                ready: *ready,
                local: None,
                device: None,
                focus_row: None,
                bot: false,
                remote: None,
            });
        }
        // Other machines' players (online).
        if let Some(n) = &self.net {
            let n = n.borrow();
            let me = n.me();
            let lob = n.lobby();
            let preview = rounds_net::lobby::assign(&lob.machines, lob.settings.team_mode);
            for p in preview.iter().filter(|p| Some(p.machine) != me) {
                let m = lob.machines.iter().find(|m| m.machine == p.machine);
                let face = m
                    .and_then(|m| m.slots.get(p.local_index as usize))
                    .map(|s| online::decode_face(&s.extra))
                    .unwrap_or_default();
                let skin = if teams > 0 {
                    p.team as usize
                } else {
                    p.colour as usize
                };
                out.push(CardView {
                    remote: Some((p.machine, p.local_index as usize, p.team)),
                    title: p.name.to_uppercase(),
                    sub: format!("ONLINE · {}", m.map(|m| m.name.as_str()).unwrap_or("?"))
                        .to_uppercase(),
                    face,
                    skin,
                    row_values: None,
                    ready: m.is_some_and(|m| m.ready),
                    local: None,
                    device: None,
                    focus_row: None,
                    bot: false,
                });
            }
        }
        out
    }
}

/// A local match of `n` bots (screenshots of in-match screens).
pub fn bot_launch(sh: &mut Shared, n: usize) -> Launch {
    let mut l = LobbyPage::demo(sh, n, false);
    for p in &mut l.players {
        p.who = Who::Bot;
    }
    l.local_launch(sh)
}

/// The launch of an online match from the host's start message; `local`
/// are this machine's players in slot-request order.
pub fn online_launch(sh: &Shared, net: &NetRef, ms: &MatchStart, local: &[Who]) -> Launch {
    online_launch_with(sh, net, ms, local, None)
}

pub fn online_launch_with(
    sh: &Shared,
    net: &NetRef,
    ms: &MatchStart,
    local: &[Who],
    status_every: Option<std::time::Duration>,
) -> Launch {
    let setup = online::setup_from_start(&sh.content, ms);
    // Sides of the "VS" intro: the sim's teams (FFA: one per player).
    let vs_names: Vec<(String, u8, usize)> = ms
        .players
        .iter()
        .map(|p| {
            let s = setup.slots.get(p.slot as usize);
            (
                p.name.clone(),
                s.map_or(p.slot, |s| s.team),
                s.map_or(0, |s| s.skin as usize),
            )
        })
        .collect();
    let mut world = World::new(&sh.content, &setup);
    // Identical on every peer (from the host's rules; part of the checksum).
    world.set_pick_mode(
        online::NetRules::decode(&ms.settings.game)
            .rules
            .pick_mode
            .to_sim(),
    );
    let me = net.borrow().me();
    let controls = ms
        .players
        .iter()
        .map(|p| {
            if Some(p.machine) == me {
                match local.get(p.local_index as usize) {
                    Some(Who::Device(d)) => SlotControl::Device(*d),
                    _ => SlotControl::Bot,
                }
            } else {
                SlotControl::Remote
            }
        })
        .collect();
    let faces = ms
        .players
        .iter()
        .map(|p| online::decode_face(&p.extra))
        .collect();
    Launch {
        setup,
        pick_mode: rounds_sim::PickMode::Auto,
        controls,
        faces,
        driver: Some({
            let mut d = online::driver::OnlineDriver::new(net.clone(), sh.content.clone(), world);
            d.status_every = status_every;
            Box::new(d)
        }),
        bot_seed: ms.seed ^ me.map_or(0, |m| m.0 as u64 * 7919),
        bot_rematch: None,
        shots: Vec::new(),
        shot_prefix: None,
        vs_names,
    }
}

/// Settings key of a device's profile.
pub fn profile_key(d: DeviceId, controllers: &[crate::input::ControllerInfo]) -> String {
    match d {
        DeviceId::KeyboardMouse | DeviceId::KeyboardLeft => "keyboard".into(),
        DeviceId::KeyboardRight => "keyboard-right".into(),
        DeviceId::Controller(id) => {
            let name = controllers
                .iter()
                .find(|c| c.id == d)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "controller".into());
            let n = controllers
                .iter()
                .filter(|c| c.name == name && matches!(c.id, DeviceId::Controller(x) if x < id))
                .count();
            format!("controller:{name}#{n}")
        }
    }
}

pub fn colour_name(c: u8) -> &'static str {
    [
        "ORANGE", "BLUE", "RED", "GREEN", "YELLOW", "PURPLE", "CYAN", "PINK", "LIME", "INDIGO",
        "SKY", "AMBER",
    ][c as usize % 12]
}

/// Card grid: columns and card size for `n` cards in the area.
fn grid(n: usize, width: f32) -> (usize, Vec2, f32) {
    let cols = if n <= 8 { 4 } else { 6 };
    let rows = n.div_ceil(cols).max(1);
    let area_w = (width - 80.0).min(2400.0);
    let gap = 20.0;
    let w = ((area_w - gap * (cols as f32 - 1.0)) / cols as f32).min(440.0);
    let area_h = 760.0;
    let h = ((area_h - gap * (rows as f32 - 1.0)) / rows as f32).min(640.0);
    (cols, Vec2::new(w, h), gap)
}

impl Page for LobbyPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        self.t += 1;
        if let Some((_, n)) = &mut self.msg {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.msg = None;
            }
        }
        self.esc_armed = self.esc_armed.saturating_sub(1);
        let mut action = self.net_tick(cx);
        if !matches!(action, Action::None) {
            return action;
        }

        // Face editor overlay.
        if let Some(ed) = &mut self.editor {
            if ed.tick(cx) {
                self.editor = None;
            }
            return Action::None;
        }

        // Players whose controller was unplugged leave the lobby (they
        // could never ready up and would block the start).
        if !cx.platform.headless {
            let before = self.players.len();
            let pads = &cx.platform.controllers;
            self.players.retain(|p| match p.who {
                Who::Device(d @ DeviceId::Controller(_)) => pads.iter().any(|c| c.id == d),
                _ => true,
            });
            if self.players.len() != before {
                self.countdown = None;
                if self
                    .sfocus
                    .is_some_and(|(d, _)| !self.players.iter().any(|p| p.who == Who::Device(d)))
                {
                    self.sfocus = None;
                }
                self.msg = Some(("CONTROLLER DISCONNECTED".into(), 120));
            }
        }

        let input = cx.input;
        let teams = self.teams(cx.shared);
        // A knock (host, entry "approve") is modal: Enter / click accepts,
        // Delete / click declines.
        let modal = !self.knocks.is_empty();
        if let Some((knock, _)) = self.knocks.first().cloned() {
            let b = knock_buttons();
            let click = |k: usize| input.click && input.pointer.is_some_and(|p| b[k].0.contains(p));
            let accept = input.dev(DeviceId::KeyboardMouse).confirm || click(0);
            let decline = input.dev(DeviceId::KeyboardRight).back || click(1);
            if (accept || decline)
                && let Some(n) = &self.net
            {
                n.borrow_mut().answer_knock(&knock, accept);
                self.knocks.remove(0);
            }
        }
        // Joins.
        let mut just_joined: Vec<DeviceId> = Vec::new();
        for (d, n) in input.devs.iter().filter(|_| !modal) {
            let joined = self.players.iter().any(|p| match p.who {
                Who::Device(x) => {
                    x == *d
                        || (matches!(x, DeviceId::KeyboardMouse | DeviceId::KeyboardLeft)
                            && matches!(d, DeviceId::KeyboardMouse | DeviceId::KeyboardLeft))
                }
                Who::Bot => false,
            });
            if joined || *d == DeviceId::KeyboardLeft {
                continue;
            }
            if self.sfocus.is_some_and(|(x, _)| x == *d) {
                continue;
            }
            let wants = n.jump
                || (n.start && !d.is_keyboard())
                || (*d == DeviceId::KeyboardRight && n.start);
            if wants && self.players.len() < MAX_PLAYERS {
                let sh = &*cx.shared;
                self.add(sh, Who::Device(*d));
                let i = self.players.len() - 1;
                self.players[i].profile = profile_key(*d, &cx.platform.controllers);
                let prof = cx.shared.settings.profile(&self.players[i].profile);
                self.players[i].face = prof.face % FACE_PRESETS;
                if teams == 0 && !self.colour_taken(prof.colour % COLOURS, i) {
                    self.players[i].colour = prof.colour % COLOURS;
                }
                cx.shared.sfx(cx.audio, "UI_Player_Added");
                // The join press must not also ready the new player.
                just_joined.push(*d);
            } else if n.back && d.is_keyboard() && self.sfocus.is_none() {
                if self.players.is_empty() || self.esc_armed > 0 {
                    return self.leave(cx);
                }
                self.esc_armed = 105;
                self.msg = Some(("PRESS ESC AGAIN TO LEAVE".into(), 105));
            }
        }

        // Settings bar geometry.
        let n_set = SETTINGS.len();
        let bar_w = (cx.width - 80.0).min(1840.0);
        let item_w = bar_w / n_set as f32;
        let set_rects: Vec<Rect> = (0..n_set)
            .map(|i| {
                Rect::new(
                    Vec2::new(-bar_w * 0.5 + item_w * (i as f32 + 0.5), 95.0),
                    Vec2::new(item_w - 10.0, 84.0),
                )
            })
            .collect();

        // Per joined device.
        let mut remove: Option<usize> = None;
        let mut edit: Option<usize> = None;
        for i in 0..self.players.len() {
            self.players[i].t += 1;
            let Who::Device(d) = self.players[i].who else {
                continue;
            };
            if modal || just_joined.contains(&d) {
                continue;
            }
            let n = input.dev(d);
            if let Some((sd, item)) = self.sfocus
                && sd == d
            {
                let mut item = item;
                if n.left {
                    item = (item + n_set - 1) % n_set;
                }
                if n.right {
                    item = (item + 1) % n_set;
                }
                self.sfocus = Some((d, item));
                if n.confirm || n.jump {
                    let a = self.change_setting(cx, SETTINGS[item], 1);
                    cx.shared.sfx(cx.audio, "UI_Button_Click");
                    if !matches!(a, Action::None) {
                        return a;
                    }
                }
                if n.down || n.back {
                    self.sfocus = None;
                }
                continue;
            }
            if n.tab && matches!(d, DeviceId::KeyboardMouse | DeviceId::KeyboardLeft) {
                let nd = if d == DeviceId::KeyboardMouse {
                    DeviceId::KeyboardLeft
                } else {
                    DeviceId::KeyboardMouse
                };
                self.players[i].who = Who::Device(nd);
                self.msg = Some((nd.label().to_uppercase(), 90));
                continue;
            }
            if n.back {
                if self.players[i].ready {
                    self.players[i].ready = false;
                } else {
                    remove = Some(i);
                }
                self.countdown = None;
                continue;
            }
            if n.jump || n.start {
                self.players[i].ready = !self.players[i].ready;
                self.countdown = None;
                cx.shared.sfx(cx.audio, "UI_Button_Click");
                continue;
            }
            if self.players[i].ready {
                continue;
            }
            if n.edit {
                edit = Some(i);
            }
            if n.up {
                if self.players[i].row == 0 {
                    self.sfocus = Some((d, 0));
                } else {
                    self.players[i].row -= 1;
                }
            }
            if n.down {
                self.players[i].row = (self.players[i].row + 1).min(1);
            }
            if n.left || n.right {
                let row = self.players[i].row;
                self.change_row(cx.shared, i, row, if n.left { -1 } else { 1 });
                cx.shared.sfx(cx.audio, "UI_Button_Hover");
            }
        }

        // Cards and mouse.
        let cards = self.cards(cx.shared);
        let total_cards = (cards.len() + usize::from(cards.len() < MAX_PLAYERS)).max(4);
        let n_draw = if cards.len() < 4 {
            4
        } else {
            total_cards.min(MAX_PLAYERS)
        };
        let (cols, size, gap) = grid(n_draw, cx.width);
        let rows = n_draw.div_ceil(cols);
        let grid_h = rows as f32 * size.y + (rows as f32 - 1.0) * gap;
        let top = 160.0 + (780.0 - grid_h).max(0.0) * 0.5;
        let card_rect = |k: usize| {
            let (r, c) = (k / cols, k % cols);
            let in_row = if r + 1 == rows {
                n_draw - r * cols
            } else {
                cols
            };
            let row_w = in_row as f32 * size.x + (in_row as f32 - 1.0) * gap;
            Rect::new(
                Vec2::new(
                    -row_w * 0.5 + size.x * 0.5 + c as f32 * (size.x + gap),
                    top + size.y * 0.5 + r as f32 * (size.y + gap),
                ),
                size,
            )
        };
        let k_ui = size.x / 420.0;
        // Mouse hit areas on local cards: arrows, face, ready, remove.
        if (input.click || input.right_click)
            && let Some(p) = input.pointer
        {
            {
                if let Some(si) = set_rects.iter().position(|r| r.contains(p)) {
                    let a = self.change_setting(
                        cx,
                        SETTINGS[si],
                        if input.right_click { -1 } else { 1 },
                    );
                    cx.shared.sfx(cx.audio, "UI_Button_Click");
                    if !matches!(a, Action::None) {
                        return a;
                    }
                }
                for (k, cv) in cards.iter().enumerate() {
                    // Host: a click on another machine's player moves it to
                    // the next team (set_team overrides its request).
                    if let (Some((m, li, team)), Some(net)) = (cv.remote, &self.net)
                        && teams > 0
                        && card_rect(k).contains(p)
                        && net.borrow().is_host()
                    {
                        let dir: i32 = if input.right_click { -1 } else { 1 };
                        let t = (team as i32 + dir).rem_euclid(teams as i32) as u8;
                        let _ = net.borrow_mut().set_team(m, li, Some(t));
                        cx.shared.sfx(cx.audio, "UI_Button_Hover");
                    }
                    let Some(li) = cv.local else { continue };
                    let r = card_rect(k);
                    if !r.contains(p) {
                        continue;
                    }
                    let (face_c, face_r) = face_geom(r);
                    let rel = p - r.c;
                    if (p - face_c).length() < face_r {
                        if input.right_click || input.click {
                            edit = Some(li);
                        }
                    } else if let Some(row) = row_at(r, p) {
                        let dir = if rel.x < 0.0 { -1 } else { 1 };
                        if !self.players[li].ready {
                            self.change_row(cx.shared, li, row, dir);
                            cx.shared.sfx(cx.audio, "UI_Button_Hover");
                        }
                    } else if rel.y > r.size.y * 0.32 {
                        self.players[li].ready = !self.players[li].ready;
                        self.countdown = None;
                        cx.shared.sfx(cx.audio, "UI_Button_Click");
                    } else if rel.y < -r.size.y * 0.38 && rel.x > r.size.x * 0.3 {
                        remove = Some(li);
                    }
                }
                // Click on the join card joins the keyboard + mouse player.
                let jk = cards.len();
                if jk < n_draw
                    && card_rect(jk).contains(p)
                    && !self.players.iter().any(|x| {
                        matches!(
                            x.who,
                            Who::Device(DeviceId::KeyboardMouse | DeviceId::KeyboardLeft)
                        )
                    })
                {
                    let sh = &*cx.shared;
                    self.add(sh, Who::Device(DeviceId::KeyboardMouse));
                    cx.shared.sfx(cx.audio, "UI_Player_Added");
                }
            }
        }
        self.hover = input
            .pointer
            .and_then(|p| set_rects.iter().position(|r| r.contains(p)));
        if let Some(i) = remove {
            self.players.remove(i);
            if self
                .sfocus
                .is_some_and(|(d, _)| !self.players.iter().any(|p| p.who == Who::Device(d)))
            {
                self.sfocus = None;
            }
            self.countdown = None;
        }
        if let Some(i) = edit
            && i < self.players.len()
        {
            let p = &self.players[i];
            let owner = match p.who {
                Who::Bot => None,
                Who::Device(d) => Some(d),
            };
            let skin = if teams > 0 {
                p.team as usize
            } else {
                p.colour as usize
            };
            self.editor = Some(FaceEditor::new(owner, p.face, skin));
            cx.shared.sfx(cx.audio, "UI_Button_Click");
        }

        // Start.
        let blocker = self.blocker(cx.shared);
        if blocker.is_none() && !self.demo {
            let c = self.countdown.get_or_insert(ticks(1.5));
            if *c == 0 {
                self.countdown = None;
                cx.shared.settings.save();
                if let Some(net) = &self.net {
                    if let Err(e) = net.borrow_mut().start_match(None) {
                        self.msg = Some((format!("CANNOT START: {e}").to_uppercase(), 200));
                    }
                } else {
                    action = Action::StartMatch(Box::new(self.local_launch(cx.shared)));
                }
            } else {
                *c -= 1;
            }
        } else {
            self.countdown = None;
        }

        // ------------------------------------------------------ draw
        let sh = &*cx.shared;
        let editable = self.can_edit_settings();
        for (i, s) in SETTINGS.iter().enumerate() {
            let r = set_rects[i];
            let focused = self.sfocus.is_some_and(|(_, k)| k == i) || self.hover == Some(i);
            cx.frame.rect(
                r,
                10.0,
                if focused {
                    style::BAR
                } else {
                    style::PANEL_LIGHT
                },
            );
            let (label, value) = self.setting_text(sh, *s);
            let grey = !editable && !matches!(s, Setting::AddBot | Setting::RemoveBot);
            if label.is_empty() {
                cx.frame.text(
                    value,
                    r.c,
                    40.0,
                    Vec2::splat(0.5),
                    if focused { Font::Bold } else { Font::Light },
                    style::TEXT,
                );
            } else {
                cx.frame.text(
                    label,
                    r.c + Vec2::new(0.0, -20.0),
                    20.0,
                    Vec2::splat(0.5),
                    Font::Regular,
                    style::GREY,
                );
                // Long values (FREE-FOR-ALL) shrink to fit the item.
                let size = if value.chars().count() > 10 {
                    28.0
                } else {
                    34.0
                };
                cx.frame.text(
                    value,
                    r.c + Vec2::new(0.0, 14.0),
                    size,
                    Vec2::splat(0.5),
                    if focused { Font::Bold } else { Font::Light },
                    if grey { style::GREY } else { style::TEXT },
                );
            }
        }
        // Room line (online).
        if self.net.is_some() || self.room.is_some() {
            let host = self.net.as_ref().is_none_or(|n| n.borrow().is_host());
            let line = match &self.room {
                Some((id, code, _)) => format!(
                    "{}ROOM {}{}",
                    if host { "HOSTING · " } else { "" },
                    code.clone().unwrap_or_else(|| id.chars().take(8).collect()),
                    if host {
                        "  ·  SHARE THE CODE TO INVITE"
                    } else {
                        ""
                    }
                ),
                None => "ONLINE LOBBY".into(),
            };
            cx.frame.text(
                line,
                Vec2::new(0.0, 150.0),
                26.0,
                Vec2::splat(0.5),
                Font::Regular,
                style::GREY,
            );
            // Invite link to the clipboard (click, or C on the keyboard).
            if let Some((_, _, invite)) = &self.room
                && !invite.is_empty()
            {
                let b = Rect::new(
                    Vec2::new((cx.width * 0.5 - 260.0).max(560.0), 150.0),
                    Vec2::new(380.0, 52.0),
                );
                let hover = cx.input.pointer.is_some_and(|p| b.contains(p));
                cx.frame.rect(
                    b,
                    10.0,
                    if hover {
                        style::PANEL_LIGHT
                    } else {
                        style::PANEL
                    },
                );
                cx.frame.text(
                    "COPY INVITE LINK (C)",
                    b.c,
                    24.0,
                    Vec2::splat(0.5),
                    if hover { Font::Bold } else { Font::Regular },
                    style::TEXT,
                );
                let key = cx
                    .raw
                    .iter()
                    .any(|p| p.device == DeviceId::KeyboardMouse && p.name == "C");
                if (cx.input.click && hover) || key {
                    cx.requests.clipboard = Some(invite.clone());
                    self.msg = Some(("INVITE LINK COPIED".into(), 90));
                    cx.shared.sfx(cx.audio, "UI_Button_Click");
                }
            }
        }
        let join_text = cx.shared.up("PROMPT_JUMP_TO_JOIN", "Press jump\nto join");
        for k in 0..n_draw {
            let r = card_rect(k);
            match cards.get(k) {
                Some(cv) => draw_card(cx, cv, r, k_ui, self.t),
                None => {
                    let flick = 0.45 + 0.1 * ((self.t as f32 * 0.07 + k as f32).sin());
                    cx.frame.rect(r, 18.0 * k_ui, style::PANEL_LIGHT);
                    if k == cards.len() {
                        cx.frame.text(
                            join_text.clone(),
                            r.c,
                            54.0 * k_ui.max(0.6),
                            Vec2::splat(0.5),
                            Font::Light,
                            Color4::grey(flick),
                        );
                        // The join buttons of every kind of device.
                        use crate::ui::{Glyph, Part};
                        let mut parts = vec![Part::Text("SPACE · RIGHT SHIFT".into())];
                        let pads = &cx.platform.controllers;
                        if !pads.is_empty() || cx.platform.headless {
                            parts.push(Part::Text("·".into()));
                            parts.push(Part::Glyph(Glyph::South));
                        }
                        let ps = pads.iter().any(|c| crate::ui::is_playstation(&c.name));
                        let glyphs = cx.shared.glyphs.clone();
                        cx.frame.prompt(
                            &glyphs,
                            ps,
                            &parts,
                            Vec2::new(r.c.x, r.c.y + 130.0 * k_ui.max(0.6)),
                            26.0 * k_ui.max(0.6),
                            Font::Regular,
                            Color4::grey(flick),
                        );
                    }
                }
            }
        }
        // Status line.
        let status = match (&self.countdown, &blocker) {
            (Some(c), _) => format!("STARTING IN {}", c / 71 + 1),
            (None, Some(b)) if !b.is_empty() => b.clone(),
            _ => {
                if self.players.iter().any(|p| !p.ready) {
                    "PRESS JUMP / START WHEN READY".into()
                } else {
                    String::new()
                }
            }
        };
        cx.frame.text(
            status,
            Vec2::new(0.0, 985.0),
            44.0,
            Vec2::splat(0.5),
            Font::Light,
            style::TEXT,
        );
        {
            use crate::ui::{Glyph, Part};
            let t = |s: &str| Part::Text(s.into());
            let help = [
                t("JOIN: SPACE · RIGHT SHIFT ·"),
                Part::Glyph(Glyph::South),
                t("   MOVE: CHANGE    EDIT FACE: E /"),
                Part::Glyph(Glyph::North),
                t("   TAB: KEYS ONLY    LEAVE: ESC /"),
                Part::Glyph(Glyph::East),
            ];
            let ps = cx
                .platform
                .controllers
                .iter()
                .any(|c| crate::ui::is_playstation(&c.name));
            let glyphs = cx.shared.glyphs.clone();
            cx.frame.prompt(
                &glyphs,
                ps,
                &help,
                Vec2::new(0.0, 1040.0),
                22.0,
                Font::Regular,
                style::DIM,
            );
        }
        if let Some((m, _)) = &self.msg {
            cx.frame.rect(
                Rect::new(Vec2::new(0.0, 540.0), Vec2::new(1100.0, 90.0)),
                12.0,
                style::PANEL,
            );
            cx.frame.text(
                m.clone(),
                Vec2::new(0.0, 540.0),
                40.0,
                Vec2::splat(0.5),
                Font::Regular,
                style::TEXT,
            );
        }
        // Knocks (host, entry "approve").
        if let Some((_, user)) = self.knocks.first() {
            cx.frame.rect(
                Rect::new(Vec2::new(0.0, 470.0), Vec2::new(1000.0, 160.0)),
                14.0,
                style::PANEL,
            );
            cx.frame.text(
                format!("{} WANTS TO JOIN", user.to_uppercase()),
                Vec2::new(0.0, 435.0),
                44.0,
                Vec2::splat(0.5),
                Font::Light,
                style::TEXT,
            );
            for (r, l) in knock_buttons() {
                cx.frame.rect(r, 10.0, style::BAR);
                cx.frame
                    .text(l, r.c, 30.0, Vec2::splat(0.5), Font::Regular, style::TEXT);
            }
        }
        action
    }

    fn match_ended(&mut self, outcome: &MatchOutcome, _shared: &mut Shared) {
        for p in &mut self.players {
            p.ready = p.who == Who::Bot;
        }
        self.last_ready = None;
        self.msg = match outcome {
            MatchOutcome::Aborted(r) => Some((format!("MATCH ABORTED: {r}").to_uppercase(), 300)),
            MatchOutcome::OnlineEnded { agreed: false, .. } => Some((
                "MATCH ENDED (RESULTS DIFFERED BETWEEN MACHINES)".into(),
                300,
            )),
            _ => None,
        };
    }

    fn left_match(&mut self, shared: &mut Shared) {
        for p in &mut self.players {
            p.ready = p.who == Who::Bot;
        }
        self.last_ready = None;
        if let Some(n) = &self.net {
            // Leaving an online match leaves the room (the others see us drop).
            n.borrow_mut().leave();
            shared.net = None;
        }
    }
}

impl LobbyPage {
    fn leave(&mut self, cx: &mut Cx<'_>) -> Action {
        if let Some(n) = self.net.take() {
            n.borrow_mut().leave();
            cx.shared.net = None;
        }
        Action::Pop
    }
}

fn knock_buttons() -> [(Rect, &'static str); 2] {
    [
        (
            Rect::new(Vec2::new(-200.0, 505.0), Vec2::new(340.0, 56.0)),
            "ACCEPT (ENTER)",
        ),
        (
            Rect::new(Vec2::new(200.0, 505.0), Vec2::new(340.0, 56.0)),
            "DECLINE (DEL)",
        ),
    ]
}

fn ticks(s: f32) -> u32 {
    (s / 0.014).ceil() as u32
}

/// Face centre and radius of a card.
fn face_geom(r: Rect) -> (Vec2, f32) {
    let rad = (r.size.x.min(r.size.y * 0.8)) * 0.2;
    (Vec2::new(r.c.x, r.top() + r.size.y * 0.36), rad)
}

/// Rows (face, colour/team) of a card: centre y.
fn row_y(r: Rect, row: usize) -> f32 {
    r.top() + r.size.y * (0.64 + 0.11 * row as f32)
}

fn row_at(r: Rect, p: Vec2) -> Option<usize> {
    (0..2).find(|&row| (p.y - row_y(r, row)).abs() < r.size.y * 0.05)
}

/// Grey with alpha (placeholder text flicker).
struct Color4;
impl Color4 {
    fn grey(a: f32) -> rounds_render::Color {
        rounds_render::Color::rgba(0.9, 0.9, 0.9, a)
    }
}

fn draw_card(cx: &mut Cx<'_>, cv: &CardView, r: Rect, k: f32, t: u32) {
    let sh = &*cx.shared;
    let body = sh.colour(cv.skin);
    let win = sh.win_colour(cv.skin);
    let mut bg = sh
        .assets
        .as_ref()
        .map(|a| a.skin(cv.skin).background)
        .unwrap_or(style::GREY);
    bg.a = 0.55;
    cx.frame.rect(r, 18.0 * k, bg);
    if cv.focus_row.is_some() {
        cx.frame.ring(
            Vec2::new(r.left() + 26.0 * k, r.top() + 26.0 * k),
            8.0 * k,
            4.0 * k,
            win,
        );
    }
    let ks = k.max(0.62);
    cx.frame.text(
        cv.title.clone(),
        Vec2::new(r.left() + 44.0 * k, r.top() + 28.0 * k),
        34.0 * ks,
        Vec2::new(0.0, 0.5),
        Font::Bold,
        win,
    );
    cx.frame.text(
        cv.sub.clone(),
        Vec2::new(r.c.x, r.top() + 64.0 * k),
        18.0 * ks,
        Vec2::new(0.5, 0.5),
        Font::Regular,
        style::GREY,
    );
    if cv.bot {
        cx.frame.text(
            "×",
            Vec2::new(r.right() - 28.0 * k, r.top() + 28.0 * k),
            40.0 * ks,
            Vec2::splat(0.5),
            Font::Fallback,
            style::GREY,
        );
    }
    let (fc, fr) = face_geom(r);
    let bob = ((t as f32) * 0.05).sin() * 3.0 * k;
    let items = sh.face_items(&cv.face);
    cx.frame
        .face(items, fc + Vec2::new(0.0, bob), fr, body, 1.0);
    if let Some((a, b)) = &cv.row_values {
        for (row, v) in [a, b].iter().enumerate() {
            let y = row_y(r, row);
            let focused = cv.focus_row == Some(row);
            if focused {
                cx.frame.rect(
                    Rect::new(
                        Vec2::new(r.c.x, y),
                        Vec2::new(r.size.x - 20.0 * k, r.size.y * 0.1),
                    ),
                    8.0 * k,
                    style::BAR,
                );
            }
            let col = if focused { style::TEXT } else { style::GREY };
            cx.frame.text(
                "<",
                Vec2::new(r.left() + 30.0 * k, y),
                36.0 * ks,
                Vec2::splat(0.5),
                Font::Light,
                col,
            );
            cx.frame.text(
                ">",
                Vec2::new(r.right() - 30.0 * k, y),
                36.0 * ks,
                Vec2::splat(0.5),
                Font::Light,
                col,
            );
            if row == 1 {
                cx.frame
                    .circle(Vec2::new(r.left() + 70.0 * k, y), 11.0 * k, body);
            }
            cx.frame.text(
                (*v).clone(),
                Vec2::new(r.c.x, y),
                30.0 * ks,
                Vec2::splat(0.5),
                if focused { Font::Bold } else { Font::Regular },
                col,
            );
        }
    }
    let ry = r.bottom() - r.size.y * 0.09;
    if cv.ready {
        cx.frame.rect(
            Rect::new(
                Vec2::new(r.c.x, ry),
                Vec2::new(r.size.x - 24.0 * k, r.size.y * 0.13),
            ),
            10.0 * k,
            win,
        );
        cx.frame.text(
            sh.up("PROMPT_READY", "Ready"),
            Vec2::new(r.c.x, ry),
            50.0 * ks,
            Vec2::splat(0.5),
            Font::Light,
            style::DARK_TEXT,
        );
    } else {
        use crate::ui::{Glyph, Part};
        let press = |p: Part| {
            vec![
                Part::Text("PRESS".into()),
                p,
                Part::Text("WHEN READY".into()),
            ]
        };
        let (parts, ps) = match cv.device {
            Some(DeviceId::Controller(id)) => (
                press(Part::Glyph(Glyph::South)),
                cx.platform.controllers.iter().any(|c| {
                    c.id == DeviceId::Controller(id) && crate::ui::is_playstation(&c.name)
                }),
            ),
            Some(DeviceId::KeyboardRight) => (press(Part::Text("RIGHT SHIFT".into())), false),
            Some(_) => (press(Part::Text("SPACE".into())), false),
            None if cv.local.is_some() => (vec![Part::Text("PRESS JUMP WHEN READY".into())], false),
            None => (vec![Part::Text("NOT READY".into())], false),
        };
        let glyphs = cx.shared.glyphs.clone();
        cx.frame.prompt(
            &glyphs,
            ps,
            &parts,
            Vec2::new(r.c.x, ry),
            24.0 * ks,
            Font::Regular,
            style::GREY,
        );
    }
}
