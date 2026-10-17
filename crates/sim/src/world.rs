//! The world state, its construction, the tick, and the checksum
//! (docs/sim-design.md "Shape").

use crate::content::{Content, MapId};
use crate::events::{Event, Events};
use crate::flow::Flow;
use crate::input::PlayerInput;
use crate::map::MapState;
use crate::math::{V2, eval_curve, key};
use crate::objects::SpawnedObject;
use crate::physics::Physics;
use crate::player::Player;
use crate::projectile::Projectile;
use crate::rng::Rngs;
use serde::{Deserialize, Serialize};

/// Fixed step (s).
pub const DT: f32 = 0.014;
/// The original's `baseTimeScale`.
pub const BASE_TIME_SCALE: f32 = 0.85;
pub const MAX_PLAYERS: usize = 12;

/// Real-time wait of `seconds` as ticks: `ceil(seconds / 0.014)`
/// (player.md 0.1; match-flow.md intro).
pub fn ticks(seconds: f32) -> u32 {
    let t = seconds as f64 / DT as f64;
    (t - 1e-6).ceil().max(0.0) as u32
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    /// Rounds, points, card picks (match-flow.md 1).
    #[default]
    ArmsRace,
    /// Free play: respawn after 2.5 s at a random spawn point, no picks.
    Sandbox,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamMode {
    /// Every player is its own team (team = slot index).
    #[default]
    FreeForAll,
    /// Teams as given per slot.
    Teams,
}

/// How the card picks of one pick phase run (match-flow.md 16, PORT
/// DECISION). Set with [`World::set_pick_mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickMode {
    /// The original: one picker after another, in player-index order, with
    /// the shared selection cursor (`World::pick`, `World::pick_selected`).
    Sequential,
    /// Every picker gets its own hand at the same time and navigates and
    /// confirms with its own input (`World::pick_hands`); the phase ends
    /// when everybody has picked.
    Simultaneous,
    /// Simultaneous when more than [`AUTO_SIMULTANEOUS_ABOVE`] players pick
    /// in that phase, else sequential.
    #[default]
    Auto,
}

/// [`PickMode::Auto`] picks simultaneously when more than this many players
/// pick in one phase.
pub const AUTO_SIMULTANEOUS_ABOVE: usize = 3;

impl PickMode {
    /// Whether a phase with `pickers` pickers runs simultaneously.
    pub fn simultaneous_for(self, pickers: usize) -> bool {
        match self {
            PickMode::Sequential => false,
            PickMode::Simultaneous => true,
            PickMode::Auto => pickers > AUTO_SIMULTANEOUS_ABOVE,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSetup {
    /// Team id (only used with `TeamMode::Teams`).
    pub team: u8,
    /// Skin/colour index (presentation; the sim stores it for convenience).
    pub skin: u8,
}

/// Identical on every peer (the lobby sends it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchSetup {
    /// 1..=12 player slots, index = player id.
    pub slots: Vec<SlotSetup>,
    pub mode: GameMode,
    pub teams: TeamMode,
    pub seed: u64,
    /// Points to win a round (original 2).
    pub points_to_win_round: u8,
    /// Rounds to win the game (original 5).
    pub rounds_to_win_game: u8,
    /// Initial pick for everyone and picks for round losers (original true).
    pub pick_phase: bool,
    /// Maps to choose from (empty = all maps of the content).
    pub map_rotation: Vec<MapId>,
}

impl MatchSetup {
    pub fn ffa(players: usize, seed: u64) -> MatchSetup {
        MatchSetup {
            slots: (0..players)
                .map(|i| SlotSetup {
                    team: i as u8,
                    skin: i as u8,
                })
                .collect(),
            mode: GameMode::ArmsRace,
            teams: TeamMode::FreeForAll,
            seed,
            points_to_win_round: 2,
            rounds_to_win_game: 5,
            pick_phase: true,
            map_rotation: vec![],
        }
    }

    /// Team id per slot as the sim uses it (dense 0..team_count).
    pub fn resolved_teams(&self) -> (Vec<u8>, u8) {
        match self.teams {
            TeamMode::FreeForAll => (
                (0..self.slots.len() as u8).collect(),
                self.slots.len() as u8,
            ),
            TeamMode::Teams => {
                let mut ids: Vec<u8> = self.slots.iter().map(|s| s.team).collect();
                ids.sort();
                ids.dedup();
                let t = self
                    .slots
                    .iter()
                    .map(|s| ids.iter().position(|x| *x == s.team).unwrap() as u8)
                    .collect();
                (t, ids.len() as u8)
            }
        }
    }
}

/// Time-scale curves (match-flow.md 8), advanced in real time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TimeCurve {
    None,
    SlowDown(f32),
    SpeedUp(f32),
}

#[allow(clippy::approx_constant)] // curve slope from the data, not 1/pi
const SLOW_DOWN: [crate::math::Key; 4] = [
    key(0.0, 1.0, -0.318, -0.318),
    key(0.065, 0.730, -14.30, -14.30),
    key(0.1056, 0.264, -4.385, -4.385),
    key(0.2, 0.15, 0.0, 0.0),
];
const SPEED_UP: [crate::math::Key; 2] = [key(0.0, 0.025, 0.0, 0.0), key(0.1, 1.0, 0.0, 0.0)];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeState {
    pub game_over_time: f32,
    pub game_start_time: f32,
    pub curve: TimeCurve,
}

impl Default for TimeState {
    fn default() -> Self {
        TimeState {
            game_over_time: 1.0,
            game_start_time: 1.0,
            curve: TimeCurve::None,
        }
    }
}

impl TimeState {
    /// `ts = 0.85 * gameOverTime * gameStartTime` (factors only when < 1).
    pub fn scale(&self) -> f32 {
        BASE_TIME_SCALE * self.game_over_time.min(1.0) * self.game_start_time.min(1.0)
    }
    pub fn slow_down(&mut self) {
        self.curve = TimeCurve::SlowDown(0.0);
        self.game_over_time = SLOW_DOWN[0].value;
    }
    pub fn speed_up(&mut self) {
        self.curve = TimeCurve::SpeedUp(0.0);
        self.game_over_time = SPEED_UP[0].value;
    }
    /// Advance by one real tick.
    pub fn tick(&mut self) {
        match self.curve {
            TimeCurve::None => {}
            TimeCurve::SlowDown(t) => {
                let t = t + DT;
                self.game_over_time = eval_curve(&SLOW_DOWN, t);
                self.curve = if t >= 0.2 {
                    TimeCurve::None
                } else {
                    TimeCurve::SlowDown(t)
                };
            }
            TimeCurve::SpeedUp(t) => {
                let t = t + DT;
                self.game_over_time = eval_curve(&SPEED_UP, t);
                self.curve = if t >= 0.1 {
                    TimeCurve::None
                } else {
                    TimeCurve::SpeedUp(t)
                };
            }
        }
    }
}

/// Match-wide settings resolved from [`MatchSetup`].
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub mode: GameMode,
    pub team_of: Vec<u8>,
    pub team_count: u8,
    pub points_to_win_round: u8,
    pub rounds_to_win_game: u8,
    pub pick_phase: bool,
    pub rotation: Vec<MapId>,
    /// See [`World::set_pick_mode`] (default [`PickMode::Auto`]).
    pub pick_mode: PickMode,
}

#[derive(Clone, Debug)]
pub struct World {
    pub(crate) frame: u32,
    pub(crate) rng: Rngs,
    pub(crate) time: TimeState,
    pub(crate) rules: Rules,
    pub(crate) flow: Flow,
    pub(crate) players: Vec<Player>,
    /// Inputs of the previous tick (edge detection).
    pub(crate) prev_inputs: Vec<PlayerInput>,
    pub(crate) inputs: Vec<PlayerInput>,
    pub(crate) projectiles: Vec<Projectile>,
    pub(crate) objects: Vec<SpawnedObject>,
    pub(crate) map: MapState,
    pub(crate) physics: Physics,
    pub(crate) next_id: u32,
    pub(crate) events: Events,
    /// Scores per team.
    pub(crate) points: Vec<u8>,
    pub(crate) rounds: Vec<u8>,
    /// Slots to remove at the start of the next step (`set_disconnected`).
    pending_forfeits: Vec<u8>,
}

impl World {
    /// A new match. Starts the game start sequence (match-flow.md 4) or, in
    /// sandbox mode, loads a map and drops the players in.
    pub fn new(content: &Content, setup: &MatchSetup) -> World {
        assert!(
            !setup.slots.is_empty() && setup.slots.len() <= MAX_PLAYERS,
            "1..=12 slots"
        );
        assert!(!content.maps.is_empty(), "content has no maps");
        let (team_of, team_count) = setup.resolved_teams();
        let rotation = if setup.map_rotation.is_empty() {
            (0..content.maps.len() as u16).map(MapId).collect()
        } else {
            setup.map_rotation.clone()
        };
        let n = setup.slots.len();
        let players = (0..n)
            .map(|i| Player::new(i as u8, team_of[i], setup.slots[i].skin))
            .collect();
        let mut w = World {
            frame: 0,
            rng: Rngs::new(setup.seed),
            time: TimeState::default(),
            rules: Rules {
                mode: setup.mode,
                team_of,
                team_count,
                points_to_win_round: setup.points_to_win_round.max(1),
                rounds_to_win_game: setup.rounds_to_win_game.max(1),
                pick_phase: setup.pick_phase,
                rotation,
                pick_mode: PickMode::default(),
            },
            flow: Flow::default(),
            players,
            prev_inputs: vec![PlayerInput::default(); n],
            inputs: vec![PlayerInput::default(); n],
            projectiles: Vec::new(),
            objects: Vec::new(),
            map: MapState::default(),
            physics: Physics::default(),
            next_id: 1,
            events: Events::default(),
            points: vec![0; team_count as usize],
            rounds: vec![0; team_count as usize],
            pending_forfeits: Vec::new(),
        };
        for p in w.players.iter_mut() {
            p.full_reset(content);
        }
        w.flow_init(content);
        w
    }

    /// Advance one tick. `inputs[i]` is player i's input (missing = zero).
    pub fn step(&mut self, content: &Content, inputs: &[PlayerInput]) {
        self.frame = self.frame.wrapping_add(1);
        self.events.begin(self.frame);
        for i in 0..self.players.len() {
            self.prev_inputs[i] = self.inputs[i];
            let mut input = inputs.get(i).copied().unwrap_or_default();
            if self.players[i].disconnected {
                input = PlayerInput::default();
            } else if input.held(PlayerInput::BTN_FORFEIT) {
                self.queue_forfeit(i);
                input = PlayerInput::default();
            }
            self.inputs[i] = input;
        }
        let mut forfeits = std::mem::take(&mut self.pending_forfeits);
        forfeits.sort_unstable();
        for i in forfeits {
            self.forfeit_player(content, i as usize);
        }
        self.time.tick();
        let ts = self.time.scale();
        let game_dt = DT * ts;

        self.flow_tick(content);
        self.map_tick(content, game_dt);

        for i in 0..self.players.len() {
            self.player_frame(content, i, ts);
        }
        self.projectiles_tick(content, game_dt);
        self.objects_tick(content, game_dt);
        self.pulls_tick(game_dt);
        for i in 0..self.players.len() {
            self.player_fixed(content, i, ts);
        }
        self.props_tick(content, game_dt);
        self.process_deaths(content);
    }

    /// Removes player `slot` from the match for good (disconnect / forfeit;
    /// docs/spec/match-flow.md 13). Call it identically on every peer
    /// *between* two steps (e.g. right after loading the confirmed freeze
    /// frame): the removal is part of the world state (and the checksum) and
    /// takes effect at the start of the next [`World::step`], so its events
    /// belong to that step. The same happens in-band for a
    /// [`PlayerInput::BTN_FORFEIT`] input. Idempotent; out-of-range slots
    /// are ignored.
    ///
    /// The player dies without a respawn effect, is parked off-map, loses
    /// its attached effects, is no longer revived or spawned on later maps,
    /// skipped in card picks and the rematch vote, and its team no longer
    /// counts as alive. The round ends as usual if this leaves fewer than two
    /// teams alive; the match continues while at least two teams have a
    /// connected player. Otherwise the remaining team wins by forfeit (a
    /// `GameWon` event, then `Phase::GameOver`, then `Phase::Ended` with
    /// `MatchEnded`), or the match ends at once if nobody is left. The score
    /// is not changed by a forfeit.
    pub fn set_disconnected(&mut self, slot: usize) {
        self.queue_forfeit(slot);
    }

    fn queue_forfeit(&mut self, slot: usize) {
        if slot < self.players.len()
            && !self.players[slot].disconnected
            && !self.pending_forfeits.contains(&(slot as u8))
        {
            self.pending_forfeits.push(slot as u8);
            self.pending_forfeits.sort_unstable();
        }
    }

    /// Chooses how card picks run ([`PickMode`]; default `Auto`). Part of
    /// the match settings: call it identically on every peer, right after
    /// [`World::new`] (it is included in the checksum). A change applies
    /// from the next pick phase on; a running phase keeps its mode.
    pub fn set_pick_mode(&mut self, mode: PickMode) {
        self.rules.pick_mode = mode;
    }

    /// The configured [`PickMode`].
    pub fn pick_mode(&self) -> PickMode {
        self.rules.pick_mode
    }

    /// Whether `slot` was removed (or is queued for removal) by
    /// [`World::set_disconnected`] or a [`PlayerInput::BTN_FORFEIT`] input.
    pub fn is_disconnected(&self, slot: usize) -> bool {
        self.players.get(slot).is_some_and(|p| p.disconnected)
            || self.pending_forfeits.contains(&(slot as u8))
    }

    /// Events produced by the last step.
    pub fn events(&self) -> &[Event] {
        &self.events.list
    }

    pub fn frame(&self) -> u32 {
        self.frame
    }

    pub(crate) fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Hash of the gameplay-relevant state (desync detection).
    pub fn checksum(&self) -> u64 {
        let mut h = Hasher::new();
        h.u32(self.frame);
        for w in [
            self.rng.gun,
            self.rng.hit,
            self.rng.cards,
            self.rng.maps,
            self.rng.misc,
        ] {
            let [a, b] = w.words();
            h.u64(a);
            h.u64(b);
        }
        h.f32(self.time.game_over_time);
        h.u32(self.rules.pick_mode as u32);
        self.flow.hash_into(&mut h);
        for p in &self.pending_forfeits {
            h.u32(0xF0F0_0000 | *p as u32);
        }
        for p in &self.points {
            h.u32(*p as u32);
        }
        for r in &self.rounds {
            h.u32(*r as u32);
        }
        for p in &self.players {
            p.hash_into(&mut h);
        }
        h.u32(self.projectiles.len() as u32);
        for p in &self.projectiles {
            p.hash_into(&mut h);
        }
        h.u32(self.objects.len() as u32);
        for o in &self.objects {
            o.hash_into(&mut h);
        }
        self.map.hash_into(&mut h);
        self.physics.hash_into(&mut h);
        h.u32(self.next_id);
        h.finish()
    }

    /// Current game time scale (0.85 normally).
    pub fn time_scale(&self) -> f32 {
        self.time.scale()
    }
}

/// FxHash-style 64-bit hasher over the state's words in a fixed order.
pub struct Hasher(u64);

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    pub fn new() -> Hasher {
        Hasher(0xcbf2_9ce4_8422_2325)
    }
    #[inline]
    pub fn u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(Self::K);
    }
    #[inline]
    pub fn u32(&mut self, x: u32) {
        self.u64(x as u64);
    }
    #[inline]
    pub fn f32(&mut self, x: f32) {
        self.u32(x.to_bits());
    }
    #[inline]
    pub fn bool(&mut self, b: bool) {
        self.u32(b as u32);
    }
    #[inline]
    pub fn v2(&mut self, v: V2) {
        self.f32(v.x);
        self.f32(v.y);
    }
    pub fn finish(&self) -> u64 {
        // Final avalanche (splitmix64).
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_conversion() {
        assert_eq!(ticks(0.014), 1);
        assert_eq!(ticks(0.7), 50);
        assert_eq!(ticks(0.2), 15);
        assert_eq!(ticks(2.6), 186);
        assert_eq!(ticks(0.0), 0);
    }

    #[test]
    fn slow_motion_curve() {
        let mut t = TimeState::default();
        assert!((t.scale() - 0.85).abs() < 1e-6);
        t.slow_down();
        for _ in 0..20 {
            t.tick();
        }
        assert!((t.game_over_time - 0.15).abs() < 1e-4);
        assert!((t.scale() - 0.1275).abs() < 1e-4);
        t.speed_up();
        for _ in 0..10 {
            t.tick();
        }
        assert!((t.scale() - 0.85).abs() < 1e-5);
    }
}

#[cfg(test)]
mod send_tests {
    #[test]
    fn world_is_send_sync_clone() {
        fn check<T: Send + Sync + Clone + 'static>() {}
        check::<super::World>();
    }
}
