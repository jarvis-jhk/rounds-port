//! Match flow (docs/spec/match-flow.md): start sequence, points, rounds,
//! card picks, map transitions, game over and rematch, sandbox; generalised
//! to N teams and 12 players. All timers are real ticks.
//!
//! Spawn assignment for more than 4 players / teams: see [`spawn_slots`].

use crate::cards::draw_slot;
use crate::content::{CardId, Content, MapId};
use crate::effects::Hook;
use crate::events::{EventKind, cue};
use crate::geom::{Shape, layers};
use crate::input::menu_nav;
use crate::math::{V2, v2};
use crate::player::{PARK, SpawnMove};
use crate::world::{GameMode, Hasher, World, ticks};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeKind {
    /// First battle of a game (after the initial picks).
    StartGame,
    /// After a round's picks.
    AfterRound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Game start sequence (match-flow.md 4), `t` ticks since start.
    Start {
        t: u32,
    },
    /// Pick queue running.
    Picking {
        after: ResumeKind,
    },
    /// Map call-in and fly-in before a battle.
    Resume {
        t: u32,
        kind: ResumeKind,
    },
    Battle,
    /// Point transition (5.3).
    PointOver {
        t: u32,
        winner: u8,
    },
    /// Round transition (5.4).
    RoundOver {
        t: u32,
        winner: u8,
    },
    GameOver {
        t: u32,
        winner: u8,
    },
    /// Rematch prompt; `yes` = highlighted answer.
    Rematch {
        yes: bool,
    },
    /// "No" chosen: the match is over, the game returns to the menu.
    Ended,
    Sandbox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickStage {
    /// Cards being dealt; `t` ticks since the pick started.
    Offering,
    Choosing,
    Ending(u32),
    Gap(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickState {
    pub player: u8,
    pub hand: Vec<CardId>,
    pub stage: PickStage,
    pub t: u32,
}

/// One picker's hand in a simultaneous pick phase ([`crate::PickMode`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PickHand {
    /// Picker, offered cards, stage and ticks since the phase started, with
    /// the same timeline as a sequential pick (match-flow.md 9.2-9.4):
    /// `Offering` (slot j dealt at 0.2 s + 0.1 s * j), `Choosing`,
    /// `Ending` (0.9 s after the confirm), `Gap` (0.1 s), then `done`.
    pub state: PickState,
    /// This picker's highlighted slot.
    pub selected: u8,
    /// The confirmed slot (None while choosing, or when aborted).
    pub picked: Option<u8>,
    /// Finished (picked and the end animation is over, or aborted because
    /// the player disconnected).
    pub done: bool,
}

impl PickHand {
    /// The confirmed card.
    pub fn picked_card(&self) -> Option<CardId> {
        self.picked
            .and_then(|s| self.state.hand.get(s as usize).copied())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub phase: Phase,
    pub battle: bool,
    pub transitioning: bool,
    /// Real deaths this tick (ascending order is applied when processing).
    pub deaths: Vec<u8>,
    pub pick_queue: Vec<u8>,
    pub pick: Option<PickState>,
    /// Selected card index; the original never resets it between picks.
    pub selected: u8,
    /// Sandbox respawns: (player, real ticks left).
    pub respawns: Vec<(u8, u32)>,
    pub last_winner: Option<u8>,
    /// The match ends after the game-over text because fewer than two teams
    /// have a connected player (docs/spec/match-flow.md 13): no rematch.
    pub forfeit_end: bool,
    /// Hands of a running simultaneous pick phase, in player order (empty in
    /// sequential mode).
    pub hands: Vec<PickHand>,
    /// The running pick phase is simultaneous.
    pub simultaneous: bool,
    /// Further passes of this pick phase ([`crate::mods::PickNCards`]):
    /// passes left and their pickers. Always 0 / empty in vanilla.
    pub repicks: u8,
    pub repickers: Vec<u8>,
}

impl Default for Flow {
    fn default() -> Self {
        Flow {
            phase: Phase::Start { t: 0 },
            battle: false,
            transitioning: false,
            deaths: Vec::new(),
            pick_queue: Vec::new(),
            pick: None,
            selected: 0,
            respawns: Vec::new(),
            last_winner: None,
            forfeit_end: false,
            hands: Vec::new(),
            simultaneous: false,
            repicks: 0,
            repickers: Vec::new(),
        }
    }
}

impl Flow {
    pub fn battle_ongoing(&self) -> bool {
        self.battle
    }

    pub(crate) fn hash_into(&self, h: &mut Hasher) {
        let (k, t) = match self.phase {
            Phase::Start { t } => (0, t),
            Phase::Picking { .. } => (1, 0),
            Phase::Resume { t, .. } => (2, t),
            Phase::Battle => (3, 0),
            Phase::PointOver { t, .. } => (4, t),
            Phase::RoundOver { t, .. } => (5, t),
            Phase::GameOver { t, .. } => (6, t),
            Phase::Rematch { yes } => (7, yes as u32),
            Phase::Ended => (8, 0),
            Phase::Sandbox => (9, 0),
        };
        h.u32(k);
        h.u32(t);
        h.bool(self.battle);
        if let Some(p) = &self.pick {
            h.u32(p.player as u32);
            for c in &p.hand {
                h.u32(c.0 as u32);
            }
            h.u32(p.t);
        }
        h.u32(self.selected as u32);
        h.bool(self.forfeit_end);
        h.bool(self.simultaneous);
        for hd in &self.hands {
            h.u32(hd.state.player as u32);
            for c in &hd.state.hand {
                h.u32(c.0 as u32);
            }
            h.u32(hd.state.t);
            h.u32(hd.selected as u32);
            h.u32(hd.picked.map_or(0xFF, |s| s as u32));
            h.bool(hd.done);
        }
        if self.repicks > 0 || !self.repickers.is_empty() {
            h.u32(0x5245_0000 | self.repicks as u32);
            for p in &self.repickers {
                h.u32(*p as u32);
            }
        }
    }
}

/// Spawn positions for `n` players on a map.
///
/// Rule (match-flow.md 7.2 GENERALISE, port decision):
/// * Free-for-all: player k < 4 uses authored point k; for k >= 4 the base is
///   point `k mod 4` and the slot is searched outward along x in 1.5-unit
///   steps (+1.5, -1.5, +3, -3, ... up to 12 units): the first candidate that
///   has ground within 3 units below, does not overlap map geometry, lies
///   inside the arena and is at least 1.5 units from every assigned slot.
///   Fallback: the base point.
/// * Teams (2 teams): team 0 uses the left anchors (0, 2), team 1 the right
///   anchors (1, 3); members alternate between their anchors, extra members
///   are derived as above. With more teams, team t uses anchor `t mod 4`.
pub fn spawn_slots(
    content: &Content,
    map: MapId,
    team_of: &[u8],
    team_count: u8,
    teams: bool,
) -> Vec<V2> {
    let def = content.map(map);
    let anchors: Vec<V2> = if def.spawn_points.is_empty() {
        vec![v2(0.0, 0.0)]
    } else {
        def.spawn_points.iter().map(|s| s.pos).collect()
    };
    let prepared = content.prepared(map);
    let scene = crate::scene::Scene {
        statics: Some(prepared),
        animated: &[],
        props: &[],
        players: &[],
    };
    let half = content.tuning.bounds_half;
    let mut slots: Vec<V2> = Vec::with_capacity(team_of.len());
    let mut team_member = vec![0usize; team_count as usize];
    for (k, team) in team_of.iter().enumerate() {
        let (anchor_idx, rank) = if teams && team_count >= 2 {
            let m = team_member[*team as usize];
            team_member[*team as usize] += 1;
            let own: Vec<usize> = if team_count == 2 {
                if *team == 0 { vec![0, 2] } else { vec![1, 3] }
            } else {
                vec![*team as usize % 4]
            };
            (own[m % own.len()] % anchors.len(), m / own.len())
        } else {
            (k % anchors.len(), k / anchors.len())
        };
        let base = anchors[anchor_idx];
        if rank == 0 && !slots.iter().any(|s| (*s - base).len() < 0.5) {
            slots.push(base);
            continue;
        }
        let mut found = None;
        let mut step = 1;
        while step <= 8 && found.is_none() {
            for sgn in [1.0f32, -1.0] {
                let c = base + v2(sgn * 1.5 * step as f32, 0.0);
                if c.x.abs() > half.x - 2.0 || c.y.abs() > half.y - 1.0 {
                    continue;
                }
                if slots.iter().any(|s| (*s - c).len() < 1.5) {
                    continue;
                }
                if scene.map_blocked(c, 0.7, layers::PLAYER_COLLISION) {
                    continue;
                }
                let ground = scene.raycast(
                    c,
                    V2::DOWN,
                    3.0,
                    layers::LEG_RAY & !layers::bit(layers::PLAYER),
                    None,
                );
                if ground.is_some() {
                    found = Some(c);
                    break;
                }
            }
            step += 1;
        }
        slots.push(found.unwrap_or(base));
    }
    let _ = Shape::Circle {
        c: V2::ZERO,
        r: 0.0,
    };
    slots
}

impl World {
    pub(crate) fn flow_init(&mut self, content: &Content) {
        match self.rules.mode {
            GameMode::ArmsRace => {
                self.flow.phase = Phase::Start { t: 0 };
                self.events.push(EventKind::MatchStarted);
                self.events.sound(cue::TEXT_APPEAR, V2::ZERO, 1.0);
            }
            GameMode::Sandbox => {
                self.flow.phase = Phase::Sandbox;
                self.load_next_map(content);
                self.call_in_map();
                // `callInImmediately`: the map is solid on the next tick.
                self.map.phase = crate::map::MapPhase::Entering(ticks(crate::map::SOLID_AFTER) - 1);
                self.flow.battle = true;
                for i in 0..self.players.len() {
                    let r = self.rng.maps.below(
                        content
                            .map(self.map.current.unwrap())
                            .spawn_points
                            .len()
                            .max(1) as u32,
                    ) as usize;
                    let to = content
                        .map(self.map.current.unwrap())
                        .spawn_points
                        .get(r)
                        .map(|s| s.pos)
                        .unwrap_or(V2::ZERO);
                    self.start_spawn_move(i, to);
                }
            }
        }
    }

    fn start_spawn_move(&mut self, i: usize, to: V2) {
        if self.players[i].disconnected {
            return;
        }
        let p = &mut self.players[i];
        p.playing = false;
        p.simulated = false;
        p.kinematic = true;
        p.spawn_move = Some(SpawnMove {
            from: p.pos,
            to,
            c: 0.0,
            hold: 10,
        });
        let from = p.pos;
        self.events.push(EventKind::PlayerSpawnMoveStarted {
            player: i as u8,
            from,
            to,
        });
    }

    /// `CallInNewMapAndMovePlayers` (+ optional revive of everyone).
    fn call_in_and_move(&mut self, content: &Content, revive: bool) {
        self.call_in_map();
        let Some(m) = self.map.current else { return };
        let teams = matches!(self.rules.team_count as usize, n if n < self.players.len());
        let slots = spawn_slots(
            content,
            m,
            &self.rules.team_of,
            self.rules.team_count,
            teams,
        );
        for (i, slot) in slots.into_iter().enumerate() {
            if revive {
                self.revive(content, i, true);
            }
            self.start_spawn_move(i, slot);
        }
    }

    fn park_players(&mut self) {
        for p in self.players.iter_mut() {
            p.simulated = false;
            p.playing = false;
            p.since_grounded = 0.0;
            p.spawn_move = None;
            p.pos = PARK;
            p.last_pos = PARK;
            p.vel = V2::ZERO;
            p.gun_pos = PARK;
        }
    }

    fn set_players_simulated(&mut self, on: bool) {
        for p in self.players.iter_mut() {
            p.simulated = on;
            if !on {
                p.since_grounded = 0.0;
            }
        }
    }

    pub(crate) fn flow_tick(&mut self, content: &Content) {
        match self.flow.phase {
            Phase::Start { t } => {
                let t = t + 1;
                self.flow.phase = Phase::Start { t };
                if t == ticks(0.25) {
                    self.events.sound(cue::TEXT_DISAPPEAR, V2::ZERO, 1.0);
                    self.park_players();
                    self.load_next_map(content);
                    self.time.speed_up();
                    self.events.push(EventKind::TimeCurve { slow: false });
                }
                if t == ticks(1.25) {
                    if self.rules.pick_phase {
                        let pickers = (0..self.players.len() as u8)
                            .filter(|i| !self.players[*i as usize].disconnected)
                            .collect();
                        self.begin_picks(pickers);
                        self.flow.phase = Phase::Picking {
                            after: ResumeKind::StartGame,
                        };
                    } else {
                        self.flow.phase = Phase::Resume {
                            t: 0,
                            kind: ResumeKind::StartGame,
                        };
                    }
                }
            }
            Phase::Picking { after } => {
                let more = if self.flow.simultaneous {
                    self.hands_tick(content)
                } else {
                    self.pick_tick(content)
                };
                if !more {
                    self.flow.simultaneous = false;
                    self.flow.phase = Phase::Resume { t: 0, kind: after };
                }
            }
            Phase::Resume { t, kind } => {
                if t == 0 {
                    match kind {
                        ResumeKind::StartGame => {
                            self.call_in_and_move(content, true);
                            self.time.speed_up();
                            self.events.push(EventKind::TimeCurve { slow: false });
                            self.begin_battle(content);
                            return;
                        }
                        ResumeKind::AfterRound => {
                            self.time.slow_down();
                            self.events.push(EventKind::TimeCurve { slow: true });
                            self.call_in_and_move(content, true);
                        }
                    }
                }
                let t = t + 1;
                self.flow.phase = Phase::Resume { t, kind };
                if t >= ticks(0.3) {
                    self.time.speed_up();
                    self.events.push(EventKind::TimeCurve { slow: false });
                    self.begin_battle(content);
                }
            }
            Phase::Battle | Phase::Ended => {}
            Phase::PointOver { t, winner } => {
                let t = t + 1;
                self.flow.phase = Phase::PointOver { t, winner };
                if t == ticks(1.0) {
                    self.load_next_map(content);
                }
                if t == ticks(1.5) {
                    self.call_in_and_move(content, true);
                }
                if t >= ticks(1.8) {
                    self.time.speed_up();
                    self.events.push(EventKind::TimeCurve { slow: false });
                    self.begin_battle(content);
                }
            }
            Phase::RoundOver { t, winner } => {
                let t = t + 1;
                self.flow.phase = Phase::RoundOver { t, winner };
                if t == ticks(1.0) {
                    self.load_next_map(content);
                }
                if t >= ticks(2.3) {
                    self.time.speed_up();
                    self.events.push(EventKind::TimeCurve { slow: false });
                    let losers: Vec<u8> = (0..self.players.len() as u8)
                        .filter(|i| {
                            self.rules.team_of[*i as usize] != winner
                                && !self.players[*i as usize].disconnected
                        })
                        .collect();
                    if self.rules.pick_phase && !losers.is_empty() {
                        self.park_players();
                        self.begin_picks(losers);
                        self.flow.phase = Phase::Picking {
                            after: ResumeKind::AfterRound,
                        };
                    } else {
                        self.flow.phase = Phase::Resume {
                            t: 0,
                            kind: ResumeKind::AfterRound,
                        };
                    }
                }
            }
            Phase::GameOver { t, winner } => {
                let t = t + 1;
                self.flow.phase = Phase::GameOver { t, winner };
                if t >= ticks(2.0) && self.flow.forfeit_end {
                    self.flow.phase = Phase::Ended;
                    self.events.push(EventKind::MatchEnded);
                } else if t >= ticks(2.0) {
                    self.flow.phase = Phase::Rematch { yes: true };
                    self.events.push(EventKind::RematchPrompt);
                    for i in 0..self.players.len() {
                        if self.players[i].disconnected {
                            continue;
                        }
                        self.players[i].simulated = true;
                        self.players[i].playing = true;
                        self.revive(content, i, true);
                    }
                    self.load_next_map(content);
                    self.time.speed_up();
                }
            }
            Phase::Rematch { yes } => {
                let mut sel = yes;
                let mut confirm = false;
                for i in 0..self.players.len() {
                    if self.players[i].disconnected {
                        continue;
                    }
                    let x = self.inputs[i].move_vec().x;
                    if x < -0.5 {
                        sel = true;
                    } else if x > 0.5 {
                        sel = false;
                    }
                    let (_, ok, _) = menu_nav(&self.inputs[i], &self.prev_inputs[i]);
                    confirm |= ok;
                }
                if sel != yes {
                    self.events.push(EventKind::RematchSelected { yes: sel });
                }
                self.flow.phase = Phase::Rematch { yes: sel };
                if confirm {
                    if sel {
                        self.rematch(content);
                    } else {
                        self.flow.phase = Phase::Ended;
                        self.events.push(EventKind::MatchEnded);
                    }
                }
            }
            Phase::Sandbox => {
                let mut k = 0;
                while k < self.flow.respawns.len() {
                    let (pl, t) = self.flow.respawns[k];
                    if t <= 1 {
                        self.flow.respawns.remove(k);
                        let m = self.map.current.unwrap();
                        let sp = &content.map(m).spawn_points;
                        let r = self.rng.maps.below(sp.len().max(1) as u32) as usize;
                        let to = sp.get(r).map(|s| s.pos).unwrap_or(V2::ZERO);
                        let i = pl as usize;
                        if self.players[i].disconnected {
                            continue;
                        }
                        self.revive(content, i, true);
                        self.teleport(i, to);
                        self.players[i].playing = true;
                        self.players[i].simulated = true;
                        self.players[i].kinematic = false;
                        self.events.push(EventKind::PlayerSpawned {
                            player: pl,
                            pos: to,
                        });
                    } else {
                        self.flow.respawns[k].1 = t - 1;
                        k += 1;
                    }
                }
            }
        }
    }

    /// Removes a player from the match (see [`World::set_disconnected`]).
    pub(crate) fn forfeit_player(&mut self, content: &Content, i: usize) {
        if i >= self.players.len() || self.players[i].disconnected {
            return;
        }
        {
            let p = &mut self.players[i];
            p.disconnected = true;
            p.dead = true;
            p.respawning = false;
            p.remaining_respawns = 0;
            p.phoenix_ticks = 0;
            p.playing = false;
            p.simulated = false;
            p.kinematic = true;
            p.spawn_move = None;
            p.dots.clear();
            p.pulls.clear();
            p.pending_blocks.clear();
            p.stun_time = 0.0;
            p.stunned = false;
            p.silence_time = 0.0;
            p.silenced = false;
            p.bounce = None;
            p.since_grounded = 0.0;
            p.pos = PARK;
            p.last_pos = PARK;
            p.vel = V2::ZERO;
            p.gun_pos = PARK;
        }
        self.clear_player_effects(content, i);
        self.events
            .push(EventKind::PlayerDisconnected { player: i as u8 });
        // Card picks: drop its place in the queue, abort a running pick.
        self.flow.pick_queue.retain(|p| *p as usize != i);
        self.flow.repickers.retain(|p| *p as usize != i);
        self.flow.respawns.retain(|(p, _)| *p as usize != i);
        if let Some(ps) = &mut self.flow.pick
            && ps.player as usize == i
            && matches!(ps.stage, PickStage::Offering | PickStage::Choosing)
        {
            self.events.push(EventKind::PickEnded { player: i as u8 });
            ps.stage = PickStage::Gap(ticks(0.1));
        }
        // Simultaneous picks: the player's hand is dropped (no card).
        for h in self.flow.hands.iter_mut() {
            if h.state.player as usize == i && !h.done && h.picked.is_none() {
                self.events.push(EventKind::PickEnded { player: i as u8 });
                h.state.stage = PickStage::Gap(ticks(0.1));
            }
        }
        // Fewer than two teams with a connected player: the match is over.
        let mut connected: Vec<u8> = self
            .players
            .iter()
            .filter(|p| !p.disconnected)
            .map(|p| p.team)
            .collect();
        connected.sort_unstable();
        connected.dedup();
        let last_team_standing = self.rules.team_count >= 2 && connected.len() < 2;
        if self.rules.mode == GameMode::Sandbox {
            if connected.is_empty() {
                self.end_match();
            }
        } else if last_team_standing || connected.is_empty() {
            self.forfeit_end(connected.first().copied());
        } else if self.flow.battle && !self.flow.transitioning {
            // A death for the round-end check at the end of the step.
            self.flow.deaths.push(i as u8);
        }
    }

    fn end_match(&mut self) {
        if self.flow.phase != Phase::Ended {
            self.flow.phase = Phase::Ended;
            self.flow.battle = false;
            self.events.push(EventKind::MatchEnded);
        }
    }

    /// Fewer than two teams are connected: `winner` (the remaining team, if
    /// any) wins by forfeit. A running game-over text finishes first; the
    /// rematch prompt and "Ended" follow without a vote. The score is
    /// unchanged.
    fn forfeit_end(&mut self, winner: Option<u8>) {
        self.flow.forfeit_end = true;
        let Some(winner) = winner else {
            return self.end_match();
        };
        match self.flow.phase {
            Phase::Ended | Phase::GameOver { .. } => {}
            Phase::Rematch { .. } => self.end_match(),
            _ => {
                self.flow.pick = None;
                self.flow.pick_queue.clear();
                self.flow.repicks = 0;
                self.flow.repickers.clear();
                self.flow.hands.clear();
                self.flow.simultaneous = false;
                self.flow.battle = false;
                self.flow.transitioning = true;
                self.flow.deaths.clear();
                self.flow.last_winner = Some(winner);
                self.set_players_simulated(false);
                self.time.slow_down();
                self.events.push(EventKind::TimeCurve { slow: true });
                self.events.push(EventKind::GameWon { team: winner });
                self.flow.phase = Phase::GameOver { t: 0, winner };
            }
        }
    }

    fn begin_battle(&mut self, content: &Content) {
        self.flow.phase = Phase::Battle;
        self.flow.battle = true;
        self.flow.transitioning = false;
        self.events.push(EventKind::BattleStarted);
        for i in 0..self.players.len() {
            self.fire_hook(content, i, Hook::RoundStart);
        }
    }

    fn rematch(&mut self, content: &Content) {
        for i in 0..self.players.len() {
            self.clear_player_effects(content, i);
            self.players[i].full_reset(content);
        }
        self.sweep_objects();
        for x in self.points.iter_mut().chain(self.rounds.iter_mut()) {
            *x = 0;
        }
        self.events.push(EventKind::ScoreReset);
        self.events.push(EventKind::Rematch);
        self.flow = Flow {
            selected: self.flow.selected,
            ..Flow::default()
        };
        self.events.push(EventKind::MatchStarted);
        self.events.sound(cue::TEXT_APPEAR, V2::ZERO, 1.0);
    }

    /// Starts a pick phase for `pickers` (player order) in the configured
    /// [`crate::PickMode`].
    fn begin_picks(&mut self, pickers: Vec<u8>) {
        self.flow.pick = None;
        self.flow.hands.clear();
        self.flow.simultaneous = self.rules.pick_mode.simultaneous_for(pickers.len());
        self.flow.repicks = self.rules.mods.picks() - 1;
        self.flow.repickers = if self.flow.repicks > 0 {
            pickers.clone()
        } else {
            Vec::new()
        };
        self.flow.pick_queue = pickers;
    }

    /// Starts the next pass of a pick phase with several picks per player
    /// ([`crate::mods::PickNCards`]). False when there is none (vanilla).
    fn next_pick_pass(&mut self) -> bool {
        if self.flow.repicks == 0 || self.flow.repickers.is_empty() {
            return false;
        }
        self.flow.repicks -= 1;
        self.flow.pick_queue = self.flow.repickers.clone();
        true
    }

    /// One tick of a simultaneous pick phase. Every picker runs the
    /// sequential pick timeline at the same time with its own hand, cursor
    /// and input; cards are drawn from the shared card PRNG slot by slot in
    /// player order. Returns false when every hand is done.
    fn hands_tick(&mut self, content: &Content) -> bool {
        if self.flow.hands.is_empty() {
            if self.flow.pick_queue.is_empty() && !self.next_pick_pass() {
                return false;
            }
            let start = self.flow.selected;
            self.flow.hands = std::mem::take(&mut self.flow.pick_queue)
                .into_iter()
                .map(|pl| PickHand {
                    state: PickState {
                        player: pl,
                        hand: Vec::new(),
                        stage: PickStage::Offering,
                        t: 0,
                    },
                    selected: start,
                    picked: None,
                    done: false,
                })
                .collect();
            for k in 0..self.flow.hands.len() {
                let pl = self.flow.hands[k].state.player;
                self.events.push(EventKind::PickStarted { player: pl });
            }
            self.events.sound(cue::BALL_APPEAR, V2::ZERO, 1.0);
        }
        let first = ticks(0.2);
        let step = ticks(0.1);
        let hand_size = self.rules.mods.hand_size();
        let mut cards_appear = false;
        for k in 0..self.flow.hands.len() {
            if self.flow.hands[k].done {
                continue;
            }
            let mut h = self.flow.hands[k].clone();
            let pl = h.state.player as usize;
            h.state.t += 1;
            if matches!(h.state.stage, PickStage::Offering | PickStage::Choosing)
                && !h.state.hand.is_empty()
            {
                let (nav, ok, _) = menu_nav(&self.inputs[pl], &self.prev_inputs[pl]);
                if nav != 0 {
                    let n = h.state.hand.len() as i32;
                    let s = (h.selected as i32 + nav).clamp(0, n - 1) as u8;
                    if s != h.selected {
                        h.selected = s;
                        self.events.push(EventKind::CardSelected {
                            player: pl as u8,
                            slot: s,
                        });
                        self.events.sound(cue::CARD_SWITCH, V2::ZERO, 1.0);
                    }
                }
                if ok && h.state.stage == PickStage::Choosing {
                    let slot = (h.selected as usize).min(h.state.hand.len() - 1);
                    let card = h.state.hand[slot];
                    self.apply_card(content, pl, card);
                    self.events.push(EventKind::CardPicked {
                        player: pl as u8,
                        card,
                        slot: slot as u8,
                    });
                    self.events.sound(cue::CARD_PICK, V2::ZERO, 1.0);
                    h.picked = Some(slot as u8);
                    h.state.stage = PickStage::Ending(ticks(0.9));
                }
            }
            match h.state.stage {
                PickStage::Offering => {
                    let n = h.state.hand.len() as u32;
                    if (n as usize) < hand_size && h.state.t >= first + step * n {
                        let owned = self.players[pl].cards.clone();
                        let c = draw_slot(content, &mut self.rng.cards, &h.state.hand, &owned);
                        h.state.hand.push(c);
                        self.events.push(EventKind::CardOffered {
                            player: pl as u8,
                            slot: n as u8,
                            card: c,
                        });
                        cards_appear |= n == 0;
                    } else if h.state.hand.len() == hand_size
                        && h.state.t >= first + step * hand_size as u32
                    {
                        h.state.stage = PickStage::Choosing;
                        h.selected = h.selected.min(hand_size as u8 - 1);
                        self.events.push(EventKind::CardSelected {
                            player: pl as u8,
                            slot: h.selected,
                        });
                    }
                }
                PickStage::Choosing => {}
                PickStage::Ending(left) => {
                    if left <= 1 {
                        self.events.push(EventKind::PickEnded { player: pl as u8 });
                        self.events.sound(cue::BALL_DISAPPEAR, V2::ZERO, 1.0);
                        h.state.stage = PickStage::Gap(ticks(0.1));
                    } else {
                        h.state.stage = PickStage::Ending(left - 1);
                    }
                }
                PickStage::Gap(left) => {
                    if left <= 1 {
                        h.done = true;
                    } else {
                        h.state.stage = PickStage::Gap(left - 1);
                    }
                }
            }
            self.flow.hands[k] = h;
        }
        if cards_appear {
            self.events.sound(cue::CARDS_APPEAR, V2::ZERO, 1.0);
        }
        if self.flow.hands.iter().all(|h| h.done) {
            self.flow.hands.clear();
            return self.next_pick_pass();
        }
        true
    }

    /// One tick of the pick queue. Returns false when the queue is done.
    fn pick_tick(&mut self, content: &Content) -> bool {
        if self.flow.pick.is_none() {
            if self.flow.pick_queue.is_empty() && !self.next_pick_pass() {
                return false;
            }
            let pl = self.flow.pick_queue.remove(0);
            self.flow.pick = Some(PickState {
                player: pl,
                hand: Vec::new(),
                stage: PickStage::Offering,
                t: 0,
            });
            self.events.push(EventKind::PickStarted { player: pl });
            self.events.sound(cue::BALL_APPEAR, V2::ZERO, 1.0);
        }
        let mut ps = self.flow.pick.take().unwrap();
        let pl = ps.player as usize;
        ps.t += 1;
        // Navigation (any stage before the pick).
        if matches!(ps.stage, PickStage::Offering | PickStage::Choosing) && !ps.hand.is_empty() {
            let (nav, ok, _) = menu_nav(&self.inputs[pl], &self.prev_inputs[pl]);
            if nav != 0 {
                let n = ps.hand.len() as i32;
                let s = (self.flow.selected as i32 + nav).clamp(0, n - 1) as u8;
                if s != self.flow.selected {
                    self.flow.selected = s;
                    self.events.push(EventKind::CardSelected {
                        player: ps.player,
                        slot: s,
                    });
                    self.events.sound(cue::CARD_SWITCH, V2::ZERO, 1.0);
                }
            }
            if ok && ps.stage == PickStage::Choosing {
                let slot = (self.flow.selected as usize).min(ps.hand.len() - 1);
                let card = ps.hand[slot];
                self.apply_card(content, pl, card);
                self.events.push(EventKind::CardPicked {
                    player: ps.player,
                    card,
                    slot: slot as u8,
                });
                self.events.sound(cue::CARD_PICK, V2::ZERO, 1.0);
                ps.stage = PickStage::Ending(ticks(0.9));
            }
        }
        match ps.stage {
            PickStage::Offering => {
                // Slot j spawns at 0.2 s + 0.1 s * j; choosing 0.1 s after the last.
                let first = ticks(0.2);
                let step = ticks(0.1);
                let hand_size = self.rules.mods.hand_size();
                let n = ps.hand.len() as u32;
                if (n as usize) < hand_size && ps.t >= first + step * n {
                    let owned = self.players[pl].cards.clone();
                    let c = draw_slot(content, &mut self.rng.cards, &ps.hand, &owned);
                    ps.hand.push(c);
                    self.events.push(EventKind::CardOffered {
                        player: ps.player,
                        slot: n as u8,
                        card: c,
                    });
                    if n == 0 {
                        self.events.sound(cue::CARDS_APPEAR, V2::ZERO, 1.0);
                    }
                } else if ps.hand.len() == hand_size && ps.t >= first + step * hand_size as u32 {
                    ps.stage = PickStage::Choosing;
                    let s = (self.flow.selected as usize).min(hand_size - 1) as u8;
                    self.flow.selected = s;
                    self.events.push(EventKind::CardSelected {
                        player: ps.player,
                        slot: s,
                    });
                }
            }
            PickStage::Choosing => {}
            PickStage::Ending(left) => {
                if left <= 1 {
                    self.events.push(EventKind::PickEnded { player: ps.player });
                    self.events.sound(cue::BALL_DISAPPEAR, V2::ZERO, 1.0);
                    ps.stage = PickStage::Gap(ticks(0.1));
                } else {
                    ps.stage = PickStage::Ending(left - 1);
                }
            }
            PickStage::Gap(left) => {
                if left <= 1 {
                    return !self.flow.pick_queue.is_empty() || self.next_pick_pass();
                }
                ps.stage = PickStage::Gap(left - 1);
            }
        }
        self.flow.pick = Some(ps);
        true
    }

    /// Round-end detection after the tick's deaths (match-flow.md 5.1-5.2).
    pub(crate) fn process_deaths(&mut self, content: &Content) {
        if self.flow.deaths.is_empty() {
            return;
        }
        let mut deaths = std::mem::take(&mut self.flow.deaths);
        deaths.sort_unstable();
        if self.rules.mode == GameMode::Sandbox {
            for d in deaths {
                if !self.players[d as usize].disconnected {
                    self.flow.respawns.push((d, ticks(2.5)));
                }
            }
            return;
        }
        if !self.flow.battle || self.flow.transitioning {
            return;
        }
        let mut alive_teams: Vec<u8> = self
            .players
            .iter()
            .filter(|p| !p.dead)
            .map(|p| p.team)
            .collect();
        alive_teams.sort_unstable();
        alive_teams.dedup();
        if alive_teams.len() >= 2 {
            return;
        }
        // Winner: the remaining team; if everybody died this tick, the team
        // of the player who died last in processing order (highest index).
        let winner = match alive_teams.first() {
            Some(t) => *t,
            None => {
                let last = deaths
                    .iter()
                    .rev()
                    .find(|d| !self.players[**d as usize].disconnected)
                    .or(deaths.last())
                    .unwrap();
                self.players[*last as usize].team
            }
        };
        self.time.slow_down();
        self.events.push(EventKind::TimeCurve { slow: true });
        self.announce(content, winner);
    }

    fn announce(&mut self, content: &Content, winner: u8) {
        self.flow.battle = false;
        self.flow.transitioning = true;
        self.flow.last_winner = Some(winner);
        self.set_players_simulated(false);
        let w = winner as usize;
        self.points[w] += 1;
        self.events.push(EventKind::PointWon {
            team: winner,
            points: self.points.clone(),
            rounds: self.rounds.clone(),
        });
        if self.points[w] >= self.rules.points_to_win_round {
            self.rounds[w] += 1;
            if self.rounds[w] >= self.rules.rounds_to_win_game {
                self.events.push(EventKind::GameWon { team: winner });
                self.flow.phase = Phase::GameOver { t: 0, winner };
            } else {
                for p in self.points.iter_mut() {
                    *p = 0;
                }
                self.events.push(EventKind::RoundWon {
                    team: winner,
                    rounds: self.rounds.clone(),
                });
                self.events.sound(cue::WIN_ROUND, V2::ZERO, 1.0);
                self.flow.phase = Phase::RoundOver { t: 0, winner };
            }
        } else {
            self.flow.phase = Phase::PointOver { t: 0, winner };
        }
        for i in 0..self.players.len() {
            self.fire_hook(content, i, Hook::PointEnd);
        }
    }
}
