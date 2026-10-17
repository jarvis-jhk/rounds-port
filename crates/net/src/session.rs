//! [`NetSession`]: rooms, the host-authoritative lobby, match start, drops
//! (freeze/resume) and end.

use crate::backend::{Backend, MemEndpoint};
use crate::input::{NetInput, Wire};
use crate::lobby;
use crate::matchrun::MatchRun;
use crate::transport::{LinkSim, Transport, TransportStats};
use crate::wire::{
    self, CH_LOBBY, Dechunker, DoneReport, GgrsParams, HOST_MACHINE, LobbyMsg, LobbyState,
    MachineId, MachineView, MatchSettings, MatchStart, PROTOCOL, Phase, SlotRequest,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::{Duration, Instant};
use warpzone_core::client::{Client, Config as WzConfig, Event, HostOptions};
use warpzone_core::net::PeerId;

pub use crate::matchrun::{Request, StateCell};
pub use ggrs::{InputStatus, NetworkStats};

/// Warpzone game id of rounds-port (`packaging/warpzone/rounds-port.toml`).
pub const GAME_ID: &str = "rounds-port";

/// How long the host waits for `Frozen` answers before dropping the silent.
const FREEZE_TIMEOUT: Duration = Duration::from_secs(3);
/// Before a session is synchronised, silence is tolerated this long.
const SYNC_TIMEOUT: Duration = Duration::from_secs(10);

/// The GGRS configuration of a match: inputs `I` (any `Pod`), saved states `S`.
pub struct GgrsConfig<I, S>(PhantomData<(I, S)>);

impl<I: NetInput, S: 'static> ggrs::Config for GgrsConfig<I, S> {
    type Input = Wire<I>;
    type InputPredictor = ggrs::PredictRepeatLast;
    type State = S;
    type Address = MachineId;
}

#[derive(Clone, Debug)]
pub struct NetConfig {
    /// Our version; rooms only accept machines with the same one.
    pub game_version: String,
    /// Warpzone server; `None` = `WARPZONE_SERVER` or the public instance.
    pub server: Option<String>,
    /// Register this username automatically if the identity has none yet.
    pub auto_register: Option<String>,
    /// Route every peer connection through the server relay (testing;
    /// also `ROUNDS_NET_FORCE_RELAY=1`).
    pub force_relay: bool,
    /// Artificial latency/jitter/loss on GGRS datagrams (testing;
    /// `ROUNDS_NET_DELAY_MS`, `ROUNDS_NET_JITTER_MS`, `ROUNDS_NET_LOSS`).
    pub link_sim: LinkSim,
    /// GGRS tuning the host puts into every match it starts.
    pub ggrs: GgrsParams,
}

impl NetConfig {
    pub fn new(game_version: &str) -> NetConfig {
        NetConfig {
            game_version: game_version.into(),
            server: None,
            auto_register: None,
            force_relay: std::env::var("ROUNDS_NET_FORCE_RELAY").is_ok_and(|v| v == "1"),
            link_sim: LinkSim::from_env(),
            ggrs: GgrsParams::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetError {
    Warpzone(String),
    NotHost,
    NotInRoom,
    InMatch,
    NoMatch,
    NoPlayers,
    /// The host needs at least one slot in the match.
    HostNotPlaying,
    NotLocal(usize),
    Ggrs(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NetError {}

/// What `--warpzone-join` / `--warpzone-host` / an invite link asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchAction {
    None,
    /// A join is under way: show "connecting…".
    Joining {
        room: String,
    },
    /// Open the host screen, then call [`NetSession::host`].
    HostScreen,
}

#[derive(Debug, Clone)]
pub enum NetEvent {
    /// Warpzone events the session does not consume: `LoggedIn`,
    /// `Registered`, `RoomHosted` (code, invite), `RoomJoined`, `RoomList`,
    /// `Knock`, `Knocking`, `KnockDenied`, `PeerJoined`, `PeerConnected`,
    /// `PeerPathChanged`, `Error`, ...
    Warpzone(Box<Event>),
    /// The host accepted us as machine `machine`.
    Welcome {
        machine: MachineId,
    },
    Rejected {
        reason: String,
    },
    /// [`NetSession::lobby`] changed.
    LobbyChanged,
    /// Host: a machine entered the room.
    MachineJoined {
        machine: MachineId,
        name: String,
    },
    /// Host: a machine left the room.
    MachineLeft {
        machine: MachineId,
    },
    /// Build the sim from `seed` and `settings` and start calling `advance`.
    MatchStarted(MatchStart),
    /// A match started without us (we joined late or have no slots).
    WaitingForNextMatch {
        match_id: u32,
    },
    Synchronizing {
        machine: MachineId,
        count: u32,
        total: u32,
    },
    Synchronized {
        machine: MachineId,
    },
    /// Synchronised with everyone: frames advance from now on.
    Running,
    NetworkInterrupted {
        machine: MachineId,
        disconnect_timeout_ms: u64,
    },
    NetworkResumed {
        machine: MachineId,
    },
    /// GGRS-level disconnect (not expected: drops go through freeze/resume).
    Disconnected {
        machine: MachineId,
    },
    /// Machines dropped out; the match is paused while everybody agrees on
    /// the frame to continue from (`advance` returns nothing meanwhile).
    MatchFrozen {
        gone: Vec<MachineId>,
    },
    /// The match continues from `from_frame` (a `Load` request for it comes
    /// with the next `advance`); `slots` get `Disconnected` zero inputs
    /// from then on.
    PlayersDropped {
        machines: Vec<MachineId>,
        slots: Vec<usize>,
        from_frame: i32,
    },
    Desync {
        frame: i32,
        local: u128,
        remote: u128,
        machine: MachineId,
    },
    /// We are ahead of the others; GGRS suggests skipping `frames` frames.
    WaitRecommendation {
        frames: u32,
    },
    /// Every playing machine reported `finish_match`; `agreed` = identical
    /// frames and checksums.
    MatchEnded {
        match_id: u32,
        reports: Vec<DoneReport>,
        agreed: bool,
    },
    /// The match cannot continue (host gone, we were dropped, we left).
    MatchAborted {
        reason: String,
    },
    /// We are no longer in a room.
    RoomLeft {
        reason: String,
    },
    /// [`NetSession::send_app`] data from another machine.
    App {
        from: MachineId,
        data: Vec<u8>,
    },
}

/// One remote machine's connection.
#[derive(Clone, Debug)]
pub struct MachineStats {
    pub machine: MachineId,
    pub name: String,
    /// GGRS's end-to-end view (ping, kbps, frames behind) while in a match.
    pub ggrs: Option<NetworkStats>,
    /// RTT of our Warpzone link to it (guests: the link to the host).
    pub link_rtt_ms: Option<u32>,
    /// That link goes through the Warpzone relay instead of directly.
    pub relayed: Option<bool>,
    /// Packets to it are forwarded by the room host.
    pub via_host: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NetStats {
    pub current_frame: i32,
    pub confirmed_frame: i32,
    pub frames_ahead: i32,
    /// Load requests (each one is a rollback).
    pub rollbacks: u64,
    /// Frames simulated again after a rollback.
    pub resimulated_frames: u64,
    pub max_rollback_depth: u32,
    /// advance() calls that could not advance (prediction limit reached).
    pub prediction_stalls: u64,
    pub desyncs: u64,
    /// Freeze/resume cycles (machines dropped mid-match).
    pub resumes: u32,
    pub transport: TransportStats,
    pub machines: Vec<MachineStats>,
}

struct HostMachine {
    peer: Option<PeerId>,
    view: MachineView,
    hello: bool,
}

struct HostFreeze {
    epoch: u8,
    gone: BTreeSet<MachineId>,
    reports: BTreeMap<MachineId, i32>,
    since: Instant,
}

struct HostLobby {
    next_machine: u8,
    machines: BTreeMap<MachineId, HostMachine>,
    next_match: u32,
    done: BTreeMap<MachineId, DoneReport>,
    metadata: BTreeMap<String, String>,
    freeze: Option<HostFreeze>,
    /// Silence before this instant does not count (match start, resume).
    watch_since: Instant,
}

enum Role {
    None,
    Host(HostLobby),
    Guest { welcomed: bool },
}

/// One machine's online state: room membership, lobby, and the running match.
///
/// Single-threaded: call everything from the game thread. Call [`poll`]
/// every frame, drain [`next_event`], and while a match runs call
/// [`add_local_input`] for the local slots and [`advance`] once per sim step.
///
/// [`poll`]: NetSession::poll
/// [`next_event`]: NetSession::next_event
/// [`add_local_input`]: NetSession::add_local_input
/// [`advance`]: NetSession::advance
pub struct NetSession<I: NetInput, S: 'static> {
    cfg: NetConfig,
    t: Rc<RefCell<Transport>>,
    events: VecDeque<NetEvent>,
    role: Role,
    dechunk: BTreeMap<PeerId, Dechunker>,
    my_slots: Vec<SlotRequest>,
    ready: bool,
    lobby: LobbyState,
    settings: MatchSettings,
    current: Option<MatchRun<I, S>>,
    stats: NetStats,
    last_match_stats: Option<NetStats>,
    /// Game-provided room metadata (e.g. `map`), merged into what the host
    /// publishes (`players`, `mode`).
    extra_metadata: BTreeMap<String, String>,
}

impl<I: NetInput, S: 'static> NetSession<I, S> {
    /// Connect to Warpzone (identity from `WARPZONE_HOME` or the shared one).
    pub fn new(cfg: NetConfig) -> Result<Self, NetError> {
        let mut wc = WzConfig::new(GAME_ID, &cfg.game_version);
        if let Some(s) = &cfg.server {
            wc.server = s.clone();
        }
        let client = Client::new(wc).map_err(|e| NetError::Warpzone(e.to_string()))?;
        Ok(Self::with_backend(Backend::Warpzone(Box::new(client)), cfg))
    }

    /// An in-process room member (tests, local multi-session runs).
    pub fn memory(endpoint: MemEndpoint, cfg: NetConfig) -> Self {
        Self::with_backend(Backend::Memory(endpoint), cfg)
    }

    fn with_backend(b: Backend, cfg: NetConfig) -> Self {
        let sim = cfg.link_sim;
        NetSession {
            cfg,
            t: Rc::new(RefCell::new(Transport::new(b, sim))),
            events: VecDeque::new(),
            role: Role::None,
            dechunk: BTreeMap::new(),
            my_slots: Vec::new(),
            ready: false,
            lobby: LobbyState::default(),
            settings: MatchSettings::default(),
            current: None,
            stats: NetStats::default(),
            last_match_stats: None,
            extra_metadata: BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------ warpzone

    fn with_client<R>(&mut self, f: impl FnOnce(&mut Client) -> R) -> Option<R> {
        match &mut self.t.borrow_mut().backend {
            Backend::Warpzone(c) => Some(f(c)),
            Backend::Memory(_) => None,
        }
    }

    /// The Warpzone client, for anything not wrapped here (ladder, mods).
    pub fn with_warpzone<R>(&mut self, f: impl FnOnce(&mut Client) -> R) -> Option<R> {
        self.with_client(f)
    }

    /// Act on `--warpzone-join <room>`, invite links, `WARPZONE_JOIN`,
    /// `--warpzone-host` / `WARPZONE_HOST=1`.
    pub fn handle_launch<A: AsRef<str>>(&mut self, args: &[A]) -> LaunchAction {
        let intent = warpzone_core::launch::from_args(args);
        if let Some(room) = intent.join {
            let secret = intent.secret.clone();
            self.apply_relay();
            self.with_client(|c| c.join(&room, None, secret.as_deref()));
            return LaunchAction::Joining { room };
        }
        if intent.host {
            LaunchAction::HostScreen
        } else {
            LaunchAction::None
        }
    }

    pub fn register(&mut self, username: &str) {
        self.with_client(|c| c.register(username));
    }

    /// Host a room. `max_players` counts machines (Warpzone members), 2..=12.
    pub fn host(&mut self, mut opts: HostOptions) {
        self.apply_relay();
        if opts.max_players == 0 {
            opts.max_players = 12;
        }
        self.with_client(|c| c.host(opts));
    }

    pub fn join(&mut self, room: &str, code: Option<&str>, secret: Option<&str>) {
        self.apply_relay();
        self.with_client(|c| c.join(room, code, secret));
    }

    pub fn join_code(&mut self, code: &str) {
        self.apply_relay();
        self.with_client(|c| c.join_code(code));
    }

    /// Result: `NetEvent::Warpzone(Event::RoomList(..))`.
    pub fn list_rooms(&mut self, has_space: bool) {
        self.with_client(|c| c.list_rooms(has_space));
    }

    pub fn answer_knock(&mut self, knock: &str, accept: bool) {
        self.with_client(|c| c.answer_knock(knock, accept));
    }

    /// Host: remove a machine from the room.
    pub fn kick(&mut self, machine: MachineId, ban: bool) {
        let acc = match &self.role {
            Role::Host(h) => h.machines.get(&machine).map(|m| m.view.account),
            _ => None,
        };
        if let Some(a) = acc {
            self.with_client(|c| c.kick(a, ban));
        }
    }

    /// Host: stop (or allow) new joiners in the room browser.
    pub fn set_room_closed(&mut self, closed: bool) {
        self.with_client(|c| c.set_closed(closed));
    }

    /// Leave the room (as host: close it).
    pub fn leave(&mut self) {
        {
            let mut t = self.t.borrow_mut();
            match &mut t.backend {
                Backend::Warpzone(c) => c.leave(),
                Backend::Memory(m) => m.leave(),
            }
        }
        self.left_room("left");
    }

    fn apply_relay(&mut self) {
        if self.cfg.force_relay {
            self.with_client(|c| c.force_relay(true));
        }
    }

    // ------------------------------------------------------------ lobby API

    pub fn is_host(&self) -> bool {
        matches!(self.role, Role::Host(_))
    }

    pub fn in_room(&self) -> bool {
        !matches!(self.role, Role::None)
    }

    /// Our machine id once welcomed (the host is always machine 0).
    pub fn me(&self) -> Option<MachineId> {
        self.t.borrow().me
    }

    pub fn lobby(&self) -> &LobbyState {
        &self.lobby
    }

    /// Announce this machine's local players (0..=12; 0 = not playing).
    pub fn set_local_slots(&mut self, slots: Vec<SlotRequest>) {
        self.my_slots = lobby::sanitize_slots(slots);
        match &mut self.role {
            Role::Host(h) => {
                if let Some(m) = h.machines.get_mut(&HOST_MACHINE) {
                    m.view.slots = self.my_slots.clone();
                }
                self.host_broadcast_state();
            }
            Role::Guest { .. } => self.send_host(&LobbyMsg::SetSlots(self.my_slots.clone())),
            Role::None => {}
        }
    }

    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        match &mut self.role {
            Role::Host(h) => {
                if let Some(m) = h.machines.get_mut(&HOST_MACHINE) {
                    m.view.ready = ready;
                }
                self.host_broadcast_state();
            }
            Role::Guest { .. } => self.send_host(&LobbyMsg::SetReady(ready)),
            Role::None => {}
        }
    }

    /// Host (or before hosting): team mode and the game's own settings for
    /// the next match.
    pub fn set_settings(&mut self, settings: MatchSettings) -> Result<(), NetError> {
        if matches!(self.role, Role::Guest { .. }) {
            return Err(NetError::NotHost);
        }
        self.settings = settings;
        self.host_broadcast_state();
        Ok(())
    }

    /// Host: publish `key = value` in the room's metadata (room browser
    /// columns, see `packaging/warpzone/rounds-port.toml`); empty removes.
    pub fn set_room_metadata(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            self.extra_metadata.remove(key);
        } else {
            self.extra_metadata.insert(key.to_owned(), value.to_owned());
        }
        if self.is_host() {
            self.host_broadcast_state();
        }
    }

    /// Host: override the team request of one machine's local slot.
    pub fn set_team(
        &mut self,
        machine: MachineId,
        local_index: usize,
        team: Option<u8>,
    ) -> Result<(), NetError> {
        let Role::Host(h) = &mut self.role else {
            return Err(NetError::NotHost);
        };
        if let Some(s) = h
            .machines
            .get_mut(&machine)
            .and_then(|m| m.view.slots.get_mut(local_index))
        {
            s.team = team;
        }
        self.host_broadcast_state();
        Ok(())
    }

    /// Host: start a match with every machine in the room that announced
    /// slots. `seed` = `None` picks one.
    pub fn start_match(&mut self, seed: Option<u64>) -> Result<MatchStart, NetError> {
        if self.current.is_some() {
            return Err(NetError::InMatch);
        }
        let Role::Host(h) = &mut self.role else {
            return Err(NetError::NotHost);
        };
        let playing: Vec<MachineView> = h
            .machines
            .values()
            .filter(|m| m.hello && !m.view.slots.is_empty())
            .map(|m| m.view.clone())
            .collect();
        let players = lobby::assign(&playing, self.settings.team_mode);
        if players.is_empty() {
            return Err(NetError::NoPlayers);
        }
        // The host coordinates drops and plays the dropped slots' dummies.
        if !players.iter().any(|p| p.machine == HOST_MACHINE) {
            return Err(NetError::HostNotPlaying);
        }
        h.next_match += 1;
        h.done.clear();
        h.freeze = None;
        h.watch_since = Instant::now();
        for m in h.machines.values_mut() {
            m.view.waiting = false;
        }
        let ms = MatchStart {
            match_id: h.next_match,
            seed: seed.unwrap_or_else(random_seed),
            settings: self.settings.clone(),
            players,
            ggrs: self.cfg.ggrs,
        };
        let peers: Vec<PeerId> = ms
            .machines()
            .iter()
            .filter_map(|m| h.machines.get(m).and_then(|x| x.peer))
            .collect();
        for p in peers {
            self.send_to(p, &LobbyMsg::Start(ms.clone()));
        }
        self.begin_match(ms.clone())?;
        self.host_broadcast_state();
        Ok(ms)
    }

    /// Host: end the running match now (e.g. abort); everyone drops it.
    pub fn end_match(&mut self) -> Result<(), NetError> {
        let Role::Host(h) = &mut self.role else {
            return Err(NetError::NotHost);
        };
        let Some(m) = &self.current else {
            return Err(NetError::NoMatch);
        };
        let reports: Vec<DoneReport> = h.done.values().copied().collect();
        let id = m.info.match_id;
        self.host_end(id, reports);
        Ok(())
    }

    /// Send game data (chat, menus) to one machine or everyone, reliably,
    /// through the host. Never use it for anything the sim depends on.
    pub fn send_app(&mut self, to: Option<MachineId>, data: Vec<u8>) {
        let Some(me) = self.me() else { return };
        let msg = LobbyMsg::App { from: me, to, data };
        if self.is_host() {
            self.host_route_app(msg);
        } else {
            self.send_host(&msg);
        }
    }

    // ------------------------------------------------------------ match API

    pub fn current_match(&self) -> Option<&MatchStart> {
        self.current.as_ref().map(|m| &m.info)
    }

    /// Global slots this machine plays (empty outside a match).
    pub fn local_slots(&self) -> &[usize] {
        self.current
            .as_ref()
            .map(|m| m.local.as_slice())
            .unwrap_or(&[])
    }

    /// Synchronised and not frozen: `advance` produces frames.
    pub fn is_running(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|m| m.running && m.frozen.is_none())
    }

    /// Input of a local slot for the next `advance` (missing ones are zero).
    pub fn add_local_input(&mut self, slot: usize, input: I) -> Result<(), NetError> {
        self.current
            .as_mut()
            .ok_or(NetError::NoMatch)?
            .add_input(slot, input)
    }

    /// One sim step. Fulfil the requests in order. Empty while synchronising,
    /// frozen, or at the prediction limit (just call again next step).
    pub fn advance(&mut self) -> Vec<Request<I, S>> {
        let Some(m) = self.current.as_mut() else {
            return Vec::new();
        };
        let out = m.advance(&mut self.stats);
        self.drain_ggrs();
        out
    }

    /// Report the end of the match: the sim reached its end at a *confirmed*
    /// `frame` (≤ [`confirmed_frame`](Self::confirmed_frame)) with this
    /// checksum. Keep calling `advance` until `MatchEnded`: the others may
    /// still need our inputs up to that frame.
    pub fn finish_match(&mut self, frame: i32, checksum: u128) -> Result<(), NetError> {
        let m = self.current.as_mut().ok_or(NetError::NoMatch)?;
        if m.finished {
            return Ok(());
        }
        m.finished = true;
        let match_id = m.info.match_id;
        let me = self.t.borrow().me.unwrap_or_default();
        match &mut self.role {
            Role::Host(h) => {
                h.done.insert(
                    me,
                    DoneReport {
                        machine: me,
                        frame,
                        checksum,
                    },
                );
                self.host_check_end();
            }
            _ => self.send_host(&LobbyMsg::Done {
                match_id,
                frame,
                checksum,
            }),
        }
        Ok(())
    }

    /// Sim frame the next `advance` simulates (counts across resumes).
    pub fn current_frame(&self) -> i32 {
        self.current
            .as_ref()
            .map(|m| m.current_frame())
            .unwrap_or(-1)
    }

    /// Newest frame for which every input is known.
    pub fn confirmed_frame(&self) -> i32 {
        self.current
            .as_ref()
            .map(|m| m.confirmed_frame())
            .unwrap_or(-1)
    }

    /// Frames we are ahead of the slowest machine: slow down a little when > 0.
    pub fn frames_ahead(&self) -> i32 {
        self.current.as_ref().map(|m| m.frames_ahead()).unwrap_or(0)
    }

    pub fn stats(&self) -> NetStats {
        let mut s = self.stats.clone();
        let t = self.t.borrow();
        s.transport = t.stats;
        if let Some(m) = &self.current {
            s.current_frame = m.current_frame();
            s.confirmed_frame = m.confirmed_frame();
            s.frames_ahead = m.frames_ahead();
        }
        let me = t.me;
        for mv in &self.lobby.machines {
            if Some(mv.machine) == me {
                continue;
            }
            let peer = if t.is_host {
                t.machine_peer.get(&mv.machine).copied()
            } else {
                t.host_peer
            };
            s.machines.push(MachineStats {
                machine: mv.machine,
                name: mv.name.clone(),
                ggrs: self
                    .current
                    .as_ref()
                    .and_then(|m| m.network_stats(mv.machine)),
                link_rtt_ms: peer
                    .and_then(|p| t.backend.peer_rtt(p))
                    .map(|d| d.as_millis() as u32),
                relayed: peer.and_then(|p| t.backend.peer_relayed(p)),
                via_host: !t.is_host && mv.machine != HOST_MACHINE,
            });
        }
        s
    }

    /// [`stats`](Self::stats) as they were when the last match ended.
    pub fn last_match_stats(&self) -> Option<&NetStats> {
        self.last_match_stats.as_ref()
    }

    pub fn next_event(&mut self) -> Option<NetEvent> {
        self.events.pop_front()
    }

    // ------------------------------------------------------------ poll

    /// Network work: Warpzone, forwarding, lobby messages, GGRS. Every frame.
    pub fn poll(&mut self) {
        self.t.borrow_mut().pump();
        loop {
            let e = self.t.borrow_mut().pending.pop_front();
            let Some(e) = e else { break };
            self.on_event(e);
        }
        if let Some(m) = self.current.as_mut() {
            m.poll();
        }
        self.drain_ggrs();
        self.host_watch();
    }

    fn emit(&mut self, e: NetEvent) {
        self.events.push_back(e);
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::LoggedIn { ref account, .. } => {
                if account.is_none()
                    && let Some(n) = self.cfg.auto_register.clone()
                {
                    self.register(&n);
                }
                self.apply_relay();
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::RoomJoined { host, .. } => {
                self.enter_room(host);
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::PeerJoined {
                peer,
                account,
                ref username,
            } => {
                let name = username.clone();
                self.on_peer_joined(peer, account, name);
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::PeerLeft { peer, .. } => self.on_peer_left(peer),
            Event::PeerData {
                peer,
                channel: CH_LOBBY,
                data,
                reliable: true,
            } => {
                let whole = self.dechunk.entry(peer).or_default().push(&data);
                if let Some(msg) = whole.and_then(|b| wire::decode::<LobbyMsg>(&b)) {
                    self.on_lobby(peer, msg);
                }
            }
            Event::PeerData { .. } => {}
            Event::RoomClosed { .. } => self.left_room("room closed"),
            Event::Kicked { banned, .. } => {
                self.left_room(if banned { "banned" } else { "kicked" })
            }
            e => self.emit(NetEvent::Warpzone(Box::new(e))),
        }
    }

    fn enter_room(&mut self, host: bool) {
        self.drop_match(None);
        self.dechunk.clear();
        let name = {
            let mut t = self.t.borrow_mut();
            t.reset_room();
            t.is_host = host;
            if host {
                t.me = Some(HOST_MACHINE);
            }
            t.backend.username().unwrap_or_else(|| "host".into())
        };
        if host {
            let account = self.t.borrow().backend.account_id();
            let mut machines = BTreeMap::new();
            let view = MachineView {
                machine: HOST_MACHINE,
                account,
                name,
                ready: self.ready,
                waiting: false,
                slots: self.my_slots.clone(),
            };
            machines.insert(
                HOST_MACHINE,
                HostMachine {
                    peer: None,
                    hello: true,
                    view,
                },
            );
            self.role = Role::Host(HostLobby {
                next_machine: 1,
                machines,
                next_match: 0,
                done: BTreeMap::new(),
                metadata: BTreeMap::new(),
                freeze: None,
                watch_since: Instant::now(),
            });
            self.emit(NetEvent::Welcome {
                machine: HOST_MACHINE,
            });
            self.host_broadcast_state();
        } else {
            self.role = Role::Guest { welcomed: false };
            self.lobby = LobbyState::default();
        }
    }

    fn left_room(&mut self, reason: &str) {
        if matches!(self.role, Role::None) {
            return;
        }
        self.drop_match(Some(reason));
        self.role = Role::None;
        self.lobby = LobbyState::default();
        self.t.borrow_mut().reset_room();
        self.emit(NetEvent::RoomLeft {
            reason: reason.into(),
        });
    }

    fn on_peer_joined(&mut self, peer: PeerId, account: i64, name: String) {
        match &mut self.role {
            Role::Host(h) => {
                if h.next_machine == u8::MAX {
                    return;
                }
                let id = MachineId(h.next_machine);
                h.next_machine += 1;
                let waiting = self.current.is_some();
                let view = MachineView {
                    machine: id,
                    account,
                    name: name.clone(),
                    ready: false,
                    waiting,
                    slots: Vec::new(),
                };
                h.machines.insert(
                    id,
                    HostMachine {
                        peer: Some(peer),
                        hello: false,
                        view,
                    },
                );
                let mut t = self.t.borrow_mut();
                t.peer_machine.insert(peer, id);
                t.machine_peer.insert(id, peer);
                drop(t);
                self.emit(NetEvent::MachineJoined { machine: id, name });
            }
            Role::Guest { .. } => {
                self.t.borrow_mut().host_peer = Some(peer);
                let hello = LobbyMsg::Hello {
                    protocol: PROTOCOL,
                    game_version: self.cfg.game_version.clone(),
                    slots: self.my_slots.clone(),
                };
                self.send_to(peer, &hello);
                if self.ready {
                    self.send_to(peer, &LobbyMsg::SetReady(true));
                }
            }
            Role::None => {}
        }
    }

    fn on_peer_left(&mut self, peer: PeerId) {
        match &mut self.role {
            Role::Host(h) => {
                let Some(id) = self.t.borrow().peer_machine.get(&peer).copied() else {
                    return;
                };
                h.machines.remove(&id);
                h.done.remove(&id);
                {
                    let mut t = self.t.borrow_mut();
                    t.peer_machine.remove(&peer);
                    t.machine_peer.remove(&id);
                }
                self.dechunk.remove(&peer);
                self.emit(NetEvent::MachineLeft { machine: id });
                self.host_drop(vec![id]);
                self.host_check_end();
                self.host_broadcast_state();
            }
            Role::Guest { .. } => {
                if self.t.borrow().host_peer == Some(peer) {
                    self.left_room("host left");
                }
            }
            Role::None => {}
        }
    }

    fn on_lobby(&mut self, peer: PeerId, msg: LobbyMsg) {
        if self.is_host() {
            self.host_on_lobby(peer, msg);
        } else if self.t.borrow().host_peer == Some(peer) {
            self.guest_on_lobby(msg);
        }
    }

    fn host_on_lobby(&mut self, peer: PeerId, msg: LobbyMsg) {
        let Some(id) = self.t.borrow().peer_machine.get(&peer).copied() else {
            return;
        };
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(m) = h.machines.get_mut(&id) else {
            return;
        };
        let this_match = |c: &Option<MatchRun<I, S>>, mid: u32| {
            c.as_ref().is_some_and(|c| c.info.match_id == mid)
        };
        match msg {
            LobbyMsg::Hello {
                protocol,
                game_version,
                slots,
            } => {
                if protocol != PROTOCOL || game_version != self.cfg.game_version {
                    let acc = m.view.account;
                    let reason = format!(
                        "version mismatch: host runs {} (protocol {PROTOCOL})",
                        self.cfg.game_version
                    );
                    self.send_to(peer, &LobbyMsg::Reject { reason });
                    self.with_client(|c| c.kick(acc, false));
                    return;
                }
                m.hello = true;
                m.view.slots = lobby::sanitize_slots(slots);
                self.send_to(peer, &LobbyMsg::Welcome { machine: id });
                self.host_broadcast_state();
            }
            LobbyMsg::SetSlots(slots) => {
                m.view.slots = lobby::sanitize_slots(slots);
                self.host_broadcast_state();
            }
            LobbyMsg::SetReady(r) => {
                m.view.ready = r;
                self.host_broadcast_state();
            }
            LobbyMsg::Done {
                match_id,
                frame,
                checksum,
            } => {
                if this_match(&self.current, match_id) {
                    h.done.insert(
                        id,
                        DoneReport {
                            machine: id,
                            frame,
                            checksum,
                        },
                    );
                    self.host_check_end();
                }
            }
            LobbyMsg::Frozen {
                match_id,
                epoch,
                frame,
            } => {
                if this_match(&self.current, match_id)
                    && let Some(f) = h.freeze.as_mut()
                    && f.epoch == epoch
                {
                    f.reports.insert(id, frame);
                    self.host_check_freeze();
                }
            }
            LobbyMsg::App { to, data, .. } => {
                self.host_route_app(LobbyMsg::App { from: id, to, data })
            }
            _ => {}
        }
    }

    fn host_route_app(&mut self, msg: LobbyMsg) {
        let LobbyMsg::App { from, to, ref data } = msg else {
            return;
        };
        if (to.is_none() || to == Some(HOST_MACHINE)) && from != HOST_MACHINE {
            self.emit(NetEvent::App {
                from,
                data: data.clone(),
            });
        }
        match to {
            None => self.host_broadcast(&msg, Some(from)),
            Some(HOST_MACHINE) => {}
            Some(x) => {
                let p = self.t.borrow().machine_peer.get(&x).copied();
                if let Some(p) = p {
                    self.send_to(p, &msg);
                }
            }
        }
    }

    fn guest_on_lobby(&mut self, msg: LobbyMsg) {
        let this_match = |c: &Option<MatchRun<I, S>>, mid: u32| {
            c.as_ref().is_some_and(|c| c.info.match_id == mid)
        };
        match msg {
            LobbyMsg::Welcome { machine } => {
                self.t.borrow_mut().me = Some(machine);
                self.role = Role::Guest { welcomed: true };
                self.emit(NetEvent::Welcome { machine });
            }
            LobbyMsg::Reject { reason } => self.emit(NetEvent::Rejected { reason }),
            LobbyMsg::State(s) => {
                self.lobby = s;
                self.emit(NetEvent::LobbyChanged);
            }
            LobbyMsg::Start(ms) => {
                if !matches!(self.role, Role::Guest { welcomed: true }) {
                    return;
                }
                if let Err(e) = self.begin_match(ms) {
                    log::warn!("cannot start match: {e}");
                    self.emit(NetEvent::MatchAborted {
                        reason: e.to_string(),
                    });
                }
            }
            LobbyMsg::Freeze {
                match_id,
                epoch,
                gone,
            } if this_match(&self.current, match_id) => {
                let me = self.me().unwrap_or_default();
                if gone.contains(&me) {
                    self.drop_match(Some("dropped from the match"));
                    return;
                }
                let frame = self.current.as_mut().map(|m| m.freeze(epoch)).unwrap_or(-1);
                self.send_host(&LobbyMsg::Frozen {
                    match_id,
                    epoch,
                    frame,
                });
                self.emit(NetEvent::MatchFrozen { gone });
            }
            LobbyMsg::Resume {
                match_id,
                epoch,
                frame,
                gone,
            } if this_match(&self.current, match_id) => {
                self.resume_local(epoch, frame, gone.into_iter().collect());
            }
            LobbyMsg::Dropped { match_id } if this_match(&self.current, match_id) => {
                self.drop_match(Some("dropped from the match"))
            }
            LobbyMsg::End { match_id, reports } if this_match(&self.current, match_id) => {
                self.finish_locally(match_id, reports)
            }
            LobbyMsg::App { from, data, .. } => self.emit(NetEvent::App { from, data }),
            _ => {}
        }
    }

    fn begin_match(&mut self, ms: MatchStart) -> Result<(), NetError> {
        let me = self.t.borrow().me.ok_or(NetError::NotInRoom)?;
        self.drop_match(Some("new match"));
        if ms.slots_of(me).is_empty() {
            self.emit(NetEvent::WaitingForNextMatch {
                match_id: ms.match_id,
            });
            return Ok(());
        }
        let run = MatchRun::new(ms.clone(), me, &self.t)?;
        self.current = Some(run);
        self.stats = NetStats::default();
        self.emit(NetEvent::MatchStarted(ms));
        Ok(())
    }

    fn resume_local(&mut self, epoch: u8, frame: i32, gone: BTreeSet<MachineId>) {
        let Some(m) = self.current.as_mut() else {
            return;
        };
        let newly: Vec<MachineId> = gone.difference(&m.gone).copied().collect();
        let slots: Vec<usize> = newly.iter().flat_map(|x| m.info.slots_of(*x)).collect();
        match m.resume(epoch, frame, gone, &self.t) {
            Ok(()) => {
                self.stats.resumes += 1;
                self.emit(NetEvent::PlayersDropped {
                    machines: newly,
                    slots,
                    from_frame: frame,
                });
            }
            Err(e) => {
                log::warn!("resume: {e}");
                self.drop_match(Some(&format!("cannot resume: {e}")));
            }
        }
    }

    fn drop_match(&mut self, reason: Option<&str>) {
        if self.current.take().is_some()
            && let Some(r) = reason
        {
            self.emit(NetEvent::MatchAborted { reason: r.into() });
        }
    }

    fn drain_ggrs(&mut self) {
        let Some(m) = self.current.as_mut() else {
            return;
        };
        let evs = m.events(&mut self.stats);
        self.events.extend(evs);
    }

    // ------------------------------------------------------------ host: drops

    /// Host: drop machines that went silent; time out freezes.
    fn host_watch(&mut self) {
        let Role::Host(h) = &self.role else { return };
        let Some(m) = &self.current else { return };
        if let Some(f) = &h.freeze {
            if f.since.elapsed() > FREEZE_TIMEOUT {
                let missing: Vec<MachineId> = m
                    .info
                    .machines()
                    .into_iter()
                    .filter(|x| {
                        *x != HOST_MACHINE && !f.gone.contains(x) && !f.reports.contains_key(x)
                    })
                    .collect();
                self.host_drop(missing);
            }
            return;
        }
        let limit = if m.running {
            Duration::from_millis(m.info.ggrs.disconnect_timeout_ms as u64)
        } else {
            SYNC_TIMEOUT
        };
        let t = self.t.borrow();
        let silent: Vec<MachineId> = m
            .info
            .machines()
            .into_iter()
            .filter(|x| *x != HOST_MACHINE && !m.gone.contains(x))
            .filter(|x| {
                let last = t
                    .last_recv
                    .get(x)
                    .copied()
                    .map_or(h.watch_since, |l| l.max(h.watch_since));
                last.elapsed() > limit
            })
            .collect();
        drop(t);
        if !silent.is_empty() {
            self.host_drop(silent);
        }
    }

    /// Host: drop `new` machines from the running match (freeze, collect
    /// restart frames, resume).
    fn host_drop(&mut self, new: Vec<MachineId>) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(m) = self.current.as_mut() else {
            return;
        };
        let mut gone: BTreeSet<MachineId> = h
            .freeze
            .as_ref()
            .map(|f| f.gone.clone())
            .unwrap_or_else(|| m.gone.clone());
        let playing = m.info.machines();
        let added: Vec<MachineId> = new
            .into_iter()
            .filter(|x| playing.contains(x) && *x != HOST_MACHINE && gone.insert(*x))
            .collect();
        if added.is_empty() {
            return;
        }
        let epoch = h
            .freeze
            .as_ref()
            .map_or(m.epoch, |f| f.epoch)
            .wrapping_add(1);
        let match_id = m.info.match_id;
        let mut reports = BTreeMap::new();
        reports.insert(HOST_MACHINE, m.freeze(epoch));
        h.freeze = Some(HostFreeze {
            epoch,
            gone: gone.clone(),
            reports,
            since: Instant::now(),
        });
        let gone_v: Vec<MachineId> = gone.iter().copied().collect();
        let t = self.t.borrow();
        let tell: Vec<(PeerId, bool)> = playing
            .iter()
            .filter(|x| **x != HOST_MACHINE)
            .filter_map(|x| t.machine_peer.get(x).map(|p| (*p, gone.contains(x))))
            .collect();
        drop(t);
        for (p, dropped) in tell {
            let msg = if dropped {
                LobbyMsg::Dropped { match_id }
            } else {
                LobbyMsg::Freeze {
                    match_id,
                    epoch,
                    gone: gone_v.clone(),
                }
            };
            self.send_to(p, &msg);
        }
        self.emit(NetEvent::MatchFrozen { gone: gone_v });
        self.host_check_freeze();
    }

    fn host_check_freeze(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(m) = &self.current else { return };
        let Some(f) = &h.freeze else { return };
        let remaining: Vec<MachineId> = m
            .info
            .machines()
            .into_iter()
            .filter(|x| !f.gone.contains(x))
            .collect();
        if !remaining.iter().all(|x| f.reports.contains_key(x)) {
            return;
        }
        let d = remaining
            .iter()
            .filter_map(|x| f.reports.get(x))
            .copied()
            .min()
            .unwrap_or(-1);
        let frame = d + 1;
        let (epoch, gone) = (f.epoch, f.gone.clone());
        h.freeze = None;
        h.watch_since = Instant::now();
        let match_id = m.info.match_id;
        let msg = LobbyMsg::Resume {
            match_id,
            epoch,
            frame,
            gone: gone.iter().copied().collect(),
        };
        let t = self.t.borrow();
        let peers: Vec<PeerId> = remaining
            .iter()
            .filter_map(|x| t.machine_peer.get(x).copied())
            .collect();
        drop(t);
        for p in peers {
            self.send_to(p, &msg);
        }
        self.resume_local(epoch, frame, gone);
        self.host_check_end();
    }

    fn host_check_end(&mut self) {
        let Role::Host(h) = &self.role else { return };
        let Some(m) = &self.current else { return };
        let gone = h.freeze.as_ref().map_or(&m.gone, |f| &f.gone);
        let all = m
            .info
            .machines()
            .iter()
            .filter(|x| h.machines.contains_key(x) && !gone.contains(x))
            .all(|x| h.done.contains_key(x));
        if all {
            let reports: Vec<DoneReport> = h.done.values().copied().collect();
            let id = m.info.match_id;
            self.host_end(id, reports);
        }
    }

    fn host_end(&mut self, match_id: u32, reports: Vec<DoneReport>) {
        let msg = LobbyMsg::End {
            match_id,
            reports: reports.clone(),
        };
        self.host_broadcast(&msg, None);
        if let Role::Host(h) = &mut self.role {
            h.freeze = None;
        }
        self.finish_locally(match_id, reports);
        self.host_broadcast_state();
    }

    fn finish_locally(&mut self, match_id: u32, reports: Vec<DoneReport>) {
        self.last_match_stats = Some(self.stats());
        self.current = None;
        let agreed = reports
            .windows(2)
            .all(|w| w[0].checksum == w[1].checksum && w[0].frame == w[1].frame);
        self.emit(NetEvent::MatchEnded {
            match_id,
            reports,
            agreed,
        });
    }

    // ------------------------------------------------------------ sending

    fn send_to(&mut self, peer: PeerId, msg: &LobbyMsg) {
        let bytes = wire::encode(msg);
        let mut t = self.t.borrow_mut();
        for c in wire::chunk(&bytes) {
            t.backend.send(peer, CH_LOBBY, &c, true);
        }
    }

    fn send_host(&mut self, msg: &LobbyMsg) {
        let p = self.t.borrow().host_peer;
        if let Some(p) = p {
            self.send_to(p, msg);
        }
    }

    fn host_broadcast(&mut self, msg: &LobbyMsg, except: Option<MachineId>) {
        let peers: Vec<PeerId> = self
            .t
            .borrow()
            .machine_peer
            .iter()
            .filter(|(m, _)| Some(**m) != except)
            .map(|(_, p)| *p)
            .collect();
        for p in peers {
            self.send_to(p, msg);
        }
    }

    fn host_broadcast_state(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let phase = match &self.current {
            Some(m) => Phase::InMatch {
                match_id: m.info.match_id,
            },
            None => Phase::Lobby,
        };
        let state = LobbyState {
            phase,
            machines: h
                .machines
                .values()
                .filter(|m| m.hello)
                .map(|m| m.view.clone())
                .collect(),
            settings: self.settings.clone(),
        };
        let slots: usize = lobby::accepted_slots(&state.machines)
            .iter()
            .map(|x| x.1)
            .sum();
        let mut md = BTreeMap::new();
        md.insert("players".to_string(), format!("{slots}"));
        let mode = match self.settings.team_mode {
            wire::TeamMode::Ffa => "free-for-all".to_string(),
            wire::TeamMode::Teams(n) => format!("{n} teams"),
        };
        md.insert("mode".to_string(), mode);
        for (k, v) in &self.extra_metadata {
            md.insert(k.clone(), v.clone());
        }
        if md != h.metadata {
            h.metadata = md.clone();
            self.with_client(|c| c.set_metadata(md));
        }
        if state == self.lobby {
            return;
        }
        self.lobby = state.clone();
        self.host_broadcast(&LobbyMsg::State(state), None);
        self.emit(NetEvent::LobbyChanged);
    }
}

fn random_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.finish()
}
