//! Online pages over Warpzone (ui.md 2.8, replaced by Warpzone rooms):
//! username registration on first use, the online home (host, browse,
//! join by code), host settings (public/unlisted, entry open/code/approve,
//! free-for-all/teams preset), the room browser, join by code, connecting,
//! then the online lobby ([`LobbyPage::online`]). Also [`AutoPage`]: the
//! automated host/join with bot players used by `scripts/net/online-test.sh`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glam::Vec2;
use rounds_net::warpzone_core::client::{Event as WzEvent, HostOptions};
use rounds_net::warpzone_core::protocol::api::{Entry, RoomInfo, Visibility};
use rounds_net::{
    LaunchAction, MatchSettings, NetConfig, NetEvent, NetSession, SlotRequest, TeamMode as NetTeams,
};

use super::lobby::{LobbyPage, Who, online_launch_with};
use super::pages::{button_column, hint, title};
use super::{Action, Cx, Page, Shared};
use crate::online::{NetRef, NetRules, game_version};
use crate::play::driver::MatchOutcome;
use crate::settings::{Face, MatchRules};
use crate::ui::{Font, ListNav, Rect, TextField, bar, style};

/// True if the command line asks to join or host (launcher, invite link).
pub fn launch_intent(args: &[String]) -> bool {
    let i = rounds_net::warpzone_core::launch::from_args(args);
    i.join.is_some() || i.host
}

/// The session, created on first use.
pub fn ensure_net(sh: &mut Shared) -> Result<NetRef, String> {
    if let Some(n) = &sh.net {
        return Ok(n.clone());
    }
    let mut cfg = NetConfig::new(&game_version(&sh.data));
    cfg.auto_register = sh.opts.username.clone();
    match NetSession::new(cfg) {
        Ok(n) => {
            let n = Rc::new(RefCell::new(n));
            sh.net = Some(n.clone());
            Ok(n)
        }
        Err(e) => Err(format!("{e}")),
    }
}

fn status_text(cx: &mut Cx<'_>, text: &str, y: f32, color: rounds_render::Color) {
    cx.frame.text(
        text,
        Vec2::new(0.0, y),
        36.0,
        Vec2::splat(0.5),
        Font::Regular,
        color,
    );
}

// ------------------------------------------------------------- home

pub struct OnlineHomePage {
    nav: ListNav,
    msg: Option<String>,
    user: Option<String>,
    from_launch: bool,
    launch_done: bool,
    demo: bool,
    asked_register: bool,
}

impl OnlineHomePage {
    pub fn new(from_launch: bool) -> OnlineHomePage {
        OnlineHomePage {
            nav: ListNav::default(),
            msg: None,
            user: None,
            from_launch,
            launch_done: false,
            demo: false,
            asked_register: false,
        }
    }

    pub fn with_message(msg: String) -> OnlineHomePage {
        let mut p = Self::new(false);
        p.msg = Some(msg);
        p
    }
}

impl Page for OnlineHomePage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, &cx.shared.up("BUTTON_ONLINE", "Online"), 200.0);
        let net = if self.demo {
            None
        } else {
            match ensure_net(cx.shared) {
                Ok(n) => Some(n),
                Err(e) => {
                    self.msg = Some(format!("WARPZONE: {e}").to_uppercase());
                    None
                }
            }
        };
        let mut action = Action::None;
        if let Some(net) = &net {
            if self.user.is_none() {
                self.user = net
                    .borrow_mut()
                    .with_warpzone(|c| c.account().map(|a| a.username))
                    .flatten();
            }
            net.borrow_mut().poll();
            loop {
                let e = net.borrow_mut().next_event();
                let Some(e) = e else { break };
                if let NetEvent::Warpzone(w) = e {
                    match *w {
                        WzEvent::LoggedIn { account, .. } => match account {
                            Some(a) => self.user = Some(a.username),
                            None if cx.shared.opts.username.is_none() && !self.asked_register => {
                                self.asked_register = true;
                                action = Action::Push(Box::new(RegisterPage::default()));
                            }
                            None => {}
                        },
                        WzEvent::Registered(a) => self.user = Some(a.username),
                        WzEvent::Error { op, message, .. } => {
                            self.msg = Some(format!("{op}: {message}").to_uppercase())
                        }
                        _ => {}
                    }
                }
            }
            if self.from_launch && !self.launch_done && self.user.is_some() {
                self.launch_done = true;
                let args = cx.shared.opts.launch_args.clone();
                match net.borrow_mut().handle_launch(&args) {
                    LaunchAction::Joining { room } => {
                        return Action::Push(Box::new(ConnectingPage::new(format!(
                            "JOINING {room}"
                        ))));
                    }
                    LaunchAction::HostScreen => return Action::Push(Box::new(HostPage::default())),
                    LaunchAction::None => {}
                }
            }
        }
        let logged = self.user.is_some();
        let sh = &cx.shared;
        let labels = vec![
            sh.up("BUTTON_HOSTROOM", "Host room"),
            "BROWSE ROOMS".into(),
            "JOIN BY CODE".into(),
            sh.up("BUTTON_BACK", "Back"),
        ];
        let enabled = [logged, logged, logged, true];
        let ev = button_column(cx, &mut self.nav, &labels, &enabled, 440.0, 76.0, 110.0);
        let line = match &self.user {
            Some(u) => format!("SIGNED IN AS {}", u.to_uppercase()),
            None => "CONNECTING TO WARPZONE...".into(),
        };
        status_text(cx, &line, 310.0, style::GREY);
        if let Some(m) = &self.msg {
            status_text(cx, m, 920.0, style::NOPE);
        }
        hint(
            cx,
            "ROOMS, CODES AND INVITES BY WARPZONE · UP TO 12 PLAYERS ON ANY NUMBER OF MACHINES",
        );
        if !matches!(action, Action::None) {
            return action;
        }
        if ev.back {
            return Action::Pop;
        }
        match ev.activate {
            Some(0) => Action::Push(Box::new(HostPage::default())),
            Some(1) => Action::Push(Box::new(BrowsePage::default())),
            Some(2) => Action::Push(Box::new(JoinCodePage::default())),
            Some(3) => Action::Pop,
            _ => Action::None,
        }
    }

    fn resumed(&mut self, sh: &mut Shared) {
        if let Some(n) = &sh.net {
            self.user = n
                .borrow_mut()
                .with_warpzone(|c| c.account().map(|a| a.username))
                .flatten();
        }
    }
}

// ------------------------------------------------------------- register

pub struct RegisterPage {
    field: TextField,
    msg: Option<String>,
    sent: bool,
}

impl Default for RegisterPage {
    fn default() -> Self {
        RegisterPage {
            field: TextField::new(16),
            msg: None,
            sent: false,
        }
    }
}

impl Page for RegisterPage {
    fn text_input(&self) -> bool {
        true
    }

    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, "CHOOSE A NAME", 250.0);
        status_text(
            cx,
            "YOUR WARPZONE USERNAME, SHOWN TO OTHER PLAYERS",
            360.0,
            style::GREY,
        );
        self.field.edit(cx.input, |c| {
            c.is_ascii_alphanumeric() || c == '_' || c == '-'
        });
        let r = Rect::new(Vec2::new(0.0, 520.0), Vec2::new(900.0, 120.0));
        cx.frame.rect(r, 14.0, style::PANEL);
        let caret = if (cx.shared.t / 30).is_multiple_of(2) {
            "_"
        } else {
            " "
        };
        cx.frame.text(
            format!("{}{caret}", self.field.value),
            r.c,
            70.0,
            Vec2::splat(0.5),
            Font::Light,
            style::TEXT,
        );
        status_text(cx, "ENTER: REGISTER    ESC: BACK", 660.0, style::GREY);
        if let Some(m) = &self.msg {
            status_text(cx, m, 760.0, style::NOPE);
        }
        if let Some(net) = cx.shared.net.clone() {
            net.borrow_mut().poll();
            loop {
                let e = net.borrow_mut().next_event();
                let Some(e) = e else { break };
                if let NetEvent::Warpzone(w) = e {
                    match *w {
                        WzEvent::Registered(_) => return Action::Pop,
                        WzEvent::Error { message, .. } => {
                            self.sent = false;
                            self.msg = Some(message.to_uppercase());
                        }
                        _ => {}
                    }
                }
            }
            if cx.input.nav.confirm && !self.sent && self.field.value.len() >= 3 {
                net.borrow_mut().register(&self.field.value);
                self.sent = true;
                self.msg = Some("REGISTERING...".into());
            }
        }
        if cx.input.nav.back {
            return Action::Pop;
        }
        Action::None
    }
}

// ------------------------------------------------------------- host

pub struct HostPage {
    nav: ListNav,
    public: bool,
    entry: u8,
    teams: bool,
    demo: bool,
}

impl Default for HostPage {
    fn default() -> Self {
        HostPage {
            nav: ListNav::default(),
            public: true,
            entry: 0,
            teams: false,
            demo: false,
        }
    }
}

impl Page for HostPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, &cx.shared.up("BUTTON_HOSTROOM", "Host room"), 200.0);
        if self.nav.t == 0 {
            self.public = cx.shared.settings.online.public;
            self.entry = cx.shared.settings.online.entry;
            self.teams = cx.shared.settings.rules.teams > 0;
        }
        let entry = ["OPEN", "CODE REQUIRED", "HOST APPROVES"][self.entry as usize % 3];
        let labels = vec![
            format!("ROOM: {}", if self.public { "PUBLIC" } else { "UNLISTED" }),
            format!("ENTRY: {entry}"),
            format!(
                "MODE: {}",
                if self.teams { "TEAMS" } else { "FREE-FOR-ALL" }
            ),
            cx.shared.up("BUTTON_HOST", "Host"),
            cx.shared.up("BUTTON_BACK", "Back"),
        ];
        let ev = button_column(cx, &mut self.nav, &labels, &[true; 5], 400.0, 64.0, 100.0);
        hint(
            cx,
            "LEFT / RIGHT OR CLICK TO CHANGE · UNLISTED ROOMS ARE REACHED BY CODE OR INVITE LINK",
        );
        let mut toggle = |i: usize, d: i32| match i {
            0 => self.public = !self.public,
            1 => self.entry = (self.entry as i32 + if d == 0 { 1 } else { d }).rem_euclid(3) as u8,
            2 => self.teams = !self.teams,
            _ => {}
        };
        if let Some((i, d)) = ev.change {
            toggle(i, d);
        }
        if ev.back {
            return Action::Pop;
        }
        match ev.activate {
            Some(i @ 0..=2) => toggle(i, 0),
            Some(3) => {
                let sh = &mut *cx.shared;
                sh.settings.online.public = self.public;
                sh.settings.online.entry = self.entry;
                if self.teams && sh.settings.rules.teams == 0 {
                    sh.settings.rules.teams = 2;
                } else if !self.teams {
                    sh.settings.rules.teams = 0;
                }
                sh.settings.save();
                if self.demo {
                    return Action::None;
                }
                let Ok(net) = ensure_net(sh) else {
                    return Action::None;
                };
                net.borrow_mut()
                    .host(host_options(self.public, self.entry, self.teams));
                return Action::Push(Box::new(ConnectingPage::new("HOSTING".into())));
            }
            Some(4) => return Action::Pop,
            _ => {}
        }
        Action::None
    }
}

pub fn host_options(public: bool, entry: u8, teams: bool) -> HostOptions {
    HostOptions {
        max_players: 12,
        visibility: if public {
            Visibility::Public
        } else {
            Visibility::Unlisted
        },
        entry: match entry {
            1 => Entry::Code,
            2 => Entry::Approve,
            _ => Entry::Open,
        },
        preset: Some(if teams { "teams" } else { "ffa" }.into()),
        ..Default::default()
    }
}

// ------------------------------------------------------------- browse

#[derive(Default)]
pub struct BrowsePage {
    rooms: Vec<RoomInfo>,
    focus: usize,
    last: Option<Instant>,
    msg: Option<String>,
    demo: bool,
}

impl Page for BrowsePage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, "ROOMS", 110.0);
        let net = if self.demo {
            None
        } else {
            ensure_net(cx.shared).ok()
        };
        if let Some(net) = &net {
            if self
                .last
                .is_none_or(|t| t.elapsed() > Duration::from_secs(4))
            {
                net.borrow_mut().list_rooms(false);
                self.last = Some(Instant::now());
            }
            net.borrow_mut().poll();
            loop {
                let e = net.borrow_mut().next_event();
                let Some(e) = e else { break };
                if let NetEvent::Warpzone(w) = e {
                    match *w {
                        WzEvent::RoomList(r) => self.rooms = r,
                        WzEvent::Error { message, .. } => self.msg = Some(message.to_uppercase()),
                        _ => {}
                    }
                }
            }
        }
        let input = cx.input;
        let n = self.rooms.len();
        let row_h = 74.0;
        let rects: Vec<Rect> = (0..n.min(10))
            .map(|i| {
                Rect::new(
                    Vec2::new(0.0, 250.0 + i as f32 * row_h),
                    Vec2::new(1500.0, row_h - 8.0),
                )
            })
            .collect();
        let back_r = Rect::new(Vec2::new(0.0, 1000.0), Vec2::new(400.0, 70.0));
        let total = rects.len() + 1;
        if input.nav.up {
            self.focus = (self.focus + total - 1) % total;
        }
        if input.nav.down {
            self.focus = (self.focus + 1) % total;
        }
        if input.mouse_mode
            && let Some(p) = input.pointer
        {
            if let Some(i) = rects.iter().position(|r| r.contains(p)) {
                self.focus = i;
            } else if back_r.contains(p) {
                self.focus = rects.len();
            }
        }
        self.focus = self.focus.min(total - 1);
        let mut act = input.nav.confirm.then_some(self.focus);
        if input.click
            && let Some(p) = input.pointer
        {
            act = rects
                .iter()
                .position(|r| r.contains(p))
                .or(back_r.contains(p).then_some(rects.len()))
                .or(act);
        }
        cx.frame.text(
            "ROOM",
            Vec2::new(-720.0, 195.0),
            24.0,
            Vec2::new(0.0, 0.5),
            Font::Regular,
            style::GREY,
        );
        cx.frame.text(
            "PLAYERS",
            Vec2::new(150.0, 195.0),
            24.0,
            Vec2::new(0.5, 0.5),
            Font::Regular,
            style::GREY,
        );
        cx.frame.text(
            "MODE",
            Vec2::new(420.0, 195.0),
            24.0,
            Vec2::new(0.5, 0.5),
            Font::Regular,
            style::GREY,
        );
        cx.frame.text(
            "ENTRY",
            Vec2::new(650.0, 195.0),
            24.0,
            Vec2::new(0.5, 0.5),
            Font::Regular,
            style::GREY,
        );
        for (i, r) in rects.iter().enumerate() {
            let room = &self.rooms[i];
            if i == self.focus {
                cx.frame.rect(*r, 10.0, style::BAR);
            } else {
                cx.frame.rect(*r, 10.0, style::PANEL_LIGHT);
            }
            let f = if i == self.focus {
                Font::Bold
            } else {
                Font::Regular
            };
            cx.frame.text(
                room.name.to_uppercase(),
                Vec2::new(-720.0, r.c.y),
                34.0,
                Vec2::new(0.0, 0.5),
                f,
                style::TEXT,
            );
            let players = room
                .metadata
                .get("players")
                .cloned()
                .unwrap_or_else(|| format!("{}/{}", room.players, room.max_players));
            cx.frame.text(
                players,
                Vec2::new(150.0, r.c.y),
                34.0,
                Vec2::splat(0.5),
                f,
                style::TEXT,
            );
            let mode = room
                .metadata
                .get("mode")
                .cloned()
                .unwrap_or_default()
                .to_uppercase();
            cx.frame.text(
                mode,
                Vec2::new(420.0, r.c.y),
                30.0,
                Vec2::splat(0.5),
                f,
                style::TEXT,
            );
            let entry = match room.entry {
                Entry::Open => "OPEN",
                Entry::Code => "CODE",
                Entry::Approve => "ASK",
            };
            cx.frame.text(
                entry,
                Vec2::new(650.0, r.c.y),
                30.0,
                Vec2::splat(0.5),
                f,
                style::GREY,
            );
        }
        if n == 0 {
            status_text(
                cx,
                if self.last.is_some() || self.demo {
                    "NO ROOMS YET - HOST ONE!"
                } else {
                    "LOOKING FOR ROOMS..."
                },
                450.0,
                style::GREY,
            );
        }
        if self.focus == rects.len() {
            bar(cx.frame, cx.input, back_r);
        }
        cx.frame.text(
            cx.shared.up("BUTTON_BACK", "Back"),
            back_r.c,
            50.0,
            Vec2::splat(0.5),
            if self.focus == rects.len() {
                Font::Bold
            } else {
                Font::Light
            },
            style::TEXT,
        );
        if let Some(m) = &self.msg {
            status_text(cx, m, 940.0, style::NOPE);
        }
        if input.nav.back {
            return Action::Pop;
        }
        match act {
            Some(i) if i < rects.len() => {
                let room = self.rooms[i].clone();
                if room.entry == Entry::Code {
                    return Action::Push(Box::new(JoinCodePage::default()));
                }
                if let Some(net) = &net {
                    net.borrow_mut().join(&room.id, None, None);
                    return Action::Push(Box::new(ConnectingPage::new(format!(
                        "JOINING {}",
                        room.name.to_uppercase()
                    ))));
                }
                Action::None
            }
            Some(_) => Action::Pop,
            None => Action::None,
        }
    }
}

// ------------------------------------------------------------- code

pub struct JoinCodePage {
    field: TextField,
    demo: bool,
}

impl Default for JoinCodePage {
    fn default() -> Self {
        JoinCodePage {
            field: TextField::new(6),
            demo: false,
        }
    }
}

impl Page for JoinCodePage {
    fn text_input(&self) -> bool {
        true
    }

    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        title(cx, "JOIN BY CODE", 250.0);
        let mut input = cx.input.clone();
        input.text = input.text.to_uppercase();
        self.field.edit(&input, |c| c.is_ascii_alphanumeric());
        let r = Rect::new(Vec2::new(0.0, 500.0), Vec2::new(640.0, 150.0));
        cx.frame.rect(r, 14.0, style::PANEL);
        let mut shown: String = self.field.value.clone();
        while shown.len() < 6 {
            shown.push('_');
        }
        let spaced: String = shown.chars().map(|c| format!("{c} ")).collect();
        cx.frame.text(
            spaced.trim_end().to_owned(),
            r.c,
            90.0,
            Vec2::splat(0.5),
            Font::Light,
            style::TEXT,
        );
        status_text(
            cx,
            &cx.shared
                .tr(
                    "PROMPT_JOINROOM_ENTER",
                    "Type the room code and press Enter",
                )
                .to_uppercase(),
            650.0,
            style::GREY,
        );
        if cx.input.nav.back {
            return Action::Pop;
        }
        if cx.input.nav.confirm
            && self.field.value.len() == 6
            && !self.demo
            && let Ok(net) = ensure_net(cx.shared)
        {
            net.borrow_mut().join_code(&self.field.value);
            return Action::Push(Box::new(ConnectingPage::new(format!(
                "JOINING {}",
                self.field.value
            ))));
        }
        Action::None
    }
}

// ------------------------------------------------------------- connecting

pub struct ConnectingPage {
    what: String,
    msg: Option<String>,
    hosted: Option<(String, Option<String>, String)>,
    t: u32,
}

impl ConnectingPage {
    pub fn new(what: String) -> ConnectingPage {
        ConnectingPage {
            what,
            msg: None,
            hosted: None,
            t: 0,
        }
    }
}

impl Page for ConnectingPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        self.t += 1;
        let dots = ".".repeat((self.t / 20 % 4) as usize);
        title(cx, &format!("{}{dots}", self.what), 450.0);
        if let Some(m) = &self.msg {
            status_text(cx, m, 600.0, style::NOPE);
        }
        status_text(
            cx,
            &cx.shared
                .tr("PROMPT_CANCEL_MOUSE", "[ESC] Cancel")
                .to_uppercase(),
            700.0,
            style::GREY,
        );
        let Some(net) = cx.shared.net.clone() else {
            return Action::Pop;
        };
        net.borrow_mut().poll();
        loop {
            let e = net.borrow_mut().next_event();
            let Some(e) = e else { break };
            match e {
                NetEvent::Warpzone(w) => match *w {
                    WzEvent::RoomHosted { room, code, invite } => {
                        println!(
                            "ROOM {} {} {invite}",
                            room.id,
                            code.clone().unwrap_or_else(|| "-".into())
                        );
                        self.hosted = Some((room.id, code, invite));
                    }
                    WzEvent::RoomJoined { room, .. } => {
                        let info = self.hosted.clone().or(Some((room.id, None, String::new())));
                        return Action::Replace(Box::new(LobbyPage::online(net.clone(), info)));
                    }
                    WzEvent::Knocking { .. } => self.what = "WAITING FOR THE HOST".into(),
                    WzEvent::KnockDenied { .. } => self.msg = Some("THE HOST DECLINED".into()),
                    WzEvent::Error { op, message, .. } => {
                        self.msg = Some(format!("{op}: {message}").to_uppercase())
                    }
                    _ => {}
                },
                NetEvent::Rejected { reason } => self.msg = Some(reason.to_uppercase()),
                _ => {}
            }
        }
        if cx.input.nav.back {
            net.borrow_mut().leave();
            return Action::Pop;
        }
        Action::None
    }
}

// ------------------------------------------------------------- demo

/// Pages for menu screenshots without a network.
pub fn demo_pages(sh: &mut Shared, screen: &str) -> Vec<Box<dyn Page>> {
    let mut home = OnlineHomePage::new(false);
    home.demo = true;
    home.user = Some("couchgang".into());
    let mut v: Vec<Box<dyn Page>> = Vec::new();
    match screen {
        "register" => {
            let mut home = OnlineHomePage::new(false);
            home.demo = true;
            v.push(Box::new(home));
            let mut r = RegisterPage::default();
            r.field.value = "couchgang".into();
            v.push(Box::new(r));
        }
        "online" => v.push(Box::new(home)),
        "host" => {
            v.push(Box::new(home));
            v.push(Box::new(HostPage {
                demo: true,
                ..HostPage::default()
            }));
        }
        "browse" => {
            v.push(Box::new(home));
            let mk = |name: &str, players: &str, mode: &str, entry: Entry| RoomInfo {
                id: format!("r-{name}"),
                game: "rounds-port".into(),
                game_version: String::new(),
                name: name.into(),
                host: name.split('\'').next().unwrap_or("").into(),
                players: 1,
                max_players: 12,
                visibility: Visibility::Public,
                entry,
                audience: Default::default(),
                ranked: false,
                staged: false,
                lockfile: None,
                preset: None,
                metadata: [
                    ("players".to_string(), players.to_string()),
                    ("mode".to_string(), mode.to_string()),
                ]
                .into_iter()
                .collect(),
                members: Vec::new(),
                created: 0,
            };
            v.push(Box::new(BrowsePage {
                rooms: vec![
                    mk("ann's free-for-all", "6", "free-for-all", Entry::Open),
                    mk("couchgang's teams", "4", "2 teams", Entry::Approve),
                    mk("bob's free-for-all", "2", "free-for-all", Entry::Open),
                    mk("eve's teams", "9", "3 teams", Entry::Code),
                ],
                demo: true,
                last: Some(Instant::now()),
                ..Default::default()
            }));
        }
        "code" => {
            v.push(Box::new(home));
            let mut p = JoinCodePage {
                demo: true,
                ..Default::default()
            };
            p.field.value = "K7QM".into();
            v.push(Box::new(p));
        }
        _ => {
            v.push(Box::new(home));
            v.push(Box::new(LobbyPage::demo_online(sh)));
        }
    }
    v
}

// ------------------------------------------------------------- automated

/// An automated online run (`--online host|join`, tests).
#[derive(Clone, Debug, Default)]
pub struct AutoPlan {
    pub host: bool,
    pub code: Option<String>,
    /// Join this room id through the launcher path (`--warpzone-join`).
    pub room: Option<String>,
    pub bots: usize,
    /// Host: start when the room has this many players, all ready.
    pub expect_players: usize,
    pub rules: MatchRules,
    pub shots: Vec<u32>,
    pub shot_prefix: Option<PathBuf>,
    /// Leave gracefully / exit without a word at this sim frame.
    pub leave_at: Option<u32>,
    pub crash_at: Option<u32>,
    pub timeout_s: u64,
    pub seed: Option<u64>,
}

pub struct AutoPage {
    plan: AutoPlan,
    started: Instant,
    requested: bool,
    in_room: bool,
    match_started: bool,
    done: Option<i32>,
    last_status: Instant,
}

impl AutoPage {
    pub fn new(plan: AutoPlan) -> AutoPage {
        AutoPage {
            plan,
            started: Instant::now(),
            requested: false,
            in_room: false,
            match_started: false,
            done: None,
            last_status: Instant::now(),
        }
    }
}

fn say(s: String) {
    println!("{s}");
}

impl Page for AutoPage {
    fn tick(&mut self, cx: &mut Cx<'_>) -> Action {
        if let Some(code) = self.done {
            return Action::Quit(code);
        }
        let timeout = Duration::from_secs(self.plan.timeout_s.max(30));
        if self.started.elapsed() > timeout {
            say("ERROR timeout".into());
            return Action::Quit(2);
        }
        title(
            cx,
            if self.plan.host {
                "HOSTING (AUTO)"
            } else {
                "JOINING (AUTO)"
            },
            450.0,
        );
        let net = match ensure_net(cx.shared) {
            Ok(n) => n,
            Err(e) => {
                say(format!("ERROR {e}"));
                return Action::Quit(1);
            }
        };
        net.borrow_mut().poll();
        let logged_in = net
            .borrow_mut()
            .with_warpzone(|c| c.account().is_some())
            .unwrap_or(false);
        loop {
            let e = net.borrow_mut().next_event();
            let Some(e) = e else { break };
            match e {
                NetEvent::Warpzone(w) => match *w {
                    WzEvent::RoomHosted { room, code, invite } => {
                        say(format!(
                            "ROOM {} {} {invite}",
                            room.id,
                            code.unwrap_or_else(|| "-".into())
                        ));
                    }
                    WzEvent::RoomJoined { room, host } => {
                        say(format!("joined room {} (host: {host})", room.id));
                        self.in_room = true;
                        let slots = (0..self.plan.bots)
                            .map(|i| SlotRequest {
                                name: format!("bot {}", i + 1),
                                colour: i as u8,
                                team: (self.plan.rules.teams > 0)
                                    .then(|| i as u8 % self.plan.rules.teams),
                                extra: crate::online::encode_face(&cx.shared.preset(i % 10)),
                            })
                            .collect();
                        let mut n = net.borrow_mut();
                        n.set_local_slots(slots);
                        n.set_ready(true);
                        if n.is_host() {
                            let r = &self.plan.rules;
                            let _ = n.set_settings(MatchSettings {
                                team_mode: if r.teams == 0 {
                                    NetTeams::Ffa
                                } else {
                                    NetTeams::Teams(r.teams)
                                },
                                game: NetRules {
                                    rules: r.clone(),
                                    sandbox: false,
                                }
                                .encode(),
                            });
                            n.set_room_metadata("map", "random");
                        }
                    }
                    WzEvent::PeerConnected { peer, relayed } => say(format!(
                        "peer {peer} connected {}",
                        if relayed { "via relay" } else { "direct" }
                    )),
                    WzEvent::Error { op, code, message } => {
                        say(format!("warpzone error {op}/{code}: {message}"));
                        if op == "join" || op == "host" {
                            return Action::Quit(1);
                        }
                    }
                    _ => {}
                },
                NetEvent::Welcome { machine } => say(format!("welcome as {machine}")),
                NetEvent::MatchStarted(ms) => {
                    say(format!(
                        "match {} started: {} slots, seed {:#x}, local {:?}",
                        ms.match_id,
                        ms.players.len(),
                        ms.seed,
                        net.borrow().local_slots()
                    ));
                    self.match_started = true;
                    let whos = vec![Who::Bot; self.plan.bots];
                    let mut l = online_launch_with(
                        cx.shared,
                        &net,
                        &ms,
                        &whos,
                        Some(Duration::from_secs(10)),
                    );
                    l.bot_rematch = Some(false);
                    l.shots = self.plan.shots.clone();
                    l.shot_prefix = self.plan.shot_prefix.clone();
                    return Action::StartMatch(Box::new(l));
                }
                NetEvent::Rejected { reason } => {
                    say(format!("ERROR rejected: {reason}"));
                    return Action::Quit(1);
                }
                NetEvent::RoomLeft { reason } => {
                    say(format!("ERROR room left: {reason}"));
                    return Action::Quit(1);
                }
                _ => {}
            }
        }
        if logged_in && !self.requested {
            self.requested = true;
            if self.plan.host {
                // Unlisted rooms get a join code.
                net.borrow_mut()
                    .host(host_options(false, 0, self.plan.rules.teams > 0));
            } else if let Some(c) = &self.plan.code {
                net.borrow_mut().join_code(c);
            } else if let Some(r) = &self.plan.room {
                let a = net.borrow_mut().handle_launch(&[
                    "rounds-port".to_string(),
                    "--warpzone-join".into(),
                    r.clone(),
                ]);
                say(format!("launch: {a:?}"));
            }
        }
        if self.plan.host && self.in_room && !self.match_started {
            let ready = {
                let n = net.borrow();
                let lob = n.lobby();
                let players: usize = lob.machines.iter().map(|m| m.slots.len()).sum();
                let all = lob
                    .machines
                    .iter()
                    .filter(|m| !m.slots.is_empty())
                    .all(|m| m.ready);
                players >= self.plan.expect_players && all
            };
            if ready {
                match net.borrow_mut().start_match(self.plan.seed) {
                    Ok(ms) => say(format!("starting match {}", ms.match_id)),
                    Err(e) => say(format!("start: {e}")),
                }
            }
        }
        if self.last_status.elapsed() > Duration::from_secs(5) && !self.match_started {
            self.last_status = Instant::now();
            let n = net.borrow();
            say(format!(
                "lobby: {} machines, {} players",
                n.lobby().machines.len(),
                n.lobby()
                    .machines
                    .iter()
                    .map(|m| m.slots.len())
                    .sum::<usize>()
            ));
        }
        Action::None
    }

    fn match_ended(&mut self, outcome: &MatchOutcome, _shared: &mut Shared) {
        self.done = Some(match outcome {
            MatchOutcome::OnlineEnded { agreed: true, .. } => 0,
            MatchOutcome::OnlineEnded { agreed: false, .. } => 4,
            _ => 1,
        });
    }

    fn left_match(&mut self, _shared: &mut Shared) {
        self.done = Some(0);
    }
}

/// Unused face helper kept for the API symmetry of demos.
#[allow(dead_code)]
fn demo_face(sh: &Shared, i: usize) -> Face {
    sh.preset(i)
}
